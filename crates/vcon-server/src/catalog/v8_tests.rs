//! The upgrade to schema version 8, tested from a server.sqlite made as
//! schema version 7 made it: version 5's text (kept in `v6_tests`) with
//! the role cap's column and the settings table added as those versions'
//! upgrades added them. No release wrote version 7; a build of `main`
//! between releases 0.4.3 and the next did.
//!
//! Version 8 adds the sessions of the users who signed in to
//! server.sqlite: the table `session`, with one row for each live session.

use rusqlite::Connection;

use super::v5_tests::{
    Old, REGISTRY_ROWS, REGISTRY_V4, TENANT_V4, declared, number, rows, tables, version,
};
use super::v6_tests::{AUDIT_V5, ROWS_V5, USERS_V5, columns};
use super::{CatalogError, SCHEMA_VERSION, open};
use crate::backend::local::LocalRegistry;
use crate::backend::{Registry, SessionRow};
use crate::envelope::Key;
use crate::ids::{TenantId, UserId};
use crate::tenants::TenantError;

/// What versions 6 and 7 added to a registry, and a setting a version-7
/// build could have kept.
const ADDED_V7: &str = "
ALTER TABLE tenant ADD COLUMN role_cap TEXT;
CREATE TABLE tenant_setting (
    tenant BLOB NOT NULL,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (tenant, name)
);
INSERT INTO tenant_setting VALUES (x'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1', 'session.idle_seconds', '600');
";

const ACME: TenantId = TenantId([0xa1; 16]);
const BETA: TenantId = TenantId([0xb2; 16]);
const USER: UserId = UserId([0x51; 16]);
const OTHER: UserId = UserId([0x52; 16]);

