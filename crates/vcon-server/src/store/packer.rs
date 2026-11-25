//! The pack run: a tenant's versions still in their own objects are packed
//! into Parquet files, one store date at a time, oldest first.
//!
//! For each file: the objects are read, the file is written in memory and
//! PUT under a new random name only if no file has it; then one catalog
//! transaction records the file and moves each version's place to its row,
//! but only if every one of them is still kept and still unpacked; then
//! the objects are removed. A run cut off at any step leaves each version
//! readable: before the commit from its object, after it from its row.
//! What a cut leaves behind, start-up recovery removes: a file no row
//! names, or the object of a packed version. A name already taken stops
//! the run with an error and is never tried under another name.

use super::{Store, StoreError, io, read_failed};
use crate::backend::pack::{self, Place};
use crate::backend::{CommitError, NewPackFile, ObjectError};
use crate::clock::nanos;
use crate::envelope::SEALED_OVERHEAD;
use crate::export::Metadata;
use crate::export::columns::{About, File, Row, SIZES, Sizes};
use crate::ids::{FileId, VersionId};

/// What a pack run may pack, and the files it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackPlan {
    /// The largest sealed object packed; a larger one stays an object.
    pub(crate) limit: u64,
    pub(crate) sizes: Sizes,
    /// How long a manifest, and a file a compaction stopped naming, stays
    /// after a newer manifest is written, for readers outside the server.
    pub(crate) grace: std::time::Duration,
    /// A pack file below this many bytes is small, and merged with the
    /// other small files of its date; 0 merges nothing.
    pub(crate) merge_below: u64,
}

impl PackPlan {
    /// The plan for the configured limit, with the export's file sizes and
    /// the default reader grace.
    pub(crate) fn with_limit(limit: u64) -> PackPlan {
        PackPlan {
            limit,
            sizes: SIZES,
            grace: crate::config::READER_GRACE,
            merge_below: SIZES.file as u64 / 2,
        }
    }
}

/// What a pack run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PackReport {
    /// Files written and recorded.
    pub files: u64,
    /// Versions moved into them.
    pub versions: u64,
    /// Files written by merging small files.
    pub merged: u64,
}

/// A file a run will write: versions of one store date, in store order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Planned {
    date: String,
    /// Each version with its store time.
    versions: Vec<(VersionId, i64)>,
}

/// A file written to the pack storage and not yet recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Written {
    file: FileId,
    date: String,
    bytes: u64,
    rows: Vec<(VersionId, Place)>,
}

impl Store {
    /// One pack run, as the module describes. A Store with no pack storage
    /// packs nothing.
    pub(crate) fn pack(&self, plan: &PackPlan) -> Result<PackReport, StoreError> {
        let mut report = PackReport::default();
        if self.packs.is_none() {
            return Ok(report);
        }
        for planned in self.plan_files(plan)? {
            let Some(written) = self.write_file(&planned, plan)? else {
                continue;
            };
            if self.record_file(&written)? {
                self.release(&written)?;
                report.files += 1;
                report.versions += written.rows.len() as u64;
            }
        }
        report.merged = self.compact(plan)?;
        self.refresh_manifest()?;
        self.publish()?;
        self.remove_expired(plan)?;
        self.prune_manifests(Some(plan.grace))?;
        Ok(report)
    }

    /// The files a run will write, from one snapshot of the catalog: the
    /// unpacked versions whose sealed object is within the limit, by store
    /// date and then store order, a new file begun once one holds about
    /// its size.
    pub(crate) fn plan_files(&self, plan: &PackPlan) -> Result<Vec<Planned>, StoreError> {
        let rows = self.read(|tx| tx.unpacked())?;
        let mut files: Vec<Planned> = Vec::new();
        let mut held = 0u64;
        for row in rows {
            let sealed = u64::try_from(row.size)
                .ok()
                .and_then(|n| n.checked_add(SEALED_OVERHEAD as u64))
                .ok_or(StoreError::Damaged)?;
            if sealed > plan.limit {
                continue;
            }
            let date = pack::date_of(row.stored_at).ok_or(StoreError::Damaged)?;
            let next = match files.last() {
                Some(last) => last.date != date || held >= plan.sizes.file as u64,
                None => true,
            };
            if next {
                files.push(Planned {
                    date,
                    versions: Vec::new(),
                });
                held = 0;
            }
            held += sealed;
            files
                .last_mut()
                .expect("pushed above")
                .versions
                .push((row.version, row.stored_at));
        }
        Ok(files)
    }

