//! The upgrade to schema version 5, tested from catalogs made as release
//! 0.4.2 made them (schema version 4): the tables below are that release's
//! text, kept here so the upgrade is always run on what it will meet.
//!
//! Version 5 adds the users and memberships tables to server.sqlite, and
//! lets every catalog's audit log hold no status (NULL) where version 4
//! held the number 0.

use rusqlite::Connection;
use time::macros::datetime;

use super::{CatalogError, SCHEMA_VERSION, open, set_key};
use crate::audit::{self, Action, Actor, Entry, NO_ANSWER};
use crate::backend::Registry;
use crate::backend::local::{LocalCatalog, LocalRegistry};
use crate::envelope::Key;

/// server.sqlite's tables in release 0.4.2, besides the audit log.
pub(super) const REGISTRY_V4: &str = "
CREATE TABLE tenant (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    retention TEXT,
    created_at INTEGER NOT NULL,
    tenant_key_id BLOB NOT NULL,
    wrapped_tenant_key BLOB NOT NULL,
    pending_wrapped_tenant_key BLOB,
    key_since INTEGER NOT NULL,
    metadata TEXT
);
CREATE TABLE master_key_use (
    master_key_id BLOB PRIMARY KEY NOT NULL,
    wraps INTEGER NOT NULL,
    since INTEGER NOT NULL
);
CREATE TABLE api_key (
    id BLOB PRIMARY KEY NOT NULL,
    tenant BLOB,
    token_hash BLOB NOT NULL,
    scopes TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE INDEX api_key_tenant ON api_key (tenant);
";

/// A tenant catalog's tables in release 0.4.2, besides the audit log.
pub(super) const TENANT_V4: &str = "
CREATE TABLE vcon (
    uuid BLOB PRIMARY KEY NOT NULL,
    current BLOB,
    deleted_at INTEGER,
    hold INTEGER NOT NULL DEFAULT 0,
    stored_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX vcon_listed ON vcon (stored_at, uuid);
CREATE TABLE version (
    version BLOB PRIMARY KEY NOT NULL,
    uuid BLOB NOT NULL,
    form TEXT NOT NULL,
    gzip INTEGER NOT NULL,
    size INTEGER NOT NULL,
    stored_at INTEGER NOT NULL,
    hidden_at INTEGER,
    tenant_key_id BLOB NOT NULL,
    wrapped_key BLOB NOT NULL,
    place_file BLOB,
    place_group INTEGER,
    place_row INTEGER
);
CREATE INDEX version_uuid ON version (uuid);
CREATE TABLE tombstone (
    uuid BLOB PRIMARY KEY NOT NULL,
    purged_at INTEGER NOT NULL
);
CREATE TABLE key_use (
    tenant_key_id BLOB PRIMARY KEY NOT NULL,
    wraps INTEGER NOT NULL
);
CREATE INDEX version_place ON version (place_file);
CREATE TABLE parquet_file (
    file_id BLOB PRIMARY KEY NOT NULL,
    date TEXT NOT NULL,
    rows INTEGER NOT NULL,
    live_rows INTEGER NOT NULL,
    bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    unnamed_at INTEGER
);
CREATE TABLE purge_pending (
    file_id BLOB PRIMARY KEY NOT NULL
);
CREATE TABLE manifest (
    seq INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    published_at INTEGER
);
";

/// Every catalog's audit log in release 0.4.2.
const AUDIT_V4: &str = "
CREATE TABLE audit (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT,
    version TEXT,
    status INTEGER NOT NULL,
    count INTEGER NOT NULL
);
CREATE INDEX audit_counted ON audit (actor, action, status, at);
";

/// Rows a 0.4.2 server.sqlite could hold: a tenant, a wrap count, a
/// tenant key and an operator key.
/// Every access key with the columns of version 4, in order: version 9
/// adds its own.
pub(super) const KEYS_V4: &str = "SELECT id, tenant, token_hash, scopes, label, created_at, \
                                  revoked_at FROM api_key ORDER BY id";

pub(super) const REGISTRY_ROWS: &str = "
INSERT INTO tenant VALUES (x'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1', 'acme', '30d', 7, x'0101010101010101',
    x'beef', NULL, 7, 'counts');
INSERT INTO master_key_use VALUES (x'0202020202020202', 3, 5);
INSERT INTO api_key VALUES (x'0303030303030303', x'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1', x'cafe',
    'read,write', 'loader', 9, NULL);
INSERT INTO api_key VALUES (x'0404040404040404', NULL, x'f00d', 'admin', 'operator', 10, 11);
";

/// Audit records of a version-4 log: a request with its status, a
/// record with a target and a version, and two that answer no request,
/// which release 0.4.3 writes with the number 0 (`listener_full`, which no
/// release up to 0.4.2 has).
const AUDIT_ROWS: &str = "
INSERT INTO audit (at, actor, action, target, version, status, count) VALUES
    ('2026-11-01T10:00:00Z', 'cli', 'tenant_create', 'a1a1', NULL, 201, 1),
    ('2026-11-01T10:01:00Z', '0303030303030303', 'read', 'u-1', 'v-1', 200, 1),
    ('2026-11-01T10:02:00Z', '', 'listener_full', 'listen', NULL, 0, 4),
    ('2026-11-01T10:02:00Z', '', 'auth_failed', NULL, NULL, 401, 9),
    ('2026-11-01T10:03:00Z', '', 'listener_full', 'operator.listen', NULL, 0, 1);
";

/// A catalog as release 0.4.2 left it.
pub(super) struct Old {
    pub(super) dir: tempfile::TempDir,
    pub(super) key: Key,
}

impl Old {
    /// A catalog file of `sql` at schema version 4, under a new key.
    pub(super) fn of(sql: &[&str]) -> Old {
        Old::at(4, sql)
    }

    pub(super) fn at(version: i64, sql: &[&str]) -> Old {
        let old = Old {
            dir: tempfile::tempdir().expect("temp dir"),
            key: Key::generate().expect("key"),
        };
        let conn = old.raw();
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
            .expect("WAL");
        for part in sql {
            conn.execute_batch(part).expect("the old catalog's text");
        }
        conn.pragma_update(None, "user_version", version)
            .expect("the old version");
        old
    }

    pub(super) fn registry() -> Old {
        Old::of(&[REGISTRY_V4, AUDIT_V4, REGISTRY_ROWS, AUDIT_ROWS])
    }

    fn tenant() -> Old {
        Old::of(&[TENANT_V4, AUDIT_V4, AUDIT_ROWS])
    }

    pub(super) fn path(&self) -> std::path::PathBuf {
        self.dir.path().join("server.sqlite")
    }

    /// The file under its key, with no upgrade.
    pub(super) fn raw(&self) -> Connection {
        let conn = Connection::open(self.path()).expect("opens the file");
        conn.execute_batch("PRAGMA cipher_log_level = NONE")
            .expect("quiet");
        set_key(&conn, &self.key).expect("key");
        conn
    }

    /// The file as this build opens it.
    pub(super) fn open(&self) -> Result<Connection, CatalogError> {
        open(&self.path(), &self.key, "")
    }

    pub(super) fn upgraded(&self) -> Connection {
        self.open().expect("the upgrade")
    }
}

pub(super) fn version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("version")
}

pub(super) fn number(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect(sql)
}

/// Each row of `sql` as text, its columns joined with `|`, NULL as `NULL`.
pub(super) fn rows(conn: &Connection, sql: &str) -> Vec<String> {
    let mut statement = conn.prepare(sql).expect(sql);
    let columns = statement.column_count();
    statement
        .query_map([], |r| {
            let mut line = Vec::new();
            for column in 0..columns {
                line.push(match r.get_ref(column)? {
                    rusqlite::types::ValueRef::Null => "NULL".to_owned(),
                    rusqlite::types::ValueRef::Integer(n) => n.to_string(),
                    rusqlite::types::ValueRef::Real(n) => n.to_string(),
                    rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
                    rusqlite::types::ValueRef::Blob(b) => {
                        b.iter().map(|byte| format!("{byte:02x}")).collect()
                    }
                });
            }
            Ok(line.join("|"))
        })
        .expect(sql)
        .collect::<Result<_, _>>()
        .expect(sql)
}

/// Every table and index a catalog declares, with its text.
pub(super) fn declared(conn: &Connection) -> Vec<String> {
    rows(
        conn,
        "SELECT type, name, tbl_name, sql FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%' ORDER BY name",
    )
}

pub(super) fn tables(conn: &Connection) -> Vec<String> {
    rows(
        conn,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )
}

const AUDIT_READ: &str =
    "SELECT seq, at, actor, action, target, version, status, count FROM audit ORDER BY seq";

fn filled() -> Entry {
    Entry {
        actor: Actor::Unparsed,
        action: Action::ListenerFull,
        target: Some("listen".to_owned()),
        version: None,
        status: NO_ANSWER,
    }
}

const USER: &str =
    "INSERT INTO user_account (id, label, issuer, subject, created_at, deactivated_at)";

// What must work.

#[test]
fn a_0_4_2_registry_gains_the_user_and_membership_tables_empty() {
    let conn = Old::registry().upgraded();
    assert_eq!(
        version(&conn),
        SCHEMA_VERSION,
        "the upgraded registry's version"
    );
    assert_eq!(
        tables(&conn),
        [
            "api_key",
            "audit",
            "master_key_use",
            "membership",
            "session",
            "tenant",
            "tenant_setting",
            "user_account"
        ],
        "the registry's tables after the upgrade"
    );
    assert_eq!(
        number(&conn, "SELECT count(*) FROM user_account"),
        0,
        "no user is made by the upgrade"
    );
    assert_eq!(
        number(&conn, "SELECT count(*) FROM membership"),
        0,
        "no membership is made by the upgrade"
    );
}

#[test]
fn a_0_4_2_registry_keeps_its_tenants_keys_and_wrap_counts() {
    let old = Old::registry();
    let read = |conn: &Connection| {
        [
            // The columns of version 4: a later version adds its own.
            rows(
                conn,
                "SELECT id, name, retention, created_at, tenant_key_id, wrapped_tenant_key,
                     pending_wrapped_tenant_key, key_since, metadata FROM tenant ORDER BY id",
            ),
            rows(conn, "SELECT * FROM master_key_use ORDER BY master_key_id"),
            rows(conn, KEYS_V4),
        ]
    };
    let before = read(&old.raw());
    assert_eq!(before[2].len(), 2, "the fixture holds two keys");
    assert_eq!(read(&old.upgraded()), before, "every row as it was");
}

#[test]
fn audit_records_keep_every_column_and_only_a_zero_status_changes() {
    let conn = Old::registry().upgraded();
    assert_eq!(
        rows(&conn, AUDIT_READ),
        [
            "1|2026-11-01T10:00:00Z|cli|tenant_create|a1a1|NULL|201|1",
            "2|2026-11-01T10:01:00Z|0303030303030303|read|u-1|v-1|200|1",
            "3|2026-11-01T10:02:00Z||listener_full|listen|NULL|NULL|4",
            "4|2026-11-01T10:02:00Z||auth_failed|NULL|NULL|401|9",
            "5|2026-11-01T10:03:00Z||listener_full|operator.listen|NULL|NULL|1",
        ],
        "the log after the upgrade"
    );
}

#[test]
fn an_upgraded_registry_is_declared_as_a_new_one_is() {
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalRegistry::open(dir.path(), Key::generate().expect("key")).expect("a new one");
    let upgraded = Old::registry().upgraded();
    assert_eq!(
        declared(&upgraded),
        declared(&new.local_conn()),
        "tables and indexes, with their text"
    );
}

#[test]
fn an_upgraded_tenant_catalog_is_declared_as_a_new_one_is() {
    use crate::backend::Catalog;
    let dir = tempfile::tempdir().expect("temp dir");
    let new = LocalCatalog::open(
        &dir.path().join("catalog.sqlite"),
        Key::generate().expect("key"),
    )
    .expect("a new one");
    let upgraded = Old::tenant().upgraded();
    assert_eq!(
        declared(&upgraded),
        declared(&new.local_conn()),
        "tables and indexes, with their text"
    );
}

#[test]
fn a_tenant_catalog_gains_no_user_tables_and_its_log_changes_the_same_way() {
    let old = Old::tenant();
    let before = tables(&old.raw());
    let conn = old.upgraded();
    assert_eq!(
        version(&conn),
        SCHEMA_VERSION,
        "the tenant catalog's version"
    );
    assert_eq!(tables(&conn), before, "the same tables");
    assert_eq!(
        rows(&conn, "SELECT seq, status FROM audit ORDER BY seq"),
        ["1|201", "2|200", "3|NULL", "4|401", "5|NULL"],
        "the tenant log's statuses"
    );
}

#[test]
fn the_log_goes_on_from_its_last_sequence_number() {
    let conn = Old::registry().upgraded();
    audit::record(&conn, &filled(), datetime!(2026-11-11 12:00 UTC)).expect("record");
    assert_eq!(
        number(&conn, "SELECT max(seq) FROM audit"),
        6,
        "the next record after five"
    );
}

#[test]
fn an_upgraded_log_takes_a_record_with_no_status() {
    let conn = Old::registry().upgraded();
    audit::record(&conn, &filled(), datetime!(2026-11-11 12:00 UTC)).expect("record");
    assert_eq!(
        rows(&conn, "SELECT typeof(status) FROM audit WHERE seq = 6"),
        ["null"],
        "what the column holds for a record that answers no request"
    );
}

#[test]
fn an_upgraded_log_is_read_back_with_null_for_no_status() {
    let conn = Old::registry().upgraded();
    let mut out = Vec::new();
    audit::export(&conn, Some(2), &mut out).expect("export");
    let text = String::from_utf8(out).expect("UTF-8");
    let statuses: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("JSON")["status"].clone())
        .collect();
    assert_eq!(
        statuses,
        [
            serde_json::Value::Null,
            serde_json::json!(401),
            serde_json::Value::Null
        ],
        "records 3 to 5 as exported"
    );
}

