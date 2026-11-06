//! The file side of a purge: a pack file that held a purged row is
//! rewritten without it, and the old file is deleted and confirmed gone,
//! before the purge returns.
//!
//! The purge's own transaction deletes the version's row, the only copy of
//! its wrapped data key, and lists its file in `purge_pending`; from then
//! on the row's sealed bytes cannot be opened by anyone. Then, for each
//! listed file: its kept rows are copied byte for byte into a new file
//! under a new name; one transaction moves their places to it, records it,
//! marks the old file unnamed and takes it off the list, but only if the
//! old file's rows are still the ones copied; then the old file is deleted
//! and HeadObject must say it is gone; then its row is removed. A run cut
//! off at any step is finished by start-up recovery and by the next purge.

use std::collections::BTreeSet;

use super::packed::Unread;
use super::{Store, StoreError, io, read_failed};
use crate::backend::pack::{self, Place};
use crate::backend::{CommitError, NewPackFile, Packs};
use crate::clock::nanos;
use crate::export::Metadata;
use crate::export::columns::{About, File, Row, SIZES};
use crate::ids::{FileId, VersionId};

/// How many times the file side of a purge goes round the files left to
/// finish before it gives up: a second purge rewriting the same file at
/// the same time makes a round find its copy out of date.
const ROUNDS: usize = 4;

/// A file's kept rows, copied into a new file not yet recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Copied {
    from: FileId,
    /// The versions copied, which `from` must still hold, and no other.
    versions: BTreeSet<[u8; 16]>,
    /// The new file and each row's place in it; None when no row was left
    /// to copy.
    to: Option<NewFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewFile {
    file: FileId,
    date: String,
    bytes: u64,
    rows: Vec<(VersionId, Place)>,
}

impl Store {
    /// Rewrites every file a purge listed, and removes every unnamed file,
    /// as the module describes. A Store with no pack storage has neither.
    pub(crate) fn finish_purges(&self) -> Result<(), StoreError> {
        let Some(packs) = self.packs.as_deref() else {
            return Ok(());
        };
        for _ in 0..ROUNDS {
            let (pending, unnamed) = {
                let mut tx = self.catalog.snapshot()?;
                (tx.purge_pending()?, tx.unnamed_files()?)
            };
            if pending.is_empty() && unnamed.is_empty() {
                return Ok(());
            }
            for file in unnamed {
                self.remove_unnamed(packs, file)?;
            }
            for file in pending {
                match self.copy_file(file)? {
                    None => {
                        let mut tx = self.catalog.immediate()?;
                        tx.delete_purge_pending(file)?;
                        super::commit(tx)?;
                    }
                    Some(copied) => {
                        if self.swap(&copied)? {
                            self.remove_unnamed(packs, file)?;
                        }
                    }
                }
            }
        }
        tracing::warn!("purges left pack files to rewrite after {ROUNDS} rounds");
        Err(StoreError::Io)
    }

    /// Copies the kept rows of `from` into a new file, byte for byte. None:
    /// `from` has no row, or the storage no longer holds it, so no byte of
    /// it is left to remove (its kept rows are damage, which `check`
    /// reports).
    pub(crate) fn copy_file(&self, from: FileId) -> Result<Option<Copied>, StoreError> {
        let packs = self.packs.as_deref().ok_or(StoreError::Io)?;
        let (info, rows) = {
            let mut tx = self.catalog.snapshot()?;
            (tx.pack_file(from)?, tx.placed_in(from)?)
        };
        let Some(info) = info else {
            return Ok(None);
        };
        let name = pack::name(&info.date, from);
        if packs.size(&name).map_err(read_failed)?.is_none() {
            return Ok(None);
        }
        let versions = rows.iter().map(|(v, _, _)| v.0).collect();
        if rows.is_empty() {
            return Ok(Some(Copied {
                from,
                versions,
                to: None,
            }));
        }
        let mut file = File::new(Metadata::None, SIZES).map_err(io)?;
        let mut placed = Vec::new();
        for (version, stored_at, packed) in rows {
            let sealed = self
                .sealed_blob(version, Some(&packed))
                .map_err(Unread::into_store)?;
            let place = file
                .push(Row {
                    version,
                    stored_at,
                    sealed,
                    about: About::default(),
                })
                .map_err(io)?;
            placed.push((version, place));
        }
        let bytes = file.finish().map_err(io)?;
        let id = FileId::generate();
        if !packs
            .put_new(&pack::name(&info.date, id), &bytes)
            .map_err(read_failed)?
        {
            tracing::warn!("a rewritten pack file's new name was already taken");
            return Err(StoreError::Io);
        }
        Ok(Some(Copied {
            from,
            versions,
            to: Some(NewFile {
                file: id,
                date: info.date,
                bytes: bytes.len() as u64,
                rows: placed,
            }),
        }))
    }

