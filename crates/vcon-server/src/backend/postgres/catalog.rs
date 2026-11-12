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
//!   catalog's. The tenant is set here and nowhere else; every statement
//!   names it too, and the policy is the second check.
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

#![cfg_attr(
    not(all(test, feature = "backend-tests")),
    expect(
        dead_code,
        reason = "a tenant's space opens these catalogs from the next change; the tests do now"
    )
)]

use std::cell::Cell;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

use time::OffsetDateTime;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{Client, Row, SimpleQueryMessage};
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
use crate::envelope::{self as column, ColumnContext, ColumnKeys, Key};
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
    lock: Option<Lease>,
    pool: Pool,
    runtime: tokio::runtime::Runtime,
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

/// The fewest connections a server's pool may have: the one that holds
/// the servers' lock, and one for a transaction.
pub(crate) const SMALLEST_POOL: u32 = 2;

impl Database {
    /// Reads the secret files, connects as a server does, and takes the
    /// lock every server shares, on a connection kept for that alone.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming the step that refused: a secret file, the
    /// pool's size, the server reached, or the lock another server or a
    /// migration holds.
    pub(crate) fn connect(settings: &PostgresSettings) -> Result<Arc<Database>, StorageError> {
        if settings.pool < SMALLEST_POOL {
            return Err(StorageError(pool_refusal(settings.pool)));
        }
        let secrets = Secrets::read(settings)?;
        let connector = Connector::new(settings, secrets, Purpose::Serve)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("vcon-postgres")
            .enable_all()
            .build()
            .map_err(|e| StorageError(format!("the runtime cannot start: {e}")))?;
        let pool = Pool::new(connector, settings.pool, settings.connect_timeout);
        let lock = runtime
            .block_on(pool.get())
            .map_err(|e| StorageError(format!("catalog.postgres: {e}")))?;
        runtime
            .block_on(schema::take_server_lock(lock.client()))
            .map_err(|why| StorageError(format!("catalog.postgres: {why}")))?;
        Ok(Arc::new(Database {
            lock: Some(lock),
            pool,
            runtime,
            #[cfg(test)]
            trips: std::sync::atomic::AtomicUsize::new(0),
        }))
    }

    /// Waits for `call` on the pool's runtime. The calling thread must not
    /// be a runtime's worker.
    fn wait<T>(&self, call: impl Future<Output = T>) -> T {
        #[cfg(test)]
        self.trips.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.runtime.handle().block_on(call)
    }

    /// A connection from the pool. Taking one that waits unused sends
    /// nothing, so it is not a round trip.
    fn lease(&self) -> Result<Lease, StoreError> {
        self.runtime.handle().block_on(self.pool.get()).map_err(io)
    }

