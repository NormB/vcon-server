//! One connection to PostgreSQL: TLS verified by rustls, the login, and the
//! checks made on the server before the connection is used.

use std::error::Error as _;
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::ClientConfig;
use tokio_postgres::config::{ChannelBinding, SslMode, TargetSessionAttrs};
use tokio_postgres::{Client, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;
use zeroize::Zeroizing;

use super::secrets::Secrets;
use crate::config::{PostgresSettings, StorageError};

/// The `application_name` every connection gives, so the server's
/// `pg_stat_activity` names the program. It holds no address or name.
pub(crate) const APPLICATION_NAME: &str = "vcon-server";

/// Where one connection is made: the host name TLS checks the certificate
/// against, the address connected to when it is not looked up by that name,
/// and the port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    /// The name, or address, the server's certificate must hold.
    pub(crate) host: String,
    /// The address connected to; None looks up `host`.
    pub(crate) addr: Option<IpAddr>,
    /// The TCP port.
    pub(crate) port: u16,
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

/// The server a connection reached, as it described itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Server {
    /// The host and port connected to.
    pub(crate) target: String,
    /// `server_version`, such as `18.6`.
    pub(crate) version: String,
    /// The role logged in as.
    pub(crate) user: String,
    /// The server process serving this connection (`pg_backend_pid()`).
    pub(crate) pid: i32,
}

impl fmt::Display for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PostgreSQL {} at {} as {} over TLS",
            self.version, self.target, self.user
        )
    }
}

/// A connection made and checked.
pub(crate) struct Connected {
    pub(crate) client: Client,
    pub(crate) server: Server,
}

impl fmt::Debug for Connected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connected")
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

/// Why no connection could be made: for each host tried, in order, why it
/// was refused. It never holds a password or a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectError(pub(crate) Vec<(String, String)>);

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tries: Vec<String> = self
            .0
            .iter()
            .map(|(target, why)| format!("{target}: {why}"))
            .collect();
        write!(f, "no server could be used: {}", tries.join("; "))
    }
}

impl std::error::Error for ConnectError {}

/// Makes connections to the servers `[catalog.postgres]` names, from the
/// secrets read at start.
pub(crate) struct Connector {
    targets: Vec<Target>,
    dbname: String,
    user: String,
    password: Option<Zeroizing<String>>,
    tls: MakeRustlsConnect,
    connect_timeout: Duration,
    options: String,
}

impl fmt::Debug for Connector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connector")
            .field("targets", &self.targets)
            .field("dbname", &self.dbname)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

/// The startup options that set the statement and lock timeouts on every
/// connection, in milliseconds, PostgreSQL's unit for both.
pub(crate) fn session_options(statement_timeout: Duration, lock_timeout: Duration) -> String {
    format!(
        "-c statement_timeout={} -c lock_timeout={}",
        statement_timeout.as_millis(),
        lock_timeout.as_millis()
    )
}

/// The TLS settings every connection uses: the AWS-LC provider, its safe
/// protocol versions, `secrets.roots` as the only trust anchors, and the
/// client certificate when there is one.
pub(crate) fn tls_config(secrets: &Secrets) -> Result<ClientConfig, StorageError> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| StorageError(format!("catalog.postgres: TLS cannot be set up: {e}")))?
        .with_root_certificates(secrets.roots.clone());
    match &secrets.client {
        None => Ok(builder.with_no_client_auth()),
        Some((chain, key)) => builder
            .with_client_auth_cert(chain.clone(), key.clone_key())
            .map_err(|e| {
                StorageError(format!(
                    "catalog.postgres.client_key_file: the key cannot be used with \
                     catalog.postgres.client_cert_file: {e}"
                ))
            }),
    }
}

impl Connector {
    /// A connector for `settings`, logging in with `secrets`.
    pub(crate) fn new(
        settings: &PostgresSettings,
        secrets: Secrets,
    ) -> Result<Connector, StorageError> {
        let targets = settings
            .hosts
            .iter()
            .map(|host| Target {
                host: host.clone(),
                addr: None,
                port: settings.port,
            })
            .collect();
        Connector::to(targets, settings, secrets)
    }

    /// A connector for `targets` in place of the hosts `settings` names.
    pub(crate) fn to(
        targets: Vec<Target>,
        settings: &PostgresSettings,
        secrets: Secrets,
    ) -> Result<Connector, StorageError> {
        let tls = MakeRustlsConnect::new(tls_config(&secrets)?);
        Ok(Connector {
            targets,
            dbname: settings.dbname.clone(),
            user: settings.user.clone(),
            password: secrets.password,
            tls,
            connect_timeout: settings.connect_timeout,
            options: session_options(settings.statement_timeout, settings.lock_timeout),
        })
    }

    /// A new connection to the first host that takes it and passes the
    /// checks, each host given at most `connect_timeout`. The connection's
    /// task runs on the runtime this is called on.
    pub(crate) async fn connect(&self) -> Result<Connected, ConnectError> {
        let mut refused = Vec::new();
        for target in &self.targets {
            let tried = tokio::time::timeout(self.connect_timeout, self.connect_to(target)).await;
            match tried {
                Ok(Ok(connected)) => return Ok(connected),
                Ok(Err(why)) => refused.push((target.to_string(), why)),
                Err(_) => refused.push((
                    target.to_string(),
                    format!(
                        "no answer within catalog.postgres.connect_timeout ({} s)",
                        self.connect_timeout.as_secs()
                    ),
                )),
            }
        }
        Err(ConnectError(refused))
    }

