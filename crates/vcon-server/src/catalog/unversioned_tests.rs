//! A catalog that holds tables but reads schema version 0. SQLite keeps
//! `PRAGMA user_version` in 32 bits, so a value outside them, written by
//! damage or by another program, reads back as another: 2^32, i64::MIN and
//! i64::MAX all read back as 0 (the tests below hold each). Before this check such a catalog was taken as new: making its tables
//! failed with "table ... already exists", which named neither the file nor
//! the damage. Now it is refused before anything is written to it, with a
//! text that names the file and the tables found. server.sqlite and every
//! tenant catalog open through `open`, so both are held here.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::{CatalogError, SCHEMA_VERSION, open};
use crate::backend::local::{Durability, LocalCatalog, LocalRegistry};
use crate::envelope::{self, Key};

const SCHEMA: &str = "CREATE TABLE note (text TEXT NOT NULL); CREATE TABLE tag (name TEXT);";

fn path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("catalog.sqlite")
}

/// A catalog made with SCHEMA under `key`, its version then set to `to`.
fn made_then_set(dir: &tempfile::TempDir, key: &Key, to: i64) {
    let conn = open(&path(dir), key, SCHEMA).expect("made");
    conn.execute("INSERT INTO note (text) VALUES ('kept')", [])
        .expect("a row");
    conn.execute_batch(&format!("PRAGMA user_version = {to}"))
        .expect("version set");
}

fn version_of(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("version")
}

/// The names in the catalog's schema, read under `key` without `open`.
fn names(path: &Path, key: &Key) -> Vec<String> {
    let conn = Connection::open(path).expect("opens");
    super::set_key(&conn, key).expect("key");
    conn.prepare("SELECT name FROM sqlite_master ORDER BY name")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("names")
}

fn refused(dir: &tempfile::TempDir, key: &Key, schema: &str) -> CatalogError {
    match open(&path(dir), key, schema) {
        Ok(_) => panic!("a catalog with tables at version 0 opened"),
        Err(e) => e,
    }
}

// Success: what opened before still opens.

#[test]
fn a_new_file_is_made_at_the_current_version() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    let conn = open(&path(&dir), &key, SCHEMA).expect("made");
    assert_eq!(
        version_of(&conn),
        SCHEMA_VERSION,
        "made at this build's version"
    );
}

#[test]
fn an_empty_database_at_version_0_is_made_as_new() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    {
        let conn = Connection::open(path(&dir)).expect("opens");
        super::set_key(&conn, &key).expect("key");
        conn.execute_batch("CREATE TABLE gone (x); DROP TABLE gone;")
            .expect("an empty database written");
    }
    let conn = open(&path(&dir), &key, SCHEMA).expect("made as new");
    assert_eq!(
        version_of(&conn),
        SCHEMA_VERSION,
        "made at this build's version"
    );
}

#[test]
fn a_catalog_at_the_current_version_opens_with_its_rows() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, SCHEMA_VERSION);
    let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
    let kept: String = conn
        .query_row("SELECT text FROM note", [], |r| r.get(0))
        .expect("row");
    assert_eq!(kept, "kept", "its row");
}

#[test]
fn a_catalog_at_version_1_is_still_upgraded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 1);
    let conn = open(&path(&dir), &key, SCHEMA).expect("upgrades");
    assert_eq!(
        version_of(&conn),
        SCHEMA_VERSION,
        "brought to this build's version"
    );
}

#[test]
fn a_new_server_sqlite_is_made() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(
        LocalRegistry::open(dir.path(), Key::generate().expect("key")).is_ok(),
        "a new server.sqlite opens"
    );
}

#[test]
fn a_new_tenant_catalog_is_made() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    assert!(
        tenant_catalog(dir.path(), &key).is_ok(),
        "a new tenant catalog opens"
    );
}

// Failure: tables at version 0 are refused, and nothing is written.

#[test]
fn tables_at_version_0_are_refused_as_unversioned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 0);
    assert!(
        matches!(
            refused(&dir, &key, SCHEMA),
            CatalogError::Unversioned { .. }
        ),
        "refused as unversioned"
    );
}

#[test]
fn the_refusal_names_the_file_and_each_table_found() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 0);
    match refused(&dir, &key, SCHEMA) {
        CatalogError::Unversioned { path: at, tables } => {
            assert_eq!(at, path(&dir), "the file");
            assert_eq!(tables, ["note", "tag"], "the tables, by name");
        }
        other => panic!("refused as unversioned, not {other:?}"),
    }
}

