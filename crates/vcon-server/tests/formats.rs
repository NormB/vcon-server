//! The on-disk formats: the page that lists each one and its version, and
//! the data each released binary wrote, opened by this build.
//!
//! Each directory under tests/data/formats/<release>/ was written by that
//! release's own binary (scripts/make-format-fixtures.py, which takes the
//! binary from the release archive pinned in scripts/docs-test-pins.txt):
//! the data directory as the release left it, the release's test master key
//! and manifest.json, which names every version kept with the SHA-256 of
//! its body and every uuid purged. Every test works on a copy.
//!
//! The rule the page states, held here for every release: this build opens
//! the data (migrating it on the first open), `check` finds nothing wrong,
//! every kept body reads back byte-equal, a second open changes no file, a
//! catalog one version past this build is refused by name, and a migration
//! that fails leaves the release's version whole.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use aws_lc_rs::digest::{SHA256, digest};
use aws_lc_rs::hkdf::{HKDF_SHA256, Salt};

use vcon_server::audit::Actor;
use vcon_server::catalog::SCHEMA_VERSION;
use vcon_server::clock::SystemClock;
use vcon_server::envelope::Key;
use vcon_server::formats::FORMATS;
use vcon_server::fs::OsFs;
use vcon_server::ids::{KeyId, TenantId, VersionId};
use vcon_server::store::{Caller, StoreError};
use vcon_server::tenants::Tenants;

/// The repository's root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The page that lists the formats.
const PAGE: &str = "website/content/docs/on-disk-format.md";

/// The directory of the fixtures.
fn formats_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/formats")
}

fn page() -> String {
    std::fs::read_to_string(root().join(PAGE)).expect("the on-disk format page is there")
}

// ---- The page's tables. ----

/// The rows of the first table after `heading` in `text`: each row's cells,
/// trimmed, the header row first and the separator row left out. None when
/// the heading or its table is not there.
fn table(text: &str, heading: &str) -> Option<Vec<Vec<String>>> {
    let mut lines = text.lines().skip_while(|l| l.trim() != heading).skip(1);
    let mut rows = Vec::new();
    let mut started = false;
    for line in lines.by_ref() {
        let line = line.trim();
        if line.starts_with('|') {
            started = true;
            let cells: Vec<String> = line
                .trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_owned())
                .collect();
            if !cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|x| x == '-' || x == ':'))
            {
                rows.push(cells);
            }
        } else if started || line.starts_with("## ") {
            break;
        }
    }
    (!rows.is_empty()).then_some(rows)
}

/// The text of a cell without the backticks around it.
fn bare(cell: &str) -> &str {
    cell.trim_matches('`')
}

/// The formats table: id to the version it states.
fn page_formats(text: &str) -> Result<BTreeMap<String, String>, String> {
    let rows = table(text, "## The formats").ok_or("no table under \"## The formats\"")?;
    let header = &rows[0];
    let at = |name: &str| {
        header
            .iter()
            .position(|h| h == name)
            .ok_or(format!("the formats table has no column {name}"))
    };
    let (id, version) = (at("Format")?, at("Version")?);
    let mut out = BTreeMap::new();
    for row in &rows[1..] {
        let key = bare(&row[id]).to_owned();
        if out
            .insert(key.clone(), bare(&row[version]).to_owned())
            .is_some()
        {
            return Err(format!("{key} is listed twice"));
        }
    }
    Ok(out)
}

/// The release table: release to (format id to the version it wrote).
fn page_releases(text: &str) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let rows = table(text, "## What each release wrote")
        .ok_or("no table under \"## What each release wrote\"")?;
    let header: Vec<&str> = rows[0].iter().map(|h| bare(h)).collect();
    if header.first() != Some(&"Release") {
        return Err(format!(
            "the release table starts with {:?}",
            header.first()
        ));
    }
    let mut out = BTreeMap::new();
    for row in &rows[1..] {
        if row.len() != header.len() {
            return Err(format!("the row of {} has {} cells", row[0], row.len()));
        }
        let wrote = header[1..]
            .iter()
            .zip(&row[1..])
            .map(|(h, c)| ((*h).to_owned(), bare(c).to_owned()))
            .collect();
        if out.insert(row[0].clone(), wrote).is_some() {
            return Err(format!("{} is listed twice", row[0]));
        }
    }
    Ok(out)
}

