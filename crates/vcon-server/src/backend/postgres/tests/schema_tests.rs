//! The schema's tests: the migrations, `migrate` itself, the version and
//! role refusals, and row-level security. The pure rules run with no
//! server; the others are marked live, as in the parent module, and run
//! against the servers `scripts/pg-test-server.sh` starts. Each live test
//! has a database of its own, because a migration changes the whole
//! database and tests run at the same time.

mod edge;
mod failure;
mod success;

#[cfg(feature = "backend-tests")]
pub(super) mod live {
    use tokio_postgres::Client;

    use super::super::super::connect::{Connected, Purpose};
    use super::super::super::schema::{self, MIGRATIONS, Migrated};
    use super::super::live::{as_user, value};
    use super::super::{ConnectError, Connector, Secrets};
    use crate::config::PostgresSettings;

    /// A tenant id, as the policy function reads it.
    pub(crate) const TENANT_A: &str = "00112233445566778899aabbccddeeff";

    /// Another tenant's id.
    pub(crate) const TENANT_B: &str = "ffeeddccbbaa99887766554433221100";

    /// Every table of the catalog: a test names all of them, so a new
    /// table cannot be missed by the refusal tests.
    pub(crate) const TENANT_TABLES: &[&str] = &[
        "audit",
        "key_use",
        "manifest",
        "parquet_file",
        "purge_pending",
        "tenant_catalog",
        "tenant_lock",
        "tombstone",
        "vcon",
        "version",
    ];

    /// The registry's tables.
    pub(crate) const REGISTRY_TABLES: &[&str] =
        &["admin_audit", "api_key", "master_key_use", "tenant"];

    /// A database of a test's own, made empty.
    pub(crate) struct Db {
        pub(crate) name: String,
        _settings_dir: tempfile::TempDir,
    }

    impl Db {
        /// A new empty database owned by `vcon_owner`, so a migration can
        /// make its schema.
        pub(crate) async fn fresh() -> Db {
            let (dir, admin) = as_user("vcon_tester");
            let bytes = aws_lc_rs::rand::generate::<[u8; 8]>(&aws_lc_rs::rand::SystemRandom::new())
                .expect("random")
                .expose();
            let name = format!("vcon_t_{}", hex(&bytes));
            let connected = connect(&admin, Purpose::Connect)
                .await
                .expect("the tester connects");
            connected
                .client
                .batch_execute(&format!("CREATE DATABASE {name} OWNER vcon_owner"))
                .await
                .expect("creates the database");
            Db {
                name,
                _settings_dir: dir,
            }
        }

        /// Settings logging in to this database as `user`.
        pub(crate) fn settings(&self, user: &str) -> (tempfile::TempDir, PostgresSettings) {
            let (dir, mut settings) = as_user(user);
            settings.dbname = self.name.clone();
            (dir, settings)
        }

        /// Connects to this database as `user` for `purpose`.
        pub(crate) async fn connect(
            &self,
            user: &str,
            purpose: Purpose,
        ) -> Result<Connected, ConnectError> {
            let (_dir, settings) = self.settings(user);
            connect(&settings, purpose).await
        }

        /// The tester's connection to this database.
        pub(crate) async fn admin(&self) -> Client {
            self.connect("vcon_tester", Purpose::Connect)
                .await
                .expect("the tester connects")
                .client
        }

        /// The application role's connection, for serving.
        pub(crate) async fn app(&self) -> Client {
            self.connect("vcon_app", Purpose::Serve)
                .await
                .expect("the application role connects")
                .client
        }

        /// `migrate`, applying `migrations`.
        pub(crate) async fn migrate_with(&self, migrations: &[&str]) -> Result<Migrated, String> {
            let mut connected = self
                .connect("vcon_migrate", Purpose::Migrate)
                .await
                .map_err(|e| e.to_string())?;
            schema::run(&mut connected.client, migrations).await
        }

        /// `migrate` with this binary's migrations.
        pub(crate) async fn migrate(&self) -> Migrated {
            self.migrate_with(MIGRATIONS).await.expect("migrates")
        }

        /// A migrated database.
        pub(crate) async fn migrated() -> Db {
            let db = Db::fresh().await;
            db.migrate().await;
            db
        }
    }

    /// A connection for `purpose` with `settings`.
    pub(crate) async fn connect(
        settings: &PostgresSettings,
        purpose: Purpose,
    ) -> Result<Connected, ConnectError> {
        let secrets = Secrets::read(settings).expect("the secret files read");
        Connector::new(settings, secrets, purpose)
            .expect("the connector")
            .connect()
            .await
    }

