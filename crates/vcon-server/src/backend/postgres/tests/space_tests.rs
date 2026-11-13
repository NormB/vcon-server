//! The PostgreSQL tenant space's tests: where a tenant's catalog rows and
//! its objects in the bucket are made, opened, listed and removed. They
//! need a PostgreSQL server and an S3-compatible service.

#[cfg(feature = "backend-tests")]
pub(super) mod on_a_server {
    use std::sync::Arc;

    use super::super::super::catalog::{CatalogError, Database, PgCatalog};
    use super::super::super::space::PgTenantSpace;
    use super::super::catalog_tests::on_a_server::database;
    use super::super::schema_tests::live::{Db, in_scope, insert_tenant_row};
    use crate::backend::conformance::{self, TestClock};
    use crate::backend::s3::S3;
    use crate::backend::s3::remote::bucket;
    use crate::backend::{Objects, TenantSpace, Under};
    use crate::envelope::{self, Key, REGISTRY};
    use crate::fs::OsFs;
    use crate::ids::{TenantId, VersionId};
    use crate::store::{Caller, OpenError, Store, StoreError};
    use crate::tenants::TenantError;

    /// The tables that hold a tenant's rows and are emptied of them when
    /// the tenant is removed.
    const REMOVED: [&str; 9] = [
        "version",
        "vcon",
        "tombstone",
        "key_use",
        "parquet_file",
        "purge_pending",
        "manifest",
        "tenant_catalog",
        "tenant_lock",
    ];

    /// A space on a migrated database and a new bucket, both the test's
    /// own.
    pub(crate) fn space() -> (Db, Arc<Database>, Arc<S3>, PgTenantSpace) {
        let (db, database) = database();
        let (s3, _) = bucket(false);
        let space = PgTenantSpace::new(database.clone(), s3.clone(), Arc::new(OsFs));
        (db, database, s3, space)
    }

    /// A tenant's key, kept wrapped so that it can be given out again: a
    /// key cannot be copied.
    pub(crate) struct Keys {
        pub(crate) tenant: TenantId,
        master: Key,
        wrapped: Vec<u8>,
    }

    impl Keys {
        pub(crate) fn new() -> Keys {
            Keys::of(TenantId::generate())
        }

        pub(crate) fn of(tenant: TenantId) -> Keys {
            let master = Key::generate().expect("a master key");
            let key = Key::generate().expect("a tenant key");
            let wrapped = envelope::wrap(&master, &envelope::Context::Tenant { tenant }, &key);
            Keys {
                tenant,
                master,
                wrapped,
            }
        }

        pub(crate) fn key(&self) -> Key {
            let ctx = envelope::Context::Tenant {
                tenant: self.tenant,
            };
            envelope::unwrap(&self.master, &ctx, &self.wrapped).expect("unwraps")
        }

        pub(crate) fn catalog(&self) -> Key {
            envelope::catalog_key(&self.key()).expect("a catalog key")
        }
    }

    pub(crate) fn clock() -> Arc<TestClock> {
        TestClock::at(conformance::START)
    }

    /// A new tenant made in `space`, and its keys.
    pub(crate) fn made(space: &PgTenantSpace) -> Keys {
        let keys = Keys::new();
        space
            .make(keys.tenant, keys.key(), clock())
            .expect("makes the tenant");
        keys
    }

    pub(crate) fn open(space: &PgTenantSpace, keys: &Keys) -> Store {
        space
            .open(keys.tenant, keys.key(), true, clock())
            .expect("opens the tenant")
    }

