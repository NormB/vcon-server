//! The upgrade to schema version 10, tested from catalogs made as schema
//! version 9 made them: server.sqlite as `v9_tests` builds version 8 with
//! the access keys' two columns added, and a tenant catalog as release
//! 0.4.2's tables with version 5's audit log (no tenant catalog table
//! changed from 5 to 9). Releases 0.4.4 to 0.4.16 wrote version 9.
//!
//! Version 10 adds the column `counted` to every audit log: 1 for a record
//! a count made, the only kind a later count adds to, and 0 for a record of
//! one request. Before it, a count added to any record with its time and
//! fields, a record of one request at the start of a minute among them. A
//! log of version 9 does not say which records a count made, so the upgrade
//! marks only those a count must have made (audit::COUNTED_BEFORE_10).

use rusqlite::Connection;
use time::OffsetDateTime;
use time::macros::datetime;

use super::v5_tests::{
    AUDIT_READ, Old, REGISTRY_ROWS, REGISTRY_V4, TENANT_V4, declared, number, rows, version,
};
use super::v6_tests::{AUDIT_V5, ROWS_V5, USERS_V5, columns};
use super::v8_tests::ADDED_V7;
use super::{CatalogError, KEY_LIFE, SCHEMA_VERSION, SESSIONS, open};
use crate::audit::{self, Action, Actor, Entry};
use crate::envelope::Key;
use crate::ids::KeyId;

/// The version this file's catalogs are made at: the one before this
/// build's, and the one every release from 0.4.4 to 0.4.16 wrote.
const BEFORE: i64 = 9;

/// The key whose id the records below name.
const LOADER: KeyId = KeyId([0x03; 8]);

/// Records a version-9 log could hold, after the two of `ROWS_V5` (seq 1, a
/// plain `user_add` at 10:00:00 exactly, and seq 2, a `listener_full`
/// counted twice). Seq 3 to 10, with what the upgrade must make of each:
/// 3 plain (a fraction of a second), 4 counted (`health` is only counted),
/// 5 counted (a count above 1), 6 plain (an `mcp` count of 1 cannot be told
/// from a record, the stated limit), 7, 8 and 9 counted (only counted
/// actions), 10 plain (a nanosecond past the minute).
const ROWS_V9: &str = "
INSERT INTO audit (at, actor, action, target, version, status, count) VALUES
    ('2026-11-20T09:00:00.25Z', 'cli', 'tenant_create', 'a1a1', NULL, 201, 1),
    ('2026-11-20T09:01:00Z', '0303030303030303', 'health', NULL, NULL, 200, 1),
    ('2026-11-20T09:01:00Z', '0303030303030303', 'read', NULL, NULL, 429, 7),
    ('2026-11-20T09:02:00Z', '0303030303030303', 'mcp', NULL, NULL, 200, 1),
    ('2026-11-20T09:03:00Z', '', 'csp_report', NULL, NULL, 204, 1),
    ('2026-11-20T09:03:00Z', '', 'auth_failed', NULL, NULL, 401, 1),
    ('2026-11-20T09:04:00Z', '0303030303030303', 'wrong_listener', NULL, NULL, 403, 1),
    ('2026-11-20T09:05:00.000000001Z', '0303030303030303', 'read', 'u-1', 'v-1', 200, 1);
";

/// What the upgrade makes of seq 1 to 10, in order.
const MARKED: [&str; 10] = [
    "1|0", "2|1", "3|0", "4|1", "5|1", "6|0", "7|1", "8|1", "9|1", "10|0",
];

/// Each record's seq and mark.
const MARKS: &str = "SELECT seq, counted FROM audit ORDER BY seq";

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
            KEY_LIFE,
            ROWS_V9,
        ],
    )
}

/// A version-9 tenant catalog with the records `rows` inserts.
fn tenant(rows: &str) -> Old {
    Old::at(BEFORE, &[TENANT_V4, AUDIT_V5, rows])
}

/// A version-9 tenant catalog holding one record, upgraded: whether the
/// upgrade marked it counted.
fn marked(at: &str, action: &str, count: i64) -> bool {
    let insert = format!(
        "INSERT INTO audit (at, actor, action, target, version, status, count)
         VALUES ('{at}', 'cli', '{action}', NULL, NULL, 200, {count});"
    );
    let conn = tenant(&insert).upgraded();
    number(&conn, "SELECT counted FROM audit") == 1
}

