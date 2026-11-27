//! More cases of the version-0 rule (`unversioned_tests` says why it
//! exists): every version a catalog can be at still opens, a file whose
//! only tables are SQLite's own is new, and the refusal's edges.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::{CatalogError, SCHEMA_VERSION, open};
use crate::backend::local::{Durability, LocalCatalog, LocalRegistry};
use crate::envelope::{self, Key};

const SCHEMA: &str = "CREATE TABLE note (text TEXT NOT NULL);";

fn path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("catalog.sqlite")
}

/// A connection to `file` under `key`, without `open`'s rules.
fn raw(file: &Path, key: &Key) -> Connection {
    let conn = Connection::open(file).expect("opens");
    super::set_key(&conn, key).expect("key");
    conn
}

fn version_of(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("version")
}

fn notes(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT text FROM note ORDER BY rowid")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

/// A catalog with SCHEMA and one row, at version `to`.
fn at(dir: &tempfile::TempDir, key: &Key, to: i64) {
    let conn = open(&path(dir), key, SCHEMA).expect("made");
    conn.execute("INSERT INTO note (text) VALUES ('kept')", [])
        .expect("a row");
    conn.execute_batch(&format!("PRAGMA user_version = {to}"))
        .expect("version set");
}

/// A database at version 0 written by `sql` alone, under `key`.
fn written(dir: &tempfile::TempDir, key: &Key, sql: &str) {
    raw(&path(dir), key).execute_batch(sql).expect("written");
}

fn refused(dir: &tempfile::TempDir, key: &Key) -> CatalogError {
    match open(&path(dir), key, SCHEMA) {
        Ok(_) => panic!("a catalog with tables at version 0 opened"),
        Err(e) => e,
    }
}

fn tables_of(e: CatalogError) -> Vec<String> {
    match e {
        CatalogError::Unversioned { tables, .. } => tables,
        other => panic!("refused as unversioned, not {other:?}"),
    }
}

/// A key that is the same each time it is asked for.
fn same(master: &Key) -> Key {
    envelope::catalog_key(master).expect("a derived key")
}

fn tenant_catalog(dir: &Path, key: &Key) -> Result<LocalCatalog, CatalogError> {
    LocalCatalog::open(
        &dir.join("catalog.sqlite"),
        same(key),
        &crate::fs::OsFs,
        Durability::Group,
    )
}

// Success: a catalog at every version this build reads opens, its rows
// kept, at this build's version.

macro_rules! upgraded_from {
    ($($name:ident: $from:expr,)*) => {$(
        #[test]
        fn $name() {
            let dir = tempfile::tempdir().expect("temp dir");
            let key = Key::generate().expect("key");
            at(&dir, &key, $from);
            let conn = open(&path(&dir), &key, SCHEMA).expect("opens");
            assert_eq!(version_of(&conn), SCHEMA_VERSION, "from {}: this build's version", $from);
            assert_eq!(notes(&conn), ["kept"], "from {}: its row", $from);
        }
    )*};
}

upgraded_from! {
    a_catalog_at_version_1_opens: 1,
    a_catalog_at_version_2_opens: 2,
    a_catalog_at_version_3_opens: 3,
    a_catalog_at_version_4_opens: 4,
    a_catalog_at_version_5_opens: 5,
    a_catalog_at_version_6_opens: 6,
    a_catalog_at_version_7_opens: 7,
    a_catalog_at_version_8_opens: 8,
    a_catalog_at_version_9_opens: 9,
    a_catalog_at_version_10_opens: 10,
}

#[test]
fn a_catalog_whose_version_is_set_back_after_the_refusal_opens() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    at(&dir, &key, 0);
    assert!(
        matches!(refused(&dir, &key), CatalogError::Unversioned { .. }),
        "refused first"
    );
    raw(&path(&dir), &key)
        .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
        .expect("set back");
    let conn = open(&path(&dir), &key, SCHEMA).expect("opens once repaired");
    assert_eq!(notes(&conn), ["kept"], "its row");
}

#[test]
fn a_server_sqlite_opens_again_after_it_is_made() {
    let dir = tempfile::tempdir().expect("temp dir");
    let master = Key::generate().expect("key");
    drop(LocalRegistry::open(dir.path(), same(&master)).expect("made"));
    assert!(
        LocalRegistry::open(dir.path(), same(&master)).is_ok(),
        "the made server.sqlite is not taken for damaged"
    );
}

#[test]
fn a_tenant_catalog_opens_again_after_it_is_made() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    drop(tenant_catalog(dir.path(), &key).expect("made"));
    assert!(
        tenant_catalog(dir.path(), &key).is_ok(),
        "the made catalog is not taken for damaged"
    );
}

#[test]
fn a_file_left_with_only_sqlite_s_autoincrement_table_is_new() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    // AUTOINCREMENT makes sqlite_sequence, which stays after the drop.
    written(
        &dir,
        &key,
        "CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT); INSERT INTO t DEFAULT VALUES; DROP TABLE t;",
    );
    let conn = open(&path(&dir), &key, SCHEMA).expect("made as new");
    assert_eq!(
        version_of(&conn),
        SCHEMA_VERSION,
        "made at this build's version"
    );
}