const SAMPLE: &str = "\
intro

## The formats

| Format | What | Version |
|---|---|---|
| `catalog` | the catalogs | 9 |
| `envelope` | each sealed blob | 1 |

## What each release wrote

| Release | `catalog` | `envelope` |
|---|---|---|
| 0.4.0 | 4 | 1 |
| 0.4.8 | 9 | 1 |

## Next
";

#[test]
fn the_table_reader_reads_the_formats_of_a_page() {
    let read = page_formats(SAMPLE).expect("the sample reads");
    let want: BTreeMap<String, String> = [("catalog", "9"), ("envelope", "1")]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .into();
    assert_eq!(read, want, "both rows, without backticks");
}

#[test]
fn the_table_reader_reads_the_releases_of_a_page() {
    let read = page_releases(SAMPLE).expect("the sample reads");
    assert_eq!(read.len(), 2, "two releases: {read:?}");
    assert_eq!(read["0.4.0"]["catalog"], "4", "0.4.0 wrote catalog 4");
    assert_eq!(read["0.4.8"]["envelope"], "1", "0.4.8 wrote envelope 1");
}

#[test]
fn a_page_without_the_formats_table_is_refused() {
    let why = page_formats("## Other\n\n| a | b |\n|---|---|\n| 1 | 2 |\n").expect_err("no table");
    assert!(why.contains("The formats"), "names the heading: {why}");
}

#[test]
fn a_format_listed_twice_is_refused() {
    let text = SAMPLE.replace(
        "| `envelope` | each sealed blob | 1 |",
        "| `catalog` | again | 9 |",
    );
    let why = page_formats(&text).expect_err("twice");
    assert!(why.contains("catalog is listed twice"), "names it: {why}");
}

#[test]
fn a_release_listed_twice_is_refused() {
    let text = SAMPLE.replace("| 0.4.8 | 9 | 1 |", "| 0.4.0 | 9 | 1 |");
    let why = page_releases(&text).expect_err("twice");
    assert!(why.contains("0.4.0 is listed twice"), "names it: {why}");
}

#[test]
fn a_release_row_with_a_cell_missing_is_refused() {
    let text = SAMPLE.replace("| 0.4.8 | 9 | 1 |", "| 0.4.8 | 9 |");
    let why = page_releases(&text).expect_err("short row");
    assert!(why.contains("0.4.8 has 2 cells"), "names the row: {why}");
}

#[test]
fn a_formats_table_without_a_version_column_is_refused() {
    let text = SAMPLE.replace("| Format | What | Version |", "| Format | What | Number |");
    let why = page_formats(&text).expect_err("no column");
    assert!(why.contains("no column Version"), "names it: {why}");
}

#[test]
fn the_table_reader_stops_at_the_next_heading() {
    let text = "## The formats\n\n## Next\n\n| Format | Version |\n|---|---|\n| `x` | 1 |\n";
    assert!(
        page_formats(text).is_err(),
        "a table under the next heading is not this one"
    );
}

#[test]
fn every_format_of_this_build_is_on_the_page_with_its_version() {
    let listed = page_formats(&page()).expect("the page's formats table");
    for format in FORMATS {
        assert_eq!(
            listed.get(format.id).map(String::as_str),
            Some(format.version),
            "{PAGE} states format {} at the version this build writes",
            format.id
        );
    }
}

#[test]
fn every_format_on_the_page_is_one_this_build_has() {
    let listed = page_formats(&page()).expect("the page's formats table");
    let ids: Vec<&str> = FORMATS.iter().map(|f| f.id).collect();
    for id in listed.keys() {
        assert!(
            ids.contains(&id.as_str()),
            "{PAGE} lists {id}, which formats::FORMATS does not"
        );
    }
}

#[test]
fn the_formats_of_this_build_have_distinct_ids() {
    let mut ids: Vec<&str> = FORMATS.iter().map(|f| f.id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), before, "one entry per format");
    assert!(before >= 6, "every format there is: {before}");
}

#[test]
fn the_release_table_has_a_column_for_every_format() {
    let releases = page_releases(&page()).expect("the page's release table");
    for (release, wrote) in &releases {
        let columns: Vec<&str> = wrote.keys().map(String::as_str).collect();
        let mut ids: Vec<&str> = FORMATS.iter().map(|f| f.id).collect();
        ids.sort_unstable();
        assert_eq!(columns, ids, "the row of {release} has one cell per format");
    }
}

