//! Where a start finds its catalogs and binds its bucket
//! (`backend::catalog_for`, `backend::objects_for`): the database a
//! PostgreSQL catalog names, connected with the locks of the work, and the
//! bucket's prefix claimed in that database's row, not in a file of the
//! data directory. They need a PostgreSQL server and an S3-compatible
//! service.

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::schema::SERVER_LOCK_HELD;
    use super::super::schema_tests::live::{Db, in_scope};
    use crate::backend::s3::claim::FILE;
    use crate::backend::s3::remote::{bucket, secret_file};
    use crate::backend::{Catalogs, ObjectPlace, catalog_for, objects_for};
    use crate::config::{CatalogStore, ObjectStore, Storage, StorageError};
    use crate::datadir::Sharing;

    /// Runs `call` on a runtime of its own.
    fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    /// A database of the test's own, migrated or empty, and the
    /// configuration's catalog naming it as the application role. The
    /// directory holds the password file.
    fn catalog(migrated: bool) -> (Db, tempfile::TempDir, CatalogStore) {
        let db = wait(async {
            if migrated {
                Db::migrated().await
            } else {
                Db::fresh().await
            }
        });
        let (dir, settings) = db.settings("vcon_app");
        (db, dir, CatalogStore::Postgres(Box::new(settings)))
    }

    /// The `[storage]` table for a new bucket, with the secret in a file.
    fn storage() -> (Storage, (tempfile::TempDir, std::path::PathBuf)) {
        let secret = secret_file();
        let (_s3, mut settings) = bucket(false);
        settings.secret_access_key_file = Some(secret.1.clone());
        let storage = Storage {
            objects: ObjectStore::S3(settings),
            parquet: None,
        };
        (storage, secret)
    }

    /// The one row of the claim's table, read in a transaction of the
    /// registry, or "" when the table is empty.
    fn claim_row(db: &Db) -> String {
        wait(async {
            let rows = in_scope(
                &db.app().await,
                "registry",
                "SELECT coalesce(string_agg(claim_id, ','), '') FROM vcon.storage_claim",
            )
            .await
            .expect("reads the table");
            rows.iter()
                .find_map(|m| match m {
                    tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_owned),
                    _ => None,
                })
                .expect("one row")
        })
    }

    // What must work.

    #[test]
    fn a_postgresql_catalog_is_the_database_connected() {
        let (_db, _dir, catalog) = catalog(true);
        let found = catalog_for(&catalog, Sharing::Serve).expect("connects");
        assert!(
            matches!(found, Catalogs::Postgres(_)),
            "the catalogs are in the database: {found:?}"
        );
    }

    #[test]
    fn a_server_holds_the_database_against_a_second_server() {
        let (_db, _dir, catalog) = catalog(true);
        let _first = catalog_for(&catalog, Sharing::Serve).expect("the first server connects");
        let StorageError(why) =
            catalog_for(&catalog, Sharing::Serve).expect_err("the second is refused");
        assert_eq!(
            why,
            format!("catalog.postgres: {SERVER_LOCK_HELD}"),
            "one server serves a database"
        );
    }

    #[test]
    fn a_command_beside_a_server_connects_while_the_server_serves() {
        let (_db, _dir, catalog) = catalog(true);
        let _server = catalog_for(&catalog, Sharing::Serve).expect("the server connects");
        let beside = catalog_for(&catalog, Sharing::Beside);
        assert!(
            matches!(beside, Ok(Catalogs::Postgres(_))),
            "a command beside a server takes nothing the server holds: {beside:?}"
        );
    }

    #[test]
    fn a_bucket_is_claimed_in_the_database_and_no_file_is_written() {
        let (db, _dir, catalog) = catalog(true);
        let (storage, _secret) = storage();
        let data = tempfile::tempdir().expect("a data directory");
        let catalogs = catalog_for(&catalog, Sharing::Serve).expect("connects");
        let place = objects_for(&storage, data.path(), &catalogs).expect("claims the prefix");
        assert!(matches!(place, ObjectPlace::S3(_)), "the bucket");
        let row = claim_row(&db);
        assert_eq!(row.len(), 32, "the database keeps the claim id: {row:?}");
        assert!(
            !data.path().join(FILE).exists(),
            "the data directory holds no claim file"
        );
    }

    #[test]
    fn with_a_local_catalog_the_claim_stays_in_the_data_directory() {
        let (storage, _secret) = storage();
        let data = tempfile::tempdir().expect("a data directory");
        let place = objects_for(&storage, data.path(), &Catalogs::Local).expect("claims");
        assert!(matches!(place, ObjectPlace::S3(_)), "the bucket");
        assert!(
            data.path().join(FILE).exists(),
            "the data directory keeps the claim, as before"
        );
    }

    #[test]
    fn a_start_inside_a_runtime_reaches_the_database_from_a_blocking_task() {
        let (_db, _dir, catalog) = catalog(true);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime");
        let found = runtime.block_on(async move {
            tokio::task::spawn_blocking(move || catalog_for(&catalog, Sharing::Serve))
                .await
                .expect("the task ends without a panic")
        });
        assert!(
            matches!(found, Ok(Catalogs::Postgres(_))),
            "as serve calls it: {found:?}"
        );
    }

    // What must be refused.

    #[test]
    fn a_command_alone_is_refused_while_a_server_serves() {
        let (_db, _dir, catalog) = catalog(true);
        let _server = catalog_for(&catalog, Sharing::Serve).expect("the server connects");
        let StorageError(why) = catalog_for(&catalog, Sharing::Alone).expect_err("refused");
        assert!(
            why.starts_with("catalog.postgres: a server or another command holds"),
            "says what holds the database: {why}"
        );
    }

    #[test]
    fn a_server_is_refused_while_a_command_alone_runs() {
        let (_db, _dir, catalog) = catalog(true);
        let _command = catalog_for(&catalog, Sharing::Alone).expect("the command connects");
        let StorageError(why) = catalog_for(&catalog, Sharing::Serve).expect_err("refused");
        assert_eq!(
            why,
            format!("catalog.postgres: {SERVER_LOCK_HELD}"),
            "the server's refusal names a command that runs alone"
        );
    }

    #[test]
    fn a_database_that_was_never_migrated_is_refused_with_the_command_to_run() {
        let (_db, _dir, catalog) = catalog(false);
        let StorageError(why) = catalog_for(&catalog, Sharing::Serve).expect_err("refused");
        assert!(
            why.starts_with("catalog.postgres: ") && why.contains("vcon-server migrate"),
            "names the command: {why}"
        );
    }

    #[test]
    fn a_second_database_is_refused_the_bucket_of_the_first() {
        let (first_db, _first_dir, first) = catalog(true);
        let (second_db, _second_dir, second) = catalog(true);
        let (storage, _secret) = storage();
        let data = tempfile::tempdir().expect("a data directory");
        let first = catalog_for(&first, Sharing::Serve).expect("connects");
        objects_for(&storage, data.path(), &first).expect("the first claims the prefix");
        let second = catalog_for(&second, Sharing::Serve).expect("connects");
        let StorageError(why) =
            objects_for(&storage, data.path(), &second).expect_err("the second is refused");
        assert!(
            why.contains("database") && !why.contains("data directory"),
            "the refusal names a database, not a data directory: {why}"
        );
        assert_eq!(claim_row(&second_db), "", "the second keeps no claim");
        assert_eq!(claim_row(&first_db).len(), 32, "the first keeps its own");
    }

    // The edges.

    #[test]
    fn the_last_holder_of_a_database_may_end_inside_a_runtime() {
        let (_db, _dir, catalog) = catalog(true);
        let catalogs = catalog_for(&catalog, Sharing::Serve).expect("connects");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        // A start that is refused after the catalogs were opened drops
        // them in serve's async body.
        runtime.block_on(async move { drop(catalogs) });
    }

    #[test]
    fn a_database_whose_holder_ended_is_free_for_the_next_server() {
        let (_db, _dir, catalog) = catalog(true);
        let first = catalog_for(&catalog, Sharing::Serve).expect("connects");
        first.give_up_locks();
        // The first holder's connections are still open: the lock went
        // because it was given up, not because a session ended.
        let next = catalog_for(&catalog, Sharing::Serve);
        assert!(
            matches!(next, Ok(Catalogs::Postgres(_))),
            "the lock was given up: {next:?}"
        );
        drop(first);
    }

    #[test]
    fn a_second_start_on_the_database_binds_the_bucket_with_the_claim_kept() {
        let (db, _dir, catalog) = catalog(true);
        let (storage, _secret) = storage();
        let data = tempfile::tempdir().expect("a data directory");
        let first = catalog_for(&catalog, Sharing::Serve).expect("connects");
        objects_for(&storage, data.path(), &first).expect("claims");
        let kept = claim_row(&db);
        let Catalogs::Postgres(database) = &first else {
            panic!("a database: {first:?}");
        };
        database.give_up_lock();
        drop(first);
        let other_host = tempfile::tempdir().expect("another host's data directory");
        let next = catalog_for(&catalog, Sharing::Serve).expect("connects");
        objects_for(&storage, other_host.path(), &next)
            .expect("the claim is the database's, whichever data directory starts");
        assert_eq!(claim_row(&db), kept, "the same claim");
    }
}
