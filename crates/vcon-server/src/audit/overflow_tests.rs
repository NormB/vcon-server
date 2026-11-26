//! A count never passes the largest number a record's count holds
//! (i64::MAX). SQLite's `+` does not fail on an integer overflow: it gives
//! a REAL, so `UPDATE audit SET count = count + ?` past i64::MAX stored a
//! floating-point count with no error, and export, which reads the count
//! as an i64, then failed on that record. PostgreSQL refuses such a sum
//! ("bigint out of range", backend/postgres/catalog.rs count_record). Here
//! a sum past i64::MAX is refused and the record keeps the count it had.

use rusqlite::Connection;
use time::OffsetDateTime;
use time::macros::datetime;

use super::testing::exported;
use super::{Action, Actor, AuditError, Entry, SCHEMA, count, count_many, record};
use crate::catalog;
use crate::envelope::Key;

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

const MINUTE: OffsetDateTime = datetime!(2026-10-23 12:34:00 UTC);
const NEXT_MINUTE: OffsetDateTime = datetime!(2026-10-23 12:35:00 UTC);
const MOST: i64 = i64::MAX;

fn refused() -> Entry {
    Entry {
        actor: Actor::Cli,
        action: Action::Read,
        target: None,
        version: None,
        status: 429,
    }
}

/// `count_many` of `times` in a transaction of its own, committed when it
/// answers Ok and dropped when it does not.
fn add(log: &mut Log, entry: &Entry, at: OffsetDateTime, times: u64) -> Result<(), AuditError> {
    let tx = log.conn.transaction().expect("tx");
    let answer = count_many(&tx, entry, at, times);
    if answer.is_ok() {
        tx.commit().expect("commit");
    }
    answer
}

/// Each record's count, oldest first, as export writes them.
fn counts(log: &Log) -> Vec<i64> {
    exported(&log.conn, None)
        .iter()
        .map(|r| r["count"].as_i64().expect("an integer count"))
        .collect()
}

/// The SQLite type of each record's count, oldest first.
fn types(log: &Log) -> Vec<String> {
    let mut statement = log
        .conn
        .prepare("SELECT typeof(count) FROM audit ORDER BY seq")
        .expect("prepare");
    statement
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("types")
}

fn at_most(log: &mut Log) {
    add(log, &refused(), MINUTE, MOST as u64).expect("a first count of the largest number");
}

// What a count past the largest number does: it is refused, and nothing
// changes.

#[test]
fn one_onto_the_largest_count_is_refused() {
    let mut log = log();
    at_most(&mut log);
    let answer = add(&mut log, &refused(), MINUTE, 1);
    assert!(
        matches!(answer, Err(AuditError::CountTooLarge)),
        "no count is larger than i64::MAX: {answer:?}"
    );
}

#[test]
fn a_record_at_the_largest_count_keeps_it_after_a_refusal() {
    let mut log = log();
    at_most(&mut log);
    let _ = add(&mut log, &refused(), MINUTE, 1);
    assert_eq!(counts(&log), [MOST], "the record keeps the largest count");
}

#[test]
fn a_refused_count_leaves_an_integer_count() {
    let mut log = log();
    at_most(&mut log);
    let _ = add(&mut log, &refused(), MINUTE, 1);
    assert_eq!(types(&log), ["integer"], "no REAL is stored");
}

#[test]
fn a_refused_count_makes_no_second_record() {
    let mut log = log();
    at_most(&mut log);
    let _ = add(&mut log, &refused(), MINUTE, 1);
    assert_eq!(
        counts(&log).len(),
        1,
        "the refusal does not start a record of its own"
    );
}

#[test]
fn the_largest_number_onto_seven_is_refused_and_seven_kept() {
    let mut log = log();
    add(&mut log, &refused(), MINUTE, 7).expect("seven");
    let answer = add(&mut log, &refused(), MINUTE, u64::MAX);
    assert!(
        matches!(answer, Err(AuditError::CountTooLarge)),
        "seven and the largest number: {answer:?}"
    );
    assert_eq!(counts(&log), [7], "as it was");
}

#[test]
fn the_largest_count_onto_one_is_refused() {
    let mut log = log();
    add(&mut log, &refused(), MINUTE, 1).expect("one");
    let answer = add(&mut log, &refused(), MINUTE, MOST as u64);
    assert!(
        matches!(answer, Err(AuditError::CountTooLarge)),
        "one and i64::MAX: {answer:?}"
    );
    assert_eq!(counts(&log), [1], "as it was");
}