#[test]
fn the_release_table_lists_exactly_the_releases_with_a_fixture() {
    let releases = page_releases(&page()).expect("the page's release table");
    let listed: Vec<&String> = releases.keys().collect();
    let fixtures = fixture_releases();
    assert_eq!(
        listed,
        fixtures.iter().collect::<Vec<_>>(),
        "one row per directory under tests/data/formats"
    );
}

#[test]
fn every_released_binary_since_0_4_0_has_a_fixture() {
    // The pins name each release's archives; each release from 0.4.0 on
    // must have its data here.
    let pins = std::fs::read_to_string(root().join("scripts/docs-test-pins.txt")).expect("pins");
    let mut pinned: Vec<String> = pins
        .lines()
        .filter_map(|l| l.split("/releases/download/v").nth(1))
        .filter_map(|rest| rest.split('/').next())
        .filter(|r| r.starts_with("0.4."))
        .map(str::to_owned)
        .collect();
    pinned.sort_by_key(|r| release_order(r));
    pinned.dedup();
    assert_eq!(
        pinned,
        fixture_releases(),
        "a fixture for every pinned 0.4 release"
    );
}

#[test]
fn the_newest_release_row_states_this_builds_versions() {
    // No format has changed since the newest release; when one does, its
    // version moves past that row and this test names it, so the page and
    // CHANGELOG say so.
    let releases = page_releases(&page()).expect("the page's release table");
    let newest = fixture_releases().pop().expect("a release");
    for format in FORMATS {
        assert_eq!(
            releases[&newest][format.id], format.version,
            "{newest} wrote {} at this build's version",
            format.id
        );
    }
}

// ---- The fixtures. ----

/// The releases with a fixture, oldest first.
fn fixture_releases() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(formats_dir())
        .expect("tests/data/formats is there")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .collect();
    out.sort_by_key(|r| release_order(r));
    out
}

fn release_order(release: &str) -> Vec<u32> {
    release
        .split('.')
        .map(|p| p.parse().expect("a number"))
        .collect()
}

/// One kept version, as the manifest names it.
struct Row {
    tenant: String,
    uuid: uuid::Uuid,
    version: VersionId,
    order: usize,
    state: String,
    sha256: String,
}

struct Manifest {
    archive_sha256: String,
    rows: Vec<Row>,
    purged: Vec<(String, uuid::Uuid)>,
}

fn manifest(release: &str) -> Manifest {
    let path = formats_dir().join(release).join("manifest.json");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the manifest")).expect("JSON");
    let text = |v: &serde_json::Value, k: &str| v[k].as_str().expect(k).to_owned();
    let rows = doc["versions"]
        .as_array()
        .expect("versions")
        .iter()
        .map(|v| Row {
            tenant: text(v, "tenant"),
            uuid: text(v, "uuid").parse().expect("a uuid"),
            version: VersionId::parse(&text(v, "version")).expect("a version id"),
            order: usize::try_from(v["order"].as_u64().expect("order")).expect("small"),
            state: text(v, "state"),
            sha256: text(v, "sha256"),
        })
        .collect();
    let purged = doc["purged"]
        .as_array()
        .expect("purged")
        .iter()
        .map(|v| (text(v, "tenant"), text(v, "uuid").parse().expect("a uuid")))
        .collect();
    assert_eq!(
        text(&doc, "release"),
        release,
        "the manifest is the release's own"
    );
    Manifest {
        archive_sha256: text(&doc, "archive_sha256"),
        rows,
        purged,
    }
}

fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// Copies `from` to `to`, files 0600 and directories 0700, as the server
/// keeps them (git keeps no mode but the executable bit).
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir(to).expect("a directory");
    chmod(to, 0o700);
    for entry in std::fs::read_dir(from).expect("read") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
            chmod(&target, 0o600);
        }
    }
}

/// A copy of one release's fixture, with a configuration and a certificate
/// of its own.
struct Case {
    release: String,
    dir: tempfile::TempDir,
    manifest: Manifest,
}