#[test]
fn the_refusal_s_text_names_the_file_the_tables_and_the_cause() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 0);
    let text = refused(&dir, &key, SCHEMA).to_string();
    assert_eq!(
        text,
        format!(
            "{} holds tables (note, tag) but no schema version: it is damaged, or another program wrote it",
            path(&dir).display()
        ),
        "the whole text"
    );
}

#[test]
fn a_refused_catalog_is_left_as_it_was() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 0);
    let before = names(&path(&dir), &key);
    refused(&dir, &key, SCHEMA);
    assert_eq!(names(&path(&dir), &key), before, "no table made or dropped");
    let conn = Connection::open(path(&dir)).expect("opens");
    super::set_key(&conn, &key).expect("key");
    assert_eq!(version_of(&conn), 0, "its version not written");
}

#[test]
fn a_refused_catalog_keeps_its_rows() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 0);
    refused(&dir, &key, SCHEMA);
    let conn = Connection::open(path(&dir)).expect("opens");
    super::set_key(&conn, &key).expect("key");
    let kept: String = conn
        .query_row("SELECT text FROM note", [], |r| r.get(0))
        .expect("row");
    assert_eq!(kept, "kept", "the row stays");
}

#[test]
fn a_version_of_2_to_the_32_reads_as_0_and_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 1 << 32);
    assert!(
        matches!(
            refused(&dir, &key, SCHEMA),
            CatalogError::Unversioned { .. }
        ),
        "the wrapped 0 is not taken for new"
    );
}

#[test]
fn a_version_of_i64_min_reads_as_0_and_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, i64::MIN);
    assert!(
        matches!(
            refused(&dir, &key, SCHEMA),
            CatalogError::Unversioned { .. }
        ),
        "the wrapped 0 is not taken for new"
    );
}

#[test]
fn a_version_of_i64_max_is_refused_as_unversioned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, i64::MAX);
    let e = refused(&dir, &key, SCHEMA);
    assert!(
        matches!(e, CatalogError::Unversioned { .. }),
        "SQLite reads a version past 32 bits back as 0: {e:?}"
    );
}

#[test]
fn a_view_alone_at_version_0_is_refused_and_named() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    {
        let conn = Connection::open(path(&dir)).expect("opens");
        super::set_key(&conn, &key).expect("key");
        conn.execute_batch("CREATE VIEW seen AS SELECT 1 AS one;")
            .expect("a view");
    }
    match refused(&dir, &key, SCHEMA) {
        CatalogError::Unversioned { tables, .. } => assert_eq!(tables, ["seen"], "the view"),
        other => panic!("refused as unversioned, not {other:?}"),
    }
}

#[test]
fn a_server_sqlite_with_tables_at_version_0_is_refused_with_the_text() {
    let dir = tempfile::tempdir().expect("temp dir");
    let master = Key::generate().expect("key");
    drop(LocalRegistry::open(dir.path(), same(&master)).expect("made"));
    set_version(&dir.path().join("server.sqlite"), &same(&master), 0);
    let refused = match LocalRegistry::open(dir.path(), same(&master)) {
        Ok(_) => panic!("a damaged server.sqlite opened"),
        Err(e) => e,
    };
    let text = crate::tenants::with_causes(&refused);
    assert!(
        text.contains("server.sqlite holds tables (") && text.contains("but no schema version"),
        "the cause is shown: {text}"
    );
    assert!(text.contains("tenant"), "a table named: {text}");
}

#[test]
fn a_refused_server_sqlite_gets_no_table() {
    let dir = tempfile::tempdir().expect("temp dir");
    let master = Key::generate().expect("key");
    drop(LocalRegistry::open(dir.path(), same(&master)).expect("made"));
    let file = dir.path().join("server.sqlite");
    set_version(&file, &same(&master), 0);
    let before = names(&file, &same(&master));
    assert!(
        LocalRegistry::open(dir.path(), same(&master)).is_err(),
        "refused"
    );
    assert_eq!(names(&file, &same(&master)), before, "no table made");
}

/// A key that is the same each time it is asked for: a key cannot be
/// copied, and the one derived from `master` is the same each time.
fn same(master: &Key) -> Key {
    envelope::catalog_key(master).expect("a derived key")
}

#[test]
fn a_tenant_catalog_with_tables_at_version_0_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    drop(tenant_catalog(dir.path(), &key).expect("made"));
    let catalog_key = envelope::catalog_key(&key).expect("catalog key");
    set_version(&dir.path().join("catalog.sqlite"), &catalog_key, 0);
    let before = names(&dir.path().join("catalog.sqlite"), &catalog_key);
    assert!(tenant_catalog(dir.path(), &key).is_err(), "refused");
    assert_eq!(
        names(&dir.path().join("catalog.sqlite"), &catalog_key),
        before,
        "no table made in it"
    );
}