#[test]
fn an_upgraded_registry_takes_users_and_memberships() {
    let conn = Old::registry().upgraded();
    conn.execute_batch(&format!(
        "{USER} VALUES (x'01', 'Local user', NULL, NULL, 1, NULL);
         {USER} VALUES (x'02', 'SSO user', 'https://idp.example.com', 'subject-1', 2, NULL);
         INSERT INTO membership (user, tenant, roles, granted_at) VALUES (x'01', NULL, 'operator', 3);
         INSERT INTO membership (user, tenant, roles, granted_at)
             VALUES (x'02', x'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1', 'auditor,reader', 4);"
    ))
    .expect("rows of each kind");
    assert_eq!(
        rows(
            &conn,
            "SELECT hex(user), roles FROM membership ORDER BY user"
        ),
        ["01|operator", "02|auditor,reader"],
        "the memberships kept"
    );
}

#[test]
fn registries_of_every_older_version_gain_the_tables() {
    for from in 1..=3 {
        // The stamp alone is older: the upgrade's own steps for 1 to 3
        // are tested where each was added.
        let old = Old::at(from, &[AUDIT_V4, AUDIT_ROWS]);
        old.raw()
            .execute_batch("CREATE TABLE tenant (id BLOB PRIMARY KEY NOT NULL)")
            .expect("a tenant table");
        let conn = old.upgraded();
        assert_eq!(version(&conn), SCHEMA_VERSION, "from {from}");
        assert!(
            tables(&conn).contains(&"user_account".to_owned()),
            "from {from}"
        );
        assert_eq!(
            number(&conn, "SELECT count(*) FROM audit WHERE status IS NULL"),
            2,
            "from {from}"
        );
    }
}