    /// Stores the `n`th vCon in `store` and reads it back, which writes
    /// audit records too.
    pub(crate) fn store_one(store: &Store, n: u8) -> uuid::Uuid {
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let uuid = format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}");
        let (found, body) = conformance::vcon(&uuid, "stored");
        store.create(&who, &found, &body).expect("creates");
        store.get(&who, found.uuid).expect("reads");
        found.uuid
    }

    /// Whether `store` reads `uuid`.
    pub(crate) fn reads(store: &Store, uuid: uuid::Uuid) -> bool {
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        store.get(&who, uuid).is_ok()
    }

    /// The number of rows of `table` a transaction of `tenant` sees, read
    /// by the application role on a connection of the test's own.
    pub(crate) fn rows(db: &Db, tenant: TenantId, table: &str) -> i64 {
        let sql = format!("SELECT count(*) FROM vcon.{table}");
        let messages = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(async { in_scope(&db.app().await, &tenant.to_string(), &sql).await })
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        messages
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0),
                _ => None,
            })
            .expect("a count")
            .parse()
            .expect("a number")
    }

    /// The rows of `tenant` in each table a removal empties, by table.
    fn kept(db: &Db, tenant: TenantId) -> Vec<(&'static str, i64)> {
        REMOVED
            .iter()
            .map(|table| (*table, rows(db, tenant, table)))
            .filter(|(_, n)| *n > 0)
            .collect()
    }

    fn open_error(result: Result<Store, OpenError>) -> OpenError {
        match result {
            Ok(_) => panic!("opened"),
            Err(e) => e,
        }
    }

    // What must work.

    #[test]
    fn a_made_tenant_opens_and_keeps_a_vcon() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let uuid = store_one(&open(&space, &keys), 1);
        assert!(
            reads(&open(&space, &keys), uuid),
            "a second open reads what the first stored"
        );
    }

    #[test]
    fn make_writes_the_lock_row_and_the_id_of_the_catalog_key() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        assert_eq!(
            kept(&db, keys.tenant),
            [("tenant_catalog", 1), ("tenant_lock", 1)],
            "the two rows of a new tenant"
        );
    }

    #[test]
    fn a_made_tenant_is_present() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        assert_eq!(
            space.present().expect("lists"),
            [keys.tenant],
            "the tenant that was made"
        );
    }

    #[test]
    fn present_lists_the_tenants_by_id() {
        let (_db, _database, _s3, space) = space();
        let mut ids = vec![
            made(&space).tenant,
            made(&space).tenant,
            made(&space).tenant,
        ];
        ids.sort_by_key(|id| id.0);
        assert_eq!(
            space.present().expect("lists"),
            ids,
            "in the order of their ids"
        );
    }

    #[test]
    fn remove_deletes_the_tenant_s_rows_of_every_table_but_the_log() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        store_one(&open(&space, &keys), 1);
        assert!(
            rows(&db, keys.tenant, "version") == 1 && rows(&db, keys.tenant, "vcon") == 1,
            "a version and its vCon were stored"
        );
        space.remove(keys.tenant).expect("removes");
        assert_eq!(kept(&db, keys.tenant), [], "no row is left in any of them");
    }

    #[test]
    fn remove_keeps_the_tenant_s_audit_records() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        store_one(&open(&space, &keys), 1);
        let before = rows(&db, keys.tenant, "audit");
        assert!(
            before >= 2,
            "the create and the read were recorded: {before}"
        );
        space.remove(keys.tenant).expect("removes");
        assert_eq!(
            rows(&db, keys.tenant, "audit"),
            before,
            "every audit record of the removed tenant is there"
        );
    }

    #[test]
    fn a_removed_tenant_is_not_present() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let other = made(&space);
        space.remove(keys.tenant).expect("removes");
        assert_eq!(
            space.present().expect("lists"),
            [other.tenant],
            "the other tenant alone"
        );
    }

    #[test]
    fn remove_leaves_another_tenant_whole() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        let other = made(&space);
        store_one(&open(&space, &keys), 1);
        let uuid = store_one(&open(&space, &other), 2);
        let before = kept(&db, other.tenant);
        space.remove(keys.tenant).expect("removes");
        assert_eq!(kept(&db, other.tenant), before, "the other tenant's rows");
        assert!(
            reads(&open(&space, &other), uuid),
            "and its object in the bucket"
        );
    }

    #[test]
    fn remove_deletes_the_tenant_s_objects_in_the_bucket() {
        let (_db, _database, s3, space) = space();
        let keys = made(&space);
        store_one(&open(&space, &keys), 1);
        assert_eq!(
            s3.tenants().expect("lists"),
            [keys.tenant],
            "the tenant has objects"
        );
        space.remove(keys.tenant).expect("removes");
        assert_eq!(s3.tenants().expect("lists"), [], "none after the removal");
    }

    #[test]
    fn a_catalog_is_under_the_key_it_was_made_under() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let new = Key::generate().expect("a key");
        assert_eq!(
            space
                .catalog_under(keys.tenant, &keys.catalog(), &new)
                .expect("reads the key's id"),
            Under::Old,
            "made under the first of the two"
        );
    }

    #[test]
    fn a_catalog_made_under_the_second_key_is_under_the_new_one() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let old = Key::generate().expect("a key");
        assert_eq!(
            space
                .catalog_under(keys.tenant, &old, &keys.catalog())
                .expect("reads the key's id"),
            Under::New,
            "made under the second of the two"
        );
    }

    #[test]
    fn a_tenant_opens_without_recovery() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let uuid = store_one(&open(&space, &keys), 1);
        let beside = space
            .open(keys.tenant, keys.key(), false, clock())
            .expect("opens beside a server");
        assert!(reads(&beside, uuid), "and reads what is stored");
    }

    #[test]
    fn making_a_tenant_twice_under_one_key_keeps_one_catalog() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        let uuid = store_one(&open(&space, &keys), 1);
        space
            .make(keys.tenant, keys.key(), clock())
            .expect("a create that is made again");
        assert_eq!(rows(&db, keys.tenant, "tenant_lock"), 1, "one lock row");
        assert!(reads(&open(&space, &keys), uuid), "and what it kept");
    }

    #[test]
    fn rows_are_removed_a_batch_at_a_time() {
        let (db, database, _s3, space) = space();
        let keys = made(&space);
        let store = open(&space, &keys);
        for n in 1..=5 {
            store_one(&store, n);
        }
        drop(store);
        assert_eq!(rows(&db, keys.tenant, "version"), 5, "five versions");
        let start = database.round_trips();
        PgCatalog::remove_in(&database, keys.tenant, 2).expect("removes");
        let small = database.round_trips() - start;
        assert_eq!(kept(&db, keys.tenant), [], "every row, in batches of two");

        let again = made(&space);
        let store = open(&space, &again);
        for n in 1..=5 {
            store_one(&store, n);
        }
        drop(store);
        let start = database.round_trips();
        PgCatalog::remove_in(&database, again.tenant, 1000).expect("removes");
        let large = database.round_trips() - start;
        assert!(
            small > large,
            "batches of two take more transactions than one batch: {small} and {large}"
        );
    }

    #[test]
    fn remove_deletes_a_row_of_each_table_and_keeps_the_audit_record() {
        let (db, _database, _s3, space) = space();
        let tenant = TenantId::generate();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async {
            let app = db.app().await;
            for table in REMOVED.iter().chain(&["audit"]) {
                in_scope(
                    &app,
                    &tenant.to_string(),
                    &insert_tenant_row(table, &tenant.to_string()),
                )
                .await
                .unwrap_or_else(|e| panic!("a row of {table}: {e}"));
            }
        });
        assert_eq!(
            kept(&db, tenant),
            REMOVED.map(|table| (table, 1)),
            "one row in each table a removal empties"
        );
        space.remove(tenant).expect("removes");
        assert_eq!(kept(&db, tenant), [], "no row is left in any of them");
        assert_eq!(rows(&db, tenant, "audit"), 1, "and the audit record is");
    }

    // What must be refused.

    #[test]
    fn a_tenant_that_was_never_made_does_not_open() {
        let (_db, _database, _s3, space) = space();
        let keys = Keys::new();
        let e = open_error(space.open(keys.tenant, keys.key(), true, clock()));
        assert!(
            matches!(e, OpenError::Store(StoreError::Io)),
            "no catalog: {e:?}"
        );
        assert_eq!(space.present().expect("lists"), [], "and none was made");
    }

    #[test]
    fn a_tenant_does_not_open_under_another_key() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let other = Keys::of(keys.tenant);
        let e = open_error(space.open(keys.tenant, other.key(), true, clock()));
        assert!(matches!(e, OpenError::WrongKey), "another key: {e:?}");
    }

    #[test]
    fn a_tenant_is_not_made_over_a_catalog_under_another_key() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let uuid = store_one(&open(&space, &keys), 1);
        let other = Keys::of(keys.tenant);
        let e = space
            .make(keys.tenant, other.key(), clock())
            .expect_err("refused");
        assert!(matches!(e, OpenError::WrongKey), "another key: {e:?}");
        assert!(
            reads(&open(&space, &keys), uuid),
            "the catalog is as it was, under its own key"
        );
    }

    #[test]
    fn a_catalog_under_neither_key_is_an_error() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let (old, new) = (
            Key::generate().expect("a key"),
            Key::generate().expect("a key"),
        );
        assert!(
            matches!(
                space.catalog_under(keys.tenant, &old, &new),
                Err(TenantError::Io)
            ),
            "neither key's id is the stored one"
        );
    }

    #[test]
    fn the_key_of_a_tenant_never_made_is_an_error() {
        let (_db, _database, _s3, space) = space();
        let keys = Keys::new();
        assert!(
            matches!(
                space.catalog_under(keys.tenant, &keys.catalog(), &keys.catalog()),
                Err(TenantError::Io)
            ),
            "no catalog to read the id from"
        );
    }

    #[test]
    fn the_registry_s_id_is_made_as_no_tenant() {
        let (_db, _database, _s3, space) = space();
        let keys = Keys::of(REGISTRY);
        let e = space
            .make(REGISTRY, keys.key(), clock())
            .expect_err("refused");
        assert!(matches!(e, OpenError::Store(StoreError::Io)), "{e:?}");
        assert_eq!(space.present().expect("lists"), [], "nothing was made");
    }

    #[test]
    fn the_registry_s_id_is_removed_as_no_tenant() {
        let (_db, database, _s3, space) = space();
        let keys = made(&space);
        assert!(
            matches!(space.remove(REGISTRY), Err(TenantError::Io)),
            "the id of all zeros is the registry's"
        );
        assert!(
            matches!(
                PgCatalog::remove_in(&database, REGISTRY, 10),
                Err(CatalogError::NoTenant)
            ),
            "and the statements say why"
        );
        assert_eq!(
            space.present().expect("lists"),
            [keys.tenant],
            "the tenant is there"
        );
    }

    #[test]
    fn the_registry_s_id_has_no_catalog_key() {
        let (_db, _database, _s3, space) = space();
        let key = Key::generate().expect("a key");
        assert!(
            matches!(
                space.catalog_under(REGISTRY, &key, &key),
                Err(TenantError::Io)
            ),
            "the registry is not a tenant"
        );
    }

    #[test]
    fn a_removed_tenant_does_not_open() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        store_one(&open(&space, &keys), 1);
        space.remove(keys.tenant).expect("removes");
        let e = open_error(space.open(keys.tenant, keys.key(), true, clock()));
        assert!(
            matches!(e, OpenError::Store(StoreError::Io)),
            "no catalog: {e:?}"
        );
    }

    #[test]
    fn a_batch_of_no_rows_is_refused() {
        let (_db, database, _s3, space) = space();
        let keys = made(&space);
        for batch in [0, -1] {
            assert!(
                matches!(
                    PgCatalog::remove_in(&database, keys.tenant, batch),
                    Err(CatalogError::Io)
                ),
                "a batch of {batch} would never end"
            );
        }
        assert_eq!(
            space.present().expect("lists"),
            [keys.tenant],
            "nothing was removed"
        );
    }

    #[test]
    fn reseal_from_a_key_the_catalog_is_not_under_is_refused_and_leaves_it_under_its_key() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let new = Key::generate().expect("a key");
        let stranger = Key::generate().expect("a key");
        assert!(
            matches!(
                space.reseal(keys.tenant, &stranger, &new),
                Err(TenantError::Io)
            ),
            "the catalog is under neither key"
        );
        assert_eq!(
            space
                .catalog_under(keys.tenant, &keys.catalog(), &new)
                .expect("reads the key's id"),
            Under::Old,
            "the catalog is under the key it had"
        );
    }

    #[test]
    fn a_tenant_s_transaction_does_not_list_the_tenants() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        let other = made(&space);
        assert_eq!(
            rows(&db, keys.tenant, "tenant_lock"),
            1,
            "its own lock row and not the one of {}",
            other.tenant
        );
    }

    // Limits and odd inputs.

    #[test]
    fn a_new_database_and_bucket_have_no_tenant() {
        let (_db, _database, _s3, space) = space();
        assert_eq!(space.present().expect("lists"), [], "none yet");
    }

    #[test]
    fn removing_a_tenant_that_was_never_made_is_no_error() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        space
            .remove(TenantId::generate())
            .expect("nothing to remove");
        assert_eq!(
            space.present().expect("lists"),
            [keys.tenant],
            "the tenant that is there stays"
        );
    }

    #[test]
    fn removing_a_tenant_twice_is_no_error() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        store_one(&open(&space, &keys), 1);
        let audit = rows(&db, keys.tenant, "audit");
        space.remove(keys.tenant).expect("removes");
        space.remove(keys.tenant).expect("a removal made again");
        assert_eq!(kept(&db, keys.tenant), [], "no row");
        assert_eq!(rows(&db, keys.tenant, "audit"), audit, "the log as it was");
    }

    #[test]
    fn a_tenant_with_objects_and_no_rows_is_present() {
        let (_db, _database, s3, space) = space();
        let tenant = TenantId::generate();
        s3.tenant(tenant)
            .put(VersionId::generate(), b"left by a removal that was cut off")
            .expect("an object");
        assert_eq!(
            space.present().expect("lists"),
            [tenant],
            "found by its objects"
        );
        space.remove(tenant).expect("removes");
        assert_eq!(space.present().expect("lists"), [], "and removed by them");
    }

    #[test]
    fn a_tenant_with_rows_and_objects_is_listed_once() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        store_one(&open(&space, &keys), 1);
        assert_eq!(
            space.present().expect("lists"),
            [keys.tenant],
            "once, though the database and the bucket both have it"
        );
    }

    #[test]
    fn one_key_given_as_old_and_new_is_the_old_one() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        assert_eq!(
            space
                .catalog_under(keys.tenant, &keys.catalog(), &keys.catalog())
                .expect("reads the key's id"),
            Under::Old,
            "the first is asked first, as the local space does"
        );
    }

    #[test]
    fn a_batch_of_one_row_removes_every_row() {
        let (db, database, _s3, space) = space();
        let keys = made(&space);
        let store = open(&space, &keys);
        for n in 1..=3 {
            store_one(&store, n);
        }
        drop(store);
        PgCatalog::remove_in(&database, keys.tenant, 1).expect("removes");
        assert_eq!(kept(&db, keys.tenant), [], "row by row");
    }

    #[test]
    fn a_tenant_made_again_after_its_removal_opens_and_its_old_records_stay() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        store_one(&open(&space, &keys), 1);
        let audit = rows(&db, keys.tenant, "audit");
        space.remove(keys.tenant).expect("removes");
        let again = Keys::of(keys.tenant);
        space
            .make(again.tenant, again.key(), clock())
            .expect("the id is free again");
        let uuid = store_one(&open(&space, &again), 2);
        assert!(reads(&open(&space, &again), uuid), "the new catalog");
        assert!(
            rows(&db, keys.tenant, "audit") > audit,
            "the old records and the new ones"
        );
    }

    #[test]
    fn a_tenant_with_no_vcon_is_removed_whole() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        space.remove(keys.tenant).expect("removes");
        assert_eq!(kept(&db, keys.tenant), [], "the two rows of a new tenant");
    }

    #[test]
    fn the_statements_list_only_tenants_with_a_lock_row() {
        let (_db, database, s3, space) = space();
        let keys = made(&space);
        let objects_only = TenantId::generate();
        s3.tenant(objects_only)
            .put(VersionId::generate(), b"an object with no row")
            .expect("an object");
        assert_eq!(
            PgCatalog::present(&database).expect("lists"),
            [keys.tenant],
            "the database's part of the list"
        );
        assert_eq!(
            space.present().expect("lists").len(),
            2,
            "the space joins both"
        );
    }
}
