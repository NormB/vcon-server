//! An SQLite catalog, encrypted with SQLCipher under its catalog key.

use std::ffi::c_int;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, ErrorCode};

use crate::envelope::Key;

/// The schema version this build makes and reads. Version 2 added the
/// time each uuid was first stored to a tenant catalog's vcon table.
pub const SCHEMA_VERSION: i64 = 2;

/// Brings a version-1 tenant catalog to version 2: each uuid's first store
/// time, taken from its earliest kept version (the closest record left of
/// it), and the index the list reads it by. The column is declared as a
/// new catalog declares it.
const VCON_STORED_AT: &str = "
ALTER TABLE vcon ADD COLUMN stored_at INTEGER NOT NULL DEFAULT 0;
UPDATE vcon SET stored_at =
    coalesce((SELECT min(stored_at) FROM version WHERE version.uuid = vcon.uuid), 0);
CREATE INDEX vcon_listed ON vcon (stored_at, uuid);
";

/// How long a statement waits for another connection's lock before it
/// fails busy. It is rusqlite's own default, stated so it is chosen, not
/// inherited; a caller that must not wait inside SQLite sets zero for the
/// call and restores this.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a catalog did not open.
#[derive(Debug)]
pub enum CatalogError {
    /// The key is not the one the catalog was made with.
    WrongKey,
    /// The catalog's schema version is not one this build knows.
    UnknownSchema(i64),
    /// SQLite failed.
    Sqlite(rusqlite::Error),
    /// A file around the catalog could not be read, written or renamed.
    Io(std::io::Error),
    /// This process may not read and write `path`, the catalog or the
    /// directory that holds it (where SQLite makes its `-wal` and `-shm`
    /// files), such as when another user owns it.
    Access {
        path: PathBuf,
        error: std::io::Error,
    },
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogError::WrongKey => {
                f.write_str("the key is not the one the catalog was made with")
            }
            CatalogError::UnknownSchema(version) => {
                write!(f, "schema version {version} is not one this build knows")
            }
            CatalogError::Sqlite(_) => f.write_str("sqlite failed"),
            CatalogError::Io(_) => {
                f.write_str("a file around the catalog could not be read, written or renamed")
            }
            CatalogError::Access { path, .. } => {
                write!(f, "this process may not read and write {}", path.display())
            }
        }
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CatalogError::Sqlite(e) => Some(e),
            CatalogError::Io(e) | CatalogError::Access { error: e, .. } => Some(e),
            CatalogError::WrongKey | CatalogError::UnknownSchema(_) => None,
        }
    }
}

impl From<rusqlite::Error> for CatalogError {
    fn from(e: rusqlite::Error) -> CatalogError {
        CatalogError::Sqlite(e)
    }
}

/// Opens the catalog at `path` under `key`, making it with `schema` if it
/// is new.
pub fn open(path: &Path, key: &Key, schema: &str) -> Result<Connection, CatalogError> {
    // SQLite opens a file it may not write read-only and fails later with
    // its own code, which no longer says why; asking the system first
    // gives the path and its error.
    if let Some(dir) = path.parent() {
        may_use(dir, libc::W_OK | libc::X_OK)?;
    }
    if path.exists() {
        may_use(path, libc::R_OK | libc::W_OK)?;
    } else {
        make_file(path)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    // SQLCipher otherwise writes its own error lines to standard error, a
    // wrong key among them; this program says what went wrong itself.
    // SQLCipher's settings are not in SQLite's pragma list, which rusqlite's
    // pragma helpers check names against, so they go as plain SQL.
    conn.execute_batch("PRAGMA cipher_log_level = NONE")?;
    set_key(&conn, key)?;
    // SQLCipher reads nothing until the first query, so a wrong key shows
    // here, as a file that is not a database.
    match conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(())) {
        Err(e) if e.sqlite_error_code() == Some(ErrorCode::NotADatabase) => {
            return Err(CatalogError::WrongKey);
        }
        other => other?,
    }
    // Each setting is stated even where this build's default already
    // matches (synchronous and secure_delete do), so a change of build
    // flags cannot drop one; the tests read each back.
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    debug_assert_eq!(mode, "wal");
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "secure_delete", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    match conn.pragma_query_value(None, "user_version", |r| r.get(0))? {
        0 => {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(schema)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }
        1 => {
            // One transaction, so a failed upgrade leaves version 1 whole.
            let tx = conn.unchecked_transaction()?;
            let tenant: bool = tx.query_row(
                "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'vcon'",
                [],
                |r| r.get(0),
            )?;
            if tenant {
                tx.execute_batch(VCON_STORED_AT)?;
            }
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }
        SCHEMA_VERSION => {}
        other => return Err(CatalogError::UnknownSchema(other)),
    }
    Ok(conn)
}

