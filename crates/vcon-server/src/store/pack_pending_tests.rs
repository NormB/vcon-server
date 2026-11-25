//! Pack files under way: a pack run and a purge's rewrite record a
//! pending row for a new file before they put it, and recovery keeps a
//! file whose writing may still be under way (store/pending.rs), so with
//! several servers on one database one server's recovery never removes
//! another's file between its put and the transaction that records it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use time::Duration;

use super::Store;
use super::packed::tests::DirPacks;
use super::packer::PackPlan;
use super::packer::tests::{Kept, held_once, with_packs};
use super::pending::{WRITE_LEASE, Writers};
use super::testing::vcon;
use super::tests::{Fixture, fixture};
use crate::backend::{CommitError, Packs};
use crate::clock::Clock;
use crate::retention::Retention;
use crate::store::{Created, StoreError};

fn run() -> PackPlan {
    PackPlan::with_limit(1 << 20)
}

fn created(f: &Fixture, n: u8) -> Kept {
    let (found, body) = vcon(
        &format!("01a07da8-c2bb-83e5-b9a2-279e0d16bd{n:02x}"),
        &format!("under way {n}"),
    );
    let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
        panic!("a new vCon")
    };
    Kept {
        uuid: found.uuid,
        version,
        body,
    }
}

/// Three vCons and pack storage, nothing packed yet.
fn three(f: &mut Fixture) -> (Arc<DirPacks>, Vec<Kept>) {
    let kept = (0..3u8).map(|n| created(f, n)).collect();
    (with_packs(f), kept)
}

fn names(packs: &DirPacks) -> BTreeSet<String> {
    packs.list().expect("listed").into_iter().collect()
}

fn pending_files(f: &Fixture) -> usize {
    f.store
        .catalog
        .snapshot()
        .expect("a snapshot")
        .pending_files()
        .expect("reads")
        .len()
}

fn lease() -> Duration {
    Duration::nanoseconds(WRITE_LEASE)
}

/// Three vCons, all packed into one file, and that file's name.
fn one_file_to_rewrite(f: &mut Fixture) -> (Arc<DirPacks>, Vec<Kept>, String) {
    let (packs, kept) = three(f);
    f.store.pack(&run()).expect("packs");
    let before = names(&packs);
    assert_eq!(before.len(), 1, "one file");
    let file = before.into_iter().next().expect("one");
    (packs, kept, file)
}

// Success: a file under way is kept, and its run goes on.

/// A file a pack run put and has not yet recorded is a write under way:
/// recovery with other writers keeps it, and the run records it after.
#[test]
fn a_pack_file_under_way_is_kept_by_recovery_and_recorded_after() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = f
        .store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    let put = names(&packs);
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(names(&packs), put, "recovery keeps the file under way");
    assert_eq!(
        f.store.record_file(&written),
        Ok(true),
        "the run records its file"
    );
    assert_eq!(pending_files(&f), 0, "the record drops the pending row");
    f.store.release(&written).expect("releases the objects");
    held_once(&f, &packs, &kept, "recorded after recovery");
}

/// A pack run's file has a pending row from before its put until its
/// record, and none after.
#[test]
fn a_pack_file_has_a_pending_row_from_its_put_to_its_record() {
    let mut f = fixture();
    let (_packs, _kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = f
        .store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    let rows = f
        .store
        .catalog
        .snapshot()
        .expect("a snapshot")
        .pending_files()
        .expect("reads");
    assert_eq!(
        rows.len(),
        1,
        "one pending row while the file is unrecorded"
    );
    assert_eq!(
        rows[0].1,
        crate::clock::nanos(f.clock.now()),
        "with the time the writing began"
    );
    assert_eq!(f.store.record_file(&written), Ok(true), "recorded");
    assert_eq!(pending_files(&f), 0, "none after the record");
}

/// A whole pack run leaves no pending row behind.
#[test]
fn a_pack_run_leaves_no_pending_file_row() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    f.store.pack(&run()).expect("packs");
    assert_eq!(pending_files(&f), 0, "no pending row after the run");
    held_once(&f, &packs, &kept, "packed");
}