fn registry() -> Old {
    Old::at(
        7,
        &[
            REGISTRY_V4,
            USERS_V5,
            AUDIT_V5,
            REGISTRY_ROWS,
            ROWS_V5,
            ADDED_V7,
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

/// A session's row, told from others by one byte of its id's hash.
fn row(n: u8, user: UserId, tenant: Option<TenantId>) -> SessionRow {
    SessionRow {
        id_hash: vec![n; 32],
        user,
        tenant,
        scopes: "read".to_owned(),
        created_at: 100,
        last_seen_at: 100,
        idle_expires_at: 1_000,
        expires_at: 5_000,
        idle_seconds: 900,
        amr: "hwk".to_owned(),
        csrf_hash: vec![n ^ 0xff; 32],
    }
}

/// Writes rows in one transaction and commits it.
fn kept(registry: &LocalRegistry, of: &[SessionRow]) {
    let mut tx = registry.immediate().expect("a transaction");
    for row in of {
        tx.insert_session(row).expect("inserted");
    }
    tx.commit().expect("committed");
}

fn live(registry: &LocalRegistry) -> Vec<String> {
    rows(
        &registry.local_conn(),
        "SELECT hex(substr(id_hash, 1, 1)) FROM session ORDER BY id_hash",
    )
}

// What must work.

#[test]
fn a_version_7_registry_gains_the_session_table() {
    let conn = registry().upgraded();
    assert_eq!(
        version(&conn),
        SCHEMA_VERSION,
        "the upgraded registry's version"
    );
    assert_eq!(
        columns(&conn, "session"),
        [
            "id_hash",
            "user",
            "tenant",
            "scopes",
            "created_at",
            "last_seen_at",
            "idle_expires_at",
            "expires_at",
            "idle_seconds",
            "amr",
            "csrf_hash"
        ],
        "the new table's columns"
    );
    assert_eq!(
        number(&conn, "SELECT count(*) FROM session"),
        0,
        "no one is signed in"
    );
}

#[test]
fn a_version_7_registry_keeps_every_row_it_had() {
    let old = registry();
    let read = |conn: &Connection| {
        [
            rows(conn, "SELECT * FROM tenant ORDER BY id"),
            rows(conn, "SELECT * FROM user_account ORDER BY id"),
            rows(conn, "SELECT * FROM membership ORDER BY user"),
            rows(conn, "SELECT * FROM api_key ORDER BY id"),
            rows(conn, "SELECT * FROM tenant_setting"),
            rows(conn, "SELECT * FROM audit ORDER BY seq"),
        ]
    };
    let before = read(&old.raw());
    assert!(before.iter().all(|table| !table.is_empty()), "rows to keep");
    assert_eq!(read(&old.upgraded()), before, "every row of every table");
}

#[test]
fn an_upgraded_version_7_registry_is_declared_as_a_new_one_is() {
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
    for table in ["membership", "tenant_setting", "session"] {
        assert!(tables(&conn).contains(&table.to_owned()), "{table}");
    }
}

#[test]
fn a_session_written_is_read_back_by_the_hash_of_its_id() {
    let (_dir, registry) = served(registry());
    let (one, two) = (row(1, USER, Some(ACME)), row(2, OTHER, None));
    kept(&registry, &[one.clone(), two.clone()]);
    let mut tx = registry.immediate().expect("a transaction");
    assert_eq!(tx.session(&one.id_hash), Ok(Some(one)), "a tenant's");
    assert_eq!(tx.session(&two.id_hash), Ok(Some(two)), "an operator's");
    assert_eq!(tx.session(&[3; 32]), Ok(None), "a hash no session has");
}

#[test]
fn a_session_seen_again_has_its_two_times_moved_and_nothing_else() {
    let (_dir, registry) = served(registry());
    let one = row(1, USER, Some(ACME));
    kept(&registry, &[one.clone(), row(2, USER, Some(ACME))]);
    let mut tx = registry.immediate().expect("a transaction");
    tx.touch_session(&one.id_hash, 400, 1_300).expect("touched");
    let moved = SessionRow {
        last_seen_at: 400,
        idle_expires_at: 1_300,
        ..one.clone()
    };
    assert_eq!(tx.session(&one.id_hash), Ok(Some(moved)), "the one touched");
    assert_eq!(
        tx.session(&[2; 32]),
        Ok(Some(row(2, USER, Some(ACME)))),
        "the other is as it was"
    );
}

#[test]
fn sessions_are_removed_one_by_hash_by_user_and_by_tenant() {
    let (_dir, registry) = served(registry());
    kept(
        &registry,
        &[
            row(1, USER, Some(ACME)),
            row(2, USER, Some(BETA)),
            row(3, OTHER, Some(ACME)),
            row(4, OTHER, Some(BETA)),
            row(5, OTHER, None),
            row(6, OTHER, Some(BETA)),
        ],
    );
    let mut tx = registry.immediate().expect("a transaction");
    tx.delete_session(&[6; 32]).expect("one removed");
    tx.commit().expect("committed");
    assert_eq!(live(&registry), ["01", "02", "03", "04", "05"], "by hash");
    let mut tx = registry.immediate().expect("a transaction");
    tx.delete_sessions_of_user(USER).expect("a user's removed");
    tx.commit().expect("committed");
    assert_eq!(
        live(&registry),
        ["03", "04", "05"],
        "by user, in every tenant"
    );
    let mut tx = registry.immediate().expect("a transaction");
    tx.delete_sessions_in(ACME).expect("a tenant's removed");
    tx.commit().expect("committed");
    assert_eq!(
        live(&registry),
        ["04", "05"],
        "by tenant: the operator's stays"
    );
}

#[test]
fn sessions_that_have_ended_are_removed_by_either_limit() {
    let (_dir, registry) = served(registry());
    let at = |n, idle_expires_at, expires_at| SessionRow {
        idle_expires_at,
        expires_at,
        ..row(n, USER, Some(ACME))
    };
    kept(
        &registry,
        &[
            at(1, 999, 5_000),
            at(2, 1_000, 5_000),
            at(3, 1_001, 5_000),
            at(4, 9_000, 1_000),
            at(5, 9_000, 1_001),
        ],
    );
    let mut tx = registry.immediate().expect("a transaction");
    tx.delete_sessions_ended(1_000).expect("removed");
    tx.commit().expect("committed");
    assert_eq!(
        live(&registry),
        ["03", "05"],
        "a limit at or before the time has passed; one after it has not"
    );
}

// What must be refused.

#[test]
fn an_upgrade_that_fails_at_the_table_leaves_version_7_whole() {
    let old = registry();
    old.raw()
        .execute_batch("CREATE TABLE session (other TEXT)")
        .expect("a table in the way");
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::Sqlite(_))),
        "the upgrade fails on the table in its way"
    );
    let after = old.raw();
    assert_eq!(version(&after), 7, "still version 7");
    assert_eq!(declared(&after), before, "nothing of the upgrade is kept");
}