#[test]
fn a_single_count_onto_the_largest_count_is_refused() {
    let mut log = log();
    at_most(&mut log);
    let tx = log.conn.transaction().expect("tx");
    let answer = count(&tx, &refused(), MINUTE);
    drop(tx);
    assert!(
        matches!(answer, Err(AuditError::CountTooLarge)),
        "count is count_many of one: {answer:?}"
    );
    assert_eq!(counts(&log), [MOST], "as it was");
}

#[test]
fn the_log_still_exports_after_a_refusal() {
    let mut log = log();
    at_most(&mut log);
    let _ = add(&mut log, &refused(), MINUTE, 1);
    let mut out = Vec::new();
    super::export(&log.conn, None, &mut out).expect("every record reads back as an integer count");
    assert_eq!(
        String::from_utf8(out).expect("UTF-8").lines().count(),
        1,
        "one record"
    );
}

#[test]
fn a_refusal_names_the_limit() {
    let text = AuditError::CountTooLarge.to_string();
    assert!(
        text.contains("9223372036854775807"),
        "the refusal names the largest count: {text}"
    );
}

#[test]
fn a_refused_count_in_a_transaction_keeps_the_writes_before_it() {
    let mut log = log();
    at_most(&mut log);
    let other = Entry {
        status: 401,
        ..refused()
    };
    let tx = log.conn.transaction().expect("tx");
    count_many(&tx, &other, MINUTE, 3).expect("another record");
    assert!(
        count_many(&tx, &refused(), MINUTE, 1).is_err(),
        "the full record refuses"
    );
    tx.commit()
        .expect("the local log goes on after a refused write");
    assert_eq!(
        counts(&log),
        [MOST, 3],
        "the refusal changed nothing it did not own"
    );
}

#[test]
fn the_largest_number_onto_the_largest_count_is_refused() {
    let mut log = log();
    at_most(&mut log);
    assert!(
        matches!(
            add(&mut log, &refused(), MINUTE, u64::MAX),
            Err(AuditError::CountTooLarge)
        ),
        "the largest onto the largest"
    );
    assert_eq!(counts(&log), [MOST], "as it was");
}

#[test]
fn every_count_after_the_first_refusal_is_refused_too() {
    let mut log = log();
    at_most(&mut log);
    for times in [1, 2, 1000] {
        assert!(
            add(&mut log, &refused(), MINUTE, times).is_err(),
            "{times} onto the largest"
        );
    }
    assert_eq!(counts(&log), [MOST], "as it was");
}

// What still counts.

#[test]
fn a_count_that_reaches_the_largest_exactly_is_kept() {
    let mut log = log();
    add(&mut log, &refused(), MINUTE, (MOST - 1) as u64).expect("one below");
    add(&mut log, &refused(), MINUTE, 1).expect("exactly the largest");
    assert_eq!(counts(&log), [MOST], "i64::MAX is a count");
    assert_eq!(types(&log), ["integer"], "and an integer");
}

#[test]
fn a_first_count_of_the_largest_number_is_kept() {
    let mut log = log();
    at_most(&mut log);
    assert_eq!(counts(&log), [MOST], "a new record takes the largest count");
}

#[test]
fn a_first_count_past_the_largest_number_is_the_largest() {
    let mut log = log();
    add(&mut log, &refused(), MINUTE, u64::MAX).expect("a new record");
    assert_eq!(
        counts(&log),
        [MOST],
        "as PostgreSQL: u64 past i64::MAX is written as i64::MAX"
    );
}

#[test]
fn two_halves_make_the_largest() {
    let mut log = log();
    let half = (MOST / 2) as u64;
    add(&mut log, &refused(), MINUTE, half).expect("half");
    add(&mut log, &refused(), MINUTE, half + 1).expect("the other half");
    assert_eq!(counts(&log), [MOST], "the sum is the largest");
}

#[test]
fn a_count_in_the_next_minute_makes_its_own_record_beside_a_full_one() {
    let mut log = log();
    at_most(&mut log);
    add(&mut log, &refused(), NEXT_MINUTE, 1).expect("a new minute");
    assert_eq!(counts(&log), [MOST, 1], "a record of its own");
}