/// A purge's rewrite writes its new file under way too: recovery with
/// other writers keeps it, and the swap records it after.
#[test]
fn a_rewrite_under_way_keeps_its_new_file_and_swaps_after() {
    let mut f = fixture();
    let (packs, kept, old) = one_file_to_rewrite(&mut f);
    let from = crate::backend::pack::parse_name(&old).expect("a name").1;
    let copied = f
        .store
        .copy_files(&[from])
        .expect("copied")
        .expect("a copy");
    let put = names(&packs);
    assert_eq!(put.len(), 2, "the old file and its copy");
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(names(&packs), put, "recovery keeps the copy under way");
    assert_eq!(f.store.swap(&copied), Ok(true), "the swap records the copy");
    assert_eq!(pending_files(&f), 0, "the swap drops the pending row");
    f.store.finish_purges().expect("finishes");
    held_once(&f, &packs, &kept, "swapped after recovery");
}

/// A file under way whose writing began a little after the recovering
/// process's clock (another server's clock ahead) is kept, as a write's
/// object is.
#[test]
fn a_pack_file_begun_ahead_of_the_clock_within_the_lease_is_kept() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = f
        .store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    let put = names(&packs);
    f.clock
        .set(f.clock.now() - lease() + Duration::nanoseconds(1));
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(names(&packs), put, "a clock behind keeps the file");
    assert_eq!(f.store.record_file(&written), Ok(true), "recorded");
    f.store.release(&written).expect("releases the objects");
    held_once(&f, &packs, &kept, "recorded");
}

// Failure: a file no write can still be under way for is removed, and its
// record refused.

/// A file whose pending row is older than the lease is a leftover:
/// recovery removes it and its row, and the run's record is refused, so
/// no row names a removed file.
#[test]
fn a_pack_file_past_its_lease_is_removed_and_its_record_refused() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = f
        .store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    f.clock.set(f.clock.now() + lease());
    f.store.recover(Writers::Maybe).expect("recovers");
    assert!(names(&packs).is_empty(), "the leftover file is removed");
    assert_eq!(pending_files(&f), 0, "and its pending row");
    assert_eq!(
        f.store.record_file(&written),
        Ok(false),
        "the record is refused"
    );
    held_once(&f, &packs, &kept, "refused");
}

/// With no other writer, a file put and not recorded is a leftover at
/// once, as a write's object is.
#[test]
fn recovery_alone_takes_an_unrecorded_pack_file_for_a_leftover() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = f
        .store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    f.store.recover(Writers::None).expect("recovers");
    assert!(names(&packs).is_empty(), "the file is removed");
    assert_eq!(pending_files(&f), 0, "and its pending row");
    assert_eq!(
        f.store.record_file(&written),
        Ok(false),
        "the record is refused"
    );
    held_once(&f, &packs, &kept, "refused");
}

/// A rewrite's copy whose pending row is older than the lease is removed,
/// and the swap refused: the old file stays named and every kept version
/// reads back.
#[test]
fn a_rewrite_past_its_lease_is_removed_and_its_swap_refused() {
    let mut f = fixture();
    let (packs, kept, old) = one_file_to_rewrite(&mut f);
    let from = crate::backend::pack::parse_name(&old).expect("a name").1;
    let copied = f
        .store
        .copy_files(&[from])
        .expect("copied")
        .expect("a copy");
    f.clock.set(f.clock.now() + lease());
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(
        names(&packs),
        BTreeSet::from([old]),
        "only the old file is left"
    );
    assert_eq!(f.store.swap(&copied), Ok(false), "the swap is refused");
    assert_eq!(pending_files(&f), 0, "no pending row is left");
    held_once(&f, &packs, &kept, "refused");
}

/// A record refused because a version was purged during the run drops the
/// file's pending row along with the file.
#[test]
fn a_refused_record_leaves_no_pending_file_row() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = f
        .store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    f.store
        .delete(&f.caller, kept[1].uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(f.store.record_file(&written), Ok(false), "refused");
    assert_eq!(pending_files(&f), 0, "no pending row is left");
    held_once(&f, &packs, &[kept[0].clone(), kept[2].clone()], "refused");
}

