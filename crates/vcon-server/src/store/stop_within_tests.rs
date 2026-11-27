//! A stop inside one tenant's step: the purge sweep and the pack run ask
//! the server's stop test between the items of a tenant's step, and end
//! after the item under way, so a stop waits for one item's work and not
//! for the whole tenant's.
//!
//! The items: a purge erases one version in a transaction of its own and
//! then removes its object; it rewrites one pack file a purge listed. A
//! pack run reads one object into the file it writes, removes one packed
//! version's object after the file's commit, and merges one date's small
//! files. The first item of a step is not asked before: the server asks
//! before each tenant's step, so a step that has begun does its first
//! item. A stop between two items leaves nothing half done: a version not
//! yet purged is kept whole, a file not yet put has no pending row, an
//! object not yet removed after its file's commit is a leftover that
//! recovery removes, and a rewrite not yet made stays listed for recovery
//! and the next sweep.
//!
//! The stop tests here answer by count ([`Stop`]) or by what the store
//! holds, never by time.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use time::Duration;

use super::Created;
use super::packed::tests::DirPacks;
use super::packer::PackPlan;
use super::packer::tests::{Kept, held_once, packed_versions, with_packs};
use super::pending::Writers;
use super::purge::PurgeReport;
use super::testing::vcon;
use super::tests::{Fixture, fixture};
use crate::backend::Packs;
use crate::clock::Clock;
use crate::fs::Fs;
use crate::retention::Retention;

const DAY: Retention = Retention::Days(1);

/// A stop test that answers false to its first `go` asks and true to
/// every ask after, and counts every ask.
struct Stop {
    go: usize,
    asked: Cell<usize>,
}

impl Stop {
    fn after(go: usize) -> Stop {
        Stop {
            go,
            asked: Cell::new(0),
        }
    }

    fn never() -> Stop {
        Stop::after(usize::MAX)
    }

    fn ask(&self) -> bool {
        let n = self.asked.get();
        self.asked.set(n + 1);
        n >= self.go
    }

    fn asked(&self) -> usize {
        self.asked.get()
    }
}

fn uuid_text(n: u8) -> String {
    format!("01a07da8-c2bb-83e5-b9a2-279e0d16be{n:02x}")
}

fn later(f: &Fixture, by: Duration) {
    f.clock.set(f.clock.now() + by);
}

/// Stores vCon `n` and answers it as kept.
fn created(f: &Fixture, n: u8) -> Kept {
    let (found, body) = vcon(&uuid_text(n), &format!("stop within {n}"));
    let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
        panic!("a new vCon")
    };
    Kept {
        uuid: found.uuid,
        version,
        body,
    }
}

/// `n` vCons, each deleted and due for purge under DAY.
fn deleted(f: &Fixture, n: u8) -> Vec<Kept> {
    let kept: Vec<Kept> = (0..n).map(|i| created(f, i)).collect();
    for k in &kept {
        f.store
            .delete(&f.caller, k.uuid, Retention::Years(7))
            .expect("deletes");
    }
    later(f, Duration::days(2));
    kept
}

/// Which of `kept` are purged, by place.
fn purged(f: &Fixture, kept: &[Kept]) -> Vec<bool> {
    kept.iter()
        .map(|k| {
            let rows: i64 = f
                .store
                .conn()
                .query_row(
                    "SELECT count(*) FROM version WHERE version = ?1",
                    [k.version.0],
                    |r| r.get(0),
                )
                .expect("counts");
            rows == 0
        })
        .collect()
}

fn purged_count(f: &Fixture, kept: &[Kept]) -> usize {
    purged(f, kept).into_iter().filter(|p| *p).count()
}

fn object_exists(f: &Fixture, k: &Kept) -> bool {
    f.fs.read(&f.store.objects_dir.join(k.version.to_string()))
        .is_ok()
}

fn tombstoned(f: &Fixture, k: &Kept) -> bool {
    let rows: i64 = f
        .store
        .conn()
        .query_row(
            "SELECT count(*) FROM tombstone WHERE uuid = ?1",
            [k.uuid.as_bytes()],
            |r| r.get(0),
        )
        .expect("counts");
    rows == 1
}

