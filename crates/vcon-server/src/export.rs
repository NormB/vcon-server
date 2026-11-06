//! `export parquet`: a copy of one tenant's kept versions as Parquet files,
//! written beside a running server, which it never changes but for one
//! audit record.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::audit::{Action, Actor, Entry};
use crate::clock::from_nanos;
use crate::store::{KeptVersion, Store, StoreError};

pub(crate) mod columns;
#[cfg(feature = "s3")]
pub(crate) mod s3;
pub(crate) mod target;

pub(crate) use columns::{SIZES, Sizes};
pub(crate) use target::{DirTarget, Target, TargetError};

/// Which clear columns a tenant's Parquet files carry beside the sealed
/// body. Each level adds to the one before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Metadata {
    /// Only the version id, the store time and the sealed body.
    #[default]
    None,
    /// Also the form, gzip, size, the vCon's times and how many parties
    /// and dialogs it has.
    Counts,
    /// Also the uuid, the subject and the parties' addresses and names:
    /// personal data in clear.
    Full,
}

impl Metadata {
    /// The level's name: `none`, `counts` or `full`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Metadata::None => "none",
            Metadata::Counts => "counts",
            Metadata::Full => "full",
        }
    }

    /// The level `as_str` names, if `text` names one.
    #[must_use]
    pub fn parse(text: &str) -> Option<Metadata> {
        [Metadata::None, Metadata::Counts, Metadata::Full]
            .into_iter()
            .find(|level| level.as_str() == text)
    }
}

/// The version of the manifest's layout, written in each as `format`.
pub const MANIFEST_FORMAT: u32 = 1;

/// The directory of a tenant's files under the target.
pub fn tenant_dir(store: &Store) -> String {
    format!("tenant={}/", store.tenant())
}

/// What one export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// The manifest's sequence number.
    pub seq: u64,
    /// The manifest's path under the target.
    pub manifest: String,
    /// Each file, with its rows and bytes.
    pub files: Vec<FileEntry>,
    /// Versions written.
    pub rows: u64,
    /// Bytes of every file written.
    pub bytes: u64,
    /// Versions listed but purged before their object was read.
    pub skipped: u64,
    /// Versions whose body could not be read for the metadata columns,
    /// which are NULL for them.
    pub unread: u64,
}

/// One file a manifest names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    /// Its path under the target.
    pub path: String,
    pub rows: u64,
    pub bytes: u64,
}

/// Why an export did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// The Store could not be read, or a version's object is missing or
    /// does not open.
    Store(StoreError),
    /// A Parquet file could not be built.
    Write(String),
    /// The target could not be written.
    Target(String),
    /// Another export took this export's manifest name first.
    Raced(String),
    /// The audit record could not be written; the export itself is whole.
    AuditFailed,
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Store(StoreError::Damaged) => {
                f.write_str("a version's object is missing or does not open; run vcon-server check")
            }
            ExportError::Store(e) => write!(f, "the tenant's data could not be read: {e:?}"),
            ExportError::Write(why) => write!(f, "a Parquet file could not be built: {why}"),
            ExportError::Target(why) => write!(f, "the target could not be written: {why}"),
            ExportError::Raced(name) => write!(
                f,
                "another export wrote {name} first; run this export again"
            ),
            ExportError::AuditFailed => f.write_str(
                "the export is written, but its audit record could not be; the manifest names its files",
            ),
        }
    }
}

/// What an export reads: the tenant's level, the store-time window in
/// nanoseconds, the sizes of its files, and the bounds on reading a body.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    pub level: Metadata,
    pub since: Option<i64>,
    pub until: Option<i64>,
    pub sizes: Sizes,
    pub limits: vcon::read::Limits,
}

/// Exports the Store's kept versions in `plan`'s window to `target`, then
/// records the run, with its outcome, in the tenant's audit log.
///
/// The versions are listed in one snapshot of the catalog. Each date's
/// rows go in files under `tenant=<id>/date=<yyyy-mm-dd>/`, each written
/// whole, and once every file is written the manifest
/// `tenant=<id>/manifest/<seq>.json` names them; a reader that goes by the
/// newest manifest never sees a file of an export cut off part way. A run
/// that fails removes the files it wrote.
pub fn export(
    store: &Store,
    plan: &Plan,
    target: &dyn Target,
    now: OffsetDateTime,
) -> Result<Exported, ExportError> {
    let mut written = Vec::new();
    let done = write_all(store, plan, target, now, &mut written);
    if done.is_err() {
        // Best effort: a file no manifest names is never read by a reader
        // that goes by the manifests, and a later export makes new names.
        for path in written.iter().rev() {
            let _ = target.remove(path);
        }
    }
    let entry = Entry {
        actor: Actor::Cli,
        action: Action::Export,
        target: None,
        version: None,
        status: if done.is_ok() { 200 } else { 500 },
    };
    let logged = store.log(&entry);
    let done = done?;
    logged.map_err(|_| ExportError::AuditFailed)?;
    Ok(done)
}

