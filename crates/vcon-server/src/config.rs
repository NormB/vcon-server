//! The server's configuration: one TOML file. A configuration error stops
//! the start and names its key, and an unknown key is an error.

use std::collections::BTreeSet;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::api::ConnectionLimits;
use crate::retention::Retention;
use crate::session::Origin;

/// The configuration `serve` runs with. Durations are written in whole
/// seconds and sizes in bytes; each must be more than zero, and each has a
/// default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The address and port `serve` listens on (`listen`).
    pub listen: SocketAddr,
    /// The file holding the server's PEM certificate chain (`tls.cert`).
    pub tls_cert: PathBuf,
    /// The file holding the server's PEM private key (`tls.key`).
    pub tls_key: PathBuf,
    /// The data directory (`data_dir`).
    pub data_dir: PathBuf,
    /// The file holding the 32-byte master key (`master_key`).
    pub master_key: PathBuf,
    /// For tenants that set none; 7 years unless configured.
    pub retention_default: Retention,
    /// Time between purge sweeps; [`PURGE_INTERVAL`] unless configured.
    pub purge_interval: Duration,
    /// Age of the oldest work waiting for key rotation before `serve`
    /// warns; [`ROTATION_WARN`] unless configured.
    pub rotation_warn: Duration,
    /// The largest request body.
    pub body_limit: usize,
    /// The largest size gzip inflation may reach while a body is inspected.
    pub inflated_limit: u64,
    /// How long a request body may take to arrive.
    pub read_timeout: Duration,
    /// How long a connection may take over its handshake and its request
    /// heads and how long it may wait between requests
    /// (`limits.timeouts.handshake`, `first_head`, `head` and `idle`), and
    /// how many a listener holds at once (`limits.connections`).
    pub connection: ConnectionLimits,
    /// Extension names accepted in `critical` and `must_support`.
    pub supported_critical: BTreeSet<String>,
    /// The MCP server's limits.
    pub mcp: crate::mcp::Limits,
    /// How many reports of a content-security-policy violation a listener
    /// takes in a minute, from every address and from one
    /// (`limits.csp_reports` and `limits.csp_reports_per_address`).
    pub reports: crate::api::ReportLimits,
    /// Where the data is kept.
    pub storage: Storage,
    /// How `export parquet` reaches a bucket: the `[export]` table.
    pub export: Export,
    /// Where the catalogs and the registry are kept: the `[catalog]` table.
    pub catalog: CatalogStore,
    /// The operator listener: the `[operator]` table. Without it, the one
    /// listener serves operator keys too.
    pub operator: Option<Operator>,
    /// The origin a browser reaches `listen` at (`public_origin`), which a
    /// request by session must come from. Without it, `listen` serves no
    /// session.
    pub public_origin: Option<Origin>,
}

/// The operator listener: the `[operator]` table. With it, `serve` listens
/// on a second address that serves operator keys only, and `listen` serves
/// tenant keys only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operator {
    /// The address and port of the operator listener (`operator.listen`).
    pub listen: SocketAddr,
    /// Its certificate chain (`operator.tls.cert`), or `tls.cert` when the
    /// table has no `[operator.tls]`.
    pub tls_cert: PathBuf,
    /// Its private key (`operator.tls.key`), or `tls.key` when the table
    /// has no `[operator.tls]`.
    pub tls_key: PathBuf,
    /// The origin a browser reaches the operator listener at
    /// (`operator.public_origin`). Without it, that listener serves no
    /// session.
    pub public_origin: Option<Origin>,
}

/// Whether two listening addresses would take the same socket: the same
/// port, other than the port 0 that asks the system for a free one, on
/// the same address, or on addresses of one family of which one is the
/// unspecified address, which takes every address of the family.
#[must_use]
pub fn same_socket(a: SocketAddr, b: SocketAddr) -> bool {
    a.port() == b.port()
        && a.port() != 0
        && (a.ip() == b.ip()
            || (a.is_ipv4() == b.is_ipv4() && (a.ip().is_unspecified() || b.ip().is_unspecified())))
}

/// Where the catalogs and the registry are kept: `catalog.backend`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CatalogStore {
    /// SQLCipher files in `data_dir`.
    #[default]
    Local,
    /// A PostgreSQL database, reached over TLS: `[catalog.postgres]`.
    Postgres(Box<PostgresSettings>),
}

/// How to reach the PostgreSQL database: the `[catalog.postgres]` table.
/// TLS, with the server's certificate chain and host name verified, is the
/// only way it is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresSettings {
    /// The servers' host names or IP addresses, tried in order.
    pub hosts: Vec<String>,
    /// The TCP port of every host.
    pub port: u16,
    /// The database.
    pub dbname: String,
    /// The role the server logs in as.
    pub user: String,
    /// PEM certificates of the authorities trusted for the servers' TLS
    /// certificates, in place of the system's; None trusts the system's.
    pub ca_file: Option<PathBuf>,
    /// The file holding the role's password, for SCRAM with channel binding.
    pub password_file: Option<PathBuf>,
    /// The file holding the PEM client certificate chain.
    pub client_cert_file: Option<PathBuf>,
    /// The file holding the client certificate's PEM private key.
    pub client_key_file: Option<PathBuf>,
    /// The most connections open at once.
    pub pool: u32,
    /// How long a connection, or a wait for a free one, may take.
    pub connect_timeout: Duration,
    /// The longest a statement may run (PostgreSQL's `statement_timeout`).
    pub statement_timeout: Duration,
    /// The longest a statement may wait for a lock (`lock_timeout`).
    pub lock_timeout: Duration,
}

/// The `[export]` table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Export {
    /// How an `s3://` target is reached (`[export.s3]`); None takes
    /// `[storage.s3]`'s service and keys.
    pub s3: Option<ExportS3>,
}

/// How to reach the S3-compatible service an export writes to: every key
/// of `[storage.s3]` but the bucket and prefix, which `--to` names, and
/// `allow_versioning`, which only the server's own storage reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportS3 {
    /// The service's URL; None is AWS S3 for the region.
    pub endpoint: Option<String>,
    /// The region, used in request signing.
    pub region: String,
    /// The bucket in the path, not in the host name.
    pub path_style: bool,
    /// The access key id; None uses the AWS default credential chain.
    pub access_key_id: Option<String>,
    /// The file holding the secret access key.
    pub secret_access_key_file: Option<PathBuf>,
    /// Accept an `http://` endpoint.
    pub allow_http: bool,
    /// PEM certificates of the authorities trusted for the endpoint's TLS
    /// certificate, in place of the system's.
    pub ca_file: Option<PathBuf>,
    /// How long one request may take.
    pub timeout: Duration,
}

impl ExportS3 {
    /// The settings for `bucket` under `prefix` on this service.
    #[must_use]
    pub fn for_bucket(&self, bucket: &str, prefix: &str) -> S3Settings {
        S3Settings {
            endpoint: self.endpoint.clone(),
            region: self.region.clone(),
            bucket: bucket.to_owned(),
            prefix: prefix.to_owned(),
            path_style: self.path_style,
            access_key_id: self.access_key_id.clone(),
            secret_access_key_file: self.secret_access_key_file.clone(),
            allow_http: self.allow_http,
            ca_file: self.ca_file.clone(),
            allow_versioning: false,
            timeout: self.timeout,
        }
    }
}

impl From<&S3Settings> for ExportS3 {
    /// The service and keys of `[storage.s3]`.
    fn from(s: &S3Settings) -> ExportS3 {
        ExportS3 {
            endpoint: s.endpoint.clone(),
            region: s.region.clone(),
            path_style: s.path_style,
            access_key_id: s.access_key_id.clone(),
            secret_access_key_file: s.secret_access_key_file.clone(),
            allow_http: s.allow_http,
            ca_file: s.ca_file.clone(),
            timeout: s.timeout,
        }
    }
}

/// Why the storage the configuration names cannot be used: the message says
/// which setting or step.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct StorageError(pub String);

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StorageError {}

/// Where the server keeps its data: the `[storage]` table. A file without
/// it keeps everything in `data_dir`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Storage {
    /// Where the sealed vCon objects are kept.
    pub objects: ObjectStore,
    /// Parquet storage, `[storage.parquet]`: the objects in the bucket are
    /// packed into Parquet files. None: they stay objects.
    pub parquet: Option<ParquetSettings>,
}

/// The `[storage.parquet]` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParquetSettings {
    /// Time from the end of one pack run to the start of the next.
    pub pack_interval: Duration,
    /// The largest sealed object packed, in bytes; a larger one stays an
    /// object.
    pub pack_limit: u64,
    /// How long an older manifest, and a file a compaction stopped naming,
    /// stays for readers outside the server.
    pub reader_grace: Duration,
}

/// How long an older manifest, and a file a compaction stopped naming,
/// stays for readers outside the server unless configured: 15 minutes.
pub const READER_GRACE: Duration = Duration::from_secs(900);

/// Time between pack runs unless configured.
pub const PACK_INTERVAL: Duration = Duration::from_secs(300);

/// The largest sealed object packed unless configured: 16 MiB, the default
/// largest request body.
pub const PACK_LIMIT: u64 = 16 << 20;

/// Where the sealed vCon objects are kept: `storage.objects`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ObjectStore {
    /// Files under `data_dir`, beside each tenant's catalog.
    #[default]
    Local,
    /// An S3-compatible bucket.
    S3(S3Settings),
}

/// How to reach the bucket: the `[storage.s3]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S3Settings {
    /// The service's URL; None is AWS S3 for the region.
    pub endpoint: Option<String>,
    /// The region, used in request signing.
    pub region: String,
    /// The bucket, which must exist.
    pub bucket: String,
    /// Written before `tenants/` in every object's name.
    pub prefix: String,
    /// The bucket in the path, not in the host name.
    pub path_style: bool,
    /// The access key id; None uses the AWS default credential chain.
    pub access_key_id: Option<String>,
    /// The file holding the secret access key.
    pub secret_access_key_file: Option<PathBuf>,
    /// Accept an `http://` endpoint.
    pub allow_http: bool,
    /// PEM certificates of the authorities trusted for the endpoint's
    /// TLS certificate, in place of the system's; None trusts the system's.
    pub ca_file: Option<PathBuf>,
    /// Start even though the bucket has versioning on.
    pub allow_versioning: bool,
    /// How long one request may take.
    pub timeout: Duration,
}

/// Time between purge sweeps unless configured: a version is purged
/// within an hour of falling due.
pub const PURGE_INTERVAL: Duration = Duration::from_secs(3600);

/// Age of the oldest work waiting for key rotation before `serve` warns,
/// unless configured: 30 days.
pub const ROTATION_WARN: Duration = Duration::from_secs(30 * 86_400);

/// The largest request body unless configured: 16 MiB. The largest of
/// 34,885 vCon files measured (public datasets and recorded test calls) was
/// 5,141,813 bytes, a two-minute recording held inline; nearly all of their
/// recordings are referenced by URL instead.
pub const BODY_LIMIT: usize = 16 << 20;

/// The largest inflation unless configured: four times [`BODY_LIMIT`], so
/// any vCon accepted plain is also accepted gzipped, with room left for
/// gzipped payloads inside its signed layers.
pub const INFLATED_LIMIT: u64 = 4 * BODY_LIMIT as u64;

/// How long a request body may take to arrive unless configured: a body at
/// [`BODY_LIMIT`] needs about 2.2 Mbit/s.
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// A configuration that could not be read, naming the key at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Reads the configuration file at `path`.
///
/// # Errors
///
/// A [`ConfigError`] naming the file if it cannot be read, or any error
/// [`parse`] gives for its text.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
    parse(&text)
}

/// Reads a configuration from its TOML text.
///
/// # Errors
///
/// A [`ConfigError`] naming the key at fault: text that is not TOML, an
/// unknown key, a required key missing, or a value of the wrong type or
/// not more than zero.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let raw: Raw = toml::from_str(text).map_err(|e| ConfigError(e.to_string()))?;
    let storage = storage(raw.storage)?;
    let catalog = catalog(raw.catalog, &storage)?;
    let retention_default = match raw.retention_default {
        None => Retention::Years(7),
        Some(text) => Retention::parse(&text).ok_or_else(|| {
            ConfigError(format!(
                "retention_default: {text:?} is not a period such as 7y, 18m or 90d"
            ))
        })?,
    };
    let tls = (raw.tls.cert, raw.tls.key);
    let public_origin = origin("public_origin", raw.public_origin)?;
    Ok(Config {
        listen: raw.listen,
        tls_cert: tls.0.clone(),
        tls_key: tls.1.clone(),
        data_dir: raw.data_dir,
        master_key: raw.master_key,
        retention_default,
        purge_interval: seconds("purge_interval", raw.purge_interval, PURGE_INTERVAL)?,
        rotation_warn: seconds("rotation_warn", raw.rotation_warn, ROTATION_WARN)?,
        body_limit: match raw.limits.body {
            None => BODY_LIMIT,
            Some(body) => usize::try_from(positive("limits.body", body)?)
                .map_err(|_| ConfigError("limits.body is too large".to_owned()))?,
        },
        inflated_limit: match raw.limits.inflated {
            None => INFLATED_LIMIT,
            Some(inflated) => positive("limits.inflated", inflated)?,
        },
        read_timeout: seconds(
            "limits.timeouts.read",
            raw.limits.timeouts.read,
            READ_TIMEOUT,
        )?,
        connection: connection_limits(&raw.limits)?,
        supported_critical: raw.supported_critical.into_iter().collect(),
        mcp: mcp_limits(&raw.mcp)?,
        reports: report_limits(&raw.limits)?,
        storage,
        export: Export {
            s3: raw.export.s3.map(export_s3).transpose()?,
        },
        catalog,
        operator: operator(raw.operator, raw.listen, &tls, public_origin.as_ref())?,
        public_origin,
    })
}

/// A listener's public origin, if `key` is written.
fn origin(key: &str, raw: Option<String>) -> Result<Option<Origin>, ConfigError> {
    raw.map(|text| {
        Origin::parse(&text).ok_or_else(|| {
            ConfigError(format!(
                "{key}: {text:?} is not an origin: write https://, a host in lower case and, \
                 unless it is 443, a port, with nothing after, such as https://vcon.example.com"
            ))
        })
    })
    .transpose()
}

/// The `[operator]` table, checked against the tenant listener.
fn operator(
    raw: Option<RawOperator>,
    listen: SocketAddr,
    tls: &(PathBuf, PathBuf),
    tenant_origin: Option<&Origin>,
) -> Result<Option<Operator>, ConfigError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if same_socket(raw.listen, listen) {
        return Err(ConfigError(format!(
            "operator.listen: {} is the socket `listen` takes ({listen}); the operator listener \
             needs an address or port of its own",
            raw.listen
        )));
    }
    let (tls_cert, tls_key) = match raw.tls {
        Some(own) => (own.cert, own.key),
        None => tls.clone(),
    };
    let public_origin = origin("operator.public_origin", raw.public_origin)?;
    // A browser keeps a cookie by host and not by port, so two listeners
    // on one host would each be sent the other's session cookie.
    if let (Some(own), Some(tenant)) = (&public_origin, tenant_origin)
        && own.host() == tenant.host()
    {
        return Err(ConfigError(format!(
            "operator.public_origin: {own} has the host of `public_origin` ({tenant}); a browser \
             keeps a session cookie by host and not by port, so the operator listener needs a \
             host name of its own"
        )));
    }
    Ok(Some(Operator {
        listen: raw.listen,
        tls_cert,
        tls_key,
        public_origin,
    }))
}

