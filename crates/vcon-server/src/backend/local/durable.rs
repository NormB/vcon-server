//! Which of a tenant catalog's commits are durable, and the one sync that
//! makes them so (a group sync).
//!
//! The catalog's connection runs with `synchronous = NORMAL`: COMMIT
//! writes the transaction's frames to the WAL without syncing it, and the
//! connection is let go. The commit then waits here until one fdatasync of
//! the WAL, begun after its frames were written, has returned; one thread
//! at a time syncs, for every commit written so far, and wakes each one it
//! covers. A transaction that only read waits, at its end, until every
//! commit it may have seen is durable, so nothing it answers can be lost.
//! A failed sync poisons the catalog: that wait and every later one, and
//! every later transaction, fail until the catalog is opened again.

use std::sync::{Condvar, Mutex, MutexGuard};

use crate::fs::SyncHandle;

/// The catalog had a failed sync: what was written since the last good one
/// may or may not be on disk, until the catalog is opened again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Poisoned;

/// The commits of one catalog file and the syncs of its WAL.
pub(crate) struct Durable {
    wal: Box<dyn SyncHandle>,
    state: Mutex<State>,
    turn: Condvar,
}

#[derive(Default)]
struct State {
    /// Commits numbered so far.
    begun: u64,
    /// The highest commit whose COMMIT has returned: its frames are in
    /// the WAL.
    written: u64,
    /// The highest commit a returned sync covers.
    synced: u64,
    /// Syncs started so far; the one running, if any, is the last.
    started: u64,
    /// The newest sync that has returned.
    finished: u64,
    syncing: bool,
    poisoned: bool,
    /// The file's data_version at the last `sync_seen` sync.
    beside_version: Option<i64>,
    /// Threads waiting now, for tests.
    waiting: usize,
}

