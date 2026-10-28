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
    /// The MCP server's limits.
    pub mcp: crate::mcp::Limits,
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
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMcp {
    calls_per_second: Option<u64>,
    concurrent: Option<u64>,
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
            assert_eq!(all.len(), 14, "{all:?}");
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
            for name in ["configuration.md", "getting-started.md"] {
                let text = page(name);
                for block in text.split("```toml\n").skip(1) {
                    let toml = block.split("```").next().expect("a block");
                    parse(toml).unwrap_or_else(|e| panic!("{name}: {e}\n{toml}"));
                    examples += 1;
                }
            }
            assert_eq!(
                examples, 2,
                "one example file on each page, so a block the split misses is caught"
            );
        }
    }

    #[test]
    fn a_missing_file_is_named() {
        let error = load(Path::new("/nonexistent/vcon-server.toml")).expect_err("missing");
        assert!(error.0.contains("/nonexistent/vcon-server.toml"), "{error}");
    }
}
