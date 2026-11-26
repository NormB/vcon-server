//! The upgrade to schema version 11, tested from catalogs made as schema
//! version 10 made them: server.sqlite as `v10_tests` builds version 9 with
//! the audit log's mark added, and a tenant catalog as release 0.4.2's
//! tables with version 5's audit log and the mark. Releases 0.4.17 and
//! 0.4.18 wrote version 10.
//!
//! Version 11 adds four columns to the tenants in server.sqlite: the
//! key-encryption key a key backend wrapped each tenant key under, as its
//! reference (`wrap_ref`) and its `key-wrap` blob (`wrapped_kek`), and the
//! same for the pending key. All four are NULL for every tenant there is:
//! its keys are under the master key file, as every release wrote them. A
//! CHECK keeps each pair whole. A tenant catalog has no tenant table, so
//! only its number changes.

use rusqlite::Connection;

use super::v5_tests::{
    AUDIT_READ, Old, REGISTRY_ROWS, REGISTRY_V4, TENANT_V4, declared, number, rows, version,
};
use super::v6_tests::{AUDIT_V5, ROWS_V5, TENANT_READ_V6, USERS_V5, columns};
use super::v8_tests::ADDED_V7;
use super::{CatalogError, KEY_LIFE, SCHEMA_VERSION, SESSIONS, open};
use crate::audit::COUNTED_COLUMN;
use crate::backend::local::LocalRegistry;
use crate::backend::{Registry, TenantRow};
use crate::envelope::Key;
use crate::ids::TenantId;
use crate::tenants::TenantError;

/// The version this file's catalogs are made at: the one before this
/// build's, and the one releases 0.4.17 and 0.4.18 wrote.
const BEFORE: i64 = 10;

/// The tenant of `REGISTRY_ROWS`.
const ACME: TenantId = TenantId([0xa1; 16]);

/// The four columns, in the order version 11 adds them.
const ADDED: [&str; 4] = [
    "wrap_ref",
    "wrapped_kek",
    "pending_wrap_ref",
    "pending_wrapped_kek",
];

/// A version-10 registry with the rows `rows` insert at version 5 (before
/// version 7's column).
fn registry_of(rows: &[&str]) -> Old {
    let mut sql = vec![REGISTRY_V4, USERS_V5, AUDIT_V5];
    sql.extend_from_slice(rows);
    sql.extend_from_slice(&[ADDED_V7, SESSIONS, KEY_LIFE, COUNTED_COLUMN]);
    Old::at(BEFORE, &sql)
}

/// A version-10 registry with two tenants, acme and beta.
fn registry() -> Old {
    registry_of(&[REGISTRY_ROWS, ROWS_V5])
}

/// The upgraded registry of `old`, as the server opens it.
fn served(old: Old) -> (tempfile::TempDir, LocalRegistry) {
    drop(old.upgraded());
    let Old { dir, key } = old;
    let registry = LocalRegistry::open(dir.path(), key).expect("the upgraded registry opens");
    (dir, registry)
}

fn acme(registry: &LocalRegistry) -> Result<Option<TenantRow>, TenantError> {
    registry.snapshot().expect("a snapshot").tenant(ACME)
}

/// Whether `sql` is refused on `conn` by a CHECK, not for another reason
/// (a column that is not there is refused too).
fn refused(conn: &Connection, sql: &str) -> bool {
    match conn.execute_batch(sql) {
        Ok(()) => false,
        Err(e) => {
            let text = e.to_string();
            assert!(text.contains("CHECK constraint failed"), "{sql}: {text}");
            true
        }
    }
}

// What must work.

#[test]
fn a_version_10_registry_gains_the_four_columns_at_the_end() {
    let conn = registry().upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "the upgraded version");
    let found = columns(&conn, "tenant");
    assert_eq!(
        found[found.len() - 4..],
        ADDED,
        "the last four columns of the tenant table: {found:?}"
    );
    assert_eq!(found.len(), 14, "ten before them: {found:?}");
}

#[test]
fn every_tenant_there_was_is_under_the_master_key_file() {
    let conn = registry().upgraded();
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM tenant WHERE wrap_ref IS NULL AND wrapped_kek IS NULL
             AND pending_wrap_ref IS NULL AND pending_wrapped_kek IS NULL"
        ),
        2,
        "neither tenant has a backend key"
    );
}

#[test]
fn a_version_10_registry_keeps_every_row_it_had() {
    let old = registry();
    let read = |conn: &Connection| {
        [
            rows(conn, TENANT_READ_V6),
            rows(conn, "SELECT * FROM user_account ORDER BY id"),
            rows(conn, "SELECT * FROM api_key ORDER BY id"),
            rows(conn, "SELECT * FROM master_key_use"),
            rows(conn, "SELECT * FROM tenant_setting"),
            rows(conn, AUDIT_READ),
        ]
    };
    let before = read(&old.raw());
    assert!(before.iter().all(|table| !table.is_empty()), "rows to keep");
    assert_eq!(read(&old.upgraded()), before, "every row of every table");
}

#[test]
fn an_upgraded_version_10_registry_is_declared_as_a_new_one_is() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalRegistry::open(dir.path(), Key::generate().expect("key")).expect("a new one");
    assert_eq!(
        declared(&registry().upgraded()),
        declared(&new.local_conn()),
        "tables and indexes, with their text"
    );
}