impl Durable {
    pub(crate) fn new(wal: Box<dyn SyncHandle>) -> Durable {
        Durable {
            wal,
            state: Mutex::new(State::default()),
            turn: Condvar::new(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("no thread panicked holding the durable state")
    }

    /// Fails once a sync has failed.
    pub(crate) fn check(&self) -> Result<(), Poisoned> {
        if self.state().poisoned {
            return Err(Poisoned);
        }
        Ok(())
    }

    /// Numbers a commit; called holding the connection, before COMMIT.
    pub(crate) fn begin_commit(&self) -> Result<u64, Poisoned> {
        let mut state = self.state();
        if state.poisoned {
            return Err(Poisoned);
        }
        state.begun += 1;
        Ok(state.begun)
    }

    /// Commit `n`'s COMMIT has returned; called holding the connection.
    pub(crate) fn written(&self, n: u64) {
        let mut state = self.state();
        state.written = state.written.max(n);
    }

    /// The commits a transaction ending now may have seen; called holding
    /// the connection, so no commit is between its COMMIT and `written`.
    pub(crate) fn seen(&self) -> u64 {
        self.state().written
    }

    /// Returns once commit `n` and every one before it is durable.
    pub(crate) fn wait(&self, n: u64) -> Result<(), Poisoned> {
        self.until(|state| state.synced >= n)
    }

    /// Returns once a sync that began after this call has returned: for a
    /// catalog opened beside a server, which cannot count the server's
    /// commits.
    pub(crate) fn sync_now(&self) -> Result<(), Poisoned> {
        // The sync running now, if any, is number `started`: it may have
        // begun before what the caller read was written.
        let want = self.state().started + 1;
        self.until(|state| state.finished >= want)
    }

    /// `sync_now`, unless the last one was made at the same `version` of
    /// the file (SQLite's data_version, which changes when another
    /// connection commits): that sync covered everything since. None
    /// always syncs.
    pub(crate) fn sync_seen(&self, version: Option<i64>) -> Result<(), Poisoned> {
        {
            let state = self.state();
            if state.poisoned {
                return Err(Poisoned);
            }
            if version.is_some() && state.beside_version == version {
                return Ok(());
            }
        }
        self.sync_now()?;
        self.state().beside_version = version;
        Ok(())
    }

    /// Waits until `done`, syncing when no other thread is: one sync at a
    /// time, each for every commit written when it starts.
    fn until(&self, done: impl Fn(&State) -> bool) -> Result<(), Poisoned> {
        let mut state = self.state();
        loop {
            if state.poisoned {
                return Err(Poisoned);
            }
            if done(&state) {
                return Ok(());
            }
            state.waiting += 1;
            if state.syncing {
                state = self
                    .turn
                    .wait(state)
                    .expect("no thread panicked holding the durable state");
                state.waiting -= 1;
                continue;
            }
            state.syncing = true;
            state.started += 1;
            let (number, covers) = (state.started, state.written);
            drop(state);
            let synced = self.wal.sync_data();
            state = self.state();
            state.waiting -= 1;
            state.syncing = false;
            match synced {
                Ok(()) => {
                    state.synced = state.synced.max(covers);
                    state.finished = number;
                }
                Err(_) => state.poisoned = true,
            }
            self.turn.notify_all();
        }
    }

    /// The threads waiting for a sync now.
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> usize {
        self.state().waiting
    }

    /// Waits until `n` threads wait for a sync; panics after 20 s.
    #[cfg(test)]
    pub(crate) fn await_waiting(&self, n: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while self.waiting() < n {
            assert!(
                std::time::Instant::now() < deadline,
                "{n} waiters within 20 s"
            );
            std::thread::yield_now();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::fs::{CrashFs, Fs};

    /// A tracker over a WAL on disk whose syncs the crash filesystem
    /// holds, fails and counts.
    struct Rig {
        _root: tempfile::TempDir,
        fs: Arc<CrashFs>,
        wal: PathBuf,
        durable: Durable,
    }

    fn rig() -> Rig {
        let root = tempfile::tempdir().expect("temp dir");
        let wal = root.path().join("catalog.sqlite-wal");
        std::fs::write(&wal, b"frames").expect("the WAL on disk");
        let fs = Arc::new(CrashFs::new(root.path()));
        let durable = Durable::new(fs.open_for_sync(&wal).expect("opens"));
        Rig {
            _root: root,
            fs,
            wal,
            durable,
        }
    }

    impl Rig {
        fn syncs(&self) -> usize {
            self.fs.syncs(&self.wal)
        }

        /// A commit numbered and written, as LocalTx's commit does it.
        fn commit(&self) -> u64 {
            let n = self.durable.begin_commit().expect("not poisoned");
            self.durable.written(n);
            n
        }

        fn await_waiting(&self, n: usize) {
            self.durable.await_waiting(n);
        }
    }

    #[test]
    fn a_wait_returns_once_a_sync_covers_the_commit() {
        let r = rig();
        let n = r.commit();
        r.durable.wait(n).expect("durable");
        assert_eq!(r.syncs(), 1, "one fdatasync made commit {n} durable");
    }

    #[test]
    fn commits_are_numbered_in_order_from_one() {
        let r = rig();
        let numbers: Vec<u64> = (0..3).map(|_| r.commit()).collect();
        assert_eq!(numbers, [1, 2, 3], "one number per commit, in order");
    }

    #[test]
    fn seen_counts_every_commit_written() {
        let r = rig();
        assert_eq!(r.durable.seen(), 0, "nothing written yet");
        r.commit();
        r.commit();
        assert_eq!(
            r.durable.seen(),
            2,
            "a transaction ending now may have seen both"
        );
    }

    #[test]
    fn one_sync_covers_every_commit_written_before_it_starts() {
        let r = rig();
        let last = (0..3).map(|_| r.commit()).last().expect("three commits");
        r.durable.wait(last).expect("durable");
        r.durable.wait(1).expect("covered already");
        r.durable.wait(2).expect("covered already");
        assert_eq!(r.syncs(), 1, "one sync for three commits");
    }

    #[test]
    fn a_wait_for_no_commit_needs_no_sync() {
        let r = rig();
        r.durable.wait(0).expect("nothing to wait for");
        assert_eq!(
            r.syncs(),
            0,
            "a transaction that saw no commit syncs nothing"
        );
    }

    #[test]
    fn a_wait_for_a_commit_already_synced_does_not_sync_again() {
        let r = rig();
        let n = r.commit();
        r.durable.wait(n).expect("durable");
        r.durable.wait(n).expect("still durable");
        assert_eq!(r.syncs(), 1, "the second wait found it covered");
    }

    #[test]
    fn a_wait_does_not_return_while_its_sync_is_held() {
        let r = rig();
        let n = r.commit();
        r.fs.hold_syncs();
        std::thread::scope(|s| {
            let waiting = s.spawn(|| r.durable.wait(n));
            r.fs.await_held(1);
            assert!(!waiting.is_finished(), "no return before the sync");
            r.fs.release_syncs();
            waiting
                .join()
                .expect("the waiter ends")
                .expect("durable after the release");
        });
        assert_eq!(r.syncs(), 1, "one sync");
    }

    #[test]
    fn a_commit_written_while_a_sync_runs_needs_a_sync_of_its_own() {
        let r = rig();
        let first = r.commit();
        r.fs.hold_syncs();
        std::thread::scope(|s| {
            let a = s.spawn(|| r.durable.wait(first));
            r.fs.await_held(1);
            let second = r.commit();
            let durable = &r.durable;
            let b = s.spawn(move || durable.wait(second));
            r.await_waiting(2);
            r.fs.release_syncs();
            a.join().expect("a ends").expect("first durable");
            b.join().expect("b ends").expect("second durable");
        });
        assert_eq!(
            r.syncs(),
            2,
            "the running sync began before the second commit was written, so it cannot cover it"
        );
    }

    #[test]
    fn waiters_behind_a_running_sync_share_the_next_one() {
        let r = rig();
        let first = r.commit();
        r.fs.hold_syncs();
        std::thread::scope(|s| {
            let a = s.spawn(|| r.durable.wait(first));
            r.fs.await_held(1);
            let (second, third) = (r.commit(), r.commit());
            let durable = &r.durable;
            let b = s.spawn(move || durable.wait(second));
            let c = s.spawn(move || durable.wait(third));
            r.await_waiting(3);
            r.fs.release_syncs();
            for (name, t) in [("a", a), ("b", b), ("c", c)] {
                t.join()
                    .unwrap_or_else(|_| panic!("{name} ends"))
                    .unwrap_or_else(|_| panic!("{name} durable"));
            }
        });
        assert_eq!(r.syncs(), 2, "the second and third commits share one sync");
    }

    #[test]
    fn many_committers_all_return_with_fewer_syncs_than_commits_at_most_one_each() {
        let r = rig();
        std::thread::scope(|s| {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        for _ in 0..25 {
                            let n = r.commit();
                            r.durable.wait(n).expect("durable");
                        }
                    })
                })
                .collect();
            for t in threads {
                t.join().expect("a committer ends");
            }
        });
        assert!(r.syncs() >= 1, "at least one sync");
        assert!(
            r.syncs() <= 200,
            "never more syncs than commits: {}",
            r.syncs()
        );
        assert_eq!(r.durable.seen(), 200, "every commit written");
    }

