//! The upgrade to schema version 6, tested from a server.sqlite made as
//! schema version 5 made it: the tables below are that version's text,
//! kept here so the upgrade is always run on what it will meet. No
//! release wrote version 5; a build of `main` between releases 0.4.3 and
//! the next did.
//!
//! Version 6 adds a tenant's role cap to server.sqlite: the column
//! `role_cap` of the tenant table, NULL for a tenant with no cap.

use rusqlite::Connection;

use super::v5_tests::{
    Old, REGISTRY_ROWS, REGISTRY_V4, TENANT_V4, declared, number, rows, tables, version,
};
use super::{CatalogError, SCHEMA_VERSION, open};
use crate::backend::Registry;
use crate::backend::local::LocalRegistry;
use crate::envelope::Key;

/// The users and memberships tables of schema version 5.
const USERS_V5: &str = "
CREATE TABLE user_account (
    id BLOB PRIMARY KEY NOT NULL,
    label TEXT NOT NULL,
    issuer TEXT,
    subject TEXT,
    created_at INTEGER NOT NULL,
    deactivated_at INTEGER,
    CHECK ((issuer IS NULL) = (subject IS NULL))
);
CREATE UNIQUE INDEX user_account_identity ON user_account (issuer, subject)
    WHERE issuer IS NOT NULL AND deactivated_at IS NULL;
CREATE TABLE membership (
    user BLOB NOT NULL,
    tenant BLOB,
    roles TEXT NOT NULL,
    granted_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX membership_in_tenant ON membership (user, tenant)
    WHERE tenant IS NOT NULL;
CREATE UNIQUE INDEX membership_as_operator ON membership (user)
    WHERE tenant IS NULL;
CREATE INDEX membership_of_tenant ON membership (tenant);
";

/// Every catalog's audit log at schema version 5.
const AUDIT_V5: &str = "
CREATE TABLE audit (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT,
    version TEXT,
    status INTEGER,
    count INTEGER NOT NULL
);
CREATE INDEX audit_counted ON audit (actor, action, status, at);
";

/// Rows a version-5 server.sqlite could hold besides those of version 4:
/// a second tenant, a user, and the user's membership in the first tenant.
const ROWS_V5: &str = "
INSERT INTO tenant VALUES (x'b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2', 'beta', NULL, 8, x'0505050505050505',
    x'feed', NULL, 8, NULL);
INSERT INTO user_account VALUES (x'c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3', 'Reader', NULL, NULL, 12, NULL);
INSERT INTO membership VALUES (x'c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3', x'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1',
    'reader,legal', 13);
INSERT INTO audit (at, actor, action, target, version, status, count) VALUES
    ('2026-11-11T10:00:00Z', 'cli', 'user_add', 'c3c3', NULL, 201, 1),
    ('2026-11-11T10:02:00Z', '', 'listener_full', 'listen', NULL, NULL, 2);
";

fn registry() -> Old {
    Old::at(
        5,
        &[REGISTRY_V4, USERS_V5, AUDIT_V5, REGISTRY_ROWS, ROWS_V5],
    )
}

const TENANT_READ: &str = "SELECT id, name, retention, created_at, tenant_key_id, \
     wrapped_tenant_key, pending_wrapped_tenant_key, key_since, metadata FROM tenant ORDER BY id";

/// The columns of a table, in order.
fn columns(conn: &Connection, table: &str) -> Vec<String> {
    rows(
        conn,
        &format!("SELECT name FROM pragma_table_info('{table}')"),
    )
}

// What must work.

#[test]
fn a_version_5_registry_gains_the_role_cap_column() {
    let conn = registry().upgraded();
    assert_eq!(version(&conn), 6, "the upgraded registry's version");
    assert_eq!(
        columns(&conn, "tenant").last().map(String::as_str),
        Some("role_cap"),
        "the new column, after the others"
    );
}

#[test]
fn every_tenant_of_an_upgraded_registry_has_no_cap() {
    let conn = registry().upgraded();
    assert_eq!(
        rows(&conn, "SELECT name, role_cap FROM tenant ORDER BY name"),
        ["acme|NULL", "beta|NULL"],
        "NULL: every tenant role is allowed, as before the upgrade"
    );
}

#[test]
fn a_version_5_registry_keeps_its_tenants_as_they_were() {
    let old = registry();
    let before = rows(&old.raw(), TENANT_READ);
    assert_eq!(before.len(), 2, "two tenants to keep");
    assert_eq!(rows(&old.upgraded(), TENANT_READ), before, "every column");
}

#[test]
fn a_version_5_registry_keeps_its_users_memberships_keys_and_log() {
    let old = registry();
    let read = |conn: &Connection| {
        [
            rows(conn, "SELECT * FROM user_account ORDER BY id"),
            rows(conn, "SELECT * FROM membership ORDER BY user"),
            rows(conn, "SELECT * FROM api_key ORDER BY id"),
            rows(conn, "SELECT * FROM master_key_use"),
            rows(conn, "SELECT * FROM audit ORDER BY seq"),
        ]
    };
    let before = read(&old.raw());
    assert!(before.iter().all(|table| !table.is_empty()), "rows to keep");
    assert_eq!(read(&old.upgraded()), before, "every row of every table");
}

#[test]
fn an_upgraded_version_5_registry_is_declared_as_a_new_one_is() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalRegistry::open(dir.path(), Key::generate().expect("key")).expect("a new one");
    assert_eq!(
        declared(&registry().upgraded()),
        declared(&new.local_conn()),
        "tables and indexes, with their text"
    );
}

