//! A tenant's reads racing the same tenant's writes on other threads, on
//! the local catalog's one connection with its group sync
//! (backend/local/durable.rs): many commits and reads wait for syncs at
//! once, one thread syncs for all of them. With no step held, each answer
//! is one whole version or the refusal of the state after a write, a
//! version purged before a read began is never its answer, and every read
//! is recorded once. Carried over from the read-connection work's
//! read_tests.rs (2026-11-24), which held the same rules there; its tests
//! of a read connection, and of a write committing inside a read's first
//! step, do not apply to one connection.

use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, Instant};

use serde_json::Value;
use uuid::Uuid;

use super::testing::vcon;
use super::tests::{Fixture, fixture};
use super::{Created, StoreError};
use crate::ids::VersionId;
use crate::retention::Retention;

const A: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bca1";
const B: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bcb2";

/// How long a race may take, per reader, before the test fails instead of
/// hanging.
const RACE_DEADLINE: Duration = Duration::from_secs(120);

/// How many reads each racing reader makes at least.
const RACE_READS: usize = 40;

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).expect("a uuid")
}

fn stored(f: &Fixture, uuid: &str, subject: &str) -> (VersionId, Vec<u8>) {
    let (found, body) = vcon(uuid, subject);
    let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
        panic!("a new vCon")
    };
    (version, body)
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

fn rec(target: &str, status: u16) -> (Value, Value) {
    (target.into(), status.into())
}

fn subject(n: usize) -> String {
    format!("v{n}")
}

/// Which version of A a body is, if it is one of the bodies a racing
/// writer stored, byte for byte.
fn which(body: &[u8], stored: usize) -> Option<usize> {
    (0..=stored).find(|&n| vcon(A, &subject(n)).1 == body)
}

/// Runs `readers` threads that read A until `done` is set and at least
/// RACE_READS times each, each read checked by `check` with the value of
/// `gate` taken before the read began; gives back their handles.
fn race_reads(
    f: &Arc<Fixture>,
    readers: usize,
    done: &Arc<AtomicBool>,
    gate: &Arc<AtomicUsize>,
    check: impl Fn(usize, Result<Vec<u8>, StoreError>) + Send + Sync + 'static,
) -> Vec<std::thread::JoinHandle<usize>> {
    let check = Arc::new(check);
    (0..readers)
        .map(|_| {
            let (f, done, gate, check) = (f.clone(), done.clone(), gate.clone(), check.clone());
            std::thread::spawn(move || {
                let deadline = Instant::now() + RACE_DEADLINE;
                let mut made = 0;
                while made < RACE_READS || !done.load(SeqCst) {
                    assert!(
                        Instant::now() < deadline,
                        "the race ended within its deadline"
                    );
                    let before = gate.load(SeqCst);
                    check(before, f.store.get(&f.caller, uuid(A)).map(|s| s.body));
                    made += 1;
                }
                made
            })
        })
        .collect()
}

fn joined(readers: Vec<std::thread::JoinHandle<usize>>) -> usize {
    readers
        .into_iter()
        .map(|r| r.join().expect("a reader ends without a failed check"))
        .sum()
}

/// Reads race a writer that replaces A again and again and purges each
/// replaced version: every read answers a whole version, never one purged
/// before the read began, and every read is recorded once.
#[test]
fn reads_racing_replaces_and_purges_answer_whole_unpurged_versions() {
    const ROUNDS: usize = 12;
    let f = Arc::new(fixture());
    let (mut current, _) = stored(&f, A, &subject(0));
    let done = Arc::new(AtomicBool::new(false));
    // Every version before this one is purged.
    let purged = Arc::new(AtomicUsize::new(0));
    let readers = race_reads(&f, 4, &done, &purged, |before, answered| {
        let body = answered.unwrap_or_else(|e| panic!("a read is answered, not {e:?}"));
        let n = which(&body, ROUNDS).expect("the body is one whole version");
        assert!(
            n >= before,
            "version {n} was purged before the read began (all below {before} were)"
        );
    });
    for n in 1..=ROUNDS {
        let (found, body) = vcon(A, &subject(n));
        current = f
            .store
            .replace(&f.caller, current, &found, &body)
            .expect("replaced");
        let report = f.store.purge_due(Retention::Days(0)).expect("purges");
        assert_eq!(report.purged, 1, "the replaced version is purged");
        purged.store(n, SeqCst);
    }
    done.store(true, SeqCst);
    let made = joined(readers);
    let recorded = reads(&f);
    assert_eq!(recorded.len(), made, "each read is recorded once");
    assert!(
        recorded.iter().all(|r| *r == rec(A, 200)),
        "every read is recorded as answered"
    );
}

