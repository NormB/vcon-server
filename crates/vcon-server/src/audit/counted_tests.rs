//! A count adds only to a record a count made: never to a plain record,
//! whatever its time. A plain record is written at the request's time and
//! a counted one at the start of its minute, so a request whose time falls
//! on the first nanosecond of a minute (an injected clock gives such
//! times) wrote a plain record with the very time, actor, action, target,
//! version and status of a count; before the `counted` column, the count's
//! UPDATE found that record and changed its count after it was written.

use rusqlite::Connection;
use serde_json::Value;
use time::OffsetDateTime;
use time::macros::datetime;

use super::testing::exported;
use super::{Action, Actor, AuditQuery, Entry, NO_ANSWER, SCHEMA, count, count_many, page, record};
use crate::catalog;
use crate::envelope::Key;
use crate::ids::VersionId;

struct Log {
    _dir: tempfile::TempDir,
    conn: Connection,
}

fn log() -> Log {
    let dir = tempfile::tempdir().expect("temp dir");
    let key = Key::generate().expect("key");
    let conn = catalog::open(&dir.path().join("catalog.sqlite"), &key, SCHEMA).expect("opens");
    Log { _dir: dir, conn }
}

/// The start of a minute, nanosecond 0: the time both kinds of record
/// share.
const MINUTE: OffsetDateTime = datetime!(2026-10-23 12:34:00 UTC);

/// A refused read with no target and no version, as the MCP gate both
/// counts (rate-limited) and a route records.
fn refused() -> Entry {
    Entry {
        actor: Actor::Cli,
        action: Action::Read,
        target: None,
        version: None,
        status: 429,
    }
}

/// Runs `f` in a transaction of the log and commits it.
fn write(log: &mut Log, f: impl FnOnce(&Connection)) {
    let tx = log.conn.transaction().expect("tx");
    f(&tx);
    tx.commit().expect("commit");
}

/// Each record's seq and count, oldest first, as export writes them.
fn counts(log: &Log) -> Vec<(i64, i64)> {
    exported(&log.conn, None)
        .iter()
        .map(|r| {
            (
                r["seq"].as_i64().expect("a seq"),
                r["count"].as_i64().expect("a count"),
            )
        })
        .collect()
}

// What must work.

#[test]
fn a_count_after_a_plain_record_at_a_minute_start_is_a_record_of_its_own() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
        count_many(tx, &refused(), MINUTE, 5).expect("count");
    });
    assert_eq!(
        counts(&log),
        [(1, 1), (2, 5)],
        "the plain record keeps its count of 1 and the count is a second record"
    );
}

/// Each record's seq and mark, oldest first.
fn marks(log: &Log) -> Vec<(i64, i64)> {
    let mut rows = log
        .conn
        .prepare("SELECT seq, counted FROM audit ORDER BY seq")
        .expect("prepared");
    rows.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("read")
        .collect::<Result<_, _>>()
        .expect("rows")
}

/// A read with a target and a version, as a route records it.
fn read() -> Entry {
    Entry {
        target: Some("01a07da8-c2bb-83e5-b9a2-279e0d16bc46".into()),
        version: Some(VersionId([0x22; 16])),
        status: 200,
        ..refused()
    }
}

/// The time `nanos` nanoseconds after [`MINUTE`].
fn after(nanos: i64) -> OffsetDateTime {
    MINUTE + time::Duration::nanoseconds(nanos)
}

#[test]
fn a_single_count_after_a_plain_record_at_a_minute_start_is_a_record_of_its_own() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
        count(tx, &refused(), MINUTE).expect("count");
    });
    assert_eq!(counts(&log), [(1, 1), (2, 1)], "two records of 1 each");
}

#[test]
fn counts_add_to_the_counted_record_past_a_plain_record_between_them() {
    let mut log = log();
    write(&mut log, |tx| {
        count_many(tx, &refused(), after(5_000_000_000), 2).expect("count");
        record(tx, &refused(), MINUTE).expect("record");
        count_many(tx, &refused(), after(30_000_000_000), 3).expect("count");
    });
    assert_eq!(
        counts(&log),
        [(1, 5), (2, 1)],
        "2 and 3 on the counted record, 1 on the plain one"
    );
}

#[test]
fn a_count_marks_its_record_counted_and_a_plain_record_stays_unmarked() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
        count(tx, &refused(), MINUTE).expect("count");
    });
    assert_eq!(marks(&log), [(1, 0), (2, 1)], "plain 0, counted 1");
}

#[test]
fn two_plain_records_at_a_minute_start_both_keep_their_count() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
        record(tx, &refused(), MINUTE).expect("record");
        count_many(tx, &refused(), MINUTE, 4).expect("count");
        count_many(tx, &refused(), MINUTE, 6).expect("count");
    });
    assert_eq!(
        counts(&log),
        [(1, 1), (2, 1), (3, 10)],
        "each plain record keeps 1, the counts share one record"
    );
}

#[test]
fn a_plain_record_written_in_another_offset_at_a_utc_minute_start_keeps_its_count() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), datetime!(2026-10-23 14:34:00 +02:00)).expect("record");
        count_many(tx, &refused(), MINUTE, 2).expect("count");
    });
    assert_eq!(counts(&log), [(1, 1), (2, 2)], "the same UTC time, apart");
}

#[test]
fn an_export_shows_no_mark() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
        count(tx, &refused(), MINUTE).expect("count");
    });
    for record in exported(&log.conn, None) {
        let mut members: Vec<&String> = record.as_object().expect("an object").keys().collect();
        members.sort();
        assert_eq!(
            members,
            [
                "action", "actor", "at", "count", "seq", "status", "target", "version"
            ],
            "the members of a record, as before the mark"
        );
    }
}

