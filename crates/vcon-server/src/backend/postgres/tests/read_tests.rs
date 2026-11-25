//! A tenant's reads on a PostgreSQL catalog while writes of the same tenant
//! run. A read's first step reads in a snapshot and takes no lock on the
//! tenant; its last step takes the tenant's lock, checks the version again
//! and records the read or its refusal. These tests hold that no read is
//! answered from a state that a write changed before the read's last
//! step: the guarantee the lock in the first step used to give. The live
//! tests need a PostgreSQL server.

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use uuid::Uuid;

    use super::super::super::catalog::{Database, PgCatalog};
    use super::super::catalog_tests::on_a_server::database;
    use super::super::live::value;
    use super::super::schema_tests::live::{Db, count, waiters};
    use crate::backend::conformance::{START, TestClock, vcon};
    use crate::backend::local::LocalObjects;
    use crate::backend::{Catalog, CatalogTx};
    use crate::clock::nanos;
    use crate::envelope::{self, Key};
    use crate::fs::OsFs;
    use crate::ids::{KeyId, TenantId, VersionId};
    use crate::retention::Retention;
    use crate::store::{Caller, Created, Store, StoreError};

    const A: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bca1";
    const B: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bcb2";

    /// How long a step a test waits for may take.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// A tenant's Store on a database of the test's own, its objects in a
    /// directory.
    struct Tenant {
        store: Store,
        caller: Caller,
        catalog_key: Key,
        tenant: TenantId,
        database: Arc<Database>,
        _objects: tempfile::TempDir,
    }

    /// The test's database and a tenant on it. The test holds the
    /// database itself, bound before the tenant (`let (_db, t) =
    /// tenant();`), so it is dropped when the test ends, pass or fail,
    /// after the test's own handle of the tenant: a read thread may still
    /// hold the tenant then, and nextest ends the process once the test
    /// returns, so a database the tenant held would never be dropped.
    fn tenant() -> (Db, Arc<Tenant>) {
        let (db, database) = database();
        let tenant = TenantId::generate();
        let key = Key::generate().expect("a tenant key");
        let catalog_key = envelope::catalog_key(&key).expect("the catalog key");
        let catalog =
            PgCatalog::create(database.clone(), tenant, &catalog_key).expect("the catalog");
        let objects = tempfile::tempdir().expect("a directory");
        let store = Store::open_on(
            tenant,
            key,
            Box::new(catalog),
            Arc::new(LocalObjects::new(objects.path().to_owned(), Arc::new(OsFs))),
            None,
            Arc::new(OsFs),
            TestClock::at(START),
            Some(crate::store::pending::Writers::None),
        )
        .expect("opens");
        let t = Arc::new(Tenant {
            store,
            caller: Caller {
                key: Key::generate().expect("a key").id(),
            },
            catalog_key,
            tenant,
            database,
            _objects: objects,
        });
        (db, t)
    }

    /// A second catalog of the tenant, as another server would open it.
    fn other(t: &Tenant) -> PgCatalog {
        PgCatalog::open(t.database.clone(), t.tenant, &t.catalog_key).expect("a second catalog")
    }

    fn stored(t: &Tenant, uuid: &str, subject: &str) -> (VersionId, Vec<u8>) {
        let (found, body) = vcon(uuid, subject);
        let Ok(Created::New(version)) = t.store.create(&t.caller, &found, &body) else {
            panic!("a new vCon")
        };
        (version, body)
    }

    fn uuid(s: &str) -> Uuid {
        Uuid::parse_str(s).expect("a uuid")
    }

    /// The read records, as (target, status), in the log's order.
    fn reads(t: &Tenant) -> Vec<(serde_json::Value, serde_json::Value)> {
        let mut out = Vec::new();
        other(t).export(0, &mut out).expect("exports");
        String::from_utf8(out)
            .expect("text")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("a JSON line"))
            .filter(|r| r["action"] == "read")
            .map(|r| (r["target"].clone(), r["status"].clone()))
            .collect()
    }

    /// One read record, as `reads` gives it.
    fn rec(target: &str, status: u16) -> (serde_json::Value, serde_json::Value) {
        (target.into(), status.into())
    }

    /// A read on a thread of its own: its answer, and the thread, which
    /// `answer` joins.
    struct Reading {
        answered: mpsc::Receiver<Result<Vec<u8>, StoreError>>,
        thread: std::thread::JoinHandle<()>,
    }

    /// `t.store.get` of `uuid` on a thread of its own.
    fn read(t: &Arc<Tenant>, uuid: Uuid) -> Reading {
        let (answer, answered) = mpsc::channel();
        let t = t.clone();
        let thread = std::thread::spawn(move || {
            let _ = answer.send(t.store.get(&t.caller, uuid).map(|s| s.body));
        });
        Reading { answered, thread }
    }

    /// A signal each time a read has ended its first step and is about to
    /// take its last.
    fn at_last_step(t: &Tenant) -> mpsc::Receiver<()> {
        let (reached, at) = mpsc::channel();
        let reached = Mutex::new(reached);
        t.store.on_last_step(Some(Arc::new(move || {
            let _ = reached.lock().expect("sender").send(());
        })));
        at
    }

    /// Runs `write` once, in the first read that reaches its last step,
    /// before that step.
    fn once_before_last_step(t: &Arc<Tenant>, write: impl FnOnce(&Tenant) + Send + 'static) {
        let once = Mutex::new(Some((Arc::downgrade(t), write)));
        t.store.on_last_step(Some(Arc::new(move || {
            if let Some((t, write)) = once.lock().expect("once").take() {
                write(&t.upgrade().expect("the tenant"));
            }
        })));
    }

    /// Whether some session of the test's database waits for a lock,
    /// asked up to fifty times a tenth of a second apart.
    fn a_session_waits(db: &Db) -> bool {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async {
            let admin = db.admin().await;
            for _ in 0..50 {
                if value(&admin, &waiters(None)).await == "1" {
                    return true;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            false
        })
    }

    /// A write of the tenant, begun on another catalog, that holds the
    /// tenant's lock until it is committed or dropped.
    fn held_write(catalog: &PgCatalog) -> Box<dyn CatalogTx + '_> {
        let mut tx = catalog.immediate().expect("the write begins");
        assert_eq!(
            tx.wraps(KeyId([1; 8])),
            Ok(0),
            "its first statement takes the tenant's lock"
        );
        tx
    }

    /// Starts a read of `uuid` while `write` holds the tenant's lock, and
    /// checks that its first step ends anyway and that its last step then
    /// waits for the lock; gives back the read's answer channel.
    fn read_under(t: &Arc<Tenant>, db: &Db, uuid: Uuid) -> Reading {
        let reached = at_last_step(t);
        let reading = read(t, uuid);
        assert!(
            reached.recv_timeout(PATIENCE).is_ok(),
            "the read's first step ended while another catalog held the tenant's lock"
        );
        assert!(a_session_waits(db), "its last step waits for the lock");
        assert!(
            reading.answered.try_recv().is_err(),
            "and the read has no answer while the write holds the lock"
        );
        t.store.on_last_step(None);
        reading
    }

    /// The read's answer, once its thread has ended: a test process that
    /// exits while such a thread is ending (dropping its clone of the
    /// tenant, freeing OpenSSL's per-thread state) can end with SIGSEGV,
    /// as store::group_tests did under coverage (CI, #272).
    fn answer(reading: Reading) -> Result<Vec<u8>, StoreError> {
        let got = reading
            .answered
            .recv_timeout(PATIENCE)
            .unwrap_or_else(|_| panic!("the read had no answer within {PATIENCE:?}"));
        reading.thread.join().expect("the reading thread ends");
        got
    }

    /// A read's first step takes no lock on the tenant: it ends while a
    /// write holds the lock, and only its last step waits for that write.
    #[test]
    fn a_reads_first_step_does_not_wait_for_a_write_of_the_tenant() {
        let (db, t) = tenant();
        let (_, body) = stored(&t, A, "first");
        let catalog = other(&t);
        let write = held_write(&catalog);
        let answered = read_under(&t, &db, uuid(A));
        write.commit().expect("the write commits");
        assert_eq!(answer(answered), Ok(body), "the read is answered after");
        assert_eq!(reads(&t), [rec(A, 200)], "and recorded once, as answered");
    }

    /// The first step saw the vCon live, but a write that held the lock at
    /// that time deleted it: the read is refused as gone, as a read that
    /// waited for the write in its first step was.
    #[test]
    fn a_vcon_a_held_write_deletes_is_refused_as_gone() {
        let (db, t) = tenant();
        stored(&t, A, "first");
        let catalog = other(&t);
        let mut write = held_write(&catalog);
        write
            .mark_deleted(uuid(A), nanos(START))
            .expect("the write deletes it");
        let answered = read_under(&t, &db, uuid(A));
        write.commit().expect("the delete commits");
        assert_eq!(answer(answered), Err(StoreError::Gone), "refused as gone");
        assert_eq!(reads(&t), [rec(A, 410)], "the refusal is recorded once");
    }

    /// The same write rolled back changes nothing: the read is answered
    /// with the body.
    #[test]
    fn a_held_write_that_is_rolled_back_leaves_the_read_answered() {
        let (db, t) = tenant();
        let (_, body) = stored(&t, A, "first");
        let catalog = other(&t);
        let mut write = held_write(&catalog);
        write
            .mark_deleted(uuid(A), nanos(START))
            .expect("the write deletes it");
        let answered = read_under(&t, &db, uuid(A));
        drop(write);
        assert_eq!(answer(answered), Ok(body), "the body");
        assert_eq!(reads(&t), [rec(A, 200)], "recorded once");
    }

    /// A uuid no vCon has, read while a write holds the lock: refused as
    /// not found, and recorded once.
    #[test]
    fn an_unknown_uuid_read_under_a_held_write_is_not_found() {
        let (db, t) = tenant();
        let catalog = other(&t);
        let write = held_write(&catalog);
        let answered = read_under(&t, &db, uuid(B));
        write.commit().expect("the write commits");
        assert_eq!(answer(answered), Err(StoreError::NotFound), "not found");
        assert_eq!(reads(&t), [rec(B, 404)], "recorded once");
    }

    /// A replace committed between a read's steps: the read never answers
    /// the version it read first; it reads again and answers the new one.
    #[test]
    fn a_replace_between_the_steps_is_read_at_the_new_version() {
        let (_db, t) = tenant();
        let (first, old) = stored(&t, A, "first");
        let (found, new) = vcon(A, "second");
        once_before_last_step(&t, move |t| {
            t.store
                .replace(&t.caller, first, &found, &new)
                .expect("replaced meanwhile");
        });
        let answered = answer(read(&t, uuid(A)));
        t.store.on_last_step(None);
        let (_, want) = vcon(A, "second");
        assert_ne!(answered, Ok(old), "never the replaced version");
        assert_eq!(answered, Ok(want), "the new version");
        assert_eq!(reads(&t), [rec(A, 200)], "recorded once");
    }

    /// A delete committed between a read's steps: refused as gone, and
    /// recorded once.
    #[test]
    fn a_delete_between_the_steps_is_refused_as_gone() {
        let (_db, t) = tenant();
        stored(&t, A, "first");
        once_before_last_step(&t, |t| {
            t.store
                .delete(&t.caller, uuid(A), Retention::Years(7))
                .expect("deleted meanwhile");
        });
        let answered = answer(read(&t, uuid(A)));
        t.store.on_last_step(None);
        assert_eq!(answered, Err(StoreError::Gone), "refused as gone");
        assert_eq!(reads(&t), [rec(A, 410)], "recorded once");
    }

    /// A vCon stored between the steps of a read that found none: the read
    /// finds it in its last step and reads it.
    #[test]
    fn a_vcon_stored_between_the_steps_is_read() {
        let (_db, t) = tenant();
        let (found, body) = vcon(B, "late");
        once_before_last_step(&t, move |t| {
            t.store
                .create(&t.caller, &found, &body)
                .expect("stored meanwhile");
        });
        let answered = answer(read(&t, uuid(B)));
        t.store.on_last_step(None);
        let (_, want) = vcon(B, "late");
        assert_eq!(answered, Ok(want), "the new vCon");
        assert_eq!(reads(&t), [rec(B, 200)], "recorded once");
    }

    // A write committed at each point of a read: the read answers the
    // state after the write, whole, and never the version the write
    // replaced, deleted or purged. Each write is committed before the
    // read's last step, so the old state is never a right answer.

    /// Where in a read a write is made.
    #[derive(Clone, Copy, Debug)]
    enum Gap {
        /// In the first step, between the read of the vCon's current
        /// version and the read of that version's row.
        InFirstStep,
        /// After the first step, before the version's body is opened.
        BeforeOpen,
        /// After the body is read, before the last step.
        BeforeLastStep,
    }

    /// Reads `uuid` once, with `write` made once at `gap` of the read, and
    /// gives back the read's answer.
    fn read_with_write_at(
        t: &Arc<Tenant>,
        gap: Gap,
        uuid: Uuid,
        write: impl FnOnce(&Tenant) + Send + 'static,
    ) -> Result<Vec<u8>, StoreError> {
        let once = Mutex::new(Some((Arc::downgrade(t), write)));
        let run = move || {
            if let Some((t, write)) = once.lock().expect("once").take() {
                write(&t.upgrade().expect("the tenant"));
            }
        };
        match gap {
            Gap::InFirstStep => t.store.in_first_step(Some(Arc::new(run))),
            Gap::BeforeOpen => t.store.once_before_open(Box::new(run)),
            Gap::BeforeLastStep => t.store.on_last_step(Some(Arc::new(run))),
        }
        let answered = answer(read(t, uuid));
        t.store.in_first_step(None);
        t.store.on_last_step(None);
        answered
    }

    /// The version of A stored first is replaced at `gap`; with `purge`,
    /// the replaced version is purged too, its object removed after the
    /// purge commits.
    fn replaced_at(gap: Gap, purge: bool) {
        let (_db, t) = tenant();
        let (first, old) = stored(&t, A, "first");
        let (found, new) = vcon(A, "second");
        let answered = read_with_write_at(&t, gap, uuid(A), move |t| {
            t.store
                .replace(&t.caller, first, &found, &new)
                .expect("replaced meanwhile");
            if purge {
                let report = t.store.purge_due(Retention::Days(0)).expect("purges");
                assert_eq!(report.purged, 1, "the replaced version is purged");
            }
        });
        let (_, want) = vcon(A, "second");
        assert_ne!(answered, Ok(old), "never the replaced version ({gap:?})");
        assert_eq!(answered, Ok(want), "the whole new version ({gap:?})");
        assert_eq!(reads(&t), [rec(A, 200)], "recorded once ({gap:?})");
    }

    /// A is deleted at `gap` and kept for `retention`: refused as gone.
    fn deleted_at(gap: Gap, retention: Retention) {
        let (_db, t) = tenant();
        stored(&t, A, "first");
        let answered = read_with_write_at(&t, gap, uuid(A), move |t| {
            t.store
                .delete(&t.caller, uuid(A), retention)
                .expect("deleted meanwhile");
        });
        assert_eq!(answered, Err(StoreError::Gone), "refused as gone ({gap:?})");
        assert_eq!(reads(&t), [rec(A, 410)], "recorded once ({gap:?})");
    }

    /// The tenant is removed at `gap`, as a removal on another server
    /// does it: its objects first, then its rows, the lock row last. The
    /// read is refused: its last step finds no tenant to lock.
    fn tenant_removed_at(gap: Gap) {
        let (_db, t) = tenant();
        let (_, old) = stored(&t, A, "first");
        let answered = read_with_write_at(&t, gap, uuid(A), |t| {
            for entry in std::fs::read_dir(t._objects.path()).expect("the objects") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    std::fs::remove_dir_all(&path).expect("removed");
                } else {
                    std::fs::remove_file(&path).expect("removed");
                }
            }
            PgCatalog::remove_in(&t.database, t.tenant, 1000).expect("the tenant is removed");
        });
        assert_ne!(
            answered,
            Ok(old),
            "never the removed tenant's version ({gap:?})"
        );
        assert_eq!(
            answered,
            Err(StoreError::Io),
            "refused: no tenant ({gap:?})"
        );
    }

    #[test]
    fn a_replace_in_the_first_step_is_read_at_the_new_version() {
        replaced_at(Gap::InFirstStep, false);
    }

    #[test]
    fn a_replace_before_the_open_is_read_at_the_new_version() {
        replaced_at(Gap::BeforeOpen, false);
    }

    #[test]
    fn a_replace_and_purge_in_the_first_step_is_read_at_the_new_version() {
        replaced_at(Gap::InFirstStep, true);
    }

    #[test]
    fn a_replace_and_purge_before_the_open_is_read_at_the_new_version() {
        replaced_at(Gap::BeforeOpen, true);
    }

    #[test]
    fn a_replace_and_purge_before_the_last_step_is_read_at_the_new_version() {
        replaced_at(Gap::BeforeLastStep, true);
    }

    #[test]
    fn a_delete_in_the_first_step_is_refused_as_gone() {
        deleted_at(Gap::InFirstStep, Retention::Years(7));
    }

    #[test]
    fn a_delete_before_the_open_is_refused_as_gone() {
        deleted_at(Gap::BeforeOpen, Retention::Years(7));
    }

    #[test]
    fn a_delete_and_purge_in_the_first_step_is_refused_as_gone() {
        deleted_at(Gap::InFirstStep, Retention::Days(0));
    }

    #[test]
    fn a_delete_and_purge_before_the_open_is_refused_as_gone() {
        deleted_at(Gap::BeforeOpen, Retention::Days(0));
    }

    #[test]
    fn a_delete_and_purge_before_the_last_step_is_refused_as_gone() {
        deleted_at(Gap::BeforeLastStep, Retention::Days(0));
    }

    #[test]
    fn a_tenant_removed_in_the_first_step_is_refused() {
        tenant_removed_at(Gap::InFirstStep);
    }

    #[test]
    fn a_tenant_removed_before_the_open_is_refused() {
        tenant_removed_at(Gap::BeforeOpen);
    }

    #[test]
    fn a_tenant_removed_before_the_last_step_is_refused() {
        tenant_removed_at(Gap::BeforeLastStep);
    }

    // Reads that race writes on other threads, with no step held: each
    // answer is one whole version or the refusal of the state after a
    // write, and a version purged before a read began is never its
    // answer.

    /// How many reads each racing reader makes at least.
    const RACE_READS: usize = 40;

    /// The subject a racing writer gives the `n`th version.
    fn subject(n: usize) -> String {
        format!("v{n}")
    }

    /// Which version of A a body is, if it is one of the bodies a racing
    /// writer stored, byte for byte.
    fn which(body: &[u8], stored: usize) -> Option<usize> {
        (0..=stored).find(|&n| vcon(A, &subject(n)).1 == body)
    }

    /// Runs `readers` threads that read A until `done` is set and at
    /// least RACE_READS times each, each read checked by `check` with the
    /// value of `gate` taken before the read began; gives back how many
    /// reads were made.
    fn race_reads(
        t: &Arc<Tenant>,
        readers: usize,
        done: &Arc<std::sync::atomic::AtomicBool>,
        gate: &Arc<std::sync::atomic::AtomicUsize>,
        check: impl Fn(usize, Result<Vec<u8>, StoreError>) + Send + Sync + 'static,
    ) -> Vec<std::thread::JoinHandle<usize>> {
        use std::sync::atomic::Ordering::SeqCst;
        let check = Arc::new(check);
        (0..readers)
            .map(|_| {
                let (t, done, gate, check) = (t.clone(), done.clone(), gate.clone(), check.clone());
                std::thread::spawn(move || {
                    let deadline = std::time::Instant::now() + PATIENCE * 6;
                    let mut made = 0;
                    while made < RACE_READS || !done.load(SeqCst) {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "the race ended within its deadline"
                        );
                        let before = gate.load(SeqCst);
                        check(before, t.store.get(&t.caller, uuid(A)).map(|s| s.body));
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
    /// replaced version: every read answers a whole version, never one
    /// purged before the read began, and every read is recorded once.
    #[test]
    fn reads_racing_replaces_and_purges_answer_whole_unpurged_versions() {
        use std::sync::atomic::Ordering::SeqCst;
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        const ROUNDS: usize = 12;
        let (_db, t) = tenant();
        let (mut current, _) = stored(&t, A, &subject(0));
        let done = Arc::new(AtomicBool::new(false));
        // Every version before this one is purged.
        let purged = Arc::new(AtomicUsize::new(0));
        let readers = race_reads(&t, 4, &done, &purged, |before, answered| {
            let body = answered.unwrap_or_else(|e| panic!("a read is answered, not {e:?}"));
            let n = which(&body, ROUNDS).expect("the body is one whole version");
            assert!(
                n >= before,
                "version {n} was purged before the read began (all below {before} were)"
            );
        });
        for n in 1..=ROUNDS {
            let (found, body) = vcon(A, &subject(n));
            current = t
                .store
                .replace(&t.caller, current, &found, &body)
                .expect("replaced");
            let report = t.store.purge_due(Retention::Days(0)).expect("purges");
            assert_eq!(report.purged, 1, "the replaced version is purged");
            purged.store(n, SeqCst);
        }
        done.store(true, SeqCst);
        let made = joined(readers);
        let recorded = reads(&t);
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
        use std::sync::atomic::Ordering::SeqCst;
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        let (_db, t) = tenant();
        let (_, body) = stored(&t, A, &subject(0));
        let done = Arc::new(AtomicBool::new(false));
        // 1 once the delete and its purge have committed.
        let gone = Arc::new(AtomicUsize::new(0));
        let answered_reads = Arc::new(AtomicUsize::new(0));
        let counted = answered_reads.clone();
        let readers = race_reads(
            &t,
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
        let deadline = std::time::Instant::now() + PATIENCE;
        while answered_reads.load(SeqCst) < 8 {
            assert!(
                std::time::Instant::now() < deadline,
                "reads were answered before the delete"
            );
            std::thread::yield_now();
        }
        t.store
            .delete(&t.caller, uuid(A), Retention::Days(0))
            .expect("deleted and purged");
        gone.store(1, SeqCst);
        done.store(true, SeqCst);
        let made = joined(readers);
        let recorded = reads(&t);
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

    /// How many databases named `name` the server holds, asked from a
    /// database of its own.
    fn databases_named(name: &str) -> i64 {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async {
            let asker = Db::fresh().await;
            let admin = asker.admin().await;
            count(
                &admin,
                &format!("SELECT count(*) FROM pg_database WHERE datname = '{name}'"),
            )
            .await
        })
    }

    /// A test's database goes when the test ends, although a thread of the
    /// test (a read that has sent its answer and not yet ended, or one a
    /// failed assertion left waiting) still holds the tenant: nextest ends
    /// the process when the test returns, and a database whose drop waits
    /// for that thread is never dropped.
    #[test]
    fn the_database_goes_when_the_test_ends_while_a_thread_holds_the_tenant() {
        let (db, t) = tenant();
        let name = db.name.clone();
        let held = t.clone();
        // The test's end: its own bindings go, in their order.
        drop(t);
        drop(db);
        assert_eq!(
            databases_named(&name),
            0,
            "the database {name} was dropped with the test's own handles"
        );
        drop(held);
    }
}
