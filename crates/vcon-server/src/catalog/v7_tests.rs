//! The upgrade to schema version 7, tested from a server.sqlite made as
//! schema version 6 made it: version 5's text (kept in `v6_tests`) with
//! the role cap's column added as that version's upgrade added it. No
//! release wrote version 6; a build of `main` between releases 0.4.3 and
//! the next did.
//!
//! Version 7 adds the settings a tenant's administrator sets to
//! server.sqlite: the table `tenant_setting`, with one row for each name a
//! tenant has set.

use rusqlite::Connection;

use super::v5_tests::{
    Old, REGISTRY_ROWS, REGISTRY_V4, TENANT_V4, declared, number, rows, tables, version,
};
use super::v6_tests::{AUDIT_V5, ROWS_V5, USERS_V5, columns};
use super::{CatalogError, SCHEMA_VERSION, open};
use crate::backend::Registry;
use crate::backend::local::LocalRegistry;
use crate::envelope::Key;
use crate::ids::TenantId;
use crate::tenants::TenantError;

/// What version 6's upgrade did to the tenant table, and a cap a
/// version-6 build could have set.
const ROLE_CAP_V6: &str = "
ALTER TABLE tenant ADD COLUMN role_cap TEXT;
UPDATE tenant SET role_cap = 'reader,auditor' WHERE name = 'acme';
";

const ACME: TenantId = TenantId([0xa1; 16]);
const BETA: TenantId = TenantId([0xb2; 16]);

fn registry() -> Old {
    Old::at(
        6,
        &[
            REGISTRY_V4,
            USERS_V5,
            AUDIT_V5,
            REGISTRY_ROWS,
            ROWS_V5,
            ROLE_CAP_V6,
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

fn pairs(of: &[(&str, &str)]) -> Vec<(String, String)> {
    of.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

const SETTING_READ: &str =
    "SELECT hex(tenant), name, value FROM tenant_setting ORDER BY tenant, name";

// What must work.

#[test]
fn a_version_6_registry_gains_the_settings_table() {
    let conn = registry().upgraded();
    assert_eq!(version(&conn), 7, "the upgraded registry's version");
    assert_eq!(
        columns(&conn, "tenant_setting"),
        ["tenant", "name", "value"],
        "the new table's columns"
    );
}

#[test]
fn the_settings_table_of_an_upgraded_registry_starts_empty() {
    let conn = registry().upgraded();
    assert_eq!(
        number(&conn, "SELECT count(*) FROM tenant_setting"),
        0,
        "no tenant has set a name: each has every default"
    );
}

#[test]
fn a_version_6_registry_keeps_every_row_it_had() {
    let old = registry();
    let read = |conn: &Connection| {
        [
            rows(conn, "SELECT * FROM tenant ORDER BY id"),
            rows(conn, "SELECT * FROM user_account ORDER BY id"),
            rows(conn, "SELECT * FROM membership ORDER BY user"),
            rows(conn, "SELECT * FROM api_key ORDER BY id"),
            rows(conn, "SELECT * FROM master_key_use"),
            rows(conn, "SELECT * FROM audit ORDER BY seq"),
        ]
    };
    let before = read(&old.raw());
    assert!(before.iter().all(|table| !table.is_empty()), "rows to keep");
    assert!(
        before[0].iter().any(|row| row.ends_with("reader,auditor")),
        "a role cap among them: {:?}",
        before[0]
    );
    assert_eq!(read(&old.upgraded()), before, "every row of every table");
}

#[test]
fn an_upgraded_version_6_registry_is_declared_as_a_new_one_is() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalRegistry::open(dir.path(), Key::generate().expect("key")).expect("a new one");
    assert_eq!(
        declared(&registry().upgraded()),
        declared(&new.local_conn()),
        "tables and indexes, with their text"
    );
}

#[test]
fn a_new_registry_is_made_at_version_7_with_the_table() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalRegistry::open(dir.path(), Key::generate().expect("key")).expect("a new one");
    let conn = new.local_conn();
    assert_eq!(version(&conn), 7, "a new registry's version");
    assert!(
        tables(&conn).contains(&"tenant_setting".to_owned()),
        "the table is in a new registry"
    );
}

#[test]
fn a_0_4_3_registry_reaches_version_7_in_one_step() {
    let conn = Old::registry().upgraded();
    assert_eq!(version(&conn), 7, "from version 4");
    assert!(
        tables(&conn).contains(&"membership".to_owned()),
        "version 5's tables"
    );
    assert!(
        columns(&conn, "tenant").contains(&"role_cap".to_owned()),
        "version 6's column"
    );
    assert!(
        tables(&conn).contains(&"tenant_setting".to_owned()),
        "version 7's table"
    );
}

#[test]
fn an_upgraded_registry_keeps_a_setting_and_reads_it_back() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    assert_eq!(tx.settings(ACME), Ok(vec![]), "none after the upgrade");
    assert_eq!(
        tx.put_setting(ACME, "session.idle_seconds", "300"),
        Ok(()),
        "set"
    );
    assert_eq!(
        tx.settings(ACME),
        Ok(pairs(&[("session.idle_seconds", "300")])),
        "read back"
    );
}

#[test]
fn a_setting_written_again_holds_the_last_value_in_one_row() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    tx.put_setting(ACME, "masking.legal", "false").expect("set");
    tx.put_setting(ACME, "masking.legal", "true")
        .expect("set again");
    assert_eq!(
        tx.settings(ACME),
        Ok(pairs(&[("masking.legal", "true")])),
        "one row, the last value"
    );
}

