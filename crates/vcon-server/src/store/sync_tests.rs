//! No answer, and nothing done after a read, before every commit it
//! depends on is synced (backend/local/durable.rs): the Store's side of the
//! tenant catalog's group sync, driven through the crash filesystem's
//! held and failing syncs.

use std::time::{Duration, Instant};

use super::testing::vcon;
use super::tests::{Fixture, fixture, wal};
use super::{Created, StoreError};
use crate::fs::{Fs, Op};
use crate::retention::Retention;

const A: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bca1";
const B: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bcb2";

fn stored(f: &Fixture, uuid: &str) -> uuid::Uuid {
    let (found, body) = vcon(uuid, "first");
    f.store.create(&f.caller, &found, &body).expect("stored");
    found.uuid
}

/// Asserts for 300 ms that `thread` has not returned.
pub(crate) fn still_waiting<T>(thread: &std::thread::ScopedJoinHandle<'_, T>, what: &str) {
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        assert!(!thread.is_finished(), "{what}");
        std::thread::yield_now();
    }
}

#[test]
fn a_create_is_not_answered_while_a_sync_it_needs_is_held() {
    let f = fixture();
    let (found, body) = vcon(A, "first");
    f.fs.hold_syncs();
    std::thread::scope(|t| {
        let creating = t.spawn(|| f.store.create(&f.caller, &found, &body));
        f.fs.await_held(1);
        still_waiting(&creating, "no answer before the sync");
        f.fs.release_syncs();
        creating
            .join()
            .expect("the create ends")
            .expect("answered after the sync");
    });
}

#[test]
fn a_read_is_not_answered_while_the_sync_of_its_record_is_held() {
    let f = fixture();
    let uuid = stored(&f, A);
    f.fs.hold_syncs();
    std::thread::scope(|t| {
        let reading = t.spawn(|| f.store.get(&f.caller, uuid));
        f.fs.await_held(1);
        still_waiting(&reading, "the read's record is not durable yet");
        f.fs.release_syncs();
        assert!(
            reading.join().expect("the read ends").is_ok(),
            "answered once its record is durable"
        );
    });
}

#[test]
fn a_read_whose_record_cannot_be_synced_is_refused_and_so_is_the_next() {
    let f = fixture();
    let uuid = stored(&f, A);
    f.fs.fail_syncs();
    assert_eq!(
        f.store.get(&f.caller, uuid).map(|s| s.version),
        Err(StoreError::Io),
        "a read whose record may be lost is not answered"
    );
    f.fs.heal_syncs();
    assert_eq!(
        f.store.get(&f.caller, uuid).map(|s| s.version),
        Err(StoreError::Io),
        "the catalog is poisoned until it is reopened"
    );
}

#[test]
fn counts_that_saw_a_commit_whose_sync_failed_are_refused() {
    let f = fixture();
    stored(&f, A);
    let (found, body) = vcon(B, "second");
    f.fs.hold_syncs();
    std::thread::scope(|t| {
        let creating = t.spawn(|| f.store.create(&f.caller, &found, &body));
        f.fs.await_held(1);
        // The create's pending row is committed and not yet durable: the
        // counts read after it.
        let counting = t.spawn(|| f.store.counts());
        f.store.catalog.await_sync_waiters(2);
        f.fs.fail_syncs();
        assert!(creating.join().expect("ends").is_err(), "the create");
        assert_eq!(
            counting.join().expect("ends"),
            Err(StoreError::Io),
            "what the counts read may be lost: they are not answered"
        );
    });
}

/// A vCon deleted for good (kept 0 days), and the path of its object.
fn deleted(f: &Fixture) -> std::path::PathBuf {
    let (found, body) = vcon(A, "first");
    let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
        panic!("a new vCon")
    };
    f.store
        .delete(&f.caller, found.uuid, Retention::Years(7))
        .expect("deleted");
    f.store.objects_dir.join(version.to_string())
}

/// The object a purge removes stays until the purge's commit is durable:
/// while that commit's sync is held the object is there; after it the
/// object goes, its Remove logged after the commit's SyncData.
#[test]
fn a_purged_object_stays_until_the_purge_commit_is_synced() {
    let f = fixture();
    let object = deleted(&f);
    let from = f.fs.log().len();
    f.fs.hold_syncs();
    std::thread::scope(|t| {
        let purging = t.spawn(|| f.store.purge_due(Retention::Days(0)));
        f.fs.await_held(1);
        still_waiting(&purging, "the purge waits for its commit's sync");
        assert!(
            f.fs.read(&object).is_ok(),
            "the object is there while the purge's commit may be lost"
        );
        f.fs.release_syncs();
        let report = purging.join().expect("ends").expect("purged");
        assert_eq!(report.purged, 1, "one version purged");
    });
    let log = f.fs.log()[from..].to_vec();
    let synced = log
        .iter()
        .position(|op| *op == Op::SyncData(wal(&f)))
        .expect("the purge's commit synced");
    let removed = log
        .iter()
        .position(|op| *op == Op::Remove(object.clone()))
        .expect("the object removed");
    assert!(
        synced < removed,
        "the commit's sync before the removal: {log:?}"
    );
}