impl Case {
    fn new(release: &str) -> Case {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let from = formats_dir().join(release);
        copy_tree(&from.join("data"), &dir.path().join("data"));
        let key = dir.path().join("master.key");
        std::fs::copy(from.join("master.key"), &key).expect("the master key");
        chmod(&key, 0o600);
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate");
        let (cert, tls) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        std::fs::write(&cert, made.cert.pem()).expect("write");
        std::fs::write(&tls, made.signing_key.serialize_pem()).expect("write");
        chmod(&tls, 0o600);
        std::fs::write(
            dir.path().join("vcon-server.toml"),
            format!(
                "listen = \"127.0.0.1:0\"\ndata_dir = {:?}\nmaster_key = {:?}\n\n[tls]\ncert = {:?}\nkey = {:?}\n",
                dir.path().join("data"),
                key,
                cert,
                tls
            ),
        )
        .expect("the configuration");
        Case {
            release: release.to_owned(),
            dir,
            manifest: manifest(release),
        }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn master_key(&self) -> PathBuf {
        self.dir.path().join("master.key")
    }

    /// Runs this build's `check` on the copy.
    fn check(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .arg("check")
            .arg("--config")
            .arg(self.dir.path().join("vcon-server.toml"))
            .output()
            .expect("the binary runs")
    }

    /// Runs `check` and asserts it passed, giving its report lines.
    fn checked(&self) -> Vec<serde_json::Value> {
        let out = self.check();
        assert!(
            out.status.success(),
            "check passes on {}'s data: {}",
            self.release,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .expect("UTF-8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("a JSON line"))
            .collect()
    }

    /// The SHA-256 of every file under the data directory, by its path.
    fn files(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).expect("read") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    walk(&path, base, out);
                } else {
                    let bytes = std::fs::read(&path).expect("read");
                    let rel = path.strip_prefix(base).expect("under").to_owned();
                    out.insert(rel, digest(&SHA256, &bytes).as_ref().to_vec());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.data(), &self.data(), &mut out);
        out
    }

    /// server.sqlite, opened as SQLCipher opens it with the catalog key
    /// derived here as the format states it (HKDF-SHA256 of the master key,
    /// no salt, info `vcon-server/v1 catalog`), and with no migration.
    fn raw_registry(&self) -> rusqlite::Connection {
        let master = std::fs::read(self.master_key()).expect("the master key");
        let mut key = [0u8; 32];
        Salt::new(HKDF_SHA256, &[])
            .extract(&master)
            .expand(&[b"vcon-server/v1 catalog"], HKDF_SHA256)
            .expect("expand")
            .fill(&mut key)
            .expect("fill");
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let conn = rusqlite::Connection::open(self.data().join("server.sqlite")).expect("opens");
        conn.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
            .expect("the key is set");
        conn
    }

    fn registry_version(&self) -> i64 {
        self.raw_registry()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("the version reads under the derived key")
    }

    fn schema(&self) -> Vec<String> {
        let conn = self.raw_registry();
        let mut statement = conn
            .prepare("SELECT coalesce(sql, name) FROM sqlite_master ORDER BY name")
            .expect("prepare");
        statement
            .query_map([], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("a row"))
            .collect()
    }

    fn tenants(&self) -> Tenants {
        Tenants::open(
            &self.data(),
            Key::load_master(&self.master_key()).expect("the master key"),
            Arc::new(OsFs),
            Arc::new(SystemClock),
        )
        .expect("this build opens the registry")
    }

    /// The id of tenant `name`.
    fn tenant(&self, tenants: &Tenants, name: &str) -> TenantId {
        tenants
            .list(Actor::Cli)
            .expect("the tenants list")
            .into_iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{}: tenant {name} is there", self.release))
            .id
    }
}

const CALLER: Caller = Caller {
    key: KeyId([0x5a; 8]),
};

fn sha_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// Success.

fn check_passes_and_counts_every_kept_version(release: &str) {
    let case = Case::new(release);
    let lines = case.checked();
    for tenant in ["tenant-a", "tenant-b"] {
        let line = lines
            .iter()
            .find(|l| l["name"] == tenant)
            .unwrap_or_else(|| panic!("check reports {tenant}: {lines:?}"));
        let kept = case
            .manifest
            .rows
            .iter()
            .filter(|r| r.tenant == tenant)
            .count();
        assert_eq!(
            line["versions"].as_u64(),
            Some(u64::try_from(kept).expect("small")),
            "{release}: check counts {tenant}'s kept versions"
        );
        assert_eq!(
            line["damaged"],
            serde_json::json!([]),
            "{release}: nothing damaged"
        );
        assert_eq!(
            line["problems"],
            serde_json::json!([]),
            "{release}: no problem"
        );
    }
}

fn every_kept_version_reads_back_byte_equal(release: &str) {
    let case = Case::new(release);
    let tenants = case.tenants();
    for name in ["tenant-a", "tenant-b"] {
        let store = tenants
            .store(case.tenant(&tenants, name))
            .expect("the tenant's store");
        let rows: Vec<&Row> = case
            .manifest
            .rows
            .iter()
            .filter(|r| r.tenant == name)
            .collect();
        assert!(!rows.is_empty(), "{release}: {name} kept versions");
        for row in rows {
            let read = store
                .legal_get(&CALLER, row.uuid, Some(row.version))
                .unwrap_or_else(|e| {
                    panic!("{release}: {name} version {} reads: {e:?}", row.version)
                });
            assert_eq!(
                sha_hex(&read.stored.body),
                row.sha256,
                "{release}: {name} version {} is the body the release stored",
                row.version
            );
            assert_eq!(
                read.deleted,
                is_deleted(&case, name, row.uuid),
                "{release}: {name} version {} is of a uuid in the state the release left",
                row.version
            );
        }
    }
}

/// Whether the manifest's latest version of `uuid` is a deleted one.
fn is_deleted(case: &Case, tenant: &str, uuid: uuid::Uuid) -> bool {
    case.manifest
        .rows
        .iter()
        .filter(|r| r.tenant == tenant && r.uuid == uuid)
        .max_by_key(|r| r.order)
        .is_some_and(|r| r.state.starts_with("deleted"))
}

fn every_uuid_lists_its_kept_versions_in_order(release: &str) {
    let case = Case::new(release);
    let tenants = case.tenants();
    for name in ["tenant-a", "tenant-b"] {
        let store = tenants
            .store(case.tenant(&tenants, name))
            .expect("the tenant's store");
        let mut by_uuid: BTreeMap<uuid::Uuid, Vec<&Row>> = BTreeMap::new();
        for row in case.manifest.rows.iter().filter(|r| r.tenant == name) {
            by_uuid.entry(row.uuid).or_default().push(row);
        }
        for rows in by_uuid.values_mut() {
            rows.sort_by_key(|r| r.order);
            let listed = store
                .legal_versions(&CALLER, rows[0].uuid)
                .expect("the versions list");
            let ids: Vec<VersionId> = listed.iter().map(|v| v.version).collect();
            let want: Vec<VersionId> = rows.iter().map(|r| r.version).collect();
            assert_eq!(
                ids, want,
                "{release}: {name} keeps these versions, oldest first"
            );
            let current: Vec<bool> = listed.iter().map(|v| v.current).collect();
            let want: Vec<bool> = rows
                .iter()
                .map(|r| r.state.starts_with("current"))
                .collect();
            assert_eq!(
                current, want,
                "{release}: {name}: the current version is the release's"
            );
        }
    }
}

fn the_registry_is_at_this_builds_version_after_the_first_open(release: &str) {
    let case = Case::new(release);
    case.checked();
    assert_eq!(
        case.registry_version(),
        SCHEMA_VERSION,
        "{release}: the first open brings server.sqlite to this build's version"
    );
}

fn a_second_open_changes_no_file(release: &str) {
    let case = Case::new(release);
    case.checked();
    let first = case.files();
    case.checked();
    assert_eq!(
        case.files(),
        first,
        "{release}: a second check writes nothing"
    );
}

// Failure.

fn a_registry_one_version_past_this_build_is_refused_by_name(release: &str) {
    let case = Case::new(release);
    case.raw_registry()
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .expect("the version is set");
    let before = case.files();
    let out = case.check();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "{release}: check refuses a later version"
    );
    assert!(
        said.contains(&format!(
            "schema version {} is not one this build knows",
            SCHEMA_VERSION + 1
        )),
        "{release}: the refusal names the version: {said}"
    );
    assert_eq!(
        case.files(),
        before,
        "{release}: the refused data is left as it was"
    );
}