#[test]
fn a_removed_setting_has_no_row() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    tx.put_setting(ACME, "masking.legal", "false").expect("set");
    tx.put_setting(ACME, "masking.reader", "false")
        .expect("set");
    assert_eq!(tx.delete_setting(ACME, "masking.legal"), Ok(()), "removed");
    assert_eq!(
        tx.settings(ACME),
        Ok(pairs(&[("masking.reader", "false")])),
        "the other stays"
    );
}

#[test]
fn a_tenants_settings_come_by_name() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    for name in ["session.idle_seconds", "masking.reader", "masking.legal"] {
        tx.put_setting(ACME, name, "x").expect("set");
    }
    let names: Vec<String> = tx
        .settings(ACME)
        .expect("read")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(
        names,
        ["masking.legal", "masking.reader", "session.idle_seconds"],
        "one order, whatever the order of writing"
    );
}

#[test]
fn settings_written_in_a_transaction_are_there_after_its_commit() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    tx.put_setting(BETA, "masking.auditor", "false")
        .expect("set");
    tx.commit().expect("commit");
    assert_eq!(
        rows(&registry.local_conn(), SETTING_READ),
        ["B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2B2|masking.auditor|false"],
        "the row, under its tenant"
    );
}

#[test]
fn a_re_encrypted_upgraded_registry_keeps_its_settings() {
    let old = registry();
    let conn = old.upgraded();
    conn.execute(
        "INSERT INTO tenant_setting VALUES (x'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1', 'masking.legal', 'false')",
        [],
    )
    .expect("a setting");
    drop(conn);
    let new_key = Key::generate().expect("key");
    super::reencrypt(&old.path(), &old.key, &new_key).expect("re-encrypted");
    let conn = open(&old.path(), &new_key, "").expect("opens under the new key");
    assert_eq!(version(&conn), 7, "the version after re-encryption");
    assert_eq!(
        rows(&conn, SETTING_READ),
        ["A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1|masking.legal|false"],
        "the setting after re-encryption"
    );
}

// What must be refused.

#[test]
fn an_upgrade_that_fails_at_the_table_leaves_version_6_whole() {
    let old = registry();
    old.raw()
        .execute_batch("CREATE TABLE tenant_setting (other TEXT)")
        .expect("a table in the way");
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::Sqlite(_))),
        "the upgrade fails on the table in its way"
    );
    let after = old.raw();
    assert_eq!(version(&after), 6, "still version 6");
    assert_eq!(declared(&after), before, "nothing of the upgrade is kept");
}

#[test]
fn a_failed_upgrade_from_0_4_3_keeps_none_of_its_steps() {
    let old = Old::registry();
    old.raw()
        .execute_batch("CREATE TABLE tenant_setting (other TEXT)")
        .expect("a table in the way");
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
        "the user tables and the role cap's column made earlier in the transaction are gone"
    );
}