/// Reads race a delete that purges A at once: a read answers the whole
/// version or is refused as gone, and every read that began after the
/// purge committed is refused.
#[test]
fn reads_racing_a_delete_and_purge_answer_the_version_or_gone() {
    let f = Arc::new(fixture());
    let (_, body) = stored(&f, A, &subject(0));
    let done = Arc::new(AtomicBool::new(false));
    // 1 once the delete and its purge have committed.
    let gone = Arc::new(AtomicUsize::new(0));
    let answered_reads = Arc::new(AtomicUsize::new(0));
    let counted = answered_reads.clone();
    let readers = race_reads(
        &f,
        4,
        &done,
        &gone,
        move |before, answered| match answered {
            Ok(b) => {
                assert_eq!(before, 0, "no read that began after the purge is answered");
                assert_eq!(b, body, "the whole version");
                counted.fetch_add(1, SeqCst);
            }
            Err(e) => assert_eq!(e, StoreError::Gone, "refused as gone, not {e:?}"),
        },
    );
    let deadline = Instant::now() + RACE_DEADLINE;
    while answered_reads.load(SeqCst) < 8 {
        assert!(
            Instant::now() < deadline,
            "reads were answered before the delete"
        );
        std::thread::yield_now();
    }
    f.store
        .delete(&f.caller, uuid(A), Retention::Days(0))
        .expect("deleted and purged");
    gone.store(1, SeqCst);
    done.store(true, SeqCst);
    let made = joined(readers);
    let recorded = reads(&f);
    assert_eq!(recorded.len(), made, "each read is recorded once");
    let answered = answered_reads.load(SeqCst);
    assert_eq!(
        recorded.iter().filter(|r| **r == rec(A, 200)).count(),
        answered,
        "each answered read is recorded as answered"
    );
    assert_eq!(
        recorded.iter().filter(|r| **r == rec(A, 410)).count(),
        made - answered,
        "each refused read is recorded as gone"
    );
    let first_gone = recorded
        .iter()
        .position(|r| *r == rec(A, 410))
        .expect("some read is refused");
    assert!(
        recorded[first_gone..].iter().all(|r| *r == rec(A, 410)),
        "after the first refusal in the log, no read is recorded as answered"
    );
}

/// Eight readers of two vCons race two writers that replace a vCon each of
/// their own: every read is answered with its vCon's one stored body and
/// recorded once, and every write is kept.
#[test]
fn readers_racing_writers_of_the_same_tenant_are_all_answered() {
    const WRITES: usize = 15;
    const OWN: [&str; 2] = [
        "01a07da8-c2bb-83e5-b9a2-279e0d16bc01",
        "01a07da8-c2bb-83e5-b9a2-279e0d16bc02",
    ];
    let f = Arc::new(fixture());
    let (_, a) = stored(&f, A, "a");
    let (_, b) = stored(&f, B, "b");
    let done = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..8)
        .map(|n| {
            let (f, done) = (f.clone(), done.clone());
            let (target, want) = if n % 2 == 0 {
                (A, a.clone())
            } else {
                (B, b.clone())
            };
            std::thread::spawn(move || {
                let deadline = Instant::now() + RACE_DEADLINE;
                let mut made = 0;
                while made < RACE_READS || !done.load(SeqCst) {
                    assert!(
                        Instant::now() < deadline,
                        "the race ended within its deadline"
                    );
                    let got = f.store.get(&f.caller, uuid(target)).map(|s| s.body);
                    assert_eq!(got, Ok(want.clone()), "the vCon's one body");
                    made += 1;
                }
                made
            })
        })
        .collect();
    let writers: Vec<_> = OWN
        .into_iter()
        .map(|own| {
            let f = f.clone();
            std::thread::spawn(move || {
                let (mut current, _) = stored(&f, own, "w0");
                for n in 1..=WRITES {
                    let (found, body) = vcon(own, &format!("w{n}"));
                    current = f
                        .store
                        .replace(&f.caller, current, &found, &body)
                        .expect("replaced");
                }
                current
            })
        })
        .collect();
    let lasts: Vec<_> = writers
        .into_iter()
        .map(|w| w.join().expect("a writer ends"))
        .collect();
    done.store(true, SeqCst);
    let made = joined(readers);
    assert_eq!(reads(&f).len(), made, "each read is recorded once");
    for (n, own) in OWN.into_iter().enumerate() {
        let got = f.store.get(&f.caller, uuid(own)).expect("reads");
        assert_eq!(
            got.version, lasts[n],
            "writer {n}'s last version is current"
        );
        assert_eq!(
            got.body,
            vcon(own, &format!("w{WRITES}")).1,
            "writer {n}'s last body"
        );
    }
}