/// The uuids the audit log records a purge of.
fn purge_records(f: &Fixture) -> BTreeSet<String> {
    f.store
        .records()
        .into_iter()
        .filter(|r| r["action"] == "purge")
        .map(|r| r["target"].as_str().expect("a target").to_owned())
        .collect()
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

fn purge_pending(f: &Fixture) -> usize {
    f.store
        .catalog
        .snapshot()
        .expect("a snapshot")
        .purge_pending()
        .expect("reads")
        .len()
}

fn objects(f: &Fixture) -> usize {
    f.store.objects.list().expect("listed").len()
}

fn files(packs: &DirPacks) -> usize {
    packs.list().expect("listed").len()
}

fn file_bytes(packs: &DirPacks) -> Vec<Vec<u8>> {
    packs.files().into_iter().map(|(_, b)| b).collect()
}

/// Every version of `kept` reads back, from its object or its row. A
/// stop during the removals leaves a packed version's object too, which
/// `held_once` refuses until recovery has removed it.
fn reads_back(f: &Fixture, kept: &[Kept], what: &str) {
    for (n, k) in kept.iter().enumerate() {
        let got = f.store.get(&f.caller, k.uuid).expect("reads");
        assert_eq!(got.body, k.body, "{what}: version {n} reads back");
    }
}

/// Recovery removes the objects a stop left, and then each version of
/// `kept` is held once.
fn whole_after_recovery(f: &Fixture, packs: &DirPacks, kept: &[Kept], what: &str) {
    f.store.recover(Writers::None).expect("recovers");
    held_once(f, packs, kept, what);
}

/// One file per date and no merge, so a few versions make several files.
fn plan() -> PackPlan {
    PackPlan {
        merge_below: 0,
        ..PackPlan::with_limit(1 << 20)
    }
}

/// A plan that merges small files, as a configured server's does.
fn merging() -> PackPlan {
    PackPlan::with_limit(1 << 20)
}

/// `n` vCons, each stored a day after the one before, so each packs into
/// a file of its own, with pack storage and nothing packed yet.
fn apart(f: &mut Fixture, n: u8) -> (Arc<DirPacks>, Vec<Kept>) {
    let mut kept = Vec::new();
    for i in 0..n {
        kept.push(created(f, i));
        later(f, Duration::days(1));
    }
    (with_packs(f), kept)
}

/// `n` vCons of one date, so they pack into one file, with pack storage
/// and nothing packed yet.
fn together(f: &mut Fixture, n: u8) -> (Arc<DirPacks>, Vec<Kept>) {
    let kept = (0..n).map(|i| created(f, i)).collect();
    (with_packs(f), kept)
}

/// `n` small pack files of one date, one vCon in each.
fn small_files(f: &mut Fixture, n: u8) -> (Arc<DirPacks>, Vec<Kept>) {
    let packs = with_packs(f);
    let mut kept = Vec::new();
    for i in 0..n {
        kept.push(created(f, i));
        f.store.pack(&plan()).expect("packs a small file");
    }
    assert_eq!(files(&packs), usize::from(n), "{n} small files");
    (packs, kept)
}

/// `n` vCons, each packed into a file of its own, then deleted and due
/// for purge under DAY: a sweep purges each row and then rewrites each
/// file away.
fn packed_and_deleted(f: &mut Fixture, n: u8) -> (Arc<DirPacks>, Vec<Kept>) {
    let (packs, kept) = apart(f, n);
    f.store.pack(&plan()).expect("packs");
    assert_eq!(files(&packs), usize::from(n), "a file for each");
    for k in &kept {
        f.store
            .delete(&f.caller, k.uuid, Retention::Years(7))
            .expect("deletes");
    }
    later(f, Duration::days(2));
    (packs, kept)
}

/// Three vCons packed into one file; the first is deleted and due for
/// purge under DAY, so a sweep purges its row and then rewrites the file
/// with the other two.
fn one_purged_of_a_shared_file(f: &mut Fixture) -> (Arc<DirPacks>, Vec<Kept>) {
    let (packs, kept) = together(f, 3);
    f.store.pack(&plan()).expect("packs");
    assert_eq!(files(&packs), 1, "one file");
    f.store
        .delete(&f.caller, kept[0].uuid, Retention::Years(7))
        .expect("deletes");
    later(f, Duration::days(2));
    (packs, kept)
}

// Success: the purge sweep ends after the version under way.

#[test]
fn a_purge_stopped_at_once_finishes_the_version_under_way() {
    let f = fixture();
    let kept = deleted(&f, 5);
    let stop = Stop::after(0);
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 1 }),
        "the version under way, no other"
    );
    assert_eq!(purged_count(&f, &kept), 1, "one version purged");
}

