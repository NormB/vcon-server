//! The local backend: a tenant's catalog is a SQLCipher file,
//! `catalog.sqlite`, and its objects are files in `objects/`, each written
//! to a temporary name, synced, renamed to its version id and its directory
//! synced.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    Catalog, CatalogTx, CommitError, ExaminedRow, HiddenRow, KeptRow, KeyRow, ListedRow,
    NewVersion, ObjectError, Objects, PageQuery, Registry, RegistryTx, TenantRow, TenantSpace,
    Under, VconRow, VersionRow, WrappedRow,
};
use crate::audit::{self, Action, Actor, AuditError, Entry};
use crate::catalog;
use crate::clock::{Clock, nanos};
use crate::envelope::{self, Context, Key};
use crate::fs::Fs;
use crate::ids::{KeyId, TenantId, VersionId};
use crate::store::{OpenError, Store, StoreError};
use crate::tenants::{OpenStep, RegistryError, TenantError};

/// The connection tests reach the local catalogs through.
#[cfg(test)]
pub(crate) type LocalConn = Connection;

/// The tables of a tenant's catalog, besides its audit log. A key column
/// says NOT NULL because SQLite lets a PRIMARY KEY other than an INTEGER
/// one hold NULL.
pub(crate) const TABLES: &str = "
CREATE TABLE vcon (
    uuid BLOB PRIMARY KEY NOT NULL,
    current BLOB,
    deleted_at INTEGER,
    hold INTEGER NOT NULL DEFAULT 0,
    stored_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX vcon_listed ON vcon (stored_at, uuid);
CREATE TABLE version (
    version BLOB PRIMARY KEY NOT NULL,
    uuid BLOB NOT NULL,
    form TEXT NOT NULL,
    gzip INTEGER NOT NULL,
    size INTEGER NOT NULL,
    stored_at INTEGER NOT NULL,
    hidden_at INTEGER,
    tenant_key_id BLOB NOT NULL,
    wrapped_key BLOB NOT NULL
);
CREATE INDEX version_uuid ON version (uuid);
CREATE TABLE tombstone (
    uuid BLOB PRIMARY KEY NOT NULL,
    purged_at INTEGER NOT NULL
);
CREATE TABLE key_use (
    tenant_key_id BLOB PRIMARY KEY NOT NULL,
    wraps INTEGER NOT NULL
);
";

/// How long a checkpoint waits before it tries again while a reader still
/// holds the WAL.
const CHECKPOINT_RETRY: std::time::Duration = std::time::Duration::from_millis(10);

/// A tenant catalog in a SQLCipher file.
pub struct LocalCatalog {
    conn: Mutex<Connection>,
    catalog_key: Key,
    #[cfg(test)]
    fault: Mutex<Option<(usize, CommitError)>>,
    #[cfg(test)]
    checkpoint_wait: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl LocalCatalog {
    /// Opens the catalog at `path` under `catalog_key`, making it if new.
    pub fn open(path: &Path, catalog_key: Key) -> Result<LocalCatalog, catalog::CatalogError> {
        let schema = format!("{TABLES}{}", audit::SCHEMA);
        let conn = catalog::open(path, &catalog_key, &schema)?;
        Ok(LocalCatalog {
            conn: Mutex::new(conn),
            catalog_key,
            #[cfg(test)]
            fault: Mutex::new(None),
            #[cfg(test)]
            checkpoint_wait: Mutex::new(None),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .expect("no thread panicked holding the catalog")
    }

    fn begin(&self, how: &str) -> Result<Box<dyn CatalogTx + '_>, StoreError> {
        let conn = self.conn();
        conn.execute_batch(how).map_err(io)?;
        Ok(Box::new(LocalTx {
            conn,
            open: true,
            #[cfg(test)]
            fault: &self.fault,
        }))
    }
}

impl Catalog for LocalCatalog {
    fn immediate(&self) -> Result<Box<dyn CatalogTx + '_>, StoreError> {
        self.begin("BEGIN IMMEDIATE")
    }

    fn snapshot(&self) -> Result<Box<dyn CatalogTx + '_>, StoreError> {
        self.begin("BEGIN DEFERRED")
    }

    fn recover(&self) -> Result<(), StoreError> {
        let busy: i64 = self
            .conn()
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
            .map_err(io)?;
        if busy != 0 {
            return Err(StoreError::Io);
        }
        Ok(())
    }

    /// Checkpoints the WAL with truncation, trying again while a reader
    /// holds it. Each try asks without waiting, so the wait for a reader
    /// happens here, between tries, with the catalog free to other callers,
    /// not inside SQLite with the catalog locked.
    fn after_purge(&self) -> Result<(), StoreError> {
        loop {
            let busy: i64 = {
                let conn = self.conn();
                conn.busy_timeout(std::time::Duration::ZERO).map_err(io)?;
                let busy = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0));
                conn.busy_timeout(catalog::BUSY_TIMEOUT).map_err(io)?;
                busy.map_err(io)?
            };
            if busy == 0 {
                return Ok(());
            }
            #[cfg(test)]
            if let Some(busy) = &*self.checkpoint_wait.lock().expect("hook") {
                let _ = busy.send(());
            }
            std::thread::sleep(CHECKPOINT_RETRY);
        }
    }

    fn backup_into(&self, to: &Path) -> Result<Vec<VersionId>, StoreError> {
        catalog::backup(&self.conn(), to, &self.catalog_key).map_err(io)?;
        let copy = catalog::open(to, &self.catalog_key, "").map_err(io)?;
        let mut rows = copy.prepare("SELECT version FROM version").map_err(io)?;
        rows.query_map([], |r| r.get::<_, [u8; 16]>(0))
            .map_err(io)?
            .map(|row| row.map(VersionId).map_err(io))
            .collect()
    }

    fn export(&self, after: i64, out: &mut dyn Write) -> Result<(), AuditError> {
        audit::export(&self.conn(), Some(after), out)
    }

    #[cfg(test)]
    fn fail_commit(&self, nth: usize, how: CommitError) {
        *self.fault.lock().expect("fault") = Some((nth, how));
    }

    #[cfg(test)]
    fn on_checkpoint_wait(&self, busy: std::sync::mpsc::Sender<()>) {
        *self.checkpoint_wait.lock().expect("hook") = Some(busy);
    }

    #[cfg(test)]
    fn local_conn(&self) -> MutexGuard<'_, Connection> {
        self.conn()
    }
}