fn a_wrong_master_key_opens_nothing_and_changes_nothing(release: &str) {
    let case = Case::new(release);
    std::fs::write(case.master_key(), [0x33u8; 32]).expect("another key");
    let before = case.files();
    let out = case.check();
    assert!(
        !out.status.success(),
        "{release}: check refuses another master key"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("key"),
        "{release}: the refusal names the key: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        case.files(),
        before,
        "{release}: nothing is written under the wrong key"
    );
}

fn a_purged_uuid_is_gone(release: &str) {
    let case = Case::new(release);
    assert!(
        !case.manifest.purged.is_empty(),
        "{release}: the workload purged a uuid"
    );
    let tenants = case.tenants();
    for (name, uuid) in &case.manifest.purged {
        let store = tenants
            .store(case.tenant(&tenants, name))
            .expect("the tenant's store");
        assert!(
            matches!(store.legal_versions(&CALLER, *uuid), Err(StoreError::Gone)),
            "{release}: a uuid {name} purged stays gone"
        );
    }
}

/// The registry's own version before this build opens it.
fn written_version(release: &str) -> i64 {
    Case::new(release).registry_version()
}

fn a_failed_migration_leaves_the_releases_version_whole(release: &str) {
    let case = Case::new(release);
    let old = case.registry_version();
    assert!(
        old < SCHEMA_VERSION,
        "{release} wrote an older version: {old}"
    );
    // The last step of every migration adds the access keys' expiry: a
    // column of that name already there makes it fail, and with it the
    // whole transaction.
    case.raw_registry()
        .execute_batch("ALTER TABLE api_key ADD COLUMN expires_at INTEGER;")
        .expect("the column in the way");
    let before = case.schema();
    let out = case.check();
    assert!(
        !out.status.success(),
        "{release}: the migration fails at its last step"
    );
    assert_eq!(
        case.registry_version(),
        old,
        "{release}: still the release's version"
    );
    assert_eq!(
        case.schema(),
        before,
        "{release}: no earlier step of the migration is kept"
    );
    case.raw_registry()
        .execute_batch("ALTER TABLE api_key DROP COLUMN expires_at;")
        .expect("the column taken away");
    case.checked();
    assert_eq!(
        case.registry_version(),
        SCHEMA_VERSION,
        "{release}: then it migrates"
    );
}