    /// Moves `copied`'s rows to its new file if `copied.from` still holds
    /// exactly those rows; if not, removes the new file and answers false.
    /// The old file is marked unnamed and taken off the purge list. A
    /// commit rolled back removes the new file; one whose outcome is
    /// unknown keeps it.
    pub(crate) fn swap(&self, copied: &Copied) -> Result<bool, StoreError> {
        let packs = self.packs.as_deref().ok_or(StoreError::Io)?;
        let remove_new = || -> Result<(), StoreError> {
            match &copied.to {
                Some(to) => packs
                    .remove(&pack::name(&to.date, to.file))
                    .map_err(read_failed),
                None => Ok(()),
            }
        };
        let mut tx = self.catalog.immediate()?;
        let now: BTreeSet<[u8; 16]> = tx
            .placed_in(copied.from)?
            .into_iter()
            .map(|(v, _, _)| v.0)
            .collect();
        if now != copied.versions || tx.pack_file(copied.from)?.is_none() {
            drop(tx);
            remove_new()?;
            return Ok(false);
        }
        if let Some(to) = &copied.to {
            tx.insert_pack_file(&NewPackFile {
                file: to.file,
                date: &to.date,
                rows: to.rows.len() as i64,
                bytes: i64::try_from(to.bytes).map_err(io)?,
                created_at: nanos(self.clock.now()),
            })?;
            for (version, place) in &to.rows {
                tx.set_place(*version, to.file, *place)?;
            }
        }
        tx.unname_pack_file(copied.from, nanos(self.clock.now()))?;
        tx.delete_purge_pending(copied.from)?;
        match tx.commit() {
            Ok(()) => Ok(true),
            Err(CommitError::RolledBack) => {
                remove_new()?;
                Err(StoreError::Io)
            }
            Err(CommitError::Unknown) => Err(StoreError::Io),
        }
    }

    /// Deletes an unnamed file, checks that the storage no longer has it,
    /// and removes its row.
    fn remove_unnamed(&self, packs: &dyn Packs, file: FileId) -> Result<(), StoreError> {
        let Some(info) = self.catalog.snapshot()?.pack_file(file)? else {
            return Ok(());
        };
        let name = pack::name(&info.date, file);
        packs.remove(&name).map_err(read_failed)?;
        if packs.size(&name).map_err(read_failed)?.is_some() {
            tracing::warn!("the storage still holds a pack file it was told to delete");
            return Err(StoreError::Io);
        }
        let mut tx = self.catalog.immediate()?;
        tx.delete_pack_file(file)?;
        super::commit(tx)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::Ordering;

    use time::Duration;

    use super::*;
    use crate::backend::{CommitError, Packs, pack};
    use crate::clock::Clock;
    use crate::retention::Retention;
    use crate::store::Created;
    use crate::store::packed::tests::DirPacks;
    use crate::store::packer::PackPlan;
    use crate::store::packer::tests::{Kept, held_once, with_packs};
    use crate::store::testing::vcon;
    use crate::store::tests::{Fixture, fixture};

    const PLAN: fn() -> PackPlan = || PackPlan::with_limit(1 << 20);
    const DAY: Retention = Retention::Days(1);

    /// A stored version, with its subject and sealed bytes for the scans.
    struct Stored {
        kept: Kept,
        subject: String,
        sealed: Vec<u8>,
    }

    fn created(f: &Fixture, n: u8) -> Stored {
        let subject = format!("SUBJECT-MARKER-{n:02}-{}", "x".repeat(40));
        let (found, body) = vcon(
            &format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}"),
            &subject,
        );
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let sealed = f.store.objects.get(version).expect("its object");
        Stored {
            kept: Kept {
                uuid: found.uuid,
                version,
                body,
            },
            subject,
            sealed,
        }
    }

