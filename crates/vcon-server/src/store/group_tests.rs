//! Reads that reach their last step together commit their records in one
//! transaction (store/group.rs), and none is answered before that commit.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::time::{Duration, Instant};

use rusqlite::{Connection, params};
use serde_json::Value;
use uuid::Uuid;

use super::testing::vcon;
use super::tests::{Fixture, fixture};
use super::{StoreError, Stored};
use crate::backend::CommitError;

const A: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bca1";
const B: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bcb2";
const C: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bcc3";

/// The longest any test here waits for its readers.
const PATIENCE: Duration = Duration::from_secs(60);

fn stored(f: &Fixture, uuid: &str, subject: &str) -> (Uuid, Vec<u8>) {
    let (found, body) = vcon(uuid, subject);
    f.store.create(&f.caller, &found, &body).expect("stored");
    (found.uuid, body)
}

/// Reads each of `uuids` on a thread of its own, holding every reader at
/// its last step until all of them wait in the queue behind the catalog
/// lock, which this thread holds. `held` runs with that lock, after the
/// queue is full and before the readers go on. Each reader's own first
/// arrival is held; a read that starts again is not.
fn together(
    f: &Arc<Fixture>,
    uuids: &[Uuid],
    held: impl FnOnce(&Connection),
) -> Vec<Result<Stored, StoreError>> {
    let n = uuids.len();
    let first = Arc::new(Barrier::new(n + 1));
    let second = Arc::new(Barrier::new(n + 1));
    let arrivals = Arc::new(AtomicUsize::new(0));
    {
        let (first, second, arrivals) = (first.clone(), second.clone(), arrivals.clone());
        *f.store.hooks.before_confirm.lock().expect("hook") = Some(Arc::new(move || {
            if arrivals.fetch_add(1, Ordering::SeqCst) < n {
                first.wait();
                second.wait();
            }
        }));
    }
    let readers: Vec<_> = uuids
        .iter()
        .map(|&uuid| {
            let f = f.clone();
            let (answer, answered) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = answer.send(f.store.get(&f.caller, uuid));
            });
            answered
        })
        .collect();
    first.wait();
    let conn = f.store.conn();
    second.wait();
    let deadline = Instant::now() + PATIENCE;
    while f.store.group.waiting() < n {
        assert!(
            Instant::now() < deadline,
            "{} of {n} readers queued",
            f.store.group.waiting()
        );
        std::thread::yield_now();
    }
    held(&conn);
    drop(conn);
    let outcomes = readers
        .into_iter()
        .enumerate()
        .map(|(i, answered)| {
            answered
                .recv_timeout(PATIENCE)
                .unwrap_or_else(|_| panic!("reader {i} of {n} not answered within {PATIENCE:?}"))
        })
        .collect();
    *f.store.hooks.before_confirm.lock().expect("hook") = None;
    outcomes
}

/// `f.store.get` on a thread of its own, failing the test if it is not
/// answered within PATIENCE, so a read that never ends fails the test
/// instead of hanging it.
fn bounded_get(f: &Arc<Fixture>, uuid: Uuid) -> Result<Stored, StoreError> {
    let (answer, answered) = mpsc::channel();
    let g = f.clone();
    std::thread::spawn(move || {
        let _ = answer.send(g.store.get(&g.caller, uuid));
    });
    answered
        .recv_timeout(PATIENCE)
        .unwrap_or_else(|_| panic!("the read was not answered within {PATIENCE:?}"))
}

fn batches(f: &Fixture) -> Vec<usize> {
    f.store.group.batches.lock().expect("batches").clone()
}

/// The read records, as (target, status), in the log's order.
fn reads(f: &Fixture) -> Vec<(Value, Value)> {
    f.store
        .records()
        .iter()
        .filter(|r| r["action"] == "read")
        .map(|r| (r["target"].clone(), r["status"].clone()))
        .collect()
}

#[test]
fn reads_that_wait_together_are_committed_in_one_batch() {
    let f = Arc::new(fixture());
    let (uuid, body) = stored(&f, A, "first");
    let outcomes = together(&f, &[uuid; 6], |_| {});
    for outcome in outcomes {
        assert_eq!(
            outcome.expect("read").body,
            body,
            "each reader gets the body"
        );
    }
    assert_eq!(batches(&f), [6], "the six reads are one batch");
    assert_eq!(reads(&f).len(), 6, "each read is recorded once");
}