    /// In tests, the round trips so far.
    #[cfg(test)]
    pub(crate) fn round_trips(&self) -> usize {
        self.trips.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        // The lock is given up while the connection still answers, so the
        // next server finds it free; a lock left to the end of the
        // session is held until the server notices the closed connection.
        if let Some(lock) = self.lock.take() {
            let _ = self.runtime.block_on(
                lock.client()
                    .batch_execute("SELECT pg_advisory_unlock_all()"),
            );
        }
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

/// The one round trip that begins a transaction of `tenant` as `how`:
/// the transaction, the tenant the policies read, and for a write the
/// lock on the tenant's row, whose absence is a removed tenant. The
/// tenant's id is 32 hex digits, so it is written into the text as it is.
pub(crate) fn begin_text(how: Begin, tenant: TenantId) -> String {
    let set = format!("SET LOCAL vcon.tenant = '{tenant}'");
    match how {
        Begin::Write => format!(
            "BEGIN ISOLATION LEVEL READ COMMITTED; {set}; \
             SELECT 1 FROM vcon.tenant_lock WHERE tenant = '\\x{tenant}' FOR UPDATE"
        ),
        Begin::Read => format!("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; {set}"),
        Begin::Create => format!("BEGIN ISOLATION LEVEL READ COMMITTED; {set}"),
    }
}

/// What a COMMIT that failed means. The server answered with an error
/// (`answered`): it rolled the transaction back. No answer came: the
/// transaction may or may not have been kept.
pub(crate) fn commit_failure(answered: bool) -> CommitError {
    if answered {
        CommitError::RolledBack
    } else {
        CommitError::Unknown
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

/// A tenant's catalog in the database.
pub(crate) struct PgCatalog {
    database: Arc<Database>,
    tenant: TenantId,
    keys: ColumnKeys,
    #[cfg(test)]
    fault: Mutex<Option<(usize, CommitError)>>,
}

impl PgCatalog {
    fn with(
        database: Arc<Database>,
        tenant: TenantId,
        catalog_key: &Key,
    ) -> Result<PgCatalog, CatalogError> {
        Ok(PgCatalog {
            database,
            tenant,
            keys: ColumnKeys::derive(catalog_key).map_err(|_| CatalogError::Io)?,
            #[cfg(test)]
            fault: Mutex::new(None),
        })
    }

    /// Makes the catalog of a new tenant, sealed under `catalog_key`: the
    /// row its writes lock, and the id of the key.
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
        let catalog = PgCatalog::with(database, tenant, catalog_key)?;
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
        Box::new(tx).commit().map_err(|_| CatalogError::Io)?;
        Ok(catalog)
    }

    /// Whether the database has a catalog for `tenant`.
    ///
    /// # Errors
    ///
    /// [`CatalogError::Io`] when the database cannot be read.
    pub(crate) fn exists(database: &Arc<Database>, tenant: TenantId) -> Result<bool, CatalogError> {
        Ok(PgCatalog::key_id(database, tenant)?.is_some())
    }

    /// The id of the key `tenant`'s catalog is sealed under, if it has a
    /// catalog.
    fn key_id(database: &Arc<Database>, tenant: TenantId) -> Result<Option<Vec<u8>>, CatalogError> {
        let lease = database.lease().map_err(|_| CatalogError::Io)?;
        let tx = PgTx::begin(database, lease, tenant, None, Begin::Read)
            .map_err(|_| CatalogError::Io)?;
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
    /// [`CatalogError::Io`] when the database cannot be read.
    pub(crate) fn open(
        database: Arc<Database>,
        tenant: TenantId,
        catalog_key: &Key,
    ) -> Result<PgCatalog, CatalogError> {
        let held = PgCatalog::key_id(&database, tenant)?.ok_or(CatalogError::NoTenant)?;
        if held != catalog_key.id().0 {
            return Err(CatalogError::WrongKey);
        }
        PgCatalog::with(database, tenant, catalog_key)
    }

    fn begin(&self, how: Begin) -> Result<PgTx<'_>, StoreError> {
        let lease = self.database.lease()?;
        PgTx::begin(&self.database, lease, self.tenant, Some(self), how)
    }
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

/// A transaction on a tenant's catalog: one pooled connection, rolled
/// back when dropped without a commit.
struct PgTx<'a> {
    database: &'a Database,
    lease: Lease,
    tenant: TenantId,
    /// The catalog whose keys seal and open the columns; none for the
    /// read that finds a tenant's key id before there is a catalog.
    catalog: Option<&'a PgCatalog>,
    open: bool,
    /// A write failed: the server refuses every statement but the end.
    aborted: Cell<bool>,
    /// A statement failed in a way that says the connection cannot be
    /// used again ([`must_drop`]): it is closed when the transaction ends.
    broken: Cell<bool>,
}

impl Drop for PgTx<'_> {
    fn drop(&mut self) {
        if self.open {
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
    fn begin(
        database: &'a Database,
        mut lease: Lease,
        tenant: TenantId,
        catalog: Option<&'a PgCatalog>,
        how: Begin,
    ) -> Result<PgTx<'a>, StoreError> {
        let began = database.wait(lease.client().simple_query(&begin_text(how, tenant)));
        let locked = match began {
            Ok(messages) => messages
                .iter()
                .any(|m| matches!(m, SimpleQueryMessage::Row(_))),
            Err(e) => {
                // The transaction may have begun before the statement
                // that failed; the connection is not kept in that state.
                lease.failed(&e);
                let rolled = database.wait(lease.client().batch_execute("ROLLBACK"));
                if rolled.is_err() {
                    lease.discard();
                }
                return Err(StoreError::Io);
            }
        };
        let tx = PgTx {
            database,
            lease,
            tenant,
            catalog,
            open: true,
            aborted: Cell::new(false),
            broken: Cell::new(false),
        };
        if how == Begin::Write && !locked {
            // No lock row: the tenant was removed. The drop rolls back.
            return Err(StoreError::Io);
        }
        Ok(tx)
    }

    fn client(&self) -> &Client {
        self.lease.client()
    }

    /// Notes a statement's failure: whether the connection may be kept.
    fn failed(&self, e: &tokio_postgres::Error) {
        if must_drop(e) {
            self.broken.set(true);
        }
    }

    fn keys(&self) -> Result<&ColumnKeys, StoreError> {
        self.catalog.map(|c| &c.keys).ok_or(StoreError::Io)
    }

    /// A read, inside a savepoint sent with it in one round trip: when it
    /// fails, the transaction is as it was before it.
    fn query(&self, sql: &str, params: &[Param<'_>]) -> Result<Vec<Row>, StoreError> {
        self.query_checked(sql, params).map_err(io)
    }

    fn query_checked(
        &self,
        sql: &str,
        params: &[Param<'_>],
    ) -> Result<Vec<Row>, tokio_postgres::Error> {
        let client = self.lease.client();
        let outcome = self.database.wait(async {
            let (saved, rows, released) = tokio::join!(
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
    fn execute(&self, sql: &str, params: &[Param<'_>]) -> Result<u64, StoreError> {
        self.execute_checked(sql, params).map_err(io)
    }

    fn execute_checked(
        &self,
        sql: &str,
        params: &[Param<'_>],
    ) -> Result<u64, tokio_postgres::Error> {
        let outcome = self
            .database
            .wait(self.lease.client().execute_typed(sql, params));
        if let Err(e) = &outcome {
            self.aborted.set(true);
            self.failed(e);
        }
        outcome
    }

    /// A write that returns rows, such as an INSERT with RETURNING: it
    /// runs as a write, with no savepoint.
    fn returning(&self, sql: &str, params: &[Param<'_>]) -> Result<Vec<Row>, StoreError> {
        let outcome = self
            .database
            .wait(self.lease.client().query_typed(sql, params));
        outcome.map_err(|e| {
            self.aborted.set(true);
            self.failed(&e);
            StoreError::Io
        })
    }

    fn batch(&self, sql: &str) -> Result<(), StoreError> {
        let outcome = self.database.wait(self.client().batch_execute(sql));
        outcome.map_err(|e| {
            self.aborted.set(true);
            self.failed(&e);
            StoreError::Io
        })
    }

    fn exists(&self, sql: &str, params: &[Param<'_>]) -> Result<bool, StoreError> {
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

    fn seal(
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

    fn unseal(
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
        let uuid = Uuid::from_bytes(id_of(&bytes)?);
        if self.uuid_mac(uuid)?.as_slice() == mac {
            Ok(uuid)
        } else {
            Err(StoreError::Damaged)
        }
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

    /// The HMAC and the sealed copy of an audit record's target.
    fn sealed_target(&self, target: Option<&str>) -> Result<SealedTarget, StoreError> {
        let Some(target) = target else {
            return Ok((None, None));
        };
        let mac = column::mac(self.keys()?, target.as_bytes());
        let sealed = self.seal("audit", "target_sealed", &mac, target.as_bytes())?;
        Ok((Some(mac.to_vec()), Some(sealed)))
    }

    /// Writes at most `limit` records after the seq `after` as JSON
    /// lines, oldest first, as the local log does (`audit::export`).
    fn audit_lines(
        &self,
        after: i64,
        limit: Option<i64>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError> {
        let rows = self
            .query_checked(
                "SELECT seq, at, actor, action, target_mac, target_sealed, version, status, count
                 FROM vcon.audit WHERE tenant = $1 AND seq > $2 ORDER BY seq LIMIT $3",
                &[
                    (&self.tenant.0.as_slice(), Type::BYTEA),
                    (&after, Type::INT8),
                    (&limit, Type::INT8),
                ],
            )
            .map_err(|e| audit_failed(&e))?;
        let damaged = || AuditError::Database("an audit record cannot be read".to_owned());
        for row in rows {
            let get = |e: tokio_postgres::Error| audit_failed(&e);
            let mac: Option<Vec<u8>> = row.try_get(4).map_err(get)?;
            let sealed: Option<Vec<u8>> = row.try_get(5).map_err(get)?;
            let target = match (mac, sealed) {
                (Some(mac), Some(sealed)) => {
                    let bytes = self
                        .unseal("audit", "target_sealed", &mac, &sealed)
                        .map_err(|_| damaged())?;
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
    /// marks a record [`CatalogTx::count`] adds to.
    fn insert_audit(&self, entry: &Entry, at: &str, counted: bool) -> Result<(), AuditError> {
        let unsealed = || AuditError::Database("an audit record cannot be sealed".to_owned());
        let (mac, sealed) = self
            .sealed_target(entry.target.as_deref())
            .map_err(|_| unsealed())?;
        let actor = entry.actor.name();
        let version = entry.version.map(|v| v.to_string());
        let status = audit::stored(entry.status).map(i64::from);
        self.execute_checked(
            "INSERT INTO vcon.audit
                 (tenant, at, actor, action, target_mac, target_sealed, version, status, count, counted)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9)",
            &[
                (&self.tenant.0.as_slice(), Type::BYTEA),
                (&at, Type::TEXT),
                (&actor, Type::TEXT),
                (&entry.action.as_str(), Type::TEXT),
                (&mac, Type::BYTEA),
                (&sealed, Type::BYTEA),
                (&version, Type::TEXT),
                (&status, Type::INT8),
                (&counted, Type::BOOL),
            ],
        )
        .map(drop)
        .map_err(|e| audit_failed(&e))
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
        let outcome = self.database.wait(
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
        self.insert_audit(entry, &audit::rfc3339(now), false)
    }

    /// Adds one to the record for this actor, action, target, version and
    /// status in the minute of `now`, making it if there is none. Only a
    /// record this call made is added to (`counted`).
    fn count(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        let unsealed = || AuditError::Database("an audit record cannot be sealed".to_owned());
        let at = audit::rfc3339(audit::minute_of(now));
        let mac = entry
            .target
            .as_deref()
            .map(|target| {
                self.keys()
                    .map(|keys| column::mac(keys, target.as_bytes()).to_vec())
            })
            .transpose()
            .map_err(|_| unsealed())?;
        let actor = entry.actor.name();
        let version = entry.version.map(|v| v.to_string());
        let status = audit::stored(entry.status).map(i64::from);
        let added = self
            .execute_checked(
                "UPDATE vcon.audit SET count = count + 1
                 WHERE tenant = $1 AND counted AND at = $2 AND actor = $3 AND action = $4
                   AND target_mac IS NOT DISTINCT FROM $5 AND version IS NOT DISTINCT FROM $6
                   AND status IS NOT DISTINCT FROM $7",
                &[
                    (&self.tenant.0.as_slice(), Type::BYTEA),
                    (&at, Type::TEXT),
                    (&actor, Type::TEXT),
                    (&entry.action.as_str(), Type::TEXT),
                    (&mac, Type::BYTEA),
                    (&version, Type::TEXT),
                    (&status, Type::INT8),
                ],
            )
            .map_err(|e| audit_failed(&e))?;
        if added == 0 {
            self.insert_audit(entry, &at, true)?;
        }
        Ok(())
    }

    fn audit_lines(
        &mut self,
        after: i64,
        limit: Option<u32>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError> {
        PgTx::audit_lines(self, after, limit.map(i64::from), out)
    }

    fn commit(mut self: Box<Self>) -> Result<(), CommitError> {
        #[cfg(test)]
        if let Some(how) = self.catalog.and_then(injected) {
            let end = match how {
                CommitError::Unknown => "COMMIT",
                CommitError::RolledBack => "ROLLBACK",
            };
            let ended = self.database.wait(self.client().batch_execute(end));
            ended.expect("ends the transaction");
            self.open = false;
            return Err(how);
        }
        if self.aborted.get() {
            // The drop sends the ROLLBACK the server is waiting for.
            return Err(CommitError::RolledBack);
        }
        let outcome = self.database.wait(self.client().batch_execute("COMMIT"));
        self.open = false;
        match outcome {
            Ok(()) => Ok(()),
            Err(e) => {
                let answered = e.code().is_some();
                if !answered {
                    self.broken.set(true);
                }
                Err(commit_failure(answered))
            }
        }
    }
}

/// In tests, the commit fault set for this commit, if any, as the local
/// catalog's: Unknown commits and reports failure; RolledBack rolls back.
#[cfg(test)]
fn injected(catalog: &PgCatalog) -> Option<CommitError> {
    let mut fault = catalog.fault.lock().expect("fault");
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