#[test]
fn a_purge_stopped_after_one_more_version_purges_two() {
    let f = fixture();
    let kept = deleted(&f, 5);
    let stop = Stop::after(1);
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 2 }),
        "two versions"
    );
    assert_eq!(purged_count(&f, &kept), 2, "two rows gone");
}

#[test]
fn a_purge_stopped_after_each_count_purges_exactly_one_more() {
    for go in 0..=3usize {
        let f = fixture();
        let kept = deleted(&f, 4);
        let stop = Stop::after(go);
        let report = f
            .store
            .purge_due_until(DAY, &|| stop.ask())
            .expect("purges");
        assert_eq!(
            report.purged,
            go as u64 + 1,
            "stopped after {go}: the report"
        );
        assert_eq!(
            purged_count(&f, &kept),
            go + 1,
            "stopped after {go}: the rows"
        );
    }
}

#[test]
fn the_next_sweep_purges_what_a_stopped_sweep_left() {
    let f = fixture();
    let kept = deleted(&f, 5);
    let stop = Stop::after(1);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges two");
    assert_eq!(
        f.store.purge_due(DAY),
        Ok(PurgeReport { purged: 3 }),
        "the rest"
    );
    assert_eq!(purged_count(&f, &kept), 5, "every version purged");
}

#[test]
fn each_version_is_whole_on_either_side_of_the_stop() {
    let f = fixture();
    let kept = deleted(&f, 5);
    let stop = Stop::after(2);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges three");
    let gone = purged(&f, &kept);
    assert_eq!(gone.iter().filter(|g| **g).count(), 3, "three purged");
    for (n, (k, gone)) in kept.iter().zip(gone).enumerate() {
        if gone {
            assert!(!object_exists(&f, k), "version {n}: purged, object gone");
            assert!(tombstoned(&f, k), "version {n}: purged, tombstone left");
        } else {
            assert!(object_exists(&f, k), "version {n}: kept, object kept");
            assert!(!tombstoned(&f, k), "version {n}: kept, no tombstone");
            assert!(
                f.store.legal_get(&f.caller, k.uuid, None).is_ok(),
                "version {n}: kept, and it reads back"
            );
        }
    }
}

#[test]
fn only_the_versions_purged_before_the_stop_are_recorded() {
    let f = fixture();
    let kept = deleted(&f, 5);
    let stop = Stop::after(1);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges two");
    let gone: BTreeSet<String> = kept
        .iter()
        .zip(purged(&f, &kept))
        .filter(|(_, gone)| *gone)
        .map(|(k, _)| k.uuid.to_string())
        .collect();
    assert_eq!(gone.len(), 2, "two purged");
    assert_eq!(purge_records(&f), gone, "a record for each, and no other");
}

#[test]
fn a_purge_never_stopped_purges_every_due_version() {
    let f = fixture();
    let kept = deleted(&f, 5);
    let stop = Stop::never();
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 5 }),
        "every version"
    );
    assert_eq!(purged_count(&f, &kept), 5, "nothing left");
}

#[test]
fn a_held_version_is_kept_by_a_stopped_and_an_unstopped_sweep() {
    let f = fixture();
    let kept = deleted(&f, 3);
    f.store
        .set_hold(&f.caller, kept[0].uuid, true)
        .expect("holds");
    let stop = Stop::after(0);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("sweeps");
    f.store.purge_due(DAY).expect("sweeps");
    assert_eq!(
        purged(&f, &kept),
        [false, true, true],
        "the held one kept, the rest purged"
    );
}

#[test]
fn a_delete_s_own_purge_is_not_stopped() {
    let f = fixture();
    let k = created(&f, 1);
    f.store
        .delete(&f.caller, k.uuid, Retention::Days(0))
        .expect("deletes and purges");
    assert_eq!(purged(&f, &[k]), [true], "purged by the delete itself");
}

// Success: the purge's pack file rewrites end after the file under way.

#[test]
fn a_stop_after_the_rows_leaves_every_rewrite_listed() {
    let mut f = fixture();
    let (packs, kept) = packed_and_deleted(&mut f, 3);
    let before = file_bytes(&packs);
    // Asked before the second and third rows, then refused before the
    // first rewrite.
    let stop = Stop::after(2);
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 3 }),
        "every row purged"
    );
    assert_eq!(purge_pending(&f), 3, "every rewrite still listed");
    assert_eq!(file_bytes(&packs), before, "no file rewritten");
    assert_eq!(purged_count(&f, &kept), 3, "the rows are gone");
}

