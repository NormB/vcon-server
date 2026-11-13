//! The connector's and the pool's tests, by kind: what must work
//! (`success`), what must be refused (`failure`), and the limits and odd
//! inputs (`edge`).
//!
//! The tests marked live run against real PostgreSQL servers that
//! `scripts/pg-test-server.sh` starts in containers, with TLS, and names in
//! `VCON_TEST_PG_*`. They are built only with the `backend-tests` feature,
//! and without those variables they fail, so a run that cannot reach the
//! servers never passes by skipping. The others need no server: they read
//! files, or talk to a small fake server on the loopback address.

mod catalog_tests;
mod edge;
mod failure;
mod registry_tests;
mod schema_tests;
mod space_tests;
mod success;

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::connect::{ConnectError, Connected, Connector, Purpose, Target};
#[cfg(feature = "backend-tests")]
use super::pool::Pool;
use super::secrets::Secrets;
use crate::config::PostgresSettings;

/// A CA, and a client certificate and key it signed, as PEM.
pub(super) struct Pki {
    pub(super) ca: String,
    pub(super) cert: String,
    pub(super) key: String,
}

/// A new CA and a client certificate for `vcon_cert` signed by it.
pub(super) fn pki() -> Pki {
    let ca_key = rcgen::KeyPair::generate().expect("a CA key");
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    let ca = ca_params.self_signed(&ca_key).expect("the CA");
    let issuer = rcgen::Issuer::new(ca_params, ca_key);
    let key = rcgen::KeyPair::generate().expect("a key");
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "vcon_cert");
    let cert = params.signed_by(&key, &issuer).expect("the certificate");
    Pki {
        ca: ca.pem(),
        cert: cert.pem(),
        key: key.serialize_pem(),
    }
}

/// Writes `bytes` to `name` in `dir` with `mode`, and returns its path.
pub(super) fn file(dir: &Path, name: &str, bytes: &[u8], mode: u32) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("writes the file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("sets its mode");
    path
}

/// Settings for `host` and `port`, logging in as `user` with the password
/// file `password_file`, trusting the CA in `ca`, with every other key at its
/// default.
pub(super) fn settings(
    host: &str,
    port: u16,
    user: &str,
    ca: &Path,
    password_file: Option<&Path>,
) -> PostgresSettings {
    PostgresSettings {
        hosts: vec![host.to_owned()],
        port,
        dbname: "vcon".to_owned(),
        user: user.to_owned(),
        ca_file: Some(ca.to_owned()),
        password_file: password_file.map(Path::to_owned),
        client_cert_file: None,
        client_key_file: None,
        pool: crate::config::POSTGRES_POOL,
        connect_timeout: crate::config::POSTGRES_CONNECT_TIMEOUT,
        statement_timeout: crate::config::POSTGRES_STATEMENT_TIMEOUT,
        lock_timeout: crate::config::POSTGRES_LOCK_TIMEOUT,
    }
}

/// A connector for `settings`, its secrets read from their files.
pub(super) fn connector(settings: &PostgresSettings) -> Connector {
    let secrets = Secrets::read(settings).expect("the secret files read");
    Connector::new(settings, secrets, Purpose::Connect).expect("the connector")
}

/// A connector for `targets`, with `settings`'s login.
pub(super) fn connector_to(targets: Vec<Target>, settings: &PostgresSettings) -> Connector {
    let secrets = Secrets::read(settings).expect("the secret files read");
    Connector::to(targets, settings, secrets, Purpose::Connect).expect("the connector")
}

/// One connection with `settings`.
pub(super) async fn connect(settings: &PostgresSettings) -> Result<Connected, ConnectError> {
    connector(settings).connect().await
}

/// A pool for `settings`.
#[cfg(feature = "backend-tests")]
pub(super) fn pool(settings: &PostgresSettings) -> Pool {
    Pool::new(connector(settings), settings.pool, settings.connect_timeout)
}

/// The why of a refused connection to its only host.
pub(super) fn why(refused: ConnectError) -> String {
    assert_eq!(refused.0.len(), 1, "one host was tried: {refused}");
    refused.0[0].1.clone()
}