/// The export itself; each path written is added to `written` as soon as
/// it is, so a failure can remove them.
fn write_all(
    store: &Store,
    plan: &Plan,
    target: &dyn Target,
    now: OffsetDateTime,
    written: &mut Vec<String>,
) -> Result<Exported, ExportError> {
    let dir = tenant_dir(store);
    let seq = next_seq(target, &dir)?;
    let kept = store
        .kept_versions(plan.since, plan.until)
        .map_err(ExportError::Store)?;
    let mut done = Exported {
        seq,
        manifest: format!("{dir}manifest/{seq:020}.json"),
        files: Vec::new(),
        rows: 0,
        bytes: 0,
        skipped: 0,
        unread: 0,
    };
    let mut live = Vec::new();
    let mut open: Option<(String, columns::File)> = None;
    for k in &kept {
        let date = date_of(k.stored_at)?;
        if let Some((open_date, file)) = &open
            && (open_date != &date || file.full())
        {
            let (open_date, file) = open.take().expect("checked above");
            put_file(target, &dir, &open_date, file, &mut done, written)?;
        }
        let Some(sealed) = store.sealed_of(k).map_err(ExportError::Store)? else {
            done.skipped += 1;
            continue;
        };
        let about = about(store, plan, k, &sealed, &mut done.unread)?;
        if open.is_none() {
            let file = columns::File::new(plan.level, plan.sizes).map_err(write_error)?;
            open = Some((date, file));
        }
        let (_, file) = open.as_mut().expect("made above");
        // An export's reader finds a row by its columns, not its place.
        let _place = file
            .push(columns::Row {
                version: k.version,
                stored_at: k.stored_at,
                sealed,
                about,
            })
            .map_err(write_error)?;
        if k.live {
            live.push(k.version.to_string());
        }
    }
    if let Some((date, file)) = open.take() {
        put_file(target, &dir, &date, file, &mut done, written)?;
    }
    let time = |at: Option<i64>| {
        at.map(|n| from_nanos(n).and_then(|t| t.format(&Rfc3339).ok()))
            .map(|t| t.ok_or(ExportError::Store(StoreError::Damaged)))
            .transpose()
    };
    let manifest = serde_json::json!({
        "format": MANIFEST_FORMAT,
        "tenant": store.tenant().to_string(),
        "seq": seq,
        "metadata": plan.level.as_str(),
        "exported_at": now.format(&Rfc3339).map_err(|e| ExportError::Write(e.to_string()))?,
        "since": time(plan.since)?,
        "until": time(plan.until)?,
        "files": done.files,
        "rows": done.rows,
        "bytes": done.bytes,
        "live": live,
    });
    let text = format!("{manifest:#}\n");
    match target.put_new(&done.manifest, text.as_bytes()) {
        Ok(()) => Ok(done),
        Err(TargetError::Exists) => Err(ExportError::Raced(done.manifest)),
        Err(TargetError::Io(why)) => Err(ExportError::Target(why)),
    }
}