/// A transaction on a local catalog, begun with `BEGIN`, and rolled back
/// when dropped without a commit.
struct LocalTx<'a> {
    conn: MutexGuard<'a, Connection>,
    open: bool,
    #[cfg(test)]
    fault: &'a Mutex<Option<(usize, CommitError)>>,
}

impl Drop for LocalTx<'_> {
    fn drop(&mut self) {
        if self.open && !self.conn.is_autocommit() {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

impl LocalTx<'_> {
    fn count_of(&self, sql: &str, args: impl rusqlite::Params) -> Result<u64, StoreError> {
        let n: i64 = self.conn.query_row(sql, args, |r| r.get(0)).map_err(io)?;
        u64::try_from(n).map_err(io)
    }

    fn exists(&self, sql: &str, args: impl rusqlite::Params) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .query_row(sql, args, |_| Ok(()))
            .optional()
            .map_err(io)?
            .is_some())
    }

    fn versions(
        &self,
        sql: &str,
        args: impl rusqlite::Params,
    ) -> Result<Vec<VersionId>, StoreError> {
        let mut rows = self.conn.prepare(sql).map_err(io)?;
        rows.query_map(args, |r| r.get::<_, [u8; 16]>(0))
            .map_err(io)?
            .map(|row| row.map(VersionId).map_err(io))
            .collect()
    }
}

