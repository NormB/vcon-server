//! The upgrade to schema version 9, tested from a server.sqlite made as
//! schema version 8 made it: version 7's text (as `v8_tests` builds it)
//! with the session table added as that version's upgrade added it. No
//! release wrote version 8; a build of `main` between releases 0.4.3 and
//! the next did.
//!
//! Version 9 adds two columns to the access keys in server.sqlite:
//! `expires_at` and `last_used_at`, both NULL for every key there is. A
//! key made before version 9 was made with no expiry and keeps none, so an
//! upgrade refuses no key that worked before it.

use super::v5_tests::{
    KEYS_V4, Old, REGISTRY_ROWS, REGISTRY_V4, TENANT_V4, declared, number, rows, tables, version,
};
use super::v6_tests::{AUDIT_V5, ROWS_V5, USERS_V5, columns};
use super::v8_tests::ADDED_V7;
use super::{CatalogError, SCHEMA_VERSION, SESSIONS, open};
use crate::backend::local::LocalRegistry;
use crate::backend::{KeyRow, Registry};
use crate::envelope::Key;
use crate::ids::{KeyId, TenantId};

/// The version this file's registry is made at, and the one before this
/// build's.
const BEFORE: i64 = 8;

/// The two keys of `REGISTRY_ROWS`: a tenant's, and an operator's that was
/// revoked.
const LOADER: KeyId = KeyId([0x03; 8]);
const OPERATOR: KeyId = KeyId([0x04; 8]);
const ACME: TenantId = TenantId([0xa1; 16]);

fn registry() -> Old {
    Old::at(
        BEFORE,
        &[
            REGISTRY_V4,
            USERS_V5,
            AUDIT_V5,
            REGISTRY_ROWS,
            ROWS_V5,
            ADDED_V7,
            SESSIONS,
        ],
    )
}

/// The upgraded registry of `old`, as the server opens it.
fn served(old: Old) -> (tempfile::TempDir, LocalRegistry) {
    drop(old.upgraded());
    let Old { dir, key } = old;
    let registry = LocalRegistry::open(dir.path(), key).expect("the upgraded registry opens");
    (dir, registry)
}

fn key(registry: &LocalRegistry, id: KeyId) -> KeyRow {
    registry
        .snapshot()
        .expect("a snapshot")
        .key(id)
        .expect("read")
        .expect("the key's row")
}

/// A new key's row, told from others by one byte of its id.
fn row(n: u8, expires_at: Option<i64>) -> KeyRow {
    KeyRow {
        id: KeyId([n; 8]),
        tenant: Some(ACME),
        token_hash: vec![n; 32],
        scopes: "read".to_owned(),
        label: "new".to_owned(),
        created_at: 100,
        revoked_at: None,
        expires_at,
        last_used_at: None,
    }
}

fn kept(registry: &LocalRegistry, row: &KeyRow) {
    let mut tx = registry.immediate().expect("a transaction");
    tx.insert_key(row).expect("inserted");
    tx.commit().expect("committed");
}

fn touched(registry: &LocalRegistry, id: KeyId, at: i64) -> bool {
    let mut tx = registry.immediate().expect("a transaction");
    let changed = tx.touch_key(id, at).expect("touched");
    tx.commit().expect("committed");
    changed
}

// What must work.

#[test]
fn a_version_8_registry_gains_the_two_columns_at_the_end() {
    let conn = registry().upgraded();
    assert_eq!(
        version(&conn),
        SCHEMA_VERSION,
        "the upgraded registry's version"
    );
    assert_eq!(
        columns(&conn, "api_key"),
        [
            "id",
            "tenant",
            "token_hash",
            "scopes",
            "label",
            "created_at",
            "revoked_at",
            "expires_at",
            "last_used_at"
        ],
        "the key table's columns"
    );
}

#[test]
fn every_key_there_was_has_no_expiry_and_no_last_used_time() {
    let conn = registry().upgraded();
    assert_eq!(
        number(&conn, "SELECT count(*) FROM api_key"),
        2,
        "both keys are there"
    );
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM api_key WHERE expires_at IS NULL AND last_used_at IS NULL"
        ),
        2,
        "neither has an expiry or a last-used time"
    );
}

#[test]
fn a_version_8_registry_keeps_every_row_it_had() {
    let old = registry();
    let read = |conn: &rusqlite::Connection| {
        [
            rows(conn, "SELECT * FROM tenant ORDER BY id"),
            rows(conn, "SELECT * FROM user_account ORDER BY id"),
            rows(conn, "SELECT * FROM membership ORDER BY user"),
            rows(conn, KEYS_V4),
            rows(conn, "SELECT * FROM tenant_setting"),
            rows(conn, "SELECT * FROM audit ORDER BY seq"),
        ]
    };
    let before = read(&old.raw());
    assert!(before.iter().all(|table| !table.is_empty()), "rows to keep");
    assert_eq!(read(&old.upgraded()), before, "every row of every table");
}