/// The number after the highest manifest under `dir`, from 1.
fn next_seq(target: &dyn Target, dir: &str) -> Result<u64, ExportError> {
    let names = target
        .list(&format!("{dir}manifest/"))
        .map_err(|e| ExportError::Target(e.to_string()))?;
    let highest = names
        .iter()
        .filter_map(|name| name.strip_suffix(".json"))
        .filter(|n| n.len() == 20 && n.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|n| n.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    Ok(highest + 1)
}

/// Finishes `file` and writes it under a new random name in `date`'s
/// directory.
fn put_file(
    target: &dyn Target,
    dir: &str,
    date: &str,
    file: columns::File,
    done: &mut Exported,
    written: &mut Vec<String>,
) -> Result<(), ExportError> {
    let rows = file.rows();
    let bytes = file.finish().map_err(write_error)?;
    let path = format!(
        "{dir}date={date}/part-{}.parquet",
        crate::ids::VersionId::generate()
    );
    match target.put_new(&path, &bytes) {
        Ok(()) => {}
        // A random 128-bit name is never taken, unless by an attacker who
        // can write the target; either way nothing is replaced.
        Err(TargetError::Exists) => return Err(ExportError::Target(format!("{path} is taken"))),
        Err(TargetError::Io(why)) => return Err(ExportError::Target(why)),
    }
    written.push(path.clone());
    done.rows += rows;
    done.bytes += bytes.len() as u64;
    done.files.push(FileEntry {
        path,
        rows,
        bytes: bytes.len() as u64,
    });
    Ok(())
}

fn write_error(e: columns::WriteError) -> ExportError {
    ExportError::Write(e.0.to_string())
}

/// The UTC date of a store time, as yyyy-mm-dd.
fn date_of(stored_at: i64) -> Result<String, ExportError> {
    crate::backend::pack::date_of(stored_at).ok_or(ExportError::Store(StoreError::Damaged))
}

/// What the plan's level writes of `k` beside its sealed body. Below
/// `counts` nothing is opened. A body that cannot be read for its fields
/// (it passed the server's limits when stored, which may have been lowered
/// since) is counted in `unread` and its parsed columns are NULL.
fn about(
    store: &Store,
    plan: &Plan,
    k: &KeptVersion,
    sealed: &[u8],
    unread: &mut u64,
) -> Result<columns::About, ExportError> {
    if plan.level == Metadata::None {
        return Ok(columns::About::default());
    }
    let body = store.open_sealed(k, sealed).map_err(ExportError::Store)?;
    let fields = match vcon::read::fields(&body, &plan.limits) {
        Ok(fields) => fields,
        Err(_) => {
            *unread += 1;
            None
        }
    };
    let fields = fields.unwrap_or_default();
    let time = |text: &Option<String>| {
        text.as_deref()
            .and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok())
            .map(crate::clock::nanos)
    };
    let count = |n: Option<usize>| n.and_then(|n| i32::try_from(n).ok());
    let mut about = columns::About {
        form: Some(k.form.clone()),
        gzip: Some(k.gzip),
        size: Some(k.size),
        created_at: time(&fields.created_at),
        updated_at: time(&fields.updated_at),
        party_count: count(fields.parties.as_ref().map(Vec::len)),
        dialog_count: count(fields.dialogs),
        ..columns::About::default()
    };
    if plan.level == Metadata::Full {
        about.uuid = Some(k.uuid);
        about.subject = fields.subject;
        about.parties = fields.parties.map(|parties| {
            let shown: Vec<serde_json::Value> = parties
                .into_iter()
                .map(|p| {
                    let mut o = serde_json::Map::new();
                    for (name, value) in [("tel", p.tel), ("mailto", p.mailto), ("name", p.name)] {
                        if let Some(value) = value {
                            o.insert(name.to_owned(), value.into());
                        }
                    }
                    serde_json::Value::Object(o)
                })
                .collect();
            serde_json::Value::Array(shown).to_string()
        });
    }
    Ok(about)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use parquet::record::Field;
    use serde_json::{Value, json};
    use uuid::Uuid;
    use vcon::read::{Limits, inspect};

    use super::*;
    use crate::clock::{Clock, nanos};
    use crate::fs::{CrashFs, Fs, OsFs};
    use crate::ids::VersionId;
    use crate::retention::Retention;
    use crate::store::Created;
    use crate::store::tests::{Fixture, fixture};

    const LIMITS: Limits = Limits::new(1 << 20);

    /// The uuid of the n-th test vCon.
    fn uuid(n: u8) -> String {
        format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}")
    }

    /// Words no file at level `none` may hold: a subject and party names
    /// and addresses (RFC 2606 domains and 555-01xx numbers).
    fn subject(n: u8) -> String {
        format!("SUBJECT-MARKER-{n:02}")
    }

    fn unsigned(n: u8) -> Vec<u8> {
        json!({
            "uuid": uuid(n),
            "created_at": "2026-10-23T12:00:00Z",
            "subject": subject(n),
            "parties": [
                {"tel": format!("+1555555{:04}", 100 + u16::from(n)), "name": format!("NAME-MARKER-{n:02}")},
                {"mailto": format!("party{n}@example.com")}
            ],
            "dialog": [{"type": "text", "body": "x", "encoding": "none"}]
        })
        .to_string()
        .into_bytes()
    }

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(bytes).expect("compress");
        e.finish().expect("finish")
    }

    fn signed(n: u8) -> Vec<u8> {
        json!({
            "payload": URL_SAFE_NO_PAD.encode(unsigned(n)),
            "signatures": [{"header": {}, "protected": "", "signature": ""}]
        })
        .to_string()
        .into_bytes()
    }

    fn encrypted(n: u8) -> Vec<u8> {
        json!({"unprotected": {"uuid": uuid(n)}, "ciphertext": "x", "recipients": []})
            .to_string()
            .into_bytes()
    }

    /// Stores `body` as a new vCon, a minute after the last store.
    fn create(f: &Fixture, body: &[u8]) -> VersionId {
        f.clock.set(f.clock.now() + time::Duration::minutes(1));
        let found = inspect(body, None, &LIMITS, &BTreeSet::new()).expect("a vCon");
        match f.store.create(&f.caller, &found, body).expect("create") {
            Created::New(v) => v,
            other => panic!("{other:?}"),
        }
    }

    /// Every form, plain and gzipped; uuid 1 replaced, so its first
    /// version is hidden; uuid 9 deleted and purged; uuid 8 stored the next
    /// day. Returns the kept versions.
    fn stored(f: &Fixture) -> Vec<KeptVersion> {
        let first = create(f, &unsigned(1));
        let again = unsigned(1).iter().chain(b" ").copied().collect::<Vec<u8>>();
        let found = inspect(&again, None, &LIMITS, &BTreeSet::new()).expect("a vCon");
        f.store
            .replace(&f.caller, first, &found, &again)
            .expect("replace");
        create(f, &gz(&unsigned(2)));
        create(f, &signed(3));
        create(f, &gz(&signed(4)));
        create(f, &encrypted(5));
        create(f, &gz(&encrypted(6)));
        create(f, &unsigned(9));
        f.store
            .delete(
                &f.caller,
                uuid(9).parse().expect("uuid"),
                Retention::Days(0),
            )
            .expect("delete purges at once");
        f.clock.set(f.clock.now() + time::Duration::days(1));
        create(f, &unsigned(8));
        f.store.kept_versions(None, None).expect("lists")
    }

    fn plan(level: Metadata) -> Plan {
        Plan {
            level,
            since: None,
            until: None,
            sizes: SIZES,
            limits: LIMITS,
        }
    }

    /// Every file under `root`, by path relative to it.
    fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    let name = path.strip_prefix(root).expect("below root");
                    out.insert(
                        name.to_string_lossy().into_owned(),
                        std::fs::read(&path).expect("reads"),
                    );
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    fn manifest(root: &Path, done: &Exported) -> Value {
        serde_json::from_slice(&std::fs::read(root.join(&done.manifest)).expect("manifest"))
            .expect("JSON")
    }

    /// The rows of every file a manifest names, with the column values.
    fn rows_of(root: &Path, done: &Exported) -> Vec<BTreeMap<String, Field>> {
        done.files
            .iter()
            .flat_map(|file| {
                columns::tests::read(&std::fs::read(root.join(&file.path)).expect("a file"))
            })
            .map(|row| row.into_iter().collect())
            .collect()
    }

    fn bytes_of(field: &Field) -> Vec<u8> {
        match field {
            Field::Bytes(b) => b.data().to_vec(),
            Field::Str(s) => s.clone().into_bytes(),
            other => panic!("not bytes: {other:?}"),
        }
    }

    fn version_of(row: &BTreeMap<String, Field>) -> VersionId {
        VersionId(bytes_of(&row["version"]).try_into().expect("16 bytes"))
    }

    fn run(f: &Fixture, level: Metadata, root: &Path) -> Exported {
        let target = DirTarget::new(root, Arc::new(OsFs));
        export(&f.store, &plan(level), &target, f.clock.now()).expect("exports")
    }

    /// 3.1 and 3.2: each kept version, the hidden one among them, is one
    /// row whose `sealed` opens to the body a legal read returns, byte for
    /// byte, for every form plain and gzipped.
    #[test]
    fn every_kept_version_opens_to_its_body_byte_for_byte() {
        let f = fixture();
        let kept = stored(&f);
        assert_eq!(kept.len(), 8, "seven vCons and one hidden version");
        let out = tempfile::tempdir().expect("temp dir");
        let done = run(&f, Metadata::None, out.path());
        let rows = rows_of(out.path(), &done);
        assert_eq!(rows.len(), kept.len(), "one row per kept version");
        assert_eq!(done.rows, kept.len() as u64, "rows reported");
        for row in &rows {
            let version = version_of(row);
            let k = kept
                .iter()
                .find(|k| k.version == version)
                .expect("a kept version");
            let body = f
                .store
                .open_sealed(k, &bytes_of(&row["sealed"]))
                .expect("opens");
            let read = f
                .store
                .legal_get(&f.caller, k.uuid, Some(version))
                .expect("legal read");
            assert_eq!(body, read.stored.body, "{version}: byte for byte");
            assert_eq!(row["stored_at"], Field::Long(k.stored_at), "store time");
        }
    }

    /// The manifest names every file with its rows and bytes, the live
    /// versions, the level and the window; files go by store date.
    #[test]
    fn the_manifest_names_each_file_and_the_live_versions() {
        let f = fixture();
        let kept = stored(&f);
        let out = tempfile::tempdir().expect("temp dir");
        let done = run(&f, Metadata::None, out.path());
        let tenant = f.store.tenant();
        assert_eq!(done.seq, 1, "the first export");
        assert_eq!(
            done.manifest,
            format!("tenant={tenant}/manifest/00000000000000000001.json"),
            "the manifest's name holds the tenant and the number"
        );
        let m = manifest(out.path(), &done);
        assert_eq!(m["format"], MANIFEST_FORMAT, "{m}");
        assert_eq!(
            m["tenant"],
            tenant.to_string(),
            "the manifest names its tenant"
        );
        assert_eq!(m["seq"], 1, "the manifest's number");
        assert_eq!(m["metadata"], "none", "the manifest records the level");
        assert_eq!(m["since"], Value::Null, "no --since");
        assert_eq!(m["until"], Value::Null, "no --until");
        assert_eq!(
            m["rows"],
            kept.len(),
            "the manifest counts every kept version"
        );
        let named: Vec<FileEntry> =
            serde_json::from_value(m["files"].clone()).expect("a list of files");
        assert_eq!(named, done.files, "the files the run reports");
        let on_disk = files(out.path());
        let mut parquet: Vec<&String> =
            on_disk.keys().filter(|p| p.ends_with(".parquet")).collect();
        parquet.sort();
        let mut listed: Vec<&String> = named.iter().map(|e| &e.path).collect();
        listed.sort();
        assert_eq!(parquet, listed, "every file on disk is named, and no other");
        for entry in &named {
            assert_eq!(
                on_disk[&entry.path].len() as u64,
                entry.bytes,
                "{}",
                entry.path
            );
            assert_eq!(
                columns::tests::read(&on_disk[&entry.path]).len() as u64,
                entry.rows,
                "{}",
                entry.path
            );
        }
        let dates: BTreeSet<&str> = named
            .iter()
            .map(|e| e.path.split('/').nth(1).expect("a date directory"))
            .collect();
        assert_eq!(
            dates,
            BTreeSet::from(["date=2026-10-23", "date=2026-10-24"]),
            "one directory per store date"
        );
        let live: BTreeSet<String> = kept
            .iter()
            .filter(|k| k.live)
            .map(|k| k.version.to_string())
            .collect();
        let manifest_live: BTreeSet<String> =
            serde_json::from_value(m["live"].clone()).expect("a list of versions");
        assert_eq!(manifest_live, live, "the live versions");
        assert_eq!(live.len(), 7, "the hidden version is not live");
        assert!(
            !on_disk.keys().any(|p| p.ends_with(".tmp")),
            "no temporary name is left"
        );
    }

    /// 3.3 and 3.4: no exported byte, in any file or the manifest, holds a
    /// uuid, a subject, or a party's number, address or name at level
    /// `none`, nor at `counts`; at `full` they are there, which shows the
    /// scan finds them. A purged vCon's are in no file at any level.
    #[test]
    fn no_level_below_full_holds_a_uuid_subject_or_party() {
        let f = fixture();
        stored(&f);
        for level in [Metadata::None, Metadata::Counts, Metadata::Full] {
            let out = tempfile::tempdir().expect("temp dir");
            run(&f, level, out.path());
            let all: Vec<u8> = files(out.path()).into_values().flatten().collect();
            let holds = |needle: &[u8]| all.windows(needle.len()).any(|w| w == needle);
            for n in [1, 2, 3, 4, 8] {
                let u: Uuid = uuid(n).parse().expect("uuid");
                assert!(
                    !holds(uuid(n).as_bytes()),
                    "{level:?}, vCon {n}: a uuid is never written as text"
                );
                let words: [Vec<u8>; 5] = [
                    u.as_bytes().to_vec(),
                    subject(n).into_bytes(),
                    format!("NAME-MARKER-{n:02}").into_bytes(),
                    format!("party{n}@example.com").into_bytes(),
                    format!("+1555555{:04}", 100 + u16::from(n)).into_bytes(),
                ];
                for word in &words {
                    assert_eq!(
                        holds(word),
                        level == Metadata::Full,
                        "{level:?}, vCon {n}: {}",
                        String::from_utf8_lossy(word)
                    );
                }
            }
            for word in [
                uuid(9).into_bytes(),
                subject(9).into_bytes(),
                "party9@example.com".into(),
            ] {
                assert!(!holds(&word), "{level:?}: the purged vCon is absent");
            }
        }
    }

    /// Each file is written at the tenant's level: its columns are that
    /// level's and no more, and its metadata records the level.
    #[test]
    fn each_file_is_written_at_the_tenants_level() {
        let f = fixture();
        stored(&f);
        for level in [Metadata::None, Metadata::Counts, Metadata::Full] {
            let out = tempfile::tempdir().expect("temp dir");
            let done = run(&f, level, out.path());
            let expected = columns::schema(level);
            for entry in &done.files {
                let bytes = std::fs::read(out.path().join(&entry.path)).expect("a file");
                let rows = columns::tests::read(&bytes);
                let names: Vec<&str> = rows[0].iter().map(|(n, _)| n.as_str()).collect();
                let wanted: Vec<&str> = expected
                    .lines()
                    .filter(|l| l.trim_end().ends_with(';'))
                    .filter_map(|l| l.split_whitespace().nth(2))
                    .map(|n| n.trim_end_matches(';'))
                    .collect();
                assert_eq!(names, wanted, "{level:?}: {}", entry.path);
                assert!(
                    columns::tests::key_values(&bytes).contains(&(
                        columns::LEVEL_KEY.to_owned(),
                        Some(level.as_str().to_owned())
                    )),
                    "{level:?}: the level is recorded"
                );
            }
        }
    }

    /// The columns of `counts` and `full`, from the catalog row and the
    /// body; NULL where the body cannot show them, as for an encrypted
    /// vCon, whose uuid alone is known.
    #[test]
    fn each_level_fills_its_columns() {
        let f = fixture();
        let kept = stored(&f);
        let out = tempfile::tempdir().expect("temp dir");
        let done = run(&f, Metadata::Full, out.path());
        let created = nanos(time::macros::datetime!(2026-10-23 12:00 UTC));
        for row in rows_of(out.path(), &done) {
            let k = kept
                .iter()
                .find(|k| k.version == version_of(&row))
                .expect("kept");
            assert_eq!(
                bytes_of(&row["form"]),
                k.form.as_bytes(),
                "form, from the catalog row"
            );
            assert_eq!(
                row["gzip"],
                Field::Bool(k.gzip),
                "gzip, from the catalog row"
            );
            assert_eq!(
                row["size"],
                Field::Long(k.size),
                "size, from the catalog row"
            );
            assert_eq!(bytes_of(&row["uuid"]), k.uuid.as_bytes(), "the uuid");
            if k.form == "encrypted" {
                for name in [
                    "created_at",
                    "updated_at",
                    "party_count",
                    "dialog_count",
                    "subject",
                    "parties",
                ] {
                    assert_eq!(row[name], Field::Null, "{name} of an encrypted vCon");
                }
                continue;
            }
            assert_eq!(row["created_at"], Field::Long(created), "created_at");
            assert_eq!(row["updated_at"], Field::Null, "no updated_at");
            assert_eq!(
                row["party_count"],
                Field::Int(2),
                "two parties in each test vCon"
            );
            assert_eq!(
                row["dialog_count"],
                Field::Int(1),
                "one dialog in each test vCon"
            );
            let n = k.uuid.as_bytes()[15];
            assert_eq!(
                bytes_of(&row["subject"]),
                subject(n).into_bytes(),
                "the subject, at full"
            );
            let parties: Value = serde_json::from_slice(&bytes_of(&row["parties"])).expect("JSON");
            assert_eq!(
                parties,
                json!([
                    {"tel": format!("+1555555{:04}", 100 + u16::from(n)), "name": format!("NAME-MARKER-{n:02}")},
                    {"mailto": format!("party{n}@example.com")}
                ]),
                "each party's members, missing ones left out"
            );
        }
        assert_eq!(done.unread, 0, "every body read");
    }

    /// 3.5: a sealed value opens with its own version's key and context
    /// wherever it sits, and with no other version's.
    #[test]
    fn a_sealed_value_opens_only_as_its_own_version() {
        let f = fixture();
        let kept = stored(&f);
        let out = tempfile::tempdir().expect("temp dir");
        let done = run(&f, Metadata::None, out.path());
        let rows = rows_of(out.path(), &done);
        let mut moved = 0;
        for row in &rows {
            let sealed = bytes_of(&row["sealed"]);
            for k in &kept {
                let opened = f.store.open_sealed(k, &sealed);
                if k.version == version_of(row) {
                    assert!(opened.is_ok(), "its own version opens it");
                } else {
                    assert_eq!(
                        opened,
                        Err(StoreError::Damaged),
                        "another version's does not"
                    );
                    moved += 1;
                }
                let mut other_uuid = k.clone();
                other_uuid.uuid = Uuid::from_bytes([0x42; 16]);
                assert_eq!(
                    f.store.open_sealed(&other_uuid, &sealed),
                    Err(StoreError::Damaged),
                    "paired with another uuid it does not open"
                );
            }
        }
        assert_eq!(
            moved,
            rows.len() * (kept.len() - 1),
            "every row was tried with every other version"
        );
    }

    /// The window: only versions stored at or after `since` and before
    /// `until`, and the manifest says which.
    #[test]
    fn the_window_bounds_what_is_exported() {
        let f = fixture();
        let kept = stored(&f);
        let out = tempfile::tempdir().expect("temp dir");
        let target = DirTarget::new(out.path(), Arc::new(OsFs));
        let since = kept[2].stored_at;
        let until = kept[5].stored_at;
        let windowed = Plan {
            since: Some(since),
            until: Some(until),
            ..plan(Metadata::None)
        };
        let done = export(&f.store, &windowed, &target, f.clock.now()).expect("exports");
        let got: BTreeSet<String> = rows_of(out.path(), &done)
            .iter()
            .map(|row| version_of(row).to_string())
            .collect();
        let want: BTreeSet<String> = kept[2..5].iter().map(|k| k.version.to_string()).collect();
        assert_eq!(got, want, "since taken, until left out");
        let m = manifest(out.path(), &done);
        let time = |n| {
            from_nanos(n)
                .expect("a time")
                .format(&Rfc3339)
                .expect("RFC 3339")
        };
        assert_eq!(m["since"], time(since), "the manifest records --since");
        assert_eq!(m["until"], time(until), "the manifest records --until");
    }

    /// Rows go into a new file once one is full; each file reads whole.
    #[test]
    fn a_full_file_is_followed_by_another() {
        let f = fixture();
        let kept = stored(&f);
        let out = tempfile::tempdir().expect("temp dir");
        let target = DirTarget::new(out.path(), Arc::new(OsFs));
        let small = Plan {
            sizes: Sizes {
                row_group: 1,
                file: 1,
            },
            ..plan(Metadata::None)
        };
        let done = export(&f.store, &small, &target, f.clock.now()).expect("exports");
        assert_eq!(done.files.len(), kept.len(), "a row a file");
        assert_eq!(
            rows_of(out.path(), &done).len(),
            kept.len(),
            "every version is in some file"
        );
    }

    /// A second export to the same target takes the next number and names
    /// only its own files; one that finds its manifest name taken is
    /// refused and leaves none of its files.
    #[test]
    fn each_export_takes_the_next_manifest_and_a_taken_one_is_refused() {
        let f = fixture();
        stored(&f);
        let out = tempfile::tempdir().expect("temp dir");
        let first = run(&f, Metadata::None, out.path());
        let second = run(&f, Metadata::None, out.path());
        assert_eq!(
            (first.seq, second.seq),
            (1, 2),
            "each export takes the next number"
        );
        let names = |done: &Exported| -> BTreeSet<String> {
            done.files.iter().map(|e| e.path.clone()).collect()
        };
        assert!(
            names(&first).is_disjoint(&names(&second)),
            "new files each time"
        );
        let before = files(out.path());
        let tenant = f.store.tenant();
        let taken = format!("tenant={tenant}/manifest/00000000000000000003.json");
        // A target that shows no third manifest while one is written, as
        // when two exports list it at once.
        struct Racing<'a>(&'a DirTarget, String);
        impl Target for Racing<'_> {
            fn put_new(&self, path: &str, bytes: &[u8]) -> Result<(), TargetError> {
                if path == self.1 {
                    self.0.put_new(path, b"the other export's")?;
                }
                self.0.put_new(path, bytes)
            }
            fn list(&self, dir: &str) -> Result<Vec<String>, TargetError> {
                self.0.list(dir)
            }
            fn remove(&self, path: &str) -> Result<(), TargetError> {
                self.0.remove(path)
            }
            fn describe(&self) -> String {
                self.0.describe()
            }
        }
        let dir = DirTarget::new(out.path(), Arc::new(OsFs));
        let raced = export(
            &f.store,
            &plan(Metadata::None),
            &Racing(&dir, taken.clone()),
            f.clock.now(),
        );
        assert_eq!(
            raced,
            Err(ExportError::Raced(taken.clone())),
            "the run that lost the name is refused, naming it"
        );
        let mut after = files(out.path());
        assert_eq!(
            after.remove(&taken).as_deref(),
            Some(&b"the other export's"[..]),
            "the other export's manifest stands"
        );
        assert_eq!(after, before, "none of the refused run's files is left");
    }

    /// Each run is recorded in the tenant's log with its outcome, and
    /// nothing else changes in the Store: rows and objects stay as they
    /// were (3.9, as far as the Store goes).
    #[test]
    fn a_run_changes_nothing_but_its_audit_record() {
        let f = fixture();
        stored(&f);
        let rows = |f: &Fixture| -> Vec<Vec<u8>> {
            let conn = f.store.conn_for_test();
            let mut out = Vec::new();
            for sql in [
                "SELECT quote(version) || quote(uuid) || quote(stored_at) || quote(hidden_at) || quote(wrapped_key) FROM version ORDER BY version",
                "SELECT quote(uuid) || quote(current) || quote(deleted_at) || quote(hold) FROM vcon ORDER BY uuid",
                "SELECT quote(uuid) FROM tombstone ORDER BY uuid",
            ] {
                let mut statement = conn.prepare(sql).expect("prepares");
                let found = statement
                    .query_map([], |r| r.get::<_, String>(0))
                    .expect("queries")
                    .map(|r| r.expect("a row").into_bytes());
                out.extend(found);
            }
            out
        };
        let objects = |f: &Fixture| f.fs.list(&f.store_objects_dir()).expect("lists");
        let (before, listed) = (rows(&f), objects(&f));
        let records = f.store.records().len();
        let out = tempfile::tempdir().expect("temp dir");
        run(&f, Metadata::Full, out.path());
        assert_eq!(rows(&f), before, "no row changed");
        assert_eq!(objects(&f), listed, "no object changed");
        let log = f.store.records();
        assert_eq!(log.len(), records + 1, "one record");
        let last = log.last().expect("a record");
        assert_eq!(last["action"], "export", "the record's action");
        assert_eq!(last["status"], 200, "a run that completed");
        assert_eq!(last["actor"], "cli", "the command line ran it");

        let failed = export(
            &f.store,
            &plan(Metadata::None),
            &DirTarget::new(Path::new("/proc/vcon-server-export"), Arc::new(OsFs)),
            f.clock.now(),
        );
        assert!(matches!(failed, Err(ExportError::Target(_))), "{failed:?}");
        let last = f.store.records().last().cloned().expect("a record");
        assert_eq!(
            (last["action"].clone(), last["status"].clone()),
            (json!("export"), json!(500)),
            "a failed run is recorded too"
        );
    }

    /// 3.7: an export cut off at every filesystem step, with a crash
    /// after, leaves no manifest that names a missing or partial file, and
    /// no file under its own name that is not whole. A run that fails
    /// without a crash removes its files; a crash may leave whole files no
    /// manifest names, which a reader going by the manifest never reads.
    #[test]
    fn an_export_cut_off_at_any_step_leaves_no_manifest_naming_a_partial_file() {
        let f = fixture();
        stored(&f);
        let small = Plan {
            sizes: Sizes {
                row_group: 1,
                file: 400,
            },
            ..plan(Metadata::Counts)
        };
        let steps = {
            let root = tempfile::tempdir().expect("temp dir");
            let fs = Arc::new(CrashFs::new(root.path()));
            let target = DirTarget::new(&root.path().join("out"), fs.clone());
            export(&f.store, &small, &target, f.clock.now()).expect("exports");
            fs.log().len()
        };
        assert!(steps > 10, "a run takes {steps} steps");
        for step in 0..steps {
            let root = tempfile::tempdir().expect("temp dir");
            let fs = Arc::new(CrashFs::new(root.path()));
            let out = root.path().join("out");
            let target = DirTarget::new(&out, fs.clone());
            fs.fail_from(step);
            assert!(
                export(&f.store, &small, &target, f.clock.now()).is_err(),
                "step {step} of {steps} fails"
            );
            fs.crash();
            check_whole(&*fs, &out, step);
        }
    }

    /// Every manifest under `out` names only whole files, and every
    /// `.parquet` name is a whole file.
    fn check_whole(fs: &dyn Fs, out: &Path, step: usize) {
        let tenant_dirs = fs.list(out).unwrap_or_default();
        for tenant in tenant_dirs {
            let dir = out.join(&tenant);
            for manifest in fs.list(&dir.join("manifest")).unwrap_or_default() {
                let path = dir.join("manifest").join(&manifest);
                let bytes = fs.read(&path).expect("reads");
                let m: Value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                    panic!("step {step}: {} is not whole: {e}", path.display())
                });
                let named: Vec<FileEntry> =
                    serde_json::from_value(m["files"].clone()).expect("files");
                for entry in named {
                    let file = fs
                        .read(&out.join(&entry.path))
                        .unwrap_or_else(|_| panic!("step {step}: {} is missing", entry.path));
                    assert_eq!(
                        file.len() as u64,
                        entry.bytes,
                        "step {step}: {}",
                        entry.path
                    );
                    assert_eq!(
                        columns::tests::read(&file).len() as u64,
                        entry.rows,
                        "the file holds the rows its manifest says"
                    );
                }
            }
            for date in fs.list(&dir).unwrap_or_default() {
                let date_dir = dir.join(&date);
                for name in fs.list(&date_dir).unwrap_or_default() {
                    let name = name.to_string_lossy().into_owned();
                    if name.ends_with(".parquet") {
                        let file = fs.read(&date_dir.join(&name)).expect("reads");
                        assert!(
                            file.starts_with(b"PAR1") && file.ends_with(b"PAR1"),
                            "step {step}: {name} is whole"
                        );
                    }
                }
            }
        }
    }

    /// 3.7, inside a file: output cut off at 0 bytes, 4, a third, a half
    /// and one short of whole leaves no file under its own name and no
    /// manifest.
    #[test]
    fn a_file_cut_off_part_way_leaves_nothing_under_its_name() {
        /// The OS filesystem, but every write stops after `cap` bytes and
        /// fails.
        struct Capped(usize);
        impl Fs for Capped {
            fn write_new(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
                if data.len() <= self.0 {
                    return OsFs.write_new(path, data);
                }
                OsFs.write_new(path, &data[..self.0])?;
                Err(std::io::Error::other("the disk is full"))
            }
            fn sync_file(&self, path: &Path) -> std::io::Result<()> {
                OsFs.sync_file(path)
            }
            fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
                OsFs.rename(from, to)
            }
            fn rename_new(&self, from: &Path, to: &Path) -> std::io::Result<()> {
                OsFs.rename_new(from, to)
            }
            fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
                OsFs.sync_dir(dir)
            }
            fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
                OsFs.read(path)
            }
            fn remove(&self, path: &Path) -> std::io::Result<()> {
                OsFs.remove(path)
            }
            fn list(&self, dir: &Path) -> std::io::Result<Vec<std::ffi::OsString>> {
                OsFs.list(dir)
            }
            fn create_dir_all(&self, dir: &Path) -> std::io::Result<()> {
                OsFs.create_dir_all(dir)
            }
            fn remove_dir_all(&self, dir: &Path) -> std::io::Result<()> {
                OsFs.remove_dir_all(dir)
            }
        }
        let f = fixture();
        stored(&f);
        let whole = {
            let out = tempfile::tempdir().expect("temp dir");
            let done = run(&f, Metadata::None, out.path());
            done.files[0].bytes as usize
        };
        for cap in [0, 4, whole / 3, whole / 2, whole - 1] {
            let out = tempfile::tempdir().expect("temp dir");
            let target = DirTarget::new(out.path(), Arc::new(Capped(cap)));
            let failed = export(&f.store, &plan(Metadata::None), &target, f.clock.now());
            assert!(
                matches!(failed, Err(ExportError::Target(_))),
                "{cap}: {failed:?}"
            );
            let left: Vec<PathBuf> = files(out.path()).keys().map(PathBuf::from).collect();
            assert!(left.is_empty(), "cap {cap} of {whole}: {left:?} left");
        }
    }

    /// 3.6: DuckDB, the CLI `VCON_TEST_DUCKDB` names (CI pins v1.5.6 by
    /// its SHA-256), reads an export as the how-to page shows: by a Hive
    /// glob and through the newest manifest. Built only with the
    /// `backend-tests` feature; without the variable every test fails, so
    /// a run without DuckDB never passes by skipping.
    #[cfg(feature = "backend-tests")]
    pub(crate) mod duckdb {
        use super::*;

        /// Runs `sql` in DuckDB's CLI, in memory, and parses its JSON rows.
        pub(crate) fn query(sql: &str) -> Vec<Value> {
            let duckdb = std::env::var("VCON_TEST_DUCKDB").unwrap_or_else(|_| {
                panic!("VCON_TEST_DUCKDB is not set: these tests need the DuckDB CLI")
            });
            let out = std::process::Command::new(&duckdb)
                .args(["-json", "-c", sql])
                .output()
                .unwrap_or_else(|e| panic!("{duckdb} does not run: {e}"));
            assert!(
                out.status.success(),
                "{sql}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let text = String::from_utf8(out.stdout).expect("UTF-8");
            if text.trim().is_empty() {
                return Vec::new();
            }
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
        }

        fn glob(root: &Path) -> String {
            format!(
                "read_parquet('{}/tenant=*/date=*/*.parquet', hive_partitioning = true)",
                root.display()
            )
        }

        /// Every kept version is a row; the version reads as a UUID, the
        /// store time as a timestamp equal to the catalog's (DuckDB keeps
        /// microseconds; the test store times are whole seconds), the
        /// partitions as a DATE and the tenant id.
        #[test]
        fn duckdb_reads_every_kept_version_with_its_types() {
            let f = fixture();
            let kept = stored(&f);
            let out = tempfile::tempdir().expect("temp dir");
            let done = run(&f, Metadata::Counts, out.path());
            let rows = query(&format!(
                "SELECT version::VARCHAR AS version, typeof(version) AS vt, \
                 epoch_ns(stored_at) AS ns, typeof(stored_at) AS st, \
                 typeof(date) AS dt, date::VARCHAR AS date, tenant, octet_length(sealed) AS n, \
                 form, gzip, size FROM {} ORDER BY stored_at, version",
                glob(out.path())
            ));
            assert_eq!(rows.len(), kept.len(), "one row per kept version");
            assert_eq!(
                done.rows as usize,
                kept.len(),
                "the run counts every kept version"
            );
            for (row, k) in rows.iter().zip(&kept) {
                let uuid = Uuid::from_bytes(k.version.0).to_string();
                assert_eq!(row["version"], uuid.as_str(), "the version id as a UUID");
                assert_eq!(row["vt"], "UUID", "the version id reads as a UUID");
                assert_eq!(row["ns"], k.stored_at, "the store time, in its unit");
                assert!(
                    row["st"]
                        .as_str()
                        .is_some_and(|t| t.starts_with("TIMESTAMP")),
                    "{}",
                    row["st"]
                );
                assert_eq!(row["dt"], "DATE", "the date partition");
                assert_eq!(
                    row["date"],
                    date_of(k.stored_at).expect("a date").as_str(),
                    "the date partition is the UTC store date"
                );
                assert_eq!(
                    row["tenant"],
                    f.store.tenant().to_string(),
                    "the tenant partition"
                );
                let sealed = f.store.sealed_of(k).expect("reads").expect("kept");
                assert_eq!(row["n"], sealed.len(), "the sealed body's length");
                assert_eq!(row["form"], k.form.as_str(), "form");
                assert_eq!(row["gzip"], k.gzip, "gzip");
                assert_eq!(row["size"], k.size, "size");
            }
            let bytes = query(&format!(
                "SELECT hex(sealed) AS h FROM {} ORDER BY stored_at, version",
                glob(out.path())
            ));
            for (row, k) in bytes.iter().zip(&kept) {
                let sealed = f.store.sealed_of(k).expect("reads").expect("kept");
                let hex: String = sealed.iter().map(|b| format!("{b:02X}")).collect();
                assert_eq!(row["h"], hex.as_str(), "sealed, byte for byte");
            }
        }

        /// The how-to's query: the newest manifest's file list, read with
        /// SET VARIABLE, counts the rows the manifest names; after a second
        /// export the glob counts both, the manifest one.
        #[test]
        fn duckdb_reads_the_files_the_newest_manifest_names() {
            let f = fixture();
            let kept = stored(&f);
            let out = tempfile::tempdir().expect("temp dir");
            run(&f, Metadata::None, out.path());
            let done = run(&f, Metadata::None, out.path());
            let root = out.path().display();
            let sql = format!(
                "SET VARIABLE files = (SELECT list('{root}/' || f.path) FROM \
                 (SELECT unnest(files) AS f FROM read_json('{root}/{}'))); \
                 SELECT count(*) AS n FROM read_parquet(getvariable('files'));",
                done.manifest
            );
            let rows = query(&sql);
            assert_eq!(rows.last().expect("a row")["n"], kept.len(), "{rows:?}");
            let globbed = query(&format!("SELECT count(*) AS n FROM {}", glob(out.path())));
            assert_eq!(
                globbed[0]["n"],
                2 * kept.len(),
                "a glob counts both exports"
            );
        }
    }
}