impl CatalogTx for LocalTx<'_> {
    fn tombstoned(&mut self, uuid: Uuid) -> Result<bool, StoreError> {
        self.exists("SELECT 1 FROM tombstone WHERE uuid = ?1", [uuid.as_bytes()])
    }

    fn vcon_row(&mut self, uuid: Uuid) -> Result<Option<VconRow>, StoreError> {
        let row: Option<(Option<[u8; 16]>, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT current, deleted_at FROM vcon WHERE uuid = ?1",
                [uuid.as_bytes()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(io)?;
        Ok(row.map(|(current, deleted_at)| VconRow {
            current: current.map(VersionId),
            deleted_at,
        }))
    }

    fn version_row(&mut self, version: VersionId) -> Result<Option<VersionRow>, StoreError> {
        let row: Option<(String, bool, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT form, gzip, wrapped_key FROM version WHERE version = ?1",
                [version.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(io)?;
        Ok(row.map(|(form, gzip, wrapped)| VersionRow {
            form,
            gzip,
            wrapped,
        }))
    }

    fn has_version(&mut self, version: VersionId, uuid: Uuid) -> Result<bool, StoreError> {
        self.exists(
            "SELECT 1 FROM version WHERE version = ?1 AND uuid = ?2",
            (version.0, uuid.as_bytes()),
        )
    }

    fn insert_version(&mut self, row: &NewVersion) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO version
                     (version, uuid, form, gzip, size, stored_at, hidden_at, tenant_key_id, wrapped_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8)",
                params![
                    row.version.0,
                    row.uuid.as_bytes(),
                    row.form,
                    row.gzip,
                    row.size,
                    row.stored_at,
                    row.key.bytes(),
                    row.wrapped,
                ],
            )
            .map_err(io)?;
        Ok(())
    }

    fn insert_vcon(
        &mut self,
        uuid: Uuid,
        current: VersionId,
        stored_at: i64,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO vcon (uuid, current, stored_at) VALUES (?1, ?2, ?3)",
                params![uuid.as_bytes(), current.0, stored_at],
            )
            .map_err(io)?;
        Ok(())
    }

    fn set_current(&mut self, uuid: Uuid, current: VersionId) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE vcon SET current = ?1 WHERE uuid = ?2",
                params![current.0, uuid.as_bytes()],
            )
            .map_err(io)?;
        Ok(())
    }

    fn hide_version(&mut self, version: VersionId, at: i64) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE version SET hidden_at = ?1 WHERE version = ?2",
                params![at, version.0],
            )
            .map_err(io)?;
        Ok(())
    }

    fn mark_deleted(&mut self, uuid: Uuid, at: i64) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE vcon SET current = NULL, deleted_at = ?1 WHERE uuid = ?2",
                params![at, uuid.as_bytes()],
            )
            .map_err(io)?;
        self.conn
            .execute(
                "UPDATE version SET hidden_at = ?1 WHERE uuid = ?2 AND hidden_at IS NULL",
                params![at, uuid.as_bytes()],
            )
            .map_err(io)?;
        Ok(())
    }

    fn wraps(&mut self, key: KeyId) -> Result<u64, StoreError> {
        let wraps: Option<i64> = self
            .conn
            .query_row(
                "SELECT wraps FROM key_use WHERE tenant_key_id = ?1",
                [key.bytes()],
                |r| r.get(0),
            )
            .optional()
            .map_err(io)?;
        u64::try_from(wraps.unwrap_or(0)).map_err(io)
    }

    fn add_wrap(&mut self, key: KeyId) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO key_use (tenant_key_id, wraps) VALUES (?1, 1)
                 ON CONFLICT (tenant_key_id) DO UPDATE SET wraps = wraps + 1",
                [key.bytes()],
            )
            .map_err(io)?;
        Ok(())
    }

    fn version_ids(&mut self) -> Result<Vec<VersionId>, StoreError> {
        self.versions("SELECT version FROM version ORDER BY version", [])
    }

    fn count_versions(&mut self) -> Result<u64, StoreError> {
        self.count_of("SELECT count(*) FROM version", [])
    }

    fn count_held(&mut self) -> Result<u64, StoreError> {
        self.count_of("SELECT count(*) FROM vcon WHERE hold != 0", [])
    }

    fn hidden_versions(&mut self, uuid: Option<Uuid>) -> Result<Vec<VersionId>, StoreError> {
        self.versions(
            "SELECT version FROM version
             WHERE hidden_at IS NOT NULL AND (?1 IS NULL OR uuid = ?1)",
            [uuid.map(|u| *u.as_bytes())],
        )
    }

    fn hidden_row(&mut self, version: VersionId) -> Result<Option<HiddenRow>, StoreError> {
        let row: Option<([u8; 16], i64, bool, bool)> = self
            .conn
            .query_row(
                "SELECT v.uuid, v.stored_at, c.hold != 0, c.deleted_at IS NOT NULL
                 FROM version v JOIN vcon c ON c.uuid = v.uuid
                 WHERE v.version = ?1 AND v.hidden_at IS NOT NULL",
                [version.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(io)?;
        Ok(row.map(|(uuid, stored_at, held, deleted)| HiddenRow {
            uuid: Uuid::from_bytes(uuid),
            stored_at,
            held,
            deleted,
        }))
    }

    fn delete_version(&mut self, version: VersionId) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM version WHERE version = ?1", [version.0])
            .map_err(io)?;
        Ok(())
    }

    fn versions_of(&mut self, uuid: Uuid) -> Result<u64, StoreError> {
        self.count_of(
            "SELECT count(*) FROM version WHERE uuid = ?1",
            [uuid.as_bytes()],
        )
    }

    fn tombstone(&mut self, uuid: Uuid, at: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM vcon WHERE uuid = ?1", [uuid.as_bytes()])
            .map_err(io)?;
        self.conn
            .execute(
                "INSERT INTO tombstone (uuid, purged_at) VALUES (?1, ?2)",
                params![uuid.as_bytes(), at],
            )
            .map_err(io)?;
        Ok(())
    }

    fn latest(&mut self, uuid: Uuid) -> Result<Option<VersionId>, StoreError> {
        self.conn
            .query_row(
                "SELECT version FROM version WHERE uuid = ?1
                 ORDER BY stored_at DESC, rowid DESC LIMIT 1",
                [uuid.as_bytes()],
                |r| r.get(0),
            )
            .optional()
            .map(|found| found.map(VersionId))
            .map_err(io)
    }

    fn kept(&mut self, uuid: Uuid) -> Result<Vec<KeptRow>, StoreError> {
        let mut rows = self
            .conn
            .prepare(
                "SELECT version, stored_at, hidden_at FROM version WHERE uuid = ?1
                 ORDER BY stored_at, rowid",
            )
            .map_err(io)?;
        rows.query_map([uuid.as_bytes()], |r| {
            Ok(KeptRow {
                version: VersionId(r.get(0)?),
                stored_at: r.get(1)?,
                hidden_at: r.get(2)?,
            })
        })
        .map_err(io)?
        .map(|row| row.map_err(io))
        .collect()
    }

    fn unhide(&mut self, version: VersionId) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE version SET hidden_at = NULL WHERE version = ?1",
                [version.0],
            )
            .map_err(io)?;
        Ok(())
    }

    fn undelete(&mut self, uuid: Uuid, version: VersionId) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE vcon SET current = ?1, deleted_at = NULL WHERE uuid = ?2",
                (version.0, uuid.as_bytes()),
            )
            .map_err(io)?;
        Ok(())
    }

    fn set_hold(&mut self, uuid: Uuid, on: bool) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE vcon SET hold = ?1 WHERE uuid = ?2",
                (on, uuid.as_bytes()),
            )
            .map_err(io)?;
        Ok(())
    }

    fn page(&mut self, query: &PageQuery) -> Result<Vec<ListedRow>, StoreError> {
        let mut rows = self
            .conn
            .prepare(
                "SELECT vcon.uuid, vcon.current, vcon.stored_at, version.stored_at
                 FROM vcon LEFT JOIN version ON version.version = vcon.current
                 WHERE vcon.current IS NOT NULL
                   AND (?1 IS NULL OR vcon.stored_at < ?1 OR (vcon.stored_at = ?1 AND vcon.uuid < ?2))
                   AND (?3 IS NULL OR vcon.stored_at >= ?3)
                   AND (?4 IS NULL OR vcon.stored_at < ?4)
                 ORDER BY vcon.stored_at DESC, vcon.uuid DESC
                 LIMIT ?5",
            )
            .map_err(io)?;
        rows.query_map(
            params![
                query.before.map(|(at, _)| at),
                query.before.map(|(_, uuid)| uuid.as_bytes().to_vec()),
                query.since,
                query.until,
                query.rows,
            ],
            |r| {
                Ok(ListedRow {
                    uuid: Uuid::from_bytes(r.get(0)?),
                    version: VersionId(r.get(1)?),
                    stored_at: r.get(2)?,
                    updated_at: r.get(3)?,
                })
            },
        )
        .map_err(io)?
        .map(|row| row.map_err(io))
        .collect()
    }

    fn wrapped_under(&mut self, key: KeyId, limit: usize) -> Result<Vec<WrappedRow>, StoreError> {
        let mut rows = self
            .conn
            .prepare(
                "SELECT version, uuid, wrapped_key FROM version
                 WHERE tenant_key_id = ?1 ORDER BY version LIMIT ?2",
            )
            .map_err(io)?;
        rows.query_map(
            params![key.bytes(), i64::try_from(limit).map_err(io)?],
            |r| {
                Ok(WrappedRow {
                    version: VersionId(r.get(0)?),
                    uuid: Uuid::from_bytes(r.get(1)?),
                    wrapped: r.get(2)?,
                })
            },
        )
        .map_err(io)?
        .map(|row| row.map_err(io))
        .collect()
    }

    fn set_wrapped(
        &mut self,
        version: VersionId,
        key: KeyId,
        wrapped: &[u8],
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "UPDATE version SET wrapped_key = ?1, tenant_key_id = ?2 WHERE version = ?3",
                params![wrapped, key.bytes(), version.0],
            )
            .map_err(io)?;
        Ok(())
    }

    fn count_not_under(&mut self, key: KeyId) -> Result<u64, StoreError> {
        self.count_of(
            "SELECT count(*) FROM version WHERE tenant_key_id != ?1",
            [key.bytes()],
        )
    }

    fn examined(&mut self) -> Result<Vec<ExaminedRow>, StoreError> {
        let mut rows = self
            .conn
            .prepare(
                "SELECT v.version, v.uuid, v.stored_at, v.hidden_at, c.hold != 0
                 FROM version v JOIN vcon c ON c.uuid = v.uuid ORDER BY v.version",
            )
            .map_err(io)?;
        rows.query_map([], |r| {
            Ok(ExaminedRow {
                version: VersionId(r.get(0)?),
                uuid: Uuid::from_bytes(r.get(1)?),
                stored_at: r.get(2)?,
                hidden_at: r.get(3)?,
                held: r.get(4)?,
            })
        })
        .map_err(io)?
        .map(|row| row.map_err(io))
        .collect()
    }

    fn purge_times(&mut self) -> Result<Vec<String>, StoreError> {
        let mut rows = self
            .conn
            .prepare("SELECT at FROM audit WHERE action = ?1 AND status = 200")
            .map_err(io)?;
        rows.query_map([Action::Purge.as_str()], |r| r.get(0))
            .map_err(io)?
            .map(|row| row.map_err(io))
            .collect()
    }

    fn begin_part(&mut self) -> Result<(), StoreError> {
        self.conn.execute_batch("SAVEPOINT part").map_err(io)
    }

    fn release_part(&mut self) -> Result<(), StoreError> {
        self.conn.execute_batch("RELEASE part").map_err(io)
    }

    fn rollback_part(&mut self) -> Result<(), StoreError> {
        self.conn
            .execute_batch("ROLLBACK TO part; RELEASE part")
            .map_err(io)
    }

    fn record(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        audit::record(&self.conn, entry, now)
    }

    fn count(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        audit::count(&self.conn, entry, now)
    }

    fn audit_lines(
        &mut self,
        after: i64,
        limit: Option<u32>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError> {
        match limit {
            Some(limit) => audit::page(&self.conn, &audit::AuditQuery { after, limit }, out),
            None => audit::export(&self.conn, Some(after), out),
        }
    }

    fn commit(mut self: Box<Self>) -> Result<(), CommitError> {
        #[cfg(test)]
        if let Some(outcome) = injected(&self.conn, self.fault) {
            self.open = false;
            return outcome;
        }
        let outcome = end(&self.conn);
        self.open = outcome == Err(CommitError::RolledBack) && !self.conn.is_autocommit();
        outcome
    }
}