#[test]
fn a_re_encrypted_upgraded_registry_keeps_its_version_and_its_tables() {
    let old = Old::registry();
    drop(old.upgraded());
    let new_key = Key::generate().expect("key");
    super::reencrypt(&old.path(), &old.key, &new_key).expect("re-encrypts");
    let conn = open(&old.path(), &new_key, "").expect("opens under the new key");
    assert_eq!(
        version(&conn),
        SCHEMA_VERSION,
        "the version after re-encryption"
    );
    assert!(
        tables(&conn).contains(&"membership".to_owned()),
        "the tables after re-encryption"
    );
    assert_eq!(
        number(&conn, "SELECT count(*) FROM audit WHERE status IS NULL"),
        2,
        "the log after re-encryption"
    );
}

// What must be refused.

#[test]
fn a_version_4_log_refused_a_record_with_no_status() {
    let old = Old::registry();
    let refused = old.raw().execute(
        "INSERT INTO audit (at, actor, action, status, count) VALUES ('t', '', 'a', NULL, 1)",
        [],
    );
    assert!(refused.is_err(), "version 4 holds a number in every record");
}

#[test]
fn the_upgraded_catalog_carries_a_version_0_4_2_does_not_know() {
    // Release 0.4.2 opens versions 0 to 4 and refuses every other number
    // (its CatalogError::UnknownSchema), so the number is what keeps an
    // older binary from writing a 0 into a log that now holds NULL.
    let conn = Old::registry().upgraded();
    assert!(
        version(&conn) > 4,
        "a number release 0.4.2 refuses: {}",
        version(&conn)
    );
}