#[test]
fn a_stop_between_rewrites_rewrites_only_the_first() {
    let mut f = fixture();
    let (packs, _kept) = packed_and_deleted(&mut f, 3);
    let stop = Stop::after(3);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges");
    assert_eq!(purge_pending(&f), 2, "two rewrites wait");
    assert_eq!(files(&packs), 2, "one file rewritten away");
}

#[test]
fn recovery_finishes_the_rewrites_a_stop_left() {
    let mut f = fixture();
    let (packs, _kept) = packed_and_deleted(&mut f, 3);
    let stop = Stop::after(2);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges the rows");
    assert_eq!(files(&packs), 3, "the files wait");
    f.store.recover(Writers::None).expect("recovers");
    assert_eq!(purge_pending(&f), 0, "nothing waits");
    assert_eq!(files(&packs), 0, "each file held only purged rows");
}

#[test]
fn the_next_sweep_finishes_the_rewrites_a_stop_left() {
    let mut f = fixture();
    let (packs, _kept) = packed_and_deleted(&mut f, 3);
    let stop = Stop::after(2);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges the rows");
    assert_eq!(
        f.store.purge_due(DAY),
        Ok(PurgeReport { purged: 0 }),
        "nothing more to purge"
    );
    assert_eq!(purge_pending(&f), 0, "nothing waits");
    assert_eq!(files(&packs), 0, "every file rewritten away");
}

// Success: the pack run ends after the object under way.

#[test]
fn a_pack_run_stopped_at_once_writes_no_file() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 4);
    let stop = Stop::after(0);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 0, "no file");
    assert_eq!(report.versions, 0, "no version");
    assert_eq!(files(&packs), 0, "nothing put");
    assert_eq!(pending_files(&f), 0, "no file under way");
    assert_eq!(objects(&f), 4, "every object kept");
    held_once(&f, &packs, &kept, "stopped at once");
}

#[test]
fn a_pack_run_stopped_while_it_reads_a_file_s_objects_writes_no_file() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 4);
    let stop = Stop::after(2);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 0, "the file is not written");
    assert_eq!(files(&packs), 0, "nothing put");
    assert_eq!(pending_files(&f), 0, "no file under way");
    assert!(packed_versions(&f).is_empty(), "nothing packed");
    held_once(&f, &packs, &kept, "stopped while reading");
}

#[test]
fn a_pack_run_stopped_after_one_file_packs_only_that_file() {
    let mut f = fixture();
    let (packs, kept) = apart(&mut f, 4);
    // Stopped once the first file is put: its object stays for recovery.
    let report = f
        .store
        .pack_until(&plan(), &|| files(&packs) >= 1)
        .expect("runs");
    assert_eq!(report.files, 1, "one file");
    assert_eq!(report.versions, 1, "its one version");
    assert_eq!(files(&packs), 1, "one file put");
    assert_eq!(packed_versions(&f).len(), 1, "one version packed");
    assert_eq!(objects(&f), 4, "the packed version's object waits");
    reads_back(&f, &kept, "stopped after one file");
    whole_after_recovery(&f, &packs, &kept, "stopped after one file");
}

#[test]
fn a_pack_run_stopped_while_it_removes_objects_leaves_them_to_recovery() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 4);
    // Three more reads and one removal go; the second removal is refused.
    let stop = Stop::after(4);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.versions, 4, "the file holds all four");
    assert_eq!(objects(&f), 3, "three objects wait for recovery");
    reads_back(&f, &kept, "stopped while removing");
    whole_after_recovery(&f, &packs, &kept, "after recovery");
    assert_eq!(objects(&f), 0, "recovery removed them");
}

#[test]
fn the_next_pack_run_packs_what_a_stopped_run_left() {
    let mut f = fixture();
    let (packs, kept) = apart(&mut f, 4);
    f.store
        .pack_until(&plan(), &|| files(&packs) >= 1)
        .expect("runs");
    let report = f.store.pack(&plan()).expect("packs the rest");
    assert_eq!(report.files, 3, "the three other files");
    assert_eq!(packed_versions(&f).len(), 4, "every version packed");
    f.store.recover(Writers::None).expect("recovers");
    assert_eq!(objects(&f), 0, "no object left");
    held_once(&f, &packs, &kept, "after the next run");
}

#[test]
fn a_pack_run_never_stopped_packs_every_object() {
    let mut f = fixture();
    let (packs, kept) = apart(&mut f, 3);
    let stop = Stop::never();
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 3, "a file for each date");
    assert_eq!(report.versions, 3, "every version");
    assert_eq!(objects(&f), 0, "no object left");
    held_once(&f, &packs, &kept, "never stopped");
}

