//! A tenant's reads on a PostgreSQL catalog while another server removes
//! the tenant: its objects first, then its rows, the lock row last. No
//! read is answered as damaged: a read answers the whole version, or is
//! refused as a removed tenant's. The live tests need a PostgreSQL server.

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use std::sync::Arc;
    use std::sync::atomic::Ordering::SeqCst;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::Duration;

    use uuid::Uuid;

    use super::super::super::catalog::{CatalogError, Database, PgCatalog};
    use super::super::catalog_tests::on_a_server::{database, entry, owner_reads, tenant_catalog};
    use super::super::schema_tests::live::{Db, waiters};
    use crate::backend::Catalog;
    use crate::backend::conformance::{START, TestClock, vcon};
    use crate::backend::local::LocalObjects;
    use crate::envelope::{self, Key, REGISTRY};
    use crate::fs::OsFs;
    use crate::ids::TenantId;
    use crate::store::{Caller, Created, Store, StoreError};

    const A: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bca1";

    /// How long a step a test waits for may take.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// How many reads each racing reader makes at least.
    const RACE_READS: usize = 40;

    /// A tenant's Store on a database of the test's own, its objects in a
    /// directory.
    struct Tenant {
        store: Store,
        caller: Caller,
        tenant: TenantId,
        database: Arc<Database>,
        objects: tempfile::TempDir,
    }

    /// The test's database and a tenant on it. The test holds the
    /// database itself, bound before the tenant (`let (db, t) =
    /// tenant();`), so it is dropped when the test ends, pass or fail: a
    /// reader thread a failed check left running may still hold the
    /// tenant, and nextest ends the process once the test returns, so a
    /// database the tenant held would never be dropped.
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
            tenant,
            database,
            objects,
        });
        (db, t)
    }

    fn stored(t: &Tenant) -> Vec<u8> {
        let (found, body) = vcon(A, "v0");
        let Ok(Created::New(_)) = t.store.create(&t.caller, &found, &body) else {
            panic!("a new vCon")
        };
        body
    }

    /// Deletes every object of the tenant's directory, as the bucket's
    /// removal does.
    fn objects_removed(t: &Tenant) {
        for entry in std::fs::read_dir(t.objects.path()).expect("the objects") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                std::fs::remove_dir_all(&path).expect("removed");
            } else {
                std::fs::remove_file(&path).expect("removed");
            }
        }
    }

    /// Runs `readers` threads that read A until `done` is set and at
    /// least RACE_READS times each, each read checked by `check` with the
    /// value of `gate` taken before the read began; gives back the
    /// threads, each of which answers how many reads it made.
    fn race_reads(
        t: &Arc<Tenant>,
        readers: usize,
        done: &Arc<AtomicBool>,
        gate: &Arc<AtomicUsize>,
        check: impl Fn(usize, Result<Vec<u8>, StoreError>) + Send + Sync + 'static,
    ) -> Vec<std::thread::JoinHandle<usize>> {
        let check = Arc::new(check);
        let a = Uuid::parse_str(A).expect("a uuid");
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
                        check(before, t.store.get(&t.caller, a).map(|s| s.body));
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

    /// Reads race the tenant's removal on another server (its objects
    /// first, then its rows, the lock row last): a read answers the whole
    /// version or is refused, never as damaged, and every read that began
    /// after the removal ended is refused.
    #[test]
    fn reads_racing_a_tenant_removal_answer_the_version_or_are_refused() {
        let (_db, t) = tenant();
        let body = stored(&t);
        let done = Arc::new(AtomicBool::new(false));
        let removed = Arc::new(AtomicUsize::new(0));
        let answered_reads = Arc::new(AtomicUsize::new(0));
        let counted = answered_reads.clone();
        let readers = race_reads(
            &t,
            4,
            &done,
            &removed,
            move |before, answered| match answered {
                Ok(b) => {
                    assert_eq!(
                        before, 0,
                        "no read that began after the removal is answered"
                    );
                    assert_eq!(b, body, "the whole version");
                    counted.fetch_add(1, SeqCst);
                }
                Err(e) => assert_ne!(e, StoreError::Damaged, "refused, not as damaged"),
            },
        );
        let deadline = std::time::Instant::now() + PATIENCE;
        while answered_reads.load(SeqCst) < 8 {
            assert!(
                std::time::Instant::now() < deadline,
                "reads were answered before the removal"
            );
            std::thread::yield_now();
        }
        PgCatalog::remove(&t.database, t.tenant, 1, || {
            objects_removed(&t);
            Ok(())
        })
        .expect("the tenant is removed");
        removed.store(1, SeqCst);
        done.store(true, SeqCst);
        joined(readers);
    }

    /// The tenant's lock row, as the owner reads it: `["false"]`,
    /// `["true"]`, or no row.
    fn mark(db: &Db, t: &Tenant) -> Vec<String> {
        owner_reads(db, t.tenant, "SELECT removing::text FROM vcon.tenant_lock")
    }

    fn present(t: &Tenant) -> bool {
        PgCatalog::present(&t.database)
            .expect("the tenants present")
            .contains(&t.tenant)
    }

    fn a() -> Uuid {
        Uuid::parse_str(A).expect("a uuid")
    }

    // What must work.

    #[test]
    fn a_made_tenant_s_lock_row_is_not_marked() {
        let (db, t) = tenant();
        assert_eq!(
            mark(&db, &t),
            ["false"],
            "a new tenant is not being removed"
        );
    }

    #[test]
    fn the_lock_row_is_marked_before_the_objects_go() {
        let (db, t) = tenant();
        let body = stored(&t);
        PgCatalog::remove(&t.database, t.tenant, 10, || {
            assert_eq!(mark(&db, &t), ["true"], "marked when the objects go");
            assert!(
                std::fs::read_dir(t.objects.path())
                    .expect("the objects")
                    .next()
                    .is_some(),
                "and the objects are still there"
            );
            assert!(!body.is_empty(), "a vCon was stored");
            objects_removed(&t);
            Ok(())
        })
        .expect("removed");
        assert_eq!(
            mark(&db, &t),
            Vec::<String>::new(),
            "the lock row is gone at the end"
        );
        assert!(!present(&t), "and the tenant is not present");
    }

    #[test]
    fn a_read_after_the_mark_is_refused_not_damaged_while_the_object_is_there() {
        let (_db, t) = tenant();
        stored(&t);
        PgCatalog::remove(&t.database, t.tenant, 10, || {
            let read = t.store.get(&t.caller, a()).map(|s| s.body);
            let Err(e) = read else {
                panic!("a read after the mark is refused")
            };
            assert_ne!(
                e,
                StoreError::Damaged,
                "refused as a removed tenant's, not as damaged"
            );
            objects_removed(&t);
            Ok(())
        })
        .expect("removed");
    }

    #[test]
    fn a_read_after_the_removal_is_refused_not_damaged() {
        let (_db, t) = tenant();
        stored(&t);
        PgCatalog::remove(&t.database, t.tenant, 1, || {
            objects_removed(&t);
            Ok(())
        })
        .expect("removed");
        let read = t.store.get(&t.caller, a()).map(|s| s.body);
        let Err(e) = read else {
            panic!("a read after the removal is refused")
        };
        assert_ne!(e, StoreError::Damaged, "refused, not as damaged");
    }

    #[test]
    fn a_removal_made_again_after_its_objects_failed_removes_the_tenant() {
        let (_db, t) = tenant();
        stored(&t);
        assert_eq!(
            PgCatalog::remove(&t.database, t.tenant, 10, || Err(CatalogError::Io)),
            Err(CatalogError::Io),
            "the objects' error"
        );
        PgCatalog::remove(&t.database, t.tenant, 10, || {
            objects_removed(&t);
            Ok(())
        })
        .expect("removed the second time");
        assert!(!present(&t), "the tenant is gone");
    }

    #[test]
    fn a_tenant_whose_lock_row_is_gone_is_removed_with_its_objects() {
        let (db, t) = tenant();
        stored(&t);
        owner_reads(
            &db,
            t.tenant,
            "DELETE FROM vcon.tenant_lock RETURNING 'gone'",
        );
        let called = AtomicBool::new(false);
        PgCatalog::remove(&t.database, t.tenant, 10, || {
            called.store(true, SeqCst);
            objects_removed(&t);
            Ok(())
        })
        .expect("removed");
        assert!(called.load(SeqCst), "its objects are removed");
        assert_eq!(
            owner_reads(&db, t.tenant, "SELECT count(*) FROM vcon.vcon"),
            ["0"],
            "and its rows"
        );
    }

    #[test]
    fn a_removal_waits_for_a_write_that_holds_the_lock_row() {
        let (db, database) = database();
        let (tenant, _, catalog) = tenant_catalog(&database);
        let mut held = catalog.immediate().expect("a write");
        held.record(&entry("held"), time::OffsetDateTime::UNIX_EPOCH)
            .expect("the write holds the lock row");
        let (ended, end) = std::sync::mpsc::channel();
        let removing = database.clone();
        let remover = std::thread::spawn(move || {
            let removed = PgCatalog::remove(&removing, tenant, 10, || Ok(()));
            let _ = ended.send(());
            removed
        });
        let deadline = std::time::Instant::now() + PATIENCE;
        while owner_reads(&db, tenant, &waiters(None)) != ["1"] {
            assert!(
                std::time::Instant::now() < deadline,
                "the removal waits for the write"
            );
            std::thread::yield_now();
        }
        assert!(
            end.try_recv().is_err(),
            "the removal has not ended while the write holds the row"
        );
        held.commit().expect("the write commits");
        assert_eq!(
            remover.join().expect("the remover ends"),
            Ok(()),
            "then the removal ends"
        );
        assert!(
            !PgCatalog::present(&database)
                .expect("present")
                .contains(&tenant),
            "and the tenant is gone"
        );
    }

    // What must be refused.

    #[test]
    fn a_write_after_the_mark_is_refused_and_stores_nothing() {
        let (db, t) = tenant();
        stored(&t);
        PgCatalog::remove(&t.database, t.tenant, 10, || {
            let (found, body) = vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bcb2", "late");
            assert!(
                t.store.create(&t.caller, &found, &body).is_err(),
                "a write after the mark is refused"
            );
            assert_eq!(
                owner_reads(&db, t.tenant, "SELECT count(*) FROM vcon.vcon"),
                ["1"],
                "and stores no vCon"
            );
            objects_removed(&t);
            Ok(())
        })
        .expect("removed");
    }

    #[test]
    fn a_removal_whose_objects_fail_keeps_the_rows_and_the_mark() {
        let (db, t) = tenant();
        stored(&t);
        assert_eq!(
            PgCatalog::remove(&t.database, t.tenant, 10, || Err(CatalogError::Io)),
            Err(CatalogError::Io),
            "the objects' error"
        );
        assert_eq!(mark(&db, &t), ["true"], "the lock row stays marked");
        assert!(
            present(&t),
            "so the tenant is present, for the next removal"
        );
        let read = t.store.get(&t.caller, a()).map(|s| s.body);
        let Err(e) = read else {
            panic!("a read of a tenant left marked is refused")
        };
        assert_ne!(e, StoreError::Damaged, "refused, not as damaged");
    }

    #[test]
    fn the_registry_s_id_is_refused_before_anything_is_removed() {
        let (_db, t) = tenant();
        let called = AtomicBool::new(false);
        assert_eq!(
            PgCatalog::remove(&t.database, REGISTRY, 10, || {
                called.store(true, SeqCst);
                Ok(())
            }),
            Err(CatalogError::NoTenant),
            "the registry is no tenant"
        );
        assert!(!called.load(SeqCst), "and no object is removed");
    }

    #[test]
    fn a_batch_of_no_rows_is_refused_after_the_objects() {
        let (db, t) = tenant();
        stored(&t);
        assert_eq!(
            PgCatalog::remove(&t.database, t.tenant, 0, || Ok(())),
            Err(CatalogError::Io),
            "remove_in's refusal of a batch of 0"
        );
        assert_eq!(
            mark(&db, &t),
            ["true"],
            "the tenant stays marked, refused to reads"
        );
    }
}
