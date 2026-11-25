//! The reader grace of manifests on the paths that finish purges: start-up
//! recovery, a sweep's recovery, every delete and every sweep. With no
//! purge listed, `finish_purges` removes no manifest: an older one a
//! reader may be using stays until a pack run finds it past the configured
//! reader grace. Only after a purge rewrote or deleted a file are the older
//! manifests removed at once, since they name the deleted file.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use time::Duration as Span;

use super::packed::tests::DirPacks;
use super::packer::PackPlan;
use super::packer::tests::{Kept, with_packs};
use super::pending::Writers;
use super::testing::vcon;
use super::tests::{Fixture, fixture};
use super::{Created, StoreError};
use crate::backend::{CommitError, Packs};
use crate::clock::Clock;
use crate::retention::Retention;

/// The default reader grace, which `plan` uses.
const GRACE: i64 = 900;

/// Packing without merging, so each run that packs writes one manifest,
/// with the default reader grace.
fn plan() -> PackPlan {
    PackPlan {
        merge_below: 0,
        ..PackPlan::with_limit(1 << 20)
    }
}

fn plan_with(grace: std::time::Duration) -> PackPlan {
    PackPlan { grace, ..plan() }
}

fn created(f: &Fixture, n: u8) -> Kept {
    let (found, body) = vcon(
        &format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}"),
        &format!("subject {n}"),
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

/// The numbers of the manifests the storage holds, in order.
fn seqs(packs: &DirPacks) -> Vec<i64> {
    let mut seqs: Vec<i64> = std::fs::read_dir(packs.path("manifest"))
        .map(|d| {
            d.map(|e| {
                let name = e.expect("an entry").file_name();
                name.to_string_lossy()
                    .trim_end_matches(".json")
                    .parse()
                    .expect("a number")
            })
            .collect()
        })
        .unwrap_or_default();
    seqs.sort_unstable();
    seqs
}

/// The catalog's manifest rows, by number.
fn rows(f: &Fixture) -> Vec<i64> {
    let conn = f.store.conn_for_test();
    let mut stmt = conn
        .prepare("SELECT seq FROM manifest ORDER BY seq")
        .expect("prepare");
    stmt.query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows")
}

/// The pack files the storage holds.
fn pack_files(packs: &DirPacks) -> Vec<String> {
    packs.list().expect("listed")
}

/// `n` vCons, each packed by a run of its own: `n` manifests, all written
/// at the fixture's start time, so every one is inside the reader grace.
fn packed_apart(f: &mut Fixture, n: u8) -> (Arc<DirPacks>, Vec<Kept>) {
    let first = created(f, 1);
    let packs = with_packs(f);
    f.store.pack(&plan()).expect("packs");
    let mut kept = vec![first];
    for i in 2..=n {
        kept.push(created(f, i));
        f.store.pack(&plan()).expect("packs");
    }
    let want: Vec<i64> = (1..=i64::from(n)).collect();
    assert_eq!(seqs(&packs), want, "one manifest per run");
    (packs, kept)
}

/// Two manifests, inside the grace, and a third vCon packed with the
/// first so a purge of the first rewrites a file that keeps a row.
fn two_and_a_shared_file(f: &mut Fixture) -> (Arc<DirPacks>, Vec<Kept>) {
    let a = created(f, 1);
    let c = created(f, 3);
    let packs = with_packs(f);
    f.store.pack(&plan()).expect("packs");
    let b = created(f, 2);
    f.store.pack(&plan()).expect("packs");
    assert_eq!(seqs(&packs), [1, 2], "two manifests");
    (packs, vec![a, b, c])
}

/// The manifests the storage holds name no file the storage lacks, and
/// none of them names `file`.
fn no_manifest_names(packs: &DirPacks, file: &str, what: &str) {
    for seq in seqs(packs) {
        let text = std::fs::read_to_string(packs.path(&super::manifest::name(seq))).expect("read");
        assert!(!text.contains(file), "{what}: manifest {seq} names {file}");
    }
}

fn later_by(f: &Fixture, seconds: i64) {
    f.clock.set(f.clock.now() + Span::seconds(seconds));
}

// Success: the grace holds where no purge rewrote a file, and a purge's
// rewrite still removes the older manifests at once.

#[test]
fn finish_purges_with_no_purge_listed_keeps_the_older_manifest() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    f.store.finish_purges().expect("finishes");
    assert_eq!(
        seqs(&packs),
        [1, 2],
        "manifest 1 stays for the reader grace"
    );
    assert_eq!(rows(&f), [1, 2], "and its catalog row");
}