#[test]
fn a_new_file_is_made_with_its_schema() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    let conn = open(&path(&dir), &key, SCHEMA).expect("made");
    assert_eq!(
        notes(&conn),
        Vec::<String>::new(),
        "the schema's table, empty"
    );
}

// Failure.

#[test]
fn a_view_and_a_table_are_both_named() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    written(
        &dir,
        &key,
        "CREATE TABLE note (text); CREATE VIEW seen AS SELECT text FROM note;",
    );
    assert_eq!(
        tables_of(refused(&dir, &key)),
        ["note", "seen"],
        "both, in order"
    );
}

#[test]
fn a_tenant_catalog_at_2_to_the_32_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    drop(tenant_catalog(dir.path(), &key).expect("made"));
    raw(&dir.path().join("catalog.sqlite"), &same(&key))
        .execute_batch("PRAGMA user_version = 4294967296")
        .expect("set");
    assert!(
        matches!(
            tenant_catalog(dir.path(), &key),
            Err(CatalogError::Unversioned { .. })
        ),
        "refused as unversioned"
    );
}

#[test]
fn a_tenant_catalog_s_refusal_names_its_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    drop(tenant_catalog(dir.path(), &key).expect("made"));
    raw(&dir.path().join("catalog.sqlite"), &same(&key))
        .execute_batch("PRAGMA user_version = 0")
        .expect("set");
    let Err(e) = tenant_catalog(dir.path(), &key) else {
        panic!("a damaged tenant catalog opened")
    };
    assert!(
        e.to_string().starts_with(&format!(
            "{} holds tables (",
            dir.path().join("catalog.sqlite").display()
        )),
        "{e}"
    );
}

#[test]
fn a_server_sqlite_at_i64_min_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let master = Key::generate().expect("key");
    drop(LocalRegistry::open(dir.path(), same(&master)).expect("made"));
    raw(&dir.path().join("server.sqlite"), &same(&master))
        .execute_batch(&format!("PRAGMA user_version = {}", i64::MIN))
        .expect("set");
    let Err(e) = LocalRegistry::open(dir.path(), same(&master)) else {
        panic!("a damaged server.sqlite opened")
    };
    assert!(
        crate::tenants::with_causes(&e).contains("but no schema version"),
        "{e}"
    );
}

#[test]
fn a_table_and_a_trigger_name_the_table_only() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    written(
        &dir,
        &key,
        "CREATE TABLE note (text); CREATE TRIGGER noted AFTER INSERT ON note BEGIN SELECT 1; END;",
    );
    assert_eq!(tables_of(refused(&dir, &key)), ["note"], "the table");
}

#[test]
fn a_table_whose_name_starts_like_sqlite_s_is_counted() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    written(&dir, &key, "CREATE TABLE sqliteish (x);");
    assert_eq!(
        tables_of(refused(&dir, &key)),
        ["sqliteish"],
        "not SQLite's own"
    );
}

#[test]
fn a_refused_catalog_gets_no_version_written() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    at(&dir, &key, 1 << 32);
    refused(&dir, &key);
    assert_eq!(version_of(&raw(&path(&dir), &key)), 0, "still reads 0");
}

// Edges.

#[test]
fn a_table_named_with_sqlite_and_a_digit_is_counted() {
    // The `_` in SQLite's prefix is a character, not LIKE's any-one.
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    written(&dir, &key, "CREATE TABLE sqlite1x (x);");
    assert_eq!(tables_of(refused(&dir, &key)), ["sqlite1x"], "counted");
}

#[test]
fn a_table_whose_name_has_a_space_is_named_as_it_is() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    written(&dir, &key, "CREATE TABLE \"my notes\" (x);");
    assert_eq!(tables_of(refused(&dir, &key)), ["my notes"], "as written");
}

#[test]
fn fifty_tables_are_all_named() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    let sql: String = (0..50)
        .map(|n| format!("CREATE TABLE t{n:02} (x);"))
        .collect();
    written(&dir, &key, &sql);
    assert_eq!(tables_of(refused(&dir, &key)).len(), 50, "each one");
}

#[test]
fn a_version_of_minus_1_is_unknown_not_unversioned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    at(&dir, &key, -1);
    assert!(
        matches!(refused(&dir, &key), CatalogError::UnknownSchema(-1)),
        "a version this build does not know"
    );
}

#[test]
fn the_largest_32_bit_version_is_unknown_not_unversioned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    at(&dir, &key, i64::from(i32::MAX));
    assert!(
        matches!(
            refused(&dir, &key),
            CatalogError::UnknownSchema(v) if v == i64::from(i32::MAX)
        ),
        "a version this build does not know"
    );
}

#[test]
fn the_next_version_is_unknown_not_unversioned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    at(&dir, &key, SCHEMA_VERSION + 1);
    assert!(
        matches!(
            refused(&dir, &key),
            CatalogError::UnknownSchema(v) if v == SCHEMA_VERSION + 1
        ),
        "a version this build does not know"
    );
}