    /// Reads `planned`'s objects, writes its file and puts it under a new
    /// name. A version whose object is gone (purged since the plan) is
    /// left out; a file with no row left is not written. None: nothing
    /// was written.
    pub(crate) fn write_file(
        &self,
        planned: &Planned,
        plan: &PackPlan,
    ) -> Result<Option<Written>, StoreError> {
        let packs = self.packs.as_deref().ok_or(StoreError::Io)?;
        let mut file = File::new(Metadata::None, plan.sizes).map_err(io)?;
        let mut rows = Vec::new();
        for &(version, stored_at) in &planned.versions {
            let sealed = match self.objects.get(version) {
                Ok(sealed) => sealed,
                Err(ObjectError::Missing) => continue,
                Err(e) => return Err(read_failed(e)),
            };
            let place = file
                .push(Row {
                    version,
                    stored_at,
                    sealed,
                    about: About::default(),
                })
                .map_err(io)?;
            rows.push((version, place));
        }
        if rows.is_empty() {
            return Ok(None);
        }
        let bytes = file.finish().map_err(io)?;
        let id = FileId::generate();
        let name = pack::name(&planned.date, id);
        match packs.put_new(&name, &bytes) {
            Ok(true) => {}
            Ok(false) => {
                // Another writer holds the name: a second server on this
                // prefix, or a fault. Never tried under another name.
                tracing::warn!("a pack file's new name was already taken; the pack run stops");
                return Err(StoreError::Io);
            }
            Err(e) => return Err(read_failed(e)),
        }
        Ok(Some(Written {
            file: id,
            date: planned.date.clone(),
            bytes: bytes.len() as u64,
            rows,
        }))
    }

    /// Records `written` and moves its versions' places, if every one of
    /// them is still kept and unpacked; if not, removes the file and
    /// answers false. A commit rolled back removes the file too; one whose
    /// outcome is unknown keeps it, since its rows may be kept.
    pub(crate) fn record_file(&self, written: &Written) -> Result<bool, StoreError> {
        let packs = self.packs.as_deref().ok_or(StoreError::Io)?;
        let name = pack::name(&written.date, written.file);
        let mut tx = self.catalog.immediate()?;
        for (version, _) in &written.rows {
            if !tx.is_unpacked(*version)? {
                drop(tx);
                packs.remove(&name).map_err(read_failed)?;
                return Ok(false);
            }
        }
        tx.insert_pack_file(&NewPackFile {
            file: written.file,
            date: &written.date,
            rows: written.rows.len() as i64,
            bytes: i64::try_from(written.bytes).map_err(io)?,
            created_at: nanos(self.clock.now()),
        })?;
        for (version, place) in &written.rows {
            tx.set_place(*version, written.file, *place)?;
        }
        tx.allocate_manifest()?;
        match tx.commit() {
            Ok(()) => Ok(true),
            Err(CommitError::RolledBack) => {
                packs.remove(&name).map_err(read_failed)?;
                Err(StoreError::Io)
            }
            Err(CommitError::Unknown) => Err(StoreError::Io),
        }
    }