#[test]
fn a_wrong_key_upgrades_no_version_6_registry() {
    let old = registry();
    let other = Key::generate().expect("key");
    assert!(
        matches!(open(&old.path(), &other, ""), Err(CatalogError::WrongKey)),
        "the wrong key is named"
    );
    assert_eq!(version(&old.raw()), 6, "still version 6");
    assert!(
        !tables(&old.raw()).contains(&"tenant_setting".to_owned()),
        "no table was added"
    );
}

#[test]
fn version_8_is_refused_and_left_as_it_was() {
    let old = Old::at(8, &[REGISTRY_V4, USERS_V5, AUDIT_V5]);
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::UnknownSchema(8))),
        "a version this build does not know"
    );
    assert_eq!(version(&old.raw()), 8, "the number is left");
    assert_eq!(declared(&old.raw()), before, "the tables are left");
}

#[test]
fn this_build_makes_a_version_a_version_6_build_refuses() {
    // A build at version 6 opens versions 0 to 6 and refuses every other
    // number, so it cannot remove a tenant and leave the tenant's settings.
    let conn = registry().upgraded();
    assert!(version(&conn) > 6, "the number: {}", version(&conn));
    assert_eq!(SCHEMA_VERSION, 7, "this build's version");
}

#[test]
fn the_table_refuses_a_second_row_for_a_tenants_name() {
    let conn = registry().upgraded();
    let insert = "INSERT INTO tenant_setting VALUES (x'a1', 'masking.legal', 'false')";
    conn.execute(insert, []).expect("the first row");
    let second = conn.execute(insert, []).expect_err("the second row");
    assert!(
        second.to_string().contains("UNIQUE"),
        "one row for a tenant's name: {second}"
    );
}

#[test]
fn the_table_refuses_a_row_with_no_tenant_name_or_value() {
    let conn = registry().upgraded();
    for row in [
        "(NULL, 'masking.legal', 'false')",
        "(x'a1', NULL, 'false')",
        "(x'a1', 'masking.legal', NULL)",
    ] {
        let refused = conn
            .execute(&format!("INSERT INTO tenant_setting VALUES {row}"), [])
            .expect_err("a row with a NULL");
        assert!(refused.to_string().contains("NOT NULL"), "{row}: {refused}");
    }
}

#[test]
fn a_setting_read_from_a_registry_without_the_table_is_an_error_not_a_default() {
    let (_dir, registry) = served(registry());
    registry
        .local_conn()
        .execute_batch("DROP TABLE tenant_setting")
        .expect("the table is gone");
    let mut tx = registry.immediate().expect("a transaction");
    assert_eq!(
        tx.settings(ACME),
        Err(TenantError::Io),
        "a registry that cannot answer"
    );
    assert_eq!(
        tx.put_setting(ACME, "masking.legal", "false"),
        Err(TenantError::Io),
        "nor take a setting"
    );
    assert_eq!(
        tx.delete_setting(ACME, "masking.legal"),
        Err(TenantError::Io),
        "nor remove one"
    );
    assert_eq!(
        tx.delete_settings_in(ACME),
        Err(TenantError::Io),
        "nor remove a tenant's"
    );
}

#[test]
fn settings_written_in_a_transaction_that_is_dropped_are_not_kept() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    tx.put_setting(ACME, "masking.legal", "false").expect("set");
    drop(tx);
    assert_eq!(
        number(
            &registry.local_conn(),
            "SELECT count(*) FROM tenant_setting"
        ),
        0,
        "rolled back"
    );
}

#[test]
fn a_blob_value_put_by_hand_does_not_read_as_a_setting() {
    let (_dir, registry) = served(registry());
    registry
        .local_conn()
        .execute(
            "INSERT INTO tenant_setting VALUES (x'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1', 'masking.legal', x'00ff')",
            [],
        )
        .expect("a value that is no text");
    let mut tx = registry.immediate().expect("a transaction");
    assert_eq!(
        tx.settings(ACME),
        Err(TenantError::Io),
        "damage, not a value"
    );
}

#[test]
fn removing_a_setting_of_another_tenant_by_name_removes_nothing() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    tx.put_setting(ACME, "masking.legal", "false").expect("set");
    assert_eq!(
        tx.delete_setting(BETA, "masking.legal"),
        Ok(()),
        "no row of beta's has the name"
    );
    assert_eq!(
        tx.settings(ACME),
        Ok(pairs(&[("masking.legal", "false")])),
        "acme's row stays"
    );
}

// Limits and odd inputs.