#[test]
fn a_lone_read_is_a_batch_of_one() {
    let f = Arc::new(fixture());
    let (uuid, body) = stored(&f, A, "first");
    assert_eq!(bounded_get(&f, uuid).expect("read").body, body, "the body");
    assert_eq!(batches(&f), [1], "one read, one batch");
}

#[test]
fn reads_one_after_another_are_batches_of_one() {
    let f = Arc::new(fixture());
    let (uuid, _) = stored(&f, A, "first");
    for _ in 0..5 {
        bounded_get(&f, uuid).expect("read");
    }
    assert_eq!(batches(&f), [1; 5], "no read waits for a later one");
}

#[test]
fn a_batch_answers_each_read_by_its_own_vcon() {
    let f = Arc::new(fixture());
    let (a, a_body) = stored(&f, A, "a");
    let (b, _) = stored(&f, B, "b");
    let (c, c_body) = stored(&f, C, "c");
    let outcomes = together(&f, &[a, b, c], |conn| {
        conn.execute(
            "UPDATE vcon SET current = NULL, deleted_at = 1 WHERE uuid = ?1",
            [b.as_bytes()],
        )
        .expect("b deleted");
    });
    assert_eq!(
        outcomes[0].as_ref().map(|s| &s.body),
        Ok(&a_body),
        "a reads"
    );
    assert_eq!(
        outcomes[1],
        Err(StoreError::Gone),
        "b was deleted while it waited"
    );
    assert_eq!(
        outcomes[2].as_ref().map(|s| &s.body),
        Ok(&c_body),
        "c reads"
    );
    assert_eq!(batches(&f), [3], "one batch");
    let mut got = reads(&f);
    got.sort_by_key(|(t, _)| t.to_string());
    assert_eq!(
        got,
        [
            (Value::from(A), Value::from(200)),
            (Value::from(B), Value::from(410)),
            (Value::from(C), Value::from(200)),
        ],
        "each read is recorded with its own outcome"
    );
}

#[test]
fn a_read_whose_vcon_was_never_there_is_refused_in_the_batch() {
    let f = Arc::new(fixture());
    let (a, a_body) = stored(&f, A, "a");
    let (b, _) = stored(&f, B, "b");
    let outcomes = together(&f, &[a, b], |conn| {
        conn.execute("DELETE FROM vcon WHERE uuid = ?1", [b.as_bytes()])
            .expect("row gone");
        conn.execute("DELETE FROM version WHERE uuid = ?1", [b.as_bytes()])
            .expect("versions gone");
    });
    assert_eq!(
        outcomes[0].as_ref().map(|s| &s.body),
        Ok(&a_body),
        "a reads"
    );
    assert_eq!(outcomes[1], Err(StoreError::NotFound), "b is not found");
    let statuses: Vec<_> = reads(&f).into_iter().map(|(_, s)| s).collect();
    assert!(
        statuses.contains(&Value::from(404)),
        "the refusal is recorded: {statuses:?}"
    );
}

#[test]
fn a_read_whose_version_changed_in_the_batch_starts_again_alone() {
    let f = Arc::new(fixture());
    let (a, _) = stored(&f, A, "first");
    let (b, b_body) = stored(&f, B, "b");
    let (_, newer) = vcon(A, "newer");
    let (version, wrapped) = f.store.write_version(a, &newer).expect("written");
    let key = f.store.tenant_key.id();
    let outcomes = together(&f, &[a, b], |conn| {
        conn.execute(
            "INSERT INTO version
                 (version, uuid, form, gzip, size, stored_at, tenant_key_id, wrapped_key)
             VALUES (?1, ?2, 'unsigned', 0, 0, 0, ?3, ?4)",
            params![version.0, a.as_bytes(), key.bytes(), wrapped],
        )
        .expect("new version");
        conn.execute(
            "UPDATE vcon SET current = ?1 WHERE uuid = ?2",
            params![version.0, a.as_bytes()],
        )
        .expect("made current");
    });
    let read = outcomes[0].as_ref().expect("a reads");
    assert_eq!(
        (read.version, &read.body),
        (version, &newer),
        "the new version"
    );
    assert_eq!(
        outcomes[1].as_ref().map(|s| &s.body),
        Ok(&b_body),
        "b reads"
    );
    assert_eq!(batches(&f), [2, 1], "a starts again in a batch of its own");
    assert_eq!(reads(&f).len(), 2, "one record for each request");
}