/// Whether this process, by its effective ids, may use `path` as `how`
/// asks (`R_OK`, `W_OK`, `X_OK`); if not, the path and the system's error.
pub(crate) fn may_use(path: &Path, how: c_int) -> Result<(), CatalogError> {
    let denied = |error| CatalogError::Access {
        path: path.to_owned(),
        error,
    };
    let text = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| denied(std::io::ErrorKind::InvalidInput.into()))?;
    // SAFETY: `text` is a NUL-terminated path that outlives the call, and
    // faccessat reads only it and the flags.
    if unsafe { libc::faccessat(libc::AT_FDCWD, text.as_ptr(), how, libc::AT_EACCESS) } != 0 {
        return Err(denied(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Copies the catalog `source` is open on to a new file at `to`, kept
/// under `key`, which must be the key `source` was opened with. It uses
/// SQLite's online backup API and copies every page in one step, so the
/// copy is the catalog as one commit left it, the WAL's committed frames
/// included, while other connections go on writing. `to` must not exist:
/// it is made empty first, so a file already there is never written over.
/// The copy and its directory are synced.
pub fn backup(source: &Connection, to: &Path, key: &Key) -> Result<(), CatalogError> {
    let io = CatalogError::Io;
    make_file(to)?;
    {
        let mut copy = Connection::open(to)?;
        copy.execute_batch("PRAGMA cipher_log_level = NONE")?;
        set_key(&copy, key)?;
        let backup = rusqlite::backup::Backup::new(source, &mut copy)?;
        // A negative count copies every remaining page in this one step.
        match backup.step(-1)? {
            rusqlite::backup::StepResult::Done => {}
            _ => {
                return Err(io(std::io::Error::other(
                    "the catalog was busy and could not be copied in one step",
                )));
            }
        }
    }
    std::fs::File::open(to)
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    let dir = to.parent().unwrap_or(Path::new("."));
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    Ok(())
}

/// Re-encrypts the catalog at `path`, kept under `old`, so that it is kept
/// under `new`: every row, the schema and its version are written to a new
/// file under `new`, which then replaces the old one by rename. A crash
/// leaves either the whole old file or the whole new one, and nothing of
/// the old file's pages is carried into the new one. A file left by an
/// interrupted run is replaced.
pub fn reencrypt(path: &Path, old: &Key, new: &Key) -> Result<(), CatalogError> {
    let copy = copy_under(path, old, new)?;
    replace(path, &copy)
}

/// Writes every row, the schema and its version of the catalog at `path`,
/// kept under `old`, to a new file beside it kept under `new`, and returns
/// that file's path. The old file is only read. A file left there by an
/// interrupted run is replaced.
pub fn copy_under(path: &Path, old: &Key, new: &Key) -> Result<PathBuf, CatalogError> {
    let target = sibling(path, ".rekey");
    for stale in [
        target.clone(),
        sibling(&target, "-wal"),
        sibling(&target, "-shm"),
    ] {
        remove_if_there(&stale)?;
    }
    make_file(&target)?;
    let conn = open(path, old, "")?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let target_text = target
        .to_str()
        .ok_or_else(|| CatalogError::Io(std::io::Error::other("the catalog path is not UTF-8")))?;
    conn.execute("ATTACH DATABASE ?1 AS rekeyed", [target_text])?;
    set_key_of(&conn, c"rekeyed", new)?;
    conn.query_row("SELECT sqlcipher_export('rekeyed')", [], |_| Ok(()))?;
    // A pragma takes no bound parameter; the version is a number read
    // from the catalog itself.
    conn.execute_batch(&format!("PRAGMA rekeyed.user_version = {version}"))?;
    conn.execute("DETACH DATABASE rekeyed", [])?;
    Ok(target)
}

/// Makes `copy` the catalog at `path`: it is synced, renamed over `path`,
/// and the directory synced. Every connection to either file must be
/// closed. Their WAL files must be empty, as a last close leaves them; one
/// still holding frames would sit beside the wrong file, so it is refused.
pub fn replace(path: &Path, copy: &Path) -> Result<(), CatalogError> {
    for file in [path, copy] {
        let wal = sibling(file, "-wal");
        if std::fs::metadata(&wal).is_ok_and(|m| m.len() > 0) {
            return Err(CatalogError::Io(std::io::Error::other(
                "a catalog's WAL was not emptied",
            )));
        }
        remove_if_there(&wal)?;
        remove_if_there(&sibling(file, "-shm"))?;
    }
    let io = CatalogError::Io;
    std::fs::File::open(copy)
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    std::fs::rename(copy, path).map_err(io)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    Ok(())
}

/// Makes an empty file at `path`, which must not exist, with the server's
/// file mode: SQLite then opens it as a new database, and makes its WAL
/// and shared-memory files with the same mode.
fn make_file(path: &Path) -> Result<(), CatalogError> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(crate::fs::FILE_MODE)
        .open(path)
        .map(drop)
        .map_err(CatalogError::Io)
}

fn remove_if_there(file: &Path) -> Result<(), CatalogError> {
    match std::fs::remove_file(file) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(CatalogError::Io(e)),
        _ => Ok(()),
    }
}

/// `path` with `suffix` added to its file name.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Keys the attached database `schema` as `set_key` keys the main one.
fn set_key_of(conn: &Connection, schema: &std::ffi::CStr, key: &Key) -> Result<(), CatalogError> {
    let literal = key.sqlcipher_raw();
    let len = c_int::try_from(literal.len()).expect("a 67-byte literal");
    // SAFETY: the handle is this open connection's; the name is a valid C
    // string and the key pointer and length are the literal's, both of
    // which outlive the call.
    let rc = unsafe {
        rusqlite::ffi::sqlite3_key_v2(conn.handle(), schema.as_ptr(), literal.as_ptr().cast(), len)
    };
    if rc != rusqlite::ffi::SQLITE_OK {
        return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rc), None).into());
    }
    Ok(())
}

