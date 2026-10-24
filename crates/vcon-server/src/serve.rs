//! `vcon-server serve`: the server, run from its configuration.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;

use crate::api::{self, App, Limits, TlsError};
use crate::clock::SystemClock;
use crate::config::Config;
use crate::envelope::{Key, KeyError};
use crate::fs::OsFs;
use crate::retention::Retention;
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
    let tenants = Arc::new(tenants);
    // The purge sweep: at start, then purge_interval after each sweep ends,
    // until the server stops.
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (swept, default) = (tenants.clone(), config.retention_default);
    let sweeper = tokio::spawn(every(
        config.purge_interval,
        async {
            let _ = stopped.await;
        },
        move || {
            let tenants = swept.clone();
            async move {
                let _ = tokio::task::spawn_blocking(move || sweep(&tenants, default)).await;
            }
        },
    ));
    let app = app_from(config, tenants);
    tracing::info!(listen = %config.listen, "serving");
    let served = api::serve(listener, tls, api::router(Arc::new(app)), shutdown).await;
    let _ = stop.send(());
    let _ = sweeper.await;
    served.map_err(ServeError::Serve)?;
    tracing::info!("stopped");
    Ok(())
}

/// What one purge sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    /// Versions purged, over every tenant.
    pub purged: u64,
    /// Tenants that could not be swept; they wait for the next sweep.
    pub failed: u64,
}

/// One purge sweep: each tenant purges what is due under its own retention
/// period, else `default`. A tenant that cannot be swept, such as one whose
/// period does not read back or that is being removed, is counted and left
/// for the next sweep. The sweep is logged with counts only, never a tenant.
pub fn sweep(tenants: &Tenants, default: Retention) -> SweepReport {
    let mut report = SweepReport::default();
    match tenants.ids() {
        Ok(ids) => {
            for id in ids {
                let purged = tenants.retention(id).and_then(|own| {
                    let store = tenants.store(id)?;
                    Ok(store.purge_due(own.unwrap_or(default)))
                });
                match purged {
                    Ok(Ok(done)) => report.purged += done.purged,
                    _ => report.failed += 1,
                }
            }
        }
        Err(_) => report.failed += 1,
    }
    tracing::info!(
        purged = report.purged,
        failed = report.failed,
        "purge sweep"
    );
    report
}

/// Runs `job` at once, then again `interval` after each run ends, until
/// `shutdown` completes; a run still going when it does is not awaited.
pub async fn every<F, Fut>(interval: Duration, shutdown: impl Future<Output = ()>, mut job: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => return,
            () = job() => {}
        }
        tokio::select! {
            biased;
            () = &mut shutdown => return,
            () = tokio::time::sleep(interval) => {}
        }
    }
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

    mod sweeps {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        use time::macros::datetime;
        use uuid::Uuid;

        use super::super::*;
        use crate::audit::Actor;
        use crate::ids::{KeyId, TenantId};
        use crate::retention::Retention;
        use crate::store::Caller;
        use crate::store::testing::vcon;
        use crate::tenants::fixture;

        const CALLER: Caller = Caller { key: KeyId([7; 8]) };

        /// A vCon stored and deleted in `id` while seven years apply, so
        /// it waits hidden for a sweep.
        fn deleted(tenants: &Tenants, id: TenantId, uuid: &str) -> Uuid {
            let store = tenants.store(id).expect("its store");
            let (found, body) = vcon(uuid, "kept");
            store.create(&CALLER, &found, &body).expect("stored");
            store
                .delete(&CALLER, found.uuid, Retention::Years(7))
                .expect("deleted");
            found.uuid
        }

        /// How many versions `id` still keeps.
        fn kept(tenants: &Tenants, id: TenantId) -> u64 {
            tenants
                .store(id)
                .expect("its store")
                .counts()
                .expect("counts")
                .versions
        }

        #[test]
        fn a_sweep_purges_what_is_due_in_each_tenant_under_its_own_period() {
            let f = fixture();
            let own = f
                .tenants
                .create(Actor::Cli, "own", Some(Retention::Days(1)))
                .expect("made");
            let default = f.tenants.create(Actor::Cli, "default", None).expect("made");
            let long = f
                .tenants
                .create(Actor::Cli, "long", Some(Retention::Years(7)))
                .expect("made");
            deleted(&f.tenants, own, "0199e5c6-0d7a-7c11-8000-000000000001");
            deleted(&f.tenants, default, "0199e5c6-0d7a-7c11-8000-000000000002");
            deleted(&f.tenants, long, "0199e5c6-0d7a-7c11-8000-000000000003");
            f.clock.set(datetime!(2026-10-25 13:00 UTC));

            let report = sweep(&f.tenants, Retention::Days(1));

            assert_eq!(
                report,
                SweepReport {
                    purged: 2,
                    failed: 0
                }
            );
            assert_eq!(kept(&f.tenants, own), 0);
            assert_eq!(kept(&f.tenants, default), 0);
            assert_eq!(kept(&f.tenants, long), 1, "seven years have not passed");
        }

        #[test]
        fn a_sweep_goes_on_past_a_tenant_it_cannot_sweep() {
            let f = fixture();
            let damaged = f.tenants.create(Actor::Cli, "damaged", None).expect("made");
            let fine = f.tenants.create(Actor::Cli, "fine", None).expect("made");
            deleted(&f.tenants, damaged, "0199e5c6-0d7a-7c11-8000-000000000001");
            deleted(&f.tenants, fine, "0199e5c6-0d7a-7c11-8000-000000000002");
            f.tenants
                .conn_for_test()
                .execute(
                    "UPDATE tenant SET retention = '7w' WHERE id = ?1",
                    [damaged.0],
                )
                .expect("damaged");
            f.clock.set(datetime!(2026-10-25 13:00 UTC));

            let report = sweep(&f.tenants, Retention::Days(1));

            assert_eq!(
                report,
                SweepReport {
                    purged: 1,
                    failed: 1
                }
            );
            assert_eq!(kept(&f.tenants, fine), 0);
            assert_eq!(kept(&f.tenants, damaged), 1, "left for a later sweep");
        }

        #[tokio::test(start_paused = true)]
        async fn every_runs_at_start_then_after_each_interval_until_shutdown() {
            let runs = Arc::new(AtomicUsize::new(0));
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            let counted = runs.clone();
            let task = tokio::spawn(every(
                Duration::from_secs(60),
                async move {
                    let _ = stopped.await;
                },
                move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    async {}
                },
            ));
            let settle = || async {
                for _ in 0..10 {
                    tokio::task::yield_now().await;
                }
            };

            settle().await;
            assert_eq!(runs.load(Ordering::SeqCst), 1, "at start");
            tokio::time::advance(Duration::from_secs(59)).await;
            settle().await;
            assert_eq!(runs.load(Ordering::SeqCst), 1, "not before the interval");
            tokio::time::advance(Duration::from_secs(1)).await;
            settle().await;
            assert_eq!(runs.load(Ordering::SeqCst), 2, "after the interval");

            stop.send(()).expect("still running");
            task.await.expect("ends on shutdown");
            tokio::time::advance(Duration::from_secs(600)).await;
            assert_eq!(runs.load(Ordering::SeqCst), 2, "none after shutdown");
        }
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
