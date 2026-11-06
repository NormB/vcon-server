//! Compaction: each pack run merges a date's small pack files into one, so
//! the many small files that frequent pack runs write become few. A file
//! is small below the plan's `merge_below`. The rows are copied byte for
//! byte into one new file, which one transaction records while it moves the
//! places and stops naming the old files, only if those still hold exactly
//! the rows copied. The old files stay for the reader grace, for readers on
//! an older manifest, and are then deleted; a purge deletes them at once,
//! since any of them may hold the purged row. A file with a purge pending
//! is never merged: the purge rewrites it.

use std::collections::{BTreeMap, HashSet};

use super::packer::PackPlan;
use super::{Store, StoreError};
use crate::clock::nanos;
use crate::ids::FileId;

impl Store {
    /// Merges each date's small files, as the module describes; the number
    /// of new files written.
    pub(crate) fn compact(&self, plan: &PackPlan) -> Result<u64, StoreError> {
        if self.packs.is_none() || plan.merge_below == 0 {
            return Ok(0);
        }
        let (named, pending) = {
            let mut tx = self.catalog.snapshot()?;
            let pending: HashSet<FileId> = tx.purge_pending()?.into_iter().collect();
            (tx.named_files()?, pending)
        };
        // Each date's small files, in the catalog's order, up to one file's
        // size together.
        let mut dates: BTreeMap<String, Vec<(FileId, u64)>> = BTreeMap::new();
        for (file, info, _) in named {
            if info.bytes < plan.merge_below && !pending.contains(&file) {
                dates.entry(info.date).or_default().push((file, info.bytes));
            }
        }
        let mut merged = 0;
        for small in dates.into_values() {
            let mut chosen = Vec::new();
            let mut total = 0u64;
            for (file, bytes) in small {
                if total.saturating_add(bytes) > plan.sizes.file as u64 && chosen.len() >= 2 {
                    break;
                }
                total = total.saturating_add(bytes);
                chosen.push(file);
            }
            if chosen.len() < 2 {
                continue;
            }
            if let Some(copied) = self.copy_files(&chosen)?
                && self.swap(&copied)?
            {
                merged += 1;
            }
        }
        Ok(merged)
    }