#[test]
fn a_stopped_pack_run_still_publishes_the_file_it_recorded() {
    let mut f = fixture();
    let (packs, _kept) = apart(&mut f, 3);
    f.store
        .pack_until(&plan(), &|| files(&packs) >= 1)
        .expect("runs");
    let unpublished = f
        .store
        .catalog
        .snapshot()
        .expect("a snapshot")
        .unpublished_manifests()
        .expect("reads");
    assert!(unpublished.is_empty(), "every manifest numbered is written");
    let manifests = std::fs::read_dir(packs.path("manifest"))
        .expect("a manifest directory")
        .count();
    assert_eq!(manifests, 1, "a manifest names the file");
}

#[test]
fn a_stopped_pack_run_merges_nothing() {
    let mut f = fixture();
    let (packs, mut kept) = small_files(&mut f, 3);
    kept.push(created(&f, 9));
    // The new object is read and put; the stop comes before its removal,
    // and the merge after it is not begun.
    let stop = Stop::after(0);
    let report = f
        .store
        .pack_until(&merging(), &|| stop.ask())
        .expect("runs");
    assert_eq!(report.files, 1, "the new object's file");
    assert_eq!(report.merged, 0, "the merge waits for the next run");
    assert_eq!(files(&packs), 4, "four small files");
    let report = f.store.pack(&merging()).expect("merges");
    assert_eq!(report.merged, 1, "the next run merges them");
    whole_after_recovery(&f, &packs, &kept, "after the merge");
}

#[test]
fn a_stopped_pack_run_leaves_expired_files_to_the_next_run() {
    let mut f = fixture();
    let (packs, mut kept) = small_files(&mut f, 2);
    let old: BTreeSet<String> = packs.list().expect("listed").into_iter().collect();
    let soon = PackPlan {
        grace: std::time::Duration::from_secs(1),
        ..merging()
    };
    assert_eq!(f.store.pack(&soon).map(|r| r.merged), Ok(1), "merged");
    later(&f, Duration::hours(1));
    kept.push(created(&f, 9));
    // The new object is read and put; the stop comes before its removal.
    let stop = Stop::after(0);
    f.store.pack_until(&soon, &|| stop.ask()).expect("runs");
    let now: BTreeSet<String> = packs.list().expect("listed").into_iter().collect();
    assert!(
        old.is_subset(&now),
        "the expired files wait: {old:?} in {now:?}"
    );
    f.store.pack(&soon).expect("the next run");
    let after: BTreeSet<String> = packs.list().expect("listed").into_iter().collect();
    assert!(
        old.is_disjoint(&after),
        "the next run removes them: {after:?}"
    );
    whole_after_recovery(&f, &packs, &kept, "after the next run");
}

// Failures: an error before the stop is still an error, and leaves what
// an error leaves.

#[test]
fn a_purge_whose_catalog_fails_on_its_first_version_fails() {
    let f = fixture();
    let kept = deleted(&f, 3);
    f.fs.fail_from(f.fs.log().len());
    let stop = Stop::never();
    let ran = f.store.purge_due_until(DAY, &|| stop.ask());
    f.fs.revive();
    assert!(ran.is_err(), "the failure is answered: {ran:?}");
    assert_eq!(stop.asked(), 0, "no ask before the first version");
    assert_eq!(kept.len(), 3, "three were due");
}

#[test]
fn a_purge_whose_catalog_fails_after_a_go_answers_the_failure() {
    let f = fixture();
    deleted(&f, 4);
    let stop = Stop::never();
    let ran = f.store.purge_due_until(DAY, &|| {
        f.fs.fail_from(f.fs.log().len());
        stop.ask()
    });
    f.fs.revive();
    assert!(ran.is_err(), "the second version's failure: {ran:?}");
    assert_eq!(stop.asked(), 1, "the run ended at its failure");
}

#[test]
fn a_rewrite_that_fails_before_the_stop_fails_the_sweep() {
    let mut f = fixture();
    let (packs, _kept) = one_purged_of_a_shared_file(&mut f);
    packs.fail_puts.store(true, Ordering::SeqCst);
    let stop = Stop::never();
    let ran = f.store.purge_due_until(DAY, &|| stop.ask());
    packs.fail_puts.store(false, Ordering::SeqCst);
    assert!(ran.is_err(), "the rewrite's put failed: {ran:?}");
    assert_eq!(purge_pending(&f), 1, "the rewrite stays listed");
    assert_eq!(stop.asked(), 1, "asked before the rewrite");
}

