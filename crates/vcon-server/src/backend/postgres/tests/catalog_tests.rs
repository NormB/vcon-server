//! A tenant's catalog in PostgreSQL: the store conformance suite, with the
//! objects in a bucket as a PostgreSQL catalog needs, and the rules that
//! are the PostgreSQL catalog's own. The tests run against the servers
//! `scripts/pg-test-server.sh` starts and an S3-compatible service, as the
//! parent module's and `backend::s3`'s do. Each test has a database of its
//! own.

use super::super::catalog::{
    Begin, CatalogError, SMALLEST_POOL, begin_text, commit_failure, gzip_byte, gzip_of, id_of,
    packed, pool_refusal, size_bytes, size_of,
};
use crate::backend::CommitError;
use crate::ids::TenantId;
use crate::store::StoreError;

const TENANT: TenantId = TenantId([0xab; 16]);

/// Sixteen bytes, as a pack file's id is kept.
fn file_id() -> Option<Vec<u8>> {
    Some(vec![7; 16])
}

#[test]
fn a_write_begins_read_committed_sets_the_tenant_and_locks_its_row() {
    let hex = "ab".repeat(16);
    assert_eq!(
        begin_text(Begin::Write, TENANT),
        format!(
            "BEGIN ISOLATION LEVEL READ COMMITTED; SET LOCAL vcon.tenant = '{hex}'; \
             SELECT 1 FROM vcon.tenant_lock WHERE tenant = '\\x{hex}' FOR UPDATE"
        ),
        "one round trip: the transaction, the tenant, the lock"
    );
}

#[test]
fn a_read_begins_repeatable_read_read_only_and_sets_the_tenant() {
    let text = begin_text(Begin::Read, TENANT);
    assert!(
        text.starts_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; "),
        "one consistent state, and no write: {text}"
    );
    assert!(
        text.ends_with(&format!("SET LOCAL vcon.tenant = '{}'", "ab".repeat(16))),
        "the tenant the policies read: {text}"
    );
}

#[test]
fn the_write_that_makes_a_tenant_sets_the_tenant_and_writes() {
    let text = begin_text(Begin::Create, TENANT);
    assert!(
        text.contains("READ COMMITTED") && !text.contains("READ ONLY"),
        "it writes: {text}"
    );
    assert!(
        text.contains("SET LOCAL vcon.tenant"),
        "in the tenant: {text}"
    );
}

#[test]
fn a_gzip_flag_reads_back_from_its_byte() {
    for gzip in [false, true] {
        assert_eq!(gzip_of(&gzip_byte(gzip)), Ok(gzip), "{gzip} reads back");
    }
}

#[test]
fn a_size_reads_back_from_its_eight_bytes() {
    for size in [0, 1, 255, 256, 1 << 40, i64::MAX] {
        assert_eq!(size_of(&size_bytes(size)), Ok(size), "{size} reads back");
    }
}

#[test]
fn a_version_with_no_place_columns_is_an_object_of_its_own() {
    assert_eq!(
        packed(None, None, None, None, None),
        Ok(None),
        "no pack file names it"
    );
}

#[test]
fn a_version_with_every_place_column_is_a_row_of_its_pack_file() {
    let place = packed(
        file_id(),
        Some(3),
        Some(9),
        Some("2026-11-12".to_owned()),
        Some(4096),
    )
    .expect("reads")
    .expect("a place");
    assert_eq!(place.file.0, [7; 16], "the file");
    assert_eq!(
        (place.place.group, place.place.row),
        (3, 9),
        "the row group and the row"
    );
    assert_eq!(
        (place.date.as_str(), place.bytes),
        ("2026-11-12", 4096),
        "the file's date and size"
    );
}

#[test]
fn a_commit_the_server_refused_was_rolled_back() {
    assert_eq!(
        commit_failure(true),
        CommitError::RolledBack,
        "the server answered COMMIT with an error"
    );
}

#[test]
fn a_pool_too_small_to_serve_is_refused_by_its_key_and_the_least() {
    let why = pool_refusal(1);
    assert!(
        why.starts_with("catalog.postgres.pool is 1: ") && why.contains("at least 2"),
        "names the key, the value and the least: {why}"
    );
}

#[test]
fn an_id_of_sixteen_bytes_reads() {
    assert_eq!(id_of(&[9; 16]), Ok([9; 16]), "a version, file or tenant id");
}

#[test]
fn a_commit_with_no_answer_may_or_may_not_have_been_kept() {
    assert_eq!(
        commit_failure(false),
        CommitError::Unknown,
        "the connection failed while COMMIT was outstanding"
    );
}

#[test]
fn a_gzip_byte_that_is_not_0_or_1_is_damage() {
    for byte in [2_u8, 0x30, 0xff] {
        assert_eq!(gzip_of(&[byte]), Err(StoreError::Damaged), "{byte}");
    }
}

#[test]
fn a_gzip_value_of_another_length_is_damage() {
    assert_eq!(gzip_of(&[]), Err(StoreError::Damaged), "no byte");
    assert_eq!(gzip_of(&[0, 0]), Err(StoreError::Damaged), "two bytes");
}

#[test]
fn a_size_of_another_length_is_damage() {
    assert_eq!(size_of(&[0; 7]), Err(StoreError::Damaged), "seven bytes");
    assert_eq!(size_of(&[0; 9]), Err(StoreError::Damaged), "nine bytes");
    assert_eq!(size_of(&[]), Err(StoreError::Damaged), "no bytes");
}

#[test]
fn an_id_of_another_length_is_damage() {
    assert_eq!(id_of(&[1; 15]), Err(StoreError::Damaged), "fifteen bytes");
    assert_eq!(id_of(&[1; 17]), Err(StoreError::Damaged), "seventeen bytes");
    assert_eq!(id_of(&[]), Err(StoreError::Damaged), "no bytes");
}

#[test]
fn a_place_with_some_columns_and_not_others_is_damage() {
    let date = || Some("2026-11-12".to_owned());
    for (file, group, row) in [
        (file_id(), None, None),
        (None, Some(1), None),
        (None, None, Some(1)),
        (file_id(), Some(1), None),
        (file_id(), None, Some(1)),
        (None, Some(1), Some(1)),
    ] {
        assert_eq!(
            packed(file.clone(), group, row, date(), Some(1)),
            Err(StoreError::Damaged),
            "{file:?} {group:?} {row:?}"
        );
    }
}