#[test]
fn a_0_4_3_registry_reaches_this_version_in_one_step() {
    let conn = Old::registry().upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "from version 4");
    let found = columns(&conn, "tenant");
    assert_eq!(
        found[found.len() - 4..],
        ADDED,
        "the four columns: {found:?}"
    );
    assert_eq!(
        number(&conn, "SELECT count(*) FROM tenant WHERE wrap_ref IS NULL"),
        1,
        "the tenant of release 0.4.3 is under the file"
    );
}

#[test]
fn a_tenant_of_a_version_10_registry_reads_back_under_the_file_key() {
    let (_dir, registry) = served(registry());
    let row = acme(&registry).expect("read").expect("the tenant");
    assert_eq!(row.wrapped, vec![0xbe, 0xef], "its wrapped key");
    assert_eq!(row.kek, None, "no backend key");
    assert_eq!(row.pending_kek, None, "no pending backend key");
}

#[test]
fn a_new_registry_is_made_at_version_11_with_the_columns() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalRegistry::open(dir.path(), Key::generate().expect("key")).expect("a new one");
    let conn = new.local_conn();
    assert_eq!(version(&conn), 11, "this build's version");
    let found = columns(&conn, "tenant");
    assert_eq!(
        found[found.len() - 4..],
        ADDED,
        "the four columns: {found:?}"
    );
}

// What must be refused, or left alone.

#[test]
fn a_kek_reference_without_its_blob_is_refused() {
    let conn = registry().upgraded();
    assert!(
        refused(&conn, "UPDATE tenant SET wrap_ref = 'pkcs11:k'"),
        "a reference alone"
    );
    assert!(
        refused(&conn, "UPDATE tenant SET pending_wrap_ref = 'pkcs11:k'"),
        "a pending reference alone"
    );
}

#[test]
fn a_kek_blob_without_its_reference_is_refused() {
    let conn = registry().upgraded();
    assert!(
        refused(&conn, "UPDATE tenant SET wrapped_kek = x'0106'"),
        "a blob alone"
    );
    assert!(
        refused(&conn, "UPDATE tenant SET pending_wrapped_kek = x'0106'"),
        "a pending blob alone"
    );
    assert!(
        !refused(
            &conn,
            "UPDATE tenant SET wrap_ref = 'pkcs11:k', wrapped_kek = x'0106'"
        ),
        "the two together are kept"
    );
}

#[test]
fn a_half_kek_read_from_the_file_is_an_error() {
    let old = registry();
    drop(old.upgraded());
    let raw = old.raw();
    raw.execute_batch(
        "PRAGMA ignore_check_constraints = ON;
         UPDATE tenant SET pending_wrapped_kek = x'0106';",
    )
    .expect("a row no build writes");
    drop(raw);
    let (_dir, registry) = served(old);
    assert_eq!(acme(&registry), Err(TenantError::Io), "half a pending KEK");
}

#[test]
fn an_upgrade_that_fails_at_a_column_leaves_version_10_whole() {
    let old = registry();
    old.raw()
        .execute_batch("ALTER TABLE tenant ADD COLUMN pending_wrapped_kek INTEGER;")
        .expect("a column in the way");
    let before = declared(&old.raw());
    assert!(old.open().is_err(), "the column cannot be added twice");
    assert_eq!(version(&old.raw()), BEFORE, "still version 10");
    assert_eq!(
        declared(&old.raw()),
        before,
        "the three columns added before it are gone"
    );
    old.raw()
        .execute_batch("ALTER TABLE tenant DROP COLUMN pending_wrapped_kek;")
        .expect("the column taken away");
    assert_eq!(
        version(&old.upgraded()),
        SCHEMA_VERSION,
        "then the upgrade runs"
    );
}

#[test]
fn a_wrong_key_upgrades_no_version_10_registry() {
    let old = registry();
    let other = Key::generate().expect("key");
    assert!(
        matches!(open(&old.path(), &other, ""), Err(CatalogError::WrongKey)),
        "the wrong key is named"
    );
    assert_eq!(version(&old.raw()), BEFORE, "still version 10");
    assert!(
        !columns(&old.raw(), "tenant").contains(&"wrap_ref".to_owned()),
        "no column was added"
    );
}

// Limits and odd inputs.

#[test]
fn a_version_10_tenant_catalog_changes_only_its_number() {
    let old = Old::at(BEFORE, &[TENANT_V4, AUDIT_V5, COUNTED_COLUMN]);
    let before = declared(&old.raw());
    let conn = old.upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "the number");
    assert_eq!(declared(&conn), before, "no table made or changed");
}

#[test]
fn opening_an_upgraded_registry_again_changes_nothing() {
    let old = registry();
    let first = old.upgraded();
    let text = declared(&first);
    let tenants = rows(&first, "SELECT * FROM tenant");
    drop(first);
    let second = old.upgraded();
    assert_eq!(declared(&second), text, "the same tables");
    assert_eq!(
        rows(&second, "SELECT * FROM tenant"),
        tenants,
        "the same rows"
    );
}

#[test]
fn this_build_makes_a_version_a_version_10_build_refuses() {
    // A build at version 10 opens versions 0 to 10 and refuses every other
    // number, so it cannot read a tenant key under a backend as if it were
    // under the master key file.
    let conn = registry().upgraded();
    assert!(version(&conn) > BEFORE, "the number: {}", version(&conn));
}

#[test]
fn a_registry_with_no_tenant_upgrades_with_the_columns() {
    let conn = registry_of(&[]).upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "upgraded");
    assert_eq!(number(&conn, "SELECT count(*) FROM tenant"), 0, "no tenant");
    assert!(
        columns(&conn, "tenant").contains(&"pending_wrapped_kek".to_owned()),
        "the columns are there"
    );
}