    /// One statement in a transaction with `vcon.tenant` set to `scope`,
    /// or the server's message; the transaction is committed on success
    /// and rolled back on failure.
    pub(crate) async fn in_scope(
        client: &Client,
        scope: &str,
        statement: &str,
    ) -> Result<Vec<tokio_postgres::SimpleQueryMessage>, String> {
        client
            .simple_query("BEGIN")
            .await
            .map_err(|e| db_message(&e))?;
        let run = async {
            client
                .simple_query(&format!("SET LOCAL vcon.tenant = '{scope}'"))
                .await?;
            client.simple_query(statement).await
        };
        let result = run.await;
        let end = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
        client.simple_query(end).await.map_err(|e| db_message(&e))?;
        result.map_err(|e| db_message(&e))
    }

    /// A statement with no tenant set at all, or the server's message.
    pub(crate) async fn no_scope(client: &Client, statement: &str) -> Result<(), String> {
        client
            .simple_query(statement)
            .await
            .map(|_| ())
            .map_err(|e| db_message(&e))
    }

    /// The server's message of `e`, or all of it for another error.
    pub(crate) fn db_message(e: &tokio_postgres::Error) -> String {
        match e.as_db_error() {
            Some(db) => db.message().to_owned(),
            None => e.to_string(),
        }
    }

    /// The number of rows `sql` counts.
    pub(crate) async fn count(client: &Client, sql: &str) -> i64 {
        value(client, sql).await.parse().expect("a count")
    }

    /// One row in `table` for the tenant `tenant`: a statement for a test
    /// that needs a row in every table, and which writes every grant.
    pub(crate) fn insert_tenant_row(table: &str, tenant: &str) -> String {
        let t = format!("'\\x{tenant}'");
        match table {
            "vcon" => format!(
                "INSERT INTO vcon.vcon (tenant, uuid_mac, uuid_sealed) VALUES ({t}, '\\x01', '\\x02')"
            ),
            "version" => format!(
                "INSERT INTO vcon.version (tenant, version, uuid_mac, form_sealed, gzip_sealed, size_sealed, stored_at, tenant_key_id, wrapped_key) \
                 VALUES ({t}, '\\x01', '\\x02', '\\x03', '\\x04', '\\x05', 1, '\\x06', '\\x07')"
            ),
            "tombstone" => format!(
                "INSERT INTO vcon.tombstone (tenant, uuid_mac, uuid_sealed, purged_at) VALUES ({t}, '\\x01', '\\x02', 1)"
            ),
            "key_use" => format!(
                "INSERT INTO vcon.key_use (tenant, tenant_key_id, wraps) VALUES ({t}, '\\x01', 1)"
            ),
            "parquet_file" => format!(
                "INSERT INTO vcon.parquet_file (tenant, file_id, date, rows, live_rows, bytes, created_at) \
                 VALUES ({t}, '\\x01', '2026-11-09', 1, 1, 1, 1)"
            ),
            "purge_pending" => {
                format!("INSERT INTO vcon.purge_pending (tenant, file_id) VALUES ({t}, '\\x01')")
            }
            "manifest" => {
                format!("INSERT INTO vcon.manifest (tenant, published_at) VALUES ({t}, 1)")
            }
            "audit" => format!(
                "INSERT INTO vcon.audit (tenant, at, actor, action, status, count) VALUES ({t}, 't', 'a', 'b', 1, 1)"
            ),
            "tenant_lock" => format!("INSERT INTO vcon.tenant_lock (tenant) VALUES ({t})"),
            "tenant_catalog" => format!(
                "INSERT INTO vcon.tenant_catalog (tenant, catalog_key_id) VALUES ({t}, '\\x01')"
            ),
            other => panic!("no insert for {other}"),
        }
    }

    /// One row in a registry table.
    pub(crate) fn insert_registry_row(table: &str) -> String {
        match table {
            "tenant" => "INSERT INTO vcon.tenant (id, name_mac, name_sealed, created_at, tenant_key_id, wrapped_tenant_key, key_since) \
                         VALUES ('\\x01', '\\x02', '\\x03', 1, '\\x04', '\\x05', 1)"
                .to_owned(),
            "master_key_use" => "INSERT INTO vcon.master_key_use (master_key_id, wraps, since) VALUES ('\\x01', 1, 1)".to_owned(),
            "api_key" => "INSERT INTO vcon.api_key (id, token_hash, scopes_sealed, label_sealed, created_at) VALUES ('\\x01', '\\x02', '\\x03', '\\x04', 1)".to_owned(),
            "admin_audit" => "INSERT INTO vcon.admin_audit (at, actor, action, status, count) VALUES ('t', 'a', 'b', 1, 1)".to_owned(),
            other => panic!("no insert for {other}"),
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