/// The audit table and its index as a new catalog declares them.
fn new_audit_text() -> Vec<String> {
    let dir = tempfile::tempdir().expect("temp dir");
    let conn = open(
        &dir.path().join("catalog.sqlite"),
        &Key::generate().expect("key"),
        audit::SCHEMA,
    )
    .expect("a new catalog");
    audit_text(&conn)
}

/// `before`, a catalog's tables and indexes as `declared` reads them at a
/// version from 5 to 9, as the upgrade to this build leaves them: with the
/// audit log's mark added, its only change to a catalog of that age that
/// the version's own step does not touch.
pub(super) fn with_mark(before: Vec<String>) -> Vec<String> {
    before
        .into_iter()
        .map(|line| {
            if line.starts_with("table|audit|audit|") {
                line.replace(
                    "count INTEGER NOT NULL\n)",
                    "count INTEGER NOT NULL\n, counted INTEGER NOT NULL DEFAULT 0 CHECK (counted IN (0, 1)))",
                )
            } else {
                line
            }
        })
        .collect()
}

fn audit_text(conn: &Connection) -> Vec<String> {
    rows(
        conn,
        "SELECT type, name, sql FROM sqlite_master WHERE tbl_name = 'audit' ORDER BY name",
    )
}

fn count_many(conn: &Connection, entry: &Entry, now: OffsetDateTime, times: u64) {
    audit::count_many(conn, entry, now, times).expect("the count is written");
}

fn refused_read() -> Entry {
    Entry {
        actor: Actor::Key(LOADER),
        action: Action::Read,
        target: None,
        version: None,
        status: 429,
    }
}

// What must work.

#[test]
fn a_version_9_registry_gains_the_column_at_the_end() {
    let conn = registry().upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "the upgraded version");
    assert_eq!(
        columns(&conn, "audit"),
        [
            "seq", "at", "actor", "action", "target", "version", "status", "count", "counted"
        ],
        "the audit log's columns"
    );
}

#[test]
fn a_version_9_tenant_catalog_gains_the_column_at_the_end() {
    let conn = tenant(ROWS_V9).upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "the upgraded version");
    assert_eq!(
        columns(&conn, "audit").last().map(String::as_str),
        Some("counted"),
        "the last column"
    );
}

#[test]
fn an_upgraded_registry_declares_its_audit_log_as_a_new_one_does() {
    assert_eq!(
        audit_text(&registry().upgraded()),
        new_audit_text(),
        "the table and its index, with their text"
    );
}

#[test]
fn an_upgraded_tenant_catalog_declares_its_audit_log_as_a_new_one_does() {
    assert_eq!(
        audit_text(&tenant(ROWS_V9).upgraded()),
        new_audit_text(),
        "the table and its index, with their text"
    );
}

#[test]
fn a_version_9_registry_keeps_every_record_as_it_was() {
    let old = registry();
    let before = rows(&old.raw(), AUDIT_READ);
    assert_eq!(before.len(), 10, "records to keep");
    assert_eq!(
        rows(&old.upgraded(), AUDIT_READ),
        before,
        "every record, every column"
    );
}

#[test]
fn a_version_9_tenant_catalog_keeps_every_record_as_it_was() {
    let old = tenant(ROWS_V9);
    let before = rows(&old.raw(), AUDIT_READ);
    assert_eq!(before.len(), 8, "records to keep");
    assert_eq!(
        rows(&old.upgraded(), AUDIT_READ),
        before,
        "every record, every column"
    );
}

#[test]
fn the_records_a_count_must_have_made_are_marked_counted() {
    assert_eq!(
        rows(&registry().upgraded(), MARKS),
        MARKED,
        "seq 2, 4, 5, 7, 8 and 9 counted, the rest plain"
    );
}