    #[test]
    fn a_failed_sync_fails_the_wait() {
        let r = rig();
        let n = r.commit();
        r.fs.fail_syncs();
        assert_eq!(
            r.durable.wait(n),
            Err(Poisoned),
            "the commit may not be on disk"
        );
    }

    #[test]
    fn after_a_failed_sync_every_later_wait_fails_even_for_a_synced_commit() {
        let r = rig();
        let first = r.commit();
        r.durable.wait(first).expect("durable");
        let second = r.commit();
        r.fs.fail_syncs();
        assert_eq!(r.durable.wait(second), Err(Poisoned), "the failed one");
        assert_eq!(
            r.durable.wait(first),
            Err(Poisoned),
            "poisoned: every wait fails"
        );
        assert_eq!(
            r.durable.wait(0),
            Err(Poisoned),
            "even one that needs no sync"
        );
    }

    #[test]
    fn after_a_failed_sync_no_transaction_begins_or_commits() {
        let r = rig();
        let n = r.commit();
        r.fs.fail_syncs();
        assert!(r.durable.wait(n).is_err(), "the sync failed");
        assert_eq!(r.durable.check(), Err(Poisoned), "no transaction begins");
        assert_eq!(
            r.durable.begin_commit(),
            Err(Poisoned),
            "no commit is numbered"
        );
    }

    #[test]
    fn a_failed_sync_is_not_retried_by_a_later_wait() {
        let r = rig();
        let n = r.commit();
        r.fs.fail_syncs();
        assert!(r.durable.wait(n).is_err(), "the sync failed");
        // Syncs work again, but a later sync could report success for
        // pages the failed writeback dropped: poison stays.
        r.fs.heal_syncs();
        assert_eq!(r.durable.wait(n), Err(Poisoned), "still poisoned");
        assert_eq!(r.syncs(), 0, "and no sync was tried again");
    }