#[test]
fn a_rewrite_refused_by_the_stop_is_not_a_failure() {
    let mut f = fixture();
    let (packs, kept) = one_purged_of_a_shared_file(&mut f);
    packs.fail_puts.store(true, Ordering::SeqCst);
    // The row goes, then the stop before the rewrite, whose put would
    // fail.
    let stop = Stop::after(0);
    let ran = f.store.purge_due_until(DAY, &|| stop.ask());
    packs.fail_puts.store(false, Ordering::SeqCst);
    assert_eq!(ran, Ok(PurgeReport { purged: 1 }), "no rewrite was tried");
    let files_put = packs
        .puts
        .lock()
        .expect("lock")
        .iter()
        .filter(|name| !name.starts_with("manifest/"))
        .count();
    assert!(
        files_put == 1,
        "only the pack run's file was put, no rewrite: {files_put}"
    );
    assert_eq!(purge_pending(&f), 1, "the rewrite stays listed");
    held_once(&f, &packs, &kept[1..], "the kept rows read back");
}

#[test]
fn a_rewrite_left_by_the_stop_is_made_by_recovery() {
    let mut f = fixture();
    let (packs, kept) = one_purged_of_a_shared_file(&mut f);
    let sealed = file_bytes(&packs);
    let stop = Stop::after(0);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges the row");
    assert_eq!(file_bytes(&packs), sealed, "the file is as it was");
    f.store.recover(Writers::None).expect("recovers");
    assert_eq!(purge_pending(&f), 0, "nothing waits");
    assert_ne!(file_bytes(&packs), sealed, "the file was rewritten");
    held_once(&f, &packs, &kept[1..], "the kept rows read back");
}

#[test]
fn a_pack_run_whose_removal_fails_before_the_stop_fails() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 2);
    // Once the file is put, every change fails: the first removal is
    // asked before, goes, and fails.
    let ran = f.store.pack_until(&plan(), &|| {
        if files(&packs) >= 1 {
            f.fs.fail_from(f.fs.log().len());
        }
        false
    });
    f.fs.revive();
    assert!(ran.is_err(), "the removal's failure is answered: {ran:?}");
    assert_eq!(files(&packs), 1, "the file was put");
    reads_back(&f, &kept, "after the failed removal");
}

#[test]
fn a_pack_run_whose_put_fails_after_a_go_answers_the_failure() {
    let mut f = fixture();
    let (packs, _kept) = apart(&mut f, 2);
    let stop = Stop::never();
    let ran = f.store.pack_until(&plan(), &|| {
        packs.fail_puts.store(true, Ordering::SeqCst);
        stop.ask()
    });
    packs.fail_puts.store(false, Ordering::SeqCst);
    assert!(ran.is_err(), "the second file's put failed: {ran:?}");
    assert_eq!(files(&packs), 1, "the first file was put");
}

#[test]
fn a_pack_run_whose_put_fails_before_the_stop_fails() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 3);
    packs.fail_puts.store(true, Ordering::SeqCst);
    let stop = Stop::never();
    let ran = f.store.pack_until(&plan(), &|| stop.ask()).map(|r| r.files);
    packs.fail_puts.store(false, Ordering::SeqCst);
    assert!(ran.is_err(), "the put failed: {ran:?}");
    assert_eq!(objects(&f), 3, "every object kept");
    held_once(&f, &packs, &kept, "after the failed put");
}

#[test]
fn a_pack_run_whose_put_would_fail_is_not_tried_after_the_stop() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 3);
    packs.fail_puts.store(true, Ordering::SeqCst);
    let stop = Stop::after(1);
    let ran = f.store.pack_until(&plan(), &|| stop.ask()).map(|r| r.files);
    packs.fail_puts.store(false, Ordering::SeqCst);
    assert_eq!(ran, Ok(0), "no put was tried");
    assert!(packs.puts.lock().expect("lock").is_empty(), "no name asked");
    held_once(&f, &packs, &kept, "after the stop");
}

#[test]
fn a_pack_run_whose_catalog_fails_before_the_stop_fails() {
    let mut f = fixture();
    let (_packs, _kept) = together(&mut f, 3);
    f.fs.fail_from(f.fs.log().len());
    let stop = Stop::never();
    let ran = f.store.pack_until(&plan(), &|| stop.ask());
    f.fs.revive();
    assert!(ran.is_err(), "the failure is answered: {ran:?}");
    assert_eq!(stop.asked(), 2, "asked before the second and third reads");
}