#[test]
fn a_catalog_from_a_newer_build_is_refused_and_left_as_it_was() {
    for newer in [SCHEMA_VERSION + 1, SCHEMA_VERSION + 2, 1000] {
        let old = Old::at(newer, &[REGISTRY_V4, AUDIT_V4, AUDIT_ROWS]);
        let before = declared(&old.raw());
        assert!(
            matches!(old.open(), Err(CatalogError::UnknownSchema(v)) if v == newer),
            "version {newer} is refused by its number"
        );
        let after = old.raw();
        assert_eq!(version(&after), newer, "the number is left");
        assert_eq!(declared(&after), before, "the tables are left");
    }
}

#[test]
fn a_negative_version_is_refused() {
    let old = Old::at(-1, &[REGISTRY_V4, AUDIT_V4]);
    assert!(
        matches!(old.open(), Err(CatalogError::UnknownSchema(-1))),
        "a number no build wrote"
    );
}

#[test]
fn an_upgrade_that_fails_at_the_user_tables_leaves_version_4_whole() {
    let old = Old::registry();
    old.raw()
        .execute_batch("CREATE TABLE membership (other INTEGER)")
        .expect("a table in the way");
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::Sqlite(_))),
        "the upgrade fails on the table in its way"
    );
    let after = old.raw();
    assert_eq!(version(&after), 4, "still version 4");
    assert_eq!(declared(&after), before, "nothing of the upgrade is kept");
    assert_eq!(
        number(&after, "SELECT count(*) FROM audit WHERE status = 0"),
        2,
        "the log still holds its zeros"
    );
}