/// Gives SQLCipher the key as a raw key, never a passphrase. sqlite3_key
/// takes the literal straight from its zeroing buffer; a PRAGMA would copy
/// it into SQL text that is not zeroed.
fn set_key(conn: &Connection, key: &Key) -> Result<(), CatalogError> {
    let literal = key.sqlcipher_raw();
    let len = c_int::try_from(literal.len()).expect("a 67-byte literal");
    // SAFETY: the handle is this open connection's, and the pointer and
    // length are the literal's, which outlives the call.
    let rc = unsafe { rusqlite::ffi::sqlite3_key(conn.handle(), literal.as_ptr().cast(), len) };
    // sqlite3_key does not fail on an open connection, so no test reaches
    // this check; it stays to fail closed.
    if rc != rusqlite::ffi::SQLITE_OK {
        return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rc), None).into());
    }
    Ok(())
}

/// Checks on a catalog's schema, for tests.
#[cfg(test)]
pub(crate) mod testing {
    use rusqlite::Connection;

    /// The columns of `table` that take NULL. `row` gives a value for each
    /// column; each column is tried as NULL in that row, and nothing is
    /// kept. The whole row must insert, so a refusal is the column's.
    pub(crate) fn columns_taking_null(
        conn: &Connection,
        table: &str,
        row: &[(&str, &str)],
    ) -> Vec<String> {
        let insert = |empty: Option<&str>| {
            let columns: Vec<&str> = row.iter().map(|(c, _)| *c).collect();
            let values: Vec<&str> = row
                .iter()
                .map(|(c, v)| if Some(*c) == empty { "NULL" } else { *v })
                .collect();
            let sql = format!(
                "INSERT INTO {table} ({}) VALUES ({})",
                columns.join(", "),
                values.join(", ")
            );
            let tx = conn.unchecked_transaction().expect("begin");
            let taken = tx.execute(&sql, []).is_ok();
            drop(tx);
            taken
        };
        assert!(insert(None), "a whole {table} row inserts");
        row.iter()
            .filter(|(column, _)| insert(Some(column)))
            .map(|(column, _)| format!("{table}.{column}"))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const SCHEMA: &str = "CREATE TABLE note (text TEXT NOT NULL);";
    const MARKER: &str = "plain-record-marker-5f3c9a";

    fn path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("catalog.sqlite")
    }