#[test]
fn a_pack_run_whose_manifest_fails_after_the_stop_still_fails() {
    let mut f = fixture();
    let (packs, _kept) = apart(&mut f, 3);
    packs.fail_manifests.store(true, Ordering::SeqCst);
    let ran = f.store.pack_until(&plan(), &|| files(&packs) >= 1);
    packs.fail_manifests.store(false, Ordering::SeqCst);
    assert!(ran.is_err(), "the manifest's failure is answered: {ran:?}");
    assert_eq!(files(&packs), 1, "the file was put before it");
}

#[test]
fn a_pack_run_whose_reads_fail_before_the_stop_fails() {
    let mut f = fixture();
    let (packs, _kept) = small_files(&mut f, 2);
    packs.fail_reads.store(true, Ordering::SeqCst);
    let stop = Stop::never();
    let ran = f.store.pack_until(&merging(), &|| stop.ask());
    packs.fail_reads.store(false, Ordering::SeqCst);
    assert!(ran.is_err(), "the merge's read failed: {ran:?}");
    assert_eq!(files(&packs), 2, "the small files are kept");
}

// Edges: where the stop is asked, and how often.

#[test]
fn a_purge_asks_between_versions_and_not_before_the_first() {
    let f = fixture();
    deleted(&f, 5);
    let stop = Stop::never();
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges");
    assert_eq!(stop.asked(), 4, "before each version but the first");
}

#[test]
fn a_stopped_purge_asks_no_more_after_it_stops() {
    let f = fixture();
    deleted(&f, 5);
    let stop = Stop::after(1);
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges");
    assert_eq!(stop.asked(), 2, "one go, one stop, then no ask");
}

#[test]
fn a_purge_with_nothing_hidden_never_asks() {
    let f = fixture();
    created(&f, 1);
    let stop = Stop::after(0);
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 0 }),
        "nothing to purge"
    );
    assert_eq!(stop.asked(), 0, "no version to ask before");
}

#[test]
fn a_hidden_version_not_yet_due_is_an_item_of_the_sweep() {
    let f = fixture();
    for n in 0..2 {
        let k = created(&f, n);
        f.store
            .delete(&f.caller, k.uuid, Retention::Years(7))
            .expect("deletes");
    }
    let stop = Stop::never();
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 0 }),
        "not due yet"
    );
    assert_eq!(stop.asked(), 1, "asked before the second listed");
}

#[test]
fn a_purge_asks_before_each_rewrite_too() {
    let mut f = fixture();
    packed_and_deleted(&mut f, 3);
    let stop = Stop::never();
    f.store
        .purge_due_until(DAY, &|| stop.ask())
        .expect("purges");
    assert_eq!(stop.asked(), 5, "three rows and three rewrites, less one");
}

#[test]
fn a_pack_run_asks_before_each_read_and_each_removal() {
    let mut f = fixture();
    together(&mut f, 3);
    let stop = Stop::never();
    f.store.pack_until(&plan(), &|| stop.ask()).expect("packs");
    assert_eq!(stop.asked(), 5, "three reads and three removals, less one");
}

#[test]
fn a_store_without_pack_storage_never_asks_in_a_pack_run() {
    let f = fixture();
    created(&f, 1);
    let stop = Stop::after(0);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 0, "nothing to pack into");
    assert_eq!(stop.asked(), 0, "no item");
}

#[test]
fn a_pack_run_with_nothing_to_pack_never_asks() {
    let mut f = fixture();
    with_packs(&mut f);
    let stop = Stop::after(0);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 0, "nothing to pack");
    assert_eq!(stop.asked(), 0, "no item");
}

#[test]
fn a_merge_that_is_the_run_s_first_item_is_not_asked_before() {
    let mut f = fixture();
    small_files(&mut f, 2);
    let stop = Stop::after(0);
    let report = f
        .store
        .pack_until(&merging(), &|| stop.ask())
        .expect("merges");
    assert_eq!(report.merged, 1, "merged");
    assert_eq!(stop.asked(), 0, "the step's first item");
}

#[test]
fn a_merge_after_other_items_is_asked_before() {
    let mut f = fixture();
    small_files(&mut f, 2);
    created(&f, 9);
    let stop = Stop::never();
    let report = f
        .store
        .pack_until(&merging(), &|| stop.ask())
        .expect("merges");
    assert_eq!(report.merged, 1, "merged");
    assert_eq!(stop.asked(), 2, "before the removal and the merge");
}