#[test]
fn a_version_6_tenant_catalog_changes_only_its_number() {
    let old = Old::at(6, &[TENANT_V4, AUDIT_V5]);
    let before = declared(&old.raw());
    let conn = old.upgraded();
    assert_eq!(version(&conn), 7, "the number");
    assert_eq!(
        declared(&conn),
        before,
        "a tenant catalog has no tenant table, so no settings table"
    );
}

#[test]
fn a_version_6_registry_with_no_tenant_gains_the_table() {
    let old = Old::at(6, &[REGISTRY_V4, USERS_V5, AUDIT_V5, ROLE_CAP_V6]);
    let conn = old.upgraded();
    assert_eq!(number(&conn, "SELECT count(*) FROM tenant"), 0, "no tenant");
    assert!(
        tables(&conn).contains(&"tenant_setting".to_owned()),
        "the table is there for the first tenant"
    );
}

#[test]
fn registries_stamped_with_every_older_version_gain_the_table() {
    for from in 1..=6 {
        let old = Old::at(from, &[]);
        old.raw()
            .execute_batch("CREATE TABLE tenant (id BLOB PRIMARY KEY NOT NULL)")
            .expect("a tenant table");
        let conn = old.upgraded();
        assert_eq!(version(&conn), 7, "from {from}");
        assert!(
            tables(&conn).contains(&"tenant_setting".to_owned()),
            "from {from}"
        );
    }
}

#[test]
fn opening_an_upgraded_version_6_registry_again_changes_nothing() {
    let old = registry();
    let first = old.upgraded();
    first
        .execute(
            "INSERT INTO tenant_setting VALUES (x'a1', 'masking.legal', 'false')",
            [],
        )
        .expect("a setting");
    let (text, settings) = (declared(&first), rows(&first, SETTING_READ));
    drop(first);
    let second = old.upgraded();
    assert_eq!(declared(&second), text, "the same tables");
    assert_eq!(rows(&second, SETTING_READ), settings, "the same rows");
}

#[test]
fn one_name_in_two_tenants_is_two_rows() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    tx.put_setting(ACME, "masking.legal", "false")
        .expect("acme's");
    tx.put_setting(BETA, "masking.legal", "true")
        .expect("beta's");
    assert_eq!(
        tx.settings(ACME),
        Ok(pairs(&[("masking.legal", "false")])),
        "acme reads its own"
    );
    assert_eq!(
        tx.settings(BETA),
        Ok(pairs(&[("masking.legal", "true")])),
        "beta reads its own"
    );
}

#[test]
fn removing_a_tenants_settings_leaves_the_other_tenants() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    tx.put_setting(ACME, "masking.legal", "false")
        .expect("acme's");
    tx.put_setting(ACME, "session.idle_seconds", "300")
        .expect("acme's");
    tx.put_setting(BETA, "masking.legal", "true")
        .expect("beta's");
    assert_eq!(tx.delete_settings_in(ACME), Ok(()), "acme's are removed");
    assert_eq!(tx.settings(ACME), Ok(vec![]), "none of acme's is left");
    assert_eq!(
        tx.settings(BETA),
        Ok(pairs(&[("masking.legal", "true")])),
        "beta's stays"
    );
}

#[test]
fn removing_a_setting_that_was_never_set_is_not_an_error() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    assert_eq!(
        tx.delete_setting(ACME, "masking.legal"),
        Ok(()),
        "nothing to remove"
    );
    assert_eq!(
        tx.delete_settings_in(ACME),
        Ok(()),
        "nor for a tenant with none"
    );
}

#[test]
fn a_tenant_with_no_row_in_the_registry_reads_no_settings() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    assert_eq!(
        tx.settings(TenantId([0xee; 16])),
        Ok(vec![]),
        "the table does not know tenants: the caller checks the tenant"
    );
}

#[test]
fn a_registry_of_many_settings_keeps_each() {
    let (_dir, registry) = served(registry());
    let mut tx = registry.immediate().expect("a transaction");
    for n in 0..200_u8 {
        tx.put_setting(TenantId([n; 16]), "session.idle_seconds", &n.to_string())
            .expect("set");
    }
    tx.commit().expect("commit");
    assert_eq!(
        number(
            &registry.local_conn(),
            "SELECT count(*) FROM tenant_setting"
        ),
        200,
        "one row for each tenant"
    );
}
