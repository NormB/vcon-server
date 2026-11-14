//! `vcon-server serve`: the server, run from its configuration.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, io};

use tokio::net::TcpListener;

use crate::access::Plane;
use crate::api::{self, App, Limits, TlsError};
use crate::backend;
use crate::clock::{Clock, SystemClock};
use crate::config::{Config, Operator, ParquetSettings, StorageError};
use crate::datadir::{self, LockError, Work};
use crate::envelope::{Key, KeyError};
use crate::fs::OsFs;
use crate::retention::Retention;
use crate::tenants::{KeyOrder, RegistryError, RotationBacklog, TenantError, Tenants, with_causes};
use time::OffsetDateTime;

/// Why the server did not start, or stopped.
#[derive(Debug)]
pub enum ServeError {
    /// The data directory could not be taken.
    DataDir(LockError),
    /// The master key file could not be read.
    MasterKey(KeyError),
    /// server.sqlite was made under another master key.
    WrongMasterKey,
    /// The registry in the catalog database was made under another master
    /// key.
    WrongDatabaseKey,
    /// The registry could not be opened.
    Open(RegistryError),
    /// server.sqlite could not be read.
    Registry(TenantError),
    /// The storage the configuration names cannot be used.
    Storage(StorageError),
    /// Some tenant's data is not under its current key.
    KeysOutOfOrder(KeyOrder),
    /// `tenants` tenants have packed versions, and the configuration has no
    /// `[storage.parquet]`.
    ParquetRemoved {
        /// How many tenants have packed versions.
        tenants: u64,
    },
    /// The certificate or key could not be used.
    Tls(TlsError),
    /// The listening address could not be bound.
    Listen(io::Error),
    /// The operator listener's certificate or key could not be used.
    OperatorTls(TlsError),
    /// The operator listener's address could not be bound.
    OperatorListen(io::Error),
    /// Serving failed.
    Serve(io::Error),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeError::DataDir(e) => write!(f, "{e}"),
            ServeError::MasterKey(KeyError::Refused(why)) => {
                write!(f, "the master key cannot be read: {why}")
            }
            ServeError::MasterKey(e) => write!(f, "the master key cannot be read: {e:?}"),
            ServeError::WrongMasterKey => {
                f.write_str("the master key is not the one server.sqlite was made under")
            }
            ServeError::WrongDatabaseKey => f.write_str(crate::tenants::WRONG_DATABASE_KEY),
            ServeError::Open(e) => {
                write!(f, "the registry cannot be opened: {}", with_causes(e))
            }
            ServeError::Registry(e) => write!(f, "server.sqlite cannot be opened: {e:?}"),
            ServeError::KeysOutOfOrder(order) => {
                let mut parts = Vec::new();
                if order.wrapped_elsewhere > 0 {
                    parts.push(format!(
                        "{} tenant key(s) do not unwrap under the master key",
                        order.wrapped_elsewhere
                    ));
                }
                if order.rotations_stopped > 0 {
                    parts.push(format!(
                        "{} key rotation(s) stopped part way; run rekey tenant again",
                        order.rotations_stopped
                    ));
                }
                if order.data_under_other_keys > 0 {
                    parts.push(format!(
                        "{} tenant(s) with data under another key",
                        order.data_under_other_keys
                    ));
                }
                if order.catalogs_unopened > 0 {
                    let why = order
                        .unopened_error
                        .as_ref()
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default();
                    parts.push(format!(
                        "{} tenant catalog(s) cannot be opened{why}",
                        order.catalogs_unopened
                    ));
                }
                write!(
                    f,
                    "{}; vcon-server check names the tenants",
                    parts.join("; ")
                )
            }
            ServeError::ParquetRemoved { tenants } => write!(
                f,
                "{tenants} tenant(s) have versions packed into Parquet files, and the \
                 configuration has no [storage.parquet]: without it the server packs no more, \
                 and the start no longer refuses a bucket with Object Lock or versioning, either \
                 of which keeps the bytes of a pack file that a purge rewrote; put the \
                 [storage.parquet] table back, since packed versions cannot be turned back into \
                 objects; vcon-server check names the tenants"
            ),
            ServeError::Tls(crate::api::TlsError::KeyFile(why)) => {
                write!(f, "the TLS certificate or key cannot be used: {why}")
            }
            ServeError::Tls(e) => write!(f, "the TLS certificate or key cannot be used: {e:?}"),
            ServeError::Listen(e) => write!(f, "the listening address cannot be bound: {e}"),
            ServeError::OperatorTls(crate::api::TlsError::KeyFile(why)) => write!(
                f,
                "the operator listener's TLS certificate or key cannot be used: {why}"
            ),
            ServeError::OperatorTls(e) => write!(
                f,
                "the operator listener's TLS certificate or key cannot be used: {e:?}"
            ),
            ServeError::OperatorListen(e) => {
                write!(f, "the operator listening address cannot be bound: {e}")
            }
            ServeError::Serve(e) => write!(f, "serving failed: {e}"),
            ServeError::Storage(e) => write!(f, "the storage cannot be used: {e}"),
        }
    }
}