#[test]
fn a_count_after_the_upgrade_adds_to_a_record_marked_counted() {
    let conn = registry().upgraded();
    count_many(
        &conn,
        &refused_read(),
        datetime!(2026-11-20 09:01:30 UTC),
        3,
    );
    assert_eq!(
        rows(
            &conn,
            "SELECT seq, count FROM audit WHERE seq >= 5 AND action = 'read'"
        ),
        ["5|10", "10|1"],
        "seq 5 goes from 7 to 10, and no record is added"
    );
    assert_eq!(
        number(&conn, "SELECT max(seq) FROM audit"),
        10,
        "no new record"
    );
}

#[test]
fn a_record_counted_once_under_an_only_counted_action_is_marked() {
    for action in [
        "health",
        "auth_failed",
        "wrong_listener",
        "listener_full",
        "csp_report",
    ] {
        assert!(
            marked("2026-11-20T09:01:00Z", action, 1),
            "{action}: no release recorded it, so a count made it"
        );
    }
}

#[test]
fn a_record_at_a_minute_start_with_a_count_above_1_is_marked_whatever_its_action() {
    for action in ["read", "mcp", "create", "user_add"] {
        assert!(
            marked("2026-11-20T09:01:00Z", action, 2),
            "{action}: only a count writes a count above 1"
        );
    }
}

#[test]
fn a_0_4_2_registry_reaches_version_10_with_its_counted_records_marked() {
    // Release 0.4.2's log (v5_tests' AUDIT_ROWS): a plain tenant_create and
    // read, a listener_full counted 4 times, an auth_failed counted 9
    // times, a listener_full counted once. Version 5 rebuilds the table
    // from this build's text, so no column is added after it.
    let conn = Old::registry().upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "one step to this build");
    assert_eq!(
        rows(&conn, MARKS),
        ["1|0", "2|0", "3|1", "4|1", "5|1"],
        "the three counted records"
    );
    assert_eq!(
        audit_text(&conn),
        new_audit_text(),
        "declared as a new one is"
    );
}

#[test]
fn the_seq_goes_on_from_the_last_record_after_the_upgrade() {
    let conn = registry().upgraded();
    audit::record(&conn, &refused_read(), datetime!(2026-11-21 08:00:00.5 UTC)).expect("a record");
    assert_eq!(
        rows(&conn, "SELECT seq, counted FROM audit WHERE seq > 10"),
        ["11|0"],
        "the next seq, a plain record"
    );
}

#[test]
fn opening_an_upgraded_registry_again_changes_nothing() {
    let old = registry();
    let first = old.upgraded();
    let (text, marks) = (declared(&first), rows(&first, MARKS));
    drop(first);
    let second = old.upgraded();
    assert_eq!(declared(&second), text, "the same tables");
    assert_eq!(rows(&second, MARKS), marks, "the same marks");
}

#[test]
fn a_new_catalog_is_made_at_version_10_with_the_column() {
    let dir = tempfile::tempdir().expect("temp dir");
    let conn = open(
        &dir.path().join("catalog.sqlite"),
        &Key::generate().expect("key"),
        audit::SCHEMA,
    )
    .expect("a new catalog");
    assert_eq!(version(&conn), 10, "this build's version");
    assert!(
        columns(&conn, "audit").contains(&"counted".to_owned()),
        "the column"
    );
}

// What must be refused, or left alone.

#[test]
fn a_plain_record_at_a_minute_start_stays_plain() {
    let conn = registry().upgraded();
    assert_eq!(
        rows(
            &conn,
            "SELECT action, count, counted FROM audit WHERE seq = 1"
        ),
        ["user_add|1|0"],
        "a record of one request, though its time is 10:00:00 exactly"
    );
}

#[test]
fn a_count_after_the_upgrade_adds_nothing_to_a_plain_record_at_a_minute_start() {
    let conn = registry().upgraded();
    let added = Entry {
        actor: Actor::Cli,
        action: Action::UserAdd,
        target: Some("c3c3".to_owned()),
        version: None,
        status: 201,
    };
    count_many(&conn, &added, datetime!(2026-11-11 10:00:10 UTC), 4);
    assert_eq!(
        rows(
            &conn,
            "SELECT seq, count, counted FROM audit WHERE action = 'user_add'"
        ),
        ["1|1|0", "11|4|1"],
        "the plain record keeps its 1; the count is a record of its own"
    );
}