/// A batch that records nothing (its read starts again) commits nothing:
/// the next commit, which a fault fails, is the second try's, so that
/// read is not answered.
#[test]
fn a_batch_that_records_nothing_commits_nothing() {
    let f = Arc::new(fixture());
    let (a, _) = stored(&f, A, "first");
    let (_, newer) = vcon(A, "newer");
    let (version, wrapped) = f.store.write_version(a, &newer).expect("written");
    let key = f.store.tenant_key.id();
    let outcomes = together(&f, &[a], |conn| {
        conn.execute(
            "INSERT INTO version
                 (version, uuid, form, gzip, size, stored_at, tenant_key_id, wrapped_key)
             VALUES (?1, ?2, 'unsigned', 0, 0, 0, ?3, ?4)",
            params![version.0, a.as_bytes(), key.bytes(), wrapped],
        )
        .expect("new version");
        conn.execute(
            "UPDATE vcon SET current = ?1 WHERE uuid = ?2",
            params![version.0, a.as_bytes()],
        )
        .expect("made current");
        f.store.catalog.fail_commit(1, CommitError::RolledBack);
    });
    assert_eq!(batches(&f), [1, 1], "the first try, then the second");
    assert_eq!(
        outcomes,
        vec![Err(StoreError::Io)],
        "the fault met the second try's commit, so the first made none"
    );
}

/// A read whose record cannot be written fails alone: the other reads of
/// its batch are answered and recorded.
#[test]
fn a_record_that_fails_fails_only_its_own_read() {
    let f = Arc::new(fixture());
    let (a, a_body) = stored(&f, A, "a");
    let (b, _) = stored(&f, B, "b");
    let outcomes = together(&f, &[a, b], |conn| {
        conn.execute_batch(&format!(
            "CREATE TEMP TRIGGER refuse_b BEFORE INSERT ON main.audit
             WHEN NEW.target = '{B}' BEGIN SELECT RAISE(ABORT, 'refused'); END;"
        ))
        .expect("trigger");
    });
    f.store
        .conn()
        .execute_batch("DROP TRIGGER temp.refuse_b")
        .expect("trigger dropped");
    assert_eq!(
        outcomes[0].as_ref().map(|s| &s.body),
        Ok(&a_body),
        "a reads"
    );
    assert_eq!(
        outcomes[1],
        Err(StoreError::AuditFailed),
        "b is not answered"
    );
    assert_eq!(batches(&f), [2], "one batch");
    assert_eq!(
        reads(&f),
        [(Value::from(A), Value::from(200))],
        "only a's read is recorded"
    );
}

/// Each read of a batch of several runs in a part of its own, and a lone
/// read in none. SQLite undoes only a failed statement, so on the local
/// catalog a read without a part fails alone too (the test above); the
/// parts are for PostgreSQL, which aborts the whole transaction at a
/// failed statement, and the PostgreSQL catalog's tests need a live
/// server. So this counts the parts the batch opens.
#[test]
fn each_read_of_a_batch_of_several_runs_in_a_part_of_its_own() {
    let f = Arc::new(fixture());
    let (uuid, _) = stored(&f, A, "first");
    together(&f, &[uuid; 3], |_| {});
    assert_eq!(
        f.store.group.parts.load(Ordering::SeqCst),
        3,
        "a part for each of the three"
    );
}

#[test]
fn a_lone_read_runs_in_no_part() {
    let f = Arc::new(fixture());
    let (uuid, _) = stored(&f, A, "first");
    bounded_get(&f, uuid).expect("read");
    assert_eq!(
        f.store.group.parts.load(Ordering::SeqCst),
        0,
        "the transaction is its own"
    );
}