#[test]
fn a_wrong_key_upgrades_no_version_7_registry() {
    let old = registry();
    let other = Key::generate().expect("key");
    assert!(
        matches!(open(&old.path(), &other, ""), Err(CatalogError::WrongKey)),
        "the wrong key is named"
    );
    assert_eq!(version(&old.raw()), 7, "still version 7");
    assert!(
        !tables(&old.raw()).contains(&"session".to_owned()),
        "no table was added"
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
fn this_build_makes_a_version_a_version_7_build_refuses() {
    // A build at version 7 opens versions 0 to 7 and refuses every other
    // number, so it cannot change a membership and leave its sessions live.
    let conn = registry().upgraded();
    assert!(version(&conn) > 7, "the number: {}", version(&conn));
}

#[test]
fn the_table_refuses_a_second_session_with_one_id_hash() {
    let (_dir, registry) = served(registry());
    kept(&registry, &[row(1, USER, Some(ACME))]);
    let mut tx = registry.immediate().expect("a transaction");
    assert_eq!(
        tx.insert_session(&row(1, OTHER, None)),
        Err(TenantError::Io),
        "one row for an id"
    );
}

#[test]
fn the_table_refuses_a_row_that_lacks_a_part_or_whose_hash_is_not_32_bytes() {
    let conn = registry().upgraded();
    let insert = |values: &str| conn.execute(&format!("INSERT INTO session VALUES ({values})"), []);
    let h = "zeroblob(32)";
    for values in [
        format!("NULL, x'51', NULL, 'read', 1, 1, 2, 3, 1, 'hwk', {h}"),
        format!("{h}, NULL, NULL, 'read', 1, 1, 2, 3, 1, 'hwk', {h}"),
        format!("{h}, x'51', NULL, NULL, 1, 1, 2, 3, 1, 'hwk', {h}"),
        format!("{h}, x'51', NULL, 'read', NULL, 1, 2, 3, 1, 'hwk', {h}"),
        format!("{h}, x'51', NULL, 'read', 1, NULL, 2, 3, 1, 'hwk', {h}"),
        format!("{h}, x'51', NULL, 'read', 1, 1, NULL, 3, 1, 'hwk', {h}"),
        format!("{h}, x'51', NULL, 'read', 1, 1, 2, NULL, 1, 'hwk', {h}"),
        format!("{h}, x'51', NULL, 'read', 1, 1, 2, 3, NULL, 'hwk', {h}"),
        format!("{h}, x'51', NULL, 'read', 1, 1, 2, 3, 1, NULL, {h}"),
        format!("{h}, x'51', NULL, 'read', 1, 1, 2, 3, 1, 'hwk', NULL"),
        format!("zeroblob(31), x'51', NULL, 'read', 1, 1, 2, 3, 1, 'hwk', {h}"),
        format!("{h}, x'51', NULL, 'read', 1, 1, 2, 3, 1, 'hwk', zeroblob(33)"),
    ] {
        assert!(insert(&values).is_err(), "{values}");
    }
    insert(&format!(
        "{h}, x'51', NULL, 'read', 1, 1, 2, 3, 1, 'hwk', {h}"
    ))
    .expect("a whole row with no tenant is taken");
}

#[test]
fn a_session_read_from_a_registry_without_the_table_is_an_error_not_no_session() {
    let (_dir, registry) = served(registry());
    registry
        .local_conn()
        .execute_batch("DROP TABLE session")
        .expect("dropped");
    let mut tx = registry.immediate().expect("a transaction");
    assert_eq!(tx.session(&[1; 32]), Err(TenantError::Io), "a failure");
}

#[test]
fn sessions_written_in_a_transaction_that_is_dropped_are_not_kept() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    tx.insert_session(&row(1, USER, Some(ACME)))
        .expect("inserted");
    drop(tx);
    assert_eq!(live(&registry), [] as [&str; 0], "nothing was committed");
}

// Limits and odd inputs.

#[test]
fn a_version_7_tenant_catalog_changes_only_its_number() {
    let old = Old::at(7, &[TENANT_V4, AUDIT_V5]);
    let before = declared(&old.raw());
    let conn = old.upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "the number");
    assert_eq!(
        declared(&conn),
        before,
        "a tenant catalog has no tenant table, so no session table"
    );
}

#[test]
fn registries_stamped_with_every_older_version_gain_the_table() {
    for from in 1..SCHEMA_VERSION {
        let old = Old::at(from, &[]);
        old.raw()
            .execute_batch("CREATE TABLE tenant (id BLOB PRIMARY KEY NOT NULL)")
            .expect("a tenant table");
        let conn = old.upgraded();
        assert_eq!(version(&conn), SCHEMA_VERSION, "from {from}");
        assert!(tables(&conn).contains(&"session".to_owned()), "from {from}");
    }
}

#[test]
fn opening_an_upgraded_version_7_registry_again_keeps_its_sessions() {
    let old = registry();
    let first = old.upgraded();
    first
        .execute(
            "INSERT INTO session VALUES (zeroblob(32), x'51', NULL, 'admin', 1, 1, 2, 3, 1, \
             'hwk', zeroblob(32))",
            [],
        )
        .expect("a session");
    let text = declared(&first);
    drop(first);
    let second = old.upgraded();
    assert_eq!(declared(&second), text, "the same tables");
    assert_eq!(
        number(&second, "SELECT count(*) FROM session"),
        1,
        "the same row"
    );
}

#[test]
fn removing_the_sessions_of_a_user_or_tenant_that_has_none_is_not_an_error() {
    let (_dir, registry) = served(registry());
    kept(&registry, &[row(1, USER, Some(ACME))]);
    let mut tx = registry.immediate().expect("a transaction");
    tx.delete_sessions_of_user(OTHER).expect("none to remove");
    tx.delete_sessions_in(BETA).expect("none to remove");
    tx.delete_session(&[9; 32]).expect("none to remove");
    tx.delete_sessions_ended(0).expect("none to remove");
    tx.commit().expect("committed");
    assert_eq!(live(&registry), ["01"], "the one session is left");
}

#[test]
fn the_session_table_is_indexed_by_user_and_by_tenant() {
    let conn = registry().upgraded();
    let indexes = rows(
        &conn,
        "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'session' \
         AND name NOT LIKE 'sqlite_%' ORDER BY name",
    );
    assert_eq!(
        indexes,
        ["session_in_tenant", "session_of_user"],
        "the two ways sessions are removed in number"
    );
}