/// A port on `ip` that nothing listens on: bound, then let go.
pub(super) async fn closed_port(ip: &str) -> u16 {
    let listener = TcpListener::bind((ip, 0)).await.expect("binds");
    listener.local_addr().expect("an address").port()
}

/// A server on the loopback address that answers PostgreSQL's SSLRequest
/// with `N`, as a server with ssl off does, and then waits.
pub(super) async fn no_tls_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let addr = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = [0u8; 8];
            if socket.read_exact(&mut request).await.is_ok() {
                let _sent = socket.write_all(b"N").await;
            }
            tokio::spawn(async move {
                let mut rest = Vec::new();
                let _read = socket.read_to_end(&mut rest).await;
            });
        }
    });
    addr
}

/// A server on the loopback address that takes each connection and never
/// answers.
pub(super) async fn silent_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let addr = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    addr
}

/// The servers `scripts/pg-test-server.sh` started, from `VCON_TEST_PG_*`.
#[cfg(feature = "backend-tests")]
pub(super) mod live {
    use std::time::Duration;

    use super::*;

    fn var(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| {
            panic!("{name} is not set: these tests need scripts/pg-test-server.sh's servers")
        })
    }

    fn port(name: &str) -> u16 {
        var(name).parse().unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// The directory of the servers' CA, client certificates and password.
    pub(crate) fn dir() -> PathBuf {
        PathBuf::from(var("VCON_TEST_PG_DIR"))
    }

    /// The primary's port on 127.0.0.1.
    pub(crate) fn primary() -> u16 {
        port("VCON_TEST_PG_PORT")
    }

    /// The primary's port on ::1.
    pub(crate) fn primary6() -> u16 {
        port("VCON_TEST_PG_PORT6")
    }

    /// The hot standby's port on 127.0.0.1.
    pub(crate) fn standby() -> u16 {
        port("VCON_TEST_PG_STANDBY_PORT")
    }

    /// The port of a server with ssl off, on 127.0.0.1.
    pub(crate) fn plain() -> u16 {
        port("VCON_TEST_PG_PLAIN_PORT")
    }

    /// The major version the servers run, such as `16`.
    pub(crate) fn major() -> String {
        var("VCON_TEST_PG_MAJOR")
    }

    /// A temporary directory holding a copy of the test password, mode
    /// 0600, and settings for the primary as `user` with it.
    pub(crate) fn as_user(user: &str) -> (tempfile::TempDir, PostgresSettings) {
        let password = std::fs::read(dir().join("password")).expect("the test password");
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let path = file(tmp.path(), "pg.password", &password, 0o600);
        let s = settings(
            "localhost",
            primary(),
            user,
            &dir().join("ca.pem"),
            Some(&path),
        );
        (tmp, s)
    }

    /// How a test with a client certificate logs in.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Login {
        /// The certificate, and no password file.
        CertificateAlone,
        /// The certificate, and the password file as well.
        CertificateAndSecret,
    }

    /// `as_user`, with `user`'s client certificate, logging in as `login`
    /// says.
    pub(crate) fn with_certificate(
        user: &str,
        login: Login,
    ) -> (tempfile::TempDir, PostgresSettings) {
        let (tmp, mut s) = as_user(user);
        let key = std::fs::read(dir().join(format!("{user}.key"))).expect("the client key");
        s.client_key_file = Some(file(tmp.path(), "client.key", &key, 0o600));
        s.client_cert_file = Some(dir().join(format!("{user}.crt")));
        if login == Login::CertificateAlone {
            s.password_file = None;
        }
        (tmp, s)
    }

    /// The test password, which no message may hold.
    pub(crate) fn password() -> String {
        std::fs::read_to_string(dir().join("password"))
            .expect("the test password")
            .trim_end()
            .to_owned()
    }

    /// One text value from `sql` on `client`.
    pub(crate) async fn value(client: &tokio_postgres::Client, sql: &str) -> String {
        let messages = client
            .simple_query(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        messages
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{sql}: no row"))
    }

    /// Waits up to five seconds for `done`.
    pub(crate) async fn within_five_seconds(mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..50 {
            if done() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        done()
    }
}