// Edge.

fn held_versions_outlive_a_sweep_under_a_0d_period(release: &str) {
    let case = Case::new(release);
    let tenants = case.tenants();
    let id = case.tenant(&tenants, "tenant-b");
    let store = tenants.store(id).expect("the tenant's store");
    let retention = tenants
        .retention(id)
        .expect("its period")
        .expect("0d is set");
    store.purge_due(retention).expect("the sweep runs");
    for row in case.manifest.rows.iter().filter(|r| r.tenant == "tenant-b") {
        let read = store.legal_get(&CALLER, row.uuid, Some(row.version));
        assert!(
            read.is_ok(),
            "{release}: tenant-b version {} survives the sweep",
            row.version
        );
    }
    assert!(
        case.manifest
            .rows
            .iter()
            .any(|r| r.tenant == "tenant-b" && r.state.starts_with("hidden+held")),
        "{release}: a hidden held version was there to purge"
    );
}

fn a_deleted_vcon_is_gone_to_an_ordinary_read(release: &str) {
    let case = Case::new(release);
    let tenants = case.tenants();
    let store = tenants
        .store(case.tenant(&tenants, "tenant-a"))
        .expect("the store");
    let deleted: Vec<&Row> = case
        .manifest
        .rows
        .iter()
        .filter(|r| r.tenant == "tenant-a" && r.state.starts_with("deleted"))
        .collect();
    assert!(deleted.len() >= 2, "{release}: deleted vCons were kept");
    for row in deleted {
        let refused = store.get(&CALLER, row.uuid).err();
        assert!(
            matches!(refused, Some(StoreError::Gone)),
            "{release}: an ordinary read answers gone for deleted version {}: {refused:?}",
            row.version
        );
    }
}

fn the_fixture_was_written_by_the_pinned_release(release: &str) {
    let pins = std::fs::read_to_string(root().join("scripts/docs-test-pins.txt")).expect("pins");
    let sha = &manifest(release).archive_sha256;
    let line = pins
        .lines()
        .find(|l| l.starts_with(sha.as_str()))
        .unwrap_or_else(|| panic!("{release}: the archive hash is a pin"));
    assert!(
        line.contains(&format!("/v{release}/vcon-server-{release}-"))
            && line.ends_with("-unknown-linux-gnu.tar.gz"),
        "{release}: the pin is the release's archive: {line}"
    );
}

