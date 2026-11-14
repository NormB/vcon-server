//! A tenant's catalog in PostgreSQL: [`Catalog`] and [`CatalogTx`] as
//! statements on the tables `vcon-server migrate` makes.
//!
//! - One [`Database`] is the process's pool, on a runtime of its own, and
//!   the lock every server shares. A call waits on that runtime
//!   ([`Database::wait`]), so it runs on a thread that is not a runtime's
//!   worker: a blocking task, a command, or a test.
//! - A transaction is one pooled connection. Its first round trip begins
//!   it, sets `vcon.tenant` for the row-level security policies, and, for
//!   a write, locks the tenant's row of `vcon.tenant_lock`, which gives
//!   the tenant's writes one order, as `BEGIN IMMEDIATE` gives a local
//!   catalog's. That round trip is the one of the transaction's first
//!   statement: the begin is sent ahead of it on the same connection, so a
//!   tenant's writes queue at their first statement, and a transaction
//!   that sends no statement sends nothing. The tenant is set here and
//!   nowhere else; every statement names it too, and the policy is the
//!   second check.
//! - A statement is sent with its parameter types (`query_typed`), in one
//!   round trip and with no prepared statement, so the server plans each
//!   with the tenant set and keeps no plan.
//! - What names a vCon is sealed or keyed ([`crate::envelope::column`]):
//!   the uuid is found by its HMAC and read from its sealed copy; a
//!   version's form, gzip flag and size, and an audit record's target, are
//!   sealed. The times, the holds, the counts and the random ids are
//!   clear.
//! - A read runs inside a savepoint, so a read that fails leaves the
//!   transaction usable, which `Store` relies on when it records a failed
//!   read. A write that fails aborts the transaction: every later call
//!   fails, and its commit answers [`CommitError::RolledBack`].

use std::cell::Cell;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

use time::OffsetDateTime;
use tokio_postgres::error::{DbError, Severity};
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Row};
use uuid::Uuid;

use super::connect::{Connector, Purpose, with_causes};
use super::pool::{Lease, Pool, must_drop};
use super::schema;
use super::secrets::Secrets;
use crate::audit::{self, Action, AuditError, Entry};
use crate::backend::pack::Place;
use crate::backend::{
    Catalog, CatalogTx, CommitError, ExaminedRow, ExportedRow, HiddenRow, KeptRow, ListedRow,
    NewPackFile, NewVersion, PackFileRow, Packed, PageQuery, UnpackedRow, VconRow, VersionRow,
    WrappedRow,
};
use crate::config::{PostgresSettings, StorageError};
use crate::envelope::{self as column, ColumnContext, ColumnKeys, Key, REGISTRY};
use crate::ids::{FileId, KeyId, TenantId, VersionId};
use crate::store::StoreError;

/// An audit record's target as its two columns hold it, the HMAC and the
/// sealed copy; both absent for a record with no target.
type SealedTarget = (Option<Vec<u8>>, Option<Vec<u8>>);

/// A statement's parameter with its PostgreSQL type.
type Param<'a> = (&'a (dyn ToSql + Sync), Type);

/// The process's connections to the catalog database: the pool, the
/// runtime its connections run on, and the lock a server holds.
pub(crate) struct Database {
    // Dropped first: the lease and the pool close their connections while
    // the runtime that drives them is still there.
    lock: std::sync::Mutex<Option<Lease>>,
    pool: Pool,
    runtime: OwnRuntime,
    /// The server the lock's connection reached, as a log line names it.
    server: String,
    /// In tests, how many times a call waited on the database: the round
    /// trips, since each wait sends one batch and reads its answer.
    #[cfg(test)]
    trips: std::sync::atomic::AtomicUsize,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

/// The database's runtime, ended without waiting when it is dropped. A
/// `tokio::runtime::Runtime` dropped where a runtime is entered panics,
/// and the last holder of a database may end in serve's async body: on a
/// start refused after the catalogs were opened, and when a server stops.
struct OwnRuntime(Option<tokio::runtime::Runtime>);

impl std::ops::Deref for OwnRuntime {
    type Target = tokio::runtime::Runtime;

    fn deref(&self) -> &tokio::runtime::Runtime {
        self.0
            .as_ref()
            .expect("the runtime is there until the drop")
    }
}

impl Drop for OwnRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background();
        }
    }
}

/// The fewest connections a server's pool may have: the one that holds
/// the servers' lock, and one for a transaction.
pub(crate) const SMALLEST_POOL: u32 = 2;

impl Database {
    /// In tests, `connect_as` for a server.
    #[cfg(all(test, feature = "backend-tests"))]
    pub(crate) fn connect(settings: &PostgresSettings) -> Result<Arc<Database>, StorageError> {
        Database::connect_as(settings, schema::Holds::Serve)
    }

    /// Reads the secret files, connects with the application role, and
    /// takes the locks of the work `holds`, on a connection kept for that
    /// alone: a server, a command beside one, or a command alone, each
    /// with the locks of its kind.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming the step that refused: a secret file, the
    /// pool's size, the server reached, or the lock, whose refusal says
    /// what may hold the database against this work.
    pub(crate) fn connect_as(
        settings: &PostgresSettings,
        holds: schema::Holds,
    ) -> Result<Arc<Database>, StorageError> {
        if settings.pool < SMALLEST_POOL {
            return Err(StorageError(pool_refusal(settings.pool)));
        }
        let secrets = Secrets::read(settings)?;
        let connector = Connector::new(settings, secrets, Purpose::Serve)?;
        let runtime = OwnRuntime(Some(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("vcon-postgres")
                .enable_all()
                .build()
                .map_err(|e| StorageError(format!("the runtime cannot start: {e}")))?,
        ));
        let pool = Pool::new(connector, settings.pool, settings.connect_timeout);
        let lock = runtime
            .block_on(pool.get())
            .map_err(|e| StorageError(format!("catalog.postgres: {e}")))?;
        runtime
            .block_on(schema::take_locks_for(lock.client(), holds))
            .map_err(|why| StorageError(format!("catalog.postgres: {why}")))?;
        let server = lock.server().to_string();
        Ok(Arc::new(Database {
            lock: std::sync::Mutex::new(Some(lock)),
            pool,
            runtime,
            server,
            #[cfg(test)]
            trips: std::sync::atomic::AtomicUsize::new(0),
        }))
    }

    /// The server reached and the role logged in as, such as `PostgreSQL
    /// 18.6 at db1.example.com:5432 as vcon_app over TLS`.
    pub(crate) fn server(&self) -> &str {
        &self.server
    }

    /// Waits for `call` on the pool's runtime. The calling thread must not
    /// be a runtime's worker.
    fn wait<T>(&self, call: impl Future<Output = T>) -> T {
        #[cfg(test)]
        self.trips.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.runtime.handle().block_on(call)
    }

    /// Gives up the servers' lock, as a server that stops does, while the
    /// connection still answers: the next server finds it free at once. A
    /// lock left to the end of the session, as a process that dies leaves
    /// it, is held until PostgreSQL notices the closed connection. A
    /// second call does nothing. The connection that held the lock goes
    /// back to the pool.
    pub(crate) fn give_up_lock(&self) {
        let held = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(mut lock) = held {
            let unlocked = self.runtime.block_on(
                lock.client()
                    .batch_execute("SELECT pg_advisory_unlock_all()"),
            );
            if unlocked.is_err() {
                // No answer: the lock goes when the server closes the
                // session, and the connection is not used again.
                lock.discard();
            }
        }
    }

    /// A connection from the pool. Taking one that waits unused sends
    /// nothing, so it is not a round trip.
    pub(super) fn lease(&self) -> Result<Lease, StoreError> {
        self.runtime.handle().block_on(self.pool.get()).map_err(io)
    }

    /// In tests, the round trips so far.
    #[cfg(all(test, feature = "backend-tests"))]
    pub(crate) fn round_trips(&self) -> usize {
        self.trips.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// In tests, how many of the pool's connections wait unused.
    #[cfg(all(test, feature = "backend-tests"))]
    pub(crate) fn idle(&self) -> usize {
        self.pool.idle()
    }
}

/// Why a pool of `size` connections cannot serve.
pub(crate) fn pool_refusal(size: u32) -> String {
    format!(
        "catalog.postgres.pool is {size}: a server needs at least {SMALLEST_POOL} connections, \
         one for the lock every server holds and one for a transaction"
    )
}

/// Why a tenant's catalog could not be made or opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CatalogError {
    /// The catalog's columns are sealed under another key.
    WrongKey,
    /// The database has no catalog for the tenant.
    NoTenant,
    /// The database already has a catalog for the tenant.
    Exists,
    /// The database could not be reached or read.
    Io,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CatalogError::WrongKey => "the tenant's catalog is under another key",
            CatalogError::NoTenant => "the database has no catalog for the tenant",
            CatalogError::Exists => "the database already has a catalog for the tenant",
            CatalogError::Io => "the database could not be read",
        })
    }
}

impl std::error::Error for CatalogError {}

/// How a transaction begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Begin {
    /// A write: read committed, with the tenant's lock row held.
    Write,
    /// A read of one consistent state: repeatable read, read only.
    Read,
    /// The write that makes the tenant's lock row, which it cannot hold
    /// yet.
    Create,
}

/// The one batch that begins a transaction of `tenant` as `how`: the
/// transaction, the tenant the policies read, and for a write the lock on
/// the tenant's row. A tenant with no such row was removed, and the batch
/// then ends in an error: it is sent in one round trip with the
/// transaction's first statement ([`PgTx::trip`]), and after an error the
/// server refuses that statement and every one after it until the
/// rollback. The tenant's id is 32 hex digits, so it is written into the
/// text as it is.
pub(crate) fn begin_text(how: Begin, tenant: TenantId) -> String {
    let set = format!("SET LOCAL vcon.tenant = '{tenant}'");
    match how {
        Begin::Write => format!(
            "BEGIN ISOLATION LEVEL READ COMMITTED; {set}; \
             DO $$BEGIN \
             PERFORM 1 FROM vcon.tenant_lock WHERE tenant = '\\x{tenant}' FOR UPDATE; \
             IF NOT FOUND THEN RAISE EXCEPTION 'tenant {tenant} has no lock row'; END IF; \
             END$$"
        ),
        Begin::Read => format!("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; {set}"),
        Begin::Create => format!("BEGIN ISOLATION LEVEL READ COMMITTED; {set}"),
    }
}

/// The advisory lock a write of the registry holds to its end, a constant
/// all instances share: the registry has one writer at a time, as the
/// local registry's one connection gives it.
pub(crate) const REGISTRY_LOCK: i64 = 0x7663_6f6e_5247_0001;

/// The one batch that begins a transaction of the registry as `how`: the
/// transaction, the marker the registry's policies read in place of a
/// tenant, and for a write the registry's lock, which the transaction's
/// end gives up.
pub(crate) fn registry_begin_text(how: Begin) -> String {
    let set = "SET LOCAL vcon.tenant = 'registry'";
    match how {
        Begin::Write | Begin::Create => format!(
            "BEGIN ISOLATION LEVEL READ COMMITTED; {set}; \
             SELECT pg_advisory_xact_lock({REGISTRY_LOCK})"
        ),
        Begin::Read => format!("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; {set}"),
    }
}

/// The audit table of a transaction: a tenant's log, or the registry's.
struct AuditTable {
    /// The table, with its schema.
    sql: &'static str,
    /// The table's name in a sealed target's context.
    sealed_as: &'static str,
    /// Whether rows carry a tenant; the registry's log has no such column.
    by_tenant: bool,
}

