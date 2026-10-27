//! The registry of tenants: each one's id, name, retention period and
//! wrapped tenant key, kept in `server.sqlite` under a catalog key derived
//! from the master key, with the admin audit log beside them. Each tenant's
//! vCons live in `tenants/<id>/`, in a Store of their own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use time::OffsetDateTime;

use crate::audit::{self, Action, Actor, Entry};
use crate::catalog::{self, CatalogError};
use crate::clock::{Clock, from_nanos, nanos};
use crate::envelope::{self, Context, Key, WrapLimit};
use crate::fs::Fs;
use crate::ids::{KeyId, TenantId};
use crate::retention::Retention;
use crate::store::{OpenError, Store};

mod backup;
mod examine;
mod keys;
mod rekey;

pub use backup::{Backup, BackupError};
pub use examine::{KeyOrder, RegistryExamination, RotationBacklog, TenantExamination};
pub use keys::{KeyInfo, NewKey};
pub use rekey::rekey_master;
#[cfg(test)]
pub(crate) use tests::{Fixture, fixture};

/// The tables of server.sqlite, besides the admin audit log. A key column
/// says NOT NULL because SQLite lets a PRIMARY KEY other than an INTEGER
/// one hold NULL. Times are nanoseconds since the epoch. A retention period
/// is written as `Retention` writes it; NULL means the server default.
const TABLES: &str = "
CREATE TABLE tenant (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    retention TEXT,
    created_at INTEGER NOT NULL,
    tenant_key_id BLOB NOT NULL,
    wrapped_tenant_key BLOB NOT NULL,
    pending_wrapped_tenant_key BLOB,
    key_since INTEGER NOT NULL
);
CREATE TABLE master_key_use (
    master_key_id BLOB PRIMARY KEY NOT NULL,
    wraps INTEGER NOT NULL,
    since INTEGER NOT NULL
);
CREATE TABLE api_key (
    id BLOB PRIMARY KEY NOT NULL,
    tenant BLOB,
    token_hash BLOB NOT NULL,
    scopes TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE INDEX api_key_tenant ON api_key (tenant);
";

/// A tenant's retention column as read: None is the server default. A
/// period that does not read back is damage, not the server default.
fn stored_retention(text: Option<String>) -> Result<Option<Retention>, TenantError> {
    text.map(|text| Retention::parse(&text).ok_or(TenantError::Io))
        .transpose()
}

/// Why a Tenants call was refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantError {
    /// No tenant has the id.
    NotFound,
    /// Another tenant has the name.
    NameTaken,
    /// The name breaks the naming rule (`good_name`).
    BadName,
    /// `Removal::Empty` was asked of a tenant that keeps versions.
    HoldsVcons,
    /// `Removal::Force` was asked of a tenant with a uuid under a legal
    /// hold.
    UnderHold,
    /// The master key is at its wrap limit and must be rotated.
    LimitReached,
    /// The audit record could not be written, so nothing was done.
    AuditFailed,
    /// The tenant is being removed.
    Closing,
    /// Only the command line may do this: make an operator key.
    CliOnly,
    /// Scopes a key of its kind cannot hold.
    BadScopes,
    /// server.sqlite was made under another master key.
    WrongKey,
    Io,
}

/// Why a tenant's Store could not be had: the registry refused, its
/// catalog does not open under its key, or this process may not use one of
/// its files. Only `check` and `serve`'s start-up check tell these apart;
/// every other caller sees `TenantError` (`From` below).
#[derive(Debug)]
pub enum OpenFailure {
    Refused(TenantError),
    WrongKey,
    Access {
        path: PathBuf,
        error: std::io::Error,
    },
}

impl From<OpenFailure> for TenantError {
    fn from(f: OpenFailure) -> TenantError {
        match f {
            OpenFailure::Refused(e) => e,
            OpenFailure::WrongKey | OpenFailure::Access { .. } => TenantError::Io,
        }
    }
}

impl From<TenantError> for OpenFailure {
    fn from(e: TenantError) -> OpenFailure {
        OpenFailure::Refused(e)
    }
}

impl From<OpenError> for OpenFailure {
    fn from(e: OpenError) -> OpenFailure {
        match e {
            OpenError::WrongKey => OpenFailure::WrongKey,
            OpenError::Access { path, error } => OpenFailure::Access { path, error },
            OpenError::Store(_) => OpenFailure::Refused(TenantError::Io),
        }
    }
}

/// A tenant as `list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantInfo {
    pub id: TenantId,
    pub name: String,
    /// The tenant's own retention period, if it set one.
    pub retention: Option<Retention>,
    pub created_at: OffsetDateTime,
}

impl TenantInfo {
    /// The tenant as the admin API and the command line show it: id, name
    /// and its own retention period, null for the server default.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id.to_string(),
            "name": self.name,
            "retention": self.retention.map(|r| r.to_string()),
        })
    }

    /// How long the tenant's vCons are kept: its own period, else the
    /// server's `default`.
    pub fn kept_for(&self, default: Retention) -> Retention {
        self.retention.unwrap_or(default)
    }
}

/// How far `remove` may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// Only a tenant that keeps no version.
    Empty,
    /// Its vCons too, unless a uuid is under a legal hold.
    Force,
    /// Even held vCons; only from the command line, with the server stopped.
    ForceOverridingHolds,
}

/// A tenant's Store as the registry holds it.
enum Slot {
    Open(Arc<Store>),
    /// Being removed: no new handle is handed out.
    Closing,
}

/// The registry of tenants.
pub struct Tenants {
    /// `<data>/tenants`, which holds a directory per tenant.
    tenants: PathBuf,
    master: Key,
    conn: Mutex<Connection>,
    /// Each tenant's Store, once a request has opened it.
    stores: Mutex<HashMap<TenantId, Slot>>,
    /// Opened beside a server: its Stores are opened without recovery.
    beside: bool,
    fs: Arc<dyn Fs>,
    clock: Arc<dyn Clock>,
    #[cfg(test)]
    hooks: tests::Hooks,
}

impl Tenants {
    /// Opens `<dir>/server.sqlite` under the master key's catalog key,
    /// making it if new, and removes any tenant directory a crash left
    /// without its row.
    pub fn open(
        dir: &Path,
        master: Key,
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
    ) -> Result<Tenants, TenantError> {
        let registry = Tenants::opened(dir, master, fs, clock, false)?;
        registry.recover()?;
        Ok(registry)
    }

    /// Opens the registry as a command running beside a server does:
    /// without start-up recovery, here or in any Store it opens, which
    /// would take the server's creates and writes in progress for a
    /// crash's leftovers.
    pub fn open_beside(
        dir: &Path,
        master: Key,
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
    ) -> Result<Tenants, TenantError> {
        Tenants::opened(dir, master, fs, clock, true)
    }

    fn opened(
        dir: &Path,
        master: Key,
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
        beside: bool,
    ) -> Result<Tenants, TenantError> {
        let catalog_key = envelope::catalog_key(&master).map_err(|_| TenantError::Io)?;
        let schema = format!("{TABLES}{}", audit::SCHEMA);
        let conn = catalog::open(&dir.join("server.sqlite"), &catalog_key, &schema).map_err(
            |e| match e {
                CatalogError::WrongKey => TenantError::WrongKey,
                _ => TenantError::Io,
            },
        )?;
        let tenants = dir.join("tenants");
        fs.create_dir_all(&tenants).map_err(io)?;
        fs.sync_dir(dir).map_err(io)?;
        Ok(Tenants {
            tenants,
            master,
            conn: Mutex::new(conn),
            stores: Mutex::new(HashMap::new()),
            beside,
            fs,
            clock,
            #[cfg(test)]
            hooks: tests::Hooks::default(),
        })
    }

    /// Removes each tenant directory with no row: a create cut off before
    /// its commit. A name that is not a tenant id is not ours and is left
    /// alone. A removal a crash undoes is made again at the next start, so
    /// none is synced.
    fn recover(&self) -> Result<(), TenantError> {
        for name in self.fs.list(&self.tenants).map_err(io)? {
            let Some(id) = name.to_str().and_then(TenantId::parse) else {
                continue;
            };
            let has_row = self
                .conn()
                .query_row("SELECT 1 FROM tenant WHERE id = ?1", [id.0], |_| Ok(()))
                .optional()
                .map_err(io)?
                .is_some();
            if !has_row {
                self.fs
                    .remove_dir_all(&self.tenants.join(&name))
                    .map_err(io)?;
            }
        }
        Ok(())
    }