/// A record whose commit is rolled back removes the file and leaves no
/// pending row.
#[test]
fn a_rolled_back_record_leaves_no_file_and_no_pending_row() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = f
        .store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    f.store.catalog.fail_commit(1, CommitError::RolledBack);
    assert_eq!(f.store.record_file(&written), Err(StoreError::Io), "fails");
    assert!(names(&packs).is_empty(), "the file is removed");
    assert_eq!(pending_files(&f), 0, "no pending row is left");
    held_once(&f, &packs, &kept, "rolled back");
}

/// A file whose pending row cannot be added is never put.
#[test]
fn a_pack_file_whose_pending_row_is_not_kept_is_not_put() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    f.store.catalog.fail_commit(1, CommitError::RolledBack);
    assert_eq!(
        f.store.write_file(&planned[0], &run()),
        Err(StoreError::Io),
        "the write fails"
    );
    assert!(names(&packs).is_empty(), "nothing is put");
    assert_eq!(pending_files(&f), 0, "no pending row");
    held_once(&f, &packs, &kept, "not put");
}

// Edge: the order inside recovery, the lease's boundary, and rows with no
// file.

/// Recovery lists the pack files before it reads the rows: a file whose
/// writing begins after recovery's read, and is put before recovery would
/// list the files, is one the read cannot know, and it is kept.
#[test]
fn a_pack_file_begun_after_recoverys_read_is_kept() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = Arc::new(Mutex::new(None));
    let slot = written.clone();
    *f.store.hooks.after_snapshot.lock().expect("hook") = Some(Box::new(move |store: &Store| {
        let file = store
            .write_file(&planned[0], &run())
            .expect("written")
            .expect("a file");
        *slot.lock().expect("slot") = Some(file);
    }));
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(
        names(&packs).len(),
        1,
        "the file put during recovery is kept"
    );
    let written = written.lock().expect("slot").take().expect("the hook ran");
    assert_eq!(f.store.record_file(&written), Ok(true), "and recorded");
    f.store.release(&written).expect("releases the objects");
    held_once(&f, &packs, &kept, "recorded");
}

/// A pending row exactly the lease old is no longer under way, as a
/// write's pending row is not.
#[test]
fn a_pack_file_exactly_the_lease_old_is_a_leftover() {
    let mut f = fixture();
    let (packs, _kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    f.store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    f.clock
        .set(f.clock.now() + lease() - Duration::nanoseconds(1));
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(names(&packs).len(), 1, "one nanosecond short: kept");
    f.clock.set(f.clock.now() + Duration::nanoseconds(1));
    f.store.recover(Writers::Maybe).expect("recovers");
    assert!(names(&packs).is_empty(), "at the lease: removed");
}

/// A pending row whose file was never put (a put that failed, a process
/// cut off before it) is dropped by recovery once no write can be under
/// way for it, and kept until then.
#[test]
fn a_pending_file_row_with_no_file_is_dropped_after_the_lease() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    packs
        .fail_puts
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(f.store.pack(&run()), Err(StoreError::Io), "the put fails");
    packs
        .fail_puts
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(pending_files(&f), 1, "the row of the file never put");
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(pending_files(&f), 1, "kept within the lease");
    f.clock.set(f.clock.now() + lease());
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(pending_files(&f), 0, "dropped after it");
    held_once(&f, &packs, &kept, "nothing packed");
}

/// A record whose commit outcome is unknown, and was in fact kept, leaves
/// the file recorded and no pending row: recovery after the lease keeps
/// the file, which a row names.
#[test]
fn an_unknown_record_that_was_kept_keeps_its_file_past_the_lease() {
    let mut f = fixture();
    let (packs, kept) = three(&mut f);
    let planned = f.store.plan_files(&run()).expect("planned");
    let written = f
        .store
        .write_file(&planned[0], &run())
        .expect("written")
        .expect("a file");
    f.store.catalog.fail_commit(1, CommitError::Unknown);
    assert_eq!(
        f.store.record_file(&written),
        Err(StoreError::Io),
        "unknown"
    );
    assert_eq!(pending_files(&f), 0, "the kept commit dropped the row");
    f.clock.set(f.clock.now() + lease() + lease());
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(names(&packs).len(), 1, "the recorded file stays");
    held_once(&f, &packs, &kept, "recorded, its objects removed");
}