#[test]
fn a_place_whose_file_row_is_missing_is_damage() {
    assert_eq!(
        packed(file_id(), Some(1), Some(1), None, None),
        Err(StoreError::Damaged),
        "the version names a file the catalog has no row for"
    );
    assert_eq!(
        packed(
            file_id(),
            Some(1),
            Some(1),
            Some("2026-11-12".to_owned()),
            None
        ),
        Err(StoreError::Damaged),
        "a date and no size"
    );
}

#[test]
fn a_place_with_a_negative_number_is_damage() {
    let date = || Some("2026-11-12".to_owned());
    assert_eq!(
        packed(file_id(), Some(-1), Some(0), date(), Some(1)),
        Err(StoreError::Damaged),
        "a negative row group"
    );
    assert_eq!(
        packed(file_id(), Some(0), Some(-1), date(), Some(1)),
        Err(StoreError::Damaged),
        "a negative row"
    );
    assert_eq!(
        packed(file_id(), Some(0), Some(0), date(), Some(-1)),
        Err(StoreError::Damaged),
        "a negative size"
    );
}

#[test]
fn a_place_whose_file_id_is_not_sixteen_bytes_is_damage() {
    assert_eq!(
        packed(
            Some(vec![7; 15]),
            Some(0),
            Some(0),
            Some("2026-11-12".to_owned()),
            Some(1)
        ),
        Err(StoreError::Damaged),
        "fifteen bytes"
    );
}

#[test]
fn a_negative_size_reads_back_as_it_was_sealed() {
    for size in [-1, i64::MIN] {
        assert_eq!(size_of(&size_bytes(size)), Ok(size), "{size}");
    }
}

#[test]
fn an_object_of_its_own_ignores_a_date_and_size_no_file_gave() {
    assert_eq!(
        packed(None, None, None, Some("2026-11-12".to_owned()), Some(1)),
        Ok(None),
        "the three place columns decide"
    );
}

#[test]
fn a_place_at_the_largest_row_group_reads_and_one_past_it_is_damage() {
    let date = || Some("2026-11-12".to_owned());
    let most = i64::from(u32::MAX);
    let place = packed(file_id(), Some(most), Some(most), date(), Some(i64::MAX))
        .expect("reads")
        .expect("a place");
    assert_eq!(
        (place.place.group, place.place.row),
        (u32::MAX, u32::MAX),
        "the largest numbers a place holds"
    );
    assert_eq!(
        packed(file_id(), Some(most + 1), Some(0), date(), Some(1)),
        Err(StoreError::Damaged),
        "one past the largest row group"
    );
}

#[test]
fn the_tenant_of_all_zeros_is_written_as_thirty_two_digits() {
    let text = begin_text(Begin::Write, TenantId([0; 16]));
    assert!(
        text.contains(&format!("'\\x{}'", "0".repeat(32))),
        "the lock row's key in full: {text}"
    );
}