/// A tenant's object files in one directory.
pub struct LocalObjects {
    dir: PathBuf,
    fs: Arc<dyn Fs>,
}

impl LocalObjects {
    pub fn new(dir: PathBuf, fs: Arc<dyn Fs>) -> LocalObjects {
        LocalObjects { dir, fs }
    }
}

fn object_error(e: std::io::Error) -> ObjectError {
    match e.kind() {
        std::io::ErrorKind::NotFound => ObjectError::Missing,
        _ => ObjectError::Io(e),
    }
}

impl Objects for LocalObjects {
    /// Writes in order: a temporary file, synced, renamed to its version
    /// id, and the directory synced.
    fn put(&self, version: VersionId, blob: &[u8]) -> Result<(), ObjectError> {
        let tmp = self.dir.join(format!("{version}.tmp"));
        let file = self.dir.join(version.to_string());
        let written = self
            .fs
            .write_new(&tmp, blob)
            .and_then(|()| self.fs.sync_file(&tmp))
            .and_then(|()| self.fs.rename(&tmp, &file))
            .and_then(|()| self.fs.sync_dir(&self.dir));
        if let Err(e) = written {
            // Best effort: start-up recovery removes any file left over.
            let _ = self.fs.remove(&tmp);
            let _ = self.fs.remove(&file);
            return Err(ObjectError::Io(e));
        }
        Ok(())
    }

