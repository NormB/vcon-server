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

use std::path::PathBuf;
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
    let (dbname, sql) = (dbname.to_owned(), sql.to_owned());
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        runtime.block_on(async {
            let mut roots = rustls::RootCertStore::empty();
            for certificate in CertificateDer::pem_file_iter(dir().join("ca.pem"))
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
                .port(var("VCON_TEST_PG_PORT").parse().map_err(|_| "the port")?)
                .user("vcon_tester")
                .password(password())
                .dbname(&dbname)
                .ssl_mode(tokio_postgres::config::SslMode::Require)
                .connect_timeout(std::time::Duration::from_secs(10));
            let (client, connection) = config
                .connect(tokio_postgres_rustls::MakeRustlsConnect::new(tls))
                .await
                .map_err(|e| format!("the tester cannot connect: {e}"))?;
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