#[test]
fn start_up_recovery_keeps_the_older_manifest() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    f.reopen();
    assert_eq!(seqs(&packs), [1, 2], "a restart removes no manifest");
}

#[test]
fn a_sweeps_recovery_with_other_writers_keeps_the_older_manifest() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(seqs(&packs), [1, 2], "recovery beside writers keeps it");
}

#[test]
fn a_second_store_opened_beside_keeps_the_older_manifest() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    let mut beside = f.open_beside_store();
    beside.packs = Some(packs.clone());
    beside.recover(Writers::Maybe).expect("recovers");
    assert_eq!(seqs(&packs), [1, 2], "another instance's open keeps it");
}

#[test]
fn a_sweep_with_nothing_due_keeps_the_older_manifest() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    let report = f.store.purge_due(Retention::Days(1)).expect("swept");
    assert_eq!(report.purged, 0, "nothing due");
    assert_eq!(seqs(&packs), [1, 2], "a sweep keeps it");
}

#[test]
fn a_delete_not_yet_due_keeps_the_older_manifest() {
    let mut f = fixture();
    let (packs, kept) = packed_apart(&mut f, 2);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Years(7))
        .expect("deleted");
    assert_eq!(seqs(&packs), [1, 2], "a delete keeps it");
}

#[test]
fn a_restore_after_a_delete_keeps_the_older_manifest() {
    let mut f = fixture();
    let (packs, kept) = packed_apart(&mut f, 2);
    f.store
        .delete(&f.caller, kept[1].uuid, Retention::Years(7))
        .expect("deleted");
    f.store.restore(&f.caller, kept[1].uuid).expect("restored");
    f.store.purge_due(Retention::Years(7)).expect("swept");
    assert_eq!(seqs(&packs), [1, 2], "delete, restore and sweep keep it");
}

#[test]
fn a_purge_that_rewrote_a_file_removes_every_older_manifest_at_once() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    let old = pack_files(&packs);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(seqs(&packs), [3], "only the purge's own manifest");
    assert_eq!(rows(&f), [3], "and only its row");
    let gone: Vec<&String> = old
        .iter()
        .filter(|n| !pack_files(&packs).contains(n))
        .collect();
    assert_eq!(gone.len(), 1, "the rewritten file is deleted: {gone:?}");
    no_manifest_names(&packs, gone[0], "after the purge");
}

#[test]
fn a_purge_that_deleted_a_whole_file_removes_every_older_manifest_at_once() {
    let mut f = fixture();
    let (packs, kept) = packed_apart(&mut f, 2);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(seqs(&packs), [3], "only the purge's own manifest");
    assert_eq!(pack_files(&packs).len(), 1, "the purged row's file is gone");
}

#[test]
fn a_sweep_that_purged_a_packed_row_removes_every_older_manifest_at_once() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Years(7))
        .expect("deleted");
    assert_eq!(seqs(&packs), [1, 2], "the delete keeps both");
    later_by(&f, 2 * 86_400);
    let report = f.store.purge_due(Retention::Days(1)).expect("swept");
    assert_eq!(report.purged, 1, "one due");
    assert_eq!(
        seqs(&packs),
        [3],
        "the sweep's rewrite removes the older ones"
    );
}

#[test]
fn a_purge_whose_removal_was_cut_off_is_pruned_at_once_by_recovery() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    packs.fail_removes.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.delete(&f.caller, kept[0].uuid, Retention::Days(0)),
        Err(StoreError::Io),
        "the removal fails"
    );
    assert_eq!(
        seqs(&packs),
        [1, 2],
        "nothing pruned before the file is gone"
    );
    packs.fail_removes.store(false, Ordering::SeqCst);
    f.reopen();
    assert_eq!(seqs(&packs), [3], "recovery finishes the purge and prunes");
}