/// A purge whose commit cannot be synced removes nothing: the commit may
/// be lost, and with it the purge, so the object must stay.
#[test]
fn a_purge_whose_commit_cannot_be_synced_keeps_the_object() {
    let f = fixture();
    let object = deleted(&f);
    let from = f.fs.log().len();
    f.fs.fail_syncs();
    assert_eq!(
        f.store.purge_due(Retention::Days(0)).map(|r| r.purged),
        Err(StoreError::Io),
        "the purge's outcome is unknown"
    );
    assert!(
        !f.fs.log()[from..].contains(&Op::Remove(object.clone())),
        "no removal after a failed sync"
    );
    assert!(f.fs.read(&object).is_ok(), "the object is kept");
}

/// A store opened beside a server, as `audit export` and `backup` open
/// it, does not answer a read before a sync of the WAL that began after
/// the read: it may have read a server commit that is not durable yet.
#[test]
fn a_store_beside_a_server_answers_only_after_a_sync_of_the_wal() {
    let f = fixture();
    let uuid = stored(&f, A);
    let beside = f.open_beside_store();
    let (found, body) = vcon(B, "second");
    f.fs.hold_syncs();
    std::thread::scope(|t| {
        // The server's commit is written and its sync held.
        let creating = t.spawn(|| f.store.create(&f.caller, &found, &body));
        f.fs.await_held(1);
        let reading = t.spawn(|| beside.get(&f.caller, uuid));
        f.fs.await_held(2);
        still_waiting(&reading, "the beside read waits for its WAL sync");
        f.fs.release_syncs();
        assert!(creating.join().expect("ends").is_ok(), "the create");
        assert!(
            reading.join().expect("ends").is_ok(),
            "answered after the sync"
        );
    });
}

/// A store beside a server whose WAL sync fails refuses the read, and
/// every later one: what it read may be lost.
#[test]
fn a_store_beside_a_server_refuses_a_read_whose_wal_sync_failed() {
    let f = fixture();
    let uuid = stored(&f, A);
    let beside = f.open_beside_store();
    f.fs.fail_syncs();
    assert_eq!(
        beside.get(&f.caller, uuid).map(|s| s.version),
        Err(StoreError::Io),
        "not answered"
    );
    f.fs.heal_syncs();
    assert_eq!(
        beside.get(&f.caller, uuid).map(|s| s.version),
        Err(StoreError::Io),
        "poisoned until it is opened again"
    );
}

/// A store beside a server syncs the WAL again only when the server
/// committed since its last sync (SQLite's data_version): a read with no
/// server commit between costs no fdatasync.
#[test]
fn a_store_beside_a_server_syncs_again_only_after_a_server_commit() {
    let f = fixture();
    let uuid = stored(&f, A);
    let beside = f.open_beside_store();
    beside.get(&f.caller, uuid).expect("read");
    let after_first = f.fs.syncs(&wal(&f));
    beside.get(&f.caller, uuid).expect("read again");
    assert_eq!(
        f.fs.syncs(&wal(&f)),
        after_first,
        "no server commit between: no sync"
    );
    stored(&f, B);
    let after_server = f.fs.syncs(&wal(&f));
    beside
        .get(&f.caller, uuid)
        .expect("read after a server commit");
    assert_eq!(
        f.fs.syncs(&wal(&f)),
        after_server + 1,
        "the server committed: one sync"
    );
}

/// A Store closed while a create's commit waits for its sync answers only
/// after that sync, and answers its failure.
#[test]
fn a_store_closed_while_a_commit_waits_answers_that_commits_sync() {
    let f = fixture();
    stored(&f, A);
    let (found, body) = vcon(B, "second");
    f.fs.hold_syncs();
    std::thread::scope(|t| {
        let creating = t.spawn(|| f.store.create(&f.caller, &found, &body));
        f.fs.await_held(1);
        let closing = t.spawn(|| f.store.close());
        f.store.catalog.await_sync_waiters(2);
        still_waiting(&closing, "the close waits for the commit's sync");
        f.fs.fail_syncs();
        assert!(creating.join().expect("ends").is_err(), "the create");
        assert_eq!(
            closing.join().expect("ends"),
            Err(StoreError::Io),
            "a commit may be lost: the close says so"
        );
    });
}

/// A Store whose commits are all durable closes with Ok.
#[test]
fn a_store_whose_commits_are_durable_closes_ok() {
    let f = fixture();
    stored(&f, A);
    assert_eq!(f.store.close(), Ok(()), "every commit is durable");
}