    /// Deletes each file no place names any more and no purge waits on,
    /// once it has been unnamed for `plan.grace`.
    pub(crate) fn remove_expired(&self, plan: &PackPlan) -> Result<(), StoreError> {
        let Some(packs) = self.packs.as_deref() else {
            return Ok(());
        };
        let now = nanos(self.clock.now());
        let grace = i64::try_from(plan.grace.as_nanos()).unwrap_or(i64::MAX);
        let (unnamed, pending) = {
            let mut tx = self.catalog.snapshot()?;
            let pending: HashSet<FileId> = tx.purge_pending()?.into_iter().collect();
            let mut unnamed = Vec::new();
            for file in tx.unnamed_files()? {
                if let Some(info) = tx.pack_file(file)? {
                    unnamed.push((file, info.unnamed_at));
                }
            }
            (unnamed, pending)
        };
        for (file, at) in unnamed {
            let expired = at.is_some_and(|at| now.saturating_sub(at) >= grace);
            if expired && !pending.contains(&file) {
                self.remove_unnamed(packs, file)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use time::Duration as Span;

    use super::*;
    use crate::backend::Packs;
    use crate::clock::Clock;
    use crate::retention::Retention;
    use crate::store::Created;
    use crate::store::packed::tests::DirPacks;
    use crate::store::packer::tests::{Kept, held_once, with_packs};
    use crate::store::testing::vcon;
    use crate::store::tests::{Fixture, fixture};

    const GRACE: std::time::Duration = std::time::Duration::from_secs(900);

    fn plan() -> PackPlan {
        PackPlan {
            grace: GRACE,
            ..PackPlan::with_limit(1 << 20)
        }
    }

    fn created(f: &Fixture, n: u8) -> (Kept, Vec<u8>) {
        let (found, body) = vcon(
            &format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}"),
            &format!("MERGE-SUBJECT-{n}"),
        );
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let sealed = f.store.objects.get(version).expect("its object");
        (
            Kept {
                uuid: found.uuid,
                version,
                body,
            },
            sealed,
        )
    }

    /// Three versions, each packed by a run of its own with merging off:
    /// three small files of one date.
    fn three_small_files(f: &mut Fixture) -> (std::sync::Arc<DirPacks>, Vec<(Kept, Vec<u8>)>) {
        let packs = with_packs(f);
        let apart = PackPlan {
            merge_below: 0,
            ..plan()
        };
        let mut kept = Vec::new();
        for n in 1..=3 {
            kept.push(created(f, n));
            f.store.pack(&apart).expect("packs");
        }
        assert_eq!(packs.list().expect("listed").len(), 3, "three files");
        (packs, kept)
    }

    fn named(f: &Fixture) -> BTreeSet<[u8; 16]> {
        let conn = f.store.conn_for_test();
        let mut rows = conn
            .prepare("SELECT file_id FROM parquet_file WHERE unnamed_at IS NULL")
            .expect("prepare");
        rows.query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    /// A run merges a date's small files into one new file; every version
    /// reads back from it, the old files stay unnamed for the grace, and a
    /// run after the grace deletes them.
    #[test]
    fn small_files_of_a_date_are_merged_and_the_old_ones_go_after_the_grace() {
        let mut f = fixture();
        let (packs, kept) = three_small_files(&mut f);
        let report = f.store.pack(&plan()).expect("packs");
        assert_eq!(report.merged, 1, "one new file");
        assert_eq!(named(&f).len(), 1, "the catalog names only it");
        assert_eq!(packs.list().expect("listed").len(), 4, "the old ones stay");
        let kept: Vec<Kept> = kept.into_iter().map(|(k, _)| k).collect();
        held_once(&f, &packs, &kept, "merged");
        f.clock.set(f.clock.now() + Span::seconds(899));
        f.store.pack(&plan()).expect("packs");
        assert_eq!(packs.list().expect("listed").len(), 4, "within the grace");
        f.clock.set(f.clock.now() + Span::seconds(1));
        f.store.pack(&plan()).expect("packs");
        assert_eq!(packs.list().expect("listed").len(), 1, "after the grace");
        held_once(&f, &packs, &kept, "after the grace");
    }

    /// A purge deletes every unnamed file at once, though its grace has
    /// not ended: an old file of a merge holds the purged row too.
    #[test]
    fn a_purge_deletes_the_merged_files_at_once() {
        let mut f = fixture();
        let (packs, kept) = three_small_files(&mut f);
        f.store.pack(&plan()).expect("packs and merges");
        let (gone, sealed) = &kept[0];
        f.store
            .delete(&f.caller, gone.uuid, Retention::Days(0))
            .expect("deleted and purged");
        for (name, bytes) in packs.files() {
            assert!(
                !bytes.windows(sealed.len()).any(|w| w == sealed),
                "{name} still holds the purged row"
            );
        }
        assert_eq!(packs.list().expect("listed").len(), 1, "one file left");
        let rest: Vec<Kept> = kept[1..].iter().map(|(k, _)| k.clone()).collect();
        held_once(&f, &packs, &rest, "after the purge");
    }

    /// A large file, a file of another date, and a file a purge waits on
    /// are not merged; nor is a date with one small file.
    #[test]
    fn only_small_files_of_one_date_and_no_purge_are_merged() {
        let mut f = fixture();
        with_packs(&mut f);
        created(&f, 1);
        f.store.pack(&plan()).expect("packs");
        assert_eq!(
            f.store.compact(&plan()),
            Ok(0),
            "one file: nothing to merge"
        );
        f.clock.set(f.clock.now() + Span::days(1));
        created(&f, 2);
        let apart = PackPlan {
            merge_below: 0,
            ..plan()
        };
        f.store.pack(&apart).expect("packs");
        assert_eq!(
            f.store.compact(&plan()),
            Ok(0),
            "two dates: nothing to merge"
        );
        created(&f, 3);
        f.store.pack(&apart).expect("packs");
        let tiny = PackPlan {
            merge_below: 10,
            ..plan()
        };
        assert_eq!(f.store.compact(&tiny), Ok(0), "files over the threshold");
        let ids: Vec<[u8; 16]> = named(&f).into_iter().collect();
        f.store
            .conn_for_test()
            .execute("INSERT INTO purge_pending (file_id) VALUES (?1)", [ids[0]])
            .expect("a purge waits on one");
        let merged = f.store.compact(&plan()).expect("compacts");
        assert!(merged <= 1, "at most the two not waited on: {merged}");
        assert!(
            named(&f).contains(&ids[0]),
            "the file a purge waits on stays named"
        );
    }

    /// 4.5: a read that found its version's place, then saw its file merged
    /// away and deleted (a grace of zero) before it read the row, reads
    /// again at the new place and returns the whole body.
    #[test]
    fn a_read_racing_a_compaction_returns_the_whole_body() {
        let mut f = fixture();
        let (_, kept) = three_small_files(&mut f);
        let now = PackPlan {
            grace: std::time::Duration::ZERO,
            ..plan()
        };
        let store = &f.store;
        let read = std::thread::scope(|scope| {
            let (go, wait) = std::sync::mpsc::channel::<()>();
            let (done, finished) = std::sync::mpsc::channel::<()>();
            *store.hooks.before_read.lock().expect("hook") = Some(Box::new(move || {
                go.send(()).expect("the merge is told");
                finished
                    .recv_timeout(std::time::Duration::from_secs(30))
                    .expect("the merge finishes");
            }));
            let merge = scope.spawn(move || {
                wait.recv_timeout(std::time::Duration::from_secs(30))
                    .expect("the read is between its steps");
                let ran = store.pack(&now).expect("packs and merges");
                done.send(()).expect("the read is told");
                ran
            });
            let read = store.get(&f.caller, kept[1].0.uuid);
            assert_eq!(merge.join().expect("merged").merged, 1, "the merge ran");
            read
        });
        assert_eq!(
            read.map(|s| s.body),
            Ok(kept[1].0.body.clone()),
            "the whole body"
        );
    }
}