    #[test]
    fn every_waiter_behind_a_failing_sync_fails() {
        let r = rig();
        let n = r.commit();
        r.fs.hold_syncs();
        std::thread::scope(|s| {
            let a = s.spawn(|| r.durable.wait(n));
            r.fs.await_held(1);
            let b = s.spawn(|| r.durable.wait(n));
            r.await_waiting(2);
            r.fs.fail_syncs();
            assert_eq!(
                a.join().expect("a ends"),
                Err(Poisoned),
                "the syncing waiter"
            );
            assert_eq!(
                b.join().expect("b ends"),
                Err(Poisoned),
                "the one behind it"
            );
        });
    }

    #[test]
    fn sync_now_syncs_even_when_no_commit_was_counted() {
        let r = rig();
        r.durable.sync_now().expect("synced");
        r.durable.sync_now().expect("synced again");
        assert_eq!(
            r.syncs(),
            2,
            "beside a server, the server's commits are not counted here: each call syncs"
        );
    }

    #[test]
    fn sync_seen_syncs_at_a_version_it_has_not_synced() {
        let r = rig();
        r.durable.sync_seen(Some(1)).expect("synced");
        assert_eq!(r.syncs(), 1, "a version not seen before: one sync");
    }

    #[test]
    fn sync_seen_skips_the_sync_at_the_version_of_its_last_one() {
        let r = rig();
        r.durable.sync_seen(Some(1)).expect("synced");
        r.durable.sync_seen(Some(1)).expect("nothing to sync");
        assert_eq!(r.syncs(), 1, "no other connection committed since: no sync");
    }

    #[test]
    fn sync_seen_syncs_again_at_a_new_version() {
        let r = rig();
        r.durable.sync_seen(Some(1)).expect("synced");
        r.durable.sync_seen(Some(2)).expect("synced again");
        assert_eq!(r.syncs(), 2, "another connection committed: a sync");
    }

    #[test]
    fn sync_seen_always_syncs_when_the_version_is_unknown() {
        let r = rig();
        r.durable.sync_seen(None).expect("synced");
        r.durable.sync_seen(None).expect("synced again");
        assert_eq!(
            r.syncs(),
            2,
            "a data_version that could not be read tells nothing: each call syncs"
        );
    }

    #[test]
    fn an_unknown_version_after_a_known_one_syncs() {
        let r = rig();
        r.durable.sync_seen(Some(1)).expect("synced");
        r.durable.sync_seen(None).expect("synced again");
        assert_eq!(r.syncs(), 2, "an unknown version is never the last one");
    }

    #[test]
    fn sync_seen_fails_once_poisoned_even_at_the_same_version() {
        let r = rig();
        r.fs.fail_syncs();
        assert_eq!(
            r.durable.sync_seen(Some(1)),
            Err(Poisoned),
            "the sync failed"
        );
        r.fs.heal_syncs();
        assert_eq!(
            r.durable.sync_seen(Some(1)),
            Err(Poisoned),
            "poisoned until the catalog is opened again"
        );
        assert_eq!(r.syncs(), 0, "no sync succeeded");
    }

    #[test]
    fn sync_now_waits_for_a_sync_that_starts_after_it() {
        let r = rig();
        let n = r.commit();
        r.fs.hold_syncs();
        std::thread::scope(|s| {
            let a = s.spawn(|| r.durable.wait(n));
            r.fs.await_held(1);
            let b = s.spawn(|| r.durable.sync_now());
            r.await_waiting(2);
            r.fs.release_syncs();
            a.join().expect("a ends").expect("durable");
            b.join().expect("b ends").expect("synced");
        });
        assert_eq!(
            r.syncs(),
            2,
            "the sync running when sync_now was called may have begun before what it read"
        );
    }

    #[test]
    fn sync_now_fails_once_poisoned() {
        let r = rig();
        r.fs.fail_syncs();
        assert_eq!(r.durable.sync_now(), Err(Poisoned), "the sync failed");
        r.fs.heal_syncs();
        assert_eq!(r.durable.sync_now(), Err(Poisoned), "and poison stays");
    }
}