/// A second connection to the fixture's catalog file, as another process
/// would open it.
fn second_connection(f: &Fixture) -> Connection {
    let key = crate::envelope::catalog_key(&f.tenant_key()).expect("catalog key");
    crate::catalog::open(&f.dir.path().join("catalog.sqlite"), &key, "")
        .expect("a second connection")
}

/// Runs `f.store.get` on a thread of its own while another connection
/// holds the catalog's write lock, and says whether the read reached its
/// last step within `within`; the lock is let go after that, and the read's
/// answer returned.
fn read_while_written(
    f: &Arc<Fixture>,
    uuid: Uuid,
    within: Duration,
) -> (bool, Result<Stored, StoreError>) {
    let mut writer = second_connection(f);
    let write = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("the write lock");
    let (reached, at_last_step) = mpsc::channel();
    let reached = std::sync::Mutex::new(reached);
    *f.store.hooks.before_confirm.lock().expect("hook") = Some(Arc::new(move || {
        let _ = reached.lock().expect("sender").send(());
    }));
    let (answer, answered) = mpsc::channel();
    let g = f.clone();
    std::thread::spawn(move || {
        let _ = answer.send(g.store.get(&g.caller, uuid));
    });
    let first_step_ended = at_last_step.recv_timeout(within).is_ok();
    drop(write);
    let outcome = answered
        .recv_timeout(PATIENCE)
        .unwrap_or_else(|_| panic!("the read was not answered within {PATIENCE:?}"));
    *f.store.hooks.before_confirm.lock().expect("hook") = None;
    (first_step_ended, outcome)
}

/// A read's first step only reads, so a writer holding the catalog does
/// not hold it up (on PostgreSQL, it takes no lock on the tenant); only
/// its last step, which records it, waits for the writer.
#[test]
fn a_reads_first_step_does_not_wait_for_a_writer() {
    let f = Arc::new(fixture());
    let (uuid, body) = stored(&f, A, "first");
    let (first_step_ended, outcome) = read_while_written(&f, uuid, Duration::from_secs(3));
    assert!(
        first_step_ended,
        "the first step ended while another connection held the write lock"
    );
    assert_eq!(
        outcome.expect("read").body,
        body,
        "the read is answered after"
    );
    assert_eq!(reads(&f).len(), 1, "and recorded once");
}

/// A read of a deleted vCon is refused in its last step, which records the
/// refusal; its first step, which found the vCon deleted, waits for no
/// writer either.
#[test]
fn a_read_of_a_deleted_vcon_is_refused_and_recorded_in_its_last_step() {
    let f = Arc::new(fixture());
    let (uuid, _) = stored(&f, A, "first");
    f.store
        .delete(&f.caller, uuid, crate::retention::Retention::Years(7))
        .expect("deleted");
    let (first_step_ended, outcome) = read_while_written(&f, uuid, Duration::from_secs(3));
    assert!(
        first_step_ended,
        "the first step did not wait for the writer"
    );
    assert_eq!(
        outcome,
        Err(StoreError::Gone),
        "the read is refused as gone"
    );
    assert_eq!(
        reads(&f),
        [(Value::from(A), Value::from(410))],
        "the refusal is recorded once"
    );
}

/// The same for a uuid no vCon ever had: refused as not found, and
/// recorded.
#[test]
fn a_read_of_an_unknown_uuid_is_refused_and_recorded_in_its_last_step() {
    let f = Arc::new(fixture());
    let uuid = Uuid::parse_str(B).expect("uuid");
    let (first_step_ended, outcome) = read_while_written(&f, uuid, Duration::from_secs(3));
    assert!(
        first_step_ended,
        "the first step did not wait for the writer"
    );
    assert_eq!(
        outcome,
        Err(StoreError::NotFound),
        "the read is refused as not found"
    );
    assert_eq!(
        reads(&f),
        [(Value::from(B), Value::from(404))],
        "the refusal is recorded once"
    );
}