#[test]
fn a_new_registry_is_made_at_version_6_with_the_column() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalRegistry::open(dir.path(), Key::generate().expect("key")).expect("a new one");
    let conn = new.local_conn();
    assert_eq!(version(&conn), 6, "a new registry's version");
    assert!(
        columns(&conn, "tenant").contains(&"role_cap".to_owned()),
        "the column is in a new registry"
    );
}

#[test]
fn the_column_of_an_upgraded_registry_takes_a_cap_and_gives_it_up() {
    let conn = registry().upgraded();
    conn.execute(
        "UPDATE tenant SET role_cap = 'reader,auditor' WHERE name = 'acme'",
        [],
    )
    .expect("a cap");
    assert_eq!(
        rows(&conn, "SELECT name, role_cap FROM tenant ORDER BY name"),
        ["acme|reader,auditor", "beta|NULL"],
        "one tenant's cap"
    );
    conn.execute("UPDATE tenant SET role_cap = NULL", [])
        .expect("no cap");
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM tenant WHERE role_cap IS NOT NULL"
        ),
        0,
        "NULL again"
    );
}

#[test]
fn a_0_4_2_registry_reaches_version_6_in_one_step() {
    let old = Old::registry();
    let conn = old.upgraded();
    assert_eq!(version(&conn), 6, "from version 4");
    assert!(
        tables(&conn).contains(&"membership".to_owned()),
        "version 5's tables"
    );
    assert_eq!(
        rows(&conn, "SELECT name, role_cap FROM tenant"),
        ["acme|NULL"],
        "version 6's column"
    );
}

#[test]
fn registries_stamped_with_every_older_version_gain_the_column() {
    for from in 1..=5 {
        let old = Old::at(from, &[]);
        old.raw()
            .execute_batch("CREATE TABLE tenant (id BLOB PRIMARY KEY NOT NULL)")
            .expect("a tenant table");
        let conn = old.upgraded();
        assert_eq!(version(&conn), 6, "from {from}");
        assert!(
            columns(&conn, "tenant").contains(&"role_cap".to_owned()),
            "from {from}"
        );
    }
}

#[test]
fn a_re_encrypted_upgraded_registry_keeps_the_column_and_a_cap() {
    let old = registry();
    let conn = old.upgraded();
    conn.execute(
        "UPDATE tenant SET role_cap = 'legal' WHERE name = 'beta'",
        [],
    )
    .expect("a cap");
    drop(conn);
    let new_key = Key::generate().expect("key");
    super::reencrypt(&old.path(), &old.key, &new_key).expect("re-encrypted");
    let conn = open(&old.path(), &new_key, "").expect("opens under the new key");
    assert_eq!(version(&conn), 6, "the version after re-encryption");
    assert_eq!(
        rows(&conn, "SELECT name, role_cap FROM tenant ORDER BY name"),
        ["acme|NULL", "beta|legal"],
        "the caps after re-encryption"
    );
}

#[test]
fn opening_an_upgraded_version_5_registry_again_changes_nothing() {
    let old = registry();
    let first = old.upgraded();
    let (text, tenants) = (declared(&first), rows(&first, "SELECT * FROM tenant"));
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
fn an_upgraded_registry_serves_the_role_cap_calls() {
    let old = registry();
    drop(old.upgraded());
    let Old { dir, key } = old;
    let registry = LocalRegistry::open(dir.path(), key).expect("the upgraded registry opens");
    let mut tx = registry.immediate().expect("a transaction");
    let acme = crate::ids::TenantId([0xa1; 16]);
    assert_eq!(
        tx.tenant(acme).expect("read").expect("acme").role_cap,
        None,
        "no cap after the upgrade"
    );
    assert_eq!(
        tx.set_role_cap(acme, Some("reader")),
        Ok(true),
        "the cap is set"
    );
    assert_eq!(
        tx.tenant(acme)
            .expect("read")
            .expect("acme")
            .role_cap
            .as_deref(),
        Some("reader"),
        "and read back"
    );
}

// What must be refused.

#[test]
fn an_upgrade_that_fails_at_the_column_leaves_version_5_whole() {
    let old = registry();
    old.raw()
        .execute_batch("ALTER TABLE tenant ADD COLUMN role_cap INTEGER")
        .expect("a column in the way");
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::Sqlite(_))),
        "the upgrade fails on the column in its way"
    );
    let after = old.raw();
    assert_eq!(version(&after), 5, "still version 5");
    assert_eq!(declared(&after), before, "nothing of the upgrade is kept");
}