/// A commit fault a test set: the commits to go before it, and how it
/// fails.
#[cfg(test)]
pub(super) type Fault = Mutex<Option<(usize, CommitError)>>;

/// What a COMMIT that failed means, from the severity of the server's
/// answer, or `None` for no answer. Only an answer of ERROR says the
/// transaction was rolled back: the server refused the COMMIT and goes on
/// with the session. FATAL and PANIC end the session, and such a message
/// may have been sent before the COMMIT arrived, so it is no answer to it;
/// with no answer the transaction may or may not have been kept. Unknown
/// is the answer that loses nothing: `Store` keeps the object, and
/// recovery decides by the row.
pub(crate) fn commit_failure(severity: Option<Severity>) -> CommitError {
    match severity {
        Some(Severity::Error) => CommitError::RolledBack,
        _ => CommitError::Unknown,
    }
}

/// A version's gzip flag as the byte that is sealed.
pub(crate) fn gzip_byte(gzip: bool) -> [u8; 1] {
    [u8::from(gzip)]
}

/// The gzip flag a sealed byte holds; any other bytes are damage.
pub(crate) fn gzip_of(bytes: &[u8]) -> Result<bool, StoreError> {
    match bytes {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(StoreError::Damaged),
    }
}

/// A version's size as the eight bytes that are sealed.
pub(crate) fn size_bytes(size: i64) -> [u8; 8] {
    size.to_be_bytes()
}

/// The size eight sealed bytes hold; any other length is damage.
pub(crate) fn size_of(bytes: &[u8]) -> Result<i64, StoreError> {
    bytes
        .try_into()
        .map(i64::from_be_bytes)
        .map_err(|_| StoreError::Damaged)
}

/// The sixteen bytes of an id column; any other length is damage.
pub(crate) fn id_of(bytes: &[u8]) -> Result<[u8; 16], StoreError> {
    bytes.try_into().map_err(|_| StoreError::Damaged)
}

/// A pack place from the five columns a row holds for it: all absent for
/// an object of its own, all present for a row of a pack file, and
/// anything else is damage.
pub(crate) fn packed(
    file: Option<Vec<u8>>,
    group: Option<i64>,
    row: Option<i64>,
    date: Option<String>,
    bytes: Option<i64>,
) -> Result<Option<Packed>, StoreError> {
    match (file, group, row, date, bytes) {
        (None, None, None, _, _) => Ok(None),
        (Some(file), Some(group), Some(row), Some(date), Some(bytes)) => {
            let damaged = |_| StoreError::Damaged;
            Ok(Some(Packed {
                file: FileId(id_of(&file)?),
                date,
                bytes: u64::try_from(bytes).map_err(damaged)?,
                place: Place {
                    group: u32::try_from(group).map_err(damaged)?,
                    row: u32::try_from(row).map_err(damaged)?,
                },
            }))
        }
        _ => Err(StoreError::Damaged),
    }
}

fn io<E>(_: E) -> StoreError {
    StoreError::Io
}

fn audit_failed(e: &tokio_postgres::Error) -> AuditError {
    AuditError::Database(with_causes(e))
}

/// The place columns of a version `v` and the join that reads them.
const PLACE: &str = "v.place_file, v.place_group, v.place_row, f.date, f.bytes";
const PLACE_JOIN: &str =
    "LEFT JOIN vcon.parquet_file f ON f.tenant = v.tenant AND f.file_id = v.place_file";

/// The join from a version `v` to its vCon `c`.
const VCON_JOIN: &str = "JOIN vcon.vcon c ON c.tenant = v.tenant AND c.uuid_mac = v.uuid_mac";

/// The tables a purge deletes from, which [`Catalog::after_purge`]
/// vacuums.
pub(crate) const PURGED_TABLES: &str = "vcon.version, vcon.vcon, vcon.tombstone";

/// A key of a tenant's audit log, opened: the targets of the log's
/// records and their MACs are made with its keys, not with the catalog
/// key's, so that a move of the catalog to a new key changes no record
/// (schema version 11).
pub(super) struct AuditKey {
    /// The id the records it sealed name: the key's own.
    id: Vec<u8>,
    keys: ColumnKeys,
}

/// The table that holds the audit keys of `tenant`'s log: a tenant's, or
/// with [`REGISTRY`] the registry's (schema version 12).
fn audit_key_table(tenant: TenantId) -> &'static str {
    if tenant == REGISTRY {
        "admin_audit_key"
    } else {
        "audit_key"
    }
}

/// Where the audit key `id` of `tenant`, or of the registry, is sealed.
fn audit_key_context(tenant: TenantId, id: &[u8]) -> ColumnContext<'_> {
    ColumnContext {
        tenant,
        table: audit_key_table(tenant),
        column: "wrapped",
        row: id,
    }
}

/// A new audit key for the log of `tenant`, or of the registry: the key
/// opened, and its copy sealed with `keys` for the log's table of keys.
///
/// # Errors
///
/// [`StoreError::Io`] when the key cannot be made or sealed.
pub(super) fn new_audit_key(
    tenant: TenantId,
    keys: &ColumnKeys,
) -> Result<(AuditKey, Vec<u8>), StoreError> {
    let key = Key::generate().map_err(io)?;
    let id = key.id().0.to_vec();
    let wrapped = column::wrap_key(keys, &audit_key_context(tenant, &id), &key).map_err(io)?;
    let opened = AuditKey {
        keys: ColumnKeys::derive(&key).map_err(io)?,
        id,
    };
    Ok((opened, wrapped))
}

/// The id of an opened audit key.
pub(super) fn audit_key_id(key: &AuditKey) -> &[u8] {
    &key.id
}

/// The audit keys in `rows` (`key_id`, `wrapped`, oldest first) of the
/// log of `tenant`, or of the registry, each opened with `keys`. An older
/// key that does not open is left out, and the records that name it
/// cannot be read.
///
/// # Errors
///
/// [`StoreError::Damaged`] when the newest does not open: it seals the
/// new records. [`StoreError::Io`] when a row cannot be read.
pub(super) fn open_audit_keys(
    tenant: TenantId,
    keys: &ColumnKeys,
    rows: &[Row],
) -> Result<Vec<AuditKey>, StoreError> {
    let mut opened = Vec::with_capacity(rows.len());
    for (at, row) in rows.iter().enumerate() {
        let id: Vec<u8> = row.try_get(0).map_err(io)?;
        let wrapped: Vec<u8> = row.try_get(1).map_err(io)?;
        match column::unwrap_key(keys, &audit_key_context(tenant, &id), &wrapped) {
            Ok(key) => opened.push(AuditKey {
                keys: ColumnKeys::derive(&key).map_err(io)?,
                id,
            }),
            Err(_) if at + 1 < rows.len() => {}
            Err(_) => return Err(StoreError::Damaged),
        }
    }
    Ok(opened)
}

/// A tenant's catalog in the database.
pub(crate) struct PgCatalog {
    database: Arc<Database>,
    tenant: TenantId,
    keys: ColumnKeys,
    /// The keys of the tenant's audit log that opened, oldest first: the
    /// last seals the new records.
    audit: Vec<AuditKey>,
    #[cfg(test)]
    fault: Mutex<Option<(usize, CommitError)>>,
}

impl PgCatalog {
    fn with(
        database: Arc<Database>,
        tenant: TenantId,
        catalog_key: &Key,
    ) -> Result<PgCatalog, CatalogError> {
        if tenant == REGISTRY {
            // The id of all zeros is the registry's in a sealed column's
            // context and in a transaction's scope: no tenant has it.
            return Err(CatalogError::NoTenant);
        }
        Ok(PgCatalog {
            database,
            tenant,
            keys: ColumnKeys::derive(catalog_key).map_err(|_| CatalogError::Io)?,
            audit: Vec::new(),
            #[cfg(test)]
            fault: Mutex::new(None),
        })
    }

    /// Makes the catalog of a new tenant, sealed under `catalog_key`: the
    /// row its writes lock, the id of the key, and the key of its audit
    /// log, random and sealed under the catalog's keys.
    ///
    /// # Errors
    ///
    /// [`CatalogError::Exists`] when the tenant has one, [`CatalogError::Io`]
    /// when the database cannot be written.
    pub(crate) fn create(
        database: Arc<Database>,
        tenant: TenantId,
        catalog_key: &Key,
    ) -> Result<PgCatalog, CatalogError> {
        let mut catalog = PgCatalog::with(database, tenant, catalog_key)?;
        let (audit, wrapped) =
            new_audit_key(tenant, &catalog.keys).map_err(|_| CatalogError::Io)?;
        let tx = catalog.begin(Begin::Create).map_err(|_| CatalogError::Io)?;
        let made = tx
            .execute(
                "INSERT INTO vcon.tenant_lock (tenant) VALUES ($1) ON CONFLICT DO NOTHING",
                &[(&tenant.0.as_slice(), Type::BYTEA)],
            )
            .map_err(|_| CatalogError::Io)?;
        if made == 0 {
            return Err(CatalogError::Exists);
        }
        tx.execute(
            "INSERT INTO vcon.tenant_catalog (tenant, catalog_key_id) VALUES ($1, $2)",
            &[
                (&tenant.0.as_slice(), Type::BYTEA),
                (&catalog_key.id().0.as_slice(), Type::BYTEA),
            ],
        )
        .map_err(|_| CatalogError::Io)?;
        tx.execute(
            // After any key the tenant's id had before: a removal keeps
            // the keys of the log with its records.
            "INSERT INTO vcon.audit_key (tenant, key_id, ordinal, wrapped)
             SELECT $1, $2, coalesce(max(ordinal), 0) + 1, $3
             FROM vcon.audit_key WHERE tenant = $1",
            &[
                (&tenant.0.as_slice(), Type::BYTEA),
                (&audit.id, Type::BYTEA),
                (&wrapped, Type::BYTEA),
            ],
        )
        .map_err(|_| CatalogError::Io)?;
        Box::new(tx).commit().map_err(|_| CatalogError::Io)?;
        catalog.audit.push(audit);
        Ok(catalog)
    }

    /// The keys of the tenant's audit log, oldest first, each opened with
    /// the catalog's keys. An older key that does not open is left out,
    /// and the records that name it cannot be read; the newest must open,
    /// since it seals the new records.
    fn audit_keys(&self) -> Result<Vec<AuditKey>, CatalogError> {
        let tx = self.begin(Begin::Read).map_err(|_| CatalogError::Io)?;
        let rows = tx
            .query(
                "SELECT key_id, wrapped FROM vcon.audit_key WHERE tenant = $1 ORDER BY ordinal",
                &[(&self.tenant.0.as_slice(), Type::BYTEA)],
            )
            .map_err(|_| CatalogError::Io)?;
        let keys = open_audit_keys(self.tenant, &self.keys, &rows).map_err(|_| CatalogError::Io)?;
        if keys.is_empty() {
            // A log with no key takes no record.
            return Err(CatalogError::Io);
        }
        Ok(keys)
    }

    /// Whether the database has a catalog for `tenant`.
    ///
    /// # Errors
    ///
    /// [`CatalogError::Io`] when the database cannot be read.
    #[cfg(all(test, feature = "backend-tests"))]
    pub(crate) fn exists(database: &Arc<Database>, tenant: TenantId) -> Result<bool, CatalogError> {
        Ok(PgCatalog::key_id(database, tenant)?.is_some())
    }