/// The `[mcp]` table's limits, each key that is not written at its default.
fn mcp_limits(raw: &RawMcp) -> Result<crate::mcp::Limits, ConfigError> {
    let default = crate::mcp::Limits::DEFAULT;
    let narrow = |key: &str, value: u64| {
        u32::try_from(positive(key, value)?).map_err(|_| ConfigError(format!("{key} is too large")))
    };
    Ok(crate::mcp::Limits {
        calls_per_second: match raw.calls_per_second {
            None => default.calls_per_second,
            Some(value) => narrow("mcp.calls_per_second", value)?,
        },
        concurrent: match raw.concurrent {
            None => default.concurrent,
            Some(value) => narrow("mcp.concurrent", value)?,
        },
        response_bytes: match raw.response_bytes {
            None => default.response_bytes,
            Some(value) => usize::try_from(positive("mcp.response_bytes", value)?)
                .map_err(|_| ConfigError("mcp.response_bytes is too large".to_owned()))?,
        },
    })
}

/// How long one S3 request may take unless configured.
pub const S3_TIMEOUT: Duration = Duration::from_secs(30);

/// The `[storage]` table checked: an S3 table only with `objects = "s3"`,
/// an `http://` endpoint only when allowed, and an access key id only with
/// the file of its secret.
fn storage(raw: RawStorage) -> Result<Storage, ConfigError> {
    let objects = match (raw.objects.as_deref(), raw.s3) {
        (None | Some("local"), None) => ObjectStore::Local,
        (None | Some("local"), Some(_)) => {
            return Err(ConfigError(
                "storage.s3 is set but storage.objects is not \"s3\"".to_owned(),
            ));
        }
        (Some("s3"), None) => {
            return Err(ConfigError(
                "storage.objects is \"s3\" but there is no storage.s3 table".to_owned(),
            ));
        }
        (Some("s3"), Some(s3)) => ObjectStore::S3(s3_settings(s3)?),
        (Some(other), _) => {
            return Err(ConfigError(format!(
                "storage.objects: {other:?} is not \"local\" or \"s3\""
            )));
        }
    };
    let parquet = match (&objects, raw.parquet) {
        (_, None) => None,
        (ObjectStore::S3(_), Some(p)) => Some(ParquetSettings {
            pack_interval: seconds(
                "storage.parquet.pack_interval",
                p.pack_interval,
                PACK_INTERVAL,
            )?,
            pack_limit: match p.pack_limit {
                None => PACK_LIMIT,
                Some(limit) => positive("storage.parquet.pack_limit", limit)?,
            },
            reader_grace: seconds("storage.parquet.reader_grace", p.reader_grace, READER_GRACE)?,
        }),
        (ObjectStore::Local, Some(_)) => {
            return Err(ConfigError(
                "storage.parquet packs the objects of a bucket, so it needs \
                 storage.objects = \"s3\""
                    .to_owned(),
            ));
        }
    };
    Ok(Storage { objects, parquet })
}

/// The PostgreSQL port unless configured.
pub const POSTGRES_PORT: u16 = 5432;

/// The most connections open at once unless configured.
pub const POSTGRES_POOL: u32 = 16;

/// How long a connection, or a wait for a free one, may take unless
/// configured.
pub const POSTGRES_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The longest a statement may run unless configured.
pub const POSTGRES_STATEMENT_TIMEOUT: Duration = Duration::from_secs(30);

/// The longest a statement may wait for a lock unless configured.
pub const POSTGRES_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest `statement_timeout` and `lock_timeout`, in whole seconds:
/// PostgreSQL holds each as milliseconds in a 32-bit integer.
pub const POSTGRES_TIMEOUT_LIMIT: u64 = i32::MAX as u64 / 1000;

/// The longest database or role name: PostgreSQL cuts a longer one to this
/// many bytes (NAMEDATALEN - 1), which would name another database or role.
pub const POSTGRES_NAME_LIMIT: usize = 63;

/// The `[catalog]` table checked: a `[catalog.postgres]` table only with
/// `backend = "postgres"`, and that only with the objects in a bucket.
fn catalog(raw: RawCatalog, storage: &Storage) -> Result<CatalogStore, ConfigError> {
    match (raw.backend.as_deref(), raw.postgres) {
        (None | Some("local"), None) => Ok(CatalogStore::Local),
        (None | Some("local"), Some(_)) => Err(ConfigError(
            "catalog.postgres is set but catalog.backend is not \"postgres\"".to_owned(),
        )),
        (Some("postgres"), None) => Err(ConfigError(
            "catalog.backend is \"postgres\" but there is no catalog.postgres table".to_owned(),
        )),
        (Some("postgres"), Some(pg)) => {
            // A shared catalog with objects in one host's data directory
            // would name objects the other hosts cannot read, and recovery
            // on one host would take the others' rows as naming missing
            // objects.
            if !matches!(storage.objects, ObjectStore::S3(_)) {
                return Err(ConfigError(
                    "catalog.backend = \"postgres\" needs storage.objects = \"s3\": a catalog \
                     in PostgreSQL names objects that every server must be able to read"
                        .to_owned(),
                ));
            }
            postgres_settings(pg).map(|pg| CatalogStore::Postgres(Box::new(pg)))
        }
        (Some(other), _) => Err(ConfigError(format!(
            "catalog.backend: {other:?} is not \"local\" or \"postgres\""
        ))),
    }
}

fn postgres_settings(raw: RawPostgres) -> Result<PostgresSettings, ConfigError> {
    const KEY: &str = "catalog.postgres";
    if raw.hosts.is_empty() {
        return Err(ConfigError(format!(
            "{KEY}.hosts is empty; name at least one server"
        )));
    }
    let mut seen = BTreeSet::new();
    for host in &raw.hosts {
        check_host(host).map_err(|why| ConfigError(format!("{KEY}.hosts: {host:?} {why}")))?;
        if !seen.insert(host.to_ascii_lowercase()) {
            return Err(ConfigError(format!("{KEY}.hosts names {host:?} twice")));
        }
    }
    let port = match raw.port {
        None => POSTGRES_PORT,
        Some(port) => u16::try_from(positive(&format!("{KEY}.port"), port)?).map_err(|_| {
            ConfigError(format!("{KEY}.port {port} is not a TCP port (1 to 65535)"))
        })?,
    };
    check_name("dbname", &raw.dbname)?;
    check_name("user", &raw.user)?;
    for (name, path) in [
        ("ca_file", &raw.ca_file),
        ("password_file", &raw.password_file),
        ("client_cert_file", &raw.client_cert_file),
        ("client_key_file", &raw.client_key_file),
    ] {
        if path.as_ref().is_some_and(|p| p.as_os_str().is_empty()) {
            return Err(ConfigError(format!("{KEY}.{name} is empty")));
        }
    }
    match (&raw.client_cert_file, &raw.client_key_file) {
        (Some(_), None) => {
            return Err(ConfigError(format!(
                "{KEY}.client_cert_file is set without {KEY}.client_key_file"
            )));
        }
        (None, Some(_)) => {
            return Err(ConfigError(format!(
                "{KEY}.client_key_file is set without {KEY}.client_cert_file"
            )));
        }
        _ => {}
    }
    if raw.password_file.is_none() && raw.client_cert_file.is_none() {
        return Err(ConfigError(format!(
            "set {KEY}.password_file, {KEY}.client_cert_file, or both: a server that asks for \
             neither is not accepted"
        )));
    }
    let pool = match raw.pool {
        None => POSTGRES_POOL,
        Some(pool) => u32::try_from(positive(&format!("{KEY}.pool"), pool)?)
            .map_err(|_| ConfigError(format!("{KEY}.pool is too large")))?,
    };
    let limited = |name: &str, value: Option<u64>, default: Duration| {
        let key = format!("{KEY}.{name}");
        let value = seconds(&key, value, default)?;
        if value.as_secs() > POSTGRES_TIMEOUT_LIMIT {
            return Err(ConfigError(format!(
                "{key} is more than {POSTGRES_TIMEOUT_LIMIT} seconds, the longest PostgreSQL holds"
            )));
        }
        Ok(value)
    };
    Ok(PostgresSettings {
        hosts: raw.hosts,
        port,
        dbname: raw.dbname,
        user: raw.user,
        ca_file: raw.ca_file,
        password_file: raw.password_file,
        client_cert_file: raw.client_cert_file,
        client_key_file: raw.client_key_file,
        pool,
        connect_timeout: seconds(
            &format!("{KEY}.connect_timeout"),
            raw.connect_timeout,
            POSTGRES_CONNECT_TIMEOUT,
        )?,
        statement_timeout: limited(
            "statement_timeout",
            raw.statement_timeout,
            POSTGRES_STATEMENT_TIMEOUT,
        )?,
        lock_timeout: limited("lock_timeout", raw.lock_timeout, POSTGRES_LOCK_TIMEOUT)?,
    })
}

/// A database or role name PostgreSQL takes as written: not empty, no NUL,
/// and no longer than [`POSTGRES_NAME_LIMIT`] bytes.
fn check_name(key: &str, name: &str) -> Result<(), ConfigError> {
    let refused = |why: &str| Err(ConfigError(format!("catalog.postgres.{key} {why}")));
    if name.is_empty() {
        return refused("is empty");
    }
    if name.contains('\0') {
        return refused("holds a NUL character, which PostgreSQL names cannot");
    }
    if name.len() > POSTGRES_NAME_LIMIT {
        return refused(&format!(
            "is {} bytes; PostgreSQL would cut it to {POSTGRES_NAME_LIMIT} bytes and name \
             another database or role",
            name.len()
        ));
    }
    Ok(())
}

/// A host the catalog may connect to: an IP address, or a DNS name in
/// ASCII (RFC 1123 labels). The reason a host is refused, after its name.
fn check_host(host: &str) -> Result<(), String> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    if host.starts_with('/') {
        return Err(
            "is a Unix socket, which carries no TLS; the catalog is reached over TCP \
                    with TLS only"
                .to_owned(),
        );
    }
    if let Some(bare) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']'))
        && bare.parse::<std::net::Ipv6Addr>().is_ok()
    {
        return Err(format!(
            "is written in brackets; write the address as {bare:?}"
        ));
    }
    if host.is_empty() {
        return Err("is empty".to_owned());
    }
    if !host.is_ascii() {
        return Err("is not ASCII; write an internationalized name in its xn-- form".to_owned());
    }
    if host.contains(':') {
        return Err(
            "holds a colon; give the port in catalog.postgres.port, which every host uses"
                .to_owned(),
        );
    }
    if host.len() > 253 {
        return Err("is longer than 253 characters, the longest DNS name".to_owned());
    }
    let good_label = |label: &str| {
        (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if !host.split('.').all(good_label) {
        return Err(
            "is not a DNS name: labels of 1 to 63 letters, digits and hyphens, joined by dots"
                .to_owned(),
        );
    }
    Ok(())
}

/// `[export.s3]` checked as `[storage.s3]` is, but for its bucket.
fn export_s3(raw: RawExportS3) -> Result<ExportS3, ConfigError> {
    check_service(
        "export.s3",
        raw.endpoint.as_deref(),
        raw.allow_http,
        raw.ca_file.is_some(),
        raw.access_key_id.is_some(),
        raw.secret_access_key_file.is_some(),
    )?;
    Ok(ExportS3 {
        endpoint: raw.endpoint,
        region: raw.region,
        path_style: raw.path_style,
        access_key_id: raw.access_key_id,
        secret_access_key_file: raw.secret_access_key_file,
        allow_http: raw.allow_http,
        ca_file: raw.ca_file,
        timeout: seconds("export.s3.timeout", raw.timeout, S3_TIMEOUT)?,
    })
}

/// The rules for how a service is reached, the same for every table that
/// names one: an `http://` endpoint only when allowed and never with a CA
/// file, any other endpoint `https://`, and an access key id only with the
/// file of its secret.
fn check_service(
    table: &str,
    endpoint: Option<&str>,
    allow_http: bool,
    ca_file: bool,
    access_key_id: bool,
    secret_access_key_file: bool,
) -> Result<(), ConfigError> {
    if let Some(endpoint) = endpoint {
        if endpoint.starts_with("http://") {
            if ca_file {
                return Err(ConfigError(format!(
                    "{table}.ca_file is set, but {table}.endpoint {endpoint:?} is plain \
                     HTTP, which uses no certificate"
                )));
            }
            if !allow_http {
                return Err(ConfigError(format!(
                    "{table}.endpoint {endpoint:?} is plain HTTP; set {table}.allow_http = \
                     true only for a test service on this host"
                )));
            }
        } else if !endpoint.starts_with("https://") {
            return Err(ConfigError(format!(
                "{table}.endpoint {endpoint:?} is not an https:// URL"
            )));
        }
    }
    match (access_key_id, secret_access_key_file) {
        (true, false) => Err(ConfigError(format!(
            "{table}.access_key_id is set without {table}.secret_access_key_file"
        ))),
        (false, true) => Err(ConfigError(format!(
            "{table}.secret_access_key_file is set without {table}.access_key_id"
        ))),
        _ => Ok(()),
    }
}

fn s3_settings(raw: RawS3) -> Result<S3Settings, ConfigError> {
    if raw.bucket.is_empty() {
        return Err(ConfigError("storage.s3.bucket is empty".to_owned()));
    }
    if !raw.prefix.is_empty() && (raw.prefix.starts_with('/') || !raw.prefix.ends_with('/')) {
        return Err(ConfigError(format!(
            "storage.s3.prefix {:?} must not start with / and must end with /, \
             such as \"server-a/\"",
            raw.prefix
        )));
    }
    check_service(
        "storage.s3",
        raw.endpoint.as_deref(),
        raw.allow_http,
        raw.ca_file.is_some(),
        raw.access_key_id.is_some(),
        raw.secret_access_key_file.is_some(),
    )?;
    Ok(S3Settings {
        endpoint: raw.endpoint,
        region: raw.region,
        bucket: raw.bucket,
        prefix: raw.prefix,
        path_style: raw.path_style,
        access_key_id: raw.access_key_id,
        secret_access_key_file: raw.secret_access_key_file,
        allow_http: raw.allow_http,
        ca_file: raw.ca_file,
        allow_versioning: raw.allow_versioning,
        timeout: seconds("storage.s3.timeout", raw.timeout, S3_TIMEOUT)?,
    })
}

fn positive(key: &str, value: u64) -> Result<u64, ConfigError> {
    if value == 0 {
        return Err(ConfigError(format!("{key} must be more than 0")));
    }
    Ok(value)
}

/// The most seconds a connection limit may be set to: an hour. A limit
/// meant to close a stalled connection that waits longer than this closes
/// none in practice.
pub const CONNECTION_LIMIT_MAX: u64 = 3600;

/// The most reports a minute `limits.csp_reports` and
/// `limits.csp_reports_per_address` may be set to. Each report taken costs
/// one write to the registry, so a larger number would bound nothing.
pub const CSP_REPORTS_MAX: u64 = 6000;

/// How many reports of a content-security-policy violation a listener
/// takes in a minute (`limits.csp_reports`) and from one address
/// (`limits.csp_reports_per_address`), or the defaults for those not
/// written. More from one address than from all is refused: the second
/// number could never be reached.
fn report_limits(limits: &RawLimits) -> Result<crate::api::ReportLimits, ConfigError> {
    let default = crate::api::ReportLimits::DEFAULT;
    let read = |key: &str, value: Option<u64>, default: u32| -> Result<u32, ConfigError> {
        let Some(value) = value else {
            return Ok(default);
        };
        let value = positive(key, value)?;
        u32::try_from(value)
            .ok()
            .filter(|_| value <= CSP_REPORTS_MAX)
            .ok_or_else(|| {
                ConfigError(format!(
                    "{key} must be at most {CSP_REPORTS_MAX}, and is {value}"
                ))
            })
    };
    let per_minute = read("limits.csp_reports", limits.csp_reports, default.per_minute)?;
    // A minute's number set under the default for one address lowers that
    // default with it, so that setting one key alone is never refused.
    let per_address = read(
        "limits.csp_reports_per_address",
        limits.csp_reports_per_address,
        default.per_address.min(per_minute),
    )?;
    if per_address > per_minute {
        return Err(ConfigError(format!(
            "limits.csp_reports_per_address ({per_address}) must be at most limits.csp_reports ({per_minute})"
        )));
    }
    Ok(crate::api::ReportLimits {
        per_minute,
        per_address,
    })
}

/// The most connections `limits.connections` may be set to: a bound that
/// keeps a mistyped number from asking for millions of file descriptors.
pub const CONNECTIONS_MAX: u64 = 65_536;

/// The most connections a listener holds at once (`limits.connections`),
/// or `default` when it is not written.
fn connections(value: Option<u64>, default: usize) -> Result<usize, ConfigError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = positive("limits.connections", value)?;
    usize::try_from(value)
        .ok()
        .filter(|_| value <= CONNECTIONS_MAX)
        .ok_or_else(|| {
            ConfigError(format!(
                "limits.connections must be at most {CONNECTIONS_MAX}, and is {value}"
            ))
        })
}