#[test]
fn an_upgraded_version_8_registry_is_declared_as_a_new_one_is() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalRegistry::open(dir.path(), Key::generate().expect("key")).expect("a new one");
    assert_eq!(
        declared(&registry().upgraded()),
        declared(&new.local_conn()),
        "tables and indexes, with their text"
    );
    assert_eq!(
        version(&new.local_conn()),
        SCHEMA_VERSION,
        "a new registry's version"
    );
}

#[test]
fn a_0_4_3_registry_reaches_this_version_in_one_step() {
    let conn = Old::registry().upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "from version 4");
    for column in ["expires_at", "last_used_at"] {
        assert!(
            columns(&conn, "api_key").contains(&column.to_owned()),
            "{column}"
        );
    }
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM api_key WHERE expires_at IS NULL"
        ),
        2,
        "the keys of release 0.4.3 have no expiry"
    );
}

#[test]
fn a_key_of_a_version_8_registry_reads_back_as_it_was_with_no_expiry() {
    let (_dir, registry) = served(registry());
    let loader = key(&registry, LOADER);
    assert_eq!(loader.tenant, Some(ACME), "its tenant");
    assert_eq!(loader.scopes, "read,write", "its scopes");
    assert_eq!(loader.revoked_at, None, "not revoked");
    assert_eq!(loader.expires_at, None, "no expiry");
    assert_eq!(loader.last_used_at, None, "no last-used time");
    let operator = key(&registry, OPERATOR);
    assert_eq!(operator.revoked_at, Some(11), "revoked as it was");
    assert_eq!(operator.expires_at, None, "no expiry");
}

#[test]
fn a_key_written_with_an_expiry_is_read_back_with_it() {
    let (_dir, registry) = served(registry());
    kept(&registry, &row(0x21, Some(5_000)));
    kept(&registry, &row(0x22, None));
    assert_eq!(
        key(&registry, KeyId([0x21; 8])),
        row(0x21, Some(5_000)),
        "dated"
    );
    assert_eq!(
        key(&registry, KeyId([0x22; 8])),
        row(0x22, None),
        "not dated"
    );
    let listed = registry
        .snapshot()
        .expect("a snapshot")
        .keys(Some(ACME))
        .expect("listed");
    assert_eq!(
        listed.iter().map(|k| k.expires_at).collect::<Vec<_>>(),
        [None, Some(5_000), None],
        "a list reads the expiry of each: the old key, then the two new by id"
    );
}

#[test]
fn a_touch_sets_the_time_of_a_key_that_has_none_and_of_one_with_an_earlier_time() {
    let (_dir, registry) = served(registry());
    assert!(touched(&registry, LOADER, 600), "a key never used");
    assert_eq!(key(&registry, LOADER).last_used_at, Some(600), "written");
    assert!(touched(&registry, LOADER, 1_200), "a later time");
    assert_eq!(key(&registry, LOADER).last_used_at, Some(1_200), "moved");
    assert_eq!(
        key(&registry, OPERATOR).last_used_at,
        None,
        "no other key is touched"
    );
}

// What must be refused.

#[test]
fn a_touch_with_the_stored_time_or_an_earlier_one_changes_nothing() {
    let (_dir, registry) = served(registry());
    assert!(touched(&registry, LOADER, 1_200), "the first");
    assert!(!touched(&registry, LOADER, 1_200), "the same time");
    assert!(!touched(&registry, LOADER, 600), "an earlier time");
    assert_eq!(key(&registry, LOADER).last_used_at, Some(1_200), "kept");
}

#[test]
fn a_touch_of_a_key_nobody_has_changes_nothing() {
    let (_dir, registry) = served(registry());
    assert!(!touched(&registry, KeyId([0x77; 8]), 600), "no row");
    assert_eq!(
        number(
            &registry.local_conn(),
            "SELECT count(*) FROM api_key WHERE last_used_at IS NOT NULL"
        ),
        0,
        "no key has a time"
    );
}

#[test]
fn an_upgrade_that_fails_at_the_second_column_leaves_version_8_whole() {
    let old = registry();
    old.raw()
        .execute_batch("ALTER TABLE api_key ADD COLUMN last_used_at INTEGER")
        .expect("a column in the way of the second step");
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::Sqlite(_))),
        "the upgrade fails on the column in its way"
    );
    let after = old.raw();
    assert_eq!(version(&after), BEFORE, "still version 8");
    assert_eq!(
        declared(&after),
        before,
        "the first column is not kept either"
    );
}

