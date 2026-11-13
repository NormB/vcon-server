//! Moving a tenant's catalog in PostgreSQL to a new catalog key, and the
//! key of a tenant's audit log: what must work, what must be refused, and
//! the edges. They need a PostgreSQL server; the tests through a tenant
//! space need an S3-compatible service too.

use super::super::catalog::all_changed;
use crate::store::StoreError;

#[test]
fn a_statement_that_changed_each_row_it_was_given_passes() {
    for rows in [0, 1, 2, 1_000] {
        assert!(
            all_changed(u64::try_from(rows).expect("fits"), rows).is_ok(),
            "{rows} rows given and changed"
        );
    }
}

#[test]
fn a_statement_that_changed_fewer_rows_or_more_stops_the_reseal() {
    for (changed, given) in [(0, 1), (1, 2), (2, 1), (1, 0), (999, 1_000), (u64::MAX, 1)] {
        assert!(
            matches!(all_changed(changed, given), Err(StoreError::Io)),
            "{changed} rows changed of {given} given"
        );
    }
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use std::sync::Arc;

    use time::OffsetDateTime;
    use uuid::Uuid;

    use super::super::super::catalog::{CatalogError, Database, PgCatalog};
    use super::super::catalog_tests::on_a_server::{database, tenant_catalog};
    use super::super::schema_tests::live::{Db, in_scope};
    use super::super::space_tests::on_a_server::{
        Keys, clock, made, open, reads, rows, space, store_one as store_vcon,
    };
    use crate::audit::{Action, Actor, AuditError, Entry};
    use crate::backend::{Catalog, NewVersion, TenantSpace, Under, VersionRow};
    use crate::envelope::{self, ColumnContext, ColumnKeys, Key, REGISTRY};
    use crate::ids::{KeyId, TenantId, VersionId};
    use crate::store::OpenError;
    use crate::tenants::TenantError;

    /// The rows a re-seal reads and writes in one statement, in the tests
    /// that do not set it.
    const BATCH: i64 = 1_000;

    /// The tables that hold a tenant's rows and that a re-seal reads.
    const TABLES: [&str; 7] = [
        "vcon",
        "version",
        "tombstone",
        "key_use",
        "audit_key",
        "tenant_catalog",
        "audit",
    ];

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn block_on<T>(work: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(work)
    }

    fn first(messages: &[tokio_postgres::SimpleQueryMessage], sql: &str) -> String {
        messages
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(row) => {
                    Some(row.get(0).unwrap_or("NULL").to_owned())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{sql}: no row"))
    }

    /// The first value `sql` gives the application role in a transaction
    /// of `tenant`.
    fn text(db: &Db, tenant: TenantId, sql: &str) -> String {
        let messages =
            block_on(async { in_scope(&db.app().await, &tenant.to_string(), sql).await })
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        first(&messages, sql)
    }

    /// Runs `sql` as the application role in a transaction of `tenant`.
    fn run(db: &Db, tenant: TenantId, sql: &str) {
        block_on(async { in_scope(&db.app().await, &tenant.to_string(), sql).await })
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// Runs `sql` as the role that owns the schema, in a transaction of
    /// `tenant`.
    fn run_as_owner(db: &Db, tenant: TenantId, sql: &str) {
        block_on(async {
            let client = db.admin().await;
            client
                .batch_execute("SET ROLE vcon_owner")
                .await
                .expect("the owner role");
            in_scope(&client, &tenant.to_string(), sql).await
        })
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// Every row of one table of `tenant`, as one hash.
    fn table(db: &Db, tenant: TenantId, table: &str) -> String {
        text(
            db,
            tenant,
            &format!(
                "SELECT coalesce(md5(string_agg(t::text, ';' ORDER BY t::text)), 'none') \
                 FROM vcon.{table} t"
            ),
        )
    }

    /// Every row of `tenant` in the tables a re-seal reads, by table.
    fn state(db: &Db, tenant: TenantId) -> Vec<(&'static str, String)> {
        TABLES
            .iter()
            .map(|name| (*name, table(db, tenant, name)))
            .collect()
    }

    /// The names of the tables whose rows differ between two states.
    fn changed(
        before: &[(&'static str, String)],
        after: &[(&'static str, String)],
    ) -> Vec<&'static str> {
        before
            .iter()
            .zip(after)
            .filter(|(b, a)| b.1 != a.1)
            .map(|(b, _)| b.0)
            .collect()
    }

    fn uuid(n: u128) -> Uuid {
        Uuid::from_u128(0x01a0_7da8_c2bb_83e5_b9a2_279e_0d16_0000 + n)
    }

    fn new_version(uuid: Uuid, version: VersionId, size: i64) -> NewVersion<'static> {
        NewVersion {
            version,
            uuid,
            form: "signed",
            gzip: true,
            size,
            stored_at: size,
            key: KeyId([5; 8]),
            wrapped: b"wrapped-key-bytes",
        }
    }

    /// Stores the vCon `n` with one version of `100 + n` bytes.
    fn put(catalog: &PgCatalog, n: u128) -> VersionId {
        let version = VersionId::generate();
        let size = 100 + i64::try_from(n).expect("small");
        let mut tx = catalog.immediate().expect("begins");
        tx.insert_version(&new_version(uuid(n), version, size))
            .expect("inserts the version");
        tx.insert_vcon(uuid(n), version, size)
            .expect("inserts the vCon");
        tx.commit().expect("commits");
        version
    }

    /// Adds a version to the vCon `n`, which becomes its current one.
    fn put_again(catalog: &PgCatalog, n: u128, size: i64) -> VersionId {
        let version = VersionId::generate();
        let mut tx = catalog.immediate().expect("begins");
        tx.insert_version(&new_version(uuid(n), version, size))
            .expect("inserts the version");
        tx.set_current(uuid(n), version).expect("sets it current");
        tx.commit().expect("commits");
        version
    }

    fn entry(target: &str) -> Entry {
        Entry {
            actor: Actor::Cli,
            action: Action::Purge,
            target: Some(target.to_owned()),
            version: None,
            status: 200,
        }
    }

    /// Writes one audit record of `target`.
    fn record(catalog: &PgCatalog, target: &str) {
        let mut tx = catalog.immediate().expect("begins");
        tx.record(&entry(target), OffsetDateTime::UNIX_EPOCH)
            .expect("records");
        tx.commit().expect("commits");
    }

    /// Counts one request of `target` in the minute of the epoch.
    fn count(catalog: &PgCatalog, target: &str) {
        let mut tx = catalog.immediate().expect("begins");
        tx.count(&entry(target), OffsetDateTime::UNIX_EPOCH)
            .expect("counts");
        tx.commit().expect("commits");
    }

    /// The targets of the audit records of `catalog`, oldest first, or
    /// the error of the export.
    fn targets(catalog: &PgCatalog) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        catalog.export(0, &mut out).map_err(|e| match e {
            AuditError::Database(why) => why,
            other => format!("{other:?}"),
        })?;
        Ok(String::from_utf8(out)
            .expect("text")
            .lines()
            .map(|line| {
                let record: serde_json::Value = serde_json::from_str(line).expect("a JSON line");
                record["target"].as_str().unwrap_or("none").to_owned()
            })
            .collect())
    }

    /// Each version the catalog examines with the uuid of its vCon: a
    /// version is found there by the MAC it shares with the vCon's row.
    fn examined(catalog: &PgCatalog) -> Vec<(VersionId, Uuid)> {
        let mut tx = catalog.snapshot().expect("begins");
        let mut rows: Vec<_> = tx
            .examined()
            .expect("examines")
            .iter()
            .map(|row| (row.version, row.uuid))
            .collect();
        rows.sort_by_key(|row| row.0.0);
        rows
    }

    /// Each version that is not packed, with its size.
    fn sizes(catalog: &PgCatalog) -> Vec<(VersionId, i64)> {
        let mut tx = catalog.snapshot().expect("begins");
        let mut rows: Vec<_> = tx
            .unpacked()
            .expect("reads the versions")
            .iter()
            .map(|row| (row.version, row.size))
            .collect();
        rows.sort_by_key(|row| row.0.0);
        rows
    }

    fn version_row(catalog: &PgCatalog, version: VersionId) -> Option<VersionRow> {
        let mut tx = catalog.snapshot().expect("begins");
        tx.version_row(version).expect("reads the version")
    }

    fn has_vcon(catalog: &PgCatalog, n: u128) -> bool {
        let mut tx = catalog.snapshot().expect("begins");
        tx.vcon_row(uuid(n)).expect("reads the vCon").is_some()
    }

    fn key() -> Key {
        Key::generate().expect("a key")
    }

    fn reseal(
        database: &Arc<Database>,
        tenant: TenantId,
        old: &Key,
        new: &Key,
    ) -> Result<(), CatalogError> {
        PgCatalog::reseal(database, tenant, old, new, BATCH)
    }

    fn opened(database: &Arc<Database>, tenant: TenantId, key: &Key) -> PgCatalog {
        PgCatalog::open(database.clone(), tenant, key).expect("opens under the key")
    }

    /// Makes a second audit key for `tenant`, sealed under `catalog_key`,
    /// and gives its id.
    fn second_audit_key(db: &Db, tenant: TenantId, catalog_key: &Key) -> String {
        let audit = key();
        let id = audit.id().0;
        let ctx = ColumnContext {
            tenant,
            table: "audit_key",
            column: "wrapped",
            row: &id,
        };
        let keys = ColumnKeys::derive(catalog_key).expect("derives");
        let wrapped = envelope::wrap_key(&keys, &ctx, &audit).expect("wraps");
        run(
            db,
            tenant,
            &format!(
                "INSERT INTO vcon.audit_key (tenant, key_id, ordinal, wrapped) \
                 VALUES ('\\x{tenant}', '\\x{}', 2, '\\x{}')",
                hex(&id),
                hex(&wrapped)
            ),
        );
        hex(&id)
    }

    const KEY_IDS: &str = "SELECT coalesce(string_agg(encode(key_id, 'hex'), ',' ORDER BY ordinal), '') \
         FROM vcon.audit_key";

    const WRAPPED: &str = "SELECT coalesce(string_agg(encode(wrapped, 'hex'), ',' ORDER BY ordinal), '') \
         FROM vcon.audit_key";

    const RECORD_KEYS: &str = "SELECT coalesce(string_agg(coalesce(encode(key_id, 'hex'), 'none'), ',' \
         ORDER BY seq), '') FROM vcon.audit";

    // What must work: the key of a tenant's audit log.

    #[test]
    fn a_new_tenant_has_one_audit_key_sealed_under_its_catalog_key() {
        let (db, database) = database();
        let (tenant, _key, _catalog) = tenant_catalog(&database);
        assert_eq!(rows(&db, tenant, "audit_key"), 1, "one key, made with it");
        assert_eq!(
            text(
                &db,
                tenant,
                "SELECT octet_length(key_id) || ' ' || octet_length(wrapped) FROM vcon.audit_key"
            ),
            format!("8 {}", 32 + envelope::SEALED_COLUMN_OVERHEAD),
            "the id of a key, and a key sealed as a column value"
        );
    }

    #[test]
    fn every_record_names_the_tenant_s_audit_key() {
        let (db, database) = database();
        let (tenant, _key, catalog) = tenant_catalog(&database);
        record(&catalog, "one");
        count(&catalog, "two");
        let id = text(&db, tenant, KEY_IDS);
        assert_eq!(
            text(&db, tenant, RECORD_KEYS),
            format!("{id},{id}"),
            "a record and a counted record"
        );
    }

    #[test]
    fn a_record_s_target_is_not_sealed_under_the_catalog_s_keys() {
        let (db, database) = database();
        let (tenant, key, catalog) = tenant_catalog(&database);
        record(&catalog, "the-target");
        let keys = ColumnKeys::derive(&key).expect("derives");
        assert_ne!(
            text(
                &db,
                tenant,
                "SELECT encode(target_mac, 'hex') FROM vcon.audit"
            ),
            hex(&envelope::mac(&keys, b"the-target")),
            "the MAC is made with the audit key's keys"
        );
    }

    #[test]
    fn a_catalog_opened_again_reads_the_targets_of_its_records() {
        let (_db, database) = database();
        let (tenant, key, catalog) = tenant_catalog(&database);
        record(&catalog, "one");
        record(&catalog, "two");
        drop(catalog);
        assert_eq!(
            targets(&opened(&database, tenant, &key)).expect("exports"),
            ["one", "two"],
            "the audit key is opened with the catalog"
        );
    }

    // What must work: the move to a new key.

    #[test]
    fn reseal_moves_the_catalog_to_the_new_key() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        assert_eq!(
            PgCatalog::under(&database, tenant, &old, &new).expect("reads the key's id"),
            Under::New,
            "the catalog names the new key"
        );
        assert!(
            has_vcon(&opened(&database, tenant, &new), 1),
            "the vCon is found by its uuid under the new key"
        );
    }

    #[test]
    fn a_resealed_catalog_reads_every_version_as_it_was() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        let first = put(&catalog, 1);
        let second = put(&catalog, 2);
        let before = (
            examined(&catalog),
            sizes(&catalog),
            version_row(&catalog, first),
            version_row(&catalog, second),
        );
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        let catalog = opened(&database, tenant, &new);
        assert_eq!(
            (
                examined(&catalog),
                sizes(&catalog),
                version_row(&catalog, first),
                version_row(&catalog, second),
            ),
            before,
            "each version's vCon, size, form and gzip flag"
        );
        assert_eq!(before.0.len(), 2, "both versions were examined");
        assert_eq!(
            before.2.expect("the first version").form,
            "signed",
            "the form was read"
        );
    }

    #[test]
    fn reseal_changes_every_sealed_column_and_mac_of_the_tenant_s_rows() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        drop(catalog);
        let columns = "SELECT encode(c.uuid_mac, 'hex') || encode(c.uuid_sealed, 'hex') || ' ' || \
                       encode(v.uuid_mac, 'hex') || ' ' || encode(v.form_sealed, 'hex') || ' ' || \
                       encode(v.gzip_sealed, 'hex') || ' ' || encode(v.size_sealed, 'hex') \
                       FROM vcon.vcon c, vcon.version v";
        let before = text(&db, tenant, columns);
        reseal(&database, tenant, &old, &key()).expect("re-seals");
        let after = text(&db, tenant, columns);
        for (was, is) in before.split(' ').zip(after.split(' ')) {
            assert_ne!(was, is, "a column under the old key is left");
        }
    }

    #[test]
    fn reseal_changes_no_audit_record() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        record(&catalog, "one");
        count(&catalog, "two");
        drop(catalog);
        let before = table(&db, tenant, "audit");
        reseal(&database, tenant, &old, &key()).expect("re-seals");
        assert_eq!(
            table(&db, tenant, "audit"),
            before,
            "every record is as it was written"
        );
        assert_eq!(rows(&db, tenant, "audit"), 2, "both records are there");
    }

    #[test]
    fn a_resealed_catalog_reads_the_targets_of_the_records_before_it() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        record(&catalog, "one");
        record(&catalog, "two");
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        assert_eq!(
            targets(&opened(&database, tenant, &new)).expect("exports"),
            ["one", "two"],
            "the audit key is the same, sealed under the new catalog key"
        );
    }

    #[test]
    fn reseal_seals_the_audit_key_again_and_keeps_its_id() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        drop(catalog);
        let (id, wrapped) = (text(&db, tenant, KEY_IDS), text(&db, tenant, WRAPPED));
        reseal(&database, tenant, &old, &key()).expect("re-seals");
        assert_eq!(text(&db, tenant, KEY_IDS), id, "the same key");
        assert_ne!(text(&db, tenant, WRAPPED), wrapped, "sealed again");
        assert_eq!(rows(&db, tenant, "audit_key"), 1, "and no second key");
    }

    #[test]
    fn a_counted_record_is_added_to_after_the_reseal() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        count(&catalog, "counted");
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        count(&opened(&database, tenant, &new), "counted");
        assert_eq!(
            text(
                &db,
                tenant,
                "SELECT count(*) || ' ' || max(count) FROM vcon.audit"
            ),
            "1 2",
            "the target's MAC is the same under the same audit key: one record, counted twice"
        );
    }

    #[test]
    fn records_after_the_reseal_name_the_same_audit_key() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        record(&catalog, "before");
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        let catalog = opened(&database, tenant, &new);
        record(&catalog, "after");
        let id = text(&db, tenant, KEY_IDS);
        assert_eq!(
            text(&db, tenant, RECORD_KEYS),
            format!("{id},{id}"),
            "one key for the whole log"
        );
        assert_eq!(
            targets(&catalog).expect("exports"),
            ["before", "after"],
            "and both targets are read"
        );
    }

    #[test]
    fn reseal_moves_a_tombstone_to_the_new_key() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        let mut tx = catalog.immediate().expect("begins");
        tx.tombstone(uuid(1), 7).expect("tombstones");
        tx.commit().expect("commits");
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        let catalog = opened(&database, tenant, &new);
        let mut tx = catalog.snapshot().expect("begins");
        assert!(
            tx.tombstoned(uuid(1)).expect("reads the tombstone"),
            "the purged uuid is found under the new key"
        );
        assert!(
            !tx.tombstoned(uuid(2)).expect("reads the tombstone"),
            "and no other uuid"
        );
    }

    #[test]
    fn a_catalog_is_moved_back_to_the_key_it_had() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        record(&catalog, "one");
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        reseal(&database, tenant, &new, &old).expect("re-seals back");
        let catalog = opened(&database, tenant, &old);
        assert!(has_vcon(&catalog, 1), "the vCon, under the first key again");
        assert_eq!(
            targets(&catalog).expect("exports"),
            ["one"],
            "and the record's target"
        );
    }

    #[test]
    fn a_tenant_s_key_is_rotated_through_its_space() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let store = open(&space, &keys);
        let stored = store_vcon(&store, 1);
        let next = Keys::of(keys.tenant);
        while store.rewrap(&next.key(), 10).expect("rewraps") > 0 {}
        drop(store);
        space
            .reseal(keys.tenant, &keys.catalog(), &next.catalog())
            .expect("re-seals");
        assert_eq!(
            space
                .catalog_under(keys.tenant, &keys.catalog(), &next.catalog())
                .expect("reads the key's id"),
            Under::New,
            "the catalog is under the new tenant key's catalog key"
        );
        let store = open(&space, &next);
        assert!(reads(&store, stored), "the vCon is read under the new key");
        let second = store_vcon(&store, 2);
        assert!(reads(&store, second), "and a new one is stored and read");
    }

    // What must be refused.

    #[test]
    fn the_old_key_does_not_open_a_resealed_catalog() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        drop(catalog);
        reseal(&database, tenant, &old, &key()).expect("re-seals");
        assert!(
            matches!(
                PgCatalog::open(database.clone(), tenant, &old),
                Err(CatalogError::WrongKey)
            ),
            "the catalog is under another key now"
        );
    }

    #[test]
    fn the_old_tenant_key_does_not_open_a_rotated_tenant() {
        let (_db, _database, _s3, space) = space();
        let keys = made(&space);
        let next = Keys::of(keys.tenant);
        space
            .reseal(keys.tenant, &keys.catalog(), &next.catalog())
            .expect("re-seals");
        assert!(
            matches!(
                space.open(keys.tenant, keys.key(), true, clock()),
                Err(OpenError::WrongKey)
            ),
            "the store says the key is the wrong one"
        );
    }

    #[test]
    fn reseal_from_a_key_the_catalog_is_not_under_changes_nothing() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        record(&catalog, "one");
        drop(catalog);
        let before = state(&db, tenant);
        assert_eq!(
            reseal(&database, tenant, &key(), &key()),
            Err(CatalogError::WrongKey),
            "under neither key"
        );
        assert_eq!(
            changed(&before, &state(&db, tenant)),
            [""; 0],
            "no row is changed"
        );
        assert!(
            has_vcon(&opened(&database, tenant, &old), 1),
            "the catalog opens under its key"
        );
    }

    #[test]
    fn reseal_of_a_tenant_the_database_does_not_have_is_refused() {
        let (_db, database) = database();
        assert_eq!(
            reseal(&database, TenantId::generate(), &key(), &key()),
            Err(CatalogError::NoTenant),
            "no catalog to move"
        );
    }

    #[test]
    fn reseal_of_the_registry_s_id_is_refused() {
        let (_db, database) = database();
        assert_eq!(
            reseal(&database, REGISTRY, &key(), &key()),
            Err(CatalogError::NoTenant),
            "the registry is no tenant"
        );
    }

    #[test]
    fn reseal_of_a_removed_tenant_is_refused() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        drop(catalog);
        PgCatalog::remove_in(&database, tenant, 10).expect("removes");
        assert_eq!(
            reseal(&database, tenant, &old, &key()),
            Err(CatalogError::NoTenant),
            "its catalog is gone"
        );
    }

    #[test]
    fn a_batch_of_no_rows_or_fewer_is_refused_and_changes_nothing() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        drop(catalog);
        let before = state(&db, tenant);
        for batch in [0, -1, i64::MIN] {
            assert_eq!(
                PgCatalog::reseal(&database, tenant, &old, &key(), batch),
                Err(CatalogError::Io),
                "a batch of {batch}"
            );
        }
        assert_eq!(
            changed(&before, &state(&db, tenant)),
            [""; 0],
            "no row is changed"
        );
    }

    #[test]
    fn a_vcon_whose_sealed_uuid_does_not_open_stops_the_reseal_and_nothing_is_kept() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        for n in 1..=3 {
            put(&catalog, n);
        }
        drop(catalog);
        // One row of three, so that the rows before it were written.
        run(
            &db,
            tenant,
            "UPDATE vcon.vcon SET uuid_sealed = '\\x00' \
             WHERE uuid_mac = (SELECT uuid_mac FROM vcon.vcon ORDER BY uuid_mac DESC LIMIT 1)",
        );
        let before = state(&db, tenant);
        let new = key();
        assert_eq!(
            PgCatalog::reseal(&database, tenant, &old, &new, 1),
            Err(CatalogError::Io),
            "a row that cannot be read"
        );
        assert_eq!(
            changed(&before, &state(&db, tenant)),
            [""; 0],
            "the transaction was rolled back"
        );
        assert_eq!(
            PgCatalog::under(&database, tenant, &old, &new).expect("reads the key's id"),
            Under::Old,
            "the catalog is under the key it had"
        );
    }

    #[test]
    fn a_version_whose_sealed_column_does_not_open_stops_the_reseal_and_nothing_is_kept() {
        for column in ["form_sealed", "gzip_sealed", "size_sealed"] {
            let (db, database) = database();
            let (tenant, old, catalog) = tenant_catalog(&database);
            put(&catalog, 1);
            put(&catalog, 2);
            drop(catalog);
            run(
                &db,
                tenant,
                &format!(
                    "UPDATE vcon.version SET {column} = '\\x00' \
                     WHERE version = \
                     (SELECT version FROM vcon.version ORDER BY version DESC LIMIT 1)"
                ),
            );
            let before = state(&db, tenant);
            assert_eq!(
                PgCatalog::reseal(&database, tenant, &old, &key(), 1),
                Err(CatalogError::Io),
                "{column} cannot be read"
            );
            assert_eq!(
                changed(&before, &state(&db, tenant)),
                [""; 0],
                "{column}: the vCons written before it were rolled back"
            );
        }
    }

    #[test]
    fn an_audit_key_that_does_not_open_stops_the_open_and_the_reseal() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        drop(catalog);
        run(&db, tenant, "UPDATE vcon.audit_key SET wrapped = '\\x00'");
        let before = state(&db, tenant);
        assert!(
            matches!(
                PgCatalog::open(database.clone(), tenant, &old),
                Err(CatalogError::Io)
            ),
            "a log whose key cannot be opened takes no record"
        );
        assert_eq!(
            reseal(&database, tenant, &old, &key()),
            Err(CatalogError::Io),
            "and is not moved"
        );
        assert_eq!(
            changed(&before, &state(&db, tenant)),
            [""; 0],
            "no row is changed"
        );
    }

    #[test]
    fn a_newest_audit_key_that_does_not_open_stops_the_open_though_an_older_one_opens() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        drop(catalog);
        let second = second_audit_key(&db, tenant, &old);
        run(
            &db,
            tenant,
            &format!("UPDATE vcon.audit_key SET wrapped = '\\x00' WHERE key_id = '\\x{second}'"),
        );
        let before = state(&db, tenant);
        assert!(
            matches!(
                PgCatalog::open(database.clone(), tenant, &old),
                Err(CatalogError::Io)
            ),
            "the key that would seal the new records does not open"
        );
        assert_eq!(
            reseal(&database, tenant, &old, &key()),
            Err(CatalogError::Io),
            "and the catalog is not moved"
        );
        assert_eq!(
            changed(&before, &state(&db, tenant)),
            [""; 0],
            "no row is changed: the older key was sealed again and rolled back"
        );
    }

    #[test]
    fn a_tenant_with_no_audit_key_is_not_opened_and_not_resealed() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        drop(catalog);
        run_as_owner(&db, tenant, "DELETE FROM vcon.audit_key");
        assert!(
            matches!(
                PgCatalog::open(database.clone(), tenant, &old),
                Err(CatalogError::Io)
            ),
            "no key to seal a record's target under"
        );
        let new = key();
        assert_eq!(
            reseal(&database, tenant, &old, &new),
            Err(CatalogError::Io),
            "a catalog that would not open is not moved"
        );
        assert_eq!(
            PgCatalog::under(&database, tenant, &old, &new).expect("reads the key's id"),
            Under::Old,
            "the catalog is under the key it had"
        );
    }

    #[test]
    fn the_audit_key_of_another_tenant_does_not_open_a_tenant_s_log() {
        let (db, database) = database();
        let (tenant, key, catalog) = tenant_catalog(&database);
        drop(catalog);
        // Another tenant under the same catalog key: only the tenant in
        // the sealed value's context is different.
        let other = TenantId::generate();
        drop(PgCatalog::create(database.clone(), other, &key).expect("the other catalog"));
        let theirs = text(&db, other, WRAPPED);
        run(
            &db,
            tenant,
            &format!("UPDATE vcon.audit_key SET wrapped = '\\x{theirs}'"),
        );
        assert!(
            matches!(
                PgCatalog::open(database.clone(), tenant, &key),
                Err(CatalogError::Io)
            ),
            "a sealed key is bound to its tenant"
        );
    }

    #[test]
    fn a_record_with_a_target_and_no_key_cannot_be_read() {
        let (db, database) = database();
        let (tenant, key, catalog) = tenant_catalog(&database);
        record(&catalog, "one");
        // A record as it was before schema version 11: its target sealed
        // with the catalog's keys, and no key named.
        let keys = ColumnKeys::derive(&key).expect("derives");
        let mac = envelope::mac(&keys, b"two");
        let ctx = ColumnContext {
            tenant,
            table: "audit",
            column: "target_sealed",
            row: &mac,
        };
        let sealed = envelope::seal_column(&keys, &ctx, b"two").expect("seals");
        run(
            &db,
            tenant,
            &format!(
                "INSERT INTO vcon.audit (tenant, at, actor, action, target_mac, target_sealed, \
                 status, count, counted) \
                 VALUES ('\\x{tenant}', 't', 'cli', 'purge', '\\x{}', '\\x{}', 200, 1, false)",
                hex(&mac),
                hex(&sealed)
            ),
        );
        assert_eq!(
            targets(&catalog),
            Err("an audit record cannot be read".to_owned()),
            "no key to open its target with"
        );
    }

    #[test]
    fn a_reseal_that_was_refused_can_be_made_again_with_the_right_key() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        drop(catalog);
        let new = key();
        assert_eq!(
            reseal(&database, tenant, &key(), &new),
            Err(CatalogError::WrongKey),
            "the wrong old key"
        );
        reseal(&database, tenant, &old, &new).expect("re-seals");
        assert!(
            has_vcon(&opened(&database, tenant, &new), 1),
            "the refusal left nothing behind"
        );
    }

    #[test]
    fn a_space_s_reseal_of_a_tenant_it_does_not_have_is_an_error() {
        let (_db, _database, _s3, space) = space();
        assert!(
            matches!(
                space.reseal(TenantId::generate(), &key(), &key()),
                Err(TenantError::Io)
            ),
            "no catalog to move"
        );
    }

    // The edges.

    #[test]
    fn a_tenant_with_no_vcon_is_resealed() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        drop(catalog);
        let new = key();
        let before = state(&db, tenant);
        reseal(&database, tenant, &old, &new).expect("re-seals");
        assert_eq!(
            changed(&before, &state(&db, tenant)),
            ["audit_key", "tenant_catalog"],
            "the audit key and the id of the catalog key"
        );
        drop(opened(&database, tenant, &new));
    }

    #[test]
    fn reseal_to_the_key_the_catalog_is_under_changes_nothing() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        let before = state(&db, tenant);
        reseal(&database, tenant, &old, &new).expect("a re-seal made again is done already");
        reseal(&database, tenant, &new, &new).expect("from and to the same key");
        assert_eq!(
            changed(&before, &state(&db, tenant)),
            [""; 0],
            "no row is changed"
        );
    }

    #[test]
    fn every_row_is_moved_whatever_the_batch() {
        for batch in [1, 2, 3, 4, 1_000] {
            let (_db, database) = database();
            let (tenant, old, catalog) = tenant_catalog(&database);
            for n in 1..=3 {
                put(&catalog, n);
            }
            put_again(&catalog, 2, 500);
            let before = (examined(&catalog), sizes(&catalog));
            drop(catalog);
            let new = key();
            PgCatalog::reseal(&database, tenant, &old, &new, batch).expect("re-seals");
            let catalog = opened(&database, tenant, &new);
            assert_eq!(
                (examined(&catalog), sizes(&catalog)),
                before,
                "a batch of {batch}: four versions of three vCons"
            );
            assert_eq!(before.0.len(), 4, "every version was examined");
            for n in 1..=3 {
                assert!(has_vcon(&catalog, n), "a batch of {batch}: vCon {n}");
            }
        }
    }

    #[test]
    fn every_version_of_a_vcon_follows_it_to_the_new_key() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        put_again(&catalog, 1, 300);
        put_again(&catalog, 1, 400);
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        let catalog = opened(&database, tenant, &new);
        let mut tx = catalog.snapshot().expect("begins");
        assert_eq!(
            tx.versions_of(uuid(1)).expect("counts the versions"),
            3,
            "each version's MAC is the vCon's new one"
        );
    }

    #[test]
    fn a_version_with_no_vcon_and_no_tombstone_is_resealed_and_does_not_stop_the_move() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        let lone = VersionId::generate();
        let mut tx = catalog.immediate().expect("begins");
        tx.insert_version(&new_version(uuid(9), lone, 900))
            .expect("inserts a version alone");
        tx.commit().expect("commits");
        let before = version_row(&catalog, lone);
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        let catalog = opened(&database, tenant, &new);
        assert_eq!(
            version_row(&catalog, lone),
            before,
            "its form and gzip flag are read under the new key"
        );
        assert!(before.is_some(), "the version was read before");
        assert!(has_vcon(&catalog, 1), "and the vCon beside it was moved");
    }

    #[test]
    fn the_versions_of_a_purged_vcon_that_are_left_follow_its_tombstone() {
        let (_db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        let mut tx = catalog.immediate().expect("begins");
        tx.tombstone(uuid(1), 7).expect("tombstones");
        tx.commit().expect("commits");
        drop(catalog);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        let catalog = opened(&database, tenant, &new);
        let mut tx = catalog.snapshot().expect("begins");
        assert_eq!(
            tx.versions_of(uuid(1)).expect("counts the versions"),
            1,
            "the version is found by the uuid the tombstone holds"
        );
    }

    #[test]
    fn reseal_of_one_tenant_changes_no_row_of_another() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        drop(catalog);
        let (other, other_key, other_catalog) = tenant_catalog(&database);
        put(&other_catalog, 1);
        record(&other_catalog, "theirs");
        drop(other_catalog);
        let before = state(&db, other);
        reseal(&database, tenant, &old, &key()).expect("re-seals");
        assert_eq!(
            changed(&before, &state(&db, other)),
            [""; 0],
            "the other tenant's rows"
        );
        let theirs = opened(&database, other, &other_key);
        assert!(has_vcon(&theirs, 1), "the other tenant's vCon");
        assert_eq!(
            targets(&theirs).expect("exports"),
            ["theirs"],
            "and its record"
        );
    }

    #[test]
    fn a_catalog_is_moved_from_key_to_key_three_times() {
        let (_db, database) = database();
        let (tenant, first, catalog) = tenant_catalog(&database);
        put(&catalog, 1);
        record(&catalog, "one");
        drop(catalog);
        let mut under = first;
        for _ in 0..3 {
            let next = key();
            reseal(&database, tenant, &under, &next).expect("re-seals");
            under = next;
        }
        let catalog = opened(&database, tenant, &under);
        assert!(has_vcon(&catalog, 1), "the vCon, under the fourth key");
        assert_eq!(
            targets(&catalog).expect("exports"),
            ["one"],
            "and the record"
        );
    }

    #[test]
    fn the_newest_of_two_audit_keys_seals_the_new_records_and_both_are_read() {
        let (db, database) = database();
        let (tenant, key, catalog) = tenant_catalog(&database);
        record(&catalog, "under-the-first");
        drop(catalog);
        let first = text(&db, tenant, KEY_IDS);
        let second = second_audit_key(&db, tenant, &key);
        let catalog = opened(&database, tenant, &key);
        record(&catalog, "under-the-second");
        assert_eq!(
            text(&db, tenant, RECORD_KEYS),
            format!("{first},{second}"),
            "each record names the key that was the newest when it was written"
        );
        assert_eq!(
            targets(&catalog).expect("exports"),
            ["under-the-first", "under-the-second"],
            "the first key is kept, and its record is read"
        );
    }

    #[test]
    fn reseal_seals_every_audit_key_of_a_tenant_again() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        record(&catalog, "under-the-first");
        drop(catalog);
        second_audit_key(&db, tenant, &old);
        record(&opened(&database, tenant, &old), "under-the-second");
        let (ids, wrapped) = (text(&db, tenant, KEY_IDS), text(&db, tenant, WRAPPED));
        let new = key();
        reseal(&database, tenant, &old, &new).expect("re-seals");
        assert_eq!(text(&db, tenant, KEY_IDS), ids, "the same two keys");
        let after = text(&db, tenant, WRAPPED);
        for (was, is) in wrapped.split(',').zip(after.split(',')) {
            assert_ne!(was, is, "a key left under the old catalog key");
        }
        assert_eq!(
            targets(&opened(&database, tenant, &new)).expect("exports"),
            ["under-the-first", "under-the-second"],
            "the records of both keys are read under the new catalog key"
        );
    }

    #[test]
    fn an_older_audit_key_that_does_not_open_is_left_and_its_records_cannot_be_read() {
        let (db, database) = database();
        let (tenant, old, catalog) = tenant_catalog(&database);
        record(&catalog, "under-the-first");
        drop(catalog);
        run(&db, tenant, "UPDATE vcon.audit_key SET wrapped = '\\x00'");
        second_audit_key(&db, tenant, &old);
        let new = key();
        reseal(&database, tenant, &old, &new).expect("the newest key opens: the move is made");
        assert!(
            text(&db, tenant, WRAPPED).starts_with("00,"),
            "the key that did not open is as it was"
        );
        let catalog = opened(&database, tenant, &new);
        assert_eq!(
            targets(&catalog),
            Err("an audit record cannot be read".to_owned()),
            "the record of the key that does not open"
        );
        record(&catalog, "under-the-second");
        assert_eq!(rows(&db, tenant, "audit"), 2, "a new record is written");
    }

    #[test]
    fn a_tenant_made_again_keeps_its_one_audit_key() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        let id = text(&db, keys.tenant, KEY_IDS);
        assert_eq!(id.len(), 16, "the id of one key, in hex: {id}");
        space
            .make(keys.tenant, keys.key(), clock())
            .expect("a create made again opens the tenant");
        assert_eq!(text(&db, keys.tenant, KEY_IDS), id, "no second key");
    }

    #[test]
    fn a_tenant_made_again_after_its_removal_gets_a_new_audit_key_after_the_kept_one() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        let first = text(&db, keys.tenant, KEY_IDS);
        space.remove(keys.tenant).expect("removes");
        let again = Keys::of(keys.tenant);
        space
            .make(again.tenant, again.key(), clock())
            .expect("the id is free again");
        let ids = text(&db, keys.tenant, KEY_IDS);
        let second = ids
            .strip_prefix(&format!("{first},"))
            .unwrap_or_else(|| panic!("the kept key first, then a new one: {ids}"))
            .to_owned();
        assert_eq!(second.len(), 16, "one new key: {ids}");
        assert_eq!(
            text(
                &db,
                keys.tenant,
                "SELECT string_agg(ordinal::text, ',' ORDER BY ordinal) FROM vcon.audit_key"
            ),
            "1,2",
            "the new key is the newest"
        );
        store_vcon(&open(&space, &again), 1);
        assert_eq!(
            text(
                &db,
                keys.tenant,
                "SELECT string_agg(DISTINCT encode(key_id, 'hex'), ',') FROM vcon.audit"
            ),
            second,
            "the records of the tenant made again name its new key"
        );
    }

    #[test]
    fn a_removed_tenant_s_audit_key_is_kept_with_its_records() {
        let (db, _database, _s3, space) = space();
        let keys = made(&space);
        let store = open(&space, &keys);
        store_vcon(&store, 1);
        drop(store);
        let id = text(&db, keys.tenant, KEY_IDS);
        assert_eq!(id.len(), 16, "the id of one key, in hex: {id}");
        space.remove(keys.tenant).expect("removes");
        assert_eq!(
            text(&db, keys.tenant, KEY_IDS),
            id,
            "the key the kept records name"
        );
        assert!(
            rows(&db, keys.tenant, "audit") > 0,
            "the records are kept, as before"
        );
    }

    #[test]
    fn a_tenant_rotated_twice_through_its_space_reads_what_each_key_stored() {
        let (_db, _database, _s3, space) = space();
        let mut keys = made(&space);
        let mut stored = Vec::new();
        for n in 1..=3 {
            let store = open(&space, &keys);
            stored.push(store_vcon(&store, n));
            let next = Keys::of(keys.tenant);
            while store.rewrap(&next.key(), 1).expect("rewraps") > 0 {}
            drop(store);
            space
                .reseal(keys.tenant, &keys.catalog(), &next.catalog())
                .expect("re-seals");
            keys = next;
        }
        let store = open(&space, &keys);
        // The message names a vCon by its place in the test: a uuid is a
        // value the catalog seals, and is not written out.
        for (at, uuid) in stored.into_iter().enumerate() {
            assert!(
                reads(&store, uuid),
                "vCon {}, stored under an earlier key",
                at + 1
            );
        }
    }
}
