//! The server's configuration: one TOML file. A configuration error stops
//! the start and names its key, and an unknown key is an error.

use std::collections::BTreeSet;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::retention::Retention;

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
    /// Extension names accepted in `critical` and `must_support`.
    pub supported_critical: BTreeSet<String>,
    /// The MCP server's limits.
    pub mcp: crate::mcp::Limits,
    /// Where the data is kept.
    pub storage: Storage,
    /// How `export parquet` reaches a bucket: the `[export]` table.
    pub export: Export,
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
}

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
    let retention_default = match raw.retention_default {
        None => Retention::Years(7),
        Some(text) => Retention::parse(&text).ok_or_else(|| {
            ConfigError(format!(
                "retention_default: {text:?} is not a period such as 7y, 18m or 90d"
            ))
        })?,
    };
    Ok(Config {
        listen: raw.listen,
        tls_cert: raw.tls.cert,
        tls_key: raw.tls.key,
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
        supported_critical: raw.supported_critical.into_iter().collect(),
        mcp: mcp_limits(&raw.mcp)?,
        storage: storage(raw.storage)?,
        export: Export {
            s3: raw.export.s3.map(export_s3).transpose()?,
        },
    })
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
    Ok(Storage { objects })
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
    #[serde(default)]
    timeouts: RawTimeouts,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTimeouts {
    read: Option<u64>,
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

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"

[limits]
body = 10485760
inflated = 104857600

[limits.timeouts]
read = 30

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

[export.s3]
endpoint = "https://analytics.example.com"
region = "eu-west-2"
path_style = true
access_key_id = "vcon-export"
secret_access_key_file = "/etc/vcon-server/export.secret"
allow_http = true
ca_file = "/etc/vcon-server/export-ca.pem"
timeout = 120
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
                supported_critical: ["known", "other"].map(String::from).into(),
                mcp: crate::mcp::Limits {
                    calls_per_second: 20,
                    concurrent: 8,
                    response_bytes: 262_144,
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
            }),
            "every key the file writes is read into its field"
        );
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
                "[limits.timeouts]\nread = 30\n",
                Config {
                    read_timeout: Duration::from_secs(60),
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
                objects: ObjectStore::Local
            },
            "a file written before storage backends keeps its meaning"
        );
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
            assert_eq!(all.len(), 35, "{all:?}");
            for key in all {
                let default = match key.as_str() {
                    "listen" | "data_dir" | "master_key" | "tls.cert" | "tls.key" => {
                        "required".to_owned()
                    }
                    "retention_default" => format!("`{}`", defaults.retention_default),
                    "purge_interval" => format!("`{}`", defaults.purge_interval.as_secs()),
                    "rotation_warn" => format!("`{}`", defaults.rotation_warn.as_secs()),
                    "limits.body" => format!("`{}`", defaults.body_limit),
                    "limits.inflated" => format!("`{}`", defaults.inflated_limit),
                    "limits.timeouts.read" => format!("`{}`", defaults.read_timeout.as_secs()),
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
                    "export.s3.region" => "required with `export.s3`".to_owned(),
                    "export.s3.endpoint"
                    | "export.s3.access_key_id"
                    | "export.s3.secret_access_key_file"
                    | "export.s3.ca_file" => "none".to_owned(),
                    "export.s3.path_style" => format!("`{}`", s3_least().path_style),
                    "export.s3.allow_http" => format!("`{}`", s3_least().allow_http),
                    "export.s3.timeout" => format!("`{}`", S3_TIMEOUT.as_secs()),
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
            for name in ["configuration.md", "getting-started.md", "container.md"] {
                let text = page(name);
                for block in text.split("```toml\n").skip(1) {
                    let toml = block.split("```").next().expect("a block");
                    parse(toml).unwrap_or_else(|e| panic!("{name}: {e}\n{toml}"));
                    examples += 1;
                }
            }
            assert_eq!(
                examples, 4,
                "two on the configuration page, one each on getting started and the container \
                 page, so a block the split misses is caught"
            );
        }
    }

    #[test]
    fn a_missing_file_is_named() {
        let error = load(Path::new("/nonexistent/vcon-server.toml")).expect_err("missing");
        assert!(error.0.contains("/nonexistent/vcon-server.toml"), "{error}");
    }
}