    /// Three vCons packed into one file; the first is deleted, so it falls
    /// due a day later.
    fn three_packed(f: &mut Fixture) -> (std::sync::Arc<DirPacks>, Vec<Stored>, String) {
        let stored: Vec<Stored> = (1..=3).map(|n| created(f, n)).collect();
        let packs = with_packs(f);
        f.store.pack(&PLAN()).expect("packs");
        let names = packs.list().expect("listed");
        assert_eq!(names.len(), 1, "one file");
        f.store
            .delete(&f.caller, stored[0].kept.uuid, Retention::Years(7))
            .expect("deleted");
        (packs, stored, names[0].clone())
    }

    fn later(f: &Fixture) {
        f.clock.set(f.clock.now() + Duration::days(2));
    }

    /// No file the storage holds has any of `gone`'s sealed bytes, uuid
    /// (as text or its 16 bytes) or subject.
    fn in_no_file(packs: &DirPacks, gone: &Stored, what: &str) {
        let uuid = gone.kept.uuid;
        for (name, bytes) in packs.files() {
            let holds = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
            assert!(
                !holds(&gone.sealed),
                "{what}: {name} holds the sealed bytes"
            );
            assert!(
                !holds(uuid.to_string().as_bytes()),
                "{what}: {name} holds the uuid"
            );
            assert!(
                !holds(uuid.as_bytes()),
                "{what}: {name} holds the uuid's bytes"
            );
            assert!(
                !holds(gone.subject.as_bytes()),
                "{what}: {name} holds the subject"
            );
        }
    }