#[test]
fn a_purge_whose_copy_was_cut_off_is_pruned_at_once_by_the_next_sweep() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    packs.fail_puts.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.delete(&f.caller, kept[0].uuid, Retention::Days(0)),
        Err(StoreError::Io),
        "the copy fails"
    );
    assert_eq!(seqs(&packs), [1, 2], "nothing pruned");
    packs.fail_puts.store(false, Ordering::SeqCst);
    f.store.purge_due(Retention::Days(1)).expect("swept");
    assert_eq!(seqs(&packs), [3], "the next sweep finishes and prunes");
}

#[test]
fn a_manifest_kept_by_recovery_goes_with_the_pack_run_after_the_grace() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    f.reopen();
    later_by(&f, GRACE);
    f.store.pack(&plan()).expect("packs");
    assert_eq!(seqs(&packs), [2], "the pack run removes it at the grace");
}

// Failure: a storage or catalog that fails is reported, and no manifest a
// reader may need is removed because of it.

#[test]
fn finish_purges_with_no_purge_listed_needs_no_removal() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    packs.fail_removes.store(true, Ordering::SeqCst);
    assert_eq!(f.store.finish_purges(), Ok(()), "nothing to remove");
    assert_eq!(seqs(&packs), [1, 2], "both stay");
}

#[test]
fn recovery_with_failing_removals_and_no_purge_succeeds() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    packs.fail_removes.store(true, Ordering::SeqCst);
    assert_eq!(f.store.recover(Writers::None), Ok(()), "recovers");
    assert_eq!(seqs(&packs), [1, 2], "both stay");
}

#[test]
fn a_delete_with_failing_removals_and_nothing_due_succeeds() {
    let mut f = fixture();
    let (packs, kept) = packed_apart(&mut f, 2);
    packs.fail_removes.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.delete(&f.caller, kept[0].uuid, Retention::Years(7)),
        Ok(()),
        "a delete not due removes nothing"
    );
    assert_eq!(seqs(&packs), [1, 2], "both stay");
}

#[test]
fn a_sweep_with_failing_removals_and_nothing_due_succeeds() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    packs.fail_removes.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.purge_due(Retention::Days(1)).map(|r| r.purged),
        Ok(0),
        "a sweep with nothing due removes nothing"
    );
    assert_eq!(seqs(&packs), [1, 2], "both stay");
}

#[test]
fn a_purge_whose_removal_fails_keeps_the_older_manifests() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    packs.fail_removes.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.delete(&f.caller, kept[0].uuid, Retention::Days(0)),
        Err(StoreError::Io),
        "the purge fails"
    );
    assert_eq!(seqs(&packs), [1, 2], "the old file is there: both stay");
}

#[test]
fn a_purge_whose_storage_keeps_the_file_keeps_the_older_manifests() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    packs.keep_removed.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.delete(&f.caller, kept[0].uuid, Retention::Days(0)),
        Err(StoreError::Io),
        "the purge fails"
    );
    assert_eq!(seqs(&packs), [1, 2], "both stay");
}

#[test]
fn a_purge_whose_copy_fails_keeps_the_older_manifests() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    packs.fail_puts.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.delete(&f.caller, kept[0].uuid, Retention::Days(0)),
        Err(StoreError::Io),
        "the purge fails"
    );
    assert_eq!(seqs(&packs), [1, 2], "both stay");
}

#[test]
fn a_purge_whose_swap_rolls_back_keeps_the_older_manifests() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Years(7))
        .expect("deleted");
    later_by(&f, 2 * 86_400);
    // The purge's own commit is the first; the swap's the second.
    f.store.catalog.fail_commit(2, CommitError::RolledBack);
    assert_eq!(
        f.store.purge_due(Retention::Days(1)),
        Err(StoreError::Io),
        "the swap fails"
    );
    assert_eq!(seqs(&packs), [1, 2], "both stay");
    f.reopen();
    assert_eq!(seqs(&packs), [3], "recovery finishes the purge and prunes");
}