    /// Reads one of SQLCipher's own settings, which rusqlite's pragma
    /// helpers refuse by name.
    pub(crate) fn cipher_setting(conn: &Connection, name: &str) -> String {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
            .expect(name)
    }

    #[test]
    fn a_catalog_runs_with_the_required_settings() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
        let text = |p: &str| -> String { conn.pragma_query_value(None, p, |r| r.get(0)).expect(p) };
        let number = |p: &str| -> i64 { conn.pragma_query_value(None, p, |r| r.get(0)).expect(p) };
        assert_eq!(text("journal_mode"), "wal");
        assert_eq!(number("synchronous"), 2, "FULL");
        assert_eq!(number("secure_delete"), 1);
        assert_eq!(number("temp_store"), 2, "MEMORY");
        assert_eq!(number("busy_timeout"), 5000, "milliseconds");
        assert_eq!(cipher_setting(&conn, "cipher_provider"), "openssl");
        assert_eq!(
            cipher_setting(&conn, "cipher_log_level"),
            "NONE",
            "SQLCipher writes no log of its own"
        );
    }

    /// The SQLCipher settings a catalog runs with, read back through
    /// rusqlite's pragma helpers on a second connection to the same file,
    /// equal what the plain SQL that `open` and `cipher_setting` use reads.
    /// crates.io rusqlite 0.41.0 refuses these names in its helpers, so
    /// this runs on the fork that accepts them; the values must not differ
    /// between the two ways of setting and reading them.
    #[test]
    fn sqlcipher_settings_read_the_same_through_the_pragma_helpers() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
        let names = ["cipher_log_level", "cipher_provider", "cipher_page_size"];
        let plain: Vec<String> = names.iter().map(|n| cipher_setting(&conn, n)).collect();

        let helpers = rusqlite_fork::Connection::open(path(&dir)).expect("opens again");
        let literal = key.sqlcipher_raw();
        let len = c_int::try_from(literal.len()).expect("a 67-byte literal");
        // SAFETY: the handle is this open connection's, and the pointer and
        // length are the literal's, which outlives the call.
        let rc = unsafe {
            rusqlite_fork::ffi::sqlite3_key(helpers.handle(), literal.as_ptr().cast(), len)
        };
        assert_eq!(rc, rusqlite_fork::ffi::SQLITE_OK);
        // Keyed first: the helper reads SQLite's pragma list, which an
        // encrypted file without its key cannot answer.
        helpers
            .pragma_update(None, "cipher_log_level", "NONE")
            .expect("cipher_log_level through pragma_update");
        let notes: i64 = helpers
            .query_row("SELECT count(*) FROM note", [], |r| r.get(0))
            .expect("the key opens the same catalog");
        assert_eq!(notes, 0);
        let read: Vec<String> = names
            .iter()
            .map(|n| {
                helpers
                    .pragma_query_value(None, n, |r| r.get(0))
                    .unwrap_or_else(|e| panic!("{n} through pragma_query_value: {e}"))
            })
            .collect();
        assert_eq!(read, plain);
        assert_eq!(plain[0], "NONE");
    }

    /// The server's rusqlite still refuses SQLCipher's pragma names in its
    /// helpers. When a rusqlite release accepts them, this fails: the plain
    /// SQL in `open` and `cipher_setting`, the rusqlite fork and its patch
    /// are then no longer needed.
    #[test]
    fn crates_io_rusqlite_refuses_sqlcipher_names() {
        let conn = Connection::open_in_memory().expect("opens");
        let refused = conn.pragma_update(None, "cipher_log_level", "NONE");
        assert!(
            matches!(&refused, Err(rusqlite::Error::SqliteFailure(_, Some(m)))
                if m == "Invalid pragma \"cipher_log_level\""),
            "this rusqlite accepts SQLCipher's pragma names ({refused:?}): set them with \
             its pragma helpers in catalog::open and cipher_setting, and remove the \
             rusqlite_fork dev-dependency and its patch (see the Development page)"
        );
    }

    /// The rusqlite fork is a test-only dependency, and the Development and
    /// Getting started pages say so with the commit Cargo.toml pins.
    mod fork_docs {
        use std::path::PathBuf;

        const FORK: &str = "https://github.com/NormB/rusqlite";

        fn read(relative: &str) -> String {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        }

        fn table(text: &str) -> toml::Table {
            text.parse().expect("toml")
        }

        /// The commit the dev-dependency pins, after checking that only the
        /// tests take rusqlite from the fork.
        fn pinned() -> String {
            let manifest = table(&read("Cargo.toml"));
            let server = &manifest["dependencies"]["rusqlite"];
            assert!(
                server.get("version").is_some(),
                "the server's rusqlite has a version"
            );
            assert!(
                server.get("git").is_none() && server.get("path").is_none(),
                "the server's rusqlite comes from crates.io"
            );
            let fork = &manifest["dev-dependencies"]["rusqlite_fork"];
            assert_eq!(fork["package"].as_str(), Some("rusqlite"));
            assert_eq!(fork["git"].as_str(), Some(FORK));
            let rev = fork["rev"].as_str().expect("a rev").to_owned();
            assert!(
                rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()),
                "a full commit id: {rev}"
            );
            rev
        }

        #[test]
        fn only_the_fork_is_patched() {
            pinned();
            let workspace = table(&read("../../Cargo.toml"));
            let patch = workspace["patch"].as_table().expect("a patch table");
            assert_eq!(
                patch.keys().collect::<Vec<_>>(),
                [FORK],
                "no crates-io patch"
            );
            let sys: Vec<&String> = patch[FORK].as_table().expect("a table").keys().collect();
            assert_eq!(sys, ["libsqlite3-sys"]);
        }

        #[test]
        fn the_development_page_names_the_pinned_commit() {
            let page = read("../../website/content/docs/development.md");
            let section = page
                .split("\n## ")
                .find(|s| s.starts_with("Test dependency from a fork\n"))
                .expect("a section on the fork");
            assert!(
                section.contains(&format!("`{}`", pinned())),
                "the pinned commit"
            );
            assert!(section.contains(FORK), "the fork's address");
            let lock = read("../../Cargo.lock");
            let version = lock
                .split("[[package]]")
                .find(|p| p.contains(&format!("source = \"git+{FORK}?rev=")))
                .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = \"")))
                .map(|v| v.trim_end_matches('"'))
                .expect("the fork in Cargo.lock");
            assert!(
                section.contains(&format!("`{version}`")),
                "the fork's version {version}"
            );
            let source = include_str!("catalog.rs");
            for test in [
                "sqlcipher_settings_read_the_same_through_the_pragma_helpers",
                "crates_io_rusqlite_refuses_sqlcipher_names",
            ] {
                assert!(section.contains(&format!("`{test}`")), "names {test}");
                assert!(source.contains(&format!("fn {test}()")), "{test} exists");
            }
        }

        #[test]
        fn the_install_page_says_what_a_build_fetches() {
            let page = read("../../website/content/docs/install.md");
            let build = page
                .split("\n## ")
                .find(|s| s.starts_with("Build from source\n"))
                .expect("a Build from source section");
            assert!(
                build.contains(FORK.trim_start_matches("https://")),
                "names the fork Cargo.toml pins, from the one constant both checks read"
            );
            assert!(
                build.contains("mkdir -p .cargo && cargo vendor >> .cargo/config.toml\n"),
                "the vendoring command"
            );
            assert!(build.contains("cargo build --release --offline\n"));
            let release = page
                .split("\n## ")
                .find(|s| s.starts_with("Download a release\n"))
                .expect("a release section");
            assert!(
                release.contains("crates.io"),
                "says which rusqlite a release holds"
            );
        }
    }

    #[test]
    fn the_schema_is_made_once_at_the_current_version() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);
        conn.execute("INSERT INTO note (text) VALUES ('kept')", [])
            .expect("insert");
        drop(conn);
        let conn =
            open(&path(&dir), &key, SCHEMA).expect("reopens without making the schema again");
        let kept: String = conn
            .query_row("SELECT text FROM note", [], |r| r.get(0))
            .expect("row");
        assert_eq!(kept, "kept");
    }

    #[test]
    fn an_unknown_schema_version_is_refused() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("set version");
        drop(conn);
        assert!(matches!(
            open(&path(&dir), &key, SCHEMA),
            Err(CatalogError::UnknownSchema(v)) if v == SCHEMA_VERSION + 1
        ));
    }

    /// A catalog made at version 1 opens at version 2, its rows kept. One
    /// with no vCon table, as server.sqlite is, changes only its number;
    /// the tenant catalog's upgrade is tested with the Store.
    #[test]
    fn a_version_1_catalog_is_upgraded_to_version_2() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
        conn.execute("INSERT INTO note (text) VALUES ('kept')", [])
            .expect("insert");
        conn.pragma_update(None, "user_version", 1)
            .expect("set version");
        drop(conn);
        let conn = open(&path(&dir), &key, "").expect("upgrades");
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(version, 2);
        assert_eq!(notes(&conn), vec!["kept"]);
    }

    /// The path a re-encryption writes before its rename.
    fn rekeyed(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("catalog.sqlite.rekey")
    }

    fn notes(conn: &Connection) -> Vec<String> {
        conn.prepare("SELECT text FROM note ORDER BY rowid")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    #[test]
    fn a_reencrypted_catalog_opens_under_the_new_key_only_and_keeps_every_row() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (old, new) = (Key::generate().expect("key"), Key::generate().expect("key"));
        let conn = open(&path(&dir), &old, SCHEMA).expect("opens");
        for n in 0..300 {
            conn.execute("INSERT INTO note (text) VALUES (?1)", [format!("note {n}")])
                .expect("insert");
        }
        drop(conn);
        reencrypt(&path(&dir), &old, &new).expect("re-encrypted");
        let raw = std::fs::read(path(&dir)).expect("the file");
        assert!(
            !raw.windows(8).any(|w| w == b"note 299"),
            "no row is written in the clear"
        );
        assert!(matches!(
            open(&path(&dir), &old, SCHEMA),
            Err(CatalogError::WrongKey)
        ));
        let conn = open(&path(&dir), &new, SCHEMA).expect("opens under the new key");
        let kept = notes(&conn);
        assert_eq!(
            kept,
            (0..300).map(|n| format!("note {n}")).collect::<Vec<_>>(),
            "every row is kept, each with its own text, in order"
        );
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION, "the schema version is kept");
        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .expect("mode");
        assert_eq!(mode, "wal");
        drop(conn);
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["catalog.sqlite"], "no temporary file is left");
    }

    #[test]
    fn a_file_left_by_an_interrupted_reencryption_is_replaced() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (old, new) = (Key::generate().expect("key"), Key::generate().expect("key"));
        let conn = open(&path(&dir), &old, SCHEMA).expect("opens");
        conn.execute("INSERT INTO note (text) VALUES ('kept')", [])
            .expect("insert");
        drop(conn);
        std::fs::write(rekeyed(&dir), b"half a file").expect("leftover");
        reencrypt(&path(&dir), &old, &new).expect("re-encrypted");
        let conn = open(&path(&dir), &new, SCHEMA).expect("opens");
        assert_eq!(notes(&conn), ["kept"]);
        assert!(!rekeyed(&dir).exists());
    }

    /// A WAL holding frames belongs to its own file: renaming that file
    /// without them would lose committed rows, and leaving the old one's
    /// beside the new file would corrupt it. Either refuses the rename.
    #[test]
    fn a_replace_refuses_while_either_wal_holds_frames() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let copy = dir.path().join("catalog.sqlite.rekey");
        for busy in [path(&dir), copy.clone()] {
            for file in [path(&dir), copy.clone()] {
                for leftover in ["", "-wal", "-shm"] {
                    let _ = std::fs::remove_file(sibling(&file, leftover));
                }
                drop(open(&file, &key, SCHEMA).expect("made"));
            }
            let held = open(&busy, &key, SCHEMA).expect("opens");
            held.pragma_update(None, "wal_autocheckpoint", 0)
                .expect("no checkpoint");
            held.execute("INSERT INTO note (text) VALUES ('in the wal')", [])
                .expect("insert");
            assert!(
                std::fs::metadata(sibling(&busy, "-wal")).is_ok_and(|m| m.len() > 0),
                "the WAL holds the row"
            );
            assert!(
                matches!(replace(&path(&dir), &copy), Err(CatalogError::Io(_))),
                "{}",
                busy.display()
            );
            assert!(copy.exists(), "nothing was renamed");
            drop(held);
        }
    }

    #[test]
    fn reencrypting_under_the_wrong_key_changes_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (old, new) = (Key::generate().expect("key"), Key::generate().expect("key"));
        drop(open(&path(&dir), &old, SCHEMA).expect("opens"));
        let other = Key::generate().expect("key");
        assert!(matches!(
            reencrypt(&path(&dir), &other, &new),
            Err(CatalogError::WrongKey)
        ));
        assert!(
            open(&path(&dir), &old, SCHEMA).is_ok(),
            "still under the old key"
        );
    }

    #[test]
    fn a_backup_holds_every_committed_row_and_opens_under_the_same_key_only() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let source = open(&path(&dir), &key, SCHEMA).expect("opens");
        source
            .execute("INSERT INTO note (text) VALUES ('committed')", [])
            .expect("insert");
        // A second connection holds a write transaction open: its row is
        // not committed, so the copy must not hold it.
        let writer = open(&path(&dir), &key, SCHEMA).expect("opens");
        writer.execute_batch("BEGIN IMMEDIATE").expect("begin");
        writer
            .execute("INSERT INTO note (text) VALUES ('uncommitted')", [])
            .expect("insert");
        let copy = dir.path().join("copy.sqlite");
        backup(&source, &copy, &key).expect("backs up");
        writer.execute_batch("COMMIT").expect("commit");
        assert!(
            !sibling(&copy, "-wal").exists()
                || std::fs::metadata(sibling(&copy, "-wal"))
                    .expect("wal")
                    .len()
                    == 0,
            "the copy is whole in its own file"
        );
        let reopened = open(&copy, &key, "").expect("the copy opens under the same key");
        let notes: Vec<String> = reopened
            .prepare("SELECT text FROM note")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert_eq!(notes, ["committed"]);
        let version: i64 = reopened
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);
        drop(reopened);
        let other = Key::generate().expect("key");
        assert!(matches!(
            open(&copy, &other, ""),
            Err(CatalogError::WrongKey)
        ));
    }

    #[test]
    fn a_backup_holds_rows_still_in_the_source_wal_and_no_plain_text() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let source = open(&path(&dir), &key, SCHEMA).expect("opens");
        source
            .execute("INSERT INTO note (text) VALUES (?1)", [MARKER])
            .expect("insert");
        assert!(
            std::fs::metadata(sibling(&path(&dir), "-wal"))
                .expect("wal")
                .len()
                > 0,
            "the row is still in the WAL"
        );
        let copy = dir.path().join("copy.sqlite");
        backup(&source, &copy, &key).expect("backs up");
        let bytes = std::fs::read(&copy).expect("read the copy");
        assert!(
            !bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
            "the copy is encrypted"
        );
        let reopened = open(&copy, &key, "").expect("opens");
        let count: i64 = reopened
            .query_row("SELECT count(*) FROM note WHERE text = ?1", [MARKER], |r| {
                r.get(0)
            })
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn a_backup_never_writes_over_a_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let source = open(&path(&dir), &key, SCHEMA).expect("opens");
        let copy = dir.path().join("copy.sqlite");
        std::fs::write(&copy, b"something else").expect("write");
        assert!(matches!(
            backup(&source, &copy, &key),
            Err(CatalogError::Io(_))
        ));
        assert_eq!(std::fs::read(&copy).expect("read"), b"something else");
    }

    #[test]
    fn a_wrong_key_does_not_open_the_catalog() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        drop(open(&path(&dir), &key, SCHEMA).expect("opens"));
        let other = Key::generate().expect("key");
        assert!(matches!(
            open(&path(&dir), &other, SCHEMA),
            Err(CatalogError::WrongKey)
        ));
        assert!(
            open(&path(&dir), &key, SCHEMA).is_ok(),
            "the right key still opens it"
        );
    }

    /// A catalog file or directory this process may not write is named,
    /// with the system's error, and never taken for a wrong key.
    #[test]
    fn a_catalog_the_process_may_not_write_is_an_access_error_naming_the_path() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path, m| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).expect("chmod")
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        drop(open(&path(&dir), &key, SCHEMA).expect("opens"));
        for (target, read_only) in [(path(&dir), 0o400), (dir.path().to_owned(), 0o500)] {
            mode(&target, read_only);
            let refused = open(&path(&dir), &key, SCHEMA);
            mode(&target, if target.is_dir() { 0o700 } else { 0o600 });
            match refused {
                Err(CatalogError::Access { path, error }) => {
                    assert_eq!(path, target);
                    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
                }
                other => panic!("{}: {other:?}", target.display()),
            }
        }
        assert!(
            open(&path(&dir), &key, SCHEMA).is_ok(),
            "opens once allowed"
        );
    }

    /// A row that cannot insert at all would make every column look as if
    /// it refused NULL.
    #[test]
    #[should_panic(expected = "a whole note row inserts")]
    fn the_null_check_needs_a_row_that_inserts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
        testing::columns_taking_null(&conn, "note", &[("misspelt", "'a'")]);
    }

    /// Run while the connection is open, so the commit is still in the WAL.
    #[test]
    fn no_plain_record_reaches_the_catalog_or_its_wal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
        conn.execute("INSERT INTO note (text) VALUES (?1)", [MARKER])
            .expect("insert");
        let wal = std::fs::metadata(dir.path().join("catalog.sqlite-wal")).expect("a WAL");
        assert!(wal.len() > 0, "the commit is in the WAL");
        for entry in std::fs::read_dir(dir.path()).expect("list") {
            let file = entry.expect("entry").path();
            let bytes = std::fs::read(&file).expect("read");
            assert!(
                !bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
                "plain record in {}",
                file.display()
            );
        }
        drop(conn);
    }
}