/// Runs the server until `shutdown` completes: locks the data directory,
/// opens the registry under the master key, and serves the API over TLS.
///
/// The opening runs on a blocking task: the registry and the catalogs are
/// read there, and a PostgreSQL catalog waits on a runtime of its own,
/// which no async worker may do. A server that stops, or whose start is
/// refused after the catalogs were reached, gives its locks on the
/// database up before it ends.
pub async fn run(config: &Config, shutdown: impl Future<Output = ()>) -> Result<(), ServeError> {
    let opening = config.clone();
    let opened = blocking(move || open(&opening)).await?;
    run_opened(config, opened, shutdown).await
}

/// [`run`], with the data directory locked and the registry opened: serves
/// until `shutdown` completes or a listener fails, and then gives up the
/// locks on the catalog database, while its connections still answer, so
/// the next server finds the database free at once.
pub(crate) async fn run_opened(
    config: &Config,
    (_lock, catalogs, tenants): Opened,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ServeError> {
    if let Some(server) = catalogs.server() {
        tracing::info!(catalog = %server, "the catalogs are in PostgreSQL");
    }
    let served = serve_opened(config, tenants, shutdown).await;
    blocking(move || catalogs.give_up_locks()).await;
    served?;
    tracing::info!("stopped");
    Ok(())
}

/// Runs `work`, which reads files or waits on the catalog database, off
/// the async workers.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(work).await {
        Ok(done) => done,
        Err(e) => std::panic::resume_unwind(e.into_panic()),
    }
}

/// What a start opens before it listens: the data directory's lock, where
/// the catalogs are, and the registry, checked.
pub(crate) type Opened = (datadir::DataDirLock, backend::Catalogs, Tenants);

/// Locks the data directory, reads the master key, reaches the catalogs
/// and then the objects, and opens the registry. A start refused after the
/// catalogs were reached gives its locks on the database up.
fn open(config: &Config) -> Result<Opened, ServeError> {
    let lock =
        datadir::lock(&config.data_dir, Work::Serve.sharing()).map_err(ServeError::DataDir)?;
    let master = Key::load_master(&config.master_key).map_err(ServeError::MasterKey)?;
    let catalogs = backend::catalog_for(&config.catalog, Work::Serve.sharing())
        .map_err(ServeError::Storage)?;
    let tenants = open_or_give_up(config, master, &catalogs)?;
    Ok((lock, catalogs, tenants))
}

/// [`open_on`], and for a start it refuses, the locks on the catalog
/// database given up while the connections still answer.
pub(crate) fn open_or_give_up(
    config: &Config,
    master: Key,
    catalogs: &backend::Catalogs,
) -> Result<Tenants, ServeError> {
    open_on(config, master, catalogs).inspect_err(|_| catalogs.give_up_locks())
}