    /// Makes a tenant: a new tenant key wrapped under the master key, the
    /// tenant's directory and catalog, and then its row. A create that
    /// fails after making its directory removes it if it can; any directory
    /// left without a row, by a crash or a failed removal, goes at the next
    /// start.
    pub fn create(
        &self,
        who: Actor,
        name: &str,
        retention: Option<Retention>,
    ) -> Result<TenantId, TenantError> {
        let id = TenantId::generate();
        let entry = |status, target: Option<TenantId>| Entry {
            actor: who,
            action: Action::TenantCreate,
            target: target.map(|t| t.to_string()),
            version: None,
            status,
        };
        let master_id = self.master.id();
        {
            let mut conn = self.conn();
            let tx = immediate(&mut conn)?;
            let refused = if !good_name(name) {
                Some((422, TenantError::BadName))
            } else if name_taken(&tx, name)? {
                Some((409, TenantError::NameTaken))
            } else if envelope::wrap_limit(wraps(&tx, master_id)?) == WrapLimit::Refuse {
                Some((503, TenantError::LimitReached))
            } else {
                None
            };
            if let Some((status, error)) = refused {
                self.record(&tx, &entry(status, None))?;
                tx.commit().map_err(io)?;
                return Err(error);
            }
            // The wrap is counted before it is made, so every wrap is
            // counted once whether this create wins or loses.
            add_wrap(&tx, master_id, nanos(self.clock.now()))?;
            tx.commit().map_err(io)?;
        }

        let tenant_key = Key::generate().map_err(|_| TenantError::Io)?;
        let key_id = tenant_key.id();
        let wrapped = envelope::wrap(&self.master, &Context::Tenant { tenant: id }, &tenant_key);
        // The directory and its catalog are made before the row, and made
        // durable: the directory's entry in `tenants/` by syncing it. Any
        // step that fails once the directory may exist removes it.
        let dir = self.tenants.join(id.to_string());
        let made = (|| {
            self.fs.create_dir_all(&dir).map_err(io)?;
            drop(
                Store::open(&dir, id, tenant_key, self.fs.clone(), self.clock.clone())
                    .map_err(|_| TenantError::Io)?,
            );
            self.fs.sync_dir(&self.tenants).map_err(io)?;
            #[cfg(test)]
            {
                let hook = self.hooks.between.lock().expect("hook").take();
                if let Some(hook) = hook {
                    hook();
                }
            }

            let mut conn = self.conn();
            let tx = immediate(&mut conn)?;
            if name_taken(&tx, name)? {
                // A refusal's record commits.
                self.record(&tx, &entry(409, None))?;
                tx.commit().map_err(io)?;
                return Err(TenantError::NameTaken);
            }
            tx.execute(
                "INSERT INTO tenant (id, name, retention, created_at, tenant_key_id,
                     wrapped_tenant_key, key_since)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?4)",
                params![
                    id.0,
                    name,
                    retention.map(|r| r.to_string()),
                    nanos(self.clock.now()),
                    key_id.bytes(),
                    wrapped
                ],
            )
            .map_err(io)?;
            self.record(&tx, &entry(201, Some(id)))?;
            tx.commit().map_err(io)?;
            Ok(id)
        })();
        if made.is_err() {
            let _ = self.fs.remove_dir_all(&dir);
        }
        made
    }

    /// Every tenant, by name.
    pub fn list(&self, who: Actor) -> Result<Vec<TenantInfo>, TenantError> {
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        let tenants = tx
            .prepare("SELECT id, name, retention, created_at FROM tenant ORDER BY name")
            .map_err(io)?
            .query_map([], |r| {
                Ok((
                    r.get::<_, [u8; 16]>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(io)?
            .map(|row| {
                let (id, name, retention, created_at) = row.map_err(io)?;
                Ok(TenantInfo {
                    id: TenantId(id),
                    name,
                    retention: stored_retention(retention)?,
                    created_at: from_nanos(created_at).ok_or(TenantError::Io)?,
                })
            })
            .collect::<Result<Vec<_>, TenantError>>()?;
        let entry = Entry {
            actor: who,
            action: Action::TenantList,
            target: None,
            version: None,
            status: 200,
        };
        self.record(&tx, &entry)?;
        tx.commit().map_err(io)?;
        Ok(tenants)
    }

    /// Sets the tenant's retention period, or with None returns it to the
    /// server default.
    pub fn set_retention(
        &self,
        who: Actor,
        id: TenantId,
        retention: Option<Retention>,
    ) -> Result<(), TenantError> {
        let entry = |action, status| Entry {
            actor: who,
            action,
            target: Some(id.to_string()),
            version: None,
            status,
        };
        let closing = matches!(self.stores().get(&id), Some(Slot::Closing));
        {
            let mut conn = self.conn();
            let tx = immediate(&mut conn)?;
            let refused = if closing {
                Some((503, TenantError::Closing))
            } else if tx
                .execute(
                    "UPDATE tenant SET retention = ?2 WHERE id = ?1",
                    params![id.0, retention.map(|r| r.to_string())],
                )
                .map_err(io)?
                == 0
            {
                Some((404, TenantError::NotFound))
            } else {
                None
            };
            let status = refused.map_or(200, |(status, _)| status);
            self.record(&tx, &entry(Action::TenantRetention, status))?;
            tx.commit().map_err(io)?;
            if let Some((_, error)) = refused {
                return Err(error);
            }
        }
        self.copy_to_tenant(id, &entry(Action::Retention, 200))
    }

    /// Writes the copy of an admin record that a tenant's own log keeps
    /// too. The change it records has already committed with its admin
    /// record, so a crash or failure here can lose only the copy; the
    /// caller then answers that the audit failed, though the change
    /// stands.
    fn copy_to_tenant(&self, tenant: TenantId, entry: &Entry) -> Result<(), TenantError> {
        let store = self.store(tenant).map_err(|_| TenantError::AuditFailed)?;
        store.log(entry).map_err(|_| TenantError::AuditFailed)
    }

    /// Removes the tenant: its row and wrapped key, then its directory.
    ///
    /// The tenant is drained before it is looked at, so a write that lands
    /// meanwhile counts: an `Empty` removal is then refused rather than
    /// losing it. A refused or failed removal leaves the tenant served
    /// again.
    pub fn remove(&self, who: Actor, id: TenantId, removal: Removal) -> Result<(), TenantError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::TenantRemove,
            target: Some(id.to_string()),
            version: None,
            status,
        };
        let drained = match self.drain(id) {
            Ok(drained) => drained,
            Err(error) => {
                self.record_alone(&entry(503))?;
                return Err(error);
            }
        };
        let result = self.remove_drained(id, removal, drained, entry);
        if result.is_err() {
            self.stores().remove(&id);
        }
        result
    }

    fn remove_drained(
        &self,
        id: TenantId,
        removal: Removal,
        drained: Option<Store>,
        entry: impl Fn(u16) -> Entry,
    ) -> Result<(), TenantError> {
        let store = match drained {
            Some(store) => store,
            None => match self.open_store(id).map_err(TenantError::from) {
                Err(TenantError::NotFound) => {
                    self.record_alone(&entry(404))?;
                    return Err(TenantError::NotFound);
                }
                other => other?,
            },
        };
        let counts = store.counts().map_err(io)?;
        let refused = match removal {
            Removal::Empty if counts.versions > 0 => Some(TenantError::HoldsVcons),
            Removal::Force if counts.held > 0 => Some(TenantError::UnderHold),
            _ => None,
        };
        if let Some(error) = refused {
            self.record_alone(&entry(409))?;
            return Err(error);
        }
        {
            let mut conn = self.conn();
            let tx = immediate(&mut conn)?;
            tx.execute("DELETE FROM tenant WHERE id = ?1", [id.0])
                .map_err(io)?;
            tx.execute(
                "UPDATE api_key SET revoked_at = ?2 WHERE tenant = ?1 AND revoked_at IS NULL",
                params![id.0, nanos(self.clock.now())],
            )
            .map_err(io)?;
            self.record(&tx, &entry(204))?;
            tx.commit().map_err(io)?;
        }
        // The removal stands with its commit. The Store is closed before
        // its directory goes; a directory left here, by a crash or a
        // failed removal, goes at the next start.
        drop(store);
        let _ = self.fs.remove_dir_all(&self.tenants.join(id.to_string()));
        self.stores().remove(&id);
        Ok(())
    }

    /// The tenant's Store, shared by every request to it.
    pub fn store(&self, id: TenantId) -> Result<Arc<Store>, TenantError> {
        self.store_or_why(id).map_err(TenantError::from)
    }

    /// `store`, saying why a Store that did not open did not.
    pub fn store_or_why(&self, id: TenantId) -> Result<Arc<Store>, OpenFailure> {
        // The Store is opened under this lock, so it is opened once: a
        // second open's recovery could remove a write's temporary file.
        let mut stores = self.stores();
        match stores.get(&id) {
            Some(Slot::Open(store)) => return Ok(store.clone()),
            Some(Slot::Closing) => return Err(TenantError::Closing.into()),
            None => {}
        }
        let store = Arc::new(self.open_store(id)?);
        stores.insert(id, Slot::Open(store.clone()));
        Ok(store)
    }

    /// Opens the tenant's Store under the key its row holds. Its callers
    /// make sure no other Store of the tenant is open: `store` holds the
    /// lock, and `remove` has drained the tenant.
    fn open_store(&self, id: TenantId) -> Result<Store, OpenFailure> {
        let wrapped: Vec<u8> = self
            .conn()
            .query_row(
                "SELECT wrapped_tenant_key FROM tenant WHERE id = ?1",
                [id.0],
                |r| r.get(0),
            )
            .optional()
            .map_err(io)?
            .ok_or(TenantError::NotFound)?;
        let tenant_key = envelope::unwrap(&self.master, &Context::Tenant { tenant: id }, &wrapped)
            .map_err(io)?;
        #[cfg(test)]
        {
            let hook = self.hooks.opening.lock().expect("hook").take();
            if let Some(hook) = hook {
                hook();
            }
        }
        let open = if self.beside {
            Store::open_beside
        } else {
            Store::open
        };
        open(
            &self.tenants.join(id.to_string()),
            id,
            tenant_key,
            self.fs.clone(),
            self.clock.clone(),
        )
        .map_err(OpenFailure::from)
    }

    /// Marks the tenant closing, so no new handle is handed out, and waits
    /// until every handle given out before is dropped. Handles are plain
    /// `Arc`s, so the wait watches the count; requests are short and
    /// removal is rare. A tenant already closing is refused, so two
    /// removals never both hold its Store.
    ///
    /// Returns the tenant's Store if a request had opened it, now held
    /// only here.
    fn drain(&self, id: TenantId) -> Result<Option<Store>, TenantError> {
        let slot = {
            let mut stores = self.stores();
            if matches!(stores.get(&id), Some(Slot::Closing)) {
                return Err(TenantError::Closing);
            }
            stores.insert(id, Slot::Closing)
        };
        let Some(Slot::Open(mut store)) = slot else {
            return Ok(None);
        };
        loop {
            match Arc::try_unwrap(store) {
                Ok(store) => return Ok(Some(store)),
                Err(shared) => {
                    store = shared;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    fn stores(&self) -> MutexGuard<'_, HashMap<TenantId, Slot>> {
        self.stores
            .lock()
            .expect("no thread panicked holding the stores")
    }

    /// Makes every audit insert on server.sqlite fail, for tests.
    #[cfg(test)]
    pub(crate) fn break_audit(&self) {
        self.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
    }

    /// server.sqlite's connection, for tests that damage it.
    #[cfg(test)]
    pub(crate) fn conn_for_test(&self) -> MutexGuard<'_, Connection> {
        self.conn()
    }

    /// The id of the tenant's current key, for tests.
    #[cfg(test)]
    pub(crate) fn tenant_key_id_for_test(&self, id: TenantId) -> Vec<u8> {
        self.conn()
            .query_row(
                "SELECT tenant_key_id FROM tenant WHERE id = ?1",
                [id.0],
                |r| r.get(0),
            )
            .expect("the tenant's row")
    }

    /// The admin log as `export` writes it, for tests.
    #[cfg(test)]
    pub(crate) fn admin_records(&self) -> Vec<serde_json::Value> {
        crate::audit::testing::exported(&self.conn(), None)
    }

    /// Marks the tenant closing, as a removal does, for tests that need a
    /// closing tenant without removing it.
    #[cfg(test)]
    pub(crate) fn close(&self, id: TenantId) {
        self.drain(id).expect("not closing yet");
    }

    /// A tenant's own retention period, None when it uses the server
    /// default, read afresh so a change counts from the next call. It is
    /// read for the server's own use, such as a delete's purge, so nothing
    /// is recorded.
    pub fn retention(&self, id: TenantId) -> Result<Option<Retention>, TenantError> {
        let row: Option<Option<String>> = self
            .conn()
            .query_row("SELECT retention FROM tenant WHERE id = ?1", [id.0], |r| {
                r.get(0)
            })
            .optional()
            .map_err(io)?;
        stored_retention(row.ok_or(TenantError::NotFound)?)
    }

    /// Every tenant's id, read for the server's own use, such as the purge
    /// sweep, so nothing is recorded.
    pub fn ids(&self) -> Result<Vec<TenantId>, TenantError> {
        let conn = self.conn();
        let mut rows = conn.prepare("SELECT id FROM tenant").map_err(io)?;
        rows.query_map([], |r| r.get::<_, [u8; 16]>(0))
            .map_err(io)?
            .map(|row| row.map(TenantId).map_err(io))
            .collect()
    }

    /// Writes the admin log after `since` as JSON lines, oldest first.
    /// Not recorded: reading the log changes nothing.
    pub fn export_admin(
        &self,
        since: Option<i64>,
        out: &mut dyn std::io::Write,
    ) -> Result<(), audit::AuditError> {
        audit::export(&self.conn(), since, out)
    }

    /// A page of the admin log as JSON lines, the lines `export_admin`
    /// writes for the same records. The read is recorded as audit_read in
    /// the admin log, in the same transaction, after the page is read.
    pub fn audit_page(
        &self,
        who: Actor,
        query: &audit::AuditQuery,
    ) -> Result<Vec<u8>, TenantError> {
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        let mut out = Vec::new();
        let read = audit::page(&tx, query, &mut out);
        self.record(
            &tx,
            &Entry {
                actor: who,
                action: Action::AuditRead,
                target: None,
                version: None,
                status: if read.is_ok() { 200 } else { 500 },
            },
        )?;
        tx.commit().map_err(io)?;
        read.map(|()| out).map_err(io)
    }

    /// Whether server.sqlite answers a query, for the health route.
    pub fn check(&self) -> Result<(), TenantError> {
        self.conn()
            .query_row("SELECT count(*) FROM tenant", [], |_| Ok(()))
            .map_err(io)
    }

    /// Records an event in the admin log, in a transaction of its own.
    pub(crate) fn record_alone(&self, entry: &Entry) -> Result<(), TenantError> {
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        self.record(&tx, entry)?;
        tx.commit().map_err(io)
    }

    /// Counts an event in the admin log, in a transaction of its own.
    pub(crate) fn count_admin(&self, entry: &Entry) -> Result<(), TenantError> {
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        audit::count(&tx, entry, self.clock.now()).map_err(|_| TenantError::AuditFailed)?;
        tx.commit().map_err(io)
    }

    fn record(&self, tx: &Transaction, entry: &Entry) -> Result<(), TenantError> {
        audit::record(tx, entry, self.clock.now()).map_err(|_| TenantError::AuditFailed)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .expect("no thread panicked holding server.sqlite")
    }
}

/// A tenant's name is a label for people: 1 to 128 characters, none of
/// them a control character, and no white space at either end, so two
/// names that look alike are alike.
fn good_name(name: &str) -> bool {
    (1..=128).contains(&name.chars().count())
        && !name.chars().any(char::is_control)
        && name.trim() == name
}

fn name_taken(tx: &Transaction, name: &str) -> Result<bool, TenantError> {
    Ok(tx
        .query_row("SELECT 1 FROM tenant WHERE name = ?1", [name], |_| Ok(()))
        .optional()
        .map_err(io)?
        .is_some())
}

fn wraps(tx: &Transaction, key: KeyId) -> Result<u64, TenantError> {
    let wraps: Option<i64> = tx
        .query_row(
            "SELECT wraps FROM master_key_use WHERE master_key_id = ?1",
            [key.bytes()],
            |r| r.get(0),
        )
        .optional()
        .map_err(io)?;
    u64::try_from(wraps.unwrap_or(0)).map_err(io)
}

fn add_wrap(tx: &Transaction, key: KeyId, now: i64) -> Result<(), TenantError> {
    tx.execute(
        "INSERT INTO master_key_use (master_key_id, wraps, since) VALUES (?1, 1, ?2)
         ON CONFLICT (master_key_id) DO UPDATE SET wraps = wraps + 1",
        params![key.bytes(), now],
    )
    .map_err(io)?;
    Ok(())
}

fn immediate(conn: &mut Connection) -> Result<Transaction<'_>, TenantError> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(io)
}

fn io<E>(_: E) -> TenantError {
    TenantError::Io
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use serde_json::Value;
    use time::macros::datetime;

    use super::*;
    use crate::audit::testing::exported;
    use crate::catalog::testing::columns_taking_null;
    use crate::clock::ManualClock;
    use crate::fs::{CrashFs, Op};
    use crate::ids::VersionId;
    use crate::store::testing::vcon;
    use crate::store::{Caller, Created, StoreError};

    const MARKER: &str = "plain-record-marker-8d21e4";
    const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

    fn caller() -> Caller {
        Caller {
            key: Key::generate().expect("key").id(),
        }
    }

    /// What a backup runs at each try, after it copies server.sqlite.
    pub(super) type RegistryHook = Box<dyn FnMut(&Tenants) + Send>;

    #[derive(Default)]
    pub(super) struct Hooks {
        /// Runs once, between a create's directory and catalog and the
        /// transaction that commits its row.
        pub(super) between: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        /// Runs once, as `store` opens a tenant's Store.
        pub(super) opening: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        /// Stops a key rotation, as a failure, after this step.
        pub(crate) rekey_stop: Mutex<Option<super::rekey::RekeyStep>>,
        /// Runs at each try of a backup, after server.sqlite is copied and
        /// before the tenants it lists are.
        pub(super) registry_copied: Mutex<Option<RegistryHook>>,
    }

    /// A master key kept wrapped, so a test can have it more than once.
    struct Master {
        holder: Key,
        wrapped: Vec<u8>,
    }

    const CTX: Context = Context::Tenant {
        tenant: TenantId([0; 16]),
    };

    impl Master {
        fn new() -> Master {
            let holder = Key::generate().expect("key");
            let wrapped = envelope::wrap(&holder, &CTX, &Key::generate().expect("key"));
            Master { holder, wrapped }
        }

        fn key(&self) -> Key {
            envelope::unwrap(&self.holder, &CTX, &self.wrapped).expect("unwraps")
        }
    }

    pub(crate) struct Fixture {
        pub(crate) dir: tempfile::TempDir,
        pub(crate) fs: Arc<CrashFs>,
        pub(crate) clock: Arc<ManualClock>,
        master: Master,
        pub(crate) tenants: Tenants,
    }

    impl Fixture {
        fn open(&self) -> Result<Tenants, TenantError> {
            Tenants::open(
                self.dir.path(),
                self.master.key(),
                self.fs.clone(),
                self.clock.clone(),
            )
        }

        /// A crash, then a start: what was not synced is lost, and the
        /// registry is opened again.
        fn restart(&mut self) {
            self.fs.crash();
            let tenants = self.open().expect("opens again");
            drop(std::mem::replace(&mut self.tenants, tenants));
        }

        pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
            self.tenants.conn()
        }

        /// The master key the registry was made under.
        pub(crate) fn master_key(&self) -> Key {
            self.master.key()
        }

        pub(crate) fn tenants_dir(&self) -> PathBuf {
            self.dir.path().join("tenants")
        }

        /// The tenant directories, as the filesystem shows them now.
        fn dirs(&self) -> Vec<OsString> {
            self.fs.list(&self.tenants_dir()).expect("list tenants")
        }

        fn count(&self, sql: &str) -> i64 {
            self.conn().query_row(sql, [], |r| r.get(0)).expect(sql)
        }

        pub(crate) fn master_wraps(&self) -> i64 {
            self.count("SELECT coalesce(sum(wraps), 0) FROM master_key_use")
        }

        fn admin_log(&self) -> Vec<Value> {
            exported(&self.conn(), None)
        }

        /// The tenant's key, unwrapped from its row.
        pub(crate) fn tenant_key(&self, id: TenantId) -> Key {
            let wrapped: Vec<u8> = self
                .conn()
                .query_row(
                    "SELECT wrapped_tenant_key FROM tenant WHERE id = ?1",
                    [id.0],
                    |r| r.get(0),
                )
                .expect("a row");
            envelope::unwrap(
                &self.master.key(),
                &Context::Tenant { tenant: id },
                &wrapped,
            )
            .expect("unwraps under the master key")
        }

        /// The tenant's Store, opened as the registry would open it.
        pub(crate) fn store(&self, id: TenantId) -> Store {
            Store::open(
                &self.tenants_dir().join(id.to_string()),
                id,
                self.tenant_key(id),
                self.fs.clone(),
                self.clock.clone(),
            )
            .expect("the store opens")
        }

        fn has_row(&self, id: TenantId) -> bool {
            self.conn()
                .query_row("SELECT 1 FROM tenant WHERE id = ?1", [id.0], |_| Ok(()))
                .optional()
                .expect("query")
                .is_some()
        }

        /// A second connection to server.sqlite, as another writer.
        fn other(&self) -> Connection {
            let catalog_key = envelope::catalog_key(&self.master.key()).expect("catalog key");
            catalog::open(&self.dir.path().join("server.sqlite"), &catalog_key, "")
                .expect("second connection")
        }
    }

    pub(crate) fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("temp dir");
        let fs = Arc::new(CrashFs::new(dir.path()));
        let clock = Arc::new(ManualClock::at(datetime!(2026-10-23 12:00 UTC)));
        let master = Master::new();
        let tenants =
            Tenants::open(dir.path(), master.key(), fs.clone(), clock.clone()).expect("opens");
        Fixture {
            dir,
            fs,
            clock,
            master,
            tenants,
        }
    }

    #[test]
    fn server_sqlite_is_a_catalog_under_the_master_keys_catalog_key() {
        let f = fixture();
        let conn = f.other();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("names");
        assert_eq!(
            tables,
            [
                "api_key",
                "audit",
                "master_key_use",
                "sqlite_sequence",
                "tenant"
            ]
        );
    }

    #[test]
    fn an_admin_audit_page_is_export_s_lines_and_records_its_read_after_them() {
        let f = fixture();
        for name in ["acme", "beta"] {
            f.tenants.create(Actor::Cli, name, None).expect("made");
        }
        let mut before = Vec::new();
        f.tenants.export_admin(None, &mut before).expect("export");
        let who = Actor::Key(Key::generate().expect("key").id());
        let query = |after, limit| audit::AuditQuery { after, limit };
        assert_eq!(
            f.tenants.audit_page(who, &query(0, 1000)),
            Ok(before.clone())
        );
        let read = exported(&f.conn(), None);
        let last = read.last().expect("records");
        assert_eq!(last["action"], "audit_read");
        assert_eq!(last["status"], 200);
        let lines: Vec<&[u8]> = before.split_inclusive(|b| *b == b'\n').collect();
        assert_eq!(
            f.tenants.audit_page(who, &query(1, 1)),
            Ok(lines[1].to_vec())
        );
        f.conn()
            .execute(
                "INSERT INTO audit (at, actor, action, status, count)
                 VALUES ('2026-10-23T12:00:00Z', 'cli', 'read', 70000, 1)",
                [],
            )
            .expect("damaged");
        assert_eq!(
            f.tenants.audit_page(who, &query(0, 1000)),
            Err(TenantError::Io)
        );
        let last = exported_last(&f);
        assert_eq!(last["action"], "audit_read");
        assert_eq!(last["status"], 500);
        f.tenants.break_audit();
        assert_eq!(
            f.tenants.audit_page(who, &query(0, 1000)),
            Err(TenantError::AuditFailed)
        );
    }

    /// The admin log's last record, read past a damaged one.
    fn exported_last(f: &Fixture) -> Value {
        let conn = f.conn();
        let (action, status): (String, i64) = conn
            .query_row(
                "SELECT action, status FROM audit ORDER BY seq DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("a record");
        serde_json::json!({"action": action, "status": status})
    }

    #[test]
    fn a_wrong_master_key_does_not_open_server_sqlite() {
        let f = fixture();
        let wrong = Tenants::open(
            f.dir.path(),
            Master::new().key(),
            f.fs.clone(),
            f.clock.clone(),
        );
        assert_eq!(wrong.err(), Some(TenantError::WrongKey));
        assert!(f.open().is_ok(), "the right key still opens it");
    }

    #[test]
    fn only_the_optional_columns_may_be_empty() {
        let f = fixture();
        let conn = f.conn();
        let tenant = [
            ("id", "x'11'"),
            ("name", "'a'"),
            ("retention", "'7y'"),
            ("created_at", "1"),
            ("tenant_key_id", "x'22'"),
            ("wrapped_tenant_key", "x'33'"),
            ("pending_wrapped_tenant_key", "x'44'"),
            ("key_since", "1"),
        ];
        let key_use = [("master_key_id", "x'55'"), ("wraps", "1"), ("since", "1")];
        let api_key = [
            ("id", "x'66'"),
            ("tenant", "x'11'"),
            ("token_hash", "x'77'"),
            ("scopes", "'read'"),
            ("label", "'a'"),
            ("created_at", "1"),
            ("revoked_at", "2"),
        ];
        let mut may_be_empty = columns_taking_null(&conn, "tenant", &tenant);
        may_be_empty.extend(columns_taking_null(&conn, "master_key_use", &key_use));
        may_be_empty.extend(columns_taking_null(&conn, "api_key", &api_key));
        assert_eq!(
            may_be_empty,
            [
                "tenant.retention",
                "tenant.pending_wrapped_tenant_key",
                "api_key.tenant",
                "api_key.revoked_at"
            ]
        );
    }

    #[test]
    fn a_name_belongs_to_one_tenant() {
        let f = fixture();
        let conn = f.conn();
        let add = |id: u8, name: &str| {
            conn.execute(
                "INSERT INTO tenant (id, name, created_at, tenant_key_id,
                     wrapped_tenant_key, key_since)
                 VALUES (?1, ?2, 1, x'22', x'33', 1)",
                params![[id; 16], name],
            )
        };
        add(1, "a").expect("the first tenant");
        add(2, "b").expect("another name");
        assert!(add(3, "a").is_err(), "a second tenant named a");
    }

    /// Run while the connection is open, so the commits are still in the
    /// WAL.
    #[test]
    fn no_plain_record_reaches_server_sqlite_or_its_wal() {
        let f = fixture();
        f.tenants
            .create(Actor::Cli, MARKER, Some(Retention::Years(7)))
            .expect("a tenant");
        let wal = std::fs::metadata(f.dir.path().join("server.sqlite-wal")).expect("a WAL");
        assert!(wal.len() > 0, "the commits are in the WAL");
        for entry in std::fs::read_dir(f.dir.path()).expect("list") {
            let file = entry.expect("entry").path();
            if !file.is_file() {
                continue;
            }
            let bytes = std::fs::read(&file).expect("read");
            assert!(
                !bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
                "plain record in {}",
                file.display()
            );
        }
    }

    #[test]
    fn a_new_registrys_tenants_directory_survives_a_crash() {
        let f = fixture();
        f.fs.crash();
        assert_eq!(
            f.fs.list(f.dir.path()).expect("list"),
            [OsString::from("tenants")]
        );
    }

    /// A tenant row's columns, besides its id and wrapped key.
    struct Row {
        name: String,
        retention: Option<String>,
        created_at: i64,
        key_id: Vec<u8>,
        pending: Option<Vec<u8>>,
        key_since: i64,
    }

    #[test]
    fn a_tenant_is_made_with_its_key_directory_catalog_and_row() {
        let f = fixture();
        let id = f
            .tenants
            .create(Actor::Cli, "acme", Some(Retention::Years(7)))
            .expect("made");
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        let other = f.tenants.create(Actor::Cli, "lab", None).expect("another");
        assert_ne!(id, other, "a new id each time");

        let Row {
            name,
            retention,
            created_at,
            key_id,
            pending,
            key_since,
        } = f
            .conn()
            .query_row(
                "SELECT name, retention, created_at, tenant_key_id,
                     pending_wrapped_tenant_key, key_since
                 FROM tenant WHERE id = ?1",
                [id.0],
                |r| {
                    Ok(Row {
                        name: r.get(0)?,
                        retention: r.get(1)?,
                        created_at: r.get(2)?,
                        key_id: r.get(3)?,
                        pending: r.get(4)?,
                        key_since: r.get(5)?,
                    })
                },
            )
            .expect("a row");
        let now = nanos(datetime!(2026-10-23 12:00 UTC));
        assert_eq!(name, "acme");
        assert_eq!(retention.as_deref(), Some("7y"));
        assert_eq!((created_at, key_since), (now, now));
        assert_eq!(pending, None);
        let key = f.tenant_key(id);
        assert_eq!(key_id, key.id().bytes());
        let lab_retention: Option<String> = f
            .conn()
            .query_row(
                "SELECT retention FROM tenant WHERE id = ?1",
                [other.0],
                |r| r.get(0),
            )
            .expect("a row");
        assert_eq!(lab_retention, None, "no period means the server default");

        let mut dirs = vec![
            OsString::from(id.to_string()),
            OsString::from(other.to_string()),
        ];
        dirs.sort();
        assert_eq!(f.dirs(), dirs);
        drop(f.store(id));
        let catalog_key = envelope::catalog_key(&key).expect("catalog key");
        let tenant_dir = f.tenants_dir().join(id.to_string());
        assert!(
            catalog::open(&tenant_dir.join("catalog.sqlite"), &catalog_key, "").is_ok(),
            "the catalog is under the tenant key's catalog key"
        );

        let (wraps, since): (i64, i64) = f
            .conn()
            .query_row(
                "SELECT wraps, since FROM master_key_use WHERE master_key_id = ?1",
                [f.master.key().id().bytes()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the master key's use");
        assert_eq!((wraps, since), (2, now));

        let log = f.admin_log();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0]["actor"], "cli");
        assert_eq!(log[0]["action"], "tenant_create");
        assert_eq!(log[0]["target"], id.to_string());
        assert_eq!(log[0]["status"], 201);
        assert_eq!(log[0]["at"], "2026-10-23T12:00:00Z");
    }

    #[test]
    fn a_taken_name_is_refused_before_anything_is_made() {
        let f = fixture();
        f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let ops = f.fs.log().len();
        assert_eq!(
            f.tenants.create(Actor::Cli, "acme", None),
            Err(TenantError::NameTaken)
        );
        assert_eq!(f.fs.log().len(), ops, "nothing made");
        assert_eq!(f.master_wraps(), 1, "nothing wrapped");
        assert_eq!(f.count("SELECT count(*) FROM tenant"), 1);
        let log = f.admin_log();
        assert_eq!(log[1]["action"], "tenant_create");
        assert_eq!(log[1]["status"], 409);
        assert_eq!(log[1]["target"], Value::Null, "no tenant was made");
    }

    #[test]
    fn a_name_is_1_to_128_characters_without_controls_or_outer_spaces() {
        let long = "é".repeat(128);
        for name in ["acme", "a", "Acme Corp", "Acme — Europe", long.as_str()] {
            assert!(good_name(name), "{name:?}");
        }
        let longer = "é".repeat(129);
        for name in [
            "",
            " acme",
            "acme ",
            "ac\nme",
            "ac\tme",
            "acme\u{7f}",
            "\u{85}acme",
            "acme\u{3000}",
            longer.as_str(),
        ] {
            assert!(!good_name(name), "{name:?}");
        }
    }

    #[test]
    fn a_bad_name_is_refused_and_recorded_before_anything_is_made() {
        let f = fixture();
        let ops = f.fs.log().len();
        assert_eq!(
            f.tenants.create(Actor::Cli, " acme", None),
            Err(TenantError::BadName)
        );
        assert_eq!(f.fs.log().len(), ops, "nothing made");
        assert_eq!(f.master_wraps(), 0, "nothing wrapped");
        assert_eq!(f.count("SELECT count(*) FROM tenant"), 0);
        let log = f.admin_records("tenant_create");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0]["status"], 422);
        assert_eq!(log[0]["target"], Value::Null, "no tenant was made");
    }

    #[test]
    fn a_master_key_at_its_wrap_limit_makes_no_tenant() {
        let f = fixture();
        f.conn()
            .execute(
                "INSERT INTO master_key_use (master_key_id, wraps, since) VALUES (?1, ?2, 1)",
                params![f.master.key().id().bytes(), 1_i64 << 32],
            )
            .expect("at the limit");
        let ops = f.fs.log().len();
        assert_eq!(
            f.tenants.create(Actor::Cli, "acme", None),
            Err(TenantError::LimitReached)
        );
        assert_eq!(f.fs.log().len(), ops, "nothing made");
        assert_eq!(f.master_wraps(), 1_i64 << 32, "nothing counted");
        assert_eq!(f.count("SELECT count(*) FROM tenant"), 0);
        assert_eq!(f.admin_log()[0]["status"], 503);
    }

    #[test]
    fn a_negative_wrap_count_is_damage_and_makes_no_tenant() {
        let f = fixture();
        f.conn()
            .execute(
                "INSERT INTO master_key_use (master_key_id, wraps, since) VALUES (?1, -1, 1)",
                [f.master.key().id().bytes()],
            )
            .expect("a damaged count");
        let ops = f.fs.log().len();
        assert_eq!(
            f.tenants.create(Actor::Cli, "acme", None),
            Err(TenantError::Io)
        );
        assert_eq!(f.fs.log().len(), ops, "nothing made");
    }

    #[test]
    fn a_name_taken_while_the_directory_is_made_loses_and_leaves_nothing() {
        let f = fixture();
        let other = f.other();
        *f.tenants.hooks.between.lock().expect("hook") = Some(Box::new(move || {
            other
                .execute(
                    "INSERT INTO tenant (id, name, created_at, tenant_key_id,
                         wrapped_tenant_key, key_since)
                     VALUES (x'01', 'acme', 1, x'02', x'03', 1)",
                    [],
                )
                .expect("another create wins the name");
        }));
        assert_eq!(
            f.tenants.create(Actor::Cli, "acme", None),
            Err(TenantError::NameTaken)
        );
        assert_eq!(f.dirs(), Vec::<OsString>::new(), "its directory is removed");
        assert_eq!(f.master_wraps(), 1, "its wrap is still counted");
        let log = f.admin_log();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0]["status"], 409);
    }

    #[test]
    fn a_create_whose_audit_record_fails_leaves_nothing() {
        let f = fixture();
        f.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants.create(Actor::Cli, "acme", None),
            Err(TenantError::AuditFailed)
        );
        assert_eq!(f.count("SELECT count(*) FROM tenant"), 0);
        assert_eq!(f.dirs(), Vec::<OsString>::new(), "its directory is removed");
    }

    #[test]
    fn a_create_whose_row_cannot_be_written_leaves_nothing() {
        let f = fixture();
        f.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_row BEFORE INSERT ON tenant
                 BEGIN SELECT RAISE(ABORT, 'the disk has failed'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants.create(Actor::Cli, "acme", None),
            Err(TenantError::Io)
        );
        assert_eq!(f.count("SELECT count(*) FROM tenant"), 0);
        assert_eq!(f.dirs(), Vec::<OsString>::new(), "its directory is removed");
    }

    #[test]
    fn a_refusal_whose_audit_record_fails_is_an_audit_failure() {
        let f = fixture();
        f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants.create(Actor::Cli, "acme", None),
            Err(TenantError::AuditFailed)
        );
    }

    /// The steps of a create that the filesystem sees: the tenant's
    /// directory, its objects directory, the sync of the tenant's directory
    /// and the sync of `tenants/`. The row's commit follows them.
    const CREATE_STEPS: usize = 4;

    #[test]
    fn a_create_writes_its_directories_in_order() {
        let f = fixture();
        let ops = f.fs.log().len();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let dir = f.tenants_dir().join(id.to_string());
        assert_eq!(
            f.fs.log()[ops..],
            [
                Op::CreateDir(dir.clone()),
                Op::CreateDir(dir.join("objects")),
                Op::SyncDir(dir),
                Op::SyncDir(f.tenants_dir()),
            ]
        );
        assert_eq!(f.fs.log().len() - ops, CREATE_STEPS);
    }

    #[test]
    fn a_create_cut_off_at_any_step_leaves_a_whole_tenant_or_none() {
        for step in 0..=CREATE_STEPS {
            let mut f = fixture();
            f.fs.fail_from(f.fs.log().len() + step);
            let result = f.tenants.create(Actor::Cli, "acme", None);
            assert_eq!(
                result.is_ok(),
                step == CREATE_STEPS,
                "step {step}: {result:?}"
            );
            f.restart();
            match result {
                Ok(id) => {
                    assert!(f.has_row(id), "step {step}");
                    assert_eq!(f.dirs(), [OsString::from(id.to_string())]);
                    let store = f.store(id);
                    drop(store);
                }
                Err(_) => {
                    assert_eq!(f.count("SELECT count(*) FROM tenant"), 0, "step {step}");
                    assert_eq!(f.dirs(), Vec::<OsString>::new(), "step {step}");
                }
            }
        }
    }

    #[test]
    fn a_create_cut_off_before_its_commit_leaves_no_tenant() {
        let mut f = fixture();
        let other = f.other();
        let fs = f.fs.clone();
        *f.tenants.hooks.between.lock().expect("hook") = Some(Box::new(move || {
            fs.fail_from(0);
            other
                .execute_batch(
                    "CREATE TRIGGER dead BEFORE INSERT ON audit
                     BEGIN SELECT RAISE(ABORT, 'the process has died'); END;",
                )
                .expect("trigger");
        }));
        assert!(f.tenants.create(Actor::Cli, "acme", None).is_err());
        assert_eq!(f.dirs().len(), 1, "the dead process removed nothing");
        f.conn()
            .execute_batch("DROP TRIGGER dead")
            .expect("drop trigger");
        f.restart();
        assert_eq!(f.count("SELECT count(*) FROM tenant"), 0);
        assert_eq!(f.dirs(), Vec::<OsString>::new(), "removed at start");
    }

    #[test]
    fn opening_removes_only_tenant_directories_with_no_row() {
        let mut f = fixture();
        let kept = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let orphan = TenantId::generate();
        let tenants = f.tenants_dir();
        for name in [orphan.to_string(), "notes".to_owned()] {
            f.fs.create_dir_all(&tenants.join(name))
                .expect("a directory");
        }
        f.fs.sync_dir(&tenants).expect("sync");
        f.restart();
        assert_eq!(
            f.dirs(),
            [OsString::from(kept.to_string()), OsString::from("notes")]
        );
        drop(f.store(kept));
    }

    /// The server's registry opens each tenant's Store with recovery, so a
    /// crash's leftovers in it go once the tenant is next used.
    #[test]
    fn a_servers_registry_opens_each_store_with_recovery() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let objects = f.tenants_dir().join(id.to_string()).join("objects");
        let leftover = format!("{}.tmp", VersionId([0x0e; 16]));
        f.fs.write_new(&objects.join(&leftover), b"x")
            .expect("write");
        let tenants = f.open().expect("opens again");
        drop(tenants.store(id).expect("its store"));
        assert!(
            !f.fs
                .list(&objects)
                .expect("list")
                .contains(&OsString::from(leftover)),
            "the leftover is removed"
        );
    }

    /// A command beside a running server must not take a create or a
    /// write the server has in progress for a crash's leftovers, in the
    /// registry or in a tenant's store.
    #[test]
    fn opening_beside_a_server_removes_nothing() {
        let f = fixture();
        let kept = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let orphan = TenantId::generate();
        let tenants = f.tenants_dir();
        f.fs.create_dir_all(&tenants.join(orphan.to_string()))
            .expect("a directory");
        f.fs.sync_dir(&tenants).expect("sync");
        let in_progress = tenants
            .join(kept.to_string())
            .join("objects")
            .join(format!("{}.tmp", VersionId([0x0e; 16])));
        f.fs.write_new(&in_progress, b"x").expect("write");
        let beside =
            Tenants::open_beside(f.dir.path(), f.master.key(), f.fs.clone(), f.clock.clone())
                .expect("opens beside");
        let mut expected = [
            OsString::from(kept.to_string()),
            OsString::from(orphan.to_string()),
        ];
        expected.sort();
        assert_eq!(f.dirs(), expected);
        drop(beside.store(kept).expect("its store"));
        assert!(
            f.fs.list(in_progress.parent().expect("objects"))
                .expect("list")
                .contains(&OsString::from(in_progress.file_name().expect("name"))),
            "the write in progress stays"
        );
    }

    #[test]
    fn an_unknown_tenant_has_no_store() {
        let f = fixture();
        assert_eq!(
            f.tenants.store(TenantId::generate()).err(),
            Some(TenantError::NotFound)
        );
    }

    #[test]
    fn every_request_to_a_tenant_shares_one_store() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let a = f.tenants.store(id).expect("a store");
        let b = f.tenants.store(id).expect("the same store");
        assert!(Arc::ptr_eq(&a, &b));
        let (found, body) = vcon(UUID, "first");
        a.create(&caller(), &found, &body).expect("stored");
        assert_eq!(b.get(&caller(), found.uuid).expect("read").body, body);
    }

    /// The first request opens the Store; one arriving meanwhile waits for
    /// it rather than opening a second.
    #[test]
    fn a_store_is_opened_once_even_by_requests_at_the_same_time() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let (opening, is_opening) = std::sync::mpsc::channel();
        let (go, wait) = std::sync::mpsc::channel::<()>();
        *f.tenants.hooks.opening.lock().expect("hook") = Some(Box::new(move || {
            opening.send(()).expect("send");
            wait.recv().expect("go");
        }));
        // Everything the threads wait on is moved into the scope, so a
        // failed assertion drops it and the threads end instead of hanging.
        let tenants = &f.tenants;
        std::thread::scope(move |s| {
            let first = s.spawn(move || tenants.store(id).expect("the first"));
            is_opening
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("the first is opening");
            let (second_done, second) = std::sync::mpsc::channel();
            s.spawn(move || {
                second_done
                    .send(tenants.store(id).expect("the second"))
                    .expect("send");
            });
            assert!(
                second
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .is_err(),
                "the second waits while the first opens"
            );
            go.send(()).expect("go");
            let first = first.join().expect("the first returns");
            let second = second.recv().expect("the second returns");
            assert!(Arc::ptr_eq(&first, &second));
        });
    }

    #[test]
    fn a_tenant_key_that_does_not_unwrap_opens_no_store() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.conn()
            .execute(
                "UPDATE tenant SET wrapped_tenant_key = x'00' WHERE id = ?1",
                [id.0],
            )
            .expect("damage the wrapped key");
        assert_eq!(f.tenants.store(id).err(), Some(TenantError::Io));
    }

    #[test]
    fn draining_refuses_new_requests_and_waits_for_those_in_flight() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let (found, body) = vcon(UUID, "first");
        let held = f.tenants.store(id).expect("a request in flight");
        std::thread::scope(|s| {
            let (done, drained) = std::sync::mpsc::channel();
            let tenants = &f.tenants;
            s.spawn(move || {
                tenants.drain(id).expect("drains");
                done.send(()).expect("send");
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while f.tenants.store(id).is_ok() {
                assert!(std::time::Instant::now() < deadline, "never closing");
                std::thread::yield_now();
            }
            assert_eq!(f.tenants.store(id).err(), Some(TenantError::Closing));
            assert!(
                drained
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .is_err(),
                "the drain waits for the request in flight"
            );
            held.create(&caller(), &found, &body)
                .expect("the request in flight finishes");
            drop(held);
            drained
                .recv()
                .expect("the drain ends once the last handle is dropped");
        });
        assert_eq!(f.tenants.store(id).err(), Some(TenantError::Closing));
    }

    #[test]
    fn a_tenant_no_request_opened_drains_at_once() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.tenants.drain(id).expect("drains");
        assert_eq!(f.tenants.store(id).err(), Some(TenantError::Closing));
    }

    impl Fixture {
        /// The tenant's own retention period, as its row holds it.
        fn own_retention(&self, id: TenantId) -> Option<String> {
            self.conn()
                .query_row("SELECT retention FROM tenant WHERE id = ?1", [id.0], |r| {
                    r.get(0)
                })
                .expect("a row")
        }

        /// A second connection to the tenant's catalog.
        pub(crate) fn tenant_catalog(&self, id: TenantId) -> Connection {
            let catalog_key = envelope::catalog_key(&self.tenant_key(id)).expect("catalog key");
            catalog::open(
                &self
                    .tenants_dir()
                    .join(id.to_string())
                    .join("catalog.sqlite"),
                &catalog_key,
                "",
            )
            .expect("the tenant's catalog")
        }

        /// The records with this action in the admin log.
        pub(crate) fn admin_records(&self, action: &str) -> Vec<Value> {
            self.admin_log()
                .into_iter()
                .filter(|r| r["action"] == action)
                .collect()
        }
    }

    #[test]
    fn retention_falls_back_to_the_server_default_when_unset() {
        let info = |retention| TenantInfo {
            id: TenantId([1; 16]),
            name: "acme".to_owned(),
            retention,
            created_at: datetime!(2026-10-23 12:00 UTC),
        };
        assert_eq!(
            info(None).kept_for(Retention::Years(7)),
            Retention::Years(7)
        );
        assert_eq!(
            info(Some(Retention::Days(90))).kept_for(Retention::Years(7)),
            Retention::Days(90)
        );
    }

    #[test]
    fn a_tenant_retention_is_read_afresh_and_not_recorded() {
        let f = fixture();
        let lab = f
            .tenants
            .create(Actor::Cli, "lab", Some(Retention::Months(18)))
            .expect("made");
        let acme = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let records = f.tenants.admin_records().len();

        assert_eq!(f.tenants.retention(lab), Ok(Some(Retention::Months(18))));
        assert_eq!(f.tenants.retention(acme), Ok(None));
        f.tenants
            .set_retention(Actor::Cli, acme, Some(Retention::Days(0)))
            .expect("changed");
        assert_eq!(f.tenants.retention(acme), Ok(Some(Retention::Days(0))));
        assert_eq!(
            f.tenants.retention(TenantId([0xee; 16])),
            Err(TenantError::NotFound)
        );
        assert_eq!(
            f.tenants.admin_records().len(),
            records + 1,
            "only the retention change is recorded"
        );
        f.tenants
            .conn_for_test()
            .execute("UPDATE tenant SET retention = '7w' WHERE id = ?1", [lab.0])
            .expect("damaged");
        assert_eq!(
            f.tenants.retention(lab),
            Err(TenantError::Io),
            "a period that does not read back is damage"
        );
    }

    #[test]
    fn ids_names_every_tenant_and_records_nothing() {
        let f = fixture();
        let a = f.tenants.create(Actor::Cli, "a", None).expect("made");
        let b = f.tenants.create(Actor::Cli, "b", None).expect("made");
        let records = f.tenants.admin_records().len();

        let mut ids = f.tenants.ids().expect("ids");
        ids.sort_by_key(|id| id.0);
        let mut made = vec![a, b];
        made.sort_by_key(|id| id.0);

        assert_eq!(ids, made);
        assert_eq!(f.tenants.admin_records().len(), records, "nothing recorded");
    }

    #[test]
    fn list_shows_every_tenant_by_name_and_is_recorded() {
        let f = fixture();
        assert_eq!(f.tenants.list(Actor::Cli), Ok(Vec::new()));
        let lab = f
            .tenants
            .create(Actor::Cli, "lab", Some(Retention::Days(90)))
            .expect("made");
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        let acme = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        assert_eq!(
            f.tenants.list(Actor::Cli),
            Ok(vec![
                TenantInfo {
                    id: acme,
                    name: "acme".to_owned(),
                    retention: None,
                    created_at: datetime!(2026-10-23 13:00 UTC),
                },
                TenantInfo {
                    id: lab,
                    name: "lab".to_owned(),
                    retention: Some(Retention::Days(90)),
                    created_at: datetime!(2026-10-23 12:00 UTC),
                },
            ])
        );
        let lists = f.admin_records("tenant_list");
        assert_eq!(lists.len(), 2, "one record per list");
        for record in lists {
            assert_eq!(record["actor"], "cli");
            assert_eq!(record["status"], 200);
            assert_eq!(record["target"], Value::Null);
        }
    }

    #[test]
    fn a_stored_period_that_does_not_read_back_is_damage() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.conn()
            .execute("UPDATE tenant SET retention = '7w' WHERE id = ?1", [id.0])
            .expect("damage the period");
        assert_eq!(f.tenants.list(Actor::Cli), Err(TenantError::Io));
    }

    #[test]
    fn a_list_whose_record_fails_shows_nothing() {
        let f = fixture();
        f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(f.tenants.list(Actor::Cli), Err(TenantError::AuditFailed));
    }

    #[test]
    fn a_retention_change_is_kept_and_recorded_in_both_logs() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        f.tenants
            .set_retention(Actor::Cli, id, Some(Retention::Months(18)))
            .expect("set");
        assert_eq!(f.own_retention(id).as_deref(), Some("18m"));
        f.tenants
            .set_retention(Actor::Cli, id, None)
            .expect("back to the default");
        assert_eq!(f.own_retention(id), None);

        let admin = f.admin_records("tenant_retention");
        assert_eq!(admin.len(), 2);
        for record in &admin {
            assert_eq!(record["actor"], "cli");
            assert_eq!(record["target"], id.to_string());
            assert_eq!(record["status"], 200);
            assert_eq!(record["at"], "2026-10-23T13:00:00Z");
        }
        let copies: Vec<Value> = exported(&f.tenant_catalog(id), None)
            .into_iter()
            .filter(|r| r["action"] == "retention")
            .collect();
        assert_eq!(copies.len(), 2, "a copy in the tenant's log for each");
        for record in &copies {
            assert_eq!(record["actor"], "cli");
            assert_eq!(record["target"], id.to_string());
            assert_eq!(record["status"], 200);
        }
    }

    #[test]
    fn a_retention_change_for_an_unknown_tenant_is_not_found() {
        let f = fixture();
        let id = TenantId::generate();
        assert_eq!(
            f.tenants
                .set_retention(Actor::Cli, id, Some(Retention::Years(1))),
            Err(TenantError::NotFound)
        );
        let admin = f.admin_records("tenant_retention");
        assert_eq!(admin.len(), 1);
        assert_eq!(admin[0]["status"], 404);
        assert_eq!(admin[0]["target"], id.to_string());
    }

    #[test]
    fn a_retention_change_for_a_closing_tenant_is_refused() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.tenants.drain(id).expect("drains");
        assert_eq!(
            f.tenants
                .set_retention(Actor::Cli, id, Some(Retention::Years(1))),
            Err(TenantError::Closing)
        );
        assert_eq!(f.own_retention(id), None, "unchanged");
        assert_eq!(f.admin_records("tenant_retention")[0]["status"], 503);
    }

    #[test]
    fn a_retention_change_whose_admin_record_fails_changes_nothing() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants
                .set_retention(Actor::Cli, id, Some(Retention::Years(1))),
            Err(TenantError::AuditFailed)
        );
        assert_eq!(f.own_retention(id), None, "unchanged");
        let copies = exported(&f.tenant_catalog(id), None)
            .into_iter()
            .filter(|r| r["action"] == "retention")
            .count();
        assert_eq!(copies, 0, "no copy of a change that was not made");
    }

    /// The change commits with its admin record; a failed copy in the
    /// tenant's log leaves the change standing and answers AuditFailed.
    #[test]
    fn a_retention_change_whose_tenant_copy_fails_stands() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.tenant_catalog(id)
            .execute_batch(
                "CREATE TRIGGER dead BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants
                .set_retention(Actor::Cli, id, Some(Retention::Years(10))),
            Err(TenantError::AuditFailed)
        );
        assert_eq!(
            f.own_retention(id).as_deref(),
            Some("10y"),
            "the change stands"
        );
        assert_eq!(f.admin_records("tenant_retention")[0]["status"], 200);
    }

    /// A tenant with one vCon stored.
    fn with_vcon(f: &Fixture) -> TenantId {
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let (found, body) = vcon(UUID, "first");
        f.tenants
            .store(id)
            .expect("its store")
            .create(&caller(), &found, &body)
            .expect("stored");
        id
    }

    fn hold(f: &Fixture, id: TenantId) {
        f.tenant_catalog(id)
            .execute("UPDATE vcon SET hold = 1", [])
            .expect("a hold");
    }

    /// Waits until the tenant refuses new requests.
    fn until_closing(f: &Fixture, id: TenantId) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while f.tenants.store(id).is_ok() {
            assert!(std::time::Instant::now() < deadline, "never closing");
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_tenant_already_closing_cannot_be_drained_again() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.tenants.drain(id).expect("drains");
        assert_eq!(f.tenants.drain(id).err(), Some(TenantError::Closing));
    }

    #[test]
    fn removing_an_unknown_tenant_is_not_found() {
        let f = fixture();
        let id = TenantId::generate();
        assert_eq!(
            f.tenants.remove(Actor::Cli, id, Removal::Empty),
            Err(TenantError::NotFound)
        );
        assert_eq!(f.tenants.store(id).err(), Some(TenantError::NotFound));
        let records = f.admin_records("tenant_remove");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["status"], 404);
        assert_eq!(records[0]["target"], id.to_string());
    }

    #[test]
    fn an_empty_tenant_is_removed_with_its_key_directory_and_row() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        drop(f.tenants.store(id).expect("opened by a request"));
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        f.tenants
            .remove(Actor::Cli, id, Removal::Empty)
            .expect("removed");
        assert!(!f.has_row(id), "its row and wrapped key are gone");
        assert_eq!(f.tenants.store(id).err(), Some(TenantError::NotFound));
        assert_eq!(f.dirs(), Vec::<OsString>::new(), "its directory is gone");
        assert_eq!(f.tenants.list(Actor::Cli), Ok(Vec::new()));
        let records = f.admin_records("tenant_remove");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["actor"], "cli");
        assert_eq!(records[0]["target"], id.to_string());
        assert_eq!(records[0]["status"], 204);
        assert_eq!(records[0]["at"], "2026-10-23T13:00:00Z");
    }

    #[test]
    fn an_empty_removal_is_refused_while_any_version_is_kept() {
        let f = fixture();
        let id = with_vcon(&f);
        let uuid = vcon(UUID, "first").0.uuid;
        f.tenants
            .store(id)
            .expect("its store")
            .delete(&caller(), uuid, Retention::Years(7))
            .expect("deleted, so only a hidden version is kept");
        assert_eq!(
            f.tenants.remove(Actor::Cli, id, Removal::Empty),
            Err(TenantError::HoldsVcons)
        );
        assert!(f.has_row(id));
        let store = f.tenants.store(id).expect("still served");
        assert_eq!(store.counts().map(|c| c.versions), Ok(1));
        assert_eq!(f.admin_records("tenant_remove")[0]["status"], 409);
    }

    #[test]
    fn a_forced_removal_takes_the_vcons_too() {
        let f = fixture();
        let id = with_vcon(&f);
        f.tenants
            .remove(Actor::Cli, id, Removal::Force)
            .expect("removed");
        assert!(!f.has_row(id));
        assert_eq!(f.dirs(), Vec::<OsString>::new());
    }

    #[test]
    fn a_forced_removal_is_refused_while_a_uuid_is_held() {
        let f = fixture();
        let id = with_vcon(&f);
        hold(&f, id);
        assert_eq!(
            f.tenants.remove(Actor::Cli, id, Removal::Force),
            Err(TenantError::UnderHold)
        );
        assert!(f.has_row(id));
        assert!(f.tenants.store(id).is_ok(), "still served");
        assert_eq!(f.admin_records("tenant_remove")[0]["status"], 409);
    }

    #[test]
    fn overriding_holds_removes_a_held_tenant() {
        let f = fixture();
        let id = with_vcon(&f);
        hold(&f, id);
        f.tenants
            .remove(Actor::Cli, id, Removal::ForceOverridingHolds)
            .expect("removed");
        assert!(!f.has_row(id));
        assert_eq!(f.dirs(), Vec::<OsString>::new());
    }

    /// The removal drains before it looks, so a write that lands meanwhile
    /// makes the tenant non-empty and the removal is refused: the write
    /// is never acknowledged and then lost.
    #[test]
    fn a_write_in_flight_during_an_empty_removal_is_kept() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let (found, body) = vcon(UUID, "first");
        let held = f.tenants.store(id).expect("a request in flight");
        let tenants = &f.tenants;
        let fx = &f;
        let (found_ref, body_ref) = (&found, &body);
        let result = std::thread::scope(move |s| {
            let (done, removed) = std::sync::mpsc::channel();
            s.spawn(move || {
                done.send(tenants.remove(Actor::Cli, id, Removal::Empty))
                    .expect("send");
            });
            until_closing(fx, id);
            assert!(
                removed
                    .recv_timeout(std::time::Duration::from_millis(100))
                    .is_err(),
                "the removal waits for the write"
            );
            held.create(&caller(), found_ref, body_ref)
                .expect("the write is acknowledged");
            drop(held);
            removed.recv().expect("the removal ends")
        });
        assert_eq!(result, Err(TenantError::HoldsVcons));
        assert_eq!(
            f.tenants
                .store(id)
                .expect("still served")
                .get(&caller(), found.uuid)
                .expect("the write is kept")
                .body,
            body
        );
    }

    #[test]
    fn a_second_removal_while_one_drains_is_refused() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let held = f.tenants.store(id).expect("a request in flight");
        let tenants = &f.tenants;
        let fx = &f;
        std::thread::scope(move |s| {
            let first = s.spawn(move || tenants.remove(Actor::Cli, id, Removal::Force));
            until_closing(fx, id);
            assert_eq!(
                fx.tenants.remove(Actor::Cli, id, Removal::Empty),
                Err(TenantError::Closing)
            );
            drop(held);
            assert_eq!(first.join().expect("the first returns"), Ok(()));
        });
        let statuses: Vec<Value> = f
            .admin_records("tenant_remove")
            .into_iter()
            .map(|r| r["status"].clone())
            .collect();
        assert_eq!(statuses, [503, 204]);
    }

    #[test]
    fn a_removal_whose_record_fails_removes_nothing() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        f.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.tenants.remove(Actor::Cli, id, Removal::Empty),
            Err(TenantError::AuditFailed)
        );
        assert!(f.has_row(id));
        assert_eq!(f.dirs().len(), 1);
        assert!(f.tenants.store(id).is_ok(), "served again");
    }

    /// The row's commit is the removal; a directory a crash leaves after
    /// it goes at the next start.
    #[test]
    fn a_removal_cut_off_after_its_commit_finishes_at_the_next_start() {
        let mut f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        drop(f.tenants.store(id).expect("opened by a request"));
        f.fs.fail_from(f.fs.log().len());
        assert_eq!(f.tenants.remove(Actor::Cli, id, Removal::Empty), Ok(()));
        assert_eq!(f.dirs().len(), 1, "the dead process removed nothing");
        f.restart();
        assert!(!f.has_row(id));
        assert_eq!(f.dirs(), Vec::<OsString>::new());
    }

    #[test]
    fn removal_deletes_the_directory_on_disk() {
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = Tenants::open(
            dir.path(),
            Master::new().key(),
            Arc::new(crate::fs::OsFs),
            Arc::new(ManualClock::at(datetime!(2026-10-23 12:00 UTC))),
        )
        .expect("opens");
        let id = tenants.create(Actor::Cli, "acme", None).expect("made");
        let tenant_dir = dir.path().join("tenants").join(id.to_string());
        assert!(tenant_dir.join("catalog.sqlite").is_file());
        tenants
            .remove(Actor::Cli, id, Removal::Empty)
            .expect("removed");
        assert!(!tenant_dir.exists());
    }

    #[test]
    fn two_tenants_keep_the_same_uuid_apart() {
        let f = fixture();
        let a = f.tenants.create(Actor::Cli, "a", None).expect("made");
        let b = f.tenants.create(Actor::Cli, "b", None).expect("made");
        let (found, kept_by_a) = vcon(UUID, "kept by a");
        let (_, kept_by_b) = vcon(UUID, "kept by b");
        let (store_a, store_b) = (
            f.tenants.store(a).expect("a's store"),
            f.tenants.store(b).expect("b's store"),
        );
        assert!(matches!(
            store_a.create(&caller(), &found, &kept_by_a),
            Ok(Created::New(_))
        ));
        assert!(matches!(
            store_b.create(&caller(), &found, &kept_by_b),
            Ok(Created::New(_))
        ));
        let read = |store: &Store| store.get(&caller(), found.uuid).map(|s| s.body);
        assert_eq!(read(&store_a), Ok(kept_by_a));
        assert_eq!(read(&store_b), Ok(kept_by_b.clone()));
        store_a
            .delete(&caller(), found.uuid, Retention::Years(7))
            .expect("a deletes its own");
        assert_eq!(read(&store_a), Err(StoreError::Gone));
        assert_eq!(read(&store_b), Ok(kept_by_b), "b's is untouched");
    }

    /// A version's data key is wrapped for its tenant by id, so the
    /// tenant's Store must be opened under that id.
    #[test]
    fn a_tenants_data_keys_are_bound_to_its_own_id() {
        let f = fixture();
        let id = f.tenants.create(Actor::Cli, "acme", None).expect("made");
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(version)) =
            f.tenants
                .store(id)
                .expect("its store")
                .create(&caller(), &found, &body)
        else {
            panic!("a new vCon")
        };
        let wrapped: Vec<u8> = f
            .tenant_catalog(id)
            .query_row(
                "SELECT wrapped_key FROM version WHERE version = ?1",
                [version.0],
                |r| r.get(0),
            )
            .expect("its row");
        let ctx = Context::Version {
            tenant: id,
            uuid: found.uuid,
            version,
        };
        assert!(envelope::unwrap(&f.tenant_key(id), &ctx, &wrapped).is_ok());
    }

    /// The answers one tenant gets for a uuid, whatever another holds.
    fn answers(store: &Store) -> Vec<String> {
        let (found, body) = vcon(UUID, "b's own");
        let (_, newer) = vcon(UUID, "b's newer");
        let kind = |r: Result<Created, StoreError>| match r {
            Ok(Created::New(_)) => "new".to_owned(),
            Ok(Created::Same(_)) => "same".to_owned(),
            Err(e) => format!("{e:?}"),
        };
        let who = caller();
        let mut answers = vec![
            format!("{:?}", store.get(&who, found.uuid).map(|_| ())),
            format!("{:?}", store.delete(&who, found.uuid, Retention::Years(7))),
            kind(store.create(&who, &found, &body)),
            kind(store.create(&who, &found, &body)),
        ];
        let current = store.get(&who, found.uuid).expect("stored").version;
        answers.push(format!(
            "{:?}",
            store.replace(&who, current, &found, &newer).map(|_| ())
        ));
        answers.push(format!(
            "{:?}",
            store.delete(&who, found.uuid, Retention::Years(7))
        ));
        answers.push(format!("{:?}", store.get(&who, found.uuid).map(|_| ())));
        answers
    }

    #[test]
    fn nothing_one_tenant_does_shows_whether_another_holds_a_uuid() {
        let alone = fixture();
        let b = alone.tenants.create(Actor::Cli, "b", None).expect("made");
        let expected = answers(&alone.tenants.store(b).expect("b's store"));

        let f = fixture();
        let a = f.tenants.create(Actor::Cli, "a", None).expect("made");
        let b = f.tenants.create(Actor::Cli, "b", None).expect("made");
        let store_a = f.tenants.store(a).expect("a's store");
        let (found, body) = vcon(UUID, "a's");
        store_a
            .create(&caller(), &found, &body)
            .expect("a holds it");
        let (other, body) = vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bc47", "a's other");
        store_a.create(&caller(), &other, &body).expect("stored");
        store_a
            .delete(&caller(), other.uuid, Retention::Years(7))
            .expect("and has deleted another");
        assert_eq!(answers(&f.tenants.store(b).expect("b's store")), expected);
        assert_eq!(
            expected,
            [
                "Err(NotFound)",
                "Err(NotFound)",
                "new",
                "same",
                "Ok(())",
                "Ok(())",
                "Err(Gone)"
            ]
        );
    }
}