    /// The id of the key `tenant`'s catalog is sealed under, if it has a
    /// catalog.
    fn key_id(database: &Arc<Database>, tenant: TenantId) -> Result<Option<Vec<u8>>, CatalogError> {
        if tenant == REGISTRY {
            // The registry's id is no tenant's, and a transaction of it
            // reads no tenant's rows.
            return Ok(None);
        }
        let lease = database.lease().map_err(|_| CatalogError::Io)?;
        let tx = PgTx::begin(database, lease, tenant, None, Begin::Read);
        let rows = tx
            .query(
                "SELECT catalog_key_id FROM vcon.tenant_catalog WHERE tenant = $1",
                &[(&tenant.0.as_slice(), Type::BYTEA)],
            )
            .map_err(|_| CatalogError::Io)?;
        rows.first()
            .map(|row| row.try_get::<_, Vec<u8>>(0).map_err(|_| CatalogError::Io))
            .transpose()
    }

    /// Opens the catalog of `tenant` under `catalog_key`.
    ///
    /// # Errors
    ///
    /// [`CatalogError::NoTenant`] when the database has none,
    /// [`CatalogError::WrongKey`] when it is sealed under another key, and
    /// [`CatalogError::Io`] when the database cannot be read, or the key
    /// of the tenant's audit log is missing or does not open.
    pub(crate) fn open(
        database: Arc<Database>,
        tenant: TenantId,
        catalog_key: &Key,
    ) -> Result<PgCatalog, CatalogError> {
        let held = PgCatalog::key_id(&database, tenant)?.ok_or(CatalogError::NoTenant)?;
        if held != catalog_key.id().0 {
            return Err(CatalogError::WrongKey);
        }
        let mut catalog = PgCatalog::with(database, tenant, catalog_key)?;
        catalog.audit = catalog.audit_keys()?;
        Ok(catalog)
    }

    /// Every tenant that has a lock row, by id.
    ///
    /// # Errors
    ///
    /// [`CatalogError::Io`] when the database cannot be read.
    pub(crate) fn present(database: &Arc<Database>) -> Result<Vec<TenantId>, CatalogError> {
        let lease = database.lease().map_err(|_| CatalogError::Io)?;
        // A registry transaction reads every tenant's lock row (schema
        // version 10); a tenant's reads its own.
        let tx = PgTx::begin(database, lease, REGISTRY, None, Begin::Read);
        tx.query("SELECT tenant FROM vcon.tenant_lock ORDER BY tenant", &[])
            .map_err(|_| CatalogError::Io)?
            .iter()
            .map(|row| {
                let bytes: Vec<u8> = row.try_get(0).map_err(|_| CatalogError::Io)?;
                id_of(&bytes).map(TenantId).map_err(|_| CatalogError::Io)
            })
            .collect()
    }

    /// A transaction that removes rows of `tenant`: it holds the tenant's
    /// lock row to its end, as a write does, when there is one. A removal
    /// that is made again finds none and still deletes what is left.
    fn begin_removal(database: &Arc<Database>, tenant: TenantId) -> Result<PgTx<'_>, CatalogError> {
        let lease = database.lease().map_err(|_| CatalogError::Io)?;
        let tx = PgTx::begin(database, lease, tenant, None, Begin::Create);
        tx.query(
            "SELECT 1 FROM vcon.tenant_lock WHERE tenant = $1 FOR UPDATE",
            &[(&tenant.0.as_slice(), Type::BYTEA)],
        )
        .map_err(|_| CatalogError::Io)?;
        Ok(tx)
    }

    /// Removes every row of `tenant` but its audit records, at most
    /// `batch` rows of a table in one transaction.
    ///
    /// # Errors
    ///
    /// [`CatalogError::NoTenant`] for the registry's id, and
    /// [`CatalogError::Io`] when the database cannot be written or `batch`
    /// is not positive.
    pub(crate) fn remove_in(
        database: &Arc<Database>,
        tenant: TenantId,
        batch: i64,
    ) -> Result<(), CatalogError> {
        if tenant == REGISTRY {
            return Err(CatalogError::NoTenant);
        }
        let most = u64::try_from(batch).map_err(|_| CatalogError::Io)?;
        if most == 0 {
            return Err(CatalogError::Io);
        }
        let id = tenant.0.as_slice();
        // The tables that grow with the vCons stored, a batch in each
        // transaction, so that no one transaction holds every row.
        for (table, key) in [
            ("version", "version"),
            ("vcon", "uuid_mac"),
            ("tombstone", "uuid_mac"),
        ] {
            let delete = format!(
                "DELETE FROM vcon.{table} WHERE tenant = $1 AND {key} IN \
                 (SELECT {key} FROM vcon.{table} WHERE tenant = $1 LIMIT $2)"
            );
            loop {
                let tx = PgCatalog::begin_removal(database, tenant)?;
                let deleted = tx
                    .execute(&delete, &[(&id, Type::BYTEA), (&batch, Type::INT8)])
                    .map_err(|_| CatalogError::Io)?;
                tx.finish().map_err(|_| CatalogError::Io)?;
                if deleted < most {
                    break;
                }
            }
        }
        // The rest in one transaction, the lock row last: until it is
        // gone the tenant is present, and a removal cut off is made again.
        // The audit records stay (`vcon.audit` refuses a DELETE).
        let tx = PgCatalog::begin_removal(database, tenant)?;
        for table in [
            "key_use",
            "parquet_file",
            "purge_pending",
            "manifest",
            "tenant_catalog",
            "tenant_lock",
        ] {
            tx.execute(
                &format!("DELETE FROM vcon.{table} WHERE tenant = $1"),
                &[(&id, Type::BYTEA)],
            )
            .map_err(|_| CatalogError::Io)?;
        }
        tx.finish().map_err(|_| CatalogError::Io)
    }

    /// Which of two catalog keys the catalog of `tenant` is under.
    ///
    /// # Errors
    ///
    /// [`CatalogError::NoTenant`] when the database has no catalog for the
    /// tenant, [`CatalogError::WrongKey`] when it is under neither key,
    /// and [`CatalogError::Io`] when the database cannot be read.
    pub(crate) fn under(
        database: &Arc<Database>,
        tenant: TenantId,
        old: &Key,
        new: &Key,
    ) -> Result<crate::backend::Under, CatalogError> {
        let held = PgCatalog::key_id(database, tenant)?.ok_or(CatalogError::NoTenant)?;
        if held == old.id().0 {
            Ok(crate::backend::Under::Old)
        } else if held == new.id().0 {
            Ok(crate::backend::Under::New)
        } else {
            Err(CatalogError::WrongKey)
        }
    }

    /// Moves the catalog of `tenant` from the catalog key `old` to `new`,
    /// in one transaction that holds the tenant's lock row: every sealed
    /// column and every MAC of the tenant's vCons, versions and tombstones
    /// is made again with the keys of `new`, each key of the tenant's
    /// audit log is sealed again, and the catalog then names `new`. It
    /// reads and writes at most `batch` rows of a table in one statement.
    ///
    /// No audit record is changed: a record's target is sealed under the
    /// audit key, which stays the same. A catalog that is under `new`
    /// already is left as it is, so a move that was made can be asked for
    /// again. Every server is stopped for it, as for the rotation it is a
    /// step of: an open catalog holds the keys it was opened with.
    ///
    /// # Errors
    ///
    /// [`CatalogError::NoTenant`] when the database has no catalog for
    /// the tenant, [`CatalogError::WrongKey`] when it is under neither
    /// key, and [`CatalogError::Io`] when a row cannot be read under
    /// `old`, the database cannot be written, or `batch` is not positive.
    /// After an error nothing is changed, unless the commit got no
    /// answer: [`PgCatalog::under`] then says which key the catalog is
    /// under.
    pub(crate) fn reseal(
        database: &Arc<Database>,
        tenant: TenantId,
        old: &Key,
        new: &Key,
        batch: i64,
    ) -> Result<(), CatalogError> {
        if batch < 1 {
            return Err(CatalogError::Io);
        }
        let held = PgCatalog::key_id(database, tenant)?.ok_or(CatalogError::NoTenant)?;
        if held == new.id().0 {
            return Ok(());
        }
        if held != old.id().0 {
            return Err(CatalogError::WrongKey);
        }
        let from = ColumnKeys::derive(old).map_err(|_| CatalogError::Io)?;
        let to = ColumnKeys::derive(new).map_err(|_| CatalogError::Io)?;
        let lease = database.lease().map_err(|_| CatalogError::Io)?;
        let tx = PgTx::begin(database, lease, tenant, Some(&from), Begin::Write);
        let moved = reseal_audit_keys(&tx, &to)
            .and_then(|()| reseal_uuids(&tx, "vcon", &to, batch))
            .and_then(|()| reseal_uuids(&tx, "tombstone", &to, batch))
            .and_then(|()| reseal_versions(&tx, &to, batch))
            .and_then(|()| {
                // The id is compared again, now with the lock row held.
                let named = tx.execute(
                    "UPDATE vcon.tenant_catalog SET catalog_key_id = $2
                     WHERE tenant = $1 AND catalog_key_id = $3",
                    &[
                        (&tenant.0.as_slice(), Type::BYTEA),
                        (&new.id().0.as_slice(), Type::BYTEA),
                        (&old.id().0.as_slice(), Type::BYTEA),
                    ],
                )?;
                if named == 1 {
                    Ok(())
                } else {
                    Err(StoreError::Io)
                }
            });
        moved.map_err(|_| CatalogError::Io)?;
        tx.finish().map_err(|_| CatalogError::Io)
    }

    fn begin(&self, how: Begin) -> Result<PgTx<'_>, StoreError> {
        let lease = self.database.lease()?;
        let tx = PgTx::begin(&self.database, lease, self.tenant, Some(&self.keys), how)
            .with_audit(&self.audit);
        #[cfg(test)]
        let tx = tx.with_fault(&self.fault);
        Ok(tx)
    }
}

/// The number of rows a statement of a re-seal must have changed.
pub(super) fn all_changed(changed: u64, of: usize) -> Result<(), StoreError> {
    if u64::try_from(of).is_ok_and(|of| of == changed) {
        Ok(())
    } else {
        Err(StoreError::Io)
    }
}

/// The statements that read the keys of an audit log, oldest first, and
/// seal one again: a tenant's log, whose statements take the tenant as
/// their last parameter, or with [`REGISTRY`] the registry's, whose table
/// has no column for one.
pub(super) fn audit_key_statements(tenant: TenantId) -> (&'static str, &'static str) {
    if tenant == REGISTRY {
        (
            "SELECT key_id, wrapped FROM vcon.admin_audit_key ORDER BY ordinal",
            "UPDATE vcon.admin_audit_key SET wrapped = $2 WHERE key_id = $1",
        )
    } else {
        (
            "SELECT key_id, wrapped FROM vcon.audit_key WHERE tenant = $1 ORDER BY ordinal",
            "UPDATE vcon.audit_key SET wrapped = $2 WHERE key_id = $1 AND tenant = $3",
        )
    }
}