    /// One connection to `target`, or why it was refused.
    async fn connect_to(&self, target: &Target) -> Result<Connected, String> {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&target.host)
            .port(target.port)
            .dbname(&self.dbname)
            .user(&self.user)
            .application_name(APPLICATION_NAME)
            .options(&self.options)
            .connect_timeout(self.connect_timeout)
            .ssl_mode(SslMode::Require)
            .target_session_attrs(TargetSessionAttrs::Any);
        if let Some(addr) = target.addr {
            config.hostaddr(addr);
        }
        match &self.password {
            Some(password) => {
                config
                    .password(password.as_bytes())
                    .channel_binding(ChannelBinding::Require);
            }
            None => {
                config.channel_binding(ChannelBinding::Disable);
            }
        }
        let (client, connection) = config
            .connect(self.tls.clone())
            .await
            .map_err(|e| why(&e))?;
        drop(config);
        tokio::spawn(async move {
            // An error here closes the client, which its next call reports.
            let _closed = connection.await;
        });
        let server = check(&client, target).await?;
        Ok(Connected { client, server })
    }
}

/// Why `e` refused a connection, with what to change where that is known.
fn why(e: &tokio_postgres::Error) -> String {
    let text = with_causes(e);
    if let Some(db) = e.as_db_error() {
        return format!(
            "the server refused: {} (SQLSTATE {})",
            db.message(),
            db.code().code()
        );
    }
    if text.contains("server did not use channel binding") {
        return format!(
            "{text}: with a password, the only login accepted is SCRAM-SHA-256 with channel \
                 binding; give the role a scram-sha-256 line under hostssl in pg_hba.conf (md5, \
                 password and trust are refused)"
        );
    }
    if text.contains("password missing") {
        return format!(
            "{text}: the server asked for a password, and catalog.postgres has no \
                 password_file"
        );
    }
    if text.contains("server does not support TLS") {
        return format!("{text}: turn ssl on in the server's postgresql.conf");
    }
    if let Some(why) = certificate_problem(e) {
        return format!("{text}: {why}");
    }
    text
}

/// What to change for a certificate refused by rustls, or by the server's
/// TLS layer in an alert, when `e` holds one.
fn certificate_problem(e: &tokio_postgres::Error) -> Option<&'static str> {
    use rustls::{AlertDescription, CertificateError};
    let mut source = e.source();
    while let Some(cause) = source {
        let tls = cause
            .downcast_ref::<std::io::Error>()
            .and_then(|io| io.get_ref())
            .and_then(|inner| inner.downcast_ref::<rustls::Error>());
        if let Some(rustls::Error::InvalidCertificate(why)) = tls {
            return Some(match why {
                CertificateError::UnknownIssuer | CertificateError::BadSignature => {
                    "the server's certificate is not signed by an authority in \
                     catalog.postgres.ca_file (or the system's, without it)"
                }
                CertificateError::NotValidForName
                | CertificateError::NotValidForNameContext { .. } => {
                    "the server's certificate does not hold the host name it was reached by"
                }
                CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
                    "the server's certificate has expired"
                }
                _ => "the server's certificate is refused",
            });
        }
        if let Some(rustls::Error::AlertReceived(
            AlertDescription::UnknownCA
            | AlertDescription::BadCertificate
            | AlertDescription::CertificateRequired
            | AlertDescription::CertificateExpired
            | AlertDescription::CertificateRevoked
            | AlertDescription::CertificateUnknown
            | AlertDescription::UnsupportedCertificate,
        )) = tls
        {
            return Some(
                "the server refused the client certificate (catalog.postgres.client_cert_file): \
                 the server's ssl_ca_file must hold the authority that signed it",
            );
        }
        source = cause.source();
    }
    None
}

/// `e` and each of its causes, joined by `: `.
pub(crate) fn with_causes(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut source = e.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// The checks every new connection passes before it is used, in one round
/// trip: not a standby, not read-only, and a role that is neither a
/// superuser nor has BYPASSRLS, which would pass row-level security.
async fn check(client: &Client, target: &Target) -> Result<Server, String> {
    const CHECK: &str = "SELECT pg_is_in_recovery(), current_setting('transaction_read_only'), \
                         r.rolsuper, r.rolbypassrls, current_setting('server_version'), \
                         pg_backend_pid(), current_user FROM pg_roles r \
                         WHERE r.rolname = current_user";
    let messages = client
        .simple_query(CHECK)
        .await
        .map_err(|e| format!("the server cannot be checked: {}", with_causes(&e)))?;
    let row = messages
        .iter()
        .find_map(|m| match m {
            SimpleQueryMessage::Row(row) => Some(row),
            _ => None,
        })
        .ok_or_else(|| {
            "the server cannot be checked: pg_roles has no row for this role".to_owned()
        })?;
    let text = |i: usize| row.get(i).unwrap_or_default();
    let user = text(6).to_owned();
    if text(0) == "t" {
        return Err(
            "the server is a standby (pg_is_in_recovery() is true); name the primary".to_owned(),
        );
    }
    if text(1) == "on" {
        return Err(
            "the server takes only read-only transactions (transaction_read_only is on); name \
             a primary that takes writes"
                .to_owned(),
        );
    }
    if text(2) == "t" {
        return Err(format!(
            "the role {user:?} is a superuser, which passes row-level security; log in as a \
             role that is not"
        ));
    }
    if text(3) == "t" {
        return Err(format!(
            "the role {user:?} has BYPASSRLS, which passes row-level security; log in as a role \
             without it"
        ));
    }
    Ok(Server {
        target: target.to_string(),
        version: text(4).to_owned(),
        user,
        pid: text(5).parse().map_err(|_| {
            "the server cannot be checked: pg_backend_pid() is not a number".to_owned()
        })?,
    })
}