#[test]
fn a_purge_whose_swap_outcome_is_unknown_keeps_the_older_manifests() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Years(7))
        .expect("deleted");
    later_by(&f, 2 * 86_400);
    f.store.catalog.fail_commit(2, CommitError::Unknown);
    assert_eq!(
        f.store.purge_due(Retention::Days(1)),
        Err(StoreError::Io),
        "the swap's outcome is unknown"
    );
    assert_eq!(seqs(&packs), [1, 2], "both stay");
    f.reopen();
    assert_eq!(seqs(&packs), [3], "recovery finishes the purge and prunes");
}

#[test]
fn a_purge_whose_own_commit_fails_keeps_the_manifests_and_the_row() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Years(7))
        .expect("deleted");
    later_by(&f, 2 * 86_400);
    f.store.catalog.fail_commit(1, CommitError::RolledBack);
    assert_eq!(
        f.store.purge_due(Retention::Days(1)),
        Err(StoreError::Io),
        "the purge's commit fails"
    );
    assert_eq!(seqs(&packs), [1, 2], "nothing purged: both stay");
}

#[test]
fn a_manifest_write_that_fails_with_no_purge_listed_removes_nothing() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    created(&f, 3);
    let plan = plan();
    for planned in f.store.plan_files(&plan).expect("planned") {
        let written = f
            .store
            .write_file(&planned, &plan)
            .expect("written")
            .expect("a file");
        assert_eq!(f.store.record_file(&written), Ok(true), "recorded");
    }
    packs.fail_manifests.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.finish_purges(),
        Err(StoreError::Io),
        "the manifest write fails"
    );
    assert_eq!(seqs(&packs), [1, 2], "both stay");
}

#[test]
fn a_recovery_that_cannot_finish_a_purge_keeps_the_older_manifests() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    packs.fail_removes.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.delete(&f.caller, kept[0].uuid, Retention::Days(0)),
        Err(StoreError::Io),
        "the purge fails"
    );
    assert_eq!(
        f.store.recover(Writers::Maybe),
        Err(StoreError::Io),
        "recovery cannot remove the file either"
    );
    assert_eq!(seqs(&packs), [1, 2], "both stay");
}

// Edge: empty stores, no pack storage, several manifests and the grace's
// boundaries.

#[test]
fn an_empty_store_with_pack_storage_finishes_with_no_manifest() {
    let mut f = fixture();
    let packs = with_packs(&mut f);
    f.store.finish_purges().expect("finishes");
    f.store.recover(Writers::Maybe).expect("recovers");
    f.store.purge_due(Retention::Days(0)).expect("swept");
    assert_eq!(seqs(&packs), Vec::<i64>::new(), "no manifest");
    assert_eq!(rows(&f), Vec::<i64>::new(), "no row");
}

#[test]
fn a_store_with_no_pack_storage_finishes_and_recovers() {
    let mut f = fixture();
    let a = created(&f, 1);
    assert_eq!(f.store.finish_purges(), Ok(()), "nothing to finish");
    assert_eq!(f.store.recover(Writers::Maybe), Ok(()), "recovers");
    f.store
        .delete(&f.caller, a.uuid, Retention::Days(0))
        .expect("deleted and purged");
    f.reopen();
    assert_eq!(rows(&f), Vec::<i64>::new(), "no manifest is numbered");
}

#[test]
fn a_single_manifest_stays_with_no_purge() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 1);
    f.store.finish_purges().expect("finishes");
    f.reopen();
    assert_eq!(seqs(&packs), [1], "the newest stays");
}

#[test]
fn several_manifests_all_stay_with_no_purge() {
    let mut f = fixture();
    let (packs, kept) = packed_apart(&mut f, 5);
    f.reopen();
    f.store.recover(Writers::Maybe).expect("recovers");
    f.store
        .delete(&f.caller, kept[2].uuid, Retention::Years(7))
        .expect("deleted");
    f.store.purge_due(Retention::Days(1)).expect("swept");
    assert_eq!(seqs(&packs), [1, 2, 3, 4, 5], "all five stay");
}

#[test]
fn several_manifests_all_go_but_the_newest_after_a_purge() {
    let mut f = fixture();
    let (packs, kept) = packed_apart(&mut f, 5);
    f.store
        .delete(&f.caller, kept[2].uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(seqs(&packs), [6], "only the purge's own");
}

#[test]
fn just_inside_the_grace_recovery_and_the_pack_run_keep_it() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    later_by(&f, GRACE - 1);
    f.reopen();
    f.store.pack(&plan()).expect("packs");
    assert_eq!(seqs(&packs), [1, 2], "a second before the grace ends");
}