#[test]
fn an_upgrade_that_fails_at_the_log_leaves_version_4_whole() {
    let old = Old::registry();
    old.raw()
        .execute_batch("CREATE TABLE audit_v4 (other INTEGER)")
        .expect("a table in the way");
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::Sqlite(_))),
        "the upgrade fails on the table in its way"
    );
    let after = old.raw();
    assert_eq!(version(&after), 4, "still version 4");
    assert_eq!(
        declared(&after),
        before,
        "the user tables made earlier in the same transaction are gone too"
    );
}

#[test]
fn a_wrong_key_upgrades_nothing() {
    let old = Old::registry();
    let other = Key::generate().expect("key");
    assert!(
        matches!(open(&old.path(), &other, ""), Err(CatalogError::WrongKey)),
        "the wrong key is named"
    );
    assert_eq!(version(&old.raw()), 4, "still version 4");
}

/// The upgraded registry with one active user of each kind.
fn with_users() -> (Old, Connection) {
    let old = Old::registry();
    let conn = old.upgraded();
    conn.execute_batch(&format!(
        "{USER} VALUES (x'01', 'Local user', NULL, NULL, 1, NULL);
         {USER} VALUES (x'02', 'SSO user', 'https://idp.example.com', 'subject-1', 2, NULL);"
    ))
    .expect("users");
    (old, conn)
}

