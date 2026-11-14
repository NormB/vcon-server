//! The registry and the tenants a server or a command opens with the
//! catalogs in PostgreSQL (`Tenants::open_in`, `Tenants::open_beside_in`
//! with `Catalogs::Postgres`): the rows are in the database, the objects
//! in the bucket, and the data directory holds neither. They need a
//! PostgreSQL server and an S3-compatible service.

use crate::tenants::{RegistryError, TenantError};

// What must work.

#[test]
fn a_database_that_cannot_be_opened_says_it_is_the_database() {
    let error = RegistryError::Database("the tables could not be read".to_owned());
    assert_eq!(
        error.to_string(),
        "the registry in the PostgreSQL database cannot be opened: the tables could not be read",
        "no file is named"
    );
}

// What must be refused.

#[test]
fn another_master_key_on_a_database_names_the_database_and_no_file() {
    let said = RegistryError::WrongDatabaseKey.to_string();
    assert!(
        said.contains("the catalog database") && !said.contains("server.sqlite"),
        "the registry is not in server.sqlite: {said}"
    );
}

// The edges.

#[test]
fn a_caller_that_does_not_report_the_open_sees_the_same_errors_as_for_files() {
    assert_eq!(
        TenantError::from(RegistryError::WrongDatabaseKey),
        TenantError::WrongKey,
        "the wrong key"
    );
    assert_eq!(
        TenantError::from(RegistryError::Database("why".to_owned())),
        TenantError::Io,
        "a database that cannot be read"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::super::schema_tests::live::{Db, in_scope};
    use super::super::space_tests::on_a_server::rows;
    use crate::audit::Actor;
    use crate::backend::conformance::{self, TestClock};
    use crate::backend::s3::remote::{bucket, secret_file};
    use crate::backend::{Catalogs, ObjectPlace, catalog_for, objects_for};
    use crate::config::{CatalogStore, ObjectStore, Storage};
    use crate::datadir::Sharing;
    use crate::envelope::Key;
    use crate::fs::OsFs;
    use crate::ids::{TenantId, VersionId};
    use crate::store::Caller;
    use crate::tenants::{BackupError, RegistryError, Removal, TenantError, Tenants};

    /// Runs `call` on a runtime of its own.
    fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    /// A deployment: a migrated database, a bucket, a data directory and a
    /// master key file, all the test's own.
    struct World {
        db: Db,
        catalog: CatalogStore,
        storage: Storage,
        dir: tempfile::TempDir,
        _settings: tempfile::TempDir,
        _secret_file: (tempfile::TempDir, PathBuf),
    }

    /// A file of 32 random bytes, mode 0600, as a master key file is.
    fn key_file(path: &Path) {
        let key: [u8; 32] = std::array::from_fn(|_| VersionId::generate().0[0]);
        std::fs::write(path, key).expect("a master key file");
        std::fs::set_permissions(
            path,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("chmod 0600");
    }

    fn world() -> World {
        let db = wait(Db::migrated());
        let (settings_dir, settings) = db.settings("vcon_app");
        let secret_file = secret_file();
        let (_s3, mut s3_settings) = bucket(false);
        s3_settings.secret_access_key_file = Some(secret_file.1.clone());
        let dir = tempfile::tempdir().expect("a directory");
        std::fs::create_dir(dir.path().join("data")).expect("a data directory");
        key_file(&dir.path().join("master.key"));
        World {
            db,
            catalog: CatalogStore::Postgres(Box::new(settings)),
            storage: Storage {
                objects: ObjectStore::S3(s3_settings),
                parquet: None,
            },
            dir,
            _settings: settings_dir,
            _secret_file: secret_file,
        }
    }

    impl World {
        fn data(&self) -> PathBuf {
            self.dir.path().join("data")
        }

        fn master(&self, file: &str) -> Key {
            Key::load_master(&self.dir.path().join(file)).expect("the master key")
        }

        /// The catalogs and the bucket, as a start of `sharing` finds
        /// them.
        fn places(&self, sharing: Sharing) -> (Catalogs, ObjectPlace) {
            let catalogs = catalog_for(&self.catalog, sharing).expect("connects");
            let objects = objects_for(&self.storage, &self.data(), &catalogs).expect("the bucket");
            (catalogs, objects)
        }

        /// The registry as a server opens it, under the key in `file`.
        fn serve_under(&self, file: &str) -> Result<(Catalogs, Tenants), RegistryError> {
            let (catalogs, objects) = self.places(Sharing::Serve);
            let opened = Tenants::open_in(
                &self.data(),
                self.master(file),
                Arc::new(OsFs),
                TestClock::at(conformance::START),
                objects,
                &catalogs,
            );
            if opened.is_err() {
                give_up(&catalogs);
            }
            opened.map(|tenants| (catalogs, tenants))
        }

        fn serve(&self) -> (Catalogs, Tenants) {
            self.serve_under("master.key").expect("the registry opens")
        }

        /// The registry as a command beside a server opens it.
        fn beside(&self) -> (Catalogs, Tenants) {
            let (catalogs, objects) = self.places(Sharing::Beside);
            let tenants = Tenants::open_beside_in(
                &self.data(),
                self.master("master.key"),
                Arc::new(OsFs),
                TestClock::at(conformance::START),
                objects,
                &catalogs,
            )
            .expect("the registry opens beside the server");
            (catalogs, tenants)
        }

        /// How many rows a table of the registry holds.
        fn registry_rows(&self, table: &str) -> i64 {
            let sql = format!("SELECT count(*) FROM vcon.{table}");
            wait(async { in_scope(&self.db.app().await, "registry", &sql).await })
                .unwrap_or_else(|e| panic!("{sql}: {e}"))
                .iter()
                .find_map(|m| match m {
                    tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0),
                    _ => None,
                })
                .expect("a count")
                .parse()
                .expect("a number")
        }

        /// How many rows of `tenant` a table of the catalogs holds.
        fn rows(&self, tenant: TenantId, table: &str) -> i64 {
            rows(&self.db, tenant, table)
        }

        /// Deletes every row of the registry's tenant table, as the
        /// schema's owner, and leaves each tenant's catalog: what a
        /// create cut off before its commit leaves.
        fn without_the_tenant_rows(&self) {
            self.as_owner(
                "ALTER TABLE vcon.tenant DISABLE ROW LEVEL SECURITY; DELETE FROM vcon.tenant; \
                 ALTER TABLE vcon.tenant ENABLE ROW LEVEL SECURITY",
            );
        }

        /// Runs `sql` as the schema's owner.
        fn as_owner(&self, sql: &str) {
            wait(async {
                let owner = self.db.admin().await;
                owner
                    .batch_execute("SET ROLE vcon_owner")
                    .await
                    .expect("the owner role");
                owner
                    .batch_execute(sql)
                    .await
                    .unwrap_or_else(|e| panic!("{sql}: {e}"));
            });
        }
    }

    /// Ends a server or a command: its locks are given up.
    fn give_up(catalogs: &Catalogs) {
        catalogs.give_up_locks();
    }

    fn stop((catalogs, tenants): (Catalogs, Tenants)) {
        drop(tenants);
        give_up(&catalogs);
    }

    /// Stores one vCon for `tenant` and reads it back.
    fn store_one(tenants: &Tenants, tenant: TenantId, n: u8) -> uuid::Uuid {
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let uuid = format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}");
        let (found, body) = conformance::vcon(&uuid, "stored");
        let store = tenants.store(tenant).expect("the tenant's store");
        store.create(&who, &found, &body).expect("creates");
        store.get(&who, found.uuid).expect("reads");
        found.uuid
    }

    fn names(tenants: &Tenants) -> Vec<String> {
        tenants
            .list(Actor::Cli)
            .expect("lists")
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    /// The names in the data directory, sorted.
    fn files(w: &World) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(w.data())
            .expect("lists")
            .map(|e| {
                e.expect("an entry")
                    .file_name()
                    .into_string()
                    .expect("UTF-8")
            })
            .collect();
        names.sort();
        names
    }

    // What must work.

    #[test]
    fn a_tenant_made_on_a_database_is_a_row_there_and_is_listed() {
        let w = world();
        let (_catalogs, tenants) = w.serve();
        let id = tenants.create(Actor::Cli, "acme", None).expect("makes");
        assert_eq!(names(&tenants), ["acme"], "listed");
        assert_eq!(
            w.registry_rows("tenant"),
            1,
            "one row of the registry's table"
        );
        assert_eq!(w.rows(id, "tenant_catalog"), 1, "and the tenant's catalog");
    }

    #[test]
    fn a_vcon_stored_on_a_database_is_read_back() {
        let w = world();
        let (_catalogs, tenants) = w.serve();
        let id = tenants.create(Actor::Cli, "acme", None).expect("makes");
        store_one(&tenants, id, 1);
        assert_eq!(
            w.rows(id, "version"),
            1,
            "its version is a row of the database"
        );
    }

    #[test]
    fn the_next_server_finds_the_tenants_and_their_vcons() {
        let w = world();
        let first = w.serve();
        let id = first.1.create(Actor::Cli, "acme", None).expect("makes");
        let uuid = store_one(&first.1, id, 1);
        stop(first);
        let (_catalogs, tenants) = w.serve();
        assert_eq!(names(&tenants), ["acme"], "the registry is the database's");
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let read = tenants.store(id).expect("the store").get(&who, uuid);
        assert!(read.is_ok(), "the vCon stored before the restart reads");
    }

    #[test]
    fn a_command_beside_a_server_reads_what_the_server_wrote() {
        let w = world();
        let (_catalogs, server) = w.serve();
        server.create(Actor::Cli, "acme", None).expect("makes");
        let (_beside, command) = w.beside();
        assert_eq!(names(&command), ["acme"], "one registry, in the database");
    }

    #[test]
    fn a_server_reads_what_a_command_beside_it_wrote() {
        let w = world();
        let (_catalogs, server) = w.serve();
        let (_beside, command) = w.beside();
        command
            .create(Actor::Cli, "from-a-command", None)
            .expect("makes");
        assert_eq!(names(&server), ["from-a-command"], "one registry");
    }

    #[test]
    fn a_tenant_removed_leaves_no_row_of_its_catalog() {
        let w = world();
        let (_catalogs, tenants) = w.serve();
        let id = tenants.create(Actor::Cli, "acme", None).expect("makes");
        store_one(&tenants, id, 1);
        tenants
            .remove(Actor::Cli, id, Removal::Force)
            .expect("removes");
        assert_eq!(names(&tenants), Vec::<String>::new(), "not listed");
        assert_eq!(w.rows(id, "tenant_catalog"), 0, "its catalog is gone");
        assert_eq!(w.rows(id, "version"), 0, "and its versions");
    }

    #[test]
    fn the_keys_of_a_database_are_in_order_for_a_start() {
        let w = world();
        let (_catalogs, tenants) = w.serve();
        tenants.create(Actor::Cli, "acme", None).expect("makes");
        let order = tenants.key_order().expect("examines");
        assert!(order.is_sound(), "as serve checks before it listens");
        assert_eq!(
            tenants.tenants_with_packed_versions().expect("counts"),
            0,
            "and nothing is packed"
        );
    }

    #[test]
    fn a_tenant_key_is_rotated_on_a_database() {
        let w = world();
        let (_catalogs, tenants) = w.serve();
        let id = tenants.create(Actor::Cli, "acme", None).expect("makes");
        let uuid = store_one(&tenants, id, 1);
        tenants.rekey_tenant(Actor::Cli, id).expect("rotates");
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let read = tenants.store(id).expect("the store").get(&who, uuid);
        assert!(read.is_ok(), "the vCon reads under the new tenant key");
    }

    // What must be refused.

    #[test]
    fn another_master_key_is_refused_as_the_database_s_wrong_key() {
        let w = world();
        stop(w.serve());
        key_file(&w.dir.path().join("other.key"));
        let refused = w.serve_under("other.key").err();
        assert!(
            matches!(refused, Some(RegistryError::WrongDatabaseKey)),
            "the database names the key it was made under: {refused:?}"
        );
    }

    #[test]
    fn local_objects_with_a_database_are_refused_and_nothing_is_written() {
        let w = world();
        let catalogs = catalog_for(&w.catalog, Sharing::Serve).expect("connects");
        let refused = Tenants::open_in(
            &w.data(),
            w.master("master.key"),
            Arc::new(OsFs),
            TestClock::at(conformance::START),
            ObjectPlace::Local,
            &catalogs,
        )
        .err();
        let Some(RegistryError::Database(why)) = refused else {
            panic!("refused as the database's: {refused:?}");
        };
        assert!(
            why.contains("storage.objects = \"s3\""),
            "names the setting: {why}"
        );
        assert_eq!(
            w.registry_rows("registry_key"),
            0,
            "the database names no key yet"
        );
    }

    #[test]
    fn a_registry_that_cannot_be_read_is_an_error_of_the_database() {
        let w = world();
        w.as_owner("REVOKE SELECT ON vcon.registry_key FROM vcon_app");
        let refused = w.serve_under("master.key").err();
        assert!(
            matches!(refused, Some(RegistryError::Database(_))),
            "not a wrong key and not a file: {refused:?}"
        );
    }

    #[test]
    fn a_backup_into_a_directory_is_refused_and_leaves_no_directory() {
        let w = world();
        let (_catalogs, tenants) = w.serve();
        tenants.create(Actor::Cli, "acme", None).expect("makes");
        let to = w.dir.path().join("backup");
        assert_eq!(
            tenants.backup(Actor::Cli, &to),
            Err(BackupError::NoFileBackup),
            "the registry is not a file"
        );
        assert!(!to.exists(), "nothing is left at the backup's path");
    }

    #[test]
    fn a_tenant_that_is_not_there_is_not_found_on_a_database() {
        let w = world();
        let (_catalogs, tenants) = w.serve();
        let missing = tenants.store(TenantId::generate()).err();
        assert_eq!(missing, Some(TenantError::NotFound), "as with files");
    }

    // The edges.

    #[test]
    fn the_data_directory_holds_no_registry_and_no_tenant() {
        let w = world();
        let (_catalogs, tenants) = w.serve();
        let id = tenants.create(Actor::Cli, "acme", None).expect("makes");
        store_one(&tenants, id, 1);
        assert_eq!(
            files(&w),
            Vec::<String>::new(),
            "no server.sqlite, no tenants directory, no claim file"
        );
    }

    #[test]
    fn a_start_removes_a_catalog_whose_tenant_has_no_row() {
        let w = world();
        let first = w.serve();
        let id = first.1.create(Actor::Cli, "acme", None).expect("makes");
        stop(first);
        // A create cut off before its commit: the catalog with no row.
        w.without_the_tenant_rows();
        assert_eq!(
            w.rows(id, "tenant_catalog"),
            1,
            "the tenant's catalog is left"
        );
        let (_catalogs, tenants) = w.serve();
        assert_eq!(names(&tenants), Vec::<String>::new(), "no tenant");
        assert_eq!(
            w.rows(id, "tenant_catalog"),
            0,
            "start-up recovery removed the catalog"
        );
    }

    #[test]
    fn a_command_beside_a_server_removes_no_catalog_a_create_is_making() {
        let w = world();
        let first = w.serve();
        let id = first.1.create(Actor::Cli, "acme", None).expect("makes");
        w.without_the_tenant_rows();
        let (_beside, _command) = w.beside();
        assert_eq!(
            w.rows(id, "tenant_catalog"),
            1,
            "a command beside a server runs no start-up recovery"
        );
    }

    #[test]
    fn the_same_master_key_opens_the_database_again_and_again() {
        let w = world();
        for round in 0..3 {
            let opened = w.serve();
            assert_eq!(
                w.registry_rows("registry_key"),
                1,
                "one key is named, round {round}"
            );
            stop(opened);
        }
    }

    /// A stopped process's lock goes when PostgreSQL sees its session
    /// closed. A server gives it up itself, before that: here its
    /// connections stay open, held by a second handle on the database, and
    /// the next server still finds the lock free.
    #[test]
    fn a_server_that_ends_gives_the_database_up_while_its_connections_are_open() {
        let w = world();
        let (catalogs, tenants) = w.serve();
        let held = catalogs.clone();
        let lock = crate::datadir::lock(&w.data(), Sharing::Serve).expect("the data directory");
        // No certificate is at these paths: the server ends as soon as it
        // has opened, as one whose listener fails does.
        let at = |name: &str| w.dir.path().join(name);
        let config = crate::config::parse(&format!(
            "listen = \"127.0.0.1:0\"\ndata_dir = {:?}\nmaster_key = {:?}\n\n[tls]\ncert = \
             {:?}\nkey = {:?}\n",
            w.data(),
            at("master.key"),
            at("no-cert.pem"),
            at("no-key.pem"),
        ))
        .expect("the configuration reads");
        let ended = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(crate::serve::run_opened(
                &config,
                (lock, catalogs, tenants),
                std::future::pending(),
            ));
        assert!(
            matches!(ended, Err(crate::serve::ServeError::Tls(_))),
            "it ended for its certificate: {ended:?}"
        );
        let next = catalog_for(&w.catalog, Sharing::Serve);
        assert!(
            matches!(next, Ok(Catalogs::Postgres(_))),
            "the lock was given up, not left to the end of the session: {next:?}"
        );
        drop(held);
    }

    /// The deployment's configuration, as its file would read.
    fn config(w: &World) -> crate::config::Config {
        let at = |name: &str| w.dir.path().join(name);
        let mut config = crate::config::parse(&format!(
            "listen = \"127.0.0.1:0\"\ndata_dir = {:?}\nmaster_key = {:?}\n\n[tls]\ncert = \
             {:?}\nkey = {:?}\n",
            w.data(),
            at("master.key"),
            at("no-cert.pem"),
            at("no-key.pem"),
        ))
        .expect("the configuration reads");
        config.storage = w.storage.clone();
        config.catalog = w.catalog.clone();
        config
    }

    #[test]
    fn a_start_refused_after_it_connected_gives_the_database_up() {
        let w = world();
        stop(w.serve());
        key_file(&w.dir.path().join("other.key"));
        let catalogs = catalog_for(&w.catalog, Sharing::Serve).expect("connects");
        let refused = crate::serve::open_or_give_up(&config(&w), w.master("other.key"), &catalogs);
        assert!(
            matches!(refused, Err(crate::serve::ServeError::WrongDatabaseKey)),
            "the wrong master key: {:?}",
            refused.err()
        );
        // This start's connections are still open.
        let next = catalog_for(&w.catalog, Sharing::Serve);
        assert!(
            matches!(next, Ok(Catalogs::Postgres(_))),
            "the refused start holds nothing: {next:?}"
        );
        drop(catalogs);
    }

    #[test]
    fn a_command_that_has_ended_holds_no_lock_while_its_connections_are_open() {
        let w = world();
        stop(w.serve());
        let catalogs = catalog_for(&w.catalog, Sharing::Alone).expect("connects");
        let mut printed = Vec::new();
        crate::commands::run_and_give_up(
            &config(&w),
            &crate::commands::Command::TenantList,
            w.master("master.key"),
            &catalogs,
            &mut printed,
        )
        .expect("lists no tenant");
        assert_eq!(printed, b"", "a new database has none");
        let next = catalog_for(&w.catalog, Sharing::Alone);
        assert!(
            matches!(next, Ok(Catalogs::Postgres(_))),
            "the next command that runs alone has the database: {next:?}"
        );
        drop(catalogs);
    }
}