#[test]
fn a_page_shows_no_mark() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
        count(tx, &refused(), MINUTE).expect("count");
    });
    let mut out = Vec::new();
    page(
        &log.conn,
        &AuditQuery {
            after: 0,
            limit: 10,
        },
        &mut out,
    )
    .expect("page");
    let text = String::from_utf8(out).expect("UTF-8");
    assert_eq!(text.lines().count(), 2, "two records: {text}");
    assert!(!text.contains("counted"), "no mark: {text}");
}

// What must not happen.

#[test]
fn a_count_adds_nothing_to_a_plain_record_with_a_target_and_a_version() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &read(), MINUTE).expect("record");
        count_many(tx, &read(), MINUTE, 9).expect("count");
    });
    assert_eq!(counts(&log), [(1, 1), (2, 9)], "the read's record keeps 1");
}

#[test]
fn a_plain_record_after_a_count_takes_none_of_the_later_counts() {
    let mut log = log();
    write(&mut log, |tx| {
        count(tx, &refused(), MINUTE).expect("count");
        record(tx, &refused(), MINUTE).expect("record");
        count_many(tx, &refused(), after(1), 7).expect("count");
    });
    assert_eq!(
        counts(&log),
        [(1, 8), (2, 1)],
        "the later count goes to seq 1"
    );
}

#[test]
fn a_plain_record_that_answers_no_request_takes_no_count() {
    let filled = Entry {
        actor: Actor::Unparsed,
        action: Action::ListenerFull,
        target: Some("listen".to_owned()),
        version: None,
        status: NO_ANSWER,
    };
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &filled, MINUTE).expect("record");
        count_many(tx, &filled, MINUTE, 3).expect("count");
    });
    assert_eq!(counts(&log), [(1, 1), (2, 3)], "no status on either, apart");
}

#[test]
fn a_mark_other_than_0_or_1_is_refused() {
    let log = log();
    for mark in ["2", "-1", "'yes'"] {
        let insert = format!(
            "INSERT INTO audit (at, actor, action, status, count, counted)
             VALUES ('2026-10-23T12:34:00Z', 'cli', 'health', 200, 1, {mark})"
        );
        assert!(
            log.conn.execute_batch(&insert).is_err(),
            "{mark} is not a mark"
        );
    }
}

#[test]
fn no_mark_is_refused() {
    let log = log();
    assert!(
        log.conn
            .execute_batch(
                "INSERT INTO audit (at, actor, action, status, count, counted)
                 VALUES ('2026-10-23T12:34:00Z', 'cli', 'health', 200, 1, NULL)"
            )
            .is_err(),
        "NULL is not a mark"
    );
}

#[test]
fn a_count_that_is_not_committed_leaves_the_plain_record_alone() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
    });
    let tx = log.conn.transaction().expect("tx");
    count_many(&tx, &refused(), MINUTE, 5).expect("count");
    drop(tx);
    assert_eq!(counts(&log), [(1, 1)], "only the plain record");
}

#[test]
fn a_record_left_out_of_a_count_is_plain_by_default() {
    // The record of one request names no mark and takes the column's
    // default: a record written another way must not become counted.
    let log = log();
    log.conn
        .execute_batch(
            "INSERT INTO audit (at, actor, action, target, version, status, count)
             VALUES ('2026-10-23T12:34:00Z', 'cli', 'read', NULL, NULL, 429, 1)",
        )
        .expect("a record");
    count_many(&log.conn, &refused(), MINUTE, 2).expect("count");
    assert_eq!(counts(&log), [(1, 1), (2, 2)], "the default is plain");
}

// Limits and odd inputs.

#[test]
fn a_plain_record_one_nanosecond_into_the_minute_is_apart_from_the_count() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), after(1)).expect("record");
        count_many(tx, &refused(), after(1), 2).expect("count");
    });
    assert_eq!(counts(&log), [(1, 1), (2, 2)], "apart, as before the fix");
}

#[test]
fn a_plain_record_at_the_last_nanosecond_of_the_minute_is_apart_from_the_count() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), after(59_999_999_999)).expect("record");
        count_many(tx, &refused(), after(59_999_999_999), 2).expect("count");
    });
    assert_eq!(counts(&log), [(1, 1), (2, 2)], "apart");
}

#[test]
fn a_count_of_nothing_after_a_plain_record_changes_nothing() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
        count_many(tx, &refused(), MINUTE, 0).expect("count");
    });
    assert_eq!(counts(&log), [(1, 1)], "no record and no change");
}

#[test]
fn a_count_in_the_next_minute_is_apart_from_a_plain_record_and_from_the_last_count() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
        count(tx, &refused(), MINUTE).expect("count");
        count(tx, &refused(), after(60_000_000_000)).expect("count");
    });
    let at: Vec<Value> = exported(&log.conn, None)
        .iter()
        .map(|r| r["at"].clone())
        .collect();
    assert_eq!(
        at,
        [
            "2026-10-23T12:34:00Z",
            "2026-10-23T12:34:00Z",
            "2026-10-23T12:35:00Z"
        ],
        "three records"
    );
    assert_eq!(counts(&log), [(1, 1), (2, 1), (3, 1)], "1 each");
}

#[test]
fn a_count_in_a_transaction_of_its_own_after_a_plain_record_is_apart() {
    let mut log = log();
    write(&mut log, |tx| {
        record(tx, &refused(), MINUTE).expect("record");
    });
    write(&mut log, |tx| {
        count_many(tx, &refused(), MINUTE, 3).expect("count");
    });
    write(&mut log, |tx| {
        count_many(tx, &refused(), MINUTE, 4).expect("count");
    });
    assert_eq!(counts(&log), [(1, 1), (2, 7)], "across transactions");
}
