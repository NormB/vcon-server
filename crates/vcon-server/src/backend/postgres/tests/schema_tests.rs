//! The schema's tests: the migrations, `migrate` itself, the version and
//! role refusals, and row-level security. The pure rules run with no
//! server; the others are marked live, as in the parent module, and run
//! against the servers `scripts/pg-test-server.sh` starts. Each live test
//! has a database of its own, because a migration changes the whole
//! database and tests run at the same time.

mod cached;
mod edge;
mod failure;
mod guard;
mod line;
mod success;
mod v10;
mod v2;
mod v3;
mod v4;
mod v5;
mod v6;
mod v7;
mod v8;
mod v9;

/// The names of the databases live tests make, and which of them a killed
/// test left behind.
pub(super) mod names {
    /// What every test database's name starts with.
    const PREFIX: &str = "vcon_t_";

    /// How old, in seconds, a test database is before a new test may
    /// remove it: longer than any test runs, so it is never a running
    /// test's.
    pub(crate) const STALE_AFTER: u64 = 1800;

    /// The name of a database made at `now`, in seconds since 1970, with
    /// `random` to tell it from another made in the same second.
    pub(crate) fn name_at(now: u64, random: &[u8]) -> String {
        let random: String = random.iter().map(|b| format!("{b:02x}")).collect();
        format!("{PREFIX}{now:x}_{random}")
    }

    /// Whether `name` is a test database made more than [`STALE_AFTER`]
    /// seconds before `now`. Any other name, and a name whose time does
    /// not read, is not.
    pub(crate) fn is_stale(name: &str, now: u64) -> bool {
        name.strip_prefix(PREFIX)
            .and_then(|rest| rest.split_once('_'))
            .and_then(|(made, _)| u64::from_str_radix(made, 16).ok())
            .is_some_and(|made| now.checked_sub(made).is_some_and(|age| age > STALE_AFTER))
    }
}

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
    pub(crate) const REGISTRY_TABLES: &[&str] = &[
        "admin_audit",
        "api_key",
        "master_key_use",
        "membership",
        "session",
        "tenant",
        "tenant_setting",
        "user_account",
    ];

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
            let name = super::names::name_at(now(), &bytes);
            let connected = connect(&admin, Purpose::Connect)
                .await
                .expect("the tester connects");
            sweep(&connected.client).await;
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

    /// How long the drop of a test's database may take before the test
    /// goes on without it; the next sweep removes what is left.
    const DROP_WITHIN: std::time::Duration = std::time::Duration::from_secs(20);

    /// The statement that drops a test database with the connections
    /// still open to it.
    fn drop_statement(name: &str) -> String {
        format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")
    }

    impl Drop for Db {
        /// Drops the database when the test ends, pass or fail, on a
        /// thread and runtime of its own, since the test's runtime may be
        /// the one that is ending. A failure here fails no test: a
        /// database left behind is removed by a later test's sweep.
        fn drop(&mut self) {
            let statement = drop_statement(&self.name);
            let dropping = std::thread::spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                // The timer needs the runtime, so it is made inside it.
                runtime.block_on(async {
                    let _ = tokio::time::timeout(DROP_WITHIN, async {
                        let (_dir, admin) = as_user("vcon_tester");
                        if let Ok(connected) = connect(&admin, Purpose::Connect).await {
                            let _ = connected.client.batch_execute(&statement).await;
                        }
                    })
                    .await;
                });
            });
            let _ = dropping.join();
        }
    }

    /// Drops the test databases a killed test left on the server: those
    /// older than any running test can be. Another test may sweep the
    /// same ones at the same time, so a drop that fails is passed over.
    async fn sweep(tester: &Client) {
        let Ok(rows) = tester
            .simple_query("SELECT datname FROM pg_database WHERE datname LIKE 'vcon\\_t\\_%'")
            .await
        else {
            return;
        };
        let now = now();
        for message in &rows {
            if let tokio_postgres::SimpleQueryMessage::Row(row) = message
                && let Some(name) = row.get(0)
                && super::names::is_stale(name, now)
            {
                let _ = tester.batch_execute(&drop_statement(name)).await;
            }
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
                "INSERT INTO vcon.audit (tenant, at, actor, action, status, count, counted) VALUES ({t}, 't', 'a', 'b', 1, 1, true)"
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
            "admin_audit" => "INSERT INTO vcon.admin_audit (at, actor, action, status, count, counted) VALUES ('t', 'a', 'b', 1, 1, true)".to_owned(),
            "user_account" => "INSERT INTO vcon.user_account (id, label_sealed, created_at) VALUES ('\\x01', '\\x02', 1)".to_owned(),
            "membership" => "INSERT INTO vcon.membership (user_id, roles_sealed, granted_at) VALUES ('\\x01', '\\x02', 1)".to_owned(),
            "tenant_setting" => "INSERT INTO vcon.tenant_setting (tenant, name, value_sealed) VALUES ('\\x01', 'n', '\\x02')".to_owned(),
            "session" => "INSERT INTO vcon.session (id_hash, user_id, scopes_sealed, created_at, last_seen_at, idle_expires_at, expires_at, idle_seconds, amr_sealed, csrf_hash) VALUES (decode(repeat('01', 32), 'hex'), '\\x01', '\\x02', 1, 1, 2, 3, 1, '\\x03', decode(repeat('04', 32), 'hex'))".to_owned(),
            other => panic!("no insert for {other}"),
        }
    }

    /// The seconds since 1970, by the system's clock.
    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_secs()
    }
}