#[test]
fn a_failed_upgrade_from_0_4_2_keeps_none_of_its_steps() {
    let old = Old::registry();
    old.raw()
        .execute_batch("ALTER TABLE tenant ADD COLUMN role_cap INTEGER")
        .expect("a column in the way");
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::Sqlite(_))),
        "the last step fails"
    );
    let after = old.raw();
    assert_eq!(version(&after), 4, "still version 4");
    assert_eq!(
        declared(&after),
        before,
        "the user tables and the new log made earlier in the transaction are gone"
    );
    assert_eq!(
        number(&after, "SELECT count(*) FROM audit WHERE status = 0"),
        2,
        "the log still holds its zeros"
    );
}

#[test]
fn a_wrong_key_upgrades_no_version_5_registry() {
    let old = registry();
    let other = Key::generate().expect("key");
    assert!(
        matches!(open(&old.path(), &other, ""), Err(CatalogError::WrongKey)),
        "the wrong key is named"
    );
    assert_eq!(version(&old.raw()), 5, "still version 5");
    assert!(
        !columns(&old.raw(), "tenant").contains(&"role_cap".to_owned()),
        "no column was added"
    );
}

#[test]
fn version_7_is_refused_and_left_as_it_was() {
    let old = Old::at(7, &[REGISTRY_V4, USERS_V5, AUDIT_V5]);
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::UnknownSchema(7))),
        "a version this build does not know"
    );
    assert_eq!(version(&old.raw()), 7, "the number is left");
    assert_eq!(declared(&old.raw()), before, "the tables are left");
}

#[test]
fn this_build_makes_a_version_a_version_5_build_refuses() {
    // A build at version 5 opens versions 0 to 5 and refuses every other
    // number, so it cannot write a tenant row without the column's place.
    let conn = registry().upgraded();
    assert!(version(&conn) > 5, "the number: {}", version(&conn));
    assert_eq!(SCHEMA_VERSION, 6, "this build's version");
}

#[test]
fn the_column_does_not_check_what_it_is_given() {
    // The registry's code is what refuses a cap that names no role
    // (`Roles::parse`); the column itself is plain text.
    let conn = registry().upgraded();
    conn.execute("UPDATE tenant SET role_cap = 'owner'", [])
        .expect("any text is stored");
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM tenant WHERE role_cap = 'owner'"
        ),
        2,
        "both rows"
    );
}

// Limits and odd inputs.

#[test]
fn a_version_5_tenant_catalog_changes_only_its_number() {
    let old = Old::at(5, &[TENANT_V4, AUDIT_V5]);
    let before = declared(&old.raw());
    let conn = old.upgraded();
    assert_eq!(version(&conn), 6, "the number");
    assert_eq!(
        declared(&conn),
        before,
        "a tenant catalog has no tenant table"
    );
}

#[test]
fn a_version_5_registry_with_no_tenant_gains_the_column() {
    let old = Old::at(5, &[REGISTRY_V4, USERS_V5, AUDIT_V5]);
    let conn = old.upgraded();
    assert_eq!(number(&conn, "SELECT count(*) FROM tenant"), 0, "no tenant");
    assert!(
        columns(&conn, "tenant").contains(&"role_cap".to_owned()),
        "the column is there for the first tenant"
    );
}

#[test]
fn a_version_5_catalog_with_no_tenant_table_changes_only_its_number() {
    let old = Old::at(5, &["CREATE TABLE note (text TEXT)"]);
    let before = declared(&old.raw());
    let conn = old.upgraded();
    assert_eq!(version(&conn), 6, "the number");
    assert_eq!(declared(&conn), before, "nothing else");
}

#[test]
fn a_registry_of_many_tenants_keeps_each_and_gives_each_no_cap() {
    let old = registry();
    {
        let conn = old.raw();
        for n in 0..100_u8 {
            conn.execute(
                "INSERT INTO tenant VALUES (?1, ?2, NULL, 1, x'01', x'02', NULL, 1, NULL)",
                rusqlite::params![vec![n; 16], format!("tenant-{n}")],
            )
            .expect("a tenant");
        }
    }
    let conn = old.upgraded();
    assert_eq!(
        number(&conn, "SELECT count(*) FROM tenant"),
        102,
        "all kept"
    );
    assert_eq!(
        number(&conn, "SELECT count(*) FROM tenant WHERE role_cap IS NULL"),
        102,
        "none has a cap"
    );
}

#[test]
fn tenant_rows_still_in_the_wal_are_upgraded_too() {
    let old = registry();
    let writer = old.raw();
    writer
        .execute(
            "INSERT INTO tenant VALUES (x'd4d4d4d4d4d4d4d4d4d4d4d4d4d4d4d4', 'gamma', NULL, 9,
                 x'06', x'07', NULL, 9, NULL)",
            [],
        )
        .expect("a row not yet checkpointed");
    let conn = old.upgraded();
    assert_eq!(
        rows(&conn, "SELECT name, role_cap FROM tenant ORDER BY name"),
        ["acme|NULL", "beta|NULL", "gamma|NULL"],
        "the row in the WAL too"
    );
    drop(writer);
}