    /// Removes the objects of the versions `written` packed.
    fn release(&self, written: &Written) -> Result<(), StoreError> {
        for (version, _) in &written.rows {
            match self.objects.remove(*version) {
                Ok(()) | Err(ObjectError::Missing) => {}
                Err(e) => return Err(read_failed(e)),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use time::Duration;

    use super::*;
    use crate::backend::Packs;
    use crate::clock::Clock;
    use crate::fs::Op;
    use crate::retention::Retention;
    use crate::store::Created;
    use crate::store::packed::tests::DirPacks;
    use crate::store::testing::vcon;
    use crate::store::tests::{Fixture, fixture};

    /// Small files and row groups, so a few versions fill several.
    const SMALL: Sizes = Sizes {
        row_group: 300,
        file: 900,
    };

    fn plan(limit: u64, sizes: Sizes) -> PackPlan {
        PackPlan {
            limit,
            sizes,
            ..PackPlan::with_limit(limit)
        }
    }

    /// A stored version: its uuid, version id and body.
    #[derive(Debug, Clone)]
    pub(crate) struct Kept {
        pub(crate) uuid: uuid::Uuid,
        pub(crate) version: VersionId,
        pub(crate) body: Vec<u8>,
    }

    fn created(f: &Fixture, n: u8, subject: &str) -> Kept {
        let (found, body) = vcon(
            &format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}"),
            subject,
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

    /// Five vCons over two store dates, the first two a day before the
    /// rest.
    fn over_two_dates(f: &Fixture) -> Vec<Kept> {
        let mut kept = Vec::new();
        for n in 0..5u8 {
            if n == 2 {
                f.clock.set(f.clock.now() + Duration::days(1));
            }
            kept.push(created(f, n, &"subject ".repeat(usize::from(n) + 3)));
        }
        kept
    }

    pub(crate) fn with_packs(f: &mut Fixture) -> Arc<DirPacks> {
        let packs = DirPacks::new();
        f.store.packs = Some(packs.clone());
        f.packs = Some(packs.clone());
        packs
    }

    /// The versions whose place names a file, by version id.
    pub(crate) fn packed_versions(f: &Fixture) -> BTreeSet<[u8; 16]> {
        let conn = f.store.conn_for_test();
        let mut rows = conn
            .prepare("SELECT version FROM version WHERE place_file IS NOT NULL")
            .expect("prepare");
        rows.query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    /// The file ids the catalog records.
    fn recorded_files(f: &Fixture) -> BTreeSet<[u8; 16]> {
        let conn = f.store.conn_for_test();
        let mut rows = conn
            .prepare("SELECT file_id FROM parquet_file")
            .expect("prepare");
        rows.query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    /// Each kept version reads back its body, and its blob is held exactly
    /// once: in its object, or, if packed, in a file the catalog records
    /// and the storage holds. No object is left of a packed version, no
    /// file that no row names, and nothing of a version no longer kept.
    pub(crate) fn held_once(f: &Fixture, packs: &DirPacks, kept: &[Kept], what: &str) {
        let objects: BTreeSet<String> = f
            .store
            .objects
            .list()
            .expect("listed")
            .into_iter()
            .collect();
        let packed = packed_versions(f);
        let recorded = recorded_files(f);
        let files: BTreeSet<[u8; 16]> = packs
            .list()
            .expect("listed")
            .iter()
            .map(|n| pack::parse_name(n).expect("a pack file's name").1.0)
            .collect();
        assert_eq!(
            files, recorded,
            "{what}: every file is recorded, and only those"
        );
        // A failure names a version by its place in `kept`, never by its
        // uuid: a message is written to the test's log.
        for (n, k) in kept.iter().enumerate() {
            let got = f.store.get(&f.caller, k.uuid).expect("reads");
            assert_eq!(got.body, k.body, "{what}: version {n} reads back");
            let as_object = objects.contains(&k.version.to_string());
            let as_row = packed.contains(&k.version.0);
            assert!(
                as_object != as_row,
                "{what}: version {n} is held once (object {as_object}, row {as_row})"
            );
        }
        assert_eq!(
            objects.len() + packed.len(),
            kept.len(),
            "{what}: nothing else is held"
        );
    }

    /// One run packs every unpacked version, one file per store date,
    /// oldest date first, each named by its date; every version reads back
    /// and no object is left. A second run finds nothing to pack.
    #[test]
    fn a_pack_run_packs_each_store_date_into_a_file_oldest_first() {
        let mut f = fixture();
        let kept = over_two_dates(&f);
        let packs = with_packs(&mut f);
        let report = f.store.pack(&PackPlan::with_limit(1 << 20)).expect("packs");
        assert_eq!(
            report,
            PackReport {
                files: 2,
                versions: 5,
                merged: 0
            },
            "two dates"
        );
        // The pack files' names; a run writes its manifest too.
        let files_put = || -> Vec<String> {
            packs
                .puts
                .lock()
                .expect("puts")
                .iter()
                .filter(|n| !n.starts_with("manifest/"))
                .cloned()
                .collect()
        };
        let dates: Vec<String> = files_put()
            .iter()
            .map(|n| pack::parse_name(n).expect("a pack file's name").0)
            .collect();
        assert_eq!(dates, ["2026-10-23", "2026-10-24"], "oldest date first");
        held_once(&f, &packs, &kept, "packed");
        assert!(
            f.store.objects.list().expect("listed").is_empty(),
            "no object is left"
        );
        assert_eq!(
            f.store.pack(&PackPlan::with_limit(1 << 20)).expect("packs"),
            PackReport::default(),
            "nothing is left to pack"
        );
        assert_eq!(files_put().len(), 2, "no file written");
    }

    /// A file that reaches its size is closed, and the date's next
    /// versions go in the next file.
    #[test]
    fn a_full_file_is_closed_and_the_date_goes_on_in_the_next() {
        let mut f = fixture();
        let kept: Vec<Kept> = (0..6u8)
            .map(|n| created(&f, n, &"long subject ".repeat(20)))
            .collect();
        let packs = with_packs(&mut f);
        let report = f.store.pack(&plan(1 << 20, SMALL)).expect("packs");
        assert!(report.files > 1, "several files: {report:?}");
        assert_eq!(report.versions, 6, "every version");
        held_once(&f, &packs, &kept, "several files");
    }

    /// A sealed object over the limit stays an object; the others are
    /// packed.
    #[test]
    fn a_version_over_the_pack_limit_stays_an_object() {
        let mut f = fixture();
        let small = created(&f, 1, "small");
        let large = created(&f, 2, &"large ".repeat(400));
        let packs = with_packs(&mut f);
        let limit = f.store.objects.get(small.version).expect("object").len() as u64;
        let report = f.store.pack(&plan(limit, SMALL)).expect("packs");
        assert_eq!(report.versions, 1, "only the small one");
        let objects = f.store.objects.list().expect("listed");
        assert_eq!(objects, [large.version.to_string()], "the large one stays");
        held_once(&f, &packs, &[small, large], "over the limit");
    }

    /// Where a pack run is cut off.
    #[derive(Debug, Clone)]
    enum Cut {
        /// The file's PUT fails.
        Put,
        /// The commit is rolled back.
        RolledBack,
        /// The commit is kept, then reported unknown.
        Unknown,
        /// The process dies at the nth step of the run's filesystem log:
        /// the commit's WAL sync or a change to the objects after it.
        At(usize, Op),
    }

    /// The filesystem log of a pack run of three versions when nothing
    /// fails.
    fn uncut_pack_log() -> Vec<Op> {
        let mut f = fixture();
        let _kept: Vec<Kept> = (0..3u8).map(|n| created(&f, n, "cut")).collect();
        with_packs(&mut f);
        let ops = f.fs.log().len();
        f.store.pack(&PackPlan::with_limit(1 << 20)).expect("packs");
        f.fs.log()[ops..].to_vec()
    }

    /// 4.3 and 4.4: a pack run cut off at any step returns an error, and
    /// every version still reads back, unless the cut failed the commit's
    /// WAL sync: then the catalog is poisoned and refuses reads until it is
    /// reopened (nothing is answered from a commit that may be lost).
    /// After a restart's recovery each version is held exactly once, and
    /// the next run packs the rest.
    #[test]
    fn a_pack_run_cut_off_at_any_step_leaves_every_version_held_once() {
        let uncut = uncut_pack_log();
        assert!(
            matches!(uncut.first(), Some(Op::SyncData(_))),
            "the commit's sync comes before any object is removed: {uncut:?}"
        );
        assert_eq!(
            uncut
                .iter()
                .filter(|op| matches!(op, Op::Remove(_)))
                .count(),
            3,
            "three objects, one removal each: {uncut:?}"
        );
        let mut cuts = vec![Cut::Put, Cut::RolledBack, Cut::Unknown];
        cuts.extend(
            uncut
                .iter()
                .cloned()
                .enumerate()
                .map(|(n, op)| Cut::At(n, op)),
        );
        for cut in cuts {
            let mut f = fixture();
            let kept: Vec<Kept> = (0..3u8).map(|n| created(&f, n, "cut")).collect();
            let packs = with_packs(&mut f);
            match &cut {
                Cut::Put => packs.fail_puts.store(true, Ordering::SeqCst),
                Cut::RolledBack => f.store.catalog.fail_commit(1, CommitError::RolledBack),
                Cut::Unknown => f.store.catalog.fail_commit(1, CommitError::Unknown),
                Cut::At(n, _) => f.fs.fail_from(f.fs.log().len() + n),
            }
            let what = format!("{cut:?}");
            let ran = f.store.pack(&PackPlan::with_limit(1 << 20));
            assert_eq!(ran, Err(StoreError::Io), "{what}: the run fails");
            if matches!(cut, Cut::Put | Cut::RolledBack) {
                assert_eq!(
                    packs.list().expect("listed"),
                    Vec::<String>::new(),
                    "{what}: no file is left that no row names, before any restart"
                );
            }
            packs.fail_puts.store(false, Ordering::SeqCst);
            // The process lives on: what it answers next is checked.
            f.fs.revive();
            let poisoned = matches!(cut, Cut::At(_, Op::SyncData(_)));
            for (n, k) in kept.iter().enumerate() {
                let read = f.store.get(&f.caller, k.uuid).map(|s| s.body);
                if poisoned {
                    assert_eq!(
                        read,
                        Err(StoreError::Io),
                        "{what}: version {n} is refused until the catalog is reopened"
                    );
                } else {
                    assert_eq!(
                        read,
                        Ok(k.body.clone()),
                        "{what}: version {n} reads back before the restart"
                    );
                }
            }
            f.fs.crash();
            f.reopen();
            held_once(&f, &packs, &kept, &format!("{what}, restarted"));
            f.store
                .pack(&PackPlan::with_limit(1 << 20))
                .expect("packs the rest");
            held_once(&f, &packs, &kept, &format!("{what}, packed again"));
            assert!(
                f.store.objects.list().expect("listed").is_empty(),
                "{what}: every version is packed in the end"
            );
        }
    }

    /// 4.15: a file name another writer took (`If-None-Match` refused)
    /// stops the run with an error; nothing is tried under another name and
    /// every version stays in its object.
    #[test]
    fn a_name_already_taken_stops_the_run_and_is_not_tried_again() {
        let mut f = fixture();
        let kept = over_two_dates(&f);
        let packs = with_packs(&mut f);
        packs.taken.store(true, Ordering::SeqCst);
        assert_eq!(
            f.store.pack(&PackPlan::with_limit(1 << 20)),
            Err(StoreError::Io),
            "the run stops"
        );
        assert_eq!(packs.puts.lock().expect("puts").len(), 1, "one name tried");
        assert!(packs.list().expect("listed").is_empty(), "no file");
        assert!(packed_versions(&f).is_empty(), "no version moved");
        held_once(&f, &packs, &kept, "taken");
    }

    /// A version purged after the run wrote its file, before the run
    /// recorded it, is in no file: the record is refused and the file
    /// removed; the next run packs the rest.
    #[test]
    fn a_version_purged_during_a_run_is_in_no_file() {
        let mut f = fixture();
        let kept: Vec<Kept> = (0..3u8).map(|n| created(&f, n, "racing")).collect();
        let packs = with_packs(&mut f);
        let gone = kept[1].clone();
        let sealed = f.store.objects.get(gone.version).expect("its object");
        let run = PackPlan::with_limit(1 << 20);
        let planned = f.store.plan_files(&run).expect("planned");
        assert_eq!(planned.len(), 1, "one file");
        let written = f
            .store
            .write_file(&planned[0], &run)
            .expect("written")
            .expect("a file");
        assert_eq!(packs.files().len(), 1, "the file is in the storage");
        f.store
            .delete(&f.caller, gone.uuid, Retention::Days(0))
            .expect("deleted and purged");
        assert_eq!(
            f.store.record_file(&written),
            Ok(false),
            "the record is refused"
        );
        for (name, bytes) in packs.files() {
            assert!(
                !bytes.windows(sealed.len()).any(|w| w == sealed),
                "{name} holds the purged version's sealed bytes"
            );
        }
        let rest = [kept[0].clone(), kept[2].clone()];
        held_once(&f, &packs, &rest, "after the race");
        f.store.pack(&run).expect("packs");
        held_once(&f, &packs, &rest, "packed on the next run");
        assert!(
            f.store.objects.list().expect("listed").is_empty(),
            "both packed"
        );
    }

    /// `packed_versions` counts the versions whose row names a pack file:
    /// none before a pack run, each one it moved after, and `examine`
    /// reports the same count.
    #[test]
    fn the_versions_a_pack_run_moved_are_counted_as_packed() {
        let mut f = fixture();
        for n in 0..3u8 {
            created(&f, n, "kept");
        }
        assert_eq!(f.store.packed_versions(), Ok(0), "all objects");
        with_packs(&mut f);
        f.store.pack(&PackPlan::with_limit(1 << 20)).expect("packs");
        assert_eq!(packed_versions(&f).len(), 3, "the run packed every version");
        assert_eq!(f.store.packed_versions(), Ok(3), "each packed one counted");
        let now = f.clock.now();
        let found = f
            .store
            .examine(Retention::Years(7), now, now)
            .expect("examined");
        assert_eq!(found.packed, 3, "examine counts them too");
    }

    /// `examine`, which runs alone, lists the object of a packed version as
    /// a leftover, as start-up recovery would remove it, and finds nothing
    /// damaged.
    #[test]
    fn examine_lists_the_object_of_a_packed_version_as_a_leftover() {
        let mut f = fixture();
        let kept: Vec<Kept> = (0..2u8).map(|n| created(&f, n, "kept")).collect();
        with_packs(&mut f);
        f.store.pack(&PackPlan::with_limit(1 << 20)).expect("packs");
        f.store
            .objects
            .put(kept[1].version, b"left by a run cut off after its commit")
            .expect("an object left behind");
        let now = f.clock.now();
        let found = f
            .store
            .examine(Retention::Years(7), now, now)
            .expect("examined");
        assert_eq!(
            found.leftovers,
            [kept[1].version.to_string()],
            "the leftover"
        );
        assert!(
            found.damaged.is_empty(),
            "nothing damaged: {:?}",
            found.damaged
        );
    }

    /// 4.12: rewrapping every data key changes no byte of any pack file,
    /// and every version reads back under the new key.
    #[test]
    fn a_rewrap_leaves_every_pack_file_byte_unchanged() {
        let mut f = fixture();
        let kept: Vec<Kept> = (0..3u8).map(|n| created(&f, n, "kept")).collect();
        let packs = with_packs(&mut f);
        f.store.pack(&PackPlan::with_limit(1 << 20)).expect("packs");
        let before = packs.files();
        let new = crate::envelope::Key::generate().expect("key");
        assert_eq!(f.store.rewrap(&new, 100), Ok(3), "every key rewrapped");
        assert_eq!(packs.files(), before, "every file, byte for byte");
        f.store.tenant_key = new;
        held_once(&f, &packs, &kept, "rewrapped");
    }

    /// Start-up recovery removes a pack file no row names and the object of
    /// a packed version, and nothing else: a name not ours stays.
    #[test]
    fn recovery_removes_unnamed_files_and_the_objects_of_packed_versions() {
        let mut f = fixture();
        let kept: Vec<Kept> = (0..2u8).map(|n| created(&f, n, "kept")).collect();
        let packs = with_packs(&mut f);
        f.store.pack(&PackPlan::with_limit(1 << 20)).expect("packs");
        let stray = pack::name("2026-10-23", FileId::generate());
        assert!(
            packs.put_new(&stray, b"no row names this").expect("put"),
            "new"
        );
        let foreign = "parquet/not-ours.txt";
        assert!(
            packs.put_new(foreign, b"someone else's").expect("put"),
            "new"
        );
        f.store
            .objects
            .put(kept[0].version, b"left by a run cut off after its commit")
            .expect("an object left by a cut-off run");
        f.reopen();
        let left = packs.list().expect("listed");
        assert!(
            !left.contains(&stray),
            "the unnamed file is removed: {left:?}"
        );
        assert!(left.iter().any(|n| n == foreign), "a name not ours stays");
        assert!(
            f.store.objects.list().expect("listed").is_empty(),
            "the packed version's object is removed"
        );
        packs.remove(foreign).expect("removed");
        held_once(&f, &packs, &kept, "recovered");
    }

    /// Runs a pack run in the gap between a read's first step and its read
    /// of the version's object, and returns the read's answer and the run's
    /// report.
    fn read_racing_a_pack(
        f: &Fixture,
        uuid: uuid::Uuid,
    ) -> (Result<Vec<u8>, StoreError>, PackReport) {
        let store = &f.store;
        std::thread::scope(|scope| {
            let (go, wait) = std::sync::mpsc::channel::<()>();
            let (done, finished) = std::sync::mpsc::channel::<()>();
            *store.hooks.before_read.lock().expect("hook") = Some(Box::new(move || {
                go.send(()).expect("the pack run is told");
                finished
                    .recv_timeout(std::time::Duration::from_secs(30))
                    .expect("the pack run finishes");
            }));
            let run = scope.spawn(move || {
                wait.recv_timeout(std::time::Duration::from_secs(30))
                    .expect("the read is between its steps");
                let report = store.pack(&PackPlan::with_limit(1 << 20)).expect("packs");
                done.send(()).expect("the read is told");
                report
            });
            let read = store.get(&f.caller, uuid).map(|s| s.body);
            (read, run.join().expect("the pack run ends"))
        })
    }

    /// The read records, as status codes, in the log's order.
    fn read_statuses(f: &Fixture) -> Vec<serde_json::Value> {
        f.store
            .records()
            .iter()
            .filter(|r| r["action"] == "read")
            .map(|r| r["status"].clone())
            .collect()
    }

    /// A read that found its version unpacked, then saw a pack run move it
    /// into a file and remove its object before it read the object, reads
    /// again at the new place and returns the whole body.
    #[test]
    fn a_read_racing_the_first_pack_of_its_version_returns_the_whole_body() {
        let mut f = fixture();
        let kept = created(&f, 1, "packed under a read");
        with_packs(&mut f);
        let (read, report) = read_racing_a_pack(&f, kept.uuid);
        assert_eq!(report.versions, 1, "the run packed the version");
        assert!(
            f.store.objects.list().expect("listed").is_empty(),
            "and removed its object under the read"
        );
        assert_eq!(
            read,
            Ok(kept.body.clone()),
            "the read returns the whole body"
        );
    }

    /// That read is recorded once, as answered, and never as damage.
    #[test]
    fn a_read_racing_the_first_pack_is_recorded_once_as_answered() {
        let mut f = fixture();
        let kept = created(&f, 2, "recorded under a pack");
        with_packs(&mut f);
        let (read, _) = read_racing_a_pack(&f, kept.uuid);
        assert!(read.is_ok(), "the read is answered: {read:?}");
        assert_eq!(
            read_statuses(&f),
            [serde_json::Value::from(200)],
            "one record, status 200"
        );
    }

    /// How long a read of a damaged version may take before the test calls
    /// it a retry without end.
    const BOUND: std::time::Duration = std::time::Duration::from_secs(20);

    /// `f.store.get` of `uuid` on a thread of its own; panics if it has no
    /// answer within `BOUND`, so a read that tries again without end fails
    /// the test instead of hanging it.
    fn bounded_get(f: &Arc<Fixture>, uuid: uuid::Uuid) -> Result<Vec<u8>, StoreError> {
        let (answer, answered) = std::sync::mpsc::channel();
        let g = f.clone();
        let reader = std::thread::spawn(move || {
            let _ = answer.send(g.store.get(&g.caller, uuid).map(|s| s.body));
        });
        let got = answered.recv_timeout(BOUND).unwrap_or_else(|_| {
            panic!("the read had no answer within {BOUND:?}: it tries again without end")
        });
        // The thread ends after it answers: it drops its clone of the store
        // and OpenSSL frees its per-thread state. Wait for that, as
        // manifest.rs's publish test does: a test process that exits while
        // the thread is ending made LeakSanitizer report OpenSSL's
        // per-thread allocations (CI, AddressSanitizer job 138173071466).
        reader.join().expect("the reading thread ends");
        got
    }

    /// An unpacked version whose object is lost, with no pack run between
    /// the read's steps, is damage: its place did not change, so the read
    /// is not tried again, and is recorded once as a failure.
    #[test]
    fn a_lost_object_of_an_unpacked_version_is_still_damage() {
        let f = Arc::new(fixture());
        let kept = created(&f, 3, "lost");
        f.store.objects.remove(kept.version).expect("removed");
        assert_eq!(
            bounded_get(&f, kept.uuid),
            Err(StoreError::Damaged),
            "damage"
        );
        assert_eq!(
            read_statuses(&f),
            [serde_json::Value::from(500)],
            "one record, status 500"
        );
    }

    /// A packed version whose file is lost is damage too: the same place
    /// still failing is the answer, after one try.
    #[test]
    fn a_packed_version_whose_file_is_lost_is_still_damage() {
        let mut f = fixture();
        let kept = created(&f, 4, "packed, then lost");
        let packs = with_packs(&mut f);
        f.store.pack(&PackPlan::with_limit(1 << 20)).expect("packs");
        for name in packs.list().expect("listed") {
            if pack::parse_name(&name).is_some() {
                packs.remove(&name).expect("the file is lost");
            }
        }
        let f = Arc::new(f);
        assert_eq!(
            bounded_get(&f, kept.uuid),
            Err(StoreError::Damaged),
            "damage"
        );
        assert_eq!(
            read_statuses(&f),
            [serde_json::Value::from(500)],
            "one record, status 500"
        );
    }

    /// A version whose row does not read (its place names a file the
    /// catalog has no row for) is damage, and the read is not tried again
    /// for it: a row that does not read is no new place.
    #[test]
    fn a_version_whose_row_does_not_read_is_damage_after_one_try() {
        let f = Arc::new(fixture());
        let kept = created(&f, 5, "a row that does not read");
        let changed = f
            .store
            .conn_for_test()
            .execute(
                "UPDATE version SET place_file = ?1, place_group = 0, place_row = 0",
                [vec![9u8; 16]],
            )
            .expect("the place names a file with no row");
        assert_eq!(changed, 1, "one version");
        assert_eq!(
            bounded_get(&f, kept.uuid),
            Err(StoreError::Damaged),
            "damage"
        );
        assert_eq!(
            read_statuses(&f),
            [serde_json::Value::from(500)],
            "one record, status 500"
        );
    }
}
