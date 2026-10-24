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
/// seconds and sizes in bytes; each must be more than zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub listen: SocketAddr,
    pub tls_cert: PathBuf,
    pub tls_key: PathBuf,
    pub data_dir: PathBuf,
    pub master_key: PathBuf,
    /// For tenants that set none; 7 years unless configured.
    pub retention_default: Retention,
    pub purge_interval: Duration,
    pub rotation_warn: Duration,
    /// The largest request body.
    pub body_limit: usize,
    /// The largest size gzip inflation may reach while a body is inspected.
    pub inflated_limit: u64,
    /// How long a request body may take to arrive.
    pub read_timeout: Duration,
    /// Extension names accepted in `critical` and `must_support`.
    pub supported_critical: BTreeSet<String>,
}

/// A configuration that could not be read, naming the key at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reads the configuration file at `path`.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
    parse(&text)
}

/// Reads a configuration from its TOML text.
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
        purge_interval: seconds("purge_interval", raw.purge_interval)?,
        rotation_warn: seconds("rotation_warn", raw.rotation_warn)?,
        body_limit: usize::try_from(positive("limits.body", raw.limits.body)?)
            .map_err(|_| ConfigError("limits.body is too large".to_owned()))?,
        inflated_limit: positive("limits.inflated", raw.limits.inflated)?,
        read_timeout: seconds("limits.timeouts.read", raw.limits.timeouts.read)?,
        supported_critical: raw.supported_critical.into_iter().collect(),
    })
}

fn positive(key: &str, value: u64) -> Result<u64, ConfigError> {
    if value == 0 {
        return Err(ConfigError(format!("{key} must be more than 0")));
    }
    Ok(value)
}

fn seconds(key: &str, value: u64) -> Result<Duration, ConfigError> {
    positive(key, value).map(Duration::from_secs)
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
    purge_interval: u64,
    rotation_warn: u64,
    limits: RawLimits,
    #[serde(default)]
    supported_critical: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTls {
    cert: PathBuf,
    key: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLimits {
    body: u64,
    inflated: u64,
    timeouts: RawTimeouts,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTimeouts {
    read: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
listen = "127.0.0.1:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"
retention_default = "18m"
purge_interval = 3600
rotation_warn = 2592000
supported_critical = ["known", "other"]

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"

[limits]
body = 10485760
inflated = 104857600

[limits.timeouts]
read = 30
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
                purge_interval: Duration::from_secs(3600),
                rotation_warn: Duration::from_secs(2_592_000),
                body_limit: 10_485_760,
                inflated_limit: 104_857_600,
                read_timeout: Duration::from_secs(30),
                supported_critical: ["known", "other"].map(String::from).into(),
            })
        );
    }

    #[test]
    fn retention_and_supported_extensions_have_defaults() {
        let text = FULL
            .replace("retention_default = \"18m\"\n", "")
            .replace("supported_critical = [\"known\", \"other\"]\n", "");
        let config = parse(&text).expect("parses");
        assert_eq!(config.retention_default, Retention::Years(7));
        assert_eq!(config.supported_critical, BTreeSet::new());
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
            ("purge_interval = 3600\n", "purge_interval"),
            ("rotation_warn = 2592000\n", "rotation_warn"),
            ("cert = \"/etc/vcon-server/cert.pem\"\n", "cert"),
            ("key = \"/etc/vcon-server/key.pem\"\n", "key"),
            ("body = 10485760\n", "body"),
            ("inflated = 104857600\n", "inflated"),
            ("read = 30\n", "read"),
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
                "purge_interval = 3600",
                "purge_interval = 0",
                "purge_interval",
            ),
            (
                "rotation_warn = 2592000",
                "rotation_warn = 0",
                "rotation_warn",
            ),
            ("body = 10485760", "body = 0", "limits.body"),
            ("inflated = 104857600", "inflated = 0", "limits.inflated"),
            ("read = 30", "read = 0", "limits.timeouts.read"),
            ("read = 30", "read = -1", "read"),
        ] {
            let error = parse(&FULL.replace(from, to)).expect_err(key);
            assert!(error.0.contains(key), "{key}: {error}");
        }
    }

    #[test]
    fn a_missing_file_is_named() {
        let error = load(Path::new("/nonexistent/vcon-server.toml")).expect_err("missing");
        assert!(error.0.contains("/nonexistent/vcon-server.toml"), "{error}");
    }
}