/// A vCon stored between a read's first step, which found nothing, and
/// its last is read: the last step finds it, and the read starts again.
#[test]
fn a_vcon_stored_after_the_first_step_found_nothing_is_read() {
    let f = Arc::new(fixture());
    let uuid = Uuid::parse_str(A).expect("uuid");
    let (found, body) = vcon(A, "late");
    let once = std::sync::Mutex::new(Some(f.clone()));
    *f.store.hooks.before_confirm.lock().expect("hook") = Some(Arc::new(move || {
        if let Some(f) = once.lock().expect("once").take() {
            f.store
                .create(&f.caller, &found, &body)
                .expect("stored meanwhile");
        }
    }));
    let outcome = bounded_get(&f, uuid);
    *f.store.hooks.before_confirm.lock().expect("hook") = None;
    let (_, want) = vcon(A, "late");
    assert_eq!(
        outcome.expect("read").body,
        want,
        "the read finds the new vCon"
    );
    assert_eq!(
        reads(&f),
        [(Value::from(A), Value::from(200))],
        "one record, for the read that was answered"
    );
}

#[test]
fn a_batch_whose_commit_rolls_back_answers_no_read() {
    let f = Arc::new(fixture());
    let (uuid, _) = stored(&f, A, "first");
    let outcomes = together(&f, &[uuid; 4], |_| {
        f.store.catalog.fail_commit(1, CommitError::RolledBack);
    });
    assert_eq!(
        outcomes,
        vec![Err(StoreError::Io); 4],
        "no reader gets a body without its record"
    );
    assert_eq!(reads(&f), [], "nothing was kept");
}

#[test]
fn a_batch_whose_commit_is_unknown_answers_no_read() {
    let f = Arc::new(fixture());
    let (uuid, _) = stored(&f, A, "first");
    let outcomes = together(&f, &[uuid; 3], |_| {
        f.store.catalog.fail_commit(1, CommitError::Unknown);
    });
    assert_eq!(
        outcomes,
        vec![Err(StoreError::Io); 3],
        "a commit that may not have been kept answers no reader"
    );
}

#[test]
fn a_failed_batch_leaves_the_next_read_working() {
    let f = Arc::new(fixture());
    let (uuid, body) = stored(&f, A, "first");
    together(&f, &[uuid; 2], |_| {
        f.store.catalog.fail_commit(1, CommitError::RolledBack);
    });
    assert_eq!(
        bounded_get(&f, uuid).expect("read").body,
        body,
        "the next read is answered"
    );
    assert_eq!(reads(&f).len(), 1, "and recorded");
}

/// Eight threads reading at once, many times, all end, all are answered,
/// and every read is recorded exactly once.
#[test]
fn many_readers_at_once_are_all_answered_and_recorded() {
    let f = Arc::new(fixture());
    let (uuid, body) = stored(&f, A, "first");
    let (done, finished) = mpsc::channel();
    for _ in 0..8 {
        let (f, body, done) = (f.clone(), body.clone(), done.clone());
        std::thread::spawn(move || {
            for _ in 0..50 {
                assert_eq!(
                    f.store.get(&f.caller, uuid).expect("read").body,
                    body,
                    "each reader gets the body"
                );
            }
            done.send(()).expect("the test waits");
        });
    }
    for i in 0..8 {
        finished
            .recv_timeout(PATIENCE)
            .unwrap_or_else(|_| panic!("reader {i} of 8 did not end within {PATIENCE:?}"));
    }
    assert_eq!(reads(&f).len(), 400, "every read recorded once");
    let total: usize = batches(&f).iter().sum();
    assert_eq!(total, 400, "every read in exactly one batch");
}

/// A record that is answered survives the Store being closed and opened
/// again: the batch's commit made it durable before the answer.
#[test]
fn an_answered_batch_is_in_the_log_after_the_store_is_opened_again() {
    let f = Arc::new(fixture());
    let (uuid, _) = stored(&f, A, "first");
    let outcomes = together(&f, &[uuid; 5], |_| {});
    assert!(outcomes.iter().all(Result::is_ok), "all answered");
    let again = f.open().expect("opens again");
    let kept = again
        .records()
        .iter()
        .filter(|r| r["action"] == "read" && r["status"] == 200)
        .count();
    assert_eq!(kept, 5, "the five answered reads are in the log");
}