#[test]
fn a_wrong_key_upgrades_no_version_8_registry() {
    let old = registry();
    let other = Key::generate().expect("key");
    assert!(
        matches!(open(&old.path(), &other, ""), Err(CatalogError::WrongKey)),
        "the wrong key is named"
    );
    assert_eq!(version(&old.raw()), BEFORE, "still version 8");
    assert!(
        !columns(&old.raw(), "api_key").contains(&"expires_at".to_owned()),
        "no column was added"
    );
}

#[test]
fn the_version_after_this_build_s_is_refused_and_left_as_it_was() {
    let next = SCHEMA_VERSION + 1;
    let old = Old::at(next, &[REGISTRY_V4, USERS_V5, AUDIT_V5]);
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::UnknownSchema(found)) if found == next),
        "a version this build does not know"
    );
    assert_eq!(version(&old.raw()), next, "the number is left");
    assert_eq!(declared(&old.raw()), before, "the tables are left");
}

#[test]
fn this_build_makes_a_version_a_version_8_build_refuses() {
    // A build at version 8 opens versions 0 to 8 and refuses every other
    // number, so it cannot make a key and leave it without the expiry this
    // build would have given it.
    let conn = registry().upgraded();
    assert!(version(&conn) > BEFORE, "the number: {}", version(&conn));
}

#[test]
fn a_second_key_with_one_id_is_refused_as_before() {
    let (_dir, registry) = served(registry());
    kept(&registry, &row(0x21, Some(5_000)));
    let mut tx = registry.immediate().expect("a transaction");
    assert!(
        tx.insert_key(&row(0x21, None)).is_err(),
        "the id is the primary key"
    );
}

// The edges.

#[test]
fn a_version_8_tenant_catalog_changes_only_its_number() {
    let old = Old::at(BEFORE, &[TENANT_V4, AUDIT_V5]);
    let before = declared(&old.raw());
    let conn = old.upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "the number");
    assert_eq!(
        declared(&conn),
        before,
        "a tenant catalog has no key table, so no new column"
    );
}

#[test]
fn registries_stamped_with_every_older_version_gain_the_columns() {
    for from in 1..SCHEMA_VERSION {
        let old = Old::at(from, &[]);
        old.raw()
            .execute_batch(
                "CREATE TABLE tenant (id BLOB PRIMARY KEY NOT NULL);
                 CREATE TABLE api_key (id BLOB PRIMARY KEY NOT NULL);",
            )
            .expect("a tenant table and a key table");
        let conn = old.upgraded();
        assert_eq!(version(&conn), SCHEMA_VERSION, "from {from}");
        assert_eq!(
            columns(&conn, "api_key"),
            ["id", "expires_at", "last_used_at"],
            "from {from}"
        );
    }
}

#[test]
fn a_catalog_with_no_key_table_gains_no_column_and_no_table() {
    for from in 1..SCHEMA_VERSION {
        let old = Old::at(from, &[]);
        old.raw()
            .execute_batch("CREATE TABLE tenant (id BLOB PRIMARY KEY NOT NULL)")
            .expect("a tenant table");
        let conn = old.upgraded();
        assert_eq!(version(&conn), SCHEMA_VERSION, "from {from}");
        assert!(
            !tables(&conn).contains(&"api_key".to_owned()),
            "from {from}: the upgrade adds columns to a table, never the table"
        );
    }
}

#[test]
fn opening_an_upgraded_version_8_registry_again_keeps_the_times_written() {
    let old = registry();
    let first = old.upgraded();
    first
        .execute_batch(
            "UPDATE api_key SET expires_at = 9000, last_used_at = 600 WHERE label = 'loader'",
        )
        .expect("times written by this build");
    let text = declared(&first);
    drop(first);
    let second = old.upgraded();
    assert_eq!(declared(&second), text, "the same tables");
    assert_eq!(
        rows(
            &second,
            "SELECT expires_at, last_used_at FROM api_key ORDER BY id"
        ),
        ["9000|600", "NULL|NULL"],
        "the same times"
    );
}

#[test]
fn a_touch_in_a_transaction_that_is_dropped_is_not_kept() {
    let (_dir, registry) = served(registry());
    {
        let mut tx = registry.immediate().expect("a transaction");
        assert!(tx.touch_key(LOADER, 600).expect("touched"), "changed");
    }
    assert_eq!(
        key(&registry, LOADER).last_used_at,
        None,
        "nothing was committed"
    );
}

#[test]
fn a_revoked_key_keeps_its_revocation_through_the_upgrade_and_a_touch() {
    let (_dir, registry) = served(registry());
    assert!(touched(&registry, OPERATOR, 600), "the row is there");
    let operator = key(&registry, OPERATOR);
    assert_eq!(operator.revoked_at, Some(11), "still revoked");
    assert_eq!(operator.last_used_at, Some(600), "the time asked");
}
