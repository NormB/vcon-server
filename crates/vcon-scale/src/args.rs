//! The command line of `vcon-scale run`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use crate::mix::DEFAULT_MIX;

/// The most vCons one run may store.
pub const MAX_COUNT: u64 = 100_000_000;
/// The most concurrent clients.
pub const MAX_CONCURRENCY: usize = 256;
/// The load cap on thor-02: twice its 14 cores.
pub const DEFAULT_MAX_LOAD: f64 = 28.0;

/// The flags that take a value; every flag does.
const FLAGS: [&str; 16] = [
    "--url",
    "--ca",
    "--key-file",
    "--count",
    "--concurrency",
    "--reads",
    "--list-pages",
    "--seed",
    "--mix",
    "--max-load",
    "--data-dir",
    "--server-pid",
    "--out",
    "--commit",
    "--disk",
    "--label",
];

/// What one run does.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// The server's base URL; it must be https.
    pub url: String,
    /// A PEM file of the CA that signed the server's certificate.
    pub ca: Option<PathBuf>,
    /// A file holding the API key (write and read scopes).
    pub key_file: PathBuf,
    /// How many vCons to store.
    pub count: u64,
    /// How many clients send at once.
    pub concurrency: usize,
    /// How many reads after the stores.
    pub reads: u64,
    /// How many list pages to follow.
    pub list_pages: u64,
    /// The generator's seed.
    pub seed: u64,
    /// The size mix, as `weight:bytes,...`.
    pub mix: String,
    /// No start above this 1-minute load.
    pub max_load: f64,
    /// The server's data directory, measured after the stores.
    pub data_dir: Option<PathBuf>,
    /// The server's process id, for its peak memory and CPU time.
    pub server_pid: Option<u32>,
    /// Where the result file goes.
    pub out: PathBuf,
    /// The commit the server was built from.
    pub commit: String,
    /// The disk the data directory is on, as stated by the caller.
    pub disk: String,
    /// The run's name in the result file.
    pub label: String,
}

/// Why a command line was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgError {
    /// A flag this command does not have.
    Unknown(String),
    /// A flag given without its value.
    NoValue(String),
    /// A required flag not given.
    Missing(&'static str),
    /// A value that is not of the flag's type.
    Bad {
        /// The flag.
        flag: String,
        /// The value given.
        value: String,
    },
    /// A value outside the flag's range.
    Range {
        /// The flag.
        flag: String,
        /// The value given.
        value: String,
    },
    /// A flag given twice.
    Twice(String),
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgError::Unknown(flag) => write!(f, "{flag}: no such flag"),
            ArgError::NoValue(flag) => write!(f, "{flag}: no value given"),
            ArgError::Missing(flag) => write!(f, "{flag} is required"),
            ArgError::Bad { flag, value } => write!(f, "{flag}: {value:?} is not allowed"),
            ArgError::Range { flag, value } => write!(f, "{flag}: {value:?} is out of range"),
            ArgError::Twice(flag) => write!(f, "{flag}: given twice"),
        }
    }
}

impl std::error::Error for ArgError {}

/// The given flags and their values.
struct Given(BTreeMap<&'static str, String>);

impl Given {
    fn text(&self, flag: &'static str) -> Option<String> {
        self.0.get(flag).cloned()
    }

    fn required(&self, flag: &'static str) -> Result<String, ArgError> {
        self.text(flag).ok_or(ArgError::Missing(flag))
    }

    /// A number in `range`, or `default` when the flag is not given.
    fn number<T>(
        &self,
        flag: &'static str,
        default: Option<T>,
        range: impl std::ops::RangeBounds<T>,
    ) -> Result<T, ArgError>
    where
        T: std::str::FromStr + PartialOrd,
    {
        let Some(value) = self.text(flag) else {
            return default.ok_or(ArgError::Missing(flag));
        };
        let number: T = value.parse().map_err(|_| ArgError::Bad {
            flag: flag.to_owned(),
            value: value.clone(),
        })?;
        if range.contains(&number) {
            Ok(number)
        } else {
            Err(ArgError::Range {
                flag: flag.to_owned(),
                value,
            })
        }
    }
}

/// Reads the arguments after `run`.
///
/// # Errors
///
/// An [`ArgError`] for the first argument that is wrong.
pub fn parse(args: &[String]) -> Result<Options, ArgError> {
    let mut given = Given(BTreeMap::new());
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let flag = FLAGS
            .iter()
            .find(|f| **f == arg.as_str())
            .ok_or_else(|| ArgError::Unknown(arg.clone()))?;
        let value = rest.next().ok_or_else(|| ArgError::NoValue(arg.clone()))?;
        if given.0.insert(flag, value.clone()).is_some() {
            return Err(ArgError::Twice(arg.clone()));
        }
    }
    let url = given.required("--url")?;
    if !url.starts_with("https://") {
        return Err(ArgError::Bad {
            flag: "--url".to_owned(),
            value: url,
        });
    }
    let max_load: f64 = given.number("--max-load", Some(DEFAULT_MAX_LOAD), 0.0..)?;
    if !(max_load > 0.0 && max_load.is_finite()) {
        return Err(ArgError::Range {
            flag: "--max-load".to_owned(),
            value: given.text("--max-load").unwrap_or_default(),
        });
    }
    Ok(Options {
        url,
        ca: given.text("--ca").map(PathBuf::from),
        key_file: PathBuf::from(given.required("--key-file")?),
        count: given.number("--count", None, 1..=MAX_COUNT)?,
        concurrency: given.number("--concurrency", Some(8), 1..=MAX_CONCURRENCY)?,
        reads: given.number("--reads", Some(1000), 0..=MAX_COUNT)?,
        list_pages: given.number("--list-pages", Some(10), 0..=10_000)?,
        seed: given.number("--seed", Some(1), ..)?,
        mix: given
            .text("--mix")
            .unwrap_or_else(|| DEFAULT_MIX.to_owned()),
        max_load,
        data_dir: given.text("--data-dir").map(PathBuf::from),
        server_pid: given
            .text("--server-pid")
            .map(|_| given.number("--server-pid", None, 1..))
            .transpose()?,
        out: PathBuf::from(given.required("--out")?),
        commit: given.text("--commit").unwrap_or_default(),
        disk: given.text("--disk").unwrap_or_default(),
        label: given.text("--label").unwrap_or_else(|| "run".to_owned()),
    })
}