/// The registry on `catalogs`, with the objects bound to them, and the
/// checks a start makes of it before it listens.
fn open_on(
    config: &Config,
    master: Key,
    catalogs: &backend::Catalogs,
) -> Result<Tenants, ServeError> {
    let objects = backend::objects_for(&config.storage, &config.data_dir, catalogs)
        .map_err(ServeError::Storage)?;
    let tenants = Tenants::open_in(
        &config.data_dir,
        master,
        Arc::new(OsFs),
        Arc::new(SystemClock),
        objects,
        catalogs,
    )
    .map_err(|e| match e {
        RegistryError::WrongKey => ServeError::WrongMasterKey,
        RegistryError::WrongDatabaseKey => ServeError::WrongDatabaseKey,
        other => ServeError::Open(other),
    })?;
    let order = tenants.key_order().map_err(ServeError::Registry)?;
    if !order.is_sound() {
        return Err(ServeError::KeysOutOfOrder(order));
    }
    // Packed versions need Parquet storage kept on: nothing turns them
    // back into objects, and only with it is the bucket checked for what
    // would keep a purged row's bytes.
    if config.storage.parquet.is_none() {
        let tenants = tenants
            .tenants_with_packed_versions()
            .map_err(ServeError::Registry)?;
        if tenants > 0 {
            return Err(ServeError::ParquetRemoved { tenants });
        }
    }
    Ok(tenants)
}

/// Serves the opened registry until `shutdown` completes or a listener
/// fails.
async fn serve_opened(
    config: &Config,
    tenants: Tenants,
    shutdown: impl Future<Output = ()>,
) -> Result<(), ServeError> {
    let tls = api::tls_config(&config.tls_cert, &config.tls_key).map_err(ServeError::Tls)?;
    let listener = TcpListener::bind(config.listen)
        .await
        .map_err(ServeError::Listen)?;
    // The address bound, which names the port the system chose when the
    // configuration gives port 0.
    let bound = listener.local_addr().map_err(ServeError::Listen)?;
    // The operator listener, when the configuration has one: bound before
    // anything is served, so a start that cannot have both serves neither.
    let operator = match &config.operator {
        Some(operator) => {
            let tls = api::tls_config_named(
                &operator.tls_cert,
                &operator.tls_key,
                operator_key_name(config, operator),
            )
            .map_err(ServeError::OperatorTls)?;
            let listener = TcpListener::bind(operator.listen)
                .await
                .map_err(ServeError::OperatorListen)?;
            let bound = listener.local_addr().map_err(ServeError::OperatorListen)?;
            Some((listener, tls, bound))
        }
        None => None,
    };
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
    // The pack run, with Parquet storage: at start, then pack_interval
    // after each run ends, until the server stops.
    let (stop_packing, packing_stopped) = tokio::sync::oneshot::channel::<()>();
    let packer = config.storage.parquet.map(|parquet| {
        let packed = tenants.clone();
        tokio::spawn(every(
            parquet.pack_interval,
            async {
                let _ = packing_stopped.await;
            },
            move || {
                let tenants = packed.clone();
                async move {
                    let _ = tokio::task::spawn_blocking(move || pack_all(&tenants, &parquet)).await;
                }
            },
        ))
    });
    // The rotation warning: at start, then each day.
    let (stop_warning, warning_stopped) = tokio::sync::oneshot::channel::<()>();
    let (watched, rotation_warn) = (tenants.clone(), config.rotation_warn);
    let warner = tokio::spawn(every(
        ROTATION_CHECK,
        async {
            let _ = warning_stopped.await;
        },
        move || {
            let tenants = watched.clone();
            async move {
                let _ = tokio::task::spawn_blocking(move || {
                    warn_if_rotation_overdue(&tenants, rotation_warn);
                })
                .await;
            }
        },
    ));
    // With an operator listener each listener serves one kind of key;
    // without one, the one listener serves both.
    let (halt, halted) = tokio::sync::watch::channel(());
    let until_halted = |mut halted: tokio::sync::watch::Receiver<()>| async move {
        // An error is the sender dropped: halted as well.
        let _ = halted.changed().await;
    };
    // Each time a listener finds every place held, that is counted in the
    // admin log under the listener's configuration key. The count is
    // written off the listener's task, which goes on to wait for a place.
    let counted = tenants.clone();
    let counts_full = move |key: &'static str| {
        let tenants = counted.clone();
        move || {
            let tenants = tenants.clone();
            tokio::task::spawn_blocking(move || tenants.count_listener_full(key));
        }
    };
    let both = async {
        let Some((operator_listener, operator_tls, operator_bound)) = operator else {
            let full = counts_full("listen");
            let app = app_from(config, tenants, Plane::Both);
            tracing::info!(listen = %bound, "serving");
            return api::serve_observed(
                listener,
                tls,
                api::router(Arc::new(app)),
                until_halted(halted),
                config.connection,
                full,
            )
            .await;
        };
        let (tenant_full, operator_full) = (counts_full("listen"), counts_full("operator.listen"));
        let tenant_app = app_from(config, tenants.clone(), Plane::Tenant);
        let operator_app = app_from(config, tenants, Plane::Operator);
        // Before the serving line: whoever waits for that line finds both
        // listeners bound.
        tracing::info!(operator_listen = %operator_bound, "the operator listener is bound");
        tracing::info!(listen = %bound, "serving");
        let (tenant, operator) = tokio::join!(
            api::serve_observed(
                listener,
                tls,
                api::router(Arc::new(tenant_app)),
                until_halted(halted.clone()),
                config.connection,
                tenant_full,
            ),
            api::serve_observed(
                operator_listener,
                operator_tls,
                api::router(Arc::new(operator_app)),
                until_halted(halted),
                config.connection,
                operator_full,
            ),
        );
        tenant.and(operator)
    };
    let mut both = std::pin::pin!(both);
    let served = tokio::select! {
        served = &mut both => served,
        () = shutdown => {
            let _ = halt.send(());
            both.await
        }
    };
    let _ = stop.send(());
    let _ = stop_warning.send(());
    let _ = stop_packing.send(());
    let _ = sweeper.await;
    let _ = warner.await;
    if let Some(packer) = packer {
        let _ = packer.await;
    }
    served.map_err(ServeError::Serve)
}