    fn get(&self, version: VersionId) -> Result<Vec<u8>, ObjectError> {
        self.fs
            .read(&self.dir.join(version.to_string()))
            .map_err(object_error)
    }

    fn remove(&self, version: VersionId) -> Result<(), ObjectError> {
        self.fs
            .remove(&self.dir.join(version.to_string()))
            .map_err(object_error)
    }

    fn list(&self) -> Result<Vec<String>, ObjectError> {
        Ok(self
            .fs
            .list(&self.dir)
            .map_err(object_error)?
            .into_iter()
            .filter_map(|name| name.into_string().ok())
            .collect())
    }

    fn remove_name(&self, name: &str) -> Result<(), ObjectError> {
        self.fs.remove(&self.dir.join(name)).map_err(object_error)
    }

    fn sync(&self) -> Result<(), ObjectError> {
        self.fs.sync_dir(&self.dir).map_err(ObjectError::Io)
    }
}

fn io<E>(_: E) -> StoreError {
    StoreError::Io
}

/// The tables of server.sqlite, besides the admin audit log. A key column
/// says NOT NULL because SQLite lets a PRIMARY KEY other than an INTEGER
/// one hold NULL. Times are nanoseconds since the epoch. A retention period
/// is written as `Retention` writes it; NULL means the server default.
pub(crate) const REGISTRY_TABLES: &str = "
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

/// The registry in `server.sqlite`.
pub struct LocalRegistry {
    conn: Mutex<Connection>,
    catalog_key: Key,
    #[cfg(test)]
    fault: Mutex<Option<(usize, CommitError)>>,
}

impl LocalRegistry {
    /// Opens `<dir>/server.sqlite` under `catalog_key`, making it if new.
    pub fn open(dir: &Path, catalog_key: Key) -> Result<LocalRegistry, RegistryError> {
        let schema = format!("{REGISTRY_TABLES}{}", audit::SCHEMA);
        let path = dir.join("server.sqlite");
        let conn = catalog::open(&path, &catalog_key, &schema).map_err(|e| match e {
            catalog::CatalogError::WrongKey => RegistryError::WrongKey,
            e => RegistryError::failed(OpenStep::OpenCatalog, path.clone(), e),
        })?;
        Ok(LocalRegistry {
            conn: Mutex::new(conn),
            catalog_key,
            #[cfg(test)]
            fault: Mutex::new(None),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .expect("no thread panicked holding server.sqlite")
    }

    fn begin(&self, how: &str) -> Result<Box<dyn RegistryTx + '_>, TenantError> {
        let conn = self.conn();
        conn.execute_batch(how).map_err(tenant_io)?;
        Ok(Box::new(LocalRegistryTx {
            conn,
            open: true,
            #[cfg(test)]
            fault: &self.fault,
        }))
    }
}

impl Registry for LocalRegistry {
    fn immediate(&self) -> Result<Box<dyn RegistryTx + '_>, TenantError> {
        self.begin("BEGIN IMMEDIATE")
    }

    fn snapshot(&self) -> Result<Box<dyn RegistryTx + '_>, TenantError> {
        self.begin("BEGIN DEFERRED")
    }

    fn export(&self, after: i64, out: &mut dyn Write) -> Result<(), AuditError> {
        audit::export(&self.conn(), Some(after), out)
    }

    fn backup_into(&self, to: &Path) -> Result<Vec<TenantId>, TenantError> {
        catalog::backup(&self.conn(), to, &self.catalog_key).map_err(tenant_io)?;
        let copy = catalog::open(to, &self.catalog_key, "").map_err(tenant_io)?;
        let mut rows = copy
            .prepare("SELECT id FROM tenant ORDER BY id")
            .map_err(tenant_io)?;
        rows.query_map([], |r| r.get::<_, [u8; 16]>(0))
            .map_err(tenant_io)?
            .map(|row| row.map(TenantId).map_err(tenant_io))
            .collect()
    }

    #[cfg(test)]
    fn fail_commit(&self, nth: usize, how: CommitError) {
        *self.fault.lock().expect("fault") = Some((nth, how));
    }

    #[cfg(test)]
    fn local_conn(&self) -> MutexGuard<'_, Connection> {
        self.conn()
    }
}

/// A transaction on server.sqlite, rolled back when dropped without a
/// commit.
struct LocalRegistryTx<'a> {
    conn: MutexGuard<'a, Connection>,
    open: bool,
    #[cfg(test)]
    fault: &'a Mutex<Option<(usize, CommitError)>>,
}

