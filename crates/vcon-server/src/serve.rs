//! `vcon-server serve`: the server, run from its configuration.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::Arc;

use tokio::net::TcpListener;

use crate::api::{self, App, Limits, TlsError};
use crate::clock::SystemClock;
use crate::config::Config;
use crate::envelope::{Key, KeyError};
use crate::fs::OsFs;
use crate::tenants::{TenantError, Tenants};

/// Why the server did not start, or stopped.
#[derive(Debug)]
pub enum ServeError {
    /// Another server holds the data directory.
    DataDirInUse,
    /// The data directory's lock could not be taken.
    DataDir(io::Error),
    /// The master key file could not be read.
    MasterKey(KeyError),
    /// server.sqlite was made under another master key.
    WrongMasterKey,
    /// server.sqlite could not be opened.
    Registry(TenantError),
    /// The certificate or key could not be used.
    Tls(TlsError),
    /// The listening address could not be bound.
    Listen(io::Error),
    /// Serving failed.
    Serve(io::Error),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeError::DataDirInUse => {
                f.write_str("the data directory is in use by another vcon-server")
            }
            ServeError::DataDir(e) => write!(f, "the data directory cannot be locked: {e}"),
            ServeError::MasterKey(e) => write!(f, "the master key cannot be read: {e:?}"),
            ServeError::WrongMasterKey => {
                f.write_str("the master key is not the one server.sqlite was made under")
            }
            ServeError::Registry(e) => write!(f, "server.sqlite cannot be opened: {e:?}"),
            ServeError::Tls(e) => write!(f, "the TLS certificate or key cannot be used: {e:?}"),
            ServeError::Listen(e) => write!(f, "the listening address cannot be bound: {e}"),
            ServeError::Serve(e) => write!(f, "serving failed: {e}"),
        }
    }
}

/// The data directory's lock, held while this value lives: the open file
/// is kept only for its lock.
#[derive(Debug)]
pub struct DataDirLock {
    _held: File,
}

/// Takes the data directory's exclusive lock, so that no second server
/// opens the same tenants: a second open's recovery could remove a write's
/// temporary file.
pub fn lock_data_dir(dir: &Path) -> Result<DataDirLock, ServeError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("lock"))
        .map_err(ServeError::DataDir)?;
    // SAFETY: the descriptor is this open file's, and flock takes only it
    // and flags.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        return Err(match error.kind() {
            io::ErrorKind::WouldBlock => ServeError::DataDirInUse,
            _ => ServeError::DataDir(error),
        });
    }
    Ok(DataDirLock { _held: file })
}

/// Runs the server until `shutdown` completes: locks the data directory,
/// opens the registry under the master key, and serves the API over TLS.
pub async fn run(config: &Config, shutdown: impl Future<Output = ()>) -> Result<(), ServeError> {
    let _lock = lock_data_dir(&config.data_dir)?;
    let master = Key::load_master(&config.master_key).map_err(ServeError::MasterKey)?;
    let tenants = Tenants::open(
        &config.data_dir,
        master,
        Arc::new(OsFs),
        Arc::new(SystemClock),
    )
    .map_err(|e| match e {
        TenantError::WrongKey => ServeError::WrongMasterKey,
        other => ServeError::Registry(other),
    })?;
    let tls = api::tls_config(&config.tls_cert, &config.tls_key).map_err(ServeError::Tls)?;
    let listener = TcpListener::bind(config.listen)
        .await
        .map_err(ServeError::Listen)?;
    let app = app_from(config, Arc::new(tenants));
    tracing::info!(listen = %config.listen, "serving");
    api::serve(listener, tls, api::router(Arc::new(app)), shutdown)
        .await
        .map_err(ServeError::Serve)?;
    tracing::info!("stopped");
    Ok(())
}

/// What the API serves from, as the configuration sets it.
fn app_from(config: &Config, tenants: Arc<Tenants>) -> App {
    App {
        tenants,
        limits: Limits {
            body: config.body_limit,
            inflated: config.inflated_limit,
            body_timeout: config.read_timeout,
        },
        supported: config.supported_critical.clone(),
        retention_default: config.retention_default,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::retention::Retention;

    #[test]
    fn the_api_takes_its_limits_and_extensions_from_the_configuration() {
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = Arc::new(
            Tenants::open(
                dir.path(),
                Key::generate().expect("key"),
                Arc::new(OsFs),
                Arc::new(SystemClock),
            )
            .expect("opens"),
        );
        let config = Config {
            listen: "127.0.0.1:8443".parse().expect("address"),
            tls_cert: "cert.pem".into(),
            tls_key: "key.pem".into(),
            data_dir: dir.path().to_owned(),
            master_key: "master.key".into(),
            retention_default: Retention::Months(18),
            purge_interval: Duration::from_secs(3600),
            rotation_warn: Duration::from_secs(86400),
            body_limit: 12_345,
            inflated_limit: 67_890,
            read_timeout: Duration::from_secs(17),
            supported_critical: ["known".to_owned()].into(),
        };
        let app = app_from(&config, tenants.clone());
        assert!(Arc::ptr_eq(&app.tenants, &tenants));
        assert_eq!(app.limits.body, 12_345);
        assert_eq!(app.limits.inflated, 67_890);
        assert_eq!(app.limits.body_timeout, Duration::from_secs(17));
        assert_eq!(app.supported, config.supported_critical);
        assert_eq!(app.retention_default, config.retention_default);
    }

    #[test]
    fn a_data_directory_is_held_by_one_server_at_a_time() {
        let dir = tempfile::tempdir().expect("temp dir");
        let first = lock_data_dir(dir.path()).expect("the first lock");
        assert!(matches!(
            lock_data_dir(dir.path()),
            Err(ServeError::DataDirInUse)
        ));
        drop(first);
        assert!(lock_data_dir(dir.path()).is_ok(), "free once released");
    }
}