/// The `[limits]` keys that bound a connection and the number of them,
/// each that is not written at its default.
fn connection_limits(limits: &RawLimits) -> Result<ConnectionLimits, ConfigError> {
    let raw = &limits.timeouts;
    let default = ConnectionLimits::default();
    let limit = |key: &str, value: Option<u64>, default: Duration| {
        let limit = seconds(key, value, default)?;
        if limit.as_secs() > CONNECTION_LIMIT_MAX {
            return Err(ConfigError(format!(
                "{key} must be at most {CONNECTION_LIMIT_MAX} seconds, and is {}",
                limit.as_secs()
            )));
        }
        Ok(limit)
    };
    Ok(ConnectionLimits {
        handshake: limit(
            "limits.timeouts.handshake",
            raw.handshake,
            default.handshake,
        )?,
        first_head: limit(
            "limits.timeouts.first_head",
            raw.first_head,
            default.first_head,
        )?,
        head: limit("limits.timeouts.head", raw.head, default.head)?,
        idle: limit("limits.timeouts.idle", raw.idle, default.idle)?,
        goodbye: default.goodbye,
        accept_pause: default.accept_pause,
        max: connections(limits.connections, default.max)?,
    })
}

/// `value` in whole seconds, or `default` when it is not written.
fn seconds(key: &str, value: Option<u64>, default: Duration) -> Result<Duration, ConfigError> {
    match value {
        None => Ok(default),
        Some(value) => positive(key, value).map(Duration::from_secs),
    }
}