#[test]
fn a_record_with_a_fraction_of_a_second_stays_plain() {
    assert!(
        !marked("2026-11-20T09:01:00.5Z", "health", 1),
        "a count writes no fraction"
    );
}

#[test]
fn a_count_of_1_under_an_action_a_release_also_recorded_stays_plain() {
    for action in ["mcp", "read", "create", "user_add"] {
        assert!(
            !marked("2026-11-20T09:01:00Z", action, 1),
            "{action}: a count of 1 cannot be told from a record"
        );
    }
}

#[test]
fn an_upgrade_that_fails_at_the_column_leaves_version_9_whole() {
    let old = registry();
    old.raw()
        .execute_batch("ALTER TABLE audit ADD COLUMN counted INTEGER;")
        .expect("a column in the way");
    let before = declared(&old.raw());
    assert!(old.open().is_err(), "the column cannot be added twice");
    assert_eq!(version(&old.raw()), BEFORE, "still version 9");
    assert_eq!(declared(&old.raw()), before, "nothing changed");
    old.raw()
        .execute_batch("ALTER TABLE audit DROP COLUMN counted;")
        .expect("the column taken away");
    assert_eq!(
        version(&old.upgraded()),
        SCHEMA_VERSION,
        "then the upgrade runs"
    );
}

#[test]
fn an_upgrade_that_fails_at_the_marking_keeps_no_column() {
    let old = registry();
    old.raw()
        .execute_batch(
            "CREATE TRIGGER in_the_way BEFORE UPDATE ON audit
             BEGIN SELECT RAISE(ABORT, 'in the way'); END;",
        )
        .expect("a trigger that refuses every change of a record");
    assert!(old.open().is_err(), "the marking is refused");
    assert_eq!(version(&old.raw()), BEFORE, "still version 9");
    assert!(
        !columns(&old.raw(), "audit").contains(&"counted".to_owned()),
        "the column added before the marking is gone with it"
    );
}

#[test]
fn a_wrong_key_upgrades_no_version_9_catalog() {
    let old = registry();
    let other = Key::generate().expect("key");
    assert!(
        matches!(open(&old.path(), &other, ""), Err(CatalogError::WrongKey)),
        "the wrong key is named"
    );
    assert_eq!(version(&old.raw()), BEFORE, "still version 9");
    assert!(
        !columns(&old.raw(), "audit").contains(&"counted".to_owned()),
        "no column was added"
    );
}

#[test]
fn the_version_after_this_builds_is_refused_and_left_as_it_was() {
    let next = SCHEMA_VERSION + 1;
    let old = Old::at(next, &[TENANT_V4, AUDIT_V5]);
    let before = declared(&old.raw());
    assert!(
        matches!(old.open(), Err(CatalogError::UnknownSchema(n)) if n == next),
        "a version this build does not know"
    );
    assert_eq!(version(&old.raw()), next, "the number is left");
    assert_eq!(declared(&old.raw()), before, "the tables are left");
}

#[test]
fn this_build_makes_a_version_a_version_9_build_refuses() {
    // A build at version 9 refuses every number past 9, so it cannot
    // write a record without the column's place.
    assert_eq!(SCHEMA_VERSION, 10, "the number this build writes");
    assert!(
        version(&registry().upgraded()) > BEFORE,
        "an upgraded catalog has it"
    );
}

#[test]
fn the_column_refuses_a_mark_other_than_0_or_1() {
    let conn = registry().upgraded();
    for mark in ["2", "-1", "'yes'"] {
        let insert = format!(
            "INSERT INTO audit (at, actor, action, status, count, counted)
             VALUES ('2026-11-21T08:00:00Z', 'cli', 'health', 200, 1, {mark})"
        );
        assert!(conn.execute_batch(&insert).is_err(), "{mark} is not a mark");
    }
}

#[test]
fn the_column_refuses_no_mark() {
    let conn = registry().upgraded();
    assert!(
        conn.execute_batch(
            "INSERT INTO audit (at, actor, action, status, count, counted)
             VALUES ('2026-11-21T08:00:00Z', 'cli', 'health', 200, 1, NULL)"
        )
        .is_err(),
        "NULL is not a mark"
    );
}

// Limits and odd inputs.