/// Seals each key of the audit log of the transaction's tenant, or of
/// the registry, again with the keys `to`. A key that does not open
/// under the transaction's keys is left as it is, as
/// [`PgCatalog::audit_keys`] leaves it out; the newest must open.
pub(super) fn reseal_audit_keys(tx: &PgTx<'_>, to: &ColumnKeys) -> Result<(), StoreError> {
    let tenant = tx.tenant;
    let id_of_tenant = tenant.0.as_slice();
    let (select, update) = audit_key_statements(tenant);
    // The registry's statements name no tenant.
    let whose: &[Param<'_>] = if tenant == REGISTRY {
        &[]
    } else {
        &[(&id_of_tenant, Type::BYTEA)]
    };
    let rows = tx.query(select, whose)?;
    if rows.is_empty() {
        // A catalog that would not open is not moved.
        return Err(StoreError::Io);
    }
    for (at, row) in rows.iter().enumerate() {
        let id: Vec<u8> = row.try_get(0).map_err(io)?;
        let wrapped: Vec<u8> = row.try_get(1).map_err(io)?;
        let ctx = audit_key_context(tenant, &id);
        match column::unwrap_key(tx.keys()?, &ctx, &wrapped) {
            Ok(key) => {
                let again = column::wrap_key(to, &ctx, &key).map_err(io)?;
                let mut params: Vec<Param<'_>> = vec![(&id, Type::BYTEA), (&again, Type::BYTEA)];
                params.extend_from_slice(whose);
                let changed = tx.execute(update, &params)?;
                all_changed(changed, 1)?;
            }
            Err(_) if at + 1 < rows.len() => {}
            Err(_) => return Err(StoreError::Damaged),
        }
    }
    Ok(())
}

/// Makes the MAC and the sealed copy of each uuid in `table`, the vCons
/// or the tombstones, again with the keys `to`, and gives each version
/// of such a uuid its new MAC. The rows are read through a cursor, which
/// shows the table as it was when the cursor was made: a row this
/// transaction has moved is not read a second time.
fn reseal_uuids(
    tx: &PgTx<'_>,
    table: &'static str,
    to: &ColumnKeys,
    batch: i64,
) -> Result<(), StoreError> {
    let tenant = tx.tenant;
    let id = tenant.0.as_slice();
    // The tenant's id is 32 hex digits, so it is written into the text
    // as it is; a DECLARE takes no parameter.
    tx.batch(&format!(
        "DECLARE reseal_rows NO SCROLL CURSOR FOR \
         SELECT uuid_mac, uuid_sealed FROM vcon.{table} WHERE tenant = '\\x{tenant}'"
    ))?;
    let fetch = format!("FETCH {batch} FROM reseal_rows");
    let move_rows = format!(
        "UPDATE vcon.{table} t SET uuid_mac = n.now, uuid_sealed = n.sealed
         FROM unnest($2::bytea[], $3::bytea[], $4::bytea[]) AS n(was, now, sealed)
         WHERE t.tenant = $1 AND t.uuid_mac = n.was"
    );
    loop {
        let rows = tx.query(&fetch, &[])?;
        if rows.is_empty() {
            break;
        }
        let mut was = Vec::with_capacity(rows.len());
        let mut now = Vec::with_capacity(rows.len());
        let mut sealed = Vec::with_capacity(rows.len());
        for row in &rows {
            let mac: Vec<u8> = row.try_get(0).map_err(io)?;
            let old: Vec<u8> = row.try_get(1).map_err(io)?;
            let uuid = tx.unseal(table, "uuid_sealed", &mac, &old)?;
            let new_mac = column::mac(to, &uuid).to_vec();
            let ctx = ColumnContext {
                tenant,
                table,
                column: "uuid_sealed",
                row: &new_mac,
            };
            sealed.push(column::seal_column(to, &ctx, &uuid).map_err(io)?);
            was.push(mac);
            now.push(new_mac);
        }
        let changed = tx.execute(
            &move_rows,
            &[
                (&id, Type::BYTEA),
                (&was, Type::BYTEA_ARRAY),
                (&now, Type::BYTEA_ARRAY),
                (&sealed, Type::BYTEA_ARRAY),
            ],
        )?;
        all_changed(changed, rows.len())?;
        // A uuid may have any number of versions, or none.
        tx.execute(
            "UPDATE vcon.version v SET uuid_mac = n.now
             FROM unnest($2::bytea[], $3::bytea[]) AS n(was, now)
             WHERE v.tenant = $1 AND v.uuid_mac = n.was",
            &[
                (&id, Type::BYTEA),
                (&was, Type::BYTEA_ARRAY),
                (&now, Type::BYTEA_ARRAY),
            ],
        )?;
    }
    tx.batch("CLOSE reseal_rows")
}

/// Seals the form, the gzip flag and the size of each version again with
/// the keys `to`, in the order of the versions' ids, which a re-seal does
/// not change. A version whose uuid neither a vCon nor a tombstone holds
/// keeps the MAC it had: no row says which uuid it was made from.
fn reseal_versions(tx: &PgTx<'_>, to: &ColumnKeys, batch: i64) -> Result<(), StoreError> {
    const COLUMNS: [&str; 3] = ["form_sealed", "gzip_sealed", "size_sealed"];
    let tenant = tx.tenant;
    let id = tenant.0.as_slice();
    let mut after: Vec<u8> = Vec::new();
    loop {
        let rows = tx.query(
            "SELECT version, form_sealed, gzip_sealed, size_sealed FROM vcon.version
             WHERE tenant = $1 AND version > $2 ORDER BY version LIMIT $3",
            &[
                (&id, Type::BYTEA),
                (&after, Type::BYTEA),
                (&batch, Type::INT8),
            ],
        )?;
        let Some(last) = rows.last() else {
            break;
        };
        after = last.try_get(0).map_err(io)?;
        let mut versions = Vec::with_capacity(rows.len());
        let mut sealed: [Vec<Vec<u8>>; 3] = [const { Vec::new() }; 3];
        for row in &rows {
            let version: Vec<u8> = row.try_get(0).map_err(io)?;
            for (at, column) in COLUMNS.into_iter().enumerate() {
                let old: Vec<u8> = row.try_get(at + 1).map_err(io)?;
                let plain = tx.unseal("version", column, &version, &old)?;
                let ctx = ColumnContext {
                    tenant,
                    table: "version",
                    column,
                    row: &version,
                };
                sealed[at].push(column::seal_column(to, &ctx, &plain).map_err(io)?);
            }
            versions.push(version);
        }
        let [form, gzip, size] = &sealed;
        let changed = tx.execute(
            "UPDATE vcon.version v
             SET form_sealed = n.form, gzip_sealed = n.gzip, size_sealed = n.size
             FROM unnest($2::bytea[], $3::bytea[], $4::bytea[], $5::bytea[])
                  AS n(version, form, gzip, size)
             WHERE v.tenant = $1 AND v.version = n.version",
            &[
                (&id, Type::BYTEA),
                (&versions, Type::BYTEA_ARRAY),
                (form, Type::BYTEA_ARRAY),
                (gzip, Type::BYTEA_ARRAY),
                (size, Type::BYTEA_ARRAY),
            ],
        )?;
        all_changed(changed, rows.len())?;
    }
    Ok(())
}

impl Catalog for PgCatalog {
    fn immediate(&self) -> Result<Box<dyn CatalogTx + '_>, StoreError> {
        Ok(Box::new(self.begin(Begin::Write)?))
    }

    fn snapshot(&self) -> Result<Box<dyn CatalogTx + '_>, StoreError> {
        Ok(Box::new(self.begin(Begin::Read)?))
    }

    /// Nothing: PostgreSQL recovers its own write-ahead log at start.
    fn recover(&self) -> Result<(), StoreError> {
        Ok(())
    }

    /// `VACUUM` of the tables a purge deletes from, so the bytes of the
    /// deleted rows leave the table files sooner. The server skips a
    /// table the role may not vacuum (before PostgreSQL 17 only the owner
    /// may) and says so in a warning, not an error; autovacuum then does
    /// the same work later.
    fn after_purge(&self) -> Result<(), StoreError> {
        let mut lease = self.database.lease()?;
        let done = self.database.wait(
            lease
                .client()
                .batch_execute(&format!("VACUUM {PURGED_TABLES}")),
        );
        done.map_err(|e| {
            lease.failed(&e);
            StoreError::Io
        })
    }

    /// Not for a PostgreSQL catalog: its backup is the database's own
    /// (`pg_basebackup`, `pg_dump`), which the how-to covers.
    fn backup_into(&self, _to: &Path) -> Result<Vec<(VersionId, Option<Packed>)>, StoreError> {
        Err(StoreError::Io)
    }

    /// Not for a PostgreSQL catalog, as [`Catalog::backup_into`].
    fn unpack_copy(&self, _copy: &Path) -> Result<(), StoreError> {
        Err(StoreError::Io)
    }

    fn export(&self, after: i64, out: &mut dyn Write) -> Result<(), AuditError> {
        let tx = self
            .begin(Begin::Read)
            .map_err(|_| AuditError::Database("the database could not be reached".to_owned()))?;
        tx.audit_lines(after, None, out)
    }

    #[cfg(test)]
    fn fail_commit(&self, nth: usize, how: CommitError) {
        *self.fault.lock().expect("fault") = Some((nth, how));
    }

    #[cfg(test)]
    fn on_checkpoint_wait(&self, _busy: std::sync::mpsc::Sender<()>) {
        // A PostgreSQL catalog has no checkpoint of its own to wait for.
    }

    #[cfg(test)]
    fn local_conn(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        panic!("a PostgreSQL catalog has no local connection")
    }
}

/// A transaction on a tenant's catalog, or on the registry: one pooled
/// connection, rolled back when dropped without a commit.
pub(super) struct PgTx<'a> {
    database: &'a Database,
    lease: Lease,
    /// Whose rows the transaction reads and writes: a tenant's, or with
    /// [`REGISTRY`] the registry's.
    tenant: TenantId,
    /// The keys that seal and open the columns; none for the read that
    /// finds a tenant's key id before there is a catalog.
    keys: Option<&'a ColumnKeys>,
    /// The keys of the tenant's audit log, oldest first; none for the
    /// registry, whose log is sealed with `keys`, and none for a
    /// transaction that writes and reads no audit record.
    audit: &'a [AuditKey],
    /// The commit fault of the catalog or registry the transaction is of.
    #[cfg(test)]
    fault: Option<&'a Fault>,
    /// How the transaction begins, until its first statement is sent:
    /// the begin goes with that statement, in its round trip.
    pending: Cell<Option<Begin>>,
    /// The begin was sent and no end yet: the drop rolls back.
    open: Cell<bool>,
    /// The begin failed: nothing of this transaction is kept, whatever
    /// is rolled back to after.
    refused: Cell<bool>,
    /// A write failed: the server refuses every statement but the end.
    aborted: Cell<bool>,
    /// A statement failed in a way that says the connection cannot be
    /// used again ([`must_drop`]): it is closed when the transaction ends.
    broken: Cell<bool>,
}

impl Drop for PgTx<'_> {
    fn drop(&mut self) {
        if self.open.get() {
            let rolled = self
                .database
                .wait(self.lease.client().batch_execute("ROLLBACK"));
            if rolled.is_err() {
                self.broken.set(true);
            }
        }
        if self.broken.get() {
            self.lease.discard();
        }
    }
}

impl<'a> PgTx<'a> {
    /// The transaction, with the commit fault a test set on its catalog
    /// or registry.
    #[cfg(test)]
    pub(super) fn with_fault(mut self, fault: &'a Fault) -> PgTx<'a> {
        self.fault = Some(fault);
        self
    }

    /// A transaction of `tenant`, or with [`REGISTRY`] of the registry, on
    /// the connection of `lease`. Nothing is sent yet: the begin goes with
    /// the first statement ([`PgTx::trip`]).
    pub(super) fn begin(
        database: &'a Database,
        lease: Lease,
        tenant: TenantId,
        keys: Option<&'a ColumnKeys>,
        how: Begin,
    ) -> PgTx<'a> {
        PgTx {
            database,
            lease,
            tenant,
            keys,
            audit: &[],
            #[cfg(test)]
            fault: None,
            pending: Cell::new(Some(how)),
            open: Cell::new(false),
            refused: Cell::new(false),
            aborted: Cell::new(false),
            broken: Cell::new(false),
        }
    }