/// The file as written, before its values are checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    listen: SocketAddr,
    tls: RawTls,
    data_dir: PathBuf,
    master_key: PathBuf,
    retention_default: Option<String>,
    purge_interval: Option<u64>,
    rotation_warn: Option<u64>,
    #[serde(default)]
    limits: RawLimits,
    #[serde(default)]
    supported_critical: Vec<String>,
    #[serde(default)]
    mcp: RawMcp,
    #[serde(default)]
    storage: RawStorage,
    #[serde(default)]
    export: RawExport,
    #[serde(default)]
    catalog: RawCatalog,
    operator: Option<RawOperator>,
    public_origin: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOperator {
    listen: SocketAddr,
    tls: Option<RawTls>,
    public_origin: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    backend: Option<String>,
    postgres: Option<RawPostgres>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPostgres {
    hosts: Vec<String>,
    port: Option<u64>,
    dbname: String,
    user: String,
    ca_file: Option<PathBuf>,
    password_file: Option<PathBuf>,
    client_cert_file: Option<PathBuf>,
    client_key_file: Option<PathBuf>,
    pool: Option<u64>,
    connect_timeout: Option<u64>,
    statement_timeout: Option<u64>,
    lock_timeout: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExport {
    s3: Option<RawExportS3>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExportS3 {
    endpoint: Option<String>,
    region: String,
    #[serde(default)]
    path_style: bool,
    access_key_id: Option<String>,
    secret_access_key_file: Option<PathBuf>,
    #[serde(default)]
    allow_http: bool,
    ca_file: Option<PathBuf>,
    timeout: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStorage {
    objects: Option<String>,
    s3: Option<RawS3>,
    parquet: Option<RawParquet>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawParquet {
    pack_interval: Option<u64>,
    pack_limit: Option<u64>,
    reader_grace: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawS3 {
    endpoint: Option<String>,
    region: String,
    bucket: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    path_style: bool,
    access_key_id: Option<String>,
    secret_access_key_file: Option<PathBuf>,
    #[serde(default)]
    allow_http: bool,
    ca_file: Option<PathBuf>,
    #[serde(default)]
    allow_versioning: bool,
    timeout: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcp {
    calls_per_second: Option<u64>,
    concurrent: Option<u64>,
    response_bytes: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTls {
    cert: PathBuf,
    key: PathBuf,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLimits {
    body: Option<u64>,
    inflated: Option<u64>,
    connections: Option<u64>,
    csp_reports: Option<u64>,
    csp_reports_per_address: Option<u64>,
    #[serde(default)]
    timeouts: RawTimeouts,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTimeouts {
    read: Option<u64>,
    handshake: Option<u64>,
    first_head: Option<u64>,
    head: Option<u64>,
    idle: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
listen = "127.0.0.1:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"
retention_default = "18m"
purge_interval = 1800
rotation_warn = 604800
supported_critical = ["known", "other"]
public_origin = "https://vcon.example.com"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"

[operator]
listen = "127.0.0.1:8444"
public_origin = "https://ops.example.com:8444"

[operator.tls]
cert = "/etc/vcon-server/operator-cert.pem"
key = "/etc/vcon-server/operator-key.pem"

[limits]
body = 10485760
inflated = 104857600
connections = 64
csp_reports = 300
csp_reports_per_address = 30

[limits.timeouts]
read = 30
handshake = 5
first_head = 15
head = 20
idle = 90

[mcp]
calls_per_second = 20
concurrent = 8
response_bytes = 262144

[storage]
objects = "s3"

[storage.s3]
endpoint = "https://objects.example.com:9000"
region = "eu-west-1"
bucket = "vcon"
prefix = "server-a/"
path_style = true
access_key_id = "vcon-server"
secret_access_key_file = "/etc/vcon-server/s3.secret"
allow_http = true
ca_file = "/etc/vcon-server/s3-ca.pem"
allow_versioning = true
timeout = 60

[storage.parquet]
pack_interval = 600
pack_limit = 8388608
reader_grace = 1800

[export.s3]
endpoint = "https://analytics.example.com"
region = "eu-west-2"
path_style = true
access_key_id = "vcon-export"
secret_access_key_file = "/etc/vcon-server/export.secret"
allow_http = true
ca_file = "/etc/vcon-server/export-ca.pem"
timeout = 120

[catalog]
backend = "postgres"

[catalog.postgres]
hosts = ["db1.example.com", "192.0.2.10", "2001:db8::10"]
port = 6432
dbname = "vcon"
user = "vcon_app"
ca_file = "/etc/vcon-server/pg-ca.pem"
password_file = "/etc/vcon-server/postgres.password"
client_cert_file = "/etc/vcon-server/pg-client.pem"
client_key_file = "/etc/vcon-server/pg-client.key"
pool = 8
connect_timeout = 10
statement_timeout = 60
lock_timeout = 20
"#;

    #[test]
    fn a_full_configuration_is_read() {
        assert_eq!(
            parse(FULL),
            Ok(Config {
                listen: "127.0.0.1:8443".parse().expect("address"),
                tls_cert: "/etc/vcon-server/cert.pem".into(),
                tls_key: "/etc/vcon-server/key.pem".into(),
                data_dir: "/var/lib/vcon-server".into(),
                master_key: "/etc/vcon-server/master.key".into(),
                retention_default: Retention::Months(18),
                purge_interval: Duration::from_secs(1800),
                rotation_warn: Duration::from_secs(604_800),
                body_limit: 10_485_760,
                inflated_limit: 104_857_600,
                read_timeout: Duration::from_secs(30),
                connection: ConnectionLimits {
                    handshake: Duration::from_secs(5),
                    first_head: Duration::from_secs(15),
                    head: Duration::from_secs(20),
                    idle: Duration::from_secs(90),
                    goodbye: Duration::from_secs(5),
                    accept_pause: Duration::from_secs(1),
                    max: 64,
                },
                supported_critical: ["known", "other"].map(String::from).into(),
                mcp: crate::mcp::Limits {
                    calls_per_second: 20,
                    concurrent: 8,
                    response_bytes: 262_144,
                },
                reports: crate::api::ReportLimits {
                    per_minute: 300,
                    per_address: 30,
                },
                storage: Storage {
                    objects: ObjectStore::S3(S3Settings {
                        endpoint: Some("https://objects.example.com:9000".to_owned()),
                        region: "eu-west-1".to_owned(),
                        bucket: "vcon".to_owned(),
                        prefix: "server-a/".to_owned(),
                        path_style: true,
                        access_key_id: Some("vcon-server".to_owned()),
                        secret_access_key_file: Some("/etc/vcon-server/s3.secret".into()),
                        allow_http: true,
                        ca_file: Some("/etc/vcon-server/s3-ca.pem".into()),
                        allow_versioning: true,
                        timeout: Duration::from_secs(60),
                    }),
                    parquet: Some(ParquetSettings {
                        pack_interval: Duration::from_secs(600),
                        pack_limit: 8_388_608,
                        reader_grace: Duration::from_secs(1800),
                    }),
                },
                export: Export {
                    s3: Some(ExportS3 {
                        endpoint: Some("https://analytics.example.com".to_owned()),
                        region: "eu-west-2".to_owned(),
                        path_style: true,
                        access_key_id: Some("vcon-export".to_owned()),
                        secret_access_key_file: Some("/etc/vcon-server/export.secret".into()),
                        allow_http: true,
                        ca_file: Some("/etc/vcon-server/export-ca.pem".into()),
                        timeout: Duration::from_secs(120),
                    }),
                },
                catalog: CatalogStore::Postgres(Box::new(PostgresSettings {
                    hosts: ["db1.example.com", "192.0.2.10", "2001:db8::10"]
                        .map(String::from)
                        .into(),
                    port: 6432,
                    dbname: "vcon".to_owned(),
                    user: "vcon_app".to_owned(),
                    ca_file: Some("/etc/vcon-server/pg-ca.pem".into()),
                    password_file: Some("/etc/vcon-server/postgres.password".into()),
                    client_cert_file: Some("/etc/vcon-server/pg-client.pem".into()),
                    client_key_file: Some("/etc/vcon-server/pg-client.key".into()),
                    pool: 8,
                    connect_timeout: Duration::from_secs(10),
                    statement_timeout: Duration::from_secs(60),
                    lock_timeout: Duration::from_secs(20),
                })),
                operator: Some(Operator {
                    listen: "127.0.0.1:8444".parse().expect("address"),
                    tls_cert: "/etc/vcon-server/operator-cert.pem".into(),
                    tls_key: "/etc/vcon-server/operator-key.pem".into(),
                    public_origin: Origin::parse("https://ops.example.com:8444"),
                }),
                public_origin: Origin::parse("https://vcon.example.com"),
            }),
            "every key the file writes is read into its field"
        );
    }

    mod operator_listener {
        use super::*;

        /// The least configuration, with an `[operator]` table made of `table`.
        fn with(table: &str) -> String {
            format!("{LEAST}\n{table}")
        }

        fn address(text: &str) -> SocketAddr {
            text.parse().expect("an address")
        }

        #[test]
        fn without_the_table_there_is_no_operator_listener() {
            assert_eq!(
                parse(LEAST).expect("parses").operator,
                None,
                "one listener, as before the table existed"
            );
        }

        #[test]
        fn the_table_with_its_address_alone_takes_the_servers_certificate_and_key() {
            let config = parse(&with("[operator]\nlisten = \"127.0.0.1:8444\"\n")).expect("parses");
            assert_eq!(
                config.operator,
                Some(Operator {
                    listen: address("127.0.0.1:8444"),
                    tls_cert: "/etc/vcon-server/cert.pem".into(),
                    tls_key: "/etc/vcon-server/key.pem".into(),
                    public_origin: None,
                }),
                "tls.cert and tls.key serve both listeners"
            );
        }

        #[test]
        fn the_table_may_name_a_certificate_and_key_of_its_own() {
            let config = parse(&with(
                "[operator]\nlisten = \"127.0.0.1:8444\"\n\n[operator.tls]\n\
                 cert = \"/etc/vcon-server/ops.pem\"\nkey = \"/etc/vcon-server/ops.key\"\n",
            ))
            .expect("parses");
            assert_eq!(
                config.operator,
                Some(Operator {
                    listen: address("127.0.0.1:8444"),
                    tls_cert: "/etc/vcon-server/ops.pem".into(),
                    tls_key: "/etc/vcon-server/ops.key".into(),
                    public_origin: None,
                }),
                "the operator listener's own pair"
            );
            assert_eq!(
                config.tls_cert,
                PathBuf::from("/etc/vcon-server/cert.pem"),
                "the tenant listener keeps its own"
            );
        }

        #[test]
        fn the_table_leaves_every_other_key_as_it_was() {
            let without = parse(LEAST).expect("parses");
            let with = parse(&with("[operator]\nlisten = \"127.0.0.1:8444\"\n")).expect("parses");
            assert_eq!(
                Config {
                    operator: None,
                    ..with
                },
                without,
                "only the operator member differs"
            );
        }

        #[test]
        fn the_operator_listener_may_be_on_any_address_the_operator_chooses() {
            for listen in [
                "127.0.0.1:8444",
                "[::1]:8444",
                "192.0.2.10:8444",
                "[2001:db8::10]:8444",
                "0.0.0.0:8444",
                "127.0.0.1:0",
            ] {
                let config =
                    parse(&with(&format!("[operator]\nlisten = \"{listen}\"\n"))).expect(listen);
                assert_eq!(
                    config.operator.expect("an operator listener").listen,
                    address(listen),
                    "{listen}"
                );
            }
        }

        #[test]
        fn the_same_port_on_another_address_is_another_socket() {
            for (tenant, operator) in [
                ("192.0.2.10:8443", "127.0.0.1:8443"),
                ("127.0.0.1:8443", "[::1]:8443"),
                ("0.0.0.0:8443", "[::1]:8443"),
            ] {
                let text = with(&format!("[operator]\nlisten = \"{operator}\"\n")).replace(
                    "127.0.0.1:8443\"\ndata_dir",
                    &format!("{tenant}\"\ndata_dir"),
                );
                assert!(
                    parse(&text).is_ok(),
                    "{tenant} and {operator}: {:?}",
                    parse(&text)
                );
            }
        }

        #[test]
        fn port_zero_twice_is_two_sockets() {
            let text = with("[operator]\nlisten = \"127.0.0.1:0\"\n")
                .replace("127.0.0.1:8443", "127.0.0.1:0");
            let config = parse(&text).expect("parses");
            assert_eq!(config.listen, address("127.0.0.1:0"), "the tenant listener");
            assert_eq!(
                config.operator.expect("an operator listener").listen,
                address("127.0.0.1:0"),
                "the operator listener"
            );
        }

        #[test]
        fn the_table_without_its_address_is_refused_by_name() {
            for table in [
                "[operator]\n",
                "[operator.tls]\ncert = \"a.pem\"\nkey = \"a.key\"\n",
            ] {
                let error = parse(&with(table)).expect_err("no address");
                assert!(error.0.contains("`listen`"), "{table}: {error}");
            }
        }

        #[test]
        fn the_tenant_listeners_own_socket_is_refused() {
            let error =
                parse(&with("[operator]\nlisten = \"127.0.0.1:8443\"\n")).expect_err("one socket");
            assert_eq!(
                error.0,
                "operator.listen: 127.0.0.1:8443 is the socket `listen` takes (127.0.0.1:8443); \
                 the operator listener needs an address or port of its own",
                "the refusal names both keys and both addresses"
            );
        }

        #[test]
        fn a_socket_the_tenant_listener_covers_is_refused() {
            for (tenant, operator) in [
                ("0.0.0.0:8443", "127.0.0.1:8443"),
                ("127.0.0.1:8443", "0.0.0.0:8443"),
                ("[::]:8443", "[::1]:8443"),
                ("[2001:db8::10]:8443", "[::]:8443"),
            ] {
                let text = with(&format!("[operator]\nlisten = \"{operator}\"\n")).replace(
                    "127.0.0.1:8443\"\ndata_dir",
                    &format!("{tenant}\"\ndata_dir"),
                );
                let error = parse(&text).expect_err("one socket");
                assert!(
                    error.0.starts_with("operator.listen: "),
                    "{tenant} and {operator}: {error}"
                );
            }
        }

        #[test]
        fn an_address_that_is_not_one_is_refused_by_name() {
            for listen in ["localhost:8444", "127.0.0.1", "8444", "", "127.0.0.1:99999"] {
                let error = parse(&with(&format!("[operator]\nlisten = \"{listen}\"\n")))
                    .expect_err(listen);
                assert!(error.0.contains("listen"), "{listen}: {error}");
            }
        }

        #[test]
        fn an_address_of_the_wrong_type_is_refused() {
            for value in ["8444", "true", "[\"127.0.0.1:8444\"]"] {
                let error =
                    parse(&with(&format!("[operator]\nlisten = {value}\n"))).expect_err(value);
                assert!(error.0.contains("listen"), "{value}: {error}");
            }
        }

        #[test]
        fn an_unknown_key_in_the_table_is_refused_by_name() {
            for (table, key) in [
                (
                    "[operator]\nlisten = \"127.0.0.1:8444\"\nport = 8444\n",
                    "port",
                ),
                (
                    "[operator]\nlisten = \"127.0.0.1:8444\"\n[operator.tls]\ncert = \"a\"\nkey = \"b\"\nca = \"c\"\n",
                    "ca",
                ),
                (
                    "[operator]\nlisten = \"127.0.0.1:8444\"\n[operator.limits]\nbody = 1\n",
                    "limits",
                ),
            ] {
                let error = parse(&with(table)).expect_err(key);
                assert!(error.0.contains(&format!("`{key}`")), "{key}: {error}");
            }
        }

        #[test]
        fn half_a_certificate_and_key_pair_is_refused_by_the_missing_half() {
            for (given, missing) in [("cert", "key"), ("key", "cert")] {
                let table = format!(
                    "[operator]\nlisten = \"127.0.0.1:8444\"\n[operator.tls]\n{given} = \"a.pem\"\n"
                );
                let error = parse(&with(&table)).expect_err(missing);
                assert!(
                    error.0.contains(&format!("`{missing}`")),
                    "{given} alone: {error}"
                );
            }
        }

        #[test]
        fn the_table_written_twice_is_refused() {
            let table = "[operator]\nlisten = \"127.0.0.1:8444\"\n[operator]\nlisten = \"127.0.0.1:8445\"\n";
            assert!(parse(&with(table)).is_err(), "TOML takes a table once");
        }

        #[test]
        fn operator_as_a_value_and_not_a_table_is_refused() {
            let text = format!("operator = \"127.0.0.1:8444\"\n{LEAST}");
            let error = parse(&text).expect_err("not a table");
            assert!(error.0.contains("operator"), "{error}");
        }

        #[test]
        fn two_addresses_are_one_socket_only_on_one_port_and_one_address() {
            let same = [
                ("127.0.0.1:8443", "127.0.0.1:8443"),
                ("0.0.0.0:8443", "127.0.0.1:8443"),
                ("192.0.2.10:8443", "0.0.0.0:8443"),
                ("[::]:8443", "[2001:db8::10]:8443"),
                ("[::1]:8443", "[::1]:8443"),
                ("0.0.0.0:8443", "0.0.0.0:8443"),
            ];
            for (a, b) in same {
                assert!(same_socket(address(a), address(b)), "{a} and {b}");
                assert!(same_socket(address(b), address(a)), "{b} and {a}");
            }
            let different = [
                ("127.0.0.1:8443", "127.0.0.1:8444"),
                ("127.0.0.1:8443", "192.0.2.10:8443"),
                ("0.0.0.0:8443", "0.0.0.0:8444"),
                ("127.0.0.1:8443", "[::1]:8443"),
                ("0.0.0.0:8443", "[::1]:8443"),
                ("127.0.0.1:0", "127.0.0.1:0"),
                ("0.0.0.0:0", "127.0.0.1:0"),
            ];
            for (a, b) in different {
                assert!(!same_socket(address(a), address(b)), "{a} and {b}");
                assert!(!same_socket(address(b), address(a)), "{b} and {a}");
            }
        }
    }

    mod public_origins {
        use super::*;
        use crate::session::Origin;

        const OPERATOR: &str = "[operator]\nlisten = \"127.0.0.1:8444\"\n";

        /// The least configuration with `public_origin` set to `tenant`,
        /// and an `[operator]` table that ends with the lines `operator`.
        fn with(tenant: Option<&str>, operator: &str) -> Result<Config, ConfigError> {
            let first = tenant.map_or_else(String::new, |o| format!("public_origin = {o}\n"));
            parse(&format!("{first}{LEAST}\n{OPERATOR}{operator}"))
        }

        fn origin(text: &str) -> Option<Origin> {
            Some(Origin::parse(text).expect("an origin"))
        }

        fn operators(config: &Config) -> Option<Origin> {
            config
                .operator
                .as_ref()
                .expect("an operator listener")
                .public_origin
                .clone()
        }

        // What must work.

        #[test]
        fn without_the_keys_no_listener_has_a_public_origin() {
            let config = with(None, "").expect("parses");
            assert_eq!(config.public_origin, None, "sessions are off on `listen`");
            assert_eq!(operators(&config), None, "and on `operator.listen`");
            assert_eq!(
                parse(LEAST).expect("parses").public_origin,
                None,
                "the least"
            );
        }

        #[test]
        fn public_origin_is_the_tenant_listener_s() {
            let config = with(Some("\"https://vcon.example.com\""), "").expect("parses");
            assert_eq!(
                config.public_origin,
                origin("https://vcon.example.com"),
                "the tenant listener's"
            );
            assert_eq!(operators(&config), None, "the operator listener has none");
        }

        #[test]
        fn operator_public_origin_is_the_operator_listener_s() {
            let config = with(
                Some("\"https://vcon.example.com\""),
                "public_origin = \"https://ops.example.com:8444\"\n",
            )
            .expect("parses");
            assert_eq!(
                config.public_origin,
                origin("https://vcon.example.com"),
                "the tenant listener's"
            );
            assert_eq!(
                operators(&config),
                origin("https://ops.example.com:8444"),
                "the operator listener's"
            );
        }

        #[test]
        fn a_server_with_one_listener_takes_public_origin_alone() {
            let text = format!("public_origin = \"https://vcon.example.com:8443\"\n{LEAST}");
            let config = parse(&text).expect("parses");
            assert_eq!(
                config.public_origin,
                origin("https://vcon.example.com:8443"),
                "the one listener's"
            );
            assert_eq!(config.operator, None, "no operator listener");
        }

        // What must be refused.

        #[test]
        fn a_public_origin_that_is_not_https_and_a_host_is_refused_by_its_key() {
            for bad in [
                "http://vcon.example.com",
                "https://vcon.example.com/",
                "https://vcon.example.com/console",
                "vcon.example.com",
                "https://Vcon.example.com",
                "",
            ] {
                let error = with(Some(&format!("{bad:?}")), "").expect_err(bad);
                assert!(
                    error.0.starts_with("public_origin: ") && error.0.contains(bad),
                    "{error}"
                );
            }
        }

        #[test]
        fn an_operator_public_origin_that_is_not_an_origin_is_refused_by_its_key() {
            let error = with(None, "public_origin = \"https://ops.example.com/admin\"\n")
                .expect_err("a path");
            assert!(
                error.0.starts_with("operator.public_origin: "),
                "the key is named: {error}"
            );
        }

        #[test]
        fn two_public_origins_on_one_host_are_refused_whatever_their_ports() {
            for (tenant, operator) in [
                ("https://vcon.example.com", "https://vcon.example.com"),
                ("https://vcon.example.com", "https://vcon.example.com:8444"),
                (
                    "https://vcon.example.com:8443",
                    "https://vcon.example.com:8444",
                ),
                ("https://[2001:db8::7]:8443", "https://[2001:db8::7]:8444"),
            ] {
                let error = with(
                    Some(&format!("{tenant:?}")),
                    &format!("public_origin = {operator:?}\n"),
                )
                .expect_err("one host");
                assert!(
                    error.0.starts_with("operator.public_origin: ") && error.0.contains("host"),
                    "{tenant} and {operator}: {error}"
                );
            }
        }

        #[test]
        fn a_public_origin_that_is_not_a_string_is_refused() {
            assert!(with(Some("8443"), "").is_err(), "a number");
            assert!(
                with(Some("[\"https://a.example.com\"]"), "").is_err(),
                "a list"
            );
            assert!(with(None, "public_origin = true\n").is_err(), "a flag");
        }

        // Limits and odd inputs.

        #[test]
        fn the_operator_listener_may_have_a_public_origin_when_the_tenant_s_has_none() {
            let config =
                with(None, "public_origin = \"https://ops.example.com\"\n").expect("parses");
            assert_eq!(config.public_origin, None, "sessions off for tenants");
            assert_eq!(
                operators(&config),
                origin("https://ops.example.com"),
                "the operator listener's"
            );
        }

        #[test]
        fn two_public_origins_on_two_hosts_may_share_a_port() {
            let config = with(
                Some("\"https://vcon.example.com:8443\""),
                "public_origin = \"https://ops.example.com:8443\"\n",
            )
            .expect("two hosts");
            assert_eq!(
                operators(&config),
                origin("https://ops.example.com:8443"),
                "the operator listener's"
            );
        }

        #[test]
        fn a_public_origin_may_be_an_address_or_localhost() {
            for text in [
                "https://localhost:8443",
                "https://192.0.2.7:8443",
                "https://[2001:db8::7]:8443",
            ] {
                let config = with(Some(&format!("{text:?}")), "").expect(text);
                assert_eq!(config.public_origin, origin(text), "{text}");
            }
        }

        #[test]
        fn a_host_that_is_another_s_prefix_is_another_host() {
            let config = with(
                Some("\"https://vcon.example.com\""),
                "public_origin = \"https://vcon.example.com.example.net\"\n",
            )
            .expect("two hosts");
            assert_eq!(
                operators(&config),
                origin("https://vcon.example.com.example.net"),
                "the operator listener's"
            );
        }
    }

    mod connection_limits {
        use super::*;

        fn with(timeouts: &str) -> Result<Config, ConfigError> {
            parse(&format!("{LEAST}\n[limits.timeouts]\n{timeouts}"))
        }

        #[test]
        fn the_defaults_are_ten_seconds_thirty_thirty_and_two_minutes_idle() {
            let config = parse(LEAST).expect("parses");
            assert_eq!(
                config.connection,
                ConnectionLimits {
                    handshake: Duration::from_secs(10),
                    first_head: Duration::from_secs(30),
                    head: Duration::from_secs(30),
                    idle: Duration::from_secs(120),
                    goodbye: Duration::from_secs(5),
                    accept_pause: Duration::from_secs(1),
                    max: 512,
                },
                "with no [limits.timeouts] table"
            );
            assert_eq!(
                config.connection,
                ConnectionLimits::default(),
                "the defaults serve applies without a configuration"
            );
        }

        #[test]
        fn each_key_sets_its_own_limit_and_no_other() {
            let default = ConnectionLimits::default();
            for (line, expected) in [
                (
                    "handshake = 3",
                    ConnectionLimits {
                        handshake: Duration::from_secs(3),
                        ..default
                    },
                ),
                (
                    "first_head = 7",
                    ConnectionLimits {
                        first_head: Duration::from_secs(7),
                        ..default
                    },
                ),
                (
                    "head = 75",
                    ConnectionLimits {
                        head: Duration::from_secs(75),
                        ..default
                    },
                ),
                (
                    "idle = 300",
                    ConnectionLimits {
                        idle: Duration::from_secs(300),
                        ..default
                    },
                ),
            ] {
                let config = with(line).expect(line);
                assert_eq!(config.connection, expected, "{line}");
                assert_eq!(
                    config.read_timeout, READ_TIMEOUT,
                    "{line}: the body limit is its own key"
                );
            }
        }

        #[test]
        fn the_four_keys_together_are_all_read() {
            let config = with("handshake = 2\nfirst_head = 4\nhead = 6\nread = 8\nidle = 10")
                .expect("parses");
            assert_eq!(
                (
                    config.connection.handshake,
                    config.connection.first_head,
                    config.connection.head,
                    config.read_timeout,
                    config.connection.idle,
                ),
                (
                    Duration::from_secs(2),
                    Duration::from_secs(4),
                    Duration::from_secs(6),
                    Duration::from_secs(8),
                    Duration::from_secs(10),
                ),
                "each key in its own member"
            );
        }

        #[test]
        fn an_idle_connection_is_kept_two_minutes_unless_told() {
            assert_eq!(
                parse(LEAST).expect("parses").connection.idle,
                Duration::from_secs(120),
                "with no [limits.timeouts] table"
            );
            assert_eq!(
                with("head = 5").expect("parses").connection.idle,
                Duration::from_secs(120),
                "with a table that does not say"
            );
        }

        #[test]
        fn the_idle_limit_may_be_shorter_than_the_head_limit_and_longer() {
            // Neither bounds the other: one is a wait between requests,
            // the other the time a request head takes once begun.
            for (idle, head) in [(1, 3600), (3600, 1), (30, 30)] {
                let config = with(&format!("idle = {idle}\nhead = {head}")).expect("parses");
                assert_eq!(
                    (config.connection.idle, config.connection.head),
                    (Duration::from_secs(idle), Duration::from_secs(head)),
                    "idle {idle}, head {head}"
                );
            }
        }

        #[test]
        fn the_wait_after_a_connection_is_told_to_close_has_no_key() {
            let error = with("goodbye = 2").expect_err("no such key");
            assert!(error.0.contains("`goodbye`"), "{error}");
            assert_eq!(
                with("idle = 9").expect("parses").connection.goodbye,
                Duration::from_secs(5),
                "five seconds whatever the keys say"
            );
        }

        #[test]
        fn one_second_and_one_hour_are_the_bounds_and_both_are_taken() {
            for key in ["handshake", "first_head", "head", "idle"] {
                for seconds in [1, CONNECTION_LIMIT_MAX] {
                    assert!(
                        with(&format!("{key} = {seconds}")).is_ok(),
                        "{key} = {seconds}"
                    );
                }
            }
        }

        #[test]
        fn zero_is_refused_by_name() {
            for key in ["handshake", "first_head", "head", "idle"] {
                let error = with(&format!("{key} = 0")).expect_err(key);
                assert_eq!(
                    error.0,
                    format!("limits.timeouts.{key} must be more than 0"),
                    "{key}"
                );
            }
        }

        #[test]
        fn more_than_an_hour_is_refused_by_name_with_the_bound() {
            for key in ["handshake", "first_head", "head", "idle"] {
                for seconds in [3601_u64, 86_400, u64::from(u32::MAX)] {
                    let error = with(&format!("{key} = {seconds}")).expect_err(key);
                    assert_eq!(
                        error.0,
                        format!(
                            "limits.timeouts.{key} must be at most 3600 seconds, and is {seconds}"
                        ),
                        "{key} = {seconds}"
                    );
                }
            }
        }

        #[test]
        fn a_value_that_is_not_whole_seconds_is_refused() {
            for key in ["handshake", "first_head", "head", "idle"] {
                for value in ["-1", "1.5", "\"10\"", "true", "[10]"] {
                    let error = with(&format!("{key} = {value}")).expect_err(key);
                    assert!(error.0.contains(key), "{key} = {value}: {error}");
                }
            }
        }

        #[test]
        fn the_wait_after_a_failed_accept_has_no_key() {
            let error = with("accept_pause = 2").expect_err("no such key");
            assert!(error.0.contains("`accept_pause`"), "{error}");
        }

        /// The least configuration with `line` in a `[limits]` table.
        fn with_limit(line: &str) -> Result<Config, ConfigError> {
            parse(&format!("{LEAST}\n[limits]\n{line}\n"))
        }

        #[test]
        fn a_listener_takes_120_reports_a_minute_and_20_from_one_address_unless_told() {
            let reports = parse(LEAST).expect("parses").reports;
            assert_eq!(
                (reports.per_minute, reports.per_address),
                (120, 20),
                "the defaults"
            );
            assert_eq!(reports, crate::api::ReportLimits::DEFAULT, "the gate's own");
        }

        #[test]
        fn both_report_limits_are_read() {
            let reports = with_limit("csp_reports = 300\ncsp_reports_per_address = 30")
                .expect("parses")
                .reports;
            assert_eq!(
                (reports.per_minute, reports.per_address),
                (300, 30),
                "as written"
            );
        }

        #[test]
        fn the_minutes_number_alone_is_read_and_one_addresss_stays() {
            let reports = with_limit("csp_reports = 500").expect("parses").reports;
            assert_eq!(
                (reports.per_minute, reports.per_address),
                (500, 20),
                "one key"
            );
        }

        #[test]
        fn one_addresss_number_alone_is_read_and_the_minutes_stays() {
            let reports = with_limit("csp_reports_per_address = 7")
                .expect("parses")
                .reports;
            assert_eq!(
                (reports.per_minute, reports.per_address),
                (120, 7),
                "one key"
            );
        }

        #[test]
        fn a_minutes_number_under_20_lowers_one_addresss_default_with_it() {
            let reports = with_limit("csp_reports = 5").expect("parses").reports;
            assert_eq!(
                (reports.per_minute, reports.per_address),
                (5, 5),
                "never more than all"
            );
        }

        #[test]
        fn the_bounds_of_the_report_limits_are_taken() {
            let least = with_limit("csp_reports = 1\ncsp_reports_per_address = 1")
                .expect("the least")
                .reports;
            assert_eq!((least.per_minute, least.per_address), (1, 1), "the least");
            let most = with_limit("csp_reports = 6000\ncsp_reports_per_address = 6000")
                .expect("the most")
                .reports;
            assert_eq!(
                (most.per_minute, most.per_address),
                (6000, 6000),
                "the most"
            );
        }

        #[test]
        fn the_report_limits_change_no_other_limit() {
            let config = with_limit("csp_reports = 300").expect("parses");
            let base = parse(LEAST).expect("parses");
            assert_eq!(config.connection, base.connection, "the connection limits");
            assert_eq!(config.body_limit, base.body_limit, "the body limit");
        }

        #[test]
        fn no_reports_at_all_is_refused_by_name() {
            for key in ["csp_reports", "csp_reports_per_address"] {
                let error = with_limit(&format!("{key} = 0")).expect_err("zero");
                assert_eq!(
                    error.0,
                    format!("limits.{key} must be more than 0"),
                    "{key}"
                );
            }
        }

        #[test]
        fn more_reports_than_6000_is_refused_by_name_with_the_bound() {
            for key in ["csp_reports", "csp_reports_per_address"] {
                for value in [6001_u64, 1_000_000, u64::from(u32::MAX) + 1] {
                    let error = with_limit(&format!("{key} = {value}")).expect_err("too many");
                    assert_eq!(
                        error.0,
                        format!("limits.{key} must be at most 6000, and is {value}"),
                        "{key} = {value}"
                    );
                }
            }
        }

        #[test]
        fn more_from_one_address_than_from_all_is_refused_with_both_numbers() {
            let error =
                with_limit("csp_reports = 10\ncsp_reports_per_address = 11").expect_err("more");
            assert_eq!(
                error.0,
                "limits.csp_reports_per_address (11) must be at most limits.csp_reports (10)",
                "the two numbers"
            );
            let error =
                with_limit("csp_reports_per_address = 121").expect_err("more than the default");
            assert!(
                error
                    .0
                    .contains("(121) must be at most limits.csp_reports (120)"),
                "{error}"
            );
        }

        #[test]
        fn a_report_limit_that_is_not_a_whole_number_is_refused() {
            for key in ["csp_reports", "csp_reports_per_address"] {
                for value in ["-1", "1.5", "\"ten\"", "true"] {
                    let error = with_limit(&format!("{key} = {value}")).expect_err(value);
                    assert!(error.0.contains(key), "{key} = {value}: {error}");
                }
            }
        }

        #[test]
        fn a_report_limit_written_outside_the_limits_table_is_refused() {
            let error = parse(&format!("{LEAST}\ncsp_reports = 5\n")).expect_err("the wrong table");
            assert!(error.0.contains("csp_reports"), "{error}");
        }

        #[test]
        fn a_listener_holds_512_connections_at_once_unless_told() {
            assert_eq!(
                parse(LEAST).expect("parses").connection.max,
                512,
                "with no [limits] table"
            );
            assert_eq!(
                with_limit("body = 1024").expect("parses").connection.max,
                512,
                "with a [limits] table that does not say"
            );
        }

        #[test]
        fn the_most_connections_at_once_is_read_and_changes_no_other_limit() {
            let config = with_limit("connections = 2000").expect("parses");
            assert_eq!(
                config.connection,
                ConnectionLimits {
                    max: 2000,
                    ..ConnectionLimits::default()
                },
                "only the most at once differs"
            );
        }

        #[test]
        fn one_connection_and_65536_are_the_bounds_and_both_are_taken() {
            for connections in [1_usize, 65_536] {
                assert_eq!(
                    with_limit(&format!("connections = {connections}"))
                        .expect("parses")
                        .connection
                        .max,
                    connections,
                    "{connections}"
                );
            }
        }

        #[test]
        fn no_connections_at_all_is_refused_by_name() {
            let error = with_limit("connections = 0").expect_err("zero");
            assert_eq!(
                error.0, "limits.connections must be more than 0",
                "a listener that takes no connection serves nothing"
            );
        }

        #[test]
        fn more_connections_than_65536_is_refused_by_name_with_the_bound() {
            for connections in [65_537_u64, 1_000_000, u64::from(u32::MAX)] {
                let error =
                    with_limit(&format!("connections = {connections}")).expect_err("too many");
                assert_eq!(
                    error.0,
                    format!("limits.connections must be at most 65536, and is {connections}"),
                    "{connections}"
                );
            }
        }

        #[test]
        fn a_number_of_connections_that_is_not_a_whole_number_is_refused() {
            for value in ["-1", "1.5", "\"512\"", "true", "[512]"] {
                let error = with_limit(&format!("connections = {value}")).expect_err(value);
                assert!(error.0.contains("connections"), "{value}: {error}");
            }
        }
    }

    #[test]
    fn retention_and_supported_extensions_have_defaults() {
        let text = FULL
            .replace("retention_default = \"18m\"\n", "")
            .replace("supported_critical = [\"known\", \"other\"]\n", "");
        let config = parse(&text).expect("parses");
        assert_eq!(
            config.retention_default,
            Retention::Years(7),
            "with no retention_default, vCons are kept seven years"
        );
        assert_eq!(
            config.supported_critical,
            BTreeSet::new(),
            "with no supported_critical, no extension is supported"
        );
    }

    /// The configuration with only the keys that have no default.
    const LEAST: &str = r#"
listen = "127.0.0.1:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"
"#;

    #[test]
    fn periods_and_limits_have_defaults() {
        let config = parse(LEAST).expect("parses");
        assert_eq!(
            config.purge_interval,
            Duration::from_secs(3600),
            "the purge sweep runs hourly by default"
        );
        assert_eq!(
            config.rotation_warn,
            Duration::from_secs(30 * 86_400),
            "an overdue rotation is warned of after 30 days by default"
        );
        assert_eq!(
            config.body_limit,
            16 << 20,
            "a body may be 16 MiB by default"
        );
        assert_eq!(
            config.inflated_limit,
            64 << 20,
            "a body may inflate to 64 MiB by default"
        );
        assert_eq!(
            config.read_timeout,
            Duration::from_secs(60),
            "a request has 60 seconds to arrive by default"
        );
        assert_eq!(
            config.mcp,
            crate::mcp::Limits::DEFAULT,
            "with no [mcp] table, the MCP endpoint takes its default limits"
        );
    }

    /// A key left out of a table that is present takes its default; the
    /// others keep their written values.
    #[test]
    fn each_limit_defaults_alone() {
        let base = parse(FULL).expect("parses");
        for (line, default) in [
            (
                "body = 10485760\n",
                Config {
                    body_limit: 16 << 20,
                    ..base.clone()
                },
            ),
            (
                "inflated = 104857600\n",
                Config {
                    inflated_limit: 64 << 20,
                    ..base.clone()
                },
            ),
            (
                "read = 30\n",
                Config {
                    read_timeout: Duration::from_secs(60),
                    ..base.clone()
                },
            ),
            (
                "purge_interval = 1800\n",
                Config {
                    purge_interval: Duration::from_secs(3600),
                    ..base.clone()
                },
            ),
            (
                "rotation_warn = 604800\n",
                Config {
                    rotation_warn: Duration::from_secs(30 * 86_400),
                    ..base.clone()
                },
            ),
            (
                "[limits.timeouts]\nread = 30\nhandshake = 5\nfirst_head = 15\nhead = 20\nidle = 90\n",
                Config {
                    read_timeout: Duration::from_secs(60),
                    connection: ConnectionLimits {
                        max: base.connection.max,
                        ..ConnectionLimits::default()
                    },
                    ..base.clone()
                },
            ),
            (
                "connections = 64\n",
                Config {
                    connection: ConnectionLimits {
                        max: 512,
                        ..base.connection
                    },
                    ..base.clone()
                },
            ),
            (
                "handshake = 5\n",
                Config {
                    connection: ConnectionLimits {
                        handshake: Duration::from_secs(10),
                        ..base.connection
                    },
                    ..base.clone()
                },
            ),
            (
                "first_head = 15\n",
                Config {
                    connection: ConnectionLimits {
                        first_head: Duration::from_secs(30),
                        ..base.connection
                    },
                    ..base.clone()
                },
            ),
            (
                "head = 20\n",
                Config {
                    connection: ConnectionLimits {
                        head: Duration::from_secs(30),
                        ..base.connection
                    },
                    ..base.clone()
                },
            ),
            (
                "idle = 90\n",
                Config {
                    connection: ConnectionLimits {
                        idle: Duration::from_secs(120),
                        ..base.connection
                    },
                    ..base.clone()
                },
            ),
            (
                "calls_per_second = 20\n",
                Config {
                    mcp: crate::mcp::Limits {
                        calls_per_second: 100,
                        ..base.mcp
                    },
                    ..base.clone()
                },
            ),
            (
                "concurrent = 8\n",
                Config {
                    mcp: crate::mcp::Limits {
                        concurrent: 100,
                        ..base.mcp
                    },
                    ..base.clone()
                },
            ),
            (
                "response_bytes = 262144\n",
                Config {
                    mcp: crate::mcp::Limits {
                        response_bytes: 1 << 20,
                        ..base.mcp
                    },
                    ..base.clone()
                },
            ),
        ] {
            assert!(FULL.contains(line), "{line}");
            assert_eq!(parse(&FULL.replace(line, "")), Ok(default), "{line}");
        }
    }

    /// The defaults differ from the full file's values, so each test above
    /// tells a default from a written value.
    #[test]
    fn the_full_file_sets_no_value_equal_to_its_default() {
        let (full, least) = (parse(FULL).expect("full"), parse(LEAST).expect("least"));
        assert_ne!(
            full.purge_interval, least.purge_interval,
            "FULL writes purge_interval as a value other than its default"
        );
        assert_ne!(
            full.rotation_warn, least.rotation_warn,
            "FULL writes rotation_warn as a value other than its default"
        );
        assert_ne!(
            full.body_limit, least.body_limit,
            "FULL writes limits.body as a value other than its default"
        );
        assert_ne!(
            full.inflated_limit, least.inflated_limit,
            "FULL writes limits.inflated as a value other than its default"
        );
        assert_ne!(
            full.read_timeout, least.read_timeout,
            "FULL writes limits.timeouts.read as a value other than its default"
        );
        assert_ne!(
            full.mcp.calls_per_second, least.mcp.calls_per_second,
            "FULL writes mcp.calls_per_second as a value other than its default"
        );
        assert_ne!(
            full.mcp.concurrent, least.mcp.concurrent,
            "FULL writes mcp.concurrent as a value other than its default"
        );
        assert_ne!(
            full.mcp.response_bytes, least.mcp.response_bytes,
            "FULL writes mcp.response_bytes as a value other than its default"
        );
        assert_ne!(
            full.storage, least.storage,
            "FULL writes storage.objects as a value other than its default"
        );
        let parquet = full.storage.parquet.expect("FULL has storage.parquet");
        let least_parquet = parquet_least();
        for (key, differs) in [
            (
                "storage.parquet.pack_interval",
                parquet.pack_interval != least_parquet.pack_interval,
            ),
            (
                "storage.parquet.pack_limit",
                parquet.pack_limit != least_parquet.pack_limit,
            ),
            (
                "storage.parquet.reader_grace",
                parquet.reader_grace != least_parquet.reader_grace,
            ),
        ] {
            assert!(
                differs,
                "FULL writes {key} as a value other than its default"
            );
        }
        let ObjectStore::S3(s3) = full.storage.objects else {
            panic!("FULL keeps its objects in S3")
        };
        let least_s3 = s3_least();
        for (key, differs) in [
            ("storage.s3.endpoint", s3.endpoint != least_s3.endpoint),
            ("storage.s3.prefix", s3.prefix != least_s3.prefix),
            (
                "storage.s3.path_style",
                s3.path_style != least_s3.path_style,
            ),
            (
                "storage.s3.access_key_id",
                s3.access_key_id != least_s3.access_key_id,
            ),
            (
                "storage.s3.secret_access_key_file",
                s3.secret_access_key_file != least_s3.secret_access_key_file,
            ),
            (
                "storage.s3.allow_http",
                s3.allow_http != least_s3.allow_http,
            ),
            ("storage.s3.ca_file", s3.ca_file != least_s3.ca_file),
            (
                "storage.s3.allow_versioning",
                s3.allow_versioning != least_s3.allow_versioning,
            ),
            ("storage.s3.timeout", s3.timeout != least_s3.timeout),
        ] {
            assert!(
                differs,
                "FULL writes {key} as a value other than its default"
            );
        }
        let export = full.export.s3.expect("FULL has export.s3");
        let least_export = parse(&format!("{LEAST}\n[export.s3]\nregion = \"us-east-1\"\n"))
            .expect("least export")
            .export
            .s3
            .expect("export.s3");
        for (key, differs) in [
            (
                "export.s3.endpoint",
                export.endpoint != least_export.endpoint,
            ),
            ("export.s3.region", export.region != least_export.region),
            (
                "export.s3.path_style",
                export.path_style != least_export.path_style,
            ),
            (
                "export.s3.access_key_id",
                export.access_key_id != least_export.access_key_id,
            ),
            (
                "export.s3.secret_access_key_file",
                export.secret_access_key_file != least_export.secret_access_key_file,
            ),
            (
                "export.s3.allow_http",
                export.allow_http != least_export.allow_http,
            ),
            ("export.s3.ca_file", export.ca_file != least_export.ca_file),
            ("export.s3.timeout", export.timeout != least_export.timeout),
        ] {
            assert!(
                differs,
                "FULL writes {key} as a value other than its default"
            );
        }
        assert_ne!(
            full.catalog, least.catalog,
            "FULL writes catalog.backend as a value other than its default"
        );
        let CatalogStore::Postgres(pg) = full.catalog else {
            panic!("FULL keeps its catalogs in PostgreSQL")
        };
        let least_pg = catalog::postgres_least();
        for (key, differs) in [
            ("catalog.postgres.port", pg.port != least_pg.port),
            ("catalog.postgres.ca_file", pg.ca_file != least_pg.ca_file),
            (
                "catalog.postgres.password_file",
                pg.password_file != least_pg.password_file,
            ),
            (
                "catalog.postgres.client_cert_file",
                pg.client_cert_file != least_pg.client_cert_file,
            ),
            (
                "catalog.postgres.client_key_file",
                pg.client_key_file != least_pg.client_key_file,
            ),
            ("catalog.postgres.pool", pg.pool != least_pg.pool),
            (
                "catalog.postgres.connect_timeout",
                pg.connect_timeout != least_pg.connect_timeout,
            ),
            (
                "catalog.postgres.statement_timeout",
                pg.statement_timeout != least_pg.statement_timeout,
            ),
            (
                "catalog.postgres.lock_timeout",
                pg.lock_timeout != least_pg.lock_timeout,
            ),
        ] {
            assert!(
                differs,
                "FULL writes {key} as a value other than its default"
            );
        }
    }

    /// The S3 settings with only the keys that have no default.
    fn s3_least() -> S3Settings {
        let text = format!(
            "{LEAST}\n[storage]\nobjects = \"s3\"\n[storage.s3]\nregion = \"us-east-1\"\nbucket = \"vcon\"\n"
        );
        match parse(&text)
            .expect("the least S3 configuration reads")
            .storage
            .objects
        {
            ObjectStore::S3(s3) => s3,
            ObjectStore::Local => panic!("objects = \"s3\" keeps the objects in S3"),
        }
    }

    #[test]
    fn without_a_storage_table_everything_is_local() {
        assert_eq!(
            parse(LEAST).expect("parses").storage,
            Storage {
                objects: ObjectStore::Local,
                parquet: None,
            },
            "a file written before storage backends keeps its meaning"
        );
    }

    /// The Parquet settings of an empty `[storage.parquet]` table.
    fn parquet_least() -> ParquetSettings {
        let text = format!(
            "{LEAST}\n[storage]\nobjects = \"s3\"\n[storage.s3]\nregion = \"us-east-1\"\n\
             bucket = \"vcon\"\n[storage.parquet]\n"
        );
        parse(&text)
            .expect("the least Parquet configuration reads")
            .storage
            .parquet
            .expect("an empty [storage.parquet] turns Parquet storage on")
    }

    #[test]
    fn parquet_settings_have_defaults() {
        assert_eq!(
            parquet_least(),
            ParquetSettings {
                pack_interval: PACK_INTERVAL,
                pack_limit: PACK_LIMIT,
                reader_grace: READER_GRACE,
            },
            "an empty table takes every default"
        );
        assert_eq!(READER_GRACE, Duration::from_secs(900), "15 minutes");
        assert_eq!(PACK_INTERVAL, Duration::from_secs(300), "five minutes");
        assert_eq!(PACK_LIMIT, 16 << 20, "16 MiB");
    }

    #[test]
    fn s3_settings_have_defaults() {
        assert_eq!(
            s3_least(),
            S3Settings {
                endpoint: None,
                region: "us-east-1".to_owned(),
                bucket: "vcon".to_owned(),
                prefix: String::new(),
                path_style: false,
                access_key_id: None,
                secret_access_key_file: None,
                allow_http: false,
                ca_file: None,
                allow_versioning: false,
                timeout: Duration::from_secs(30),
            },
            "only region and bucket are required"
        );
    }

    /// `[export.s3]` takes `[storage.s3]`'s keys but the bucket, the
    /// prefix and `allow_versioning`, with the same defaults and rules,
    /// each refusal naming its own table.
    #[test]
    fn export_s3_settings_are_read_and_checked_as_storage_s3_is() {
        let export = |extra: &str| format!("{LEAST}\n[export.s3]\nregion = \"us-east-1\"\n{extra}");
        assert_eq!(
            parse(LEAST).expect("least").export,
            Export::default(),
            "no table"
        );
        assert_eq!(
            parse(&export("")).expect("least export").export.s3,
            Some(ExportS3 {
                endpoint: None,
                region: "us-east-1".to_owned(),
                path_style: false,
                access_key_id: None,
                secret_access_key_file: None,
                allow_http: false,
                ca_file: None,
                timeout: S3_TIMEOUT,
            }),
            "only region is required"
        );
        for (extra, key) in [
            (
                "endpoint = \"http://127.0.0.1:9000\"\n",
                "export.s3.allow_http",
            ),
            ("endpoint = \"objects.example.com\"\n", "export.s3.endpoint"),
            (
                "access_key_id = \"vcon\"\n",
                "export.s3.secret_access_key_file",
            ),
            (
                "secret_access_key_file = \"/s\"\n",
                "export.s3.access_key_id",
            ),
            ("timeout = 0\n", "export.s3.timeout"),
            (
                "endpoint = \"http://127.0.0.1:9000\"\nallow_http = true\nca_file = \"/c.pem\"\n",
                "export.s3.ca_file",
            ),
            ("bucket = \"vcon\"\n", "bucket"),
            ("prefix = \"a/\"\n", "prefix"),
            ("allow_versioning = true\n", "allow_versioning"),
        ] {
            let ConfigError(message) = parse(&export(extra)).expect_err(extra);
            assert!(message.contains(key), "{extra}: {message}");
        }
        assert!(
            parse(&format!("{LEAST}\n[export.s3]\n"))
                .expect_err("no region")
                .0
                .contains("region"),
            "region is required"
        );
    }

    /// The settings an `s3://` target uses: this service, with `--to`'s
    /// bucket and prefix; from `[storage.s3]`, its service and keys.
    #[test]
    fn an_export_service_names_its_bucket_and_prefix() {
        let full = parse(FULL).expect("full");
        let export = full.export.s3.expect("export.s3");
        let s = export.for_bucket("reports", "vcon/");
        assert_eq!(
            (s.bucket.as_str(), s.prefix.as_str()),
            ("reports", "vcon/"),
            "the target's bucket and prefix"
        );
        assert_eq!(s.endpoint, export.endpoint, "the export's endpoint");
        assert_eq!(s.region, export.region, "the export's region");
        assert_eq!(
            s.access_key_id, export.access_key_id,
            "the export's access key id"
        );
        assert_eq!(
            s.secret_access_key_file, export.secret_access_key_file,
            "the export's secret file"
        );
        assert_eq!(s.ca_file, export.ca_file, "the export's CA file");
        assert_eq!(
            (s.path_style, s.allow_http),
            (true, true),
            "FULL sets path_style and allow_http"
        );
        assert_eq!(s.timeout, Duration::from_secs(120), "the export's timeout");
        assert!(!s.allow_versioning, "never read for an export");
        let ObjectStore::S3(storage) = full.storage.objects else {
            panic!("FULL keeps its objects in S3")
        };
        let from = ExportS3::from(&storage).for_bucket(&storage.bucket, &storage.prefix);
        assert_eq!(
            from,
            S3Settings {
                allow_versioning: false,
                ..storage
            },
            "storage.s3's service and keys, whole"
        );
    }

    #[test]
    fn s3_settings_that_do_not_hold_together_are_named() {
        let s3 = |extra: &str| {
            format!(
                "{LEAST}\n[storage]\nobjects = \"s3\"\n[storage.s3]\nregion = \"us-east-1\"\nbucket = \"vcon\"\n{extra}"
            )
        };
        for (text, key) in [
            (
                format!("{LEAST}\n[storage]\nobjects = \"s3\"\n"),
                "storage.s3",
            ),
            (
                format!("{LEAST}\n[storage]\nobjects = \"s3\"\n[storage.s3]\nbucket = \"vcon\"\n"),
                "region",
            ),
            (
                format!(
                    "{LEAST}\n[storage]\nobjects = \"s3\"\n[storage.s3]\nregion = \"us-east-1\"\n"
                ),
                "bucket",
            ),
            (
                s3("endpoint = \"http://127.0.0.1:9000\"\n"),
                "storage.s3.allow_http",
            ),
            (
                s3("endpoint = \"objects.example.com\"\n"),
                "storage.s3.endpoint",
            ),
            (
                s3("access_key_id = \"vcon-server\"\n"),
                "storage.s3.secret_access_key_file",
            ),
            (
                s3("secret_access_key_file = \"/etc/vcon-server/s3.secret\"\n"),
                "storage.s3.access_key_id",
            ),
            (s3("timeout = 0\n"), "storage.s3.timeout"),
            (
                s3(
                    "endpoint = \"http://127.0.0.1:9000\"\nallow_http = true\nca_file = \"/etc/vcon-server/s3-ca.pem\"\n",
                ),
                "storage.s3.ca_file",
            ),
            (s3("prefix = \"/server-a/\"\n"), "storage.s3.prefix"),
            (s3("prefix = \"server-a\"\n"), "storage.s3.prefix"),
            (
                s3("bucket = \"\"\n").replace("bucket = \"vcon\"\n", ""),
                "storage.s3.bucket",
            ),
            (
                format!("{LEAST}\n[storage]\nobjects = \"tape\"\n"),
                "objects",
            ),
            (format!("{LEAST}\n[storage.parquet]\n"), "storage.parquet"),
            (
                format!("{LEAST}\n[storage]\nobjects = \"local\"\n[storage.parquet]\n"),
                "storage.parquet",
            ),
            (
                s3("[storage.parquet]\npack_interval = 0\n"),
                "storage.parquet.pack_interval",
            ),
            (
                s3("[storage.parquet]\npack_limit = 0\n"),
                "storage.parquet.pack_limit",
            ),
            (
                s3("[storage.parquet]\nreader_grace = 0\n"),
                "storage.parquet.reader_grace",
            ),
            (
                format!(
                    "{LEAST}\n[storage]\nobjects = \"local\"\n[storage.s3]\nregion = \"us-east-1\"\nbucket = \"vcon\"\n"
                ),
                "storage.s3",
            ),
        ] {
            let error = parse(&text).expect_err(key);
            assert!(error.0.contains(key), "the error names {key}: {error}");
        }
    }

    #[test]
    fn an_http_endpoint_reads_when_allowed() {
        let text = format!(
            "{LEAST}\n[storage]\nobjects = \"s3\"\n[storage.s3]\nregion = \"us-east-1\"\nbucket = \"vcon\"\nendpoint = \"http://127.0.0.1:9000\"\nallow_http = true\n"
        );
        assert!(
            parse(&text).is_ok(),
            "allow_http lets a test container's plain endpoint through"
        );
    }

    /// Each error names its key.
    #[test]
    fn every_missing_key_without_a_default_is_named() {
        for (line, key) in [
            ("listen = \"127.0.0.1:8443\"\n", "listen"),
            ("data_dir = \"/var/lib/vcon-server\"\n", "data_dir"),
            (
                "master_key = \"/etc/vcon-server/master.key\"\n",
                "master_key",
            ),
            ("cert = \"/etc/vcon-server/cert.pem\"\n", "cert"),
            ("key = \"/etc/vcon-server/key.pem\"\n", "key"),
        ] {
            assert!(FULL.contains(line), "{line}");
            let error = parse(&FULL.replace(line, "")).expect_err(key);
            assert!(error.0.contains(&format!("`{key}`")), "{key}: {error}");
        }
    }

    #[test]
    fn an_unknown_key_is_named() {
        for (text, key) in [
            (format!("colour = \"blue\"\n{FULL}"), "colour"),
            (FULL.replace("read = 30", "read = 30\nwrite = 30"), "write"),
            (FULL.replace("[tls]", "[tls]\nca = \"x\""), "ca"),
            (
                FULL.replace("[mcp]", "[mcp]\nresources = true"),
                "resources",
            ),
        ] {
            let error = parse(&text).expect_err(key);
            assert!(error.0.contains(&format!("`{key}`")), "{key}: {error}");
        }
    }

    #[test]
    fn a_bad_value_is_named() {
        for (from, to, key) in [
            ("\"127.0.0.1:8443\"", "\"localhost\"", "listen"),
            ("\"18m\"", "\"7w\"", "retention_default"),
            (
                "purge_interval = 1800",
                "purge_interval = 0",
                "purge_interval",
            ),
            (
                "rotation_warn = 604800",
                "rotation_warn = 0",
                "rotation_warn",
            ),
            ("body = 10485760", "body = 0", "limits.body"),
            ("inflated = 104857600", "inflated = 0", "limits.inflated"),
            ("connections = 64", "connections = 0", "limits.connections"),
            (
                "connections = 64",
                "connections = 65537",
                "limits.connections",
            ),
            (
                "calls_per_second = 20",
                "calls_per_second = 0",
                "mcp.calls_per_second",
            ),
            ("concurrent = 8", "concurrent = 0", "mcp.concurrent"),
            (
                "response_bytes = 262144",
                "response_bytes = 0",
                "mcp.response_bytes",
            ),
            (
                "concurrent = 8",
                "concurrent = 4294967296",
                "mcp.concurrent",
            ),
            ("read = 30", "read = 0", "limits.timeouts.read"),
            ("read = 30", "read = -1", "read"),
            (
                "handshake = 5",
                "handshake = 0",
                "limits.timeouts.handshake",
            ),
            (
                "handshake = 5",
                "handshake = 3601",
                "limits.timeouts.handshake",
            ),
            ("handshake = 5", "handshake = \"5s\"", "handshake"),
            (
                "first_head = 15",
                "first_head = 0",
                "limits.timeouts.first_head",
            ),
            (
                "first_head = 15",
                "first_head = 3601",
                "limits.timeouts.first_head",
            ),
            ("first_head = 15", "first_head = 1.5", "first_head"),
            ("\nhead = 20", "\nhead = 0", "limits.timeouts.head"),
            ("\nhead = 20", "\nhead = 3601", "limits.timeouts.head"),
            ("\nhead = 20", "\nhead = 18446744073709551615", "head"),
            ("idle = 90", "idle = 0", "limits.timeouts.idle"),
            ("idle = 90", "idle = 3601", "limits.timeouts.idle"),
            ("idle = 90", "idle = \"2m\"", "idle"),
        ] {
            let error = parse(&FULL.replace(from, to)).expect_err(key);
            assert!(error.0.contains(key), "{key}: {error}");
        }
    }

    /// The website's configuration page lists every key the full file
    /// sets, each with the default the code gives it, and every example
    /// file on the site reads.
    mod site_page {
        use super::*;

        fn page(name: &str) -> String {
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../website/content/docs")
                .join(name);
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        }

        /// Every key in a TOML table, dotted.
        fn keys(table: &toml::Table, prefix: &str, out: &mut Vec<String>) {
            for (key, value) in table {
                let name = format!("{prefix}{key}");
                match value {
                    toml::Value::Table(inner) => keys(inner, &format!("{name}."), out),
                    _ => out.push(name),
                }
            }
        }

        /// The table row for `key`, by its first cell.
        fn row<'a>(text: &'a str, key: &str) -> &'a str {
            let start = format!("| `{key}` |");
            let mut rows = text.lines().filter(|l| l.starts_with(&start));
            let row = rows.next().unwrap_or_else(|| panic!("no row for {key}"));
            assert!(rows.next().is_none(), "two rows for {key}");
            row
        }

        #[test]
        fn every_key_is_listed_with_its_default() {
            let text = page("configuration.md");
            let defaults = parse(LEAST).expect("least");
            let mut all = Vec::new();
            keys(&FULL.parse::<toml::Table>().expect("toml"), "", &mut all);
            assert_eq!(all.len(), 63, "{all:?}");
            let pg = catalog::postgres_least();
            for key in all {
                let default = match key.as_str() {
                    "listen" | "data_dir" | "master_key" | "tls.cert" | "tls.key" => {
                        "required".to_owned()
                    }
                    "operator.listen" => "none: `listen` serves both".to_owned(),
                    "operator.tls.cert" => "`tls.cert`".to_owned(),
                    "operator.tls.key" => "`tls.key`".to_owned(),
                    "public_origin" | "operator.public_origin" => {
                        assert_eq!(defaults.public_origin, None, "the page says none");
                        "none: the listener serves no session".to_owned()
                    }
                    "retention_default" => format!("`{}`", defaults.retention_default),
                    "purge_interval" => format!("`{}`", defaults.purge_interval.as_secs()),
                    "rotation_warn" => format!("`{}`", defaults.rotation_warn.as_secs()),
                    "limits.body" => format!("`{}`", defaults.body_limit),
                    "limits.inflated" => format!("`{}`", defaults.inflated_limit),
                    "limits.connections" => format!("`{}`", defaults.connection.max),
                    "limits.csp_reports" => format!("`{}`", defaults.reports.per_minute),
                    "limits.csp_reports_per_address" => {
                        format!("`{}`", defaults.reports.per_address)
                    }
                    "limits.timeouts.read" => format!("`{}`", defaults.read_timeout.as_secs()),
                    "limits.timeouts.handshake" => {
                        format!("`{}`", defaults.connection.handshake.as_secs())
                    }
                    "limits.timeouts.first_head" => {
                        format!("`{}`", defaults.connection.first_head.as_secs())
                    }
                    "limits.timeouts.head" => format!("`{}`", defaults.connection.head.as_secs()),
                    "limits.timeouts.idle" => format!("`{}`", defaults.connection.idle.as_secs()),
                    "mcp.calls_per_second" => format!("`{}`", defaults.mcp.calls_per_second),
                    "mcp.concurrent" => format!("`{}`", defaults.mcp.concurrent),
                    "mcp.response_bytes" => format!("`{}`", defaults.mcp.response_bytes),
                    "storage.objects" => "`local`".to_owned(),
                    "storage.s3.region" | "storage.s3.bucket" => "required with `s3`".to_owned(),
                    "storage.s3.endpoint"
                    | "storage.s3.access_key_id"
                    | "storage.s3.secret_access_key_file"
                    | "storage.s3.ca_file" => "none".to_owned(),
                    "storage.s3.prefix" => "empty".to_owned(),
                    "storage.s3.path_style" => format!("`{}`", s3_least().path_style),
                    "storage.s3.allow_http" => format!("`{}`", s3_least().allow_http),
                    "storage.s3.allow_versioning" => {
                        format!("`{}`", s3_least().allow_versioning)
                    }
                    "storage.s3.timeout" => format!("`{}`", s3_least().timeout.as_secs()),
                    "storage.parquet.pack_interval" => {
                        format!("`{}`", parquet_least().pack_interval.as_secs())
                    }
                    "storage.parquet.pack_limit" => format!("`{}`", parquet_least().pack_limit),
                    "storage.parquet.reader_grace" => {
                        format!("`{}`", parquet_least().reader_grace.as_secs())
                    }
                    "export.s3.region" => "required with `export.s3`".to_owned(),
                    "export.s3.endpoint"
                    | "export.s3.access_key_id"
                    | "export.s3.secret_access_key_file"
                    | "export.s3.ca_file" => "none".to_owned(),
                    "export.s3.path_style" => format!("`{}`", s3_least().path_style),
                    "export.s3.allow_http" => format!("`{}`", s3_least().allow_http),
                    "export.s3.timeout" => format!("`{}`", S3_TIMEOUT.as_secs()),
                    "catalog.backend" => "`local`".to_owned(),
                    "catalog.postgres.hosts"
                    | "catalog.postgres.dbname"
                    | "catalog.postgres.user" => "required with `postgres`".to_owned(),
                    "catalog.postgres.ca_file"
                    | "catalog.postgres.password_file"
                    | "catalog.postgres.client_cert_file"
                    | "catalog.postgres.client_key_file" => "none".to_owned(),
                    "catalog.postgres.port" => format!("`{}`", pg.port),
                    "catalog.postgres.pool" => format!("`{}`", pg.pool),
                    "catalog.postgres.connect_timeout" => {
                        format!("`{}`", pg.connect_timeout.as_secs())
                    }
                    "catalog.postgres.statement_timeout" => {
                        format!("`{}`", pg.statement_timeout.as_secs())
                    }
                    "catalog.postgres.lock_timeout" => {
                        format!("`{}`", pg.lock_timeout.as_secs())
                    }
                    "supported_critical" => {
                        assert!(
                            defaults.supported_critical.is_empty(),
                            "the page says none, which holds only while the default is empty"
                        );
                        "none".to_owned()
                    }
                    other => panic!("a key this test does not know: {other}"),
                };
                let row = row(&text, &key);
                let last = row
                    .trim_end_matches('|')
                    .rsplit('|')
                    .next()
                    .expect("a last cell")
                    .trim();
                assert!(
                    last.starts_with(&default),
                    "{key}: `{last}` is not {default}"
                );
            }
        }

        /// The limits no key sets are stated as the code fixes them.
        #[test]
        fn every_fixed_limit_is_listed_with_its_value() {
            let text = page("configuration.md");
            for (limit, value) in [
                ("Signed layers in one body", vcon::read::MAX_SIGNED_LAYERS),
                (
                    "Arrays and objects open at once",
                    vcon::read::MAX_JSON_DEPTH,
                ),
            ] {
                let start = format!("| {limit} |");
                let rows: Vec<&str> = text.lines().filter(|l| l.starts_with(&start)).collect();
                assert_eq!(rows.len(), 1, "one row for {limit}");
                let cell = rows[0].split('|').nth(2).expect("a value cell").trim();
                assert_eq!(cell, format!("`{value}`"), "{limit}");
            }
        }

        #[test]
        fn every_example_file_on_the_site_reads() {
            let mut examples = 0;
            for name in [
                "configuration.md",
                "getting-started.md",
                "container.md",
                "postgres-catalog.md",
            ] {
                let text = page(name);
                for block in text.split("```toml\n").skip(1) {
                    let toml = block.split("```").next().expect("a block");
                    parse(toml).unwrap_or_else(|e| panic!("{name}: {e}\n{toml}"));
                    examples += 1;
                }
            }
            assert_eq!(
                examples, 7,
                "four on the configuration page, one each on getting started, the \
                 container page and the PostgreSQL catalog page, so a block the split misses \
                 is caught"
            );
        }
    }

    /// The `[catalog]` table: where the catalogs and the registry are kept,
    /// and how the PostgreSQL database is reached.
    mod catalog {
        use super::*;

        /// LEAST with its objects in a bucket and its catalogs in PostgreSQL,
        /// with only the `[catalog.postgres]` keys that have no default and
        /// a password file.
        pub(super) fn postgres_text() -> String {
            format!(
                "{LEAST}\n[storage]\nobjects = \"s3\"\n\n[storage.s3]\nregion = \"us-east-1\"\n\
                 bucket = \"vcon\"\n\n[catalog]\nbackend = \"postgres\"\n\n[catalog.postgres]\n\
                 hosts = [\"db1.example.com\"]\ndbname = \"vcon\"\nuser = \"vcon_app\"\n\
                 password_file = \"/etc/vcon-server/pg.password\"\n"
            )
        }

        /// `postgres_text` with `from` replaced by `to`; `from` must be there.
        fn edited(from: &str, to: &str) -> String {
            let text = postgres_text();
            assert!(text.contains(from), "the least text holds {from:?}");
            text.replace(from, to)
        }

        /// `postgres_text` with `line` added to `[catalog.postgres]`.
        fn with(line: &str) -> String {
            format!("{}{line}\n", postgres_text())
        }

        /// The PostgreSQL settings `text` reads to.
        fn postgres(text: &str) -> PostgresSettings {
            match parse(text)
                .unwrap_or_else(|e| panic!("{e}\n{text}"))
                .catalog
            {
                CatalogStore::Postgres(pg) => *pg,
                CatalogStore::Local => panic!("backend = \"postgres\" keeps the catalogs there"),
            }
        }

        /// The PostgreSQL settings with only the keys that have no default.
        pub(super) fn postgres_least() -> PostgresSettings {
            postgres(&postgres_text())
        }

        /// The message `text` is refused with.
        fn refused(text: &str) -> String {
            parse(text).expect_err(text).0
        }

        mod success {
            use super::*;

            #[test]
            fn a_postgres_catalog_is_read_with_its_defaults() {
                assert_eq!(
                    postgres_least(),
                    PostgresSettings {
                        hosts: vec!["db1.example.com".to_owned()],
                        port: 5432,
                        dbname: "vcon".to_owned(),
                        user: "vcon_app".to_owned(),
                        ca_file: None,
                        password_file: Some("/etc/vcon-server/pg.password".into()),
                        client_cert_file: None,
                        client_key_file: None,
                        pool: 16,
                        connect_timeout: Duration::from_secs(5),
                        statement_timeout: Duration::from_secs(30),
                        lock_timeout: Duration::from_secs(10),
                    },
                    "only hosts, dbname, user and a way to log in are required"
                );
            }

            #[test]
            fn without_a_catalog_table_the_catalogs_are_local() {
                assert_eq!(
                    parse(LEAST).expect("parses").catalog,
                    CatalogStore::Local,
                    "a file written before the PostgreSQL catalog keeps its meaning"
                );
            }

            #[test]
            fn backend_local_is_the_local_catalog() {
                let text = format!("{LEAST}\n[catalog]\nbackend = \"local\"\n");
                assert_eq!(
                    parse(&text).expect("parses").catalog,
                    CatalogStore::Local,
                    "\"local\" names the default"
                );
            }

            #[test]
            fn a_client_certificate_alone_is_enough_to_log_in() {
                let text = edited(
                    "password_file = \"/etc/vcon-server/pg.password\"\n",
                    "client_cert_file = \"/c.pem\"\nclient_key_file = \"/c.key\"\n",
                );
                let pg = postgres(&text);
                assert_eq!(
                    (pg.password_file, pg.client_cert_file, pg.client_key_file),
                    (None, Some("/c.pem".into()), Some("/c.key".into())),
                    "a certificate and its key, with no password"
                );
            }

            #[test]
            fn a_password_and_a_client_certificate_are_read_together() {
                let pg = postgres(&with(
                    "client_cert_file = \"/c.pem\"\nclient_key_file = \"/c.key\"",
                ));
                assert!(
                    pg.password_file.is_some() && pg.client_cert_file.is_some(),
                    "both ways of logging in, as pg_hba.conf's clientcert=verify-full asks"
                );
            }

            #[test]
            fn several_hosts_are_kept_in_their_order() {
                let text = edited(
                    "hosts = [\"db1.example.com\"]",
                    "hosts = [\"db2.example.com\", \"db1.example.com\", \"192.0.2.10\"]",
                );
                assert_eq!(
                    postgres(&text).hosts,
                    ["db2.example.com", "db1.example.com", "192.0.2.10"],
                    "tried in the order written"
                );
            }

            #[test]
            fn parquet_storage_with_a_postgres_catalog_is_read() {
                let text = edited(
                    "bucket = \"vcon\"\n",
                    "bucket = \"vcon\"\n\n[storage.parquet]\n",
                );
                let config = parse(&text).expect("parses");
                assert!(
                    config.storage.parquet.is_some()
                        && matches!(config.catalog, CatalogStore::Postgres(_)),
                    "Parquet files in the bucket and the catalogs in PostgreSQL"
                );
            }
        }

        mod failure {
            use super::*;

            /// `text` is refused with a message holding every one of `names`.
            fn refused_naming(text: &str, names: &[&str]) {
                let message = refused(text);
                for name in names {
                    assert!(message.contains(name), "names {name}: {message}");
                }
            }

            #[test]
            fn an_unknown_backend_is_named() {
                refused_naming(
                    &edited("backend = \"postgres\"", "backend = \"mysql\""),
                    &["catalog.backend", "\"mysql\""],
                );
            }

            #[test]
            fn a_postgres_table_without_the_postgres_backend_is_refused() {
                refused_naming(
                    &edited("backend = \"postgres\"", "backend = \"local\""),
                    &["catalog.postgres", "catalog.backend"],
                );
            }

            #[test]
            fn the_postgres_backend_without_its_table_is_refused() {
                let text = format!(
                    "{LEAST}\n[storage]\nobjects = \"s3\"\n[storage.s3]\nregion = \"us-east-1\"\n\
                     bucket = \"vcon\"\n[catalog]\nbackend = \"postgres\"\n"
                );
                refused_naming(&text, &["catalog.postgres"]);
            }

            #[test]
            fn a_postgres_catalog_with_local_objects_is_refused() {
                let text = format!(
                    "{LEAST}\n[catalog]\nbackend = \"postgres\"\n[catalog.postgres]\n\
                     hosts = [\"db1.example.com\"]\ndbname = \"vcon\"\nuser = \"vcon_app\"\n\
                     password_file = \"/p\"\n"
                );
                refused_naming(&text, &["catalog.backend", "storage.objects", "\"s3\""]);
            }

            #[test]
            fn an_empty_host_list_is_refused() {
                refused_naming(
                    &edited("hosts = [\"db1.example.com\"]", "hosts = []"),
                    &["catalog.postgres.hosts"],
                );
            }

            #[test]
            fn an_empty_host_name_is_refused() {
                refused_naming(
                    &edited("hosts = [\"db1.example.com\"]", "hosts = [\"\"]"),
                    &["catalog.postgres.hosts"],
                );
            }

            #[test]
            fn a_host_that_is_not_a_dns_name_is_refused() {
                for host in [
                    "db_1.example.com",
                    "db..example.com",
                    "-db.example.com",
                    "db .example.com",
                ] {
                    refused_naming(
                        &edited("\"db1.example.com\"", &format!("{host:?}")),
                        &["catalog.postgres.hosts", host],
                    );
                }
            }

            #[test]
            fn a_missing_host_list_is_named() {
                refused_naming(&edited("hosts = [\"db1.example.com\"]\n", ""), &["hosts"]);
            }

            #[test]
            fn a_port_of_zero_is_refused() {
                refused_naming(&with("port = 0"), &["catalog.postgres.port"]);
            }

            #[test]
            fn a_port_above_65535_is_refused() {
                refused_naming(&with("port = 65536"), &["catalog.postgres.port"]);
            }

            #[test]
            fn an_empty_database_name_is_refused() {
                refused_naming(
                    &edited("dbname = \"vcon\"", "dbname = \"\""),
                    &["catalog.postgres.dbname"],
                );
            }

            #[test]
            fn a_missing_database_name_is_named() {
                refused_naming(&edited("dbname = \"vcon\"\n", ""), &["dbname"]);
            }

            #[test]
            fn an_empty_user_is_refused() {
                refused_naming(
                    &edited("user = \"vcon_app\"", "user = \"\""),
                    &["catalog.postgres.user"],
                );
            }

            #[test]
            fn a_missing_user_is_named() {
                refused_naming(&edited("user = \"vcon_app\"\n", ""), &["user"]);
            }

            #[test]
            fn no_password_and_no_client_certificate_is_refused() {
                refused_naming(
                    &edited("password_file = \"/etc/vcon-server/pg.password\"\n", ""),
                    &[
                        "catalog.postgres.password_file",
                        "catalog.postgres.client_cert_file",
                    ],
                );
            }

            #[test]
            fn a_client_certificate_without_its_key_is_refused() {
                refused_naming(
                    &with("client_cert_file = \"/c.pem\""),
                    &["catalog.postgres.client_key_file"],
                );
            }

            #[test]
            fn a_client_key_without_its_certificate_is_refused() {
                refused_naming(
                    &with("client_key_file = \"/c.key\""),
                    &["catalog.postgres.client_cert_file"],
                );
            }

            #[test]
            fn an_empty_file_name_is_refused() {
                for (key, text) in [
                    ("ca_file", with("ca_file = \"\"")),
                    (
                        "password_file",
                        edited("\"/etc/vcon-server/pg.password\"", "\"\""),
                    ),
                    (
                        "client_cert_file",
                        with("client_cert_file = \"\"\nclient_key_file = \"/c.key\""),
                    ),
                    (
                        "client_key_file",
                        with("client_cert_file = \"/c.pem\"\nclient_key_file = \"\""),
                    ),
                ] {
                    refused_naming(&text, &[&format!("catalog.postgres.{key}")]);
                }
            }

            #[test]
            fn a_pool_of_zero_is_refused() {
                refused_naming(&with("pool = 0"), &["catalog.postgres.pool"]);
            }

            #[test]
            fn a_pool_too_large_to_count_is_refused() {
                refused_naming(&with("pool = 4294967296"), &["catalog.postgres.pool"]);
            }

            #[test]
            fn a_connect_timeout_of_zero_is_refused() {
                refused_naming(
                    &with("connect_timeout = 0"),
                    &["catalog.postgres.connect_timeout"],
                );
            }

            #[test]
            fn a_statement_timeout_of_zero_is_refused() {
                refused_naming(
                    &with("statement_timeout = 0"),
                    &["catalog.postgres.statement_timeout"],
                );
            }

            #[test]
            fn a_lock_timeout_of_zero_is_refused() {
                refused_naming(
                    &with("lock_timeout = 0"),
                    &["catalog.postgres.lock_timeout"],
                );
            }

            /// There is no sslmode: TLS with verification is the only mode.
            #[test]
            fn an_sslmode_key_is_an_unknown_key() {
                refused_naming(&with("sslmode = \"disable\""), &["`sslmode`"]);
            }

            #[test]
            fn an_unknown_key_in_the_catalog_table_is_named() {
                refused_naming(
                    &edited(
                        "backend = \"postgres\"",
                        "backend = \"postgres\"\nreplicas = 2",
                    ),
                    &["`replicas`"],
                );
            }
        }

        mod edge {
            use super::*;

            fn host_text(host: &str) -> String {
                edited("\"db1.example.com\"", &format!("{host:?}"))
            }

            #[test]
            fn ipv6_hosts_are_read() {
                for host in ["2001:db8::10", "::1", "2001:db8:0:0:0:0:0:10"] {
                    assert_eq!(
                        postgres(&host_text(host)).hosts,
                        [host],
                        "an IPv6 address is a host as written"
                    );
                }
            }

            #[test]
            fn a_bracketed_ipv6_host_is_refused_with_its_bare_form() {
                let message = refused(&host_text("[2001:db8::10]"));
                assert!(
                    message.contains("catalog.postgres.hosts")
                        && message.contains("\"2001:db8::10\""),
                    "names the key and the form to write: {message}"
                );
            }

            /// A Unix socket carries no TLS, and TLS is the only way the
            /// catalog is reached.
            #[test]
            fn a_unix_socket_host_is_refused_because_tls_is_required() {
                let message = refused(&host_text("/var/run/postgresql"));
                assert!(
                    message.contains("catalog.postgres.hosts")
                        && message.contains("Unix socket")
                        && message.contains("TLS"),
                    "says why: {message}"
                );
            }

            #[test]
            fn a_unicode_host_name_is_refused_with_a_pointer_to_its_ascii_form() {
                let message = refused(&host_text("bücher.example.com"));
                assert!(
                    message.contains("catalog.postgres.hosts") && message.contains("xn--"),
                    "an internationalized name is written in its xn-- form: {message}"
                );
            }

            #[test]
            fn an_ascii_form_of_a_unicode_host_name_is_read() {
                assert_eq!(
                    postgres(&host_text("xn--bcher-kva.example.com")).hosts,
                    ["xn--bcher-kva.example.com"],
                    "the xn-- form is a DNS name"
                );
            }

            #[test]
            fn unicode_database_and_role_names_are_read() {
                let text = edited("dbname = \"vcon\"", "dbname = \"vcon_数据\"")
                    .replace("user = \"vcon_app\"", "user = \"vcon_ünï\"");
                let pg = postgres(&text);
                assert_eq!(
                    (pg.dbname.as_str(), pg.user.as_str()),
                    ("vcon_数据", "vcon_ünï"),
                    "PostgreSQL names may hold any character but NUL"
                );
            }

            /// PostgreSQL cuts a longer name to 63 bytes, which would log in
            /// to another database or as another role than written.
            #[test]
            fn a_name_of_63_bytes_is_read_and_one_of_64_refused() {
                let at_limit = "数".repeat(21);
                assert_eq!(at_limit.len(), 63, "21 three-byte characters");
                for key in ["dbname", "user"] {
                    let line = |name: &str| format!("{key} = \"{name}\"");
                    let from = if key == "dbname" {
                        "dbname = \"vcon\""
                    } else {
                        "user = \"vcon_app\""
                    };
                    let pg = postgres(&edited(from, &line(&at_limit)));
                    let read = if key == "dbname" { pg.dbname } else { pg.user };
                    assert_eq!(read, at_limit, "{key}: 63 bytes is PostgreSQL's limit");
                    let message = refused(&edited(from, &line(&format!("{at_limit}a"))));
                    assert!(
                        message.contains(&format!("catalog.postgres.{key}"))
                            && message.contains("63 bytes"),
                        "{key}: {message}"
                    );
                }
            }

            #[test]
            fn a_nul_in_a_name_is_refused() {
                for (from, to, key) in [
                    ("dbname = \"vcon\"", "dbname = \"vc\\u0000on\"", "dbname"),
                    ("user = \"vcon_app\"", "user = \"vcon\\u0000\"", "user"),
                ] {
                    let message = refused(&edited(from, to));
                    assert!(
                        message.contains(&format!("catalog.postgres.{key}"))
                            && message.contains("NUL"),
                        "{key}: {message}"
                    );
                }
            }

            #[test]
            fn the_first_and_last_ports_are_read() {
                for port in [1, 65535] {
                    assert_eq!(
                        u64::from(postgres(&with(&format!("port = {port}"))).port),
                        port,
                        "every TCP port but 0"
                    );
                }
            }

            /// PostgreSQL holds both timeouts as milliseconds in a 32-bit
            /// integer: 2147483647 ms, so 2147483 whole seconds.
            #[test]
            fn timeouts_at_the_postgresql_limit_are_read_and_one_past_refused() {
                for key in ["statement_timeout", "lock_timeout"] {
                    let pg = postgres(&with(&format!("{key} = 2147483")));
                    let read = if key == "statement_timeout" {
                        pg.statement_timeout
                    } else {
                        pg.lock_timeout
                    };
                    assert_eq!(read, Duration::from_secs(2_147_483), "{key} at the limit");
                    let message = refused(&with(&format!("{key} = 2147484")));
                    assert!(
                        message.contains(&format!("catalog.postgres.{key}"))
                            && message.contains("2147483"),
                        "{key}: {message}"
                    );
                }
            }

            #[test]
            fn a_one_second_connect_timeout_is_read() {
                assert_eq!(
                    postgres(&with("connect_timeout = 1")).connect_timeout,
                    Duration::from_secs(1),
                    "the shortest timeout"
                );
            }

            #[test]
            fn the_same_host_twice_is_refused() {
                let message = refused(&edited(
                    "hosts = [\"db1.example.com\"]",
                    "hosts = [\"db1.example.com\", \"DB1.example.com\"]",
                ));
                assert!(
                    message.contains("catalog.postgres.hosts") && message.contains("twice"),
                    "a DNS name in any case is one host: {message}"
                );
            }

            #[test]
            fn a_dns_label_of_63_characters_is_read_and_one_of_64_refused() {
                let label = "a".repeat(63);
                let host = format!("{label}.example.com");
                assert_eq!(
                    postgres(&host_text(&host)).hosts,
                    [host],
                    "63 is the DNS limit"
                );
                let message = refused(&host_text(&format!("a{label}.example.com")));
                assert!(message.contains("catalog.postgres.hosts"), "{message}");
            }

            #[test]
            fn a_host_name_of_more_than_253_characters_is_refused() {
                let label = "a".repeat(63);
                let at_limit = format!("{label}.{label}.{label}.{}", "a".repeat(61));
                assert_eq!(at_limit.len(), 253, "four labels");
                assert_eq!(
                    postgres(&host_text(&at_limit)).hosts,
                    std::slice::from_ref(&at_limit),
                    "253 characters is the DNS limit"
                );
                let message = refused(&host_text(&format!("a{at_limit}")));
                assert!(message.contains("catalog.postgres.hosts"), "{message}");
            }

            #[test]
            fn a_port_written_in_a_host_is_refused_with_the_port_key_named() {
                let message = refused(&host_text("db1.example.com:5432"));
                assert!(
                    message.contains("catalog.postgres.hosts")
                        && message.contains("catalog.postgres.port"),
                    "the port has its own key: {message}"
                );
            }
        }
    }

    #[test]
    fn a_missing_file_is_named() {
        let error = load(Path::new("/nonexistent/vcon-server.toml")).expect_err("missing");
        assert!(error.0.contains("/nonexistent/vcon-server.toml"), "{error}");
    }
}