/// What one pack run over every tenant did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PackRunReport {
    /// Pack files written and recorded, over every tenant.
    pub files: u64,
    /// Versions moved into them.
    pub versions: u64,
    /// Tenants whose run failed; they wait for the next run.
    pub failed: u64,
}

/// One pack run over every tenant, as `Store::pack` does for one, with the
/// configured limit and reader grace. A tenant whose run fails is
/// counted and left for the next run. Logged with counts only, never a
/// tenant.
pub fn pack_all(tenants: &Tenants, settings: &ParquetSettings) -> PackRunReport {
    let mut report = PackRunReport::default();
    let plan = crate::store::packer::PackPlan {
        grace: settings.reader_grace,
        ..crate::store::packer::PackPlan::with_limit(settings.pack_limit)
    };
    match tenants.ids() {
        Ok(ids) => {
            for id in ids {
                match tenants.store(id).map(|store| store.pack(&plan)) {
                    Ok(Ok(done)) => {
                        report.files += done.files;
                        report.versions += done.versions;
                    }
                    _ => report.failed += 1,
                }
            }
        }
        Err(_) => report.failed += 1,
    }
    tracing::info!(
        files = report.files,
        versions = report.versions,
        failed = report.failed,
        "pack run"
    );
    report
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
                    Ok(store.purge_due(crate::tenants::kept_for(own, default)))
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

/// The configuration key that names the operator listener's key file:
/// `operator.tls.key`, or `tls.key` when the `[operator]` table names no
/// pair of its own and the listener takes the server's.
fn operator_key_name(config: &Config, operator: &Operator) -> &'static str {
    if operator.tls_key == config.tls_key && operator.tls_cert == config.tls_cert {
        "tls.key"
    } else {
        "operator.tls.key"
    }
}

/// What the API serves from on a listener of `plane`, as the
/// configuration sets it.
fn app_from(config: &Config, tenants: Arc<Tenants>, plane: Plane) -> App {
    tenants.allow_no_expiry(config.keys_no_expiry);
    App {
        tenants,
        limits: Limits {
            body: config.body_limit,
            inflated: config.inflated_limit,
            body_timeout: config.read_timeout,
        },
        supported: config.supported_critical.clone(),
        retention_default: config.retention_default,
        mcp: config.mcp,
        plane,
        reports: crate::api::ReportGate::new(config.reports),
        origin: origin_for(config, plane),
    }
}

/// The public origin of the listener of `plane`: `operator.public_origin`
/// for the operator listener, `public_origin` for the tenant listener and
/// for the one listener of a server with no operator listener.
fn origin_for(config: &Config, plane: Plane) -> Option<crate::session::Origin> {
    match plane {
        Plane::Both | Plane::Tenant => config.public_origin.clone(),
        Plane::Operator => config
            .operator
            .as_ref()
            .and_then(|operator| operator.public_origin.clone()),
    }
}