#[test]
fn a_tenant_catalog_s_refusal_is_not_a_table_that_exists() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    drop(tenant_catalog(dir.path(), &key).expect("made"));
    let catalog_key = envelope::catalog_key(&key).expect("catalog key");
    set_version(&dir.path().join("catalog.sqlite"), &catalog_key, 0);
    let text = match tenant_catalog(dir.path(), &key) {
        Ok(_) => panic!("a damaged tenant catalog opened"),
        Err(e) => e.to_string(),
    };
    assert!(
        !text.contains("already exists"),
        "not SQLite's text: {text}"
    );
    assert!(text.contains("vcon"), "the vcon table named: {text}");
}

// Edges.

#[test]
fn a_table_whose_name_starts_with_sqlite_is_not_counted() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    {
        let conn = Connection::open(path(&dir)).expect("opens");
        super::set_key(&conn, &key).expect("key");
        // ANALYZE makes sqlite_stat1, SQLite's own table.
        conn.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1); ANALYZE; DROP TABLE t;")
            .expect("written");
    }
    let conn = open(&path(&dir), &key, SCHEMA).expect("made as new");
    assert_eq!(
        version_of(&conn),
        SCHEMA_VERSION,
        "SQLite's own tables are not the catalog's"
    );
}

#[test]
fn one_table_is_named_alone() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    let conn = open(&path(&dir), &key, "CREATE TABLE only (x);").expect("made");
    conn.execute_batch("PRAGMA user_version = 0").expect("set");
    drop(conn);
    let text = refused(&dir, &key, "CREATE TABLE only (x);").to_string();
    assert!(text.contains("holds tables (only) but"), "{text}");
}

#[test]
fn the_tables_are_named_in_order_of_name() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    let schema = "CREATE TABLE zeta (x); CREATE TABLE alpha (x); CREATE TABLE mid (x);";
    let conn = open(&path(&dir), &key, schema).expect("made");
    conn.execute_batch("PRAGMA user_version = 0").expect("set");
    drop(conn);
    match refused(&dir, &key, schema) {
        CatalogError::Unversioned { tables, .. } => {
            assert_eq!(tables, ["alpha", "mid", "zeta"], "sorted");
        }
        other => panic!("refused as unversioned, not {other:?}"),
    }
}

#[test]
fn an_index_is_not_named_as_a_table() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    let schema = "CREATE TABLE one (x); CREATE INDEX one_x ON one (x);";
    let conn = open(&path(&dir), &key, schema).expect("made");
    conn.execute_batch("PRAGMA user_version = 0").expect("set");
    drop(conn);
    match refused(&dir, &key, schema) {
        CatalogError::Unversioned { tables, .. } => assert_eq!(tables, ["one"], "the table only"),
        other => panic!("refused as unversioned, not {other:?}"),
    }
}

#[test]
fn the_refusal_has_no_source_and_one_line() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 0);
    let e = refused(&dir, &key, SCHEMA);
    assert!(
        std::error::Error::source(&e).is_none(),
        "the text says it all"
    );
    assert_eq!(e.to_string().lines().count(), 1, "one line");
}

#[test]
fn a_catalog_refused_once_is_refused_again() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 0);
    refused(&dir, &key, SCHEMA);
    assert!(
        matches!(
            refused(&dir, &key, SCHEMA),
            CatalogError::Unversioned { .. }
        ),
        "the second open is refused as the first was"
    );
}

#[test]
fn a_wrong_key_is_still_a_wrong_key() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    made_then_set(&dir, &key, 0);
    let other = Key::generate().expect("key");
    assert!(
        matches!(refused(&dir, &other, SCHEMA), CatalogError::WrongKey),
        "a key that does not open it is reported first"
    );
}

/// Sets the version of the catalog at `file`, read under `key`.
fn set_version(file: &Path, key: &Key, to: i64) {
    let conn = Connection::open(file).expect("opens");
    super::set_key(&conn, key).expect("key");
    conn.execute_batch(&format!("PRAGMA user_version = {to}"))
        .expect("version set");
}

/// A tenant catalog in `dir` under the tenant key `key`, as a Store opens
/// it.
fn tenant_catalog(dir: &Path, key: &Key) -> Result<LocalCatalog, CatalogError> {
    let catalog_key = envelope::catalog_key(key).expect("catalog key");
    LocalCatalog::open(
        &dir.join("catalog.sqlite"),
        catalog_key,
        &crate::fs::OsFs,
        Durability::Group,
    )
}