#[test]
fn a_pack_run_stopped_after_its_last_removal_is_whole() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 2);
    // One more read and two removals: every ask goes.
    let stop = Stop::after(3);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.versions, 2, "both packed");
    assert_eq!(stop.asked(), 3, "no ask after the last item");
    assert_eq!(objects(&f), 0, "no object left");
    held_once(&f, &packs, &kept, "stopped at the end");
}

#[test]
fn a_pack_run_of_objects_over_the_limit_never_asks() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 2);
    let stop = Stop::after(0);
    let tiny = PackPlan { limit: 1, ..plan() };
    let report = f.store.pack_until(&tiny, &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 0, "every object over the limit");
    assert_eq!(stop.asked(), 0, "no object to read");
    held_once(&f, &packs, &kept, "over the limit");
}

#[test]
fn a_pack_run_stopped_on_a_file_s_last_read_writes_no_file() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 3);
    // The second read goes, the third is refused.
    let stop = Stop::after(1);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 0, "no file");
    assert_eq!(
        stop.asked(),
        2,
        "the third read was asked, then the run ended"
    );
    held_once(&f, &packs, &kept, "stopped at the last read");
}

#[test]
fn a_pack_run_stopped_between_files_asks_no_more() {
    let mut f = fixture();
    apart(&mut f, 3);
    // The first file's removal goes; the second file's read is refused.
    let stop = Stop::after(1);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 1, "the first file");
    assert_eq!(stop.asked(), 2, "the run ended at the refused ask");
}

#[test]
fn a_stopped_pack_run_s_file_is_recorded_once_and_reads_back() {
    let mut f = fixture();
    let (packs, kept) = together(&mut f, 3);
    // Two more reads go; the first removal is refused.
    let stop = Stop::after(2);
    let report = f.store.pack_until(&plan(), &|| stop.ask()).expect("runs");
    assert_eq!(report.files, 1, "the file under way is finished");
    assert_eq!(pending_files(&f), 0, "and no longer under way");
    assert_eq!(objects(&f), 3, "its objects wait for recovery");
    reads_back(&f, &kept, "stopped after the put");
    whole_after_recovery(&f, &packs, &kept, "stopped after the put");
}

#[test]
fn a_purge_of_one_version_never_asks() {
    let f = fixture();
    let kept = deleted(&f, 1);
    let stop = Stop::after(0);
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 1 }),
        "the only version"
    );
    assert_eq!(stop.asked(), 0, "no item after the first");
    assert!(!object_exists(&f, &kept[0]), "its object removed");
}

#[test]
fn a_version_whose_object_is_already_gone_is_still_an_item() {
    let f = fixture();
    let kept = deleted(&f, 3);
    f.fs.remove(&f.store.objects_dir.join(kept[1].version.to_string()))
        .expect("removes an object");
    let stop = Stop::after(1);
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 2 }),
        "two purged, then the stop"
    );
}

#[test]
fn a_replaced_version_is_an_item_of_the_sweep() {
    let f = fixture();
    let first = created(&f, 1);
    let (found, body) = vcon(&uuid_text(1), "second");
    f.store
        .replace(&f.caller, first.version, &found, &body)
        .expect("replaces");
    let other = created(&f, 2);
    f.store
        .delete(&f.caller, other.uuid, Retention::Years(7))
        .expect("deletes");
    later(&f, Duration::days(2));
    let stop = Stop::after(0);
    assert_eq!(
        f.store.purge_due_until(DAY, &|| stop.ask()),
        Ok(PurgeReport { purged: 1 }),
        "one of the two hidden versions"
    );
    assert_eq!(stop.asked(), 1, "asked before the second");
    assert_eq!(
        f.store.get(&f.caller, first.uuid).expect("reads").body,
        body,
        "the live version reads back"
    );
}

#[test]
fn pack_and_purge_due_without_a_stop_do_every_item() {
    let mut f = fixture();
    let (_packs, _kept) = together(&mut f, 3);
    assert_eq!(
        f.store.pack(&plan()).map(|r| r.versions),
        Ok(3),
        "pack packs every object"
    );
    let g = fixture();
    deleted(&g, 3);
    assert_eq!(
        g.store.purge_due(DAY),
        Ok(PurgeReport { purged: 3 }),
        "purge_due purges every version due"
    );
}