fn every_object_starts_with_the_envelope_version_the_page_states(release: &str) {
    let releases = page_releases(&page()).expect("the release table");
    let stated: u8 = releases[release]["envelope"].parse().expect("a number");
    let case = Case::new(release);
    let objects: Vec<PathBuf> = case
        .files()
        .into_keys()
        .filter(|p| p.components().any(|c| c.as_os_str() == "objects"))
        .collect();
    assert!(
        !objects.is_empty(),
        "{release}: the release wrote object files"
    );
    for path in objects {
        let bytes = std::fs::read(case.data().join(&path)).expect("read");
        assert_eq!(
            bytes.first(),
            Some(&stated),
            "{release}: {} opens with the envelope's version",
            path.display()
        );
    }
}

fn the_page_states_the_catalog_version_the_release_wrote(release: &str) {
    let releases = page_releases(&page()).expect("the release table");
    assert_eq!(
        releases[release]["catalog"],
        written_version(release).to_string(),
        "{PAGE}: the catalog version {release} wrote"
    );
}

/// The tests of one release's fixture; `older` adds the ones only data at
/// an earlier catalog version than this build's can have.
macro_rules! release {
    ($name:ident, $release:literal $(, $older:ident)?) => {
        mod $name {
            #[test]
            fn check_passes_and_counts_every_kept_version() {
                super::check_passes_and_counts_every_kept_version($release);
            }
            #[test]
            fn every_kept_version_reads_back_byte_equal() {
                super::every_kept_version_reads_back_byte_equal($release);
            }
            #[test]
            fn every_uuid_lists_its_kept_versions_in_order() {
                super::every_uuid_lists_its_kept_versions_in_order($release);
            }
            #[test]
            fn the_registry_is_at_this_builds_version_after_the_first_open() {
                super::the_registry_is_at_this_builds_version_after_the_first_open($release);
            }
            #[test]
            fn a_second_open_changes_no_file() {
                super::a_second_open_changes_no_file($release);
            }
            #[test]
            fn a_registry_one_version_past_this_build_is_refused_by_name() {
                super::a_registry_one_version_past_this_build_is_refused_by_name($release);
            }
            #[test]
            fn a_wrong_master_key_opens_nothing_and_changes_nothing() {
                super::a_wrong_master_key_opens_nothing_and_changes_nothing($release);
            }
            #[test]
            fn a_purged_uuid_is_gone() {
                super::a_purged_uuid_is_gone($release);
            }
            #[test]
            fn held_versions_outlive_a_sweep_under_a_0d_period() {
                super::held_versions_outlive_a_sweep_under_a_0d_period($release);
            }
            #[test]
            fn a_deleted_vcon_is_gone_to_an_ordinary_read() {
                super::a_deleted_vcon_is_gone_to_an_ordinary_read($release);
            }
            #[test]
            fn the_fixture_was_written_by_the_pinned_release() {
                super::the_fixture_was_written_by_the_pinned_release($release);
            }
            #[test]
            fn every_object_starts_with_the_envelope_version_the_page_states() {
                super::every_object_starts_with_the_envelope_version_the_page_states($release);
            }
            #[test]
            fn the_page_states_the_catalog_version_the_release_wrote() {
                super::the_page_states_the_catalog_version_the_release_wrote($release);
            }
            $(
                #[test]
                fn a_failed_migration_leaves_the_releases_version_whole() {
                    let _ = stringify!($older);
                    super::a_failed_migration_leaves_the_releases_version_whole($release);
                }
            )?
        }
    };
}

release!(release_0_4_0, "0.4.0", older);
release!(release_0_4_1, "0.4.1", older);
release!(release_0_4_2, "0.4.2", older);
release!(release_0_4_3, "0.4.3", older);
release!(release_0_4_4, "0.4.4");
release!(release_0_4_5, "0.4.5");
release!(release_0_4_6, "0.4.6");
release!(release_0_4_7, "0.4.7");
release!(release_0_4_8, "0.4.8");

#[test]
fn the_older_tests_cover_exactly_the_releases_that_wrote_an_older_version() {
    let older: Vec<String> = fixture_releases()
        .into_iter()
        .filter(|r| written_version(r) < SCHEMA_VERSION)
        .collect();
    assert_eq!(
        older,
        ["0.4.0", "0.4.1", "0.4.2", "0.4.3"],
        "the releases whose fixtures get the failed-migration test"
    );
}