#[test]
fn another_status_counts_beside_a_full_record() {
    let mut log = log();
    at_most(&mut log);
    let other = Entry {
        status: 503,
        ..refused()
    };
    add(&mut log, &other, MINUTE, 5).expect("another record");
    add(&mut log, &other, MINUTE, 5).expect("added to");
    assert_eq!(counts(&log), [MOST, 10], "only the full record refuses");
}

#[test]
fn a_plain_record_beside_a_full_count_is_written() {
    let mut log = log();
    at_most(&mut log);
    let tx = log.conn.transaction().expect("tx");
    record(&tx, &refused(), MINUTE).expect("a plain record is no count");
    tx.commit().expect("commit");
    assert_eq!(counts(&log), [MOST, 1], "the plain record keeps 1");
}

#[test]
fn a_count_of_nothing_onto_the_largest_answers_ok() {
    let mut log = log();
    at_most(&mut log);
    add(&mut log, &refused(), MINUTE, 0).expect("a count of nothing adds nothing");
    assert_eq!(counts(&log), [MOST], "as it was");
}

#[test]
fn a_count_below_the_limit_adds_as_before() {
    let mut log = log();
    add(&mut log, &refused(), MINUTE, 40).expect("forty");
    add(&mut log, &refused(), MINUTE, 2).expect("two");
    assert_eq!(counts(&log), [42], "the ordinary sum");
}

#[test]
fn a_plain_record_at_the_largest_is_not_a_count_target() {
    let mut log = log();
    let tx = log.conn.transaction().expect("tx");
    record(&tx, &refused(), MINUTE).expect("plain");
    tx.commit().expect("commit");
    log.conn
        .execute("UPDATE audit SET count = ?1 WHERE counted = 0", [MOST])
        .expect("a plain record set to the largest");
    add(&mut log, &refused(), MINUTE, 1).expect("a count makes its own record");
    assert_eq!(
        counts(&log),
        [MOST, 1],
        "the plain record is never added to"
    );
}

// The bounds i64::MAX - 1, i64::MAX and i64::MAX + 1, by each way a count
// is added.

/// A counted record at `count`, made by one count of `count`.
fn at(log: &mut Log, count: i64) {
    add(log, &refused(), MINUTE, count as u64).expect("a first count");
}

#[test]
fn a_single_count_onto_one_below_the_largest_reaches_it() {
    let mut log = log();
    at(&mut log, MOST - 1);
    let tx = log.conn.transaction().expect("tx");
    count(&tx, &refused(), MINUTE).expect("one more");
    tx.commit().expect("commit");
    assert_eq!(counts(&log), [MOST], "i64::MAX by count");
}

#[test]
fn two_onto_two_below_the_largest_reaches_it() {
    let mut log = log();
    at(&mut log, MOST - 2);
    add(&mut log, &refused(), MINUTE, 2).expect("two more");
    assert_eq!(counts(&log), [MOST], "the sum is the largest");
}

#[test]
fn a_first_count_of_one_is_one() {
    let mut log = log();
    add(&mut log, &refused(), MINUTE, 1).expect("one");
    assert_eq!(counts(&log), [1], "the ordinary first count");
}

#[test]
fn export_writes_the_largest_count_exactly() {
    let mut log = log();
    at_most(&mut log);
    let mut out = Vec::new();
    super::export(&log.conn, None, &mut out).expect("export");
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.contains("\"count\":9223372036854775807"),
        "the number, not a float: {text}"
    );
}

#[test]
fn a_page_of_the_log_reads_the_largest_count() {
    let mut log = log();
    at_most(&mut log);
    let mut out = Vec::new();
    super::page(
        &log.conn,
        &super::AuditQuery {
            after: 0,
            limit: 10,
        },
        &mut out,
    )
    .expect("a page");
    let text = String::from_utf8(out).expect("UTF-8");
    assert!(
        text.contains("9223372036854775807"),
        "the page has it: {text}"
    );
}

#[test]
fn a_record_one_below_the_largest_is_an_integer() {
    let mut log = log();
    at(&mut log, MOST - 1);
    assert_eq!(types(&log), ["integer"], "no REAL");
    assert_eq!(counts(&log), [MOST - 1], "as written");
}

#[test]
fn many_small_counts_below_the_bound_all_add() {
    let mut log = log();
    at(&mut log, MOST - 10);
    for _ in 0..10 {
        add(&mut log, &refused(), MINUTE, 1).expect("room for each");
    }
    assert_eq!(counts(&log), [MOST], "ten more reach the largest");
}