fn refused(conn: &Connection, sql: &str) -> bool {
    conn.execute_batch(sql).is_err()
}

#[test]
fn two_active_users_cannot_share_an_identity() {
    let (_old, conn) = with_users();
    assert!(
        refused(
            &conn,
            &format!(
                "{USER} VALUES (x'03', 'Again', 'https://idp.example.com', 'subject-1', 3, NULL)"
            )
        ),
        "the same issuer and subject, both active"
    );
}

#[test]
fn an_identity_is_an_issuer_and_a_subject_or_neither() {
    let (_old, conn) = with_users();
    for (issuer, subject) in [
        ("'https://idp.example.com'", "NULL"),
        ("NULL", "'subject-2'"),
    ] {
        assert!(
            refused(
                &conn,
                &format!("{USER} VALUES (x'03', 'Half', {issuer}, {subject}, 3, NULL)")
            ),
            "issuer {issuer} with subject {subject}"
        );
    }
}

#[test]
fn a_user_needs_an_id_a_label_and_a_creation_time() {
    let (_old, conn) = with_users();
    for values in [
        "(NULL, 'No id', NULL, NULL, 3, NULL)",
        "(x'03', NULL, NULL, NULL, 3, NULL)",
        "(x'03', 'No time', NULL, NULL, NULL, NULL)",
        "(x'01', 'Taken id', NULL, NULL, 3, NULL)",
    ] {
        assert!(
            refused(&conn, &format!("{USER} VALUES {values}")),
            "{values}"
        );
    }
}

const MEMBER: &str = "INSERT INTO membership (user, tenant, roles, granted_at)";

#[test]
fn a_user_has_one_membership_in_a_tenant() {
    let (_old, conn) = with_users();
    conn.execute_batch(&format!("{MEMBER} VALUES (x'01', x'a1', 'reader', 1)"))
        .expect("the first");
    assert!(
        refused(
            &conn,
            &format!("{MEMBER} VALUES (x'01', x'a1', 'auditor', 2)")
        ),
        "a second membership of the same user in the same tenant"
    );
}

#[test]
fn a_user_has_one_operator_membership() {
    let (_old, conn) = with_users();
    conn.execute_batch(&format!("{MEMBER} VALUES (x'01', NULL, 'operator', 1)"))
        .expect("the first");
    assert!(
        refused(
            &conn,
            &format!("{MEMBER} VALUES (x'01', NULL, 'operator', 2)")
        ),
        "a second operator membership of the same user"
    );
}

#[test]
fn a_membership_needs_a_user_roles_and_a_time() {
    let (_old, conn) = with_users();
    for values in [
        "(NULL, x'a1', 'reader', 1)",
        "(x'01', x'a1', NULL, 1)",
        "(x'01', x'a1', 'reader', NULL)",
    ] {
        assert!(
            refused(&conn, &format!("{MEMBER} VALUES {values}")),
            "{values}"
        );
    }
}