#[test]
fn just_outside_the_grace_finish_purges_leaves_it_to_the_pack_run() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    later_by(&f, GRACE);
    f.store.finish_purges().expect("finishes");
    assert_eq!(seqs(&packs), [1, 2], "finish_purges prunes nothing itself");
    f.store.pack(&plan()).expect("packs");
    assert_eq!(seqs(&packs), [2], "the pack run removes it at the grace");
}

#[test]
fn a_longer_configured_grace_is_kept_past_the_default() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    let long = plan_with(std::time::Duration::from_secs(1800));
    later_by(&f, GRACE);
    f.reopen();
    f.store.purge_due(Retention::Days(1)).expect("swept");
    f.store.pack(&long).expect("packs");
    assert_eq!(
        seqs(&packs),
        [1, 2],
        "past 900 s, inside the configured 1800 s"
    );
    later_by(&f, 1800 - GRACE - 1);
    f.store.pack(&long).expect("packs");
    assert_eq!(seqs(&packs), [1, 2], "a second before 1800 s");
    later_by(&f, 1);
    f.store.pack(&long).expect("packs");
    assert_eq!(seqs(&packs), [2], "at 1800 s it goes");
}

#[test]
fn a_purge_inside_the_grace_still_prunes_at_once() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    later_by(&f, 1);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(
        seqs(&packs),
        [3],
        "a second after manifest 2, pruned anyway"
    );
}

#[test]
fn a_purge_of_a_version_in_no_file_keeps_the_older_manifest() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 2);
    let loose = created(&f, 9);
    f.store
        .delete(&f.caller, loose.uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(seqs(&packs), [1, 2], "no file rewritten: both stay");
}

#[test]
fn manifests_written_after_a_purge_wait_for_the_grace_again() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(seqs(&packs), [3], "pruned by the purge");
    created(&f, 4);
    f.store.pack(&plan()).expect("packs");
    f.store.purge_due(Retention::Days(1)).expect("swept");
    f.reopen();
    assert_eq!(seqs(&packs), [3, 4], "the next pair keeps its grace");
}

#[test]
fn an_unwritten_manifest_is_written_and_the_older_one_kept() {
    let mut f = fixture();
    let (packs, _) = packed_apart(&mut f, 1);
    created(&f, 2);
    let plan = plan();
    for planned in f.store.plan_files(&plan).expect("planned") {
        let written = f
            .store
            .write_file(&planned, &plan)
            .expect("written")
            .expect("a file");
        assert_eq!(f.store.record_file(&written), Ok(true), "recorded");
    }
    assert_eq!(seqs(&packs), [1], "manifest 2 numbered, not written");
    f.store.finish_purges().expect("finishes");
    assert_eq!(seqs(&packs), [1, 2], "2 written, 1 kept");
}

#[test]
fn a_purge_cut_off_after_its_rewrite_is_pruned_inside_the_grace() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    packs.keep_removed.store(true, Ordering::SeqCst);
    assert_eq!(
        f.store.delete(&f.caller, kept[0].uuid, Retention::Days(0)),
        Err(StoreError::Io),
        "the storage keeps the file"
    );
    packs.keep_removed.store(false, Ordering::SeqCst);
    later_by(&f, GRACE - 1);
    f.store.recover(Writers::Maybe).expect("recovers");
    assert_eq!(seqs(&packs), [3], "inside the grace, pruned at once");
}

#[test]
fn a_second_finish_after_a_purge_prunes_nothing_more() {
    let mut f = fixture();
    let (packs, kept) = two_and_a_shared_file(&mut f);
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Days(0))
        .expect("deleted and purged");
    created(&f, 4);
    f.store.pack(&plan()).expect("packs");
    assert_eq!(seqs(&packs), [3, 4], "two again");
    f.store.finish_purges().expect("finishes");
    assert_eq!(seqs(&packs), [3, 4], "nothing listed: both stay");
}