    /// The transaction, with the keys of the audit log it writes: a
    /// tenant's, or the registry's.
    pub(super) fn with_audit(mut self, audit: &'a [AuditKey]) -> PgTx<'a> {
        self.audit = audit;
        self
    }

    fn client(&self) -> &Client {
        self.lease.client()
    }

    /// One round trip of the transaction: `call`, and before it in the
    /// same round trip the begin ([`begin_text`]), when `call` is the
    /// transaction's first. The two are written to the connection in this
    /// order before either answer is read, and the server runs them in
    /// that order. When the begin fails, its error is the answer: the
    /// server then refuses `call` and every later statement, the commit
    /// answers [`CommitError::RolledBack`], and the drop rolls back.
    ///
    /// A begin whose `BEGIN` itself did not run would leave `call` outside
    /// any transaction. The batch is fixed text that the server parses
    /// whole, so that is not expected; if it happened, no tenant would be
    /// set, and the policies refuse a row read or written with no tenant.
    fn trip<T>(
        &self,
        call: impl Future<Output = Result<T, tokio_postgres::Error>>,
    ) -> Result<T, tokio_postgres::Error> {
        let Some(how) = self.pending.take() else {
            return self.database.wait(call);
        };
        let text = if self.tenant == REGISTRY {
            registry_begin_text(how)
        } else {
            begin_text(how, self.tenant)
        };
        let client = self.lease.client();
        self.open.set(true);
        let (began, done) = self
            .database
            .wait(async { tokio::join!(biased; client.batch_execute(&text), call) });
        match began {
            Ok(()) => done,
            Err(e) => {
                self.refused.set(true);
                Err(e)
            }
        }
    }

    /// Notes a statement's failure: whether the connection may be kept.
    fn failed(&self, e: &tokio_postgres::Error) {
        if must_drop(e) {
            self.broken.set(true);
        }
    }

    pub(super) fn keys(&self) -> Result<&ColumnKeys, StoreError> {
        self.keys.ok_or(StoreError::Io)
    }

    /// The log this transaction writes and reads.
    fn audit_table(&self) -> AuditTable {
        if self.tenant == REGISTRY {
            AuditTable {
                sql: "vcon.admin_audit",
                sealed_as: "admin_audit",
                by_tenant: false,
            }
        } else {
            AuditTable {
                sql: "vcon.audit",
                sealed_as: "audit",
                by_tenant: true,
            }
        }
    }

    /// A read, inside a savepoint sent with it in one round trip: when it
    /// fails, the transaction is as it was before it.
    pub(super) fn query(&self, sql: &str, params: &[Param<'_>]) -> Result<Vec<Row>, StoreError> {
        self.query_checked(sql, params).map_err(io)
    }

    fn query_checked(
        &self,
        sql: &str,
        params: &[Param<'_>],
    ) -> Result<Vec<Row>, tokio_postgres::Error> {
        let client = self.lease.client();
        let outcome = self.trip(async {
            let (saved, rows, released) = tokio::join!(
                biased;
                client.batch_execute("SAVEPOINT read"),
                client.query_typed(sql, params),
                client.batch_execute("RELEASE SAVEPOINT read"),
            );
            match saved.and(rows) {
                Ok(rows) => released.map(|()| rows),
                Err(e) => {
                    // Back to the savepoint; if that fails too, the
                    // transaction stays aborted and its end rolls back.
                    let _ = client
                        .batch_execute("ROLLBACK TO SAVEPOINT read; RELEASE SAVEPOINT read")
                        .await;
                    Err(e)
                }
            }
        });
        if let Err(e) = &outcome {
            self.failed(e);
        }
        outcome
    }

    /// A write, and the number of rows it changed. A write that fails
    /// aborts the transaction.
    pub(super) fn execute(&self, sql: &str, params: &[Param<'_>]) -> Result<u64, StoreError> {
        self.execute_checked(sql, params).map_err(io)
    }

    fn execute_checked(
        &self,
        sql: &str,
        params: &[Param<'_>],
    ) -> Result<u64, tokio_postgres::Error> {
        let outcome = self.trip(self.lease.client().execute_typed(sql, params));
        if let Err(e) = &outcome {
            self.aborted.set(true);
            self.failed(e);
        }
        outcome
    }

    /// A write that returns rows, such as an INSERT with RETURNING: it
    /// runs as a write, with no savepoint.
    fn returning(&self, sql: &str, params: &[Param<'_>]) -> Result<Vec<Row>, StoreError> {
        let outcome = self.trip(self.lease.client().query_typed(sql, params));
        outcome.map_err(|e| {
            self.aborted.set(true);
            self.failed(&e);
            StoreError::Io
        })
    }

    pub(super) fn batch(&self, sql: &str) -> Result<(), StoreError> {
        let outcome = self.trip(self.client().batch_execute(sql));
        outcome.map_err(|e| {
            self.aborted.set(true);
            self.failed(&e);
            StoreError::Io
        })
    }

    pub(super) fn exists(&self, sql: &str, params: &[Param<'_>]) -> Result<bool, StoreError> {
        Ok(!self.query(sql, params)?.is_empty())
    }

    fn count_of(&self, sql: &str, params: &[Param<'_>]) -> Result<u64, StoreError> {
        let rows = self.query(sql, params)?;
        let n: i64 = rows.first().ok_or(StoreError::Io)?.try_get(0).map_err(io)?;
        u64::try_from(n).map_err(io)
    }

    /// The ids in the first column of what `sql` selects, in its order.
    fn ids(&self, sql: &str, params: &[Param<'_>]) -> Result<Vec<[u8; 16]>, StoreError> {
        self.query(sql, params)?
            .iter()
            .map(|row| id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?))
            .collect()
    }

    pub(super) fn seal(
        &self,
        table: &str,
        column: &str,
        row: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, StoreError> {
        let ctx = ColumnContext {
            tenant: self.tenant,
            table,
            column,
            row,
        };
        column::seal_column(self.keys()?, &ctx, plaintext).map_err(io)
    }

    pub(super) fn unseal(
        &self,
        table: &str,
        column: &str,
        row: &[u8],
        sealed: &[u8],
    ) -> Result<Vec<u8>, StoreError> {
        let ctx = ColumnContext {
            tenant: self.tenant,
            table,
            column,
            row,
        };
        column::open_column(self.keys()?, &ctx, sealed).map_err(|_| StoreError::Damaged)
    }

    /// The HMAC a uuid is found by.
    fn uuid_mac(&self, uuid: Uuid) -> Result<[u8; 32], StoreError> {
        Ok(column::mac(self.keys()?, uuid.as_bytes()))
    }

    /// The uuid a row of `table` holds sealed, bound to the HMAC that is
    /// the row's key.
    fn uuid_of(&self, table: &str, mac: &[u8], sealed: &[u8]) -> Result<Uuid, StoreError> {
        let bytes = self.unseal(table, "uuid_sealed", mac, sealed)?;
        Ok(Uuid::from_bytes(id_of(&bytes)?))
    }

    /// The uuid in the columns `sealed` and `mac` of `row`, a vCon's.
    fn row_uuid(&self, row: &Row, sealed: usize, mac: usize) -> Result<Uuid, StoreError> {
        let sealed: Vec<u8> = row.try_get(sealed).map_err(io)?;
        let mac: Vec<u8> = row.try_get(mac).map_err(io)?;
        self.uuid_of("vcon", &mac, &sealed)
    }

    /// The pack place in the five columns of `row` from `at`.
    fn row_place(row: &Row, at: usize) -> Result<Option<Packed>, StoreError> {
        packed(
            row.try_get(at).map_err(io)?,
            row.try_get(at + 1).map_err(io)?,
            row.try_get(at + 2).map_err(io)?,
            row.try_get(at + 3).map_err(io)?,
            row.try_get(at + 4).map_err(io)?,
        )
    }

    /// The three sealed values of a version in `row` from `at`, opened:
    /// its form, its gzip flag and its size.
    fn row_sealed(
        &self,
        row: &Row,
        version: VersionId,
        at: usize,
    ) -> Result<(String, bool, i64), StoreError> {
        let get = |i: usize| row.try_get::<_, Vec<u8>>(i).map_err(io);
        let form = self.unseal("version", "form_sealed", &version.0, &get(at)?)?;
        let gzip = self.unseal("version", "gzip_sealed", &version.0, &get(at + 1)?)?;
        let size = self.unseal("version", "size_sealed", &version.0, &get(at + 2)?)?;
        Ok((
            String::from_utf8(form).map_err(|_| StoreError::Damaged)?,
            gzip_of(&gzip)?,
            size_of(&size)?,
        ))
    }

    /// The keys that seal the target of a new audit record, and the id
    /// of the audit key the record names: the newest audit key of the
    /// log, a tenant's or the registry's.
    fn audit_writer(&self) -> Result<(&ColumnKeys, &[u8]), StoreError> {
        let newest = self.audit.last().ok_or(StoreError::Io)?;
        Ok((&newest.keys, &newest.id))
    }

    /// The keys that open the target of a record that names the audit
    /// key `id`. A record that names no key, or a key that is not among
    /// the opened ones, cannot be read.
    fn audit_reader(&self, id: Option<&[u8]>) -> Result<&ColumnKeys, StoreError> {
        let id = id.ok_or(StoreError::Damaged)?;
        self.audit
            .iter()
            .find(|key| key.id == id)
            .map(|key| &key.keys)
            .ok_or(StoreError::Damaged)
    }

    /// The HMAC and the sealed copy of an audit record's target, made
    /// with `keys`.
    fn sealed_target(
        &self,
        keys: &ColumnKeys,
        target: Option<&str>,
    ) -> Result<SealedTarget, StoreError> {
        let Some(target) = target else {
            return Ok((None, None));
        };
        let mac = column::mac(keys, target.as_bytes());
        let ctx = ColumnContext {
            tenant: self.tenant,
            table: self.audit_table().sealed_as,
            column: "target_sealed",
            row: &mac,
        };
        let sealed = column::seal_column(keys, &ctx, target.as_bytes()).map_err(io)?;
        Ok((Some(mac.to_vec()), Some(sealed)))
    }

    /// Writes at most `limit` records after the seq `after` as JSON
    /// lines, oldest first, as the local log does (`audit::export`).
    pub(super) fn audit_lines(
        &self,
        after: i64,
        limit: Option<i64>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError> {
        let table = self.audit_table();
        let tenant = self.tenant.0.as_slice();
        // The tenant is the last parameter, which only a tenant's log has.
        let mut params: Vec<Param<'_>> = vec![(&after, Type::INT8), (&limit, Type::INT8)];
        let of_tenant = if table.by_tenant {
            params.push((&tenant, Type::BYTEA));
            "AND tenant = $3"
        } else {
            ""
        };
        let rows = self
            .query_checked(
                &format!(
                    // A record names the audit key that sealed its target.
                    "SELECT seq, at, actor, action, target_mac, target_sealed, version, status, \
                     count, key_id FROM {} WHERE seq > $1 {of_tenant} ORDER BY seq LIMIT $2",
                    table.sql
                ),
                &params,
            )
            .map_err(|e| audit_failed(&e))?;
        let damaged = || AuditError::Database("an audit record cannot be read".to_owned());
        for row in rows {
            let get = |e: tokio_postgres::Error| audit_failed(&e);
            let mac: Option<Vec<u8>> = row.try_get(4).map_err(get)?;
            let sealed: Option<Vec<u8>> = row.try_get(5).map_err(get)?;
            let target = match (mac, sealed) {
                (Some(mac), Some(sealed)) => {
                    let key_id: Option<Vec<u8>> = row.try_get(9).map_err(get)?;
                    let keys = self
                        .audit_reader(key_id.as_deref())
                        .map_err(|_| damaged())?;
                    let ctx = ColumnContext {
                        tenant: self.tenant,
                        table: table.sealed_as,
                        column: "target_sealed",
                        row: &mac,
                    };
                    let bytes = column::open_column(keys, &ctx, &sealed).map_err(|_| damaged())?;
                    Some(String::from_utf8(bytes).map_err(|_| damaged())?)
                }
                (None, None) => None,
                _ => return Err(damaged()),
            };
            let record = serde_json::json!({
                "seq": row.try_get::<_, i64>(0).map_err(get)?,
                "at": row.try_get::<_, String>(1).map_err(get)?,
                "actor": row.try_get::<_, String>(2).map_err(get)?,
                "action": row.try_get::<_, String>(3).map_err(get)?,
                "target": target,
                "version": row.try_get::<_, Option<String>>(6).map_err(get)?,
                "status": row.try_get::<_, Option<i64>>(7).map_err(get)?,
                "count": row.try_get::<_, i64>(8).map_err(get)?,
            });
            writeln!(out, "{record}").map_err(AuditError::Write)?;
        }
        Ok(())
    }

    /// Inserts one audit record at `at`, counting `count`; `counted`
    /// marks a record [`PgTx::count_record`] adds to.
    fn insert_audit(
        &self,
        entry: &Entry,
        at: &str,
        count: i64,
        counted: bool,
    ) -> Result<(), AuditError> {
        let unsealed = || AuditError::Database("an audit record cannot be sealed".to_owned());
        let (keys, key_id) = self.audit_writer().map_err(|_| unsealed())?;
        let (mac, sealed) = self
            .sealed_target(keys, entry.target.as_deref())
            .map_err(|_| unsealed())?;
        let actor = entry.actor.name();
        let version = entry.version.map(|v| v.to_string());
        let status = audit::stored(entry.status).map(i64::from);
        let action = entry.action.as_str();
        let table = self.audit_table();
        let tenant = self.tenant.0.as_slice();
        // The tenant is the last parameter, which only a tenant's log has.
        let mut params: Vec<Param<'_>> = vec![
            (&at, Type::TEXT),
            (&actor, Type::TEXT),
            (&action, Type::TEXT),
            (&mac, Type::BYTEA),
            (&sealed, Type::BYTEA),
            (&version, Type::TEXT),
            (&status, Type::INT8),
            (&count, Type::INT8),
            (&counted, Type::BOOL),
        ];
        params.push((&key_id, Type::BYTEA));
        let (column, value) = if table.by_tenant {
            params.push((&tenant, Type::BYTEA));
            (", key_id, tenant", ", $10, $11")
        } else {
            (", key_id", ", $10")
        };
        self.execute_checked(
            &format!(
                "INSERT INTO {} (at, actor, action, target_mac, target_sealed, version, status, \
                 count, counted{column}) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9{value})",
                table.sql
            ),
            &params,
        )
        .map(drop)
        .map_err(|e| audit_failed(&e))
    }

    /// Inserts one audit record at the time `now`.
    pub(super) fn record(&self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        self.insert_audit(entry, &audit::rfc3339(now), 1, false)
    }

    /// Adds `times` to the record for this actor, action, target, version
    /// and status in the minute of `now`, making it if there is none. Only
    /// a record this call made is added to (`counted`). A count of nothing
    /// makes no record.
    ///
    /// A new record is written with its whole count, and a record that is
    /// there is added to in one UPDATE: since schema version 9 an UPDATE
    /// of a log makes a count larger and changes nothing else
    /// (`vcon.audit_append_only`). A sum past the largest count the column
    /// holds is the server's error and adds nothing.
    pub(super) fn count_record(
        &self,
        entry: &Entry,
        now: OffsetDateTime,
        times: u64,
    ) -> Result<(), AuditError> {
        if times == 0 {
            return Ok(());
        }
        let times = i64::try_from(times).unwrap_or(i64::MAX);
        let unsealed = || AuditError::Database("an audit record cannot be sealed".to_owned());
        let at = audit::rfc3339(audit::minute_of(now));
        // The MAC of a tenant's record is made with its newest audit
        // key, so a record under an older key is not added to: a new one
        // is made, which names the newest.
        let (keys, _) = self.audit_writer().map_err(|_| unsealed())?;
        let mac = entry
            .target
            .as_deref()
            .map(|target| column::mac(keys, target.as_bytes()).to_vec());
        let actor = entry.actor.name();
        let version = entry.version.map(|v| v.to_string());
        let status = audit::stored(entry.status).map(i64::from);
        let action = entry.action.as_str();
        let table = self.audit_table();
        let tenant = self.tenant.0.as_slice();
        // The tenant is the last parameter, which only a tenant's log has.
        let mut params: Vec<Param<'_>> = vec![
            (&at, Type::TEXT),
            (&actor, Type::TEXT),
            (&action, Type::TEXT),
            (&mac, Type::BYTEA),
            (&version, Type::TEXT),
            (&status, Type::INT8),
            (&times, Type::INT8),
        ];
        let of_tenant = if table.by_tenant {
            params.push((&tenant, Type::BYTEA));
            "AND tenant = $8"
        } else {
            ""
        };
        let more = format!(
            "UPDATE {} SET count = count + $7
             WHERE counted AND at = $1 AND actor = $2 AND action = $3
               AND target_mac IS NOT DISTINCT FROM $4 AND version IS NOT DISTINCT FROM $5
               AND status IS NOT DISTINCT FROM $6 {of_tenant}",
            table.sql
        );
        let added = self
            .execute_checked(&more, &params)
            .map_err(|e| audit_failed(&e))?;
        if added == 0 {
            return self.insert_audit(entry, &at, times, true);
        }
        Ok(())
    }

    /// Ends the transaction, keeping what it wrote.
    pub(super) fn finish(&self) -> Result<(), CommitError> {
        #[cfg(test)]
        if let Some(how) = self.fault.and_then(injected) {
            let end = match how {
                CommitError::Unknown => "COMMIT",
                CommitError::RolledBack => "ROLLBACK",
            };
            if self.open.get() {
                let ended = self.database.wait(self.client().batch_execute(end));
                ended.expect("ends the transaction");
            }
            self.pending.set(None);
            self.open.set(false);
            return self.ended(Err(how));
        }
        if self.aborted.get() || self.refused.get() {
            // The drop sends the ROLLBACK the server is waiting for.
            return Err(CommitError::RolledBack);
        }
        if self.pending.take().is_some() {
            // No statement was sent, so no begin was: nothing to keep.
            return Ok(());
        }
        let outcome = self.database.wait(self.client().batch_execute("COMMIT"));
        self.open.set(false);
        let outcome =
            outcome.map_err(|e| commit_failure(e.as_db_error().and_then(DbError::parsed_severity)));
        self.ended(outcome)
    }
}

impl CatalogTx for PgTx<'_> {
    fn tombstoned(&mut self, uuid: Uuid) -> Result<bool, StoreError> {
        let mac = self.uuid_mac(uuid)?;
        self.exists(
            "SELECT 1 FROM vcon.tombstone WHERE tenant = $1 AND uuid_mac = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
            ],
        )
    }

    fn vcon_row(&mut self, uuid: Uuid) -> Result<Option<VconRow>, StoreError> {
        let mac = self.uuid_mac(uuid)?;
        let rows = self.query(
            "SELECT current, deleted_at FROM vcon.vcon WHERE tenant = $1 AND uuid_mac = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
            ],
        )?;
        rows.first()
            .map(|row| {
                let current: Option<Vec<u8>> = row.try_get(0).map_err(io)?;
                Ok(VconRow {
                    current: current
                        .map(|bytes| id_of(&bytes).map(VersionId))
                        .transpose()?,
                    deleted_at: row.try_get(1).map_err(io)?,
                })
            })
            .transpose()
    }

    fn version_row(&mut self, version: VersionId) -> Result<Option<VersionRow>, StoreError> {
        let rows = self.query(
            &format!(
                "SELECT v.form_sealed, v.gzip_sealed, v.size_sealed, v.wrapped_key, {PLACE}
                 FROM vcon.version v {PLACE_JOIN} WHERE v.tenant = $1 AND v.version = $2"
            ),
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
            ],
        )?;
        rows.first()
            .map(|row| {
                let (form, gzip, _size) = self.row_sealed(row, version, 0)?;
                Ok(VersionRow {
                    form,
                    gzip,
                    wrapped: row.try_get(3).map_err(io)?,
                    place: PgTx::row_place(row, 4)?,
                })
            })
            .transpose()
    }

    fn has_version(&mut self, version: VersionId, uuid: Uuid) -> Result<bool, StoreError> {
        let mac = self.uuid_mac(uuid)?;
        self.exists(
            "SELECT 1 FROM vcon.version WHERE tenant = $1 AND version = $2 AND uuid_mac = $3",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
            ],
        )
    }

    fn insert_version(&mut self, row: &NewVersion) -> Result<(), StoreError> {
        let mac = self.uuid_mac(row.uuid)?;
        let id = row.version.0;
        let form = self.seal("version", "form_sealed", &id, row.form.as_bytes())?;
        let gzip = self.seal("version", "gzip_sealed", &id, &gzip_byte(row.gzip))?;
        let size = self.seal("version", "size_sealed", &id, &size_bytes(row.size))?;
        self.execute(
            "INSERT INTO vcon.version
                 (tenant, version, uuid_mac, form_sealed, gzip_sealed, size_sealed, stored_at,
                  hidden_at, tenant_key_id, wrapped_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, $8, $9)",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&id.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
                (&form, Type::BYTEA),
                (&gzip, Type::BYTEA),
                (&size, Type::BYTEA),
                (&row.stored_at, Type::INT8),
                (&row.key.0.as_slice(), Type::BYTEA),
                (&row.wrapped, Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn insert_vcon(
        &mut self,
        uuid: Uuid,
        current: VersionId,
        stored_at: i64,
    ) -> Result<(), StoreError> {
        let mac = self.uuid_mac(uuid)?;
        let sealed = self.seal("vcon", "uuid_sealed", &mac, uuid.as_bytes())?;
        self.execute(
            "INSERT INTO vcon.vcon (tenant, uuid_mac, uuid_sealed, current, stored_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
                (&sealed, Type::BYTEA),
                (&current.0.as_slice(), Type::BYTEA),
                (&stored_at, Type::INT8),
            ],
        )
        .map(drop)
    }

    fn set_current(&mut self, uuid: Uuid, current: VersionId) -> Result<(), StoreError> {
        let mac = self.uuid_mac(uuid)?;
        self.execute(
            "UPDATE vcon.vcon SET current = $3 WHERE tenant = $1 AND uuid_mac = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
                (&current.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn hide_version(&mut self, version: VersionId, at: i64) -> Result<(), StoreError> {
        self.execute(
            "UPDATE vcon.version SET hidden_at = $3 WHERE tenant = $1 AND version = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
                (&at, Type::INT8),
            ],
        )
        .map(drop)
    }

    fn mark_deleted(&mut self, uuid: Uuid, at: i64) -> Result<(), StoreError> {
        let mac = self.uuid_mac(uuid)?;
        let params: [Param<'_>; 3] = [
            (&self.tenant.0.as_slice(), Type::BYTEA),
            (&mac.as_slice(), Type::BYTEA),
            (&at, Type::INT8),
        ];
        self.execute(
            "UPDATE vcon.vcon SET current = NULL, deleted_at = $3
             WHERE tenant = $1 AND uuid_mac = $2",
            &params,
        )?;
        self.execute(
            "UPDATE vcon.version SET hidden_at = $3
             WHERE tenant = $1 AND uuid_mac = $2 AND hidden_at IS NULL",
            &params,
        )
        .map(drop)
    }

    fn wraps(&mut self, key: KeyId) -> Result<u64, StoreError> {
        let rows = self.query(
            "SELECT wraps FROM vcon.key_use WHERE tenant = $1 AND tenant_key_id = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&key.0.as_slice(), Type::BYTEA),
            ],
        )?;
        let wraps: i64 = rows
            .first()
            .map(|row| row.try_get(0).map_err(io))
            .transpose()?
            .unwrap_or(0);
        u64::try_from(wraps).map_err(io)
    }

    fn add_wrap(&mut self, key: KeyId) -> Result<(), StoreError> {
        self.execute(
            "INSERT INTO vcon.key_use (tenant, tenant_key_id, wraps) VALUES ($1, $2, 1)
             ON CONFLICT (tenant, tenant_key_id) DO UPDATE SET wraps = vcon.key_use.wraps + 1",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&key.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn unpacked(&mut self) -> Result<Vec<UnpackedRow>, StoreError> {
        let rows = self.query(
            "SELECT version, stored_at, size_sealed FROM vcon.version
             WHERE tenant = $1 AND place_file IS NULL ORDER BY stored_at, version",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        rows.iter()
            .map(|row| {
                let version = VersionId(id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?)?);
                let sealed: Vec<u8> = row.try_get(2).map_err(io)?;
                let size = self.unseal("version", "size_sealed", &version.0, &sealed)?;
                Ok(UnpackedRow {
                    version,
                    stored_at: row.try_get(1).map_err(io)?,
                    size: size_of(&size)?,
                })
            })
            .collect()
    }

    fn is_unpacked(&mut self, version: VersionId) -> Result<bool, StoreError> {
        self.exists(
            "SELECT 1 FROM vcon.version
             WHERE tenant = $1 AND version = $2 AND place_file IS NULL",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
            ],
        )
    }

    fn insert_pack_file(&mut self, row: &NewPackFile) -> Result<(), StoreError> {
        self.execute(
            "INSERT INTO vcon.parquet_file (tenant, file_id, date, rows, live_rows, bytes, created_at)
             VALUES ($1, $2, $3, $4, $4, $5, $6)",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&row.file.0.as_slice(), Type::BYTEA),
                (&row.date, Type::TEXT),
                (&row.rows, Type::INT8),
                (&row.bytes, Type::INT8),
                (&row.created_at, Type::INT8),
            ],
        )
        .map(drop)
    }

    fn set_place(
        &mut self,
        version: VersionId,
        file: FileId,
        place: Place,
    ) -> Result<(), StoreError> {
        self.execute(
            "UPDATE vcon.version SET place_file = $3, place_group = $4, place_row = $5
             WHERE tenant = $1 AND version = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
                (&file.0.as_slice(), Type::BYTEA),
                (&i64::from(place.group), Type::INT8),
                (&i64::from(place.row), Type::INT8),
            ],
        )
        .map(drop)
    }

    fn pack_files(&mut self) -> Result<Vec<FileId>, StoreError> {
        let ids = self.ids(
            "SELECT file_id FROM vcon.parquet_file WHERE tenant = $1 ORDER BY file_id",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        Ok(ids.into_iter().map(FileId).collect())
    }

    fn pack_file(&mut self, file: FileId) -> Result<Option<PackFileRow>, StoreError> {
        let rows = self.query(
            "SELECT date, bytes, unnamed_at FROM vcon.parquet_file
             WHERE tenant = $1 AND file_id = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&file.0.as_slice(), Type::BYTEA),
            ],
        )?;
        rows.first()
            .map(|row| {
                let bytes: i64 = row.try_get(1).map_err(io)?;
                Ok(PackFileRow {
                    date: row.try_get(0).map_err(io)?,
                    bytes: u64::try_from(bytes).map_err(|_| StoreError::Damaged)?,
                    unnamed_at: row.try_get(2).map_err(io)?,
                })
            })
            .transpose()
    }

    fn placed_in(&mut self, file: FileId) -> Result<Vec<(VersionId, i64, Packed)>, StoreError> {
        let rows = self.query(
            &format!(
                "SELECT v.version, v.stored_at, {PLACE} FROM vcon.version v {PLACE_JOIN}
                 WHERE v.tenant = $1 AND v.place_file = $2 ORDER BY v.place_group, v.place_row"
            ),
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&file.0.as_slice(), Type::BYTEA),
            ],
        )?;
        rows.iter()
            .map(|row| {
                let version = VersionId(id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?)?);
                let packed = PgTx::row_place(row, 2)?.ok_or(StoreError::Damaged)?;
                Ok((version, row.try_get(1).map_err(io)?, packed))
            })
            .collect()
    }