// Limits and odd inputs.

#[test]
fn an_empty_log_upgrades_and_starts_at_one() {
    let conn = Old::of(&[REGISTRY_V4, AUDIT_V4]).upgraded();
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM sqlite_sequence WHERE seq IS NULL OR name <> 'audit'"
        ),
        0,
        "the upgrade leaves no counter without a number, and none for another table"
    );
    audit::record(&conn, &filled(), datetime!(2026-11-11 12:00 UTC)).expect("record");
    assert_eq!(
        rows(&conn, "SELECT seq FROM audit"),
        ["1"],
        "the first record of a log that was empty"
    );
}

#[test]
fn a_sequence_number_already_given_is_never_given_again() {
    // SQLite keeps the highest number given in sqlite_sequence, which can
    // be past the last row's.
    let old = Old::registry();
    old.raw()
        .execute_batch("UPDATE sqlite_sequence SET seq = 50 WHERE name = 'audit'")
        .expect("a higher number given");
    let conn = old.upgraded();
    audit::record(&conn, &filled(), datetime!(2026-11-11 12:00 UTC)).expect("record");
    assert_eq!(
        number(&conn, "SELECT max(seq) FROM audit"),
        51,
        "the number after the highest given"
    );
    assert_eq!(
        rows(&conn, "SELECT name FROM sqlite_sequence ORDER BY name"),
        ["audit"],
        "no counter is left for the old table"
    );
}

#[test]
fn a_number_given_by_a_log_with_no_record_left_is_never_given_again() {
    let old = Old::of(&[REGISTRY_V4, AUDIT_V4]);
    old.raw()
        .execute_batch("INSERT INTO sqlite_sequence (name, seq) VALUES ('audit', 7)")
        .expect("numbers given, and no record left");
    let conn = old.upgraded();
    audit::record(&conn, &filled(), datetime!(2026-11-11 12:00 UTC)).expect("record");
    assert_eq!(
        rows(&conn, "SELECT seq FROM audit"),
        ["8"],
        "the number after the highest given"
    );
}

#[test]
fn a_catalog_with_no_log_and_no_tenant_table_changes_only_its_number() {
    let old = Old::of(&["CREATE TABLE note (text TEXT)"]);
    let before = declared(&old.raw());
    let conn = old.upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "the number");
    assert_eq!(declared(&conn), before, "nothing else");
}

#[test]
fn opening_an_upgraded_catalog_again_changes_nothing() {
    let old = Old::registry();
    let first = old.upgraded();
    let (tables, log) = (declared(&first), rows(&first, AUDIT_READ));
    drop(first);
    let second = old.upgraded();
    assert_eq!(declared(&second), tables, "the same tables");
    assert_eq!(rows(&second, AUDIT_READ), log, "the same log");
}

#[test]
fn a_count_in_the_minute_of_an_upgraded_record_adds_to_it() {
    let conn = Old::registry().upgraded();
    audit::count(&conn, &filled(), datetime!(2026-11-01 10:02:30 UTC)).expect("count");
    assert_eq!(
        rows(
            &conn,
            "SELECT seq, count FROM audit WHERE action = 'listener_full' ORDER BY seq"
        ),
        ["3|5", "5|1"],
        "the record begun in that minute before the upgrade holds the new count, and no record is added"
    );
}

#[test]
fn a_long_log_keeps_every_record() {
    let old = Old::of(&[REGISTRY_V4, AUDIT_V4]);
    {
        let mut conn = old.raw();
        let tx = conn.transaction().expect("tx");
        for n in 0..5000 {
            tx.execute(
                "INSERT INTO audit (at, actor, action, status, count) VALUES ('t', '', 'a', ?1, 1)",
                [n % 3 * 200],
            )
            .expect("insert");
        }
        tx.commit().expect("commit");
    }
    let conn = old.upgraded();
    assert_eq!(
        rows(
            &conn,
            "SELECT count(*), sum(seq), count(status), sum(status) FROM audit"
        ),
        ["5000|12502500|3333|999800"],
        "rows, their numbers, and the statuses that are not zero"
    );
}

