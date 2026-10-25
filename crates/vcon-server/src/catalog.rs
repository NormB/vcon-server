//! An SQLite catalog, encrypted with SQLCipher under its catalog key.

use std::ffi::c_int;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, ErrorCode};

use crate::envelope::Key;

/// The schema version this build makes and reads.
pub const SCHEMA_VERSION: i64 = 1;

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
}

impl From<rusqlite::Error> for CatalogError {
    fn from(e: rusqlite::Error) -> CatalogError {
        CatalogError::Sqlite(e)
    }
}

/// Opens the catalog at `path` under `key`, making it with `schema` if it
/// is new.
pub fn open(path: &Path, key: &Key, schema: &str) -> Result<Connection, CatalogError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    // SQLCipher otherwise writes its own error lines to standard error, a
    // wrong key among them; this program says what went wrong itself.
    // SQLCipher's settings are not in SQLite's pragma list, which rusqlite's
    // pragma helpers check names against, so they go as plain SQL.
    conn.execute_batch("PRAGMA cipher_log_level = WARN")?;
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
        SCHEMA_VERSION => {}
        other => return Err(CatalogError::UnknownSchema(other)),
    }
    Ok(conn)
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

    #[test]
    fn the_schema_is_made_once_at_version_1() {
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
        conn.pragma_update(None, "user_version", 2)
            .expect("set version");
        drop(conn);
        assert!(matches!(
            open(&path(&dir), &key, SCHEMA),
            Err(CatalogError::UnknownSchema(2))
        ));
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
        assert_eq!(kept.len(), 300);
        assert_eq!(kept[299], "note 299");
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