#[test]
fn a_count_of_the_largest_in_two_minutes_makes_two_full_records() {
    let mut log = log();
    at_most(&mut log);
    add(&mut log, &refused(), NEXT_MINUTE, u64::MAX).expect("its own record");
    assert_eq!(counts(&log), [MOST, MOST], "each minute holds the largest");
}

#[test]
fn two_onto_one_below_the_largest_is_refused() {
    let mut log = log();
    at(&mut log, MOST - 1);
    assert!(
        matches!(
            add(&mut log, &refused(), MINUTE, 2),
            Err(AuditError::CountTooLarge)
        ),
        "i64::MAX + 1"
    );
    assert_eq!(counts(&log), [MOST - 1], "as it was");
}

#[test]
fn the_largest_onto_one_below_the_largest_is_refused() {
    let mut log = log();
    at(&mut log, MOST - 1);
    assert!(
        add(&mut log, &refused(), MINUTE, MOST as u64).is_err(),
        "far past"
    );
    assert_eq!(types(&log), ["integer"], "no REAL");
}

#[test]
fn a_refused_count_after_ten_that_fit_keeps_the_largest() {
    let mut log = log();
    at(&mut log, MOST - 10);
    for _ in 0..10 {
        add(&mut log, &refused(), MINUTE, 1).expect("fits");
    }
    assert!(
        add(&mut log, &refused(), MINUTE, 1).is_err(),
        "the eleventh does not"
    );
    assert_eq!(counts(&log), [MOST], "as it was");
}

#[test]
fn a_refusal_is_no_sqlite_error() {
    let mut log = log();
    at_most(&mut log);
    let answer = add(&mut log, &refused(), MINUTE, 1);
    assert!(
        !matches!(answer, Err(AuditError::Sqlite(_))),
        "the refusal is the count's, not SQLite's: {answer:?}"
    );
}

#[test]
fn a_refused_count_then_a_plain_record_in_one_transaction_commits() {
    let mut log = log();
    at_most(&mut log);
    let tx = log.conn.transaction().expect("tx");
    assert!(count_many(&tx, &refused(), MINUTE, 1).is_err(), "refused");
    record(&tx, &refused(), MINUTE).expect("a plain record");
    tx.commit().expect("commit");
    assert_eq!(counts(&log), [MOST, 1], "the plain record is kept");
}

#[test]
fn a_refused_count_then_a_count_of_another_minute_in_one_transaction_commits() {
    let mut log = log();
    at_most(&mut log);
    let tx = log.conn.transaction().expect("tx");
    assert!(count_many(&tx, &refused(), MINUTE, 1).is_err(), "refused");
    count_many(&tx, &refused(), NEXT_MINUTE, 4).expect("another minute");
    tx.commit().expect("commit");
    assert_eq!(counts(&log), [MOST, 4], "the next minute's record");
}

#[test]
fn a_page_after_a_refusal_reads_every_record() {
    let mut log = log();
    at_most(&mut log);
    let _ = add(&mut log, &refused(), MINUTE, 1);
    add(&mut log, &refused(), NEXT_MINUTE, 1).expect("another");
    let mut out = Vec::new();
    super::page(
        &log.conn,
        &super::AuditQuery {
            after: 0,
            limit: 10,
        },
        &mut out,
    )
    .expect("a page");
    assert_eq!(
        String::from_utf8(out).expect("UTF-8").lines().count(),
        2,
        "both records"
    );
}

#[test]
fn a_first_count_one_past_the_largest_is_the_largest() {
    let mut log = log();
    add(&mut log, &refused(), MINUTE, MOST as u64 + 1).expect("a new record");
    assert_eq!(
        counts(&log),
        [MOST],
        "i64::MAX + 1 as a first count is written as i64::MAX"
    );
}

#[test]
fn a_count_of_nothing_onto_one_below_the_largest_adds_nothing() {
    let mut log = log();
    at(&mut log, MOST - 1);
    add(&mut log, &refused(), MINUTE, 0).expect("nothing");
    assert_eq!(counts(&log), [MOST - 1], "as it was");
}