    /// The rows of `purge_pending` and the files marked unnamed.
    fn left_to_finish(f: &Fixture) -> (i64, i64) {
        let conn = f.store.conn_for_test();
        let pending = conn
            .query_row("SELECT count(*) FROM purge_pending", [], |r| r.get(0))
            .expect("counted");
        let unnamed = conn
            .query_row(
                "SELECT count(*) FROM parquet_file WHERE unnamed_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .expect("counted");
        (pending, unnamed)
    }

    /// 4.6, locally: once `purge_due` returns, the purged row's file is
    /// gone (its name no longer listed, HeadObject finds nothing), no file
    /// holds the purged vCon's sealed bytes, uuid or subject, and the other
    /// versions read back from the new file.
    #[test]
    fn a_purge_removes_the_purged_rows_bytes_before_it_returns() {
        let mut f = fixture();
        let (packs, stored, old) = three_packed(&mut f);
        later(&f);
        assert_eq!(f.store.purge_due(DAY).expect("swept").purged, 1, "one due");
        assert!(
            !packs.list().expect("listed").contains(&old),
            "the old file is not listed"
        );
        assert_eq!(
            packs.size(&old).expect("answers"),
            None,
            "nor found by name"
        );
        in_no_file(&packs, &stored[0], "after the purge");
        let rest: Vec<Kept> = stored[1..].iter().map(|s| s.kept.clone()).collect();
        held_once(&f, &packs, &rest, "after the purge");
        assert_eq!(left_to_finish(&f), (0, 0), "nothing is left to finish");
        assert_eq!(f.store.check().expect("checked"), [], "nothing missing");
    }

    /// A file whose every row is purged is deleted, and no new file is
    /// written for it.
    #[test]
    fn a_file_whose_every_row_is_purged_leaves_no_file() {
        let mut f = fixture();
        let one = created(&f, 1);
        let packs = with_packs(&mut f);
        f.store.pack(&PLAN()).expect("packs");
        f.store
            .delete(&f.caller, one.kept.uuid, Retention::Days(0))
            .expect("deleted and purged");
        assert_eq!(
            packs.list().expect("listed"),
            Vec::<String>::new(),
            "no file"
        );
        assert_eq!(left_to_finish(&f), (0, 0), "nothing is left to finish");
    }

    /// 4.8: a held version is copied into the new file, never left out, and
    /// a hold that commits before the purge keeps its version and bytes.
    #[test]
    fn a_held_version_is_copied_into_the_rewritten_file() {
        let mut f = fixture();
        let (packs, stored, _) = three_packed(&mut f);
        f.store
            .delete(&f.caller, stored[1].kept.uuid, Retention::Years(7))
            .expect("deleted");
        f.store
            .set_hold(&f.caller, stored[1].kept.uuid, true)
            .expect("held");
        later(&f);
        assert_eq!(
            f.store.purge_due(DAY).expect("swept").purged,
            1,
            "only the free one"
        );
        in_no_file(&packs, &stored[0], "after the purge");
        let held = &stored[1];
        let legal = f
            .store
            .legal_get(&f.caller, held.kept.uuid, Some(held.kept.version))
            .expect("the held version reads");
        assert_eq!(legal.stored.body, held.kept.body, "byte for byte");
        let files = packs.files();
        assert!(
            files
                .iter()
                .any(|(_, b)| b.windows(held.sealed.len()).any(|w| w == held.sealed)),
            "the held version's sealed bytes are in the new file"
        );
    }

    /// Where the file side of a purge is cut off.
    #[derive(Debug, Clone, Copy)]
    enum Cut {
        /// The new file's PUT fails.
        Put,
        /// The swap's commit is rolled back.
        RolledBack,
        /// The swap's commit is kept, then reported unknown.
        Unknown,
        /// Deleting the old file fails.
        Delete,
        /// The storage answers the delete but keeps the file.
        Kept,
    }

    /// 4.7: a purge cut off after its crypto-erase, at any step of its file
    /// side, fails, and the purged version cannot be read; a restart's
    /// recovery finishes the rewrite and the removal, and then 4.6 holds.
    #[test]
    fn a_purge_cut_off_after_its_crypto_erase_is_finished_by_recovery() {
        for cut in [
            Cut::Put,
            Cut::RolledBack,
            Cut::Unknown,
            Cut::Delete,
            Cut::Kept,
        ] {
            let what = format!("{cut:?}");
            let mut f = fixture();
            let (packs, stored, old) = three_packed(&mut f);
            later(&f);
            match cut {
                Cut::Put => packs.fail_puts.store(true, Ordering::SeqCst),
                // The purge's own commit is the first; the swap's the second.
                Cut::RolledBack => f.store.catalog.fail_commit(2, CommitError::RolledBack),
                Cut::Unknown => f.store.catalog.fail_commit(2, CommitError::Unknown),
                Cut::Delete => packs.fail_removes.store(true, Ordering::SeqCst),
                Cut::Kept => packs.keep_removed.store(true, Ordering::SeqCst),
            }
            assert_eq!(
                f.store.purge_due(DAY),
                Err(StoreError::Io),
                "{what}: the purge does not return as done"
            );
            assert_eq!(
                f.store
                    .legal_get(&f.caller, stored[0].kept.uuid, Some(stored[0].kept.version))
                    .map(|l| l.stored.body),
                Err(StoreError::Gone),
                "{what}: the purged version is gone at once"
            );
            for flag in [&packs.fail_puts, &packs.fail_removes, &packs.keep_removed] {
                flag.store(false, Ordering::SeqCst);
            }
            f.reopen();
            assert!(
                !packs.list().expect("listed").contains(&old),
                "{what}: old file gone"
            );
            in_no_file(&packs, &stored[0], &what);
            let rest: Vec<Kept> = stored[1..].iter().map(|s| s.kept.clone()).collect();
            held_once(&f, &packs, &rest, &format!("{what}, recovered"));
            assert_eq!(
                left_to_finish(&f),
                (0, 0),
                "{what}: nothing is left to finish"
            );
        }
    }

    /// A rewrite that copied a file whose rows changed before its swap (a
    /// second purge rewrote it first) is refused, and its new file removed;
    /// the second purge's own rewrite stands.
    #[test]
    fn a_rewrite_whose_file_changed_after_its_copy_is_refused() {
        let mut f = fixture();
        let (packs, stored, _) = three_packed(&mut f);
        let file = {
            let conn = f.store.conn_for_test();
            conn.query_row("SELECT file_id FROM parquet_file", [], |r| r.get(0))
                .map(FileId)
                .expect("the file")
        };
        let copied = f
            .store
            .copy_file(file)
            .expect("copied")
            .expect("rows to copy");
        let mid = packs.files().len();
        assert_eq!(mid, 2, "the old file and the copy");
        later(&f);
        f.store.purge_due(DAY).expect("swept");
        assert_eq!(f.store.swap(&copied), Ok(false), "the copy is out of date");
        in_no_file(&packs, &stored[0], "after both");
        let rest: Vec<Kept> = stored[1..].iter().map(|s| s.kept.clone()).collect();
        held_once(&f, &packs, &rest, "after both");
    }

    /// The export, which reads beside a server from its own snapshot, finds
    /// a version whose file a purge rewrote and deleted at its new place.
    #[test]
    fn the_export_reads_a_version_whose_file_was_rewritten_under_it() {
        let mut f = fixture();
        let (_, stored, _) = three_packed(&mut f);
        let listed = f.store.kept_versions(None, None).expect("listed");
        later(&f);
        f.store.purge_due(DAY).expect("swept");
        let kept = listed
            .iter()
            .find(|k| k.version == stored[2].kept.version)
            .expect("listed before the purge");
        assert_eq!(
            f.store.sealed_of(kept).expect("reads").expect("kept"),
            stored[2].sealed,
            "byte for byte, from the new file"
        );
        let purged = listed
            .iter()
            .find(|k| k.version == stored[0].kept.version)
            .expect("listed before the purge");
        assert_eq!(
            f.store.sealed_of(purged),
            Ok(None),
            "the purged one is gone"
        );
    }

    /// A read that found its version's place, then its file rewritten and
    /// deleted before it read the row, reads again at the new place and
    /// returns the whole body.
    #[test]
    fn a_read_racing_a_purge_rewrite_returns_the_whole_body() {
        let mut f = fixture();
        let (packs, stored, _) = three_packed(&mut f);
        later(&f);
        let moved: BTreeSet<String> = packs.list().expect("listed").into_iter().collect();
        // The purge runs in the gap between the read's catalog transaction
        // and its read of the row.
        let store = &f.store;
        let read = std::thread::scope(|scope| {
            let (go, wait) = std::sync::mpsc::channel::<()>();
            let (done, finished) = std::sync::mpsc::channel::<()>();
            *store.hooks.before_read.lock().expect("hook") = Some(Box::new(move || {
                go.send(()).expect("the purge is told");
                finished.recv().expect("the purge finishes");
            }));
            let purge = scope.spawn(move || {
                wait.recv().expect("the read is between its steps");
                let swept = store.purge_due(DAY).expect("swept");
                done.send(()).expect("the read is told");
                swept
            });
            let read = store.get(&f.caller, stored[1].kept.uuid);
            assert_eq!(purge.join().expect("purged").purged, 1, "the purge ran");
            read
        });
        assert_eq!(
            read.map(|s| s.body),
            Ok(stored[1].kept.body.clone()),
            "the whole body"
        );
        let now: BTreeSet<String> = packs.list().expect("listed").into_iter().collect();
        assert!(
            moved.is_disjoint(&now),
            "the file was rewritten under the read"
        );
        assert!(
            !now.iter().any(|n| pack::parse_name(n).is_none()),
            "only pack files: {now:?}"
        );
    }
}