#[test]
fn an_identity_is_free_again_once_its_user_is_deactivated() {
    let (_old, conn) = with_users();
    conn.execute_batch(&format!(
        "UPDATE user_account SET deactivated_at = 5 WHERE id = x'02';
         {USER} VALUES (x'03', 'Again', 'https://idp.example.com', 'subject-1', 6, NULL);"
    ))
    .expect("the identity under a new user");
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM user_account WHERE subject = 'subject-1'"
        ),
        2,
        "the deactivated user and the new one"
    );
}

#[test]
fn local_users_share_no_identity_to_collide_on() {
    let (_old, conn) = with_users();
    conn.execute_batch(&format!(
        "{USER} VALUES (x'03', 'Local user', NULL, NULL, 3, NULL)"
    ))
    .expect("a second local user, even with the same label");
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM user_account WHERE issuer IS NULL"
        ),
        2,
        "both local users"
    );
}

#[test]
fn one_subject_under_two_issuers_is_two_identities() {
    let (_old, conn) = with_users();
    conn.execute_batch(&format!(
        "{USER} VALUES (x'03', 'Other', 'https://other.example.com', 'subject-1', 3, NULL)"
    ))
    .expect("the same subject from another issuer");
    assert_eq!(
        number(
            &conn,
            "SELECT count(*) FROM user_account WHERE subject = 'subject-1'"
        ),
        2,
        "one user for each issuer"
    );
}

#[test]
fn a_user_may_be_an_operator_and_a_member_of_several_tenants() {
    let (_old, conn) = with_users();
    conn.execute_batch(&format!(
        "{MEMBER} VALUES (x'01', NULL, 'operator', 1);
         {MEMBER} VALUES (x'01', x'a1', 'reader', 2);
         {MEMBER} VALUES (x'01', x'a2', 'auditor', 3);
         {MEMBER} VALUES (x'02', x'a1', 'reader', 4);"
    ))
    .expect("memberships that do not collide");
    assert_eq!(
        number(&conn, "SELECT count(*) FROM membership"),
        4,
        "every membership"
    );
}

#[test]
fn statuses_other_than_zero_are_left_whatever_they_are() {
    let old = Old::of(&[REGISTRY_V4, AUDIT_V4]);
    old.raw()
        .execute_batch(
            "INSERT INTO audit (at, actor, action, status, count) VALUES
                ('t', '', 'a', 1, 1), ('t', '', 'a', 204, 1), ('t', '', 'a', 599, 1),
                ('t', '', 'a', 65535, 1), ('t', '', 'a', 0, 1)",
        )
        .expect("rows");
    assert_eq!(
        rows(&old.upgraded(), "SELECT status FROM audit ORDER BY seq"),
        ["1", "204", "599", "65535", "NULL"],
        "only the zero changes"
    );
}

#[test]
fn committed_records_still_in_the_wal_are_upgraded_too() {
    let old = Old::registry();
    {
        // A second connection holds the WAL open, so the first one's close
        // does not fold it into the file.
        let holder = old.raw();
        number(&holder, "SELECT count(*) FROM audit");
        let writer = old.raw();
        writer
            .execute_batch(
                "INSERT INTO audit (at, actor, action, status, count) VALUES ('late', '', 'a', 0, 1)",
            )
            .expect("a record in the WAL");
        drop(writer);
        drop(holder);
    }
    let conn = old.upgraded();
    assert_eq!(
        rows(&conn, "SELECT seq, at, status FROM audit WHERE seq = 6"),
        ["6|late|NULL"],
        "the last record written before the upgrade"
    );
}