#[test]
fn each_way_a_catalog_does_not_open_has_its_own_words() {
    let said: std::collections::BTreeSet<String> = [
        CatalogError::WrongKey,
        CatalogError::NoTenant,
        CatalogError::Exists,
        CatalogError::Io,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(said.len(), 4, "four different messages: {said:?}");
}

#[test]
fn the_smallest_pool_is_the_lock_and_one_transaction() {
    assert_eq!(SMALLEST_POOL, 2, "one connection each");
    assert!(
        pool_refusal(0).contains("is 0"),
        "a pool of none is named as it is"
    );
}

#[cfg(feature = "backend-tests")]
pub(super) mod on_a_server {
    use std::sync::Arc;

    use time::OffsetDateTime;
    use uuid::Uuid;

    use super::super::super::catalog::{CatalogError, Database, PgCatalog, pool_refusal};
    use super::super::super::schema::{MIGRATIONS, SERVER_LOCK_HELD, SERVERS_LOCK};
    use super::super::live::value;
    use super::super::schema_tests::live::{Db, in_scope};
    use crate::audit::{Action, Actor, AuditError, Entry};
    use crate::backend::conformance::{self, Harness, TestClock};
    use crate::backend::s3::remote::{CatalogAndS3, CatalogFor, bucket};
    use crate::backend::{Catalog, CommitError, NewVersion, PageQuery};
    use crate::config::StorageError;
    use crate::envelope::Key;
    use crate::ids::{KeyId, TenantId, VersionId};
    use crate::store::{Caller, ListQuery, StoreError};

    /// The round trips to the database of one `Store::create`, one
    /// `Store::get` and one page of `Store::list`, as first measured; a
    /// change that adds one is seen here.
    const CREATE_TRIPS: usize = 13;
    const GET_TRIPS: usize = 10;
    const LIST_TRIPS: usize = 4;

    /// A migrated database of the test's own and the application role's
    /// pool on it. The runtime that made the database is dropped before
    /// any catalog call, which waits on the pool's own.
    pub(crate) fn database() -> (Db, Arc<Database>) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let db = runtime.block_on(Db::migrated());
        let (_dir, settings) = db.settings("vcon_app");
        let database = Database::connect(&settings).expect("the application role connects");
        (db, database)
    }

    /// A new tenant's catalog in `database`, under a new catalog key.
    pub(crate) fn tenant_catalog(database: &Arc<Database>) -> (TenantId, Key, PgCatalog) {
        let tenant = TenantId::generate();
        let key = Key::generate().expect("a key");
        let catalog = PgCatalog::create(database.clone(), tenant, &key).expect("the catalog");
        (tenant, key, catalog)
    }

    /// Catalogs in `database`: made at a tenant's first open, opened under
    /// the same key after.
    fn postgres_catalogs(database: &Arc<Database>) -> CatalogFor {
        let database = database.clone();
        Arc::new(move |tenant, catalog_key| {
            let catalog = if PgCatalog::exists(&database, tenant).expect("reads the tenant") {
                PgCatalog::open(database.clone(), tenant, &catalog_key).expect("opens")
            } else {
                PgCatalog::create(database.clone(), tenant, &catalog_key).expect("creates")
            };
            Box::new(catalog)
        })
    }

    #[test]
    fn a_postgres_catalog_with_s3_objects_conforms() {
        let (_db, database) = database();
        let (s3, _) = bucket(false);
        conformance::run(&CatalogAndS3 {
            catalog: postgres_catalogs(&database),
            s3,
            packed: false,
        });
    }

    #[test]
    fn a_postgres_catalog_with_parquet_storage_in_the_bucket_conforms() {
        let (_db, database) = database();
        let (s3, _) = bucket(false);
        conformance::run(&CatalogAndS3 {
            catalog: postgres_catalogs(&database),
            s3,
            packed: true,
        });
    }

    /// Runs `call` on a runtime of its own, for a test's own connections.
    fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    /// Runs `sql` as the schema's owner, around the application role.
    fn as_owner(db: &Db, sql: &str) {
        wait(async {
            db.admin()
                .await
                .batch_execute(&format!("SET ROLE vcon_owner; {sql}"))
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        });
    }

    /// The first column of each row `sql` gives the owner in `tenant`'s
    /// scope.
    fn owner_reads(db: &Db, tenant: TenantId, sql: &str) -> Vec<String> {
        wait(async {
            let admin = db.admin().await;
            admin
                .batch_execute("SET ROLE vcon_owner")
                .await
                .expect("the owner role");
            in_scope(&admin, &tenant.to_string(), sql)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"))
                .iter()
                .filter_map(|m| match m {
                    tokio_postgres::SimpleQueryMessage::Row(row) => {
                        Some(row.get(0).unwrap_or("").to_owned())
                    }
                    _ => None,
                })
                .collect()
        })
    }

    fn a_uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    /// A version row for `uuid`, stored at `stored_at`.
    fn new_version(uuid: Uuid, version: VersionId, stored_at: i64) -> NewVersion<'static> {
        NewVersion {
            version,
            uuid,
            form: "unsigned",
            gzip: false,
            size: 321,
            stored_at,
            key: KeyId([5; 8]),
            wrapped: b"wrapped-key-bytes",
        }
    }

    /// Stores a vCon of `uuid` with one version, in a transaction of its
    /// own, and gives the version's id.
    fn store_one(catalog: &PgCatalog, uuid: Uuid, stored_at: i64) -> VersionId {
        let version = VersionId::generate();
        let mut tx = catalog.immediate().expect("begins");
        tx.insert_version(&new_version(uuid, version, stored_at))
            .expect("inserts the version");
        tx.insert_vcon(uuid, version, stored_at)
            .expect("inserts the vCon");
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

    /// The audit records of `catalog`, each parsed.
    fn exported(catalog: &PgCatalog) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        catalog.export(0, &mut out).expect("exports");
        String::from_utf8(out)
            .expect("text")
            .lines()
            .map(|line| serde_json::from_str(line).expect("a JSON line"))
            .collect()
    }

    #[test]
    fn a_server_connects_the_moment_migrate_has_returned() {
        for round in 0..5 {
            let (_db, database) = database();
            assert!(
                PgCatalog::exists(&database, TenantId::generate()).is_ok(),
                "round {round}: the server reads the database"
            );
        }
    }

    #[test]
    fn a_new_tenants_catalog_is_made_once_and_opens_under_its_key() {
        let (_db, database) = database();
        let tenant = TenantId::generate();
        assert_eq!(
            PgCatalog::exists(&database, tenant),
            Ok(false),
            "no catalog before it is made"
        );
        let key = Key::generate().expect("a key");
        PgCatalog::create(database.clone(), tenant, &key).expect("made");
        assert_eq!(
            PgCatalog::exists(&database, tenant),
            Ok(true),
            "there once made"
        );
        let opened = PgCatalog::open(database.clone(), tenant, &key).expect("opens");
        assert_eq!(
            opened.snapshot().expect("begins").count_versions(),
            Ok(0),
            "a new tenant has no version"
        );
    }

    #[test]
    fn a_vcon_and_its_version_read_back_as_they_were_stored() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let version = store_one(&catalog, a_uuid(1), 77);
        let mut tx = catalog.snapshot().expect("begins");
        let vcon = tx.vcon_row(a_uuid(1)).expect("reads").expect("a row");
        assert_eq!(
            (vcon.current, vcon.deleted_at),
            (Some(version), None),
            "the vCon names its version"
        );
        let row = tx.version_row(version).expect("reads").expect("a row");
        assert_eq!(
            (row.form.as_str(), row.gzip, row.wrapped.as_slice()),
            ("unsigned", false, b"wrapped-key-bytes".as_slice()),
            "the sealed form and flag open, and the wrapped key is as given"
        );
        assert!(row.place.is_none(), "an object of its own");
        assert_eq!(
            tx.has_version(version, a_uuid(1)),
            Ok(true),
            "the version is that uuid's"
        );
        assert_eq!(
            tx.has_version(version, a_uuid(2)),
            Ok(false),
            "and no other's"
        );
    }

    #[test]
    fn the_database_holds_no_uuid_form_size_or_audit_target_in_the_clear() {
        let (db, database) = database();
        let (tenant, _, catalog) = tenant_catalog(&database);
        let uuid = Uuid::parse_str("01a07da8-c2bb-83e5-b9a2-279e0d16bc01").expect("a uuid");
        store_one(&catalog, uuid, 77);
        let mut tx = catalog.immediate().expect("begins");
        tx.record(&entry("target-in-the-clear"), OffsetDateTime::UNIX_EPOCH)
            .expect("records");
        tx.tombstone(a_uuid(9), 5).expect("a tombstone");
        tx.commit().expect("commits");
        let mut held = String::new();
        for table in ["vcon", "version", "tombstone", "audit"] {
            let rows = owner_reads(&db, tenant, &format!("SELECT t::text FROM vcon.{table} t"));
            assert!(!rows.is_empty(), "{table} holds a row to look in");
            held.push_str(&rows.join("\n"));
        }
        let simple = uuid.simple().to_string();
        for (what, text) in [
            ("the uuid's bytes", simple.as_str()),
            ("the uuid as text", "01a07da8-c2bb-83e5-b9a2-279e0d16bc01"),
            ("the tombstoned uuid", "09090909090909090909090909090909"),
            ("the form", "unsigned"),
            ("the form's bytes", "756e7369676e6564"),
            ("the audit target", "target-in-the-clear"),
            ("the audit target's bytes", "7461726765742d696e"),
            ("the size's bytes", "0000000000000141"),
        ] {
            assert!(!held.contains(text), "{what} is not in a row: {text}");
        }
        assert!(
            held.contains(&tenant.to_string()),
            "the rows were read: each holds its tenant in the clear"
        );
    }

    #[test]
    fn the_newest_version_and_the_order_of_versions_follow_storing_at_one_time() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let uuid = a_uuid(1);
        let first = store_one(&catalog, uuid, 100);
        let mut stored = vec![first];
        for _ in 0..6 {
            let version = VersionId::generate();
            let mut tx = catalog.immediate().expect("begins");
            tx.insert_version(&new_version(uuid, version, 100))
                .expect("inserts");
            tx.commit().expect("commits");
            stored.push(version);
        }
        let mut tx = catalog.snapshot().expect("begins");
        assert_eq!(
            tx.latest(uuid),
            Ok(stored.last().copied()),
            "the version stored last, though every stored_at is the same"
        );
        let kept: Vec<VersionId> = tx
            .kept(uuid)
            .expect("reads")
            .iter()
            .map(|row| row.version)
            .collect();
        assert_eq!(kept, stored, "in the order they were stored");
    }

    #[test]
    fn pages_at_one_stored_time_give_every_vcon_once() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut all = std::collections::BTreeSet::new();
        for n in 1..=7 {
            store_one(&catalog, a_uuid(n), 500);
            all.insert(a_uuid(n));
        }
        let mut seen = Vec::new();
        let mut before = None;
        loop {
            let page = catalog
                .snapshot()
                .expect("begins")
                .page(&PageQuery {
                    before,
                    since: None,
                    until: None,
                    rows: 3,
                })
                .expect("a page");
            let Some(last) = page.last() else { break };
            before = Some((last.stored_at, last.uuid));
            seen.extend(page.iter().map(|row| row.uuid));
        }
        assert_eq!(seen.len(), 7, "seven vCons over three pages: {seen:?}");
        assert_eq!(
            seen.iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            all,
            "each once"
        );
    }

    #[test]
    fn a_record_and_a_count_are_exported_with_the_target_opened() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let at = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(90);
        let mut tx = catalog.immediate().expect("begins");
        tx.record(&entry("one"), at).expect("records");
        tx.count(&entry("one"), at).expect("counts");
        tx.count(&entry("one"), at).expect("counts again");
        tx.count(&entry("two"), at).expect("counts another target");
        tx.commit().expect("commits");
        let lines = exported(&catalog);
        let shown: Vec<(&str, &str, i64)> = lines
            .iter()
            .map(|l| {
                (
                    l["target"].as_str().expect("a target"),
                    l["at"].as_str().expect("a time"),
                    l["count"].as_i64().expect("a count"),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("one", "1970-01-01T00:01:30Z", 1),
                ("one", "1970-01-01T00:01:00Z", 2),
                ("two", "1970-01-01T00:01:00Z", 1),
            ],
            "the record at its second, and one counted record a target in the minute"
        );
        assert!(
            lines.iter().all(|l| l["actor"] == "cli"
                && l["action"] == Action::Purge.as_str()
                && l["status"] == 200
                && l["version"].is_null()),
            "the clear members: {lines:?}"
        );
        assert!(
            lines
                .windows(2)
                .all(|w| w[0]["seq"].as_i64() < w[1]["seq"].as_i64()),
            "oldest first"
        );
    }

    #[test]
    fn a_page_of_the_log_holds_the_records_after_a_seq_up_to_its_limit() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut tx = catalog.immediate().expect("begins");
        for n in 0..5 {
            tx.record(&entry(&format!("t{n}")), OffsetDateTime::UNIX_EPOCH)
                .expect("records");
        }
        tx.commit().expect("commits");
        let all = exported(&catalog);
        let second = all[1]["seq"].as_i64().expect("a seq");
        let mut out = Vec::new();
        catalog
            .snapshot()
            .expect("begins")
            .audit_lines(second, Some(2), &mut out)
            .expect("a page");
        let page: Vec<serde_json::Value> = String::from_utf8(out)
            .expect("text")
            .lines()
            .map(|l| serde_json::from_str(l).expect("JSON"))
            .collect();
        assert_eq!(
            page.iter().map(|l| l["target"].clone()).collect::<Vec<_>>(),
            ["t2", "t3"],
            "the two records after the second"
        );
    }

    #[test]
    fn a_snapshot_reads_one_state_while_a_write_commits() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        store_one(&catalog, a_uuid(1), 1);
        let mut snapshot = catalog.snapshot().expect("begins");
        assert_eq!(snapshot.count_versions(), Ok(1), "one version at the start");
        store_one(&catalog, a_uuid(2), 2);
        assert_eq!(
            snapshot.count_versions(),
            Ok(1),
            "the snapshot does not see the later commit"
        );
        drop(snapshot);
        assert_eq!(
            catalog.snapshot().expect("begins").count_versions(),
            Ok(2),
            "a new snapshot does"
        );
    }

    #[test]
    fn a_second_write_of_a_tenant_waits_for_the_first_to_end() {
        let (db, database) = database();
        let (tenant, key, catalog) = tenant_catalog(&database);
        let first = catalog.immediate().expect("the first write begins");
        let second = std::thread::spawn({
            let database = database.clone();
            move || {
                let catalog = PgCatalog::open(database, tenant, &key).expect("opens");
                let begun = catalog.immediate().map(drop);
                // The transaction borrows the catalog, so it ends first.
                drop(catalog);
                begun
            }
        });
        let waiting = wait(async {
            let admin = db.admin().await;
            within_five_seconds_of(
                &admin,
                "SELECT count(*) FROM pg_locks WHERE NOT granted",
                "1",
            )
            .await
        });
        assert!(waiting, "the second write waits for the tenant's lock row");
        assert!(!second.is_finished(), "and has not begun");
        first.commit().expect("the first commits");
        assert_eq!(
            second.join().expect("the thread ends"),
            Ok(()),
            "then the second begins"
        );
    }

    /// Asks `sql` up to fifty times, a tenth of a second apart, until its
    /// value is `want`.
    async fn within_five_seconds_of(
        client: &tokio_postgres::Client,
        sql: &str,
        want: &str,
    ) -> bool {
        for _ in 0..50 {
            if value(client, sql).await == want {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        false
    }

    #[test]
    fn writes_of_two_tenants_do_not_wait_for_each_other() {
        let (_db, database) = database();
        let (_, _, one) = tenant_catalog(&database);
        let (_, _, two) = tenant_catalog(&database);
        let held = one.immediate().expect("one tenant's write begins");
        store_one(&two, a_uuid(1), 1);
        held.commit().expect("and commits after the other's");
        assert_eq!(
            two.snapshot().expect("begins").count_versions(),
            Ok(1),
            "the other tenant's write was kept"
        );
    }

    #[test]
    fn a_tenant_reads_and_changes_no_row_of_another() {
        let (_db, database) = database();
        let (_, _, one) = tenant_catalog(&database);
        let (_, _, two) = tenant_catalog(&database);
        let version = store_one(&one, a_uuid(1), 1);
        let mut tx = two.immediate().expect("begins");
        assert_eq!(tx.count_versions(), Ok(0), "none of the other's versions");
        assert!(
            tx.vcon_row(a_uuid(1)).expect("reads").is_none(),
            "the same uuid is another vCon in another tenant"
        );
        assert!(
            tx.version_row(version).expect("reads").is_none(),
            "a version id of another tenant finds nothing"
        );
        tx.delete_version(version).expect("deletes nothing");
        tx.commit().expect("commits");
        assert_eq!(
            one.snapshot().expect("begins").count_versions(),
            Ok(1),
            "the first tenant's version is still there"
        );
    }

    #[test]
    fn a_wrap_count_starts_at_none_and_adds_one_a_wrap() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let key = KeyId([3; 8]);
        let mut tx = catalog.immediate().expect("begins");
        assert_eq!(tx.wraps(key), Ok(0), "a key no version is wrapped under");
        for _ in 0..3 {
            tx.add_wrap(key).expect("adds");
        }
        assert_eq!(tx.wraps(key), Ok(3), "three wraps");
        assert_eq!(
            tx.wraps(KeyId([4; 8])),
            Ok(0),
            "another key's count is its own"
        );
        tx.commit().expect("commits");
    }

    #[test]
    fn a_part_that_is_rolled_back_leaves_what_came_before_it() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut tx = catalog.immediate().expect("begins");
        tx.add_wrap(KeyId([1; 8])).expect("before the part");
        tx.begin_part().expect("a part");
        tx.add_wrap(KeyId([2; 8])).expect("in the part");
        tx.rollback_part().expect("rolled back");
        tx.begin_part().expect("another part");
        tx.add_wrap(KeyId([3; 8])).expect("in it");
        tx.release_part().expect("kept");
        tx.commit().expect("commits");
        let mut tx = catalog.snapshot().expect("begins");
        assert_eq!(
            (
                tx.wraps(KeyId([1; 8])),
                tx.wraps(KeyId([2; 8])),
                tx.wraps(KeyId([3; 8]))
            ),
            (Ok(1), Ok(0), Ok(1)),
            "the write before, not the part rolled back, and the part kept"
        );
    }

    #[test]
    fn a_transaction_dropped_without_a_commit_keeps_nothing_and_frees_the_lock() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut tx = catalog.immediate().expect("begins");
        tx.add_wrap(KeyId([1; 8])).expect("writes");
        drop(tx);
        let mut tx = catalog.immediate().expect("the lock is free");
        assert_eq!(tx.wraps(KeyId([1; 8])), Ok(0), "the write was rolled back");
        tx.commit().expect("commits");
    }

    #[test]
    fn a_pooled_connection_serves_one_tenant_after_another() {
        let (_db, database) = database();
        let (_, _, one) = tenant_catalog(&database);
        let (_, _, two) = tenant_catalog(&database);
        for round in 0..4 {
            store_one(&one, a_uuid(round), 1);
            assert_eq!(
                two.snapshot().expect("begins").count_versions(),
                Ok(0),
                "round {round}: the connection the first tenant used reads the second's rows only"
            );
        }
        assert_eq!(
            one.snapshot().expect("begins").count_versions(),
            Ok(4),
            "the first tenant's four"
        );
    }

    #[test]
    fn the_calls_of_a_transaction_are_one_round_trip_each() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let start = database.round_trips();
        let mut tx = catalog.immediate().expect("begins");
        assert_eq!(database.round_trips() - start, 1, "begin, tenant and lock");
        tx.vcon_row(a_uuid(1)).expect("reads");
        assert_eq!(
            database.round_trips() - start,
            2,
            "a read with its savepoint"
        );
        tx.add_wrap(KeyId([1; 8])).expect("writes");
        assert_eq!(database.round_trips() - start, 3, "a write");
        tx.commit().expect("commits");
        assert_eq!(database.round_trips() - start, 4, "the commit");
        let start = database.round_trips();
        drop(catalog.snapshot().expect("begins"));
        assert_eq!(
            database.round_trips() - start,
            2,
            "a snapshot begins in one and its drop rolls back in one"
        );
    }

    #[test]
    fn after_a_purge_the_tables_are_vacuumed_where_the_role_may() {
        let (db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let version = store_one(&catalog, a_uuid(1), 1);
        let mut tx = catalog.immediate().expect("begins");
        tx.delete_version(version).expect("deletes");
        tx.commit().expect("commits");
        catalog
            .after_purge()
            .expect("VACUUM is no error on any version");
        let may = wait(async {
            let admin = db.admin().await;
            value(
                &admin,
                "SELECT current_setting('server_version_num')::int >= 170000",
            )
            .await
        }) == "t";
        let vacuumed = wait(async {
            let admin = db.admin().await;
            within_five_seconds_of(
                &admin,
                "SELECT count(*) FROM pg_stat_all_tables \
                 WHERE schemaname = 'vcon' AND relname IN ('version', 'vcon', 'tombstone') \
                   AND vacuum_count > 0",
                "3",
            )
            .await
        });
        assert_eq!(
            vacuumed, may,
            "from PostgreSQL 17 the role's VACUUM runs; before it the server skips the tables"
        );
    }

    #[test]
    fn recover_has_nothing_to_do_and_says_so() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let start = database.round_trips();
        assert_eq!(catalog.recover(), Ok(()), "PostgreSQL recovers its own log");
        assert_eq!(database.round_trips(), start, "no statement is sent");
    }

    #[test]
    fn a_second_server_on_the_database_is_refused_by_the_lock() {
        let (db, _database) = database();
        let (_dir, settings) = db.settings("vcon_app");
        let StorageError(why) = Database::connect(&settings).expect_err("refused");
        assert_eq!(
            why,
            format!("catalog.postgres: {SERVER_LOCK_HELD}"),
            "names the lock"
        );
    }

    #[test]
    fn a_server_is_refused_while_migrate_holds_the_locks() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let db = runtime.block_on(Db::migrated());
        let _holder = runtime.block_on(async {
            let holder = db.admin().await;
            holder
                .batch_execute(&format!("SELECT pg_advisory_lock({SERVERS_LOCK})"))
                .await
                .expect("holds the lock migrate takes");
            holder
        });
        let (_dir, settings) = db.settings("vcon_app");
        let StorageError(why) = Database::connect(&settings).expect_err("refused");
        assert!(why.contains("holds this database's lock"), "says so: {why}");
    }

    #[test]
    fn migrate_is_refused_while_a_server_is_connected() {
        let (db, _database) = database();
        let refused = wait(db.migrate_with(MIGRATIONS)).expect_err("refused");
        assert!(
            refused.contains("another server or command holds the database's locks"),
            "says so: {refused}"
        );
    }

    #[test]
    fn a_pool_of_one_is_refused_before_any_connection() {
        let (db, database) = database();
        drop(database);
        let (_dir, mut settings) = db.settings("vcon_app");
        settings.pool = 1;
        settings.hosts = vec!["host.invalid".to_owned()];
        let StorageError(why) = Database::connect(&settings).expect_err("refused");
        assert_eq!(
            why,
            pool_refusal(1),
            "the pool's rule, with no server named"
        );
    }

    #[test]
    fn a_catalog_under_another_key_does_not_open() {
        let (_db, database) = database();
        let (tenant, _, _catalog) = tenant_catalog(&database);
        let other = Key::generate().expect("a key");
        assert_eq!(
            PgCatalog::open(database.clone(), tenant, &other).err(),
            Some(CatalogError::WrongKey),
            "the key id kept with the tenant is another's"
        );
    }

    #[test]
    fn a_tenant_with_no_catalog_does_not_open() {
        let (_db, database) = database();
        let key = Key::generate().expect("a key");
        assert_eq!(
            PgCatalog::open(database.clone(), TenantId::generate(), &key).err(),
            Some(CatalogError::NoTenant),
            "nothing was made for it"
        );
    }

    #[test]
    fn a_catalog_is_not_made_twice() {
        let (_db, database) = database();
        let (tenant, key, _catalog) = tenant_catalog(&database);
        assert_eq!(
            PgCatalog::create(database.clone(), tenant, &key).err(),
            Some(CatalogError::Exists),
            "the tenant has its lock row"
        );
        let other = Key::generate().expect("a key");
        assert_eq!(
            PgCatalog::create(database.clone(), tenant, &other).err(),
            Some(CatalogError::Exists),
            "under another key too"
        );
        assert!(
            PgCatalog::open(database.clone(), tenant, &key).is_ok(),
            "and the first key still opens it"
        );
    }

    #[test]
    fn a_write_of_a_removed_tenant_does_not_begin() {
        let (db, database) = database();
        let (tenant, _, catalog) = tenant_catalog(&database);
        owner_reads(&db, tenant, "DELETE FROM vcon.tenant_lock RETURNING 'gone'");
        assert_eq!(
            catalog.immediate().map(drop),
            Err(StoreError::Io),
            "no lock row: the tenant was removed"
        );
        assert_eq!(
            catalog.snapshot().expect("a read begins").count_versions(),
            Ok(0),
            "and the connection it used went back to the pool usable"
        );
    }

    #[test]
    fn a_read_that_fails_leaves_the_transaction_usable() {
        let (db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut tx = catalog.immediate().expect("begins");
        tx.record(&entry("before"), OffsetDateTime::UNIX_EPOCH)
            .expect("records");
        as_owner(&db, "REVOKE SELECT ON vcon.audit FROM vcon_app");
        let mut out = Vec::new();
        let failed = tx.audit_lines(0, Some(10), &mut out);
        assert!(
            matches!(failed, Err(AuditError::Database(_))),
            "the read is refused: {failed:?}"
        );
        assert!(out.is_empty(), "and wrote nothing");
        tx.record(&entry("after"), OffsetDateTime::UNIX_EPOCH)
            .expect("the transaction still takes a record");
        assert_eq!(
            tx.wraps(KeyId([1; 8])),
            Ok(0),
            "and a read of another table"
        );
        tx.commit().expect("and commits");
        as_owner(&db, "GRANT SELECT ON vcon.audit TO vcon_app");
        assert_eq!(
            exported(&catalog)
                .iter()
                .map(|l| l["target"].clone())
                .collect::<Vec<_>>(),
            ["before", "after"],
            "both records were kept"
        );
    }

    #[test]
    fn a_write_that_fails_aborts_the_transaction_and_its_commit_is_rolled_back() {
        let (db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut tx = catalog.immediate().expect("begins");
        tx.add_wrap(KeyId([1; 8])).expect("a write before");
        as_owner(&db, "REVOKE INSERT ON vcon.tombstone FROM vcon_app");
        assert_eq!(
            tx.tombstone(a_uuid(1), 1),
            Err(StoreError::Io),
            "the write is refused"
        );
        assert_eq!(
            tx.wraps(KeyId([1; 8])),
            Err(StoreError::Io),
            "every later call fails"
        );
        assert_eq!(
            tx.add_wrap(KeyId([1; 8])),
            Err(StoreError::Io),
            "a write too"
        );
        assert_eq!(
            tx.commit(),
            Err(CommitError::RolledBack),
            "and the commit says nothing was kept"
        );
        let mut tx = catalog
            .immediate()
            .expect("the lock and the connection are free");
        assert_eq!(
            tx.wraps(KeyId([1; 8])),
            Ok(0),
            "the write before was not kept"
        );
        tx.commit().expect("commits");
    }

    #[test]
    fn rolling_back_a_part_ends_the_abort_its_failed_write_left() {
        let (db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        as_owner(&db, "REVOKE INSERT ON vcon.tombstone FROM vcon_app");
        let mut tx = catalog.immediate().expect("begins");
        tx.add_wrap(KeyId([1; 8])).expect("before the part");
        tx.begin_part().expect("a part");
        assert_eq!(
            tx.tombstone(a_uuid(1), 1),
            Err(StoreError::Io),
            "the part's write is refused"
        );
        tx.rollback_part().expect("back to the part's start");
        tx.add_wrap(KeyId([1; 8])).expect("the transaction goes on");
        tx.commit().expect("and commits");
        assert_eq!(
            catalog.snapshot().expect("begins").wraps(KeyId([1; 8])),
            Ok(2),
            "both writes outside the part were kept"
        );
    }

    #[test]
    fn a_commit_whose_connection_is_lost_is_unknown_and_the_connection_is_not_reused() {
        let (db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut tx = catalog.immediate().expect("begins");
        tx.add_wrap(KeyId([1; 8])).expect("writes");
        let ended = wait(async {
            // A role may end a session of its own.
            let same_role = db.app().await;
            value(
                &same_role,
                "SELECT count(pg_terminate_backend(pid)) FROM pg_stat_activity \
                 WHERE datname = current_database() AND state = 'idle in transaction' \
                   AND pid <> pg_backend_pid()",
            )
            .await
        });
        assert_eq!(ended, "1", "the one open transaction's connection is ended");
        assert_eq!(
            tx.commit(),
            Err(CommitError::Unknown),
            "no answer to COMMIT"
        );
        let mut tx = catalog.immediate().expect("a new connection");
        assert_eq!(
            tx.wraps(KeyId([1; 8])),
            Ok(0),
            "the server rolled the lost transaction back"
        );
        tx.commit().expect("commits");
    }

    #[test]
    fn the_commit_faults_a_test_sets_end_the_transaction_as_they_say() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        catalog.fail_commit(1, CommitError::Unknown);
        let mut tx = catalog.immediate().expect("begins");
        tx.add_wrap(KeyId([1; 8])).expect("writes");
        assert_eq!(
            tx.commit(),
            Err(CommitError::Unknown),
            "reported as unknown"
        );
        catalog.fail_commit(2, CommitError::RolledBack);
        let mut tx = catalog.immediate().expect("begins");
        tx.add_wrap(KeyId([1; 8])).expect("writes");
        tx.commit().expect("the first commit after is real");
        let mut tx = catalog.immediate().expect("begins");
        tx.add_wrap(KeyId([1; 8])).expect("writes");
        assert_eq!(
            tx.commit(),
            Err(CommitError::RolledBack),
            "the second is rolled back"
        );
        assert_eq!(
            catalog.snapshot().expect("begins").wraps(KeyId([1; 8])),
            Ok(2),
            "the unknown one was kept, the real one too, the rolled back one not"
        );
    }

    #[test]
    fn a_sealed_value_copied_to_another_row_does_not_open() {
        let (db, database) = database();
        let (tenant, _, catalog) = tenant_catalog(&database);
        let one = store_one(&catalog, a_uuid(1), 1);
        let two = store_one(&catalog, a_uuid(2), 2);
        owner_reads(
            &db,
            tenant,
            &format!(
                "UPDATE vcon.version SET form_sealed = \
                 (SELECT form_sealed FROM vcon.version WHERE version = '\\x{one}') \
                 WHERE version = '\\x{two}' RETURNING 'moved'"
            ),
        );
        let mut tx = catalog.snapshot().expect("begins");
        assert!(
            matches!(tx.version_row(two), Err(StoreError::Damaged)),
            "the form sealed for another version is damage"
        );
        assert!(
            tx.version_row(one).expect("reads").is_some(),
            "the row it was sealed for still opens"
        );
    }

    #[test]
    fn a_sealed_uuid_copied_to_another_vcon_does_not_open() {
        let (db, database) = database();
        let (tenant, _, catalog) = tenant_catalog(&database);
        store_one(&catalog, a_uuid(1), 1);
        store_one(&catalog, a_uuid(2), 2);
        owner_reads(
            &db,
            tenant,
            "UPDATE vcon.vcon SET uuid_sealed = \
             (SELECT uuid_sealed FROM vcon.vcon ORDER BY stored_at LIMIT 1) \
             WHERE stored_at = 2 RETURNING 'moved'",
        );
        let page = catalog.snapshot().expect("begins").page(&PageQuery {
            before: None,
            since: None,
            until: None,
            rows: 10,
        });
        assert!(
            matches!(page, Err(StoreError::Damaged)),
            "a uuid sealed for another row is damage, never another vCon's uuid"
        );
    }

    #[test]
    fn a_row_sealed_under_another_tenants_key_does_not_open() {
        let (db, database) = database();
        let (tenant, _, catalog) = tenant_catalog(&database);
        let (other, _, theirs) = tenant_catalog(&database);
        let version = store_one(&catalog, a_uuid(1), 1);
        let their_version = store_one(&theirs, a_uuid(1), 1);
        let sealed = owner_reads(
            &db,
            other,
            &format!(
                "SELECT encode(form_sealed, 'hex') FROM vcon.version \
                 WHERE version = '\\x{their_version}'"
            ),
        );
        owner_reads(
            &db,
            tenant,
            &format!(
                "UPDATE vcon.version SET form_sealed = '\\x{}' WHERE version = '\\x{version}' \
                 RETURNING 'planted'",
                sealed[0]
            ),
        );
        assert!(
            matches!(
                catalog.snapshot().expect("begins").version_row(version),
                Err(StoreError::Damaged)
            ),
            "another tenant's sealed value is damage here"
        );
    }

    #[test]
    fn a_sealed_value_cut_short_or_changed_is_damage() {
        let (db, database) = database();
        let (tenant, _, catalog) = tenant_catalog(&database);
        let version = store_one(&catalog, a_uuid(1), 1);
        for change in [
            "set_byte(size_sealed, 20, get_byte(size_sealed, 20) # 1)",
            "substring(size_sealed from 1 for 5)",
            "''::bytea",
        ] {
            owner_reads(
                &db,
                tenant,
                &format!("UPDATE vcon.version SET size_sealed = {change} RETURNING 'changed'"),
            );
            assert!(
                matches!(
                    catalog.snapshot().expect("begins").unpacked(),
                    Err(StoreError::Damaged)
                ),
                "{change}: the size does not open"
            );
        }
        assert!(
            catalog
                .snapshot()
                .expect("begins")
                .is_unpacked(version)
                .expect("a clear column still reads"),
            "the clear columns are not touched by it"
        );
    }

    #[test]
    fn a_backup_into_a_file_is_not_a_postgres_catalogs() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let dir = tempfile::tempdir().expect("a directory");
        assert!(
            matches!(
                catalog.backup_into(&dir.path().join("copy.sqlite")),
                Err(StoreError::Io)
            ),
            "the database's own backup is the catalog's"
        );
        assert_eq!(
            catalog.unpack_copy(&dir.path().join("copy.sqlite")),
            Err(StoreError::Io),
            "and there is no copy to unpack"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("reads").count(),
            0,
            "no file was made"
        );
    }

    #[test]
    fn a_count_adds_only_to_a_record_a_count_made() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        // The minute's first instant: a record and a count hold one time.
        let at = OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(3);
        let mut tx = catalog.immediate().expect("begins");
        tx.record(&entry("same"), at).expect("records");
        tx.count(&entry("same"), at).expect("counts");
        tx.commit().expect("commits");
        let counts: Vec<i64> = exported(&catalog)
            .iter()
            .map(|l| l["count"].as_i64().expect("a count"))
            .collect();
        assert_eq!(
            counts,
            [1, 1],
            "the request's own record is not changed by a later count"
        );
    }

    #[test]
    fn a_record_with_no_target_and_no_status_is_exported_with_nulls() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut tx = catalog.immediate().expect("begins");
        let version = VersionId::generate();
        tx.record(
            &Entry {
                actor: Actor::Sweep,
                action: Action::Purge,
                target: None,
                version: Some(version),
                status: crate::audit::NO_ANSWER,
            },
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("records");
        tx.count(
            &Entry {
                actor: Actor::Sweep,
                action: Action::Purge,
                target: None,
                version: None,
                status: crate::audit::NO_ANSWER,
            },
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("counts");
        tx.count(
            &Entry {
                actor: Actor::Sweep,
                action: Action::Purge,
                target: None,
                version: None,
                status: crate::audit::NO_ANSWER,
            },
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("counts the same record");
        tx.commit().expect("commits");
        let lines = exported(&catalog);
        assert_eq!(
            lines.len(),
            2,
            "one record and one counted record: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .all(|l| l["target"].is_null() && l["status"].is_null()),
            "no target and no status: {lines:?}"
        );
        assert_eq!(
            (lines[0]["version"].as_str(), lines[1]["count"].as_i64()),
            (Some(version.to_string().as_str()), Some(2)),
            "the version as text, and NULLs matched by the count"
        );
    }

    #[test]
    fn a_hold_a_delete_and_a_tombstone_read_back() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let version = store_one(&catalog, a_uuid(1), 10);
        let mut tx = catalog.immediate().expect("begins");
        tx.set_hold(a_uuid(1), true).expect("holds");
        tx.mark_deleted(a_uuid(1), 20).expect("deletes");
        let hidden = tx
            .hidden_row(version)
            .expect("reads")
            .expect("a hidden row");
        assert_eq!(
            (hidden.uuid, hidden.stored_at, hidden.held, hidden.deleted),
            (a_uuid(1), 10, true, true),
            "the uuid opened, and the hold and the delete"
        );
        assert_eq!(tx.count_held(), Ok(1), "one vCon held");
        assert_eq!(
            tx.hidden_versions(Some(a_uuid(1))),
            Ok(vec![version]),
            "the version is hidden"
        );
        assert_eq!(
            tx.hidden_versions(Some(a_uuid(2))),
            Ok(vec![]),
            "none for another uuid"
        );
        tx.delete_version(version).expect("purges the version");
        tx.tombstone(a_uuid(1), 30).expect("and leaves a tombstone");
        assert_eq!(tx.tombstoned(a_uuid(1)), Ok(true), "the uuid is gone");
        assert_eq!(tx.tombstoned(a_uuid(2)), Ok(false), "another is not");
        assert!(
            tx.vcon_row(a_uuid(1)).expect("reads").is_none(),
            "and its vCon row with it"
        );
        tx.commit().expect("commits");
    }

    #[test]
    fn a_page_with_no_vcon_and_a_cursor_past_the_last_are_empty() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let query = |before| PageQuery {
            before,
            since: None,
            until: None,
            rows: 5,
        };
        assert_eq!(
            catalog
                .snapshot()
                .expect("begins")
                .page(&query(None))
                .expect("a page")
                .len(),
            0,
            "an empty tenant"
        );
        store_one(&catalog, a_uuid(1), 50);
        assert_eq!(
            catalog
                .snapshot()
                .expect("begins")
                .page(&query(Some((50, a_uuid(1)))))
                .expect("a page")
                .len(),
            0,
            "nothing before the only vCon"
        );
        assert_eq!(
            catalog
                .snapshot()
                .expect("begins")
                .page(&PageQuery {
                    before: None,
                    since: Some(51),
                    until: None,
                    rows: 5
                })
                .expect("a page")
                .len(),
            0,
            "nothing stored since a later time"
        );
    }

    #[test]
    fn a_snapshot_takes_no_write() {
        let (_db, database) = database();
        let (_, _, catalog) = tenant_catalog(&database);
        let mut tx = catalog.snapshot().expect("begins");
        assert_eq!(
            tx.add_wrap(KeyId([1; 8])),
            Err(StoreError::Io),
            "a read-only transaction refuses the write"
        );
        assert_eq!(
            tx.commit(),
            Err(CommitError::RolledBack),
            "and ends rolled back"
        );
    }

    #[test]
    fn a_store_call_costs_a_fixed_number_of_round_trips() {
        let (_db, database) = database();
        let (s3, _) = bucket(false);
        let harness = CatalogAndS3 {
            catalog: postgres_catalogs(&database),
            s3,
            packed: false,
        };
        let fresh = harness.fresh(TestClock::at(conformance::START));
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let uuid = "01a07da8-c2bb-83e5-b9a2-279e0d16bc01";
        let (found, body) = conformance::vcon(uuid, "counted");
        let start = database.round_trips();
        fresh.store.create(&who, &found, &body).expect("creates");
        let create = database.round_trips() - start;
        let start = database.round_trips();
        fresh.store.get(&who, found.uuid).expect("reads");
        let get = database.round_trips() - start;
        let start = database.round_trips();
        fresh
            .store
            .list(
                &who,
                &ListQuery {
                    limit: 10,
                    cursor: None,
                    since: None,
                    until: None,
                },
            )
            .expect("lists");
        let list = database.round_trips() - start;
        assert_eq!(
            (create, get, list),
            (CREATE_TRIPS, GET_TRIPS, LIST_TRIPS),
            "create, get and a list page, each counted at the database"
        );
    }
}