#[test]
fn a_record_with_a_target_and_a_version_is_bounded_too() {
    let mut log = log();
    let read = Entry {
        target: Some("01a07da8-c2bb-83e5-b9a2-279e0d16bc46".into()),
        version: Some(crate::ids::VersionId([0x22; 16])),
        ..refused()
    };
    add(&mut log, &read, MINUTE, u64::MAX).expect("full");
    assert!(
        add(&mut log, &read, MINUTE, 1).is_err(),
        "the match on target and version finds the full record"
    );
    assert_eq!(counts(&log), [MOST], "as it was");
}

#[test]
fn a_record_with_no_answer_is_bounded_too() {
    let mut log = log();
    let unanswered = Entry {
        status: super::NO_ANSWER,
        ..refused()
    };
    add(&mut log, &unanswered, MINUTE, u64::MAX).expect("full");
    assert!(
        add(&mut log, &unanswered, MINUTE, 1).is_err(),
        "a NULL status matches its record"
    );
    assert_eq!(counts(&log), [MOST], "as it was");
}

#[test]
fn another_actor_counts_beside_a_full_record() {
    let mut log = log();
    at_most(&mut log);
    let sweep = Entry {
        actor: Actor::Sweep,
        ..refused()
    };
    add(&mut log, &sweep, MINUTE, 2).expect("its own record");
    assert_eq!(counts(&log), [MOST, 2], "only the full record refuses");
}

#[test]
fn another_action_counts_beside_a_full_record() {
    let mut log = log();
    at_most(&mut log);
    let purge = Entry {
        action: Action::Purge,
        ..refused()
    };
    add(&mut log, &purge, MINUTE, 2).expect("its own record");
    assert_eq!(counts(&log), [MOST, 2], "only the full record refuses");
}

#[test]
fn a_time_later_in_the_same_minute_finds_the_full_record() {
    let mut log = log();
    at_most(&mut log);
    let later = datetime!(2026-10-23 12:34:59.999 UTC);
    assert!(
        add(&mut log, &refused(), later, 1).is_err(),
        "the minute's record is full"
    );
    assert_eq!(counts(&log), [MOST], "as it was");
}

// The tenant catalog's own path: Store::count adds one in a transaction of
// its own.

fn tenant_store() -> (crate::tenants::Fixture, std::sync::Arc<crate::store::Store>) {
    let f = crate::tenants::fixture();
    let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
    let store = f.tenants.store(id).expect("opens");
    (f, store)
}

fn tenant_counts(store: &crate::store::Store) -> Vec<i64> {
    store
        .records()
        .iter()
        .filter(|r| r["action"] == "read")
        .map(|r| r["count"].as_i64().expect("an integer count"))
        .collect()
}

#[test]
fn a_tenant_count_onto_one_below_the_largest_reaches_it() {
    let (_f, store) = tenant_store();
    store.count(&refused()).expect("a first count");
    store
        .conn_for_test()
        .execute(
            "UPDATE audit SET count = ?1 WHERE counted = 1 AND action = 'read'",
            [MOST - 1],
        )
        .expect("one below");
    store.count(&refused()).expect("one more");
    assert_eq!(
        tenant_counts(&store),
        [MOST],
        "the tenant's record reaches the largest"
    );
}

#[test]
fn a_tenant_count_onto_the_largest_is_refused_and_kept() {
    let (_f, store) = tenant_store();
    store.count(&refused()).expect("a first count");
    store
        .conn_for_test()
        .execute(
            "UPDATE audit SET count = ?1 WHERE counted = 1 AND action = 'read'",
            [MOST],
        )
        .expect("the largest");
    assert_eq!(
        store.count(&refused()),
        Err(crate::store::StoreError::AuditFailed),
        "refused as an audit failure"
    );
    assert_eq!(tenant_counts(&store), [MOST], "as it was");
}

#[test]
fn a_tenant_export_after_a_refusal_reads_back() {
    let (_f, store) = tenant_store();
    store.count(&refused()).expect("a first count");
    store
        .conn_for_test()
        .execute(
            "UPDATE audit SET count = ?1 WHERE counted = 1 AND action = 'read'",
            [MOST],
        )
        .expect("the largest");
    let _ = store.count(&refused());
    let mut out = Vec::new();
    store
        .export(None, &mut out)
        .expect("every record reads back");
    assert!(
        String::from_utf8(out)
            .expect("UTF-8")
            .contains("9223372036854775807"),
        "the largest count"
    );
}