/// Whether key rotation is overdue: the oldest purge or removal waiting
/// for it is older than `rotation_warn`.
pub fn rotation_overdue(
    backlog: &RotationBacklog,
    rotation_warn: Duration,
    now: OffsetDateTime,
) -> bool {
    [backlog.oldest_purge, backlog.oldest_removal]
        .into_iter()
        .flatten()
        .any(|at| now - at > rotation_warn)
}

/// How often `serve` looks at the rotation backlog.
const ROTATION_CHECK: Duration = Duration::from_secs(86_400);

/// Logs a warning, with counts only, when key rotation is overdue.
fn warn_if_rotation_overdue(tenants: &Tenants, rotation_warn: Duration) {
    match tenants.rotation_backlog() {
        Ok(backlog) if rotation_overdue(&backlog, rotation_warn, SystemClock.now()) => {
            tracing::warn!(
                tenants_with_purges = backlog.tenants_with_purges,
                purges = backlog.purges,
                removals = backlog.removals,
                "key rotation overdue: purged vCons and removed tenants stay readable on old copies until it"
            );
        }
        Ok(_) => {}
        Err(_) => tracing::warn!("the key rotation backlog could not be read"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    mod startup {
        use time::macros::datetime;

        use super::*;

        /// The refusal names the table that was taken out, says what goes
        /// wrong without it, and where the tenants are named.
        #[test]
        fn a_start_without_storage_parquet_over_packed_versions_names_the_table() {
            assert_eq!(
                ServeError::ParquetRemoved { tenants: 2 }.to_string(),
                "2 tenant(s) have versions packed into Parquet files, and the configuration has no \
                 [storage.parquet]: without it the server packs no more, and the start no longer \
                 refuses a bucket with Object Lock or versioning, either of which keeps the bytes of \
                 a pack file that a purge rewrote; put the [storage.parquet] table back, since packed \
                 versions cannot be turned back into objects; vcon-server check names the tenants",
                "the refusal's text"
            );
        }

        const NOW: OffsetDateTime = datetime!(2026-10-25 12:00 UTC);
        const DAY: Duration = Duration::from_secs(86_400);

        #[test]
        fn rotation_is_overdue_once_the_oldest_waiting_work_passes_rotation_warn() {
            let backlog =
                |purge: Option<OffsetDateTime>, removal: Option<OffsetDateTime>| RotationBacklog {
                    tenants_with_purges: u64::from(purge.is_some()),
                    purges: u64::from(purge.is_some()),
                    oldest_purge: purge,
                    removals: u64::from(removal.is_some()),
                    oldest_removal: removal,
                };
            let day_ago = Some(datetime!(2026-10-24 12:00 UTC));
            let older = Some(datetime!(2026-10-24 11:59:59 UTC));
            for (b, overdue) in [
                (backlog(None, None), false),
                (backlog(day_ago, None), false),
                (backlog(older, None), true),
                (backlog(None, older), true),
                (backlog(day_ago, older), true),
            ] {
                assert_eq!(rotation_overdue(&b, DAY, NOW), overdue, "{b:?}");
            }
        }

        #[test]
        fn a_refusal_for_keys_out_of_order_counts_and_points_at_check() {
            let order = KeyOrder {
                wrapped_elsewhere: 1,
                rotations_stopped: 2,
                data_under_other_keys: 3,
                ..KeyOrder::default()
            };
            assert_eq!(
                ServeError::KeysOutOfOrder(order).to_string(),
                "1 tenant key(s) do not unwrap under the master key; \
                 2 key rotation(s) stopped part way; run rekey tenant again; \
                 3 tenant(s) with data under another key; \
                 vcon-server check names the tenants",
                "each count in its fixed place, then where to look"
            );
        }

        /// Catalogs the server may not open are counted apart from keys,
        /// with the system's error, and never a tenant or a path.
        #[test]
        fn a_refusal_for_catalogs_it_may_not_open_gives_the_error_not_a_key() {
            let order = KeyOrder {
                catalogs_unopened: 2,
                unopened_error: Some("Permission denied (os error 13)".to_owned()),
                ..KeyOrder::default()
            };
            assert_eq!(
                ServeError::KeysOutOfOrder(order).to_string(),
                "2 tenant catalog(s) cannot be opened: Permission denied (os error 13); vcon-server check names the tenants"
            );
        }
    }
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
            connection: crate::api::ConnectionLimits::default(),
            supported_critical: ["known".to_owned()].into(),
            mcp: crate::mcp::Limits {
                calls_per_second: 3,
                concurrent: 4,
                response_bytes: 5,
            },
            reports: crate::api::ReportLimits::DEFAULT,
            storage: crate::config::Storage::default(),
            export: crate::config::Export::default(),
            catalog: crate::config::CatalogStore::Local,
            operator: None,
            public_origin: None,
            keys_no_expiry: false,
        };
        assert!(
            tenants.no_expiry_allowed(),
            "before the configuration is applied a key may have no expiry"
        );
        let app = app_from(&config, tenants.clone(), Plane::Tenant);
        assert!(
            !tenants.no_expiry_allowed(),
            "keys.no_expiry = false reaches the registry the API makes keys in"
        );
        assert_eq!(app.plane, Plane::Tenant, "the plane it was given");
        assert!(Arc::ptr_eq(&app.tenants, &tenants));
        assert_eq!(app.limits.body, 12_345);
        assert_eq!(app.limits.inflated, 67_890);
        assert_eq!(app.limits.body_timeout, Duration::from_secs(17));
        assert_eq!(app.supported, config.supported_critical);
        assert_eq!(app.retention_default, config.retention_default);
        assert_eq!(app.mcp, config.mcp);
        assert_eq!(app.origin, None, "no public origin is configured");
    }

    /// A configuration with these lines before and after the least one.
    fn configured(first: &str, last: &str) -> Config {
        let text = format!(
            "{first}listen = \"127.0.0.1:8443\"\ndata_dir = \"/var/lib/vcon-server\"\n\
             master_key = \"/etc/vcon-server/master.key\"\n[tls]\ncert = \"/c.pem\"\n\
             key = \"/k.pem\"\n{last}"
        );
        crate::config::parse(&text).expect("a configuration")
    }

    const OPERATOR: &str =
        "[operator]\nlisten = \"127.0.0.1:8444\"\npublic_origin = \"https://ops.example.com\"\n";

    #[test]
    fn each_listener_takes_its_own_public_origin() {
        let config = configured("public_origin = \"https://vcon.example.com\"\n", OPERATOR);
        let origin = |plane| origin_for(&config, plane).map(|o| o.as_str().to_owned());
        assert_eq!(
            origin(Plane::Tenant).as_deref(),
            Some("https://vcon.example.com"),
            "`listen` takes public_origin"
        );
        assert_eq!(
            origin(Plane::Operator).as_deref(),
            Some("https://ops.example.com"),
            "`operator.listen` takes operator.public_origin"
        );
    }

    #[test]
    fn the_one_listener_of_a_server_takes_public_origin_for_both_planes() {
        let config = configured("public_origin = \"https://vcon.example.com\"\n", "");
        assert_eq!(
            origin_for(&config, Plane::Both),
            crate::session::Origin::parse("https://vcon.example.com"),
            "tenants' and operators' sessions come from the one origin"
        );
    }

    #[test]
    fn a_listener_with_no_public_origin_has_none_and_does_not_take_the_other_s() {
        let config = configured("", OPERATOR);
        assert_eq!(
            origin_for(&config, Plane::Tenant),
            None,
            "not the operator's"
        );
        let config = configured("public_origin = \"https://vcon.example.com\"\n", "");
        assert_eq!(
            origin_for(&config, Plane::Operator),
            None,
            "not the tenant's"
        );
        let config = configured("", "");
        assert_eq!(origin_for(&config, Plane::Both), None, "none is configured");
    }

    mod sweeps {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        use time::macros::datetime;
        use uuid::Uuid;

        use crate::audit::Actor;
        use crate::ids::{KeyId, TenantId};
        use crate::retention::Retention;
        use crate::serve::{SweepReport, every, sweep};
        use crate::store::Caller;
        use crate::store::testing::vcon;
        use crate::tenants::{Tenants, fixture};

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
}