    fn add_purge_pending(&mut self, file: FileId) -> Result<(), StoreError> {
        self.execute(
            "INSERT INTO vcon.purge_pending (tenant, file_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&file.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn purge_pending(&mut self) -> Result<Vec<FileId>, StoreError> {
        let ids = self.ids(
            "SELECT file_id FROM vcon.purge_pending WHERE tenant = $1 ORDER BY file_id",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        Ok(ids.into_iter().map(FileId).collect())
    }

    fn delete_purge_pending(&mut self, file: FileId) -> Result<(), StoreError> {
        self.execute(
            "DELETE FROM vcon.purge_pending WHERE tenant = $1 AND file_id = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&file.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn unname_pack_file(&mut self, file: FileId, at: i64) -> Result<(), StoreError> {
        self.execute(
            "UPDATE vcon.parquet_file SET unnamed_at = $3, live_rows = 0
             WHERE tenant = $1 AND file_id = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&file.0.as_slice(), Type::BYTEA),
                (&at, Type::INT8),
            ],
        )
        .map(drop)
    }

    fn unnamed_files(&mut self) -> Result<Vec<FileId>, StoreError> {
        let ids = self.ids(
            "SELECT file_id FROM vcon.parquet_file
             WHERE tenant = $1 AND unnamed_at IS NOT NULL ORDER BY file_id",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        Ok(ids.into_iter().map(FileId).collect())
    }

    fn delete_pack_file(&mut self, file: FileId) -> Result<(), StoreError> {
        self.execute(
            "DELETE FROM vcon.parquet_file WHERE tenant = $1 AND file_id = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&file.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn allocate_manifest(&mut self) -> Result<i64, StoreError> {
        let rows = self.returning(
            "INSERT INTO vcon.manifest (tenant, published_at) VALUES ($1, NULL) RETURNING seq",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        rows.first().ok_or(StoreError::Io)?.try_get(0).map_err(io)
    }

    fn unpublished_manifests(&mut self) -> Result<Vec<i64>, StoreError> {
        let rows = self.query(
            "SELECT seq FROM vcon.manifest
             WHERE tenant = $1 AND published_at IS NULL ORDER BY seq",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        rows.iter().map(|row| row.try_get(0).map_err(io)).collect()
    }

    fn mark_published(&mut self, seq: i64, at: i64) -> Result<(), StoreError> {
        self.execute(
            "UPDATE vcon.manifest SET published_at = $3 WHERE tenant = $1 AND seq = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&seq, Type::INT8),
                (&at, Type::INT8),
            ],
        )
        .map(drop)
    }

    fn published_manifests(&mut self) -> Result<Vec<(i64, i64)>, StoreError> {
        let rows = self.query(
            "SELECT seq, published_at FROM vcon.manifest
             WHERE tenant = $1 AND published_at IS NOT NULL ORDER BY seq",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        rows.iter()
            .map(|row| Ok((row.try_get(0).map_err(io)?, row.try_get(1).map_err(io)?)))
            .collect()
    }

    fn delete_manifest(&mut self, seq: i64) -> Result<(), StoreError> {
        self.execute(
            "DELETE FROM vcon.manifest WHERE tenant = $1 AND seq = $2",
            &[(&self.tenant.0.as_slice(), Type::BYTEA), (&seq, Type::INT8)],
        )
        .map(drop)
    }

    fn named_files(&mut self) -> Result<Vec<(FileId, PackFileRow, i64)>, StoreError> {
        let rows = self.query(
            "SELECT file_id, date, bytes, rows FROM vcon.parquet_file
             WHERE tenant = $1 AND unnamed_at IS NULL ORDER BY date, file_id",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        rows.iter()
            .map(|row| {
                let file = FileId(id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?)?);
                let bytes: i64 = row.try_get(2).map_err(io)?;
                let info = PackFileRow {
                    date: row.try_get(1).map_err(io)?,
                    bytes: u64::try_from(bytes).map_err(|_| StoreError::Damaged)?,
                    unnamed_at: None,
                };
                Ok((file, info, row.try_get(3).map_err(io)?))
            })
            .collect()
    }

    fn live_versions(&mut self) -> Result<Vec<VersionId>, StoreError> {
        let ids = self.ids(
            "SELECT current FROM vcon.vcon
             WHERE tenant = $1 AND current IS NOT NULL AND deleted_at IS NULL ORDER BY current",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        Ok(ids.into_iter().map(VersionId).collect())
    }

    fn places(
        &mut self,
    ) -> Result<Vec<(VersionId, Result<Option<Packed>, StoreError>)>, StoreError> {
        let rows = self.query(
            &format!(
                "SELECT v.version, {PLACE} FROM vcon.version v {PLACE_JOIN}
                 WHERE v.tenant = $1 ORDER BY v.version"
            ),
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        rows.iter()
            .map(|row| {
                let version = VersionId(id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?)?);
                Ok((version, PgTx::row_place(row, 1)))
            })
            .collect()
    }

    fn count_versions(&mut self) -> Result<u64, StoreError> {
        self.count_of(
            "SELECT count(*) FROM vcon.version WHERE tenant = $1",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )
    }

    fn count_packed(&mut self) -> Result<u64, StoreError> {
        self.count_of(
            "SELECT count(*) FROM vcon.version WHERE tenant = $1 AND place_file IS NOT NULL",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )
    }

    fn count_held(&mut self) -> Result<u64, StoreError> {
        self.count_of(
            "SELECT count(*) FROM vcon.vcon WHERE tenant = $1 AND hold <> 0",
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )
    }

    fn hidden_versions(&mut self, uuid: Option<Uuid>) -> Result<Vec<VersionId>, StoreError> {
        let mac = uuid
            .map(|uuid| self.uuid_mac(uuid).map(|mac| mac.to_vec()))
            .transpose()?;
        let ids = self.ids(
            "SELECT version FROM vcon.version
             WHERE tenant = $1 AND hidden_at IS NOT NULL AND ($2::bytea IS NULL OR uuid_mac = $2)
             ORDER BY seq",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac, Type::BYTEA),
            ],
        )?;
        Ok(ids.into_iter().map(VersionId).collect())
    }

    fn hidden_row(&mut self, version: VersionId) -> Result<Option<HiddenRow>, StoreError> {
        let rows = self.query(
            &format!(
                "SELECT c.uuid_sealed, c.uuid_mac, v.stored_at, c.hold <> 0, c.deleted_at IS NOT NULL
                 FROM vcon.version v {VCON_JOIN}
                 WHERE v.tenant = $1 AND v.version = $2 AND v.hidden_at IS NOT NULL"
            ),
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
            ],
        )?;
        rows.first()
            .map(|row| {
                Ok(HiddenRow {
                    uuid: self.row_uuid(row, 0, 1)?,
                    stored_at: row.try_get(2).map_err(io)?,
                    held: row.try_get(3).map_err(io)?,
                    deleted: row.try_get(4).map_err(io)?,
                })
            })
            .transpose()
    }

    fn delete_version(&mut self, version: VersionId) -> Result<(), StoreError> {
        self.execute(
            "DELETE FROM vcon.version WHERE tenant = $1 AND version = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn versions_of(&mut self, uuid: Uuid) -> Result<u64, StoreError> {
        let mac = self.uuid_mac(uuid)?;
        self.count_of(
            "SELECT count(*) FROM vcon.version WHERE tenant = $1 AND uuid_mac = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
            ],
        )
    }

    fn tombstone(&mut self, uuid: Uuid, at: i64) -> Result<(), StoreError> {
        let mac = self.uuid_mac(uuid)?;
        let sealed = self.seal("tombstone", "uuid_sealed", &mac, uuid.as_bytes())?;
        self.execute(
            "DELETE FROM vcon.vcon WHERE tenant = $1 AND uuid_mac = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
            ],
        )?;
        self.execute(
            "INSERT INTO vcon.tombstone (tenant, uuid_mac, uuid_sealed, purged_at)
             VALUES ($1, $2, $3, $4)",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
                (&sealed, Type::BYTEA),
                (&at, Type::INT8),
            ],
        )
        .map(drop)
    }

    fn latest(&mut self, uuid: Uuid) -> Result<Option<VersionId>, StoreError> {
        let mac = self.uuid_mac(uuid)?;
        let ids = self.ids(
            "SELECT version FROM vcon.version WHERE tenant = $1 AND uuid_mac = $2
             ORDER BY stored_at DESC, seq DESC LIMIT 1",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
            ],
        )?;
        Ok(ids.first().copied().map(VersionId))
    }

    fn kept(&mut self, uuid: Uuid) -> Result<Vec<KeptRow>, StoreError> {
        let mac = self.uuid_mac(uuid)?;
        let rows = self.query(
            "SELECT version, stored_at, hidden_at FROM vcon.version
             WHERE tenant = $1 AND uuid_mac = $2 ORDER BY stored_at, seq",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
            ],
        )?;
        rows.iter()
            .map(|row| {
                Ok(KeptRow {
                    version: VersionId(id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?)?),
                    stored_at: row.try_get(1).map_err(io)?,
                    hidden_at: row.try_get(2).map_err(io)?,
                })
            })
            .collect()
    }

    fn unhide(&mut self, version: VersionId) -> Result<(), StoreError> {
        self.execute(
            "UPDATE vcon.version SET hidden_at = NULL WHERE tenant = $1 AND version = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn undelete(&mut self, uuid: Uuid, version: VersionId) -> Result<(), StoreError> {
        let mac = self.uuid_mac(uuid)?;
        self.execute(
            "UPDATE vcon.vcon SET current = $3, deleted_at = NULL
             WHERE tenant = $1 AND uuid_mac = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn set_hold(&mut self, uuid: Uuid, on: bool) -> Result<(), StoreError> {
        let mac = self.uuid_mac(uuid)?;
        self.execute(
            "UPDATE vcon.vcon SET hold = $3 WHERE tenant = $1 AND uuid_mac = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&mac.as_slice(), Type::BYTEA),
                (&i64::from(on), Type::INT8),
            ],
        )
        .map(drop)
    }

    /// The order within one `stored_at` is the uuid's HMAC, since the
    /// uuid itself is not in the database: it is as fixed as the local
    /// catalog's uuid order, and a cursor's uuid is found by its HMAC.
    fn page(&mut self, query: &PageQuery) -> Result<Vec<ListedRow>, StoreError> {
        let before_at = query.before.map(|(at, _)| at);
        let before_mac = query
            .before
            .map(|(_, uuid)| self.uuid_mac(uuid).map(|mac| mac.to_vec()))
            .transpose()?;
        let rows = self.query(
            "SELECT c.uuid_sealed, c.uuid_mac, c.current, c.stored_at, v.stored_at
             FROM vcon.vcon c
             LEFT JOIN vcon.version v ON v.tenant = c.tenant AND v.version = c.current
             WHERE c.tenant = $1 AND c.current IS NOT NULL
               AND ($2::bigint IS NULL OR c.stored_at < $2
                    OR (c.stored_at = $2 AND c.uuid_mac < $3))
               AND ($4::bigint IS NULL OR c.stored_at >= $4)
               AND ($5::bigint IS NULL OR c.stored_at < $5)
             ORDER BY c.stored_at DESC, c.uuid_mac DESC
             LIMIT $6",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&before_at, Type::INT8),
                (&before_mac, Type::BYTEA),
                (&query.since, Type::INT8),
                (&query.until, Type::INT8),
                (&query.rows, Type::INT8),
            ],
        )?;
        rows.iter()
            .map(|row| {
                Ok(ListedRow {
                    uuid: self.row_uuid(row, 0, 1)?,
                    version: VersionId(id_of(&row.try_get::<_, Vec<u8>>(2).map_err(io)?)?),
                    stored_at: row.try_get(3).map_err(io)?,
                    updated_at: row.try_get(4).map_err(io)?,
                })
            })
            .collect()
    }

    fn wrapped_under(&mut self, key: KeyId, limit: usize) -> Result<Vec<WrappedRow>, StoreError> {
        let limit = i64::try_from(limit).map_err(io)?;
        let rows = self.query(
            &format!(
                "SELECT v.version, c.uuid_sealed, c.uuid_mac, v.wrapped_key
                 FROM vcon.version v {VCON_JOIN}
                 WHERE v.tenant = $1 AND v.tenant_key_id = $2 ORDER BY v.version LIMIT $3"
            ),
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&key.0.as_slice(), Type::BYTEA),
                (&limit, Type::INT8),
            ],
        )?;
        rows.iter()
            .map(|row| {
                Ok(WrappedRow {
                    version: VersionId(id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?)?),
                    uuid: self.row_uuid(row, 1, 2)?,
                    wrapped: row.try_get(3).map_err(io)?,
                })
            })
            .collect()
    }

    fn set_wrapped(
        &mut self,
        version: VersionId,
        key: KeyId,
        wrapped: &[u8],
    ) -> Result<(), StoreError> {
        self.execute(
            "UPDATE vcon.version SET wrapped_key = $3, tenant_key_id = $4
             WHERE tenant = $1 AND version = $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&version.0.as_slice(), Type::BYTEA),
                (&wrapped, Type::BYTEA),
                (&key.0.as_slice(), Type::BYTEA),
            ],
        )
        .map(drop)
    }

    fn count_not_under(&mut self, key: KeyId) -> Result<u64, StoreError> {
        self.count_of(
            "SELECT count(*) FROM vcon.version WHERE tenant = $1 AND tenant_key_id <> $2",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&key.0.as_slice(), Type::BYTEA),
            ],
        )
    }

    fn examined(&mut self) -> Result<Vec<ExaminedRow>, StoreError> {
        let rows = self.query(
            &format!(
                "SELECT v.version, c.uuid_sealed, c.uuid_mac, v.stored_at, v.hidden_at, c.hold <> 0
                 FROM vcon.version v {VCON_JOIN} WHERE v.tenant = $1 ORDER BY v.version"
            ),
            &[(&self.tenant.0.as_slice(), Type::BYTEA)],
        )?;
        rows.iter()
            .map(|row| {
                Ok(ExaminedRow {
                    version: VersionId(id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?)?),
                    uuid: self.row_uuid(row, 1, 2)?,
                    stored_at: row.try_get(3).map_err(io)?,
                    hidden_at: row.try_get(4).map_err(io)?,
                    held: row.try_get(5).map_err(io)?,
                })
            })
            .collect()
    }

    fn exported(
        &mut self,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<ExportedRow>, StoreError> {
        let rows = self.query(
            &format!(
                "SELECT v.version, c.uuid_sealed, c.uuid_mac, v.form_sealed, v.gzip_sealed,
                        v.size_sealed, v.stored_at,
                        c.current IS NOT DISTINCT FROM v.version AND c.deleted_at IS NULL,
                        v.wrapped_key, {PLACE}
                 FROM vcon.version v {VCON_JOIN} {PLACE_JOIN}
                 WHERE v.tenant = $1 AND ($2::bigint IS NULL OR v.stored_at >= $2)
                   AND ($3::bigint IS NULL OR v.stored_at < $3)
                 ORDER BY v.stored_at, v.version"
            ),
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&since, Type::INT8),
                (&until, Type::INT8),
            ],
        )?;
        rows.iter()
            .map(|row| {
                let version = VersionId(id_of(&row.try_get::<_, Vec<u8>>(0).map_err(io)?)?);
                let (form, gzip, size) = self.row_sealed(row, version, 3)?;
                Ok(ExportedRow {
                    version,
                    uuid: self.row_uuid(row, 1, 2)?,
                    form,
                    gzip,
                    size,
                    stored_at: row.try_get(6).map_err(io)?,
                    live: row.try_get(7).map_err(io)?,
                    wrapped: row.try_get(8).map_err(io)?,
                    place: PgTx::row_place(row, 9)?,
                })
            })
            .collect()
    }

    fn purge_times(&mut self) -> Result<Vec<String>, StoreError> {
        let rows = self.query(
            "SELECT at FROM vcon.audit WHERE tenant = $1 AND action = $2 AND status = 200",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&Action::Purge.as_str(), Type::TEXT),
            ],
        )?;
        rows.iter().map(|row| row.try_get(0).map_err(io)).collect()
    }

    fn begin_part(&mut self) -> Result<(), StoreError> {
        self.batch("SAVEPOINT part")
    }

    fn release_part(&mut self) -> Result<(), StoreError> {
        self.batch("RELEASE SAVEPOINT part")
    }

    /// Back to where the part began. It also ends the abort a failed
    /// write of the part left, so the transaction goes on.
    fn rollback_part(&mut self) -> Result<(), StoreError> {
        let outcome = self.trip(
            self.client()
                .batch_execute("ROLLBACK TO SAVEPOINT part; RELEASE SAVEPOINT part"),
        );
        match outcome {
            Ok(()) => {
                self.aborted.set(false);
                Ok(())
            }
            Err(e) => {
                self.aborted.set(true);
                self.failed(&e);
                Err(StoreError::Io)
            }
        }
    }

    fn record(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        PgTx::record(self, entry, now)
    }

    fn count(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        self.count_record(entry, now, 1)
    }

    fn audit_lines(
        &mut self,
        after: i64,
        limit: Option<u32>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError> {
        PgTx::audit_lines(self, after, limit.map(i64::from), out)
    }

    fn commit(self: Box<Self>) -> Result<(), CommitError> {
        self.finish()
    }
}

impl PgTx<'_> {
    /// A commit's outcome, on its way to the caller: after an unknown one
    /// the connection's state is not known either, so it is closed and
    /// not kept.
    fn ended(&self, outcome: Result<(), CommitError>) -> Result<(), CommitError> {
        if outcome == Err(CommitError::Unknown) {
            self.broken.set(true);
        }
        outcome
    }
}

/// In tests, the commit fault set for this commit, if any, as the local
/// catalog's: Unknown commits and reports failure; RolledBack rolls back.
#[cfg(test)]
fn injected(fault: &Fault) -> Option<CommitError> {
    let mut fault = fault.lock().expect("fault");
    match *fault {
        Some((1, how)) => {
            *fault = None;
            Some(how)
        }
        Some((nth, how)) => {
            *fault = Some((nth - 1, how));
            None
        }
        None => None,
    }
}