#[test]
fn an_empty_log_upgrades_with_nothing_marked() {
    let conn = tenant("").upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "upgraded");
    assert_eq!(
        number(&conn, "SELECT count(*) FROM audit"),
        0,
        "no record made"
    );
}

#[test]
fn a_catalog_with_no_audit_log_changes_only_its_number() {
    let old = Old::at(BEFORE, &[TENANT_V4]);
    let before = declared(&old.raw());
    let conn = old.upgraded();
    assert_eq!(version(&conn), SCHEMA_VERSION, "the number");
    assert_eq!(declared(&conn), before, "no table made or changed");
}

#[test]
fn a_time_with_an_offset_or_a_space_is_not_a_minute_start() {
    for at in [
        "2026-11-20T09:01:00+00:00",
        "2026-11-20 09:01:00Z",
        "2026-11-20T09:01:00z",
        "2026-11-20T09:01:00",
        "2026-11-20T09:01Z",
    ] {
        assert!(
            !marked(at, "health", 2),
            "{at}: not as the server writes a minute"
        );
    }
}

#[test]
fn the_first_and_last_seconds_of_a_minute_are_told_apart() {
    assert!(
        marked("2026-11-20T09:01:00Z", "read", 2),
        "second 0 is a minute start"
    );
    assert!(
        !marked("2026-11-20T09:01:01Z", "read", 2),
        "second 1 is not"
    );
    assert!(
        !marked("2026-11-20T09:00:59.999999999Z", "read", 2),
        "the last nanosecond before it is not"
    );
}

#[test]
fn the_last_minute_of_a_day_is_a_minute_start() {
    assert!(
        marked("2026-12-31T23:59:00Z", "auth_failed", 1),
        "23:59:00 on the last day of a year"
    );
}

#[test]
fn every_minute_the_server_writes_is_a_minute_start_to_the_upgrade() {
    // Times from the clock as a count writes them: the start of their
    // minute, in RFC 3339, from a year's start to its end.
    for seconds in [0_i64, 59, 3_600, 86_399, 31_535_999] {
        let now = datetime!(2026-01-01 00:00:00 UTC) + time::Duration::seconds(seconds);
        let at = audit::rfc3339(audit::minute_of(now));
        assert!(marked(&at, "read", 3), "{at}");
    }
}

#[test]
fn a_counted_record_with_no_status_is_marked() {
    let conn = tenant(
        "INSERT INTO audit (at, actor, action, target, version, status, count)
         VALUES ('2026-11-20T09:01:00Z', '', 'listener_full', 'listen', NULL, NULL, 1);",
    )
    .upgraded();
    assert_eq!(
        rows(&conn, "SELECT status, counted FROM audit"),
        ["NULL|1"],
        "a listener's state, counted"
    );
}

#[test]
fn a_count_of_1_left_plain_takes_a_second_record_in_its_minute_and_the_total_holds() {
    // The stated limit: an mcp count of 1 is left plain, so a count in
    // the same minute after the upgrade makes a second record.
    let conn = registry().upgraded();
    let mcp = Entry {
        actor: Actor::Key(LOADER),
        action: Action::Mcp,
        target: None,
        version: None,
        status: 200,
    };
    count_many(&conn, &mcp, datetime!(2026-11-20 09:02:40 UTC), 2);
    assert_eq!(
        rows(
            &conn,
            "SELECT seq, count, counted FROM audit WHERE action = 'mcp'"
        ),
        ["6|1|0", "11|2|1"],
        "two records of the minute"
    );
    assert_eq!(
        number(&conn, "SELECT sum(count) FROM audit WHERE action = 'mcp'"),
        3,
        "the minute's total"
    );
}

#[test]
fn records_still_in_the_wal_are_upgraded_too() {
    let old = tenant("");
    let writer = old.raw();
    writer
        .execute_batch(
            "INSERT INTO audit (at, actor, action, target, version, status, count)
             VALUES ('2026-11-20T09:01:00Z', '', 'auth_failed', NULL, NULL, 401, 5);",
        )
        .expect("a record not yet checkpointed");
    let conn = old.upgraded();
    assert_eq!(
        rows(&conn, MARKS),
        ["1|1"],
        "the record in the WAL is marked"
    );
    drop(writer);
}
