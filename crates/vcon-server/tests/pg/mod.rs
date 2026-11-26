//! A PostgreSQL database of a test's own, on the servers
//! `scripts/pg-test-server.sh` starts (`VCON_TEST_PG_*`), for the tests
//! that run the `vcon-server` binary with its catalogs in PostgreSQL. One
//! server serves a database, so each such test makes its own and drops it
//! when it ends.
//!
//! The names are those of the library's live tests
//! (`backend/postgres/tests/schema_tests.rs`, `names`): `vcon_t_`, the
//! time in hexadecimal, and random digits, so that their sweep removes a
//! database a killed test left, and `pg-test-server.sh leftovers` counts
//! it.
//!
//! Each connection a helper here makes, and each configuration it writes
//! for the binary, takes the one live-test deadline of
//! tests/support/live_deadline.rs, which the crate including this module
//! includes at its root.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;

/// The value of the environment variable `name`.
pub fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set: these tests need scripts/pg-test-server.sh's servers")
    })
}

/// The directory `pg-test-server.sh` wrote: the CA, the password.
pub fn dir() -> PathBuf {
    PathBuf::from(var("VCON_TEST_PG_DIR"))
}

/// The test password, which no message may hold.
pub fn password() -> String {
    std::fs::read_to_string(dir().join("password"))
        .expect("the test password")
        .trim_end()
        .to_owned()
}

/// Runs `sql` in the database `dbname` as the role `vcon_tester`, which
/// may create databases and is a member of `vcon_owner`, and gives the
/// first column of each row it answered. It runs on a thread and runtime
/// of its own, so a test may call it inside its runtime or outside one.
pub fn tester(dbname: &str, sql: &str) -> Result<Vec<String>, String> {
    query_as("vcon_tester", dbname, sql)
}

/// Runs `sql` in the database `dbname` as the superuser `vcon_super`,
/// which row-level security does not filter, as `tester` runs it: for a
/// test that reads every row of every tenant.
#[allow(
    dead_code,
    reason = "this module is part of several test crates; only tests/formats.rs calls it"
)]
pub fn superuser(dbname: &str, sql: &str) -> Result<Vec<String>, String> {
    query_as("vcon_super", dbname, sql)
}

/// Runs `sql` in the database `dbname` as the login role `role` on the
/// test primary, within the live-test deadline.
fn query_as(role: &str, dbname: &str, sql: &str) -> Result<Vec<String>, String> {
    Server::primary().query(role, dbname, sql, crate::live_deadline::LIVE_DEADLINE)
}

/// A PostgreSQL server a helper logs in to: its port on localhost, the CA
/// that signed its certificate, and the password of its login roles.
pub struct Server {
    pub port: u16,
    pub ca_file: PathBuf,
    pub password: String,
}

impl Server {
    /// The primary `pg-test-server.sh` started.
    pub fn primary() -> Server {
        let port = var("VCON_TEST_PG_PORT");
        Server {
            port: port
                .parse()
                .unwrap_or_else(|e| panic!("VCON_TEST_PG_PORT {port:?}: {e}")),
            ca_file: dir().join("ca.pem"),
            password: password(),
        }
    }

    /// Runs `sql` in the database `dbname` as the login role `role`, with
    /// the password over verified TLS, giving the connection `deadline`
    /// from its first packet to the end of the login. It runs on a thread
    /// and runtime of its own, so a test may call it inside its runtime or
    /// outside one.
    pub fn query(
        &self,
        role: &str,
        dbname: &str,
        sql: &str,
        deadline: std::time::Duration,
    ) -> Result<Vec<String>, String> {
        let (role, dbname, sql) = (role.to_owned(), dbname.to_owned(), sql.to_owned());
        let (port, ca_file, password) = (self.port, self.ca_file.clone(), self.password.clone());
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            runtime.block_on(async {
                let mut roots = rustls::RootCertStore::empty();
                for certificate in CertificateDer::pem_file_iter(&ca_file)
                    .map_err(|e| format!("the test CA: {e}"))?
                {
                    roots
                        .add(certificate.map_err(|e| format!("the test CA: {e}"))?)
                        .map_err(|e| format!("the test CA: {e}"))?;
                }
                let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
                let tls = rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(|e| e.to_string())?
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                let mut config = tokio_postgres::Config::new();
                config
                    .host("localhost")
                    .port(port)
                    .user(&role)
                    .password(&password)
                    .dbname(&dbname)
                    .ssl_mode(tokio_postgres::config::SslMode::Require);
                // tokio-postgres's own connect_timeout bounds only the TCP
                // connection; this bounds the TLS handshake and the login
                // as well.
                let connecting = config.connect(tokio_postgres_rustls::MakeRustlsConnect::new(tls));
                let (client, connection) = tokio::time::timeout(deadline, connecting)
                    .await
                    .map_err(|_| {
                        crate::live_deadline::missed(
                            &format!("{role} at localhost:{port}"),
                            deadline,
                        )
                    })?
                    .map_err(|e| format!("{role} cannot connect: {e}"))?;
                let driving = tokio::spawn(connection);
                let answered = client
                    .simple_query(&sql)
                    .await
                    .map_err(|e| match e.as_db_error() {
                        Some(db) => db.message().to_owned(),
                        None => e.to_string(),
                    });
                drop(client);
                let _ = driving.await;
                Ok(answered?
                    .iter()
                    .filter_map(|message| match message {
                        tokio_postgres::SimpleQueryMessage::Row(row) => {
                            Some(row.get(0).unwrap_or_default().to_owned())
                        }
                        _ => None,
                    })
                    .collect())
            })
        })
        .join()
        .map_err(|_| "the tester's thread panicked".to_owned())?
    }
}

/// The `[catalog]` and `[catalog.postgres]` tables for the test primary:
/// the database `dbname`, the role `user` with the password in
/// `password_file`, the CA in `ca_file`, and the live-test deadline.
pub fn catalog_tables(dbname: &str, user: &str, ca_file: &Path, password_file: &Path) -> String {
    crate::live_deadline::postgres_tables(
        &var("VCON_TEST_PG_PORT"),
        dbname,
        user,
        ca_file,
        password_file,
        crate::live_deadline::LIVE_DEADLINE,
    )
}

/// A database of a test's own, owned by `vcon_owner` and empty, dropped
/// when the value is.
pub struct Db {
    pub name: String,
}

impl Db {
    /// A new empty database: `vcon-server migrate` makes its schema.
    pub fn fresh() -> Db {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_secs();
        let mut random = [0u8; 8];
        {
            use std::io::Read;
            std::fs::File::open("/dev/urandom")
                .expect("urandom")
                .read_exact(&mut random)
                .expect("8 random bytes");
        }
        let random: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let name = format!("vcon_t_{now:x}_{random}");
        tester("vcon", &format!("CREATE DATABASE {name} OWNER vcon_owner"))
            .expect("creates the database");
        Db { name }
    }

    /// Runs `sql` in this database as the schema's owner.
    pub fn as_owner(&self, sql: &str) -> Vec<String> {
        tester(&self.name, &format!("SET ROLE vcon_owner; {sql}"))
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }
}

impl Drop for Db {
    /// A failure here fails no test: a database left behind is removed by
    /// a later live test's sweep.
    fn drop(&mut self) {
        let _ = tester(
            "vcon",
            &format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name),
        );
    }
}