impl Drop for LocalRegistryTx<'_> {
    fn drop(&mut self) {
        if self.open && !self.conn.is_autocommit() {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

const TENANT_COLUMNS: &str = "id, name, retention, created_at, tenant_key_id,
    wrapped_tenant_key, pending_wrapped_tenant_key, key_since";

fn tenant_row(r: &rusqlite::Row) -> rusqlite::Result<TenantRow> {
    Ok(TenantRow {
        id: TenantId(r.get(0)?),
        name: r.get(1)?,
        retention: r.get(2)?,
        created_at: r.get(3)?,
        tenant_key_id: KeyId(r.get(4)?),
        wrapped: r.get(5)?,
        pending: r.get(6)?,
        key_since: r.get(7)?,
    })
}

const KEY_COLUMNS: &str = "id, tenant, token_hash, scopes, label, created_at, revoked_at";

fn key_row(r: &rusqlite::Row) -> rusqlite::Result<KeyRow> {
    Ok(KeyRow {
        id: KeyId(r.get(0)?),
        tenant: r.get::<_, Option<[u8; 16]>>(1)?.map(TenantId),
        token_hash: r.get(2)?,
        scopes: r.get(3)?,
        label: r.get(4)?,
        created_at: r.get(5)?,
        revoked_at: r.get(6)?,
    })
}

impl RegistryTx for LocalRegistryTx<'_> {
    fn tenant(&mut self, id: TenantId) -> Result<Option<TenantRow>, TenantError> {
        self.conn
            .query_row(
                &format!("SELECT {TENANT_COLUMNS} FROM tenant WHERE id = ?1"),
                [id.0],
                tenant_row,
            )
            .optional()
            .map_err(tenant_io)
    }

    fn tenants(&mut self) -> Result<Vec<TenantRow>, TenantError> {
        let mut rows = self
            .conn
            .prepare(&format!(
                "SELECT {TENANT_COLUMNS} FROM tenant ORDER BY name"
            ))
            .map_err(tenant_io)?;
        rows.query_map([], tenant_row)
            .map_err(tenant_io)?
            .map(|row| row.map_err(tenant_io))
            .collect()
    }

    fn name_taken(&mut self, name: &str) -> Result<bool, TenantError> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM tenant WHERE name = ?1", [name], |_| Ok(()))
            .optional()
            .map_err(tenant_io)?
            .is_some())
    }

    fn insert_tenant(&mut self, row: &TenantRow) -> Result<(), TenantError> {
        self.conn
            .execute(
                "INSERT INTO tenant (id, name, retention, created_at, tenant_key_id,
                     wrapped_tenant_key, pending_wrapped_tenant_key, key_since)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    row.id.0,
                    row.name,
                    row.retention,
                    row.created_at,
                    row.tenant_key_id.bytes(),
                    row.wrapped,
                    row.pending,
                    row.key_since,
                ],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn set_retention(
        &mut self,
        id: TenantId,
        retention: Option<&str>,
    ) -> Result<bool, TenantError> {
        let changed = self
            .conn
            .execute(
                "UPDATE tenant SET retention = ?2 WHERE id = ?1",
                params![id.0, retention],
            )
            .map_err(tenant_io)?;
        Ok(changed != 0)
    }

    fn delete_tenant(&mut self, id: TenantId, at: i64) -> Result<(), TenantError> {
        self.conn
            .execute("DELETE FROM tenant WHERE id = ?1", [id.0])
            .map_err(tenant_io)?;
        self.conn
            .execute(
                "UPDATE api_key SET revoked_at = ?2 WHERE tenant = ?1 AND revoked_at IS NULL",
                params![id.0, at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn set_pending(&mut self, id: TenantId, pending: &[u8]) -> Result<(), TenantError> {
        self.conn
            .execute(
                "UPDATE tenant SET pending_wrapped_tenant_key = ?1 WHERE id = ?2",
                params![pending, id.0],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn promote_pending(&mut self, id: TenantId, key: KeyId, at: i64) -> Result<(), TenantError> {
        self.conn
            .execute(
                "UPDATE tenant SET tenant_key_id = ?1,
                    wrapped_tenant_key = pending_wrapped_tenant_key,
                    pending_wrapped_tenant_key = NULL, key_since = ?2
                 WHERE id = ?3",
                params![key.bytes(), at, id.0],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn master_wraps(&mut self, key: KeyId) -> Result<u64, TenantError> {
        let wraps: Option<i64> = self
            .conn
            .query_row(
                "SELECT wraps FROM master_key_use WHERE master_key_id = ?1",
                [key.bytes()],
                |r| r.get(0),
            )
            .optional()
            .map_err(tenant_io)?;
        u64::try_from(wraps.unwrap_or(0)).map_err(tenant_io)
    }

    fn add_master_wrap(&mut self, key: KeyId, at: i64) -> Result<(), TenantError> {
        self.conn
            .execute(
                "INSERT INTO master_key_use (master_key_id, wraps, since) VALUES (?1, 1, ?2)
                 ON CONFLICT (master_key_id) DO UPDATE SET wraps = wraps + 1",
                params![key.bytes(), at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn master_since(&mut self, key: KeyId) -> Result<Option<i64>, TenantError> {
        self.conn
            .query_row(
                "SELECT since FROM master_key_use WHERE master_key_id = ?1",
                [key.bytes()],
                |r| r.get(0),
            )
            .optional()
            .map_err(tenant_io)
    }

    fn key(&mut self, id: KeyId) -> Result<Option<KeyRow>, TenantError> {
        self.conn
            .query_row(
                &format!("SELECT {KEY_COLUMNS} FROM api_key WHERE id = ?1"),
                [id.bytes()],
                key_row,
            )
            .optional()
            .map_err(tenant_io)
    }

    fn keys(&mut self, tenant: Option<TenantId>) -> Result<Vec<KeyRow>, TenantError> {
        let mut rows = self
            .conn
            .prepare(&format!(
                "SELECT {KEY_COLUMNS} FROM api_key
                 WHERE ?1 IS NULL OR tenant = ?1 ORDER BY created_at, id"
            ))
            .map_err(tenant_io)?;
        rows.query_map([tenant.map(|t| t.0)], key_row)
            .map_err(tenant_io)?
            .map(|row| row.map_err(tenant_io))
            .collect()
    }

    fn insert_key(&mut self, row: &KeyRow) -> Result<(), TenantError> {
        self.conn
            .execute(
                "INSERT INTO api_key (id, tenant, token_hash, scopes, label, created_at, revoked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    row.id.bytes(),
                    row.tenant.map(|t| t.0),
                    row.token_hash,
                    row.scopes,
                    row.label,
                    row.created_at,
                    row.revoked_at,
                ],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn revoke_key(&mut self, id: KeyId, at: i64) -> Result<(), TenantError> {
        self.conn
            .execute(
                "UPDATE api_key SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id.bytes(), at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn record_times(&mut self, action: Action, status: u16) -> Result<Vec<String>, TenantError> {
        let mut rows = self
            .conn
            .prepare("SELECT at FROM audit WHERE action = ?1 AND status = ?2")
            .map_err(tenant_io)?;
        rows.query_map(params![action.as_str(), status], |r| r.get(0))
            .map_err(tenant_io)?
            .map(|row| row.map_err(tenant_io))
            .collect()
    }

    fn record(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        audit::record(&self.conn, entry, now)
    }

    fn count(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
        audit::count(&self.conn, entry, now)
    }

    fn audit_lines(
        &mut self,
        after: i64,
        limit: Option<u32>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError> {
        match limit {
            Some(limit) => audit::page(&self.conn, &audit::AuditQuery { after, limit }, out),
            None => audit::export(&self.conn, Some(after), out),
        }
    }

    fn commit(mut self: Box<Self>) -> Result<(), CommitError> {
        #[cfg(test)]
        if let Some(outcome) = injected(&self.conn, self.fault) {
            self.open = false;
            return outcome;
        }
        let outcome = end(&self.conn);
        self.open = outcome == Err(CommitError::RolledBack) && !self.conn.is_autocommit();
        outcome
    }
}

/// Each tenant's directory under `<data>/tenants`: its catalog file and
/// its objects directory.
pub struct LocalTenantSpace {
    tenants: PathBuf,
    fs: Arc<dyn Fs>,
}

impl LocalTenantSpace {
    /// The space under `<data>/tenants`, made if new and made durable.
    pub fn open(data: &Path, fs: Arc<dyn Fs>) -> Result<LocalTenantSpace, RegistryError> {
        let tenants = data.join("tenants");
        fs.create_dir_all(&tenants)
            .map_err(|e| RegistryError::failed(OpenStep::MakeTenantsDir, tenants.clone(), e))?;
        fs.sync_dir(data)
            .map_err(|e| RegistryError::failed(OpenStep::SyncDataDir, data.to_path_buf(), e))?;
        Ok(LocalTenantSpace { tenants, fs })
    }

    fn dir(&self, id: TenantId) -> PathBuf {
        self.tenants.join(id.to_string())
    }

    fn catalog(&self, id: TenantId) -> PathBuf {
        self.dir(id).join("catalog.sqlite")
    }
}

impl TenantSpace for LocalTenantSpace {
    /// The directory and its catalog are made, and made durable: the
    /// directory's entry in `tenants/` by syncing it.
    fn make(&self, id: TenantId, key: Key, clock: Arc<dyn Clock>) -> Result<(), OpenError> {
        let dir = self.dir(id);
        self.fs.create_dir_all(&dir).map_err(|_| StoreError::Io)?;
        drop(Store::open(&dir, id, key, self.fs.clone(), clock)?);
        self.fs
            .sync_dir(&self.tenants)
            .map_err(|_| StoreError::Io)?;
        Ok(())
    }

    fn open(
        &self,
        id: TenantId,
        key: Key,
        recover: bool,
        clock: Arc<dyn Clock>,
    ) -> Result<Store, OpenError> {
        let open = if recover {
            Store::open
        } else {
            Store::open_beside
        };
        open(&self.dir(id), id, key, self.fs.clone(), clock)
    }

    /// A name that is not a tenant id is not ours and is left out.
    fn present(&self) -> Result<Vec<TenantId>, TenantError> {
        Ok(self
            .fs
            .list(&self.tenants)
            .map_err(tenant_io)?
            .iter()
            .filter_map(|name| name.to_str().and_then(TenantId::parse))
            .collect())
    }

    fn remove(&self, id: TenantId) -> Result<(), TenantError> {
        self.fs.remove_dir_all(&self.dir(id)).map_err(tenant_io)
    }

    fn catalog_under(&self, id: TenantId, old: &Key, new: &Key) -> Result<Under, TenantError> {
        let path = self.catalog(id);
        match catalog::open(&path, old, "") {
            Ok(conn) => {
                drop(conn);
                Ok(Under::Old)
            }
            Err(catalog::CatalogError::WrongKey) => {
                drop(catalog::open(&path, new, "").map_err(tenant_io)?);
                Ok(Under::New)
            }
            Err(e) => Err(tenant_io(e)),
        }
    }

    fn reseal(&self, id: TenantId, old: &Key, new: &Key) -> Result<(), TenantError> {
        catalog::reencrypt(&self.catalog(id), old, new).map_err(tenant_io)
    }
}

fn tenant_io<E>(_: E) -> TenantError {
    TenantError::Io
}

/// Rotates the master key of the local registry in `dir`, as
/// `tenants::rekey_master` describes.
pub(crate) fn rekey_master(
    dir: &Path,
    old: &Key,
    new: &Key,
    now: OffsetDateTime,
    stop: Option<crate::tenants::MasterStep>,
) -> Result<(), TenantError> {
    let stop_after = |step| match stop == Some(step) {
        true => Err(TenantError::Io),
        false => Ok(()),
    };
    let path = dir.join("server.sqlite");
    let old_catalog = envelope::catalog_key(old).map_err(tenant_io)?;
    let new_catalog = envelope::catalog_key(new).map_err(tenant_io)?;
    match catalog::open(&path, &old_catalog, "") {
        Ok(conn) => drop(conn),
        // A run stopped after its rename left server.sqlite under the new
        // key: nothing is left to do.
        Err(catalog::CatalogError::WrongKey) => {
            return match catalog::open(&path, &new_catalog, "") {
                Ok(_) => Ok(()),
                Err(catalog::CatalogError::WrongKey) => Err(TenantError::WrongKey),
                Err(e) => Err(tenant_io(e)),
            };
        }
        Err(e) => return Err(tenant_io(e)),
    }
    let copy = catalog::copy_under(&path, &old_catalog, &new_catalog).map_err(tenant_io)?;
    stop_after(crate::tenants::MasterStep::Copied)?;
    {
        let mut conn = catalog::open(&copy, &new_catalog, "").map_err(tenant_io)?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(tenant_io)?;
        let rows: Vec<(TenantId, Vec<u8>, Option<Vec<u8>>)> = tx
            .prepare("SELECT id, wrapped_tenant_key, pending_wrapped_tenant_key FROM tenant")
            .map_err(tenant_io)?
            .query_map([], |r| Ok((TenantId(r.get(0)?), r.get(1)?, r.get(2)?)))
            .map_err(tenant_io)?
            .collect::<Result<_, _>>()
            .map_err(tenant_io)?;
        let mut wraps = 0_i64;
        for (id, wrapped, pending) in rows {
            let ctx = Context::Tenant { tenant: id };
            let rewrap = |w: &[u8]| envelope::rewrap(old, new, &ctx, w).map_err(tenant_io);
            let pending = pending.as_deref().map(rewrap).transpose()?;
            wraps += 1 + i64::from(pending.is_some());
            tx.execute(
                "UPDATE tenant SET wrapped_tenant_key = ?1, pending_wrapped_tenant_key = ?2
                 WHERE id = ?3",
                params![rewrap(&wrapped)?, pending, id.0],
            )
            .map_err(tenant_io)?;
        }
        tx.execute("DELETE FROM master_key_use", [])
            .map_err(tenant_io)?;
        tx.execute(
            "INSERT INTO master_key_use (master_key_id, wraps, since) VALUES (?1, ?2, ?3)",
            params![new.id().bytes(), wraps, nanos(now)],
        )
        .map_err(tenant_io)?;
        let entry = Entry {
            actor: Actor::Cli,
            action: Action::RekeyMaster,
            target: None,
            version: None,
            status: 200,
        };
        audit::record(&tx, &entry, now).map_err(|_| TenantError::AuditFailed)?;
        tx.commit().map_err(tenant_io)?;
    }
    stop_after(crate::tenants::MasterStep::Rewrapped)?;
    catalog::replace(&path, &copy).map_err(tenant_io)?;
    stop_after(crate::tenants::MasterStep::Renamed)
}

/// Commits the transaction open on `conn`. A COMMIT that fails while the
/// transaction is still open kept nothing, and the transaction's drop
/// rolls it back; one that fails with the transaction gone may have been
/// kept.
fn end(conn: &Connection) -> Result<(), CommitError> {
    match conn.execute_batch("COMMIT") {
        Ok(()) => Ok(()),
        Err(_) if !conn.is_autocommit() => Err(CommitError::RolledBack),
        Err(_) => Err(CommitError::Unknown),
    }
}

/// In tests, the commit fault set for this commit, if any: Unknown commits
/// and reports failure; RolledBack rolls back.
#[cfg(test)]
fn injected(
    conn: &Connection,
    fault: &Mutex<Option<(usize, CommitError)>>,
) -> Option<Result<(), CommitError>> {
    let mut fault = fault.lock().expect("fault");
    match *fault {
        Some((1, how)) => {
            *fault = None;
            let sql = match how {
                CommitError::Unknown => "COMMIT",
                CommitError::RolledBack => "ROLLBACK",
            };
            conn.execute_batch(sql).expect("ends the transaction");
            Some(Err(how))
        }
        Some((nth, how)) => {
            *fault = Some((nth - 1, how));
            None
        }
        None => None,
    }
}
