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
    Catalog, CatalogTx, CommitError, ExaminedRow, ExportedRow, HiddenRow, Job, KeptRow, KeyRow,
    ListedRow, MembershipRow, NewPackFile, NewVersion, ObjectError, ObjectPlace, Objects,
    PackFileRow, Packed, PageQuery, Registry, RegistryTx, SessionRow, TenantRow, TenantSpace,
    Under, UnpackedRow, UserRow, VconRow, VersionRow, WrappedRow,
};
use crate::audit::{self, Action, Actor, AuditError, Entry};
use crate::catalog::{self, Cached};
use crate::clock::{Clock, nanos};
use crate::envelope::{self, Context, Key};
use crate::fs::Fs;
use crate::ids::{FileId, KeyId, TenantId, UserId, VersionId};
use crate::store::{OpenError, Store, StoreError};
use crate::tenants::{OpenStep, RegistryError, TenantError};

mod durable;

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
    wrapped_key BLOB NOT NULL,
    place_file BLOB,
    place_group INTEGER,
    place_row INTEGER
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

/// The pending rows of the writes under way (store/pending.rs): a table of
/// the connection's own, in memory (`temp_store = MEMORY`) and never in
/// the file, so the catalog's schema and version stay as they are. A data
/// directory is written by one process, which holds its lock, so every
/// write under way is one of this process's, on this connection; a row
/// goes with the process, as its write does. The pack files under way
/// are kept the same way.
const PENDING_WRITES: &str = "
CREATE TEMP TABLE pending_write (
    version BLOB PRIMARY KEY NOT NULL,
    started_at INTEGER NOT NULL
);
CREATE TEMP TABLE pending_file (
    file_id BLOB PRIMARY KEY NOT NULL,
    started_at INTEGER NOT NULL
);
";

/// The creating rows of the tenant creates under way (store/pending.rs):
/// a table of the registry connection's own, kept out of server.sqlite as
/// `pending_write` is kept out of a catalog, and for the same reason: a
/// data directory is written by one process, so every create under way is
/// one of this process's.
const PENDING_TENANTS: &str = "
CREATE TEMP TABLE pending_tenant (
    id BLOB PRIMARY KEY NOT NULL,
    started_at INTEGER NOT NULL
);
";

/// How long a checkpoint waits before it tries again while a reader still
/// holds the WAL.
const CHECKPOINT_RETRY: std::time::Duration = std::time::Duration::from_millis(10);

/// How a tenant catalog's commits are made durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// The process that writes the data directory: a server, or a command
    /// that runs alone. The connection runs with `synchronous = NORMAL`; a
    /// commit lets the connection go once its frames are in the WAL and
    /// returns after one fdatasync of the WAL covers it (a group sync), and
    /// a transaction that only read waits until every commit it may have
    /// seen is durable (durable.rs).
    Group,
    /// A command beside a server. Its own commits sync inside COMMIT
    /// (`synchronous = FULL`), so the server never sees one of them before
    /// it is durable; and since it can read the server's commits before
    /// the server has synced them, which it cannot count, each of its
    /// transactions syncs the WAL at its end, before anything is answered.
    Beside,
}

/// A tenant catalog in a SQLCipher file.
pub struct LocalCatalog {
    conn: Mutex<Connection>,
    catalog_key: Key,
    durable: durable::Durable,
    durability: Durability,
    #[cfg(test)]
    fault: Mutex<Option<(usize, CommitError)>>,
    #[cfg(test)]
    checkpoint_wait: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl LocalCatalog {
    /// Opens the catalog at `path` under `catalog_key`, making it if new,
    /// and opens its WAL for the syncs `durability` makes, through `fs`.
    pub fn open(
        path: &Path,
        catalog_key: Key,
        fs: &dyn Fs,
        durability: Durability,
    ) -> Result<LocalCatalog, catalog::CatalogError> {
        let schema = format!("{TABLES}{}{}", catalog::PACK_TABLES, audit::SCHEMA);
        let conn = catalog::open(path, &catalog_key, &schema)?;
        conn.execute_batch(PENDING_WRITES)?;
        if durability == Durability::Group {
            conn.pragma_update(None, "synchronous", "NORMAL")?;
        }
        // The WAL exists once the connection has read the file in WAL mode
        // (catalog::open reads its schema version). It is opened here, once,
        // so the handle sees every writeback error from now on; synced, so
        // commits an earlier process left unsynced (a process that crashed)
        // are durable before anything here answers from them; and the
        // directory synced, so the file's entry is too.
        let wal = fs
            .open_for_sync(&catalog::sibling(path, "-wal"))
            .and_then(|wal| wal.sync_data().map(|()| wal))
            .and_then(|wal| {
                fs.sync_dir(path.parent().unwrap_or(Path::new(".")))
                    .map(|()| wal)
            })
            .map_err(catalog::CatalogError::Io)?;
        Ok(LocalCatalog {
            conn: Mutex::new(conn),
            catalog_key,
            durable: durable::Durable::new(wal),
            durability,
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
        self.durable.check().map_err(io)?;
        conn.cached_run(how, []).map_err(io)?;
        Ok(Box::new(LocalTx {
            conn: Held::On(conn),
            durable: &self.durable,
            durability: self.durability,
            open: true,
            #[cfg(test)]
            fault: &self.fault,
        }))
    }
}

/// A transaction's hold on the tenant's connection, let go before the
/// transaction waits for a sync.
enum Held<'a> {
    On(MutexGuard<'a, Connection>),
    Gone,
}

impl std::ops::Deref for Held<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        match self {
            Held::On(conn) => conn,
            Held::Gone => unreachable!("a transaction uses its connection only until it ends"),
        }
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
            .cached_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
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
                let busy = conn.cached_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0));
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

    /// The copy holds only what is durable: with `Group`, every commit
    /// written is synced before the copy is made (no commit can start while
    /// the connection is held); with `Beside`, the WAL is synced after the
    /// copy is made, before it is returned.
    fn backup_into(&self, to: &Path) -> Result<Vec<(VersionId, Option<Packed>)>, StoreError> {
        {
            let conn = self.conn();
            if self.durability == Durability::Group {
                self.durable.wait(self.durable.seen()).map_err(io)?;
            }
            catalog::backup(&conn, to, &self.catalog_key).map_err(io)?;
            if self.durability == Durability::Beside {
                self.durable.sync_seen(data_version(&conn)).map_err(io)?;
            }
        }
        let copy = catalog::open(to, &self.catalog_key, "").map_err(io)?;
        let mut rows = copy
            .prepare_cached(&format!(
                "SELECT v.version, {PLACE} FROM version v {PLACE_JOIN}"
            ))
            .map_err(io)?;
        rows.query_map([], |r| Ok((VersionId(r.get(0)?), raw_place(r, 1)?)))
            .map_err(io)?
            .map(|row| {
                let (version, place) = row.map_err(io)?;
                Ok((version, packed(place)?))
            })
            .collect()
    }

    fn unpack_copy(&self, copy: &Path) -> Result<(), StoreError> {
        let copy = catalog::open(copy, &self.catalog_key, "").map_err(io)?;
        copy.execute_batch(
            "BEGIN IMMEDIATE;
             UPDATE version SET place_file = NULL, place_group = NULL, place_row = NULL;
             DELETE FROM parquet_file;
             DELETE FROM purge_pending;
             DELETE FROM manifest;
             COMMIT;",
        )
        .map_err(io)
    }

    /// Writes nothing that is not durable: with `Group`, every commit
    /// written is synced first (none can start while the connection is
    /// held); with `Beside`, a read transaction fixes what is read, the WAL
    /// is synced, and only then are the records written out.
    fn export(&self, after: i64, out: &mut dyn Write) -> Result<(), AuditError> {
        let conn = self.conn();
        let poisoned = |_| AuditError::Sqlite(not_durable());
        match self.durability {
            Durability::Group => {
                self.durable.wait(self.durable.seen()).map_err(poisoned)?;
                audit::export(&conn, Some(after), out)
            }
            Durability::Beside => {
                conn.execute_batch("BEGIN DEFERRED")?;
                let exported = (|| {
                    let _: i64 =
                        conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
                    self.durable
                        .sync_seen(data_version(&conn))
                        .map_err(poisoned)?;
                    audit::export(&conn, Some(after), out)
                })();
                let _ = conn.execute_batch("ROLLBACK");
                exported
            }
        }
    }

    /// With `Group`: holding the connection (no commit can begin), waits
    /// until a sync covers every commit written, which is no wait when
    /// each has returned; Io once any sync failed. With `Beside`, whose
    /// own commits sync inside COMMIT and whose reads synced at their end:
    /// Io once a sync failed.
    fn close(&self) -> Result<(), StoreError> {
        let _conn = self.conn();
        match self.durability {
            Durability::Group => self.durable.wait(self.durable.seen()).map_err(io),
            Durability::Beside => self.durable.check().map_err(io),
        }
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

    #[cfg(test)]
    fn await_sync_waiters(&self, n: usize) {
        self.durable.await_waiting(n);
    }
}

/// A transaction on a local catalog, begun with `BEGIN`, and rolled back
/// when dropped without a commit.
struct LocalTx<'a> {
    conn: Held<'a>,
    durable: &'a durable::Durable,
    durability: Durability,
    open: bool,
    #[cfg(test)]
    fault: &'a Mutex<Option<(usize, CommitError)>>,
}

impl Drop for LocalTx<'_> {
    fn drop(&mut self) {
        // A failed wait cannot be reported here; the catalog is poisoned,
        // so every later transaction fails (CatalogTx::end).
        let _ = self.finish();
    }
}

impl LocalTx<'_> {
    /// Ends the transaction if it is not ended: rolls back what is open,
    /// lets the connection go, then waits until every commit it may have
    /// seen is durable.
    fn finish(&mut self) -> Result<(), durable::Poisoned> {
        if matches!(self.conn, Held::Gone) {
            return Ok(());
        }
        if self.open && !self.conn.is_autocommit() {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        match self.durability {
            Durability::Group => {
                let seen = self.durable.seen();
                self.conn = Held::Gone;
                self.durable.wait(seen)
            }
            Durability::Beside => {
                let version = data_version(&self.conn);
                self.conn = Held::Gone;
                self.durable.sync_seen(version)
            }
        }
    }
}

/// The error of a catalog whose WAL could not be synced (durable.rs).
fn not_durable() -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_IOERR),
        Some("the catalog's WAL could not be synced".to_owned()),
    )
}

/// SQLite's data_version of the file as `conn` sees it: it changes when
/// another connection commits. None if it cannot be read.
fn data_version(conn: &Connection) -> Option<i64> {
    conn.cached_row("PRAGMA data_version", [], |r| r.get(0))
        .ok()
}

impl LocalTx<'_> {
    fn count_of(&self, sql: &str, args: impl rusqlite::Params) -> Result<u64, StoreError> {
        let n: i64 = self.conn.cached_row(sql, args, |r| r.get(0)).map_err(io)?;
        u64::try_from(n).map_err(io)
    }

    fn exists(&self, sql: &str, args: impl rusqlite::Params) -> Result<bool, StoreError> {
        Ok(self
            .conn
            .cached_row(sql, args, |_| Ok(()))
            .optional()
            .map_err(io)?
            .is_some())
    }

    /// The file ids `sql` selects, in its order.
    fn file_ids(&self, sql: &str) -> Result<Vec<FileId>, StoreError> {
        let mut rows = self.conn.prepare_cached(sql).map_err(io)?;
        rows.query_map([], |r| r.get::<_, [u8; 16]>(0))
            .map_err(io)?
            .map(|row| row.map(FileId).map_err(io))
            .collect()
    }

    fn versions(
        &self,
        sql: &str,
        args: impl rusqlite::Params,
    ) -> Result<Vec<VersionId>, StoreError> {
        let mut rows = self.conn.prepare_cached(sql).map_err(io)?;
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
            .cached_row(
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
        let row: Option<(String, bool, Vec<u8>, RawPlace)> = self
            .conn
            .cached_row(
                // CAST: a bare `?1` here, with the join, makes SQLite mark the
                // statement to be prepared again when the value changes, and
                // the kept statement is then parsed on every read.
                &format!(
                    "SELECT v.form, v.gzip, v.wrapped_key, {PLACE} FROM version v {PLACE_JOIN}
                     WHERE v.version = CAST(?1 AS BLOB)"
                ),
                [version.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, raw_place(r, 3)?)),
            )
            .optional()
            .map_err(io)?;
        row.map(|(form, gzip, wrapped, place)| {
            Ok(VersionRow {
                form,
                gzip,
                wrapped,
                place: packed(place)?,
            })
        })
        .transpose()
    }

    fn has_version(&mut self, version: VersionId, uuid: Uuid) -> Result<bool, StoreError> {
        self.exists(
            "SELECT 1 FROM version WHERE version = ?1 AND uuid = ?2",
            (version.0, uuid.as_bytes()),
        )
    }

    fn insert_version(&mut self, row: &NewVersion) -> Result<(), StoreError> {
        self.conn
            .cached_run(
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
            .cached_run(
                "INSERT INTO vcon (uuid, current, stored_at) VALUES (?1, ?2, ?3)",
                params![uuid.as_bytes(), current.0, stored_at],
            )
            .map_err(io)?;
        Ok(())
    }

    fn set_current(&mut self, uuid: Uuid, current: VersionId) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "UPDATE vcon SET current = ?1 WHERE uuid = ?2",
                params![current.0, uuid.as_bytes()],
            )
            .map_err(io)?;
        Ok(())
    }

    fn hide_version(&mut self, version: VersionId, at: i64) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "UPDATE version SET hidden_at = ?1 WHERE version = ?2",
                params![at, version.0],
            )
            .map_err(io)?;
        Ok(())
    }

    fn mark_deleted(&mut self, uuid: Uuid, at: i64) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "UPDATE vcon SET current = NULL, deleted_at = ?1 WHERE uuid = ?2",
                params![at, uuid.as_bytes()],
            )
            .map_err(io)?;
        self.conn
            .cached_run(
                "UPDATE version SET hidden_at = ?1 WHERE uuid = ?2 AND hidden_at IS NULL",
                params![at, uuid.as_bytes()],
            )
            .map_err(io)?;
        Ok(())
    }

    fn wraps(&mut self, key: KeyId) -> Result<u64, StoreError> {
        let wraps: Option<i64> = self
            .conn
            .cached_row(
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
            .cached_run(
                "INSERT INTO key_use (tenant_key_id, wraps) VALUES (?1, 1)
                 ON CONFLICT (tenant_key_id) DO UPDATE SET wraps = wraps + 1",
                [key.bytes()],
            )
            .map_err(io)?;
        Ok(())
    }

    fn add_pending(&mut self, version: VersionId, at: i64) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO pending_write (version, started_at) VALUES (?1, ?2)",
                params![version.0, at],
            )
            .map(drop)
            .map_err(io)
    }

    fn drop_pending(&mut self, version: VersionId) -> Result<bool, StoreError> {
        self.conn
            .execute("DELETE FROM pending_write WHERE version = ?1", [version.0])
            .map(|deleted| deleted > 0)
            .map_err(io)
    }

    fn pending(&mut self) -> Result<Vec<(VersionId, i64)>, StoreError> {
        let mut rows = self
            .conn
            .prepare("SELECT version, started_at FROM pending_write ORDER BY version")
            .map_err(io)?;
        rows.query_map([], |r| Ok((VersionId(r.get(0)?), r.get(1)?)))
            .map_err(io)?
            .collect::<Result<_, _>>()
            .map_err(io)
    }

    fn add_pending_file(&mut self, file: FileId, at: i64) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO pending_file (file_id, started_at) VALUES (?1, ?2)",
                params![file.0, at],
            )
            .map(drop)
            .map_err(io)
    }

    fn drop_pending_file(&mut self, file: FileId) -> Result<bool, StoreError> {
        self.conn
            .execute("DELETE FROM pending_file WHERE file_id = ?1", [file.0])
            .map(|deleted| deleted > 0)
            .map_err(io)
    }

    fn pending_files(&mut self) -> Result<Vec<(FileId, i64)>, StoreError> {
        let mut rows = self
            .conn
            .prepare("SELECT file_id, started_at FROM pending_file ORDER BY file_id")
            .map_err(io)?;
        rows.query_map([], |r| Ok((FileId(r.get(0)?), r.get(1)?)))
            .map_err(io)?
            .collect::<Result<_, _>>()
            .map_err(io)
    }

    fn unpacked(&mut self) -> Result<Vec<UnpackedRow>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached(
                "SELECT version, stored_at, size FROM version WHERE place_file IS NULL
                 ORDER BY stored_at, version",
            )
            .map_err(io)?;
        rows.query_map([], |r| {
            Ok(UnpackedRow {
                version: VersionId(r.get(0)?),
                stored_at: r.get(1)?,
                size: r.get(2)?,
            })
        })
        .map_err(io)?
        .map(|row| row.map_err(io))
        .collect()
    }

    fn is_unpacked(&mut self, version: VersionId) -> Result<bool, StoreError> {
        self.exists(
            "SELECT 1 FROM version WHERE version = ?1 AND place_file IS NULL",
            [version.0],
        )
    }

    fn insert_pack_file(&mut self, row: &NewPackFile) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "INSERT INTO parquet_file (file_id, date, rows, live_rows, bytes, created_at)
                 VALUES (?1, ?2, ?3, ?3, ?4, ?5)",
                params![row.file.0, row.date, row.rows, row.bytes, row.created_at],
            )
            .map(drop)
            .map_err(io)
    }

    fn set_place(
        &mut self,
        version: VersionId,
        file: FileId,
        place: super::pack::Place,
    ) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "UPDATE version SET place_file = ?1, place_group = ?2, place_row = ?3
                 WHERE version = ?4",
                params![file.0, place.group, place.row, version.0],
            )
            .map(drop)
            .map_err(io)
    }

    fn pack_files(&mut self) -> Result<Vec<FileId>, StoreError> {
        self.file_ids("SELECT file_id FROM parquet_file ORDER BY file_id")
    }

    fn pack_file(&mut self, file: FileId) -> Result<Option<PackFileRow>, StoreError> {
        let row: Option<(String, i64, Option<i64>)> = self
            .conn
            .cached_row(
                "SELECT date, bytes, unnamed_at FROM parquet_file WHERE file_id = ?1",
                [file.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(io)?;
        row.map(|(date, bytes, unnamed_at)| {
            Ok(PackFileRow {
                date,
                bytes: u64::try_from(bytes).map_err(|_| StoreError::Damaged)?,
                unnamed_at,
            })
        })
        .transpose()
    }

    fn placed_in(&mut self, file: FileId) -> Result<Vec<(VersionId, i64, Packed)>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached(&format!(
                "SELECT v.version, v.stored_at, {PLACE} FROM version v {PLACE_JOIN}
                 WHERE v.place_file = ?1 ORDER BY v.place_group, v.place_row"
            ))
            .map_err(io)?;
        rows.query_map([file.0], |r| {
            Ok((VersionId(r.get(0)?), r.get(1)?, raw_place(r, 2)?))
        })
        .map_err(io)?
        .map(|row| {
            let (version, stored_at, place) = row.map_err(io)?;
            let packed = packed(place)?.ok_or(StoreError::Damaged)?;
            Ok((version, stored_at, packed))
        })
        .collect()
    }

    fn add_purge_pending(&mut self, file: FileId) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "INSERT OR IGNORE INTO purge_pending (file_id) VALUES (?1)",
                [file.0],
            )
            .map(drop)
            .map_err(io)
    }

    fn purge_pending(&mut self) -> Result<Vec<FileId>, StoreError> {
        self.file_ids("SELECT file_id FROM purge_pending ORDER BY file_id")
    }

    fn delete_purge_pending(&mut self, file: FileId) -> Result<(), StoreError> {
        self.conn
            .cached_run("DELETE FROM purge_pending WHERE file_id = ?1", [file.0])
            .map(drop)
            .map_err(io)
    }

    fn unname_pack_file(&mut self, file: FileId, at: i64) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "UPDATE parquet_file SET unnamed_at = ?2, live_rows = 0 WHERE file_id = ?1",
                params![file.0, at],
            )
            .map(drop)
            .map_err(io)
    }

    fn unnamed_files(&mut self) -> Result<Vec<FileId>, StoreError> {
        self.file_ids(
            "SELECT file_id FROM parquet_file WHERE unnamed_at IS NOT NULL ORDER BY file_id",
        )
    }

    fn delete_pack_file(&mut self, file: FileId) -> Result<(), StoreError> {
        self.conn
            .cached_run("DELETE FROM parquet_file WHERE file_id = ?1", [file.0])
            .map(drop)
            .map_err(io)
    }

    fn allocate_manifest(&mut self) -> Result<i64, StoreError> {
        self.conn
            .cached_run("INSERT INTO manifest (published_at) VALUES (NULL)", [])
            .map_err(io)?;
        Ok(self.conn.last_insert_rowid())
    }

    fn unpublished_manifests(&mut self) -> Result<Vec<i64>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached("SELECT seq FROM manifest WHERE published_at IS NULL ORDER BY seq")
            .map_err(io)?;
        rows.query_map([], |r| r.get(0))
            .map_err(io)?
            .map(|row| row.map_err(io))
            .collect()
    }

    fn mark_published(&mut self, seq: i64, at: i64) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "UPDATE manifest SET published_at = ?2 WHERE seq = ?1",
                params![seq, at],
            )
            .map(drop)
            .map_err(io)
    }

    fn published_manifests(&mut self) -> Result<Vec<(i64, i64)>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached(
                "SELECT seq, published_at FROM manifest WHERE published_at IS NOT NULL
                 ORDER BY seq",
            )
            .map_err(io)?;
        rows.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(io)?
            .map(|row| row.map_err(io))
            .collect()
    }

    fn delete_manifest(&mut self, seq: i64) -> Result<(), StoreError> {
        self.conn
            .cached_run("DELETE FROM manifest WHERE seq = ?1", [seq])
            .map(drop)
            .map_err(io)
    }

    fn named_files(&mut self) -> Result<Vec<(FileId, PackFileRow, i64)>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached(
                "SELECT file_id, date, bytes, rows FROM parquet_file WHERE unnamed_at IS NULL
                 ORDER BY date, file_id",
            )
            .map_err(io)?;
        rows.query_map([], |r| {
            Ok((
                r.get::<_, [u8; 16]>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map_err(io)?
        .map(|row| {
            let (file, date, bytes, count) = row.map_err(io)?;
            let bytes = u64::try_from(bytes).map_err(|_| StoreError::Damaged)?;
            let info = PackFileRow {
                date,
                bytes,
                unnamed_at: None,
            };
            Ok((FileId(file), info, count))
        })
        .collect()
    }

    fn live_versions(&mut self) -> Result<Vec<VersionId>, StoreError> {
        self.versions(
            "SELECT current FROM vcon WHERE current IS NOT NULL AND deleted_at IS NULL
             ORDER BY current",
            [],
        )
    }

    fn places(
        &mut self,
    ) -> Result<Vec<(VersionId, Result<Option<Packed>, StoreError>)>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached(&format!(
                "SELECT v.version, {PLACE} FROM version v {PLACE_JOIN} ORDER BY v.version"
            ))
            .map_err(io)?;
        rows.query_map([], |r| Ok((VersionId(r.get(0)?), raw_place(r, 1)?)))
            .map_err(io)?
            .map(|row| {
                let (version, place) = row.map_err(io)?;
                Ok((version, packed(place)))
            })
            .collect()
    }

    fn count_versions(&mut self) -> Result<u64, StoreError> {
        self.count_of("SELECT count(*) FROM version", [])
    }

    fn count_packed(&mut self) -> Result<u64, StoreError> {
        self.count_of(
            "SELECT count(*) FROM version WHERE place_file IS NOT NULL",
            [],
        )
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
            .cached_row(
                "SELECT v.uuid, v.stored_at, c.hold != 0, c.deleted_at IS NOT NULL
                 FROM version v JOIN vcon c ON c.uuid = v.uuid
                 WHERE v.version = CAST(?1 AS BLOB) AND v.hidden_at IS NOT NULL",
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
            .cached_run("DELETE FROM version WHERE version = ?1", [version.0])
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
            .cached_run("DELETE FROM vcon WHERE uuid = ?1", [uuid.as_bytes()])
            .map_err(io)?;
        self.conn
            .cached_run(
                "INSERT INTO tombstone (uuid, purged_at) VALUES (?1, ?2)",
                params![uuid.as_bytes(), at],
            )
            .map_err(io)?;
        Ok(())
    }

    fn latest(&mut self, uuid: Uuid) -> Result<Option<VersionId>, StoreError> {
        self.conn
            .cached_row(
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
            .prepare_cached(
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
            .cached_run(
                "UPDATE version SET hidden_at = NULL WHERE version = ?1",
                [version.0],
            )
            .map_err(io)?;
        Ok(())
    }

    fn undelete(&mut self, uuid: Uuid, version: VersionId) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "UPDATE vcon SET current = ?1, deleted_at = NULL WHERE uuid = ?2",
                (version.0, uuid.as_bytes()),
            )
            .map_err(io)?;
        Ok(())
    }

    fn set_hold(&mut self, uuid: Uuid, on: bool) -> Result<(), StoreError> {
        self.conn
            .cached_run(
                "UPDATE vcon SET hold = ?1 WHERE uuid = ?2",
                (on, uuid.as_bytes()),
            )
            .map_err(io)?;
        Ok(())
    }

    fn page(&mut self, query: &PageQuery) -> Result<Vec<ListedRow>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached(
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
            .prepare_cached(
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
            .cached_run(
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
            .prepare_cached(
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

    fn exported(
        &mut self,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<ExportedRow>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached(&format!(
                "SELECT v.version, v.uuid, v.form, v.gzip, v.size, v.stored_at,
                            c.current IS v.version AND c.deleted_at IS NULL, v.wrapped_key,
                            {PLACE}
                     FROM version v JOIN vcon c ON c.uuid = v.uuid {PLACE_JOIN}
                     WHERE (?1 IS NULL OR v.stored_at >= ?1) AND (?2 IS NULL OR v.stored_at < ?2)
                     ORDER BY v.stored_at, v.version"
            ))
            .map_err(io)?;
        rows.query_map(params![since, until], |r| {
            Ok((
                ExportedRow {
                    version: VersionId(r.get(0)?),
                    uuid: Uuid::from_bytes(r.get(1)?),
                    form: r.get(2)?,
                    gzip: r.get(3)?,
                    size: r.get(4)?,
                    stored_at: r.get(5)?,
                    live: r.get(6)?,
                    wrapped: r.get(7)?,
                    place: None,
                },
                raw_place(r, 8)?,
            ))
        })
        .map_err(io)?
        .map(|row| {
            let (row, place) = row.map_err(io)?;
            Ok(ExportedRow {
                place: packed(place)?,
                ..row
            })
        })
        .collect()
    }

    fn purge_times(&mut self) -> Result<Vec<String>, StoreError> {
        let mut rows = self
            .conn
            .prepare_cached("SELECT at FROM audit WHERE action = ?1 AND status = 200")
            .map_err(io)?;
        rows.query_map([Action::Purge.as_str()], |r| r.get(0))
            .map_err(io)?
            .map(|row| row.map_err(io))
            .collect()
    }

    fn begin_part(&mut self) -> Result<(), StoreError> {
        self.conn
            .cached_run("SAVEPOINT part", [])
            .map(drop)
            .map_err(io)
    }

    fn release_part(&mut self) -> Result<(), StoreError> {
        self.conn
            .cached_run("RELEASE part", [])
            .map(drop)
            .map_err(io)
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

    /// With `Group`: numbered, COMMIT (frames written, not synced), the
    /// connection let go, then the wait for the group sync; with `Beside`,
    /// COMMIT syncs (FULL), which covers every frame before it as well.
    fn commit(mut self: Box<Self>) -> Result<(), CommitError> {
        let number = match self.durability {
            // Poisoned: nothing more is written to this catalog; Drop
            // rolls the transaction back.
            Durability::Group => Some(
                self.durable
                    .begin_commit()
                    .map_err(|_| CommitError::RolledBack)?,
            ),
            Durability::Beside => None,
        };
        #[cfg(test)]
        if let Some(outcome) = injected(&self.conn, self.fault) {
            self.open = false;
            if let Some(n) = number {
                self.durable.written(n);
            }
            return outcome;
        }
        let outcome = end(&self.conn);
        if let Some(n) = number {
            self.durable.written(n);
        }
        self.open = outcome == Err(CommitError::RolledBack) && !self.conn.is_autocommit();
        outcome?;
        self.conn = Held::Gone;
        match number {
            Some(n) => self.durable.wait(n).map_err(|_| CommitError::Unknown),
            None => Ok(()),
        }
    }

    fn end(mut self: Box<Self>) -> Result<(), StoreError> {
        self.finish().map_err(io)
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

/// A version's place as selected by `PLACE`: its file, row group and
/// row, and the file's date and length from its row.
type RawPlace = (
    Option<[u8; 16]>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    Option<i64>,
);

/// The columns of a version's place, `v` being its version row and `f` its
/// file's, which `PLACE_JOIN` joins.
const PLACE: &str = "v.place_file, v.place_group, v.place_row, f.date, f.bytes";

/// Joins a version row `v` to its pack file's row, if it has one.
const PLACE_JOIN: &str = "LEFT JOIN parquet_file f ON f.file_id = v.place_file";

/// The `PLACE` columns of `r`, starting at column `at`.
fn raw_place(r: &rusqlite::Row, at: usize) -> rusqlite::Result<RawPlace> {
    Ok((
        r.get(at)?,
        r.get(at + 1)?,
        r.get(at + 2)?,
        r.get(at + 3)?,
        r.get(at + 4)?,
    ))
}

/// A version's place: None with no file; a file with no row of its own,
/// or a part of the place missing or out of range, is damage.
fn packed(raw: RawPlace) -> Result<Option<Packed>, StoreError> {
    match raw {
        (None, None, None, _, _) => Ok(None),
        (Some(file), Some(group), Some(row), Some(date), Some(bytes)) => {
            let damaged = |_| StoreError::Damaged;
            Ok(Some(Packed {
                file: FileId(file),
                date,
                bytes: u64::try_from(bytes).map_err(damaged)?,
                place: super::pack::Place {
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

/// The tables of server.sqlite, besides the admin audit log. A key column
/// says NOT NULL because SQLite lets a PRIMARY KEY other than an INTEGER
/// one hold NULL. Times are nanoseconds since the epoch. A retention period
/// is written as `Retention` writes it; NULL means the server default. A
/// metadata level is written as `Metadata` writes it; NULL means `none`.
/// A role cap is written as `Roles` writes it; NULL means no cap. Its column
/// is declared after the closing line break of `metadata` because that is
/// the text SQLite keeps for a column an upgrade adds to a table.
pub(crate) const REGISTRY_TABLES: &str = "
CREATE TABLE tenant (
    id BLOB PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    retention TEXT,
    created_at INTEGER NOT NULL,
    tenant_key_id BLOB NOT NULL,
    wrapped_tenant_key BLOB NOT NULL,
    pending_wrapped_tenant_key BLOB,
    key_since INTEGER NOT NULL,
    metadata TEXT
, role_cap TEXT);
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
, expires_at INTEGER, last_used_at INTEGER);
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
        let schema = format!(
            "{REGISTRY_TABLES}{}{}{}{}",
            catalog::USER_TABLES,
            catalog::TENANT_SETTINGS,
            catalog::SESSIONS,
            audit::SCHEMA
        );
        let path = dir.join("server.sqlite");
        let conn = catalog::open(&path, &catalog_key, &schema).map_err(|e| match e {
            catalog::CatalogError::WrongKey => RegistryError::WrongKey,
            e => RegistryError::failed(OpenStep::OpenCatalog, path.clone(), e),
        })?;
        conn.execute_batch(PENDING_TENANTS)
            .map_err(|e| RegistryError::failed(OpenStep::OpenCatalog, path.clone(), e))?;
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
        conn.cached_run(how, []).map_err(tenant_io)?;
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
            .prepare_cached("SELECT id FROM tenant ORDER BY id")
            .map_err(tenant_io)?;
        rows.query_map([], |r| r.get::<_, [u8; 16]>(0))
            .map_err(tenant_io)?
            .map(|row| row.map(TenantId).map_err(tenant_io))
            .collect()
    }

    /// Always held: the data directory's lock (datadir.rs) lets one
    /// process serve it, so no other server shares the work.
    fn lead(&self, _job: Job, _ttl: std::time::Duration) -> Result<bool, TenantError> {
        Ok(true)
    }

    /// Nothing to give up: no other process waits for the work.
    fn step_down(&self, _job: Job) -> Result<(), TenantError> {
        Ok(())
    }

    #[cfg(test)]
    fn fail_commit(&self, nth: usize, how: CommitError) {
        *self.fault.lock().expect("fault") = Some((nth, how));
    }

    #[cfg(test)]
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
impl LocalRegistry {
    /// The connection to server.sqlite, for tests that read or break the
    /// registry directly.
    pub(crate) fn local_conn(&self) -> MutexGuard<'_, Connection> {
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
    wrapped_tenant_key, pending_wrapped_tenant_key, key_since, metadata, role_cap";

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
        metadata: r.get(8)?,
        role_cap: r.get(9)?,
    })
}

const KEY_COLUMNS: &str =
    "id, tenant, token_hash, scopes, label, created_at, revoked_at, expires_at, last_used_at";

fn key_row(r: &rusqlite::Row) -> rusqlite::Result<KeyRow> {
    Ok(KeyRow {
        id: KeyId(r.get(0)?),
        tenant: r.get::<_, Option<[u8; 16]>>(1)?.map(TenantId),
        token_hash: r.get(2)?,
        scopes: r.get(3)?,
        label: r.get(4)?,
        created_at: r.get(5)?,
        revoked_at: r.get(6)?,
        expires_at: r.get(7)?,
        last_used_at: r.get(8)?,
    })
}

const USER_COLUMNS: &str = "id, label, issuer, subject, created_at, deactivated_at";

fn user_row(r: &rusqlite::Row) -> rusqlite::Result<UserRow> {
    let issuer: Option<String> = r.get(2)?;
    let subject: Option<String> = r.get(3)?;
    Ok(UserRow {
        id: UserId(r.get(0)?),
        label: r.get(1)?,
        // The table's CHECK holds that both are there or neither is.
        identity: issuer.zip(subject),
        created_at: r.get(4)?,
        deactivated_at: r.get(5)?,
    })
}

const MEMBERSHIP_COLUMNS: &str = "user, tenant, roles, granted_at";

fn membership_row(r: &rusqlite::Row) -> rusqlite::Result<MembershipRow> {
    Ok(MembershipRow {
        user: UserId(r.get(0)?),
        tenant: r.get::<_, Option<[u8; 16]>>(1)?.map(TenantId),
        roles: r.get(2)?,
        granted_at: r.get(3)?,
    })
}

const SESSION_COLUMNS: &str = "id_hash, user, tenant, scopes, created_at, last_seen_at, \
     idle_expires_at, expires_at, idle_seconds, amr, csrf_hash";

fn session_row(r: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id_hash: r.get(0)?,
        user: UserId(r.get(1)?),
        tenant: r.get::<_, Option<[u8; 16]>>(2)?.map(TenantId),
        scopes: r.get(3)?,
        created_at: r.get(4)?,
        last_seen_at: r.get(5)?,
        idle_expires_at: r.get(6)?,
        expires_at: r.get(7)?,
        idle_seconds: r.get(8)?,
        amr: r.get(9)?,
        csrf_hash: r.get(10)?,
    })
}

impl RegistryTx for LocalRegistryTx<'_> {
    fn tenant(&mut self, id: TenantId) -> Result<Option<TenantRow>, TenantError> {
        self.conn
            .cached_row(
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
            .prepare_cached(&format!(
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
            .cached_row("SELECT 1 FROM tenant WHERE name = ?1", [name], |_| Ok(()))
            .optional()
            .map_err(tenant_io)?
            .is_some())
    }

    fn insert_tenant(&mut self, row: &TenantRow) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "INSERT INTO tenant (id, name, retention, created_at, tenant_key_id,
                     wrapped_tenant_key, pending_wrapped_tenant_key, key_since, metadata,
                     role_cap)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    row.id.0,
                    row.name,
                    row.retention,
                    row.created_at,
                    row.tenant_key_id.bytes(),
                    row.wrapped,
                    row.pending,
                    row.key_since,
                    row.metadata,
                    row.role_cap,
                ],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn set_role_cap(&mut self, id: TenantId, cap: Option<&str>) -> Result<bool, TenantError> {
        let changed = self
            .conn
            .cached_run(
                "UPDATE tenant SET role_cap = ?2 WHERE id = ?1",
                params![id.0, cap],
            )
            .map_err(tenant_io)?;
        Ok(changed != 0)
    }

    fn set_metadata(&mut self, id: TenantId, level: Option<&str>) -> Result<bool, TenantError> {
        let changed = self
            .conn
            .cached_run(
                "UPDATE tenant SET metadata = ?2 WHERE id = ?1",
                params![id.0, level],
            )
            .map_err(tenant_io)?;
        Ok(changed != 0)
    }

    fn set_retention(
        &mut self,
        id: TenantId,
        retention: Option<&str>,
    ) -> Result<bool, TenantError> {
        let changed = self
            .conn
            .cached_run(
                "UPDATE tenant SET retention = ?2 WHERE id = ?1",
                params![id.0, retention],
            )
            .map_err(tenant_io)?;
        Ok(changed != 0)
    }

    fn delete_tenant(&mut self, id: TenantId, at: i64) -> Result<(), TenantError> {
        self.conn
            .cached_run("DELETE FROM tenant WHERE id = ?1", [id.0])
            .map_err(tenant_io)?;
        self.conn
            .cached_run(
                "UPDATE api_key SET revoked_at = ?2 WHERE tenant = ?1 AND revoked_at IS NULL",
                params![id.0, at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn set_pending(&mut self, id: TenantId, pending: &[u8]) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "UPDATE tenant SET pending_wrapped_tenant_key = ?1 WHERE id = ?2",
                params![pending, id.0],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn promote_pending(&mut self, id: TenantId, key: KeyId, at: i64) -> Result<(), TenantError> {
        self.conn
            .cached_run(
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
            .cached_row(
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
            .cached_run(
                "INSERT INTO master_key_use (master_key_id, wraps, since) VALUES (?1, 1, ?2)
                 ON CONFLICT (master_key_id) DO UPDATE SET wraps = wraps + 1",
                params![key.bytes(), at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn add_creating(&mut self, id: TenantId, at: i64) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "INSERT INTO pending_tenant (id, started_at) VALUES (?1, ?2)",
                params![id.0, at],
            )
            .map(drop)
            .map_err(tenant_io)
    }

    fn drop_creating(&mut self, id: TenantId) -> Result<bool, TenantError> {
        self.conn
            .cached_run("DELETE FROM pending_tenant WHERE id = ?1", [id.0])
            .map(|deleted| deleted > 0)
            .map_err(tenant_io)
    }

    fn creating(&mut self) -> Result<Vec<(TenantId, i64)>, TenantError> {
        let mut rows = self
            .conn
            .prepare_cached("SELECT id, started_at FROM pending_tenant ORDER BY id")
            .map_err(tenant_io)?;
        rows.query_map([], |r| {
            let id: Vec<u8> = r.get(0)?;
            Ok((id, r.get::<_, i64>(1)?))
        })
        .map_err(tenant_io)?
        .map(|row| {
            let (id, at) = row.map_err(tenant_io)?;
            let id: [u8; 16] = id.try_into().map_err(tenant_io)?;
            Ok((TenantId(id), at))
        })
        .collect()
    }

    fn master_since(&mut self, key: KeyId) -> Result<Option<i64>, TenantError> {
        self.conn
            .cached_row(
                "SELECT since FROM master_key_use WHERE master_key_id = ?1",
                [key.bytes()],
                |r| r.get(0),
            )
            .optional()
            .map_err(tenant_io)
    }

    fn key(&mut self, id: KeyId) -> Result<Option<KeyRow>, TenantError> {
        self.conn
            .cached_row(
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
            .prepare_cached(&format!(
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
            .cached_run(
                &format!(
                    "INSERT INTO api_key ({KEY_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
                ),
                params![
                    row.id.bytes(),
                    row.tenant.map(|t| t.0),
                    row.token_hash,
                    row.scopes,
                    row.label,
                    row.created_at,
                    row.revoked_at,
                    row.expires_at,
                    row.last_used_at,
                ],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn touch_key(&mut self, id: KeyId, at: i64) -> Result<bool, TenantError> {
        let changed = self
            .conn
            .cached_run(
                "UPDATE api_key SET last_used_at = ?2
                 WHERE id = ?1 AND (last_used_at IS NULL OR last_used_at < ?2)",
                params![id.bytes(), at],
            )
            .map_err(tenant_io)?;
        Ok(changed > 0)
    }

    fn revoke_key(&mut self, id: KeyId, at: i64) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "UPDATE api_key SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id.bytes(), at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn user(&mut self, id: UserId) -> Result<Option<UserRow>, TenantError> {
        self.conn
            .cached_row(
                &format!("SELECT {USER_COLUMNS} FROM user_account WHERE id = ?1"),
                [id.0],
                user_row,
            )
            .optional()
            .map_err(tenant_io)
    }

    fn users(&mut self) -> Result<Vec<UserRow>, TenantError> {
        let mut rows = self
            .conn
            .prepare_cached(&format!(
                "SELECT {USER_COLUMNS} FROM user_account ORDER BY created_at, id"
            ))
            .map_err(tenant_io)?;
        rows.query_map([], user_row)
            .map_err(tenant_io)?
            .map(|row| row.map_err(tenant_io))
            .collect()
    }

    fn identity_taken(&mut self, issuer: &str, subject: &str) -> Result<bool, TenantError> {
        self.conn
            .cached_row(
                "SELECT count(*) > 0 FROM user_account
                 WHERE issuer = ?1 AND subject = ?2 AND deactivated_at IS NULL",
                params![issuer, subject],
                |r| r.get(0),
            )
            .map_err(tenant_io)
    }

    fn insert_user(&mut self, row: &UserRow) -> Result<(), TenantError> {
        let (issuer, subject) = match &row.identity {
            Some((issuer, subject)) => (Some(issuer), Some(subject)),
            None => (None, None),
        };
        self.conn
            .cached_run(
                &format!(
                    "INSERT INTO user_account ({USER_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
                ),
                params![
                    row.id.0,
                    row.label,
                    issuer,
                    subject,
                    row.created_at,
                    row.deactivated_at,
                ],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn deactivate_user(&mut self, id: UserId, at: i64) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "UPDATE user_account SET deactivated_at = ?2
                 WHERE id = ?1 AND deactivated_at IS NULL",
                params![id.0, at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn membership(
        &mut self,
        user: UserId,
        tenant: Option<TenantId>,
    ) -> Result<Option<MembershipRow>, TenantError> {
        self.conn
            .cached_row(
                &format!(
                    "SELECT {MEMBERSHIP_COLUMNS} FROM membership WHERE user = ?1 AND tenant IS ?2"
                ),
                params![user.0, tenant.map(|t| t.0)],
                membership_row,
            )
            .optional()
            .map_err(tenant_io)
    }

    fn memberships(&mut self) -> Result<Vec<MembershipRow>, TenantError> {
        let mut rows = self
            .conn
            .prepare_cached(&format!(
                "SELECT {MEMBERSHIP_COLUMNS} FROM membership ORDER BY user, granted_at, tenant"
            ))
            .map_err(tenant_io)?;
        rows.query_map([], membership_row)
            .map_err(tenant_io)?
            .map(|row| row.map_err(tenant_io))
            .collect()
    }

    fn insert_membership(&mut self, row: &MembershipRow) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                &format!("INSERT INTO membership ({MEMBERSHIP_COLUMNS}) VALUES (?1, ?2, ?3, ?4)"),
                params![
                    row.user.0,
                    row.tenant.map(|t| t.0),
                    row.roles,
                    row.granted_at
                ],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn set_roles(&mut self, row: &MembershipRow) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "UPDATE membership SET roles = ?3 WHERE user = ?1 AND tenant IS ?2",
                params![row.user.0, row.tenant.map(|t| t.0), row.roles],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_membership(
        &mut self,
        user: UserId,
        tenant: Option<TenantId>,
    ) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "DELETE FROM membership WHERE user = ?1 AND tenant IS ?2",
                params![user.0, tenant.map(|t| t.0)],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_memberships_of_user(&mut self, user: UserId) -> Result<(), TenantError> {
        self.conn
            .cached_run("DELETE FROM membership WHERE user = ?1", [user.0])
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_memberships_in(&mut self, tenant: TenantId) -> Result<(), TenantError> {
        self.conn
            .cached_run("DELETE FROM membership WHERE tenant = ?1", [tenant.0])
            .map_err(tenant_io)?;
        Ok(())
    }

    fn session(&mut self, id_hash: &[u8]) -> Result<Option<SessionRow>, TenantError> {
        self.conn
            .cached_row(
                &format!("SELECT {SESSION_COLUMNS} FROM session WHERE id_hash = ?1"),
                [id_hash],
                session_row,
            )
            .optional()
            .map_err(tenant_io)
    }

    fn insert_session(&mut self, row: &SessionRow) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                &format!(
                    "INSERT INTO session ({SESSION_COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
                ),
                params![
                    row.id_hash,
                    row.user.0,
                    row.tenant.map(|t| t.0),
                    row.scopes,
                    row.created_at,
                    row.last_seen_at,
                    row.idle_expires_at,
                    row.expires_at,
                    row.idle_seconds,
                    row.amr,
                    row.csrf_hash
                ],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn touch_session(
        &mut self,
        id_hash: &[u8],
        last_seen_at: i64,
        idle_expires_at: i64,
    ) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "UPDATE session SET last_seen_at = ?2, idle_expires_at = ?3 WHERE id_hash = ?1",
                params![id_hash, last_seen_at, idle_expires_at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_session(&mut self, id_hash: &[u8]) -> Result<(), TenantError> {
        self.conn
            .cached_run("DELETE FROM session WHERE id_hash = ?1", [id_hash])
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_sessions_of_user(&mut self, user: UserId) -> Result<(), TenantError> {
        self.conn
            .cached_run("DELETE FROM session WHERE user = ?1", [user.0])
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_sessions_in(&mut self, tenant: TenantId) -> Result<(), TenantError> {
        self.conn
            .cached_run("DELETE FROM session WHERE tenant = ?1", [tenant.0])
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_sessions_ended(&mut self, at: i64) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "DELETE FROM session WHERE idle_expires_at <= ?1 OR expires_at <= ?1",
                [at],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn settings(&mut self, tenant: TenantId) -> Result<Vec<(String, String)>, TenantError> {
        let mut rows = self
            .conn
            .prepare_cached(
                "SELECT name, value FROM tenant_setting WHERE tenant = ?1 ORDER BY name",
            )
            .map_err(tenant_io)?;
        rows.query_map([tenant.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(tenant_io)?
            .map(|row| row.map_err(tenant_io))
            .collect()
    }

    fn put_setting(
        &mut self,
        tenant: TenantId,
        name: &str,
        value: &str,
    ) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "INSERT INTO tenant_setting (tenant, name, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT (tenant, name) DO UPDATE SET value = excluded.value",
                params![tenant.0, name, value],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_setting(&mut self, tenant: TenantId, name: &str) -> Result<(), TenantError> {
        self.conn
            .cached_run(
                "DELETE FROM tenant_setting WHERE tenant = ?1 AND name = ?2",
                params![tenant.0, name],
            )
            .map_err(tenant_io)?;
        Ok(())
    }

    fn delete_settings_in(&mut self, tenant: TenantId) -> Result<(), TenantError> {
        self.conn
            .cached_run("DELETE FROM tenant_setting WHERE tenant = ?1", [tenant.0])
            .map_err(tenant_io)?;
        Ok(())
    }

    fn record_times(&mut self, action: Action, status: u16) -> Result<Vec<String>, TenantError> {
        let mut rows = self
            .conn
            .prepare_cached("SELECT at FROM audit WHERE action = ?1 AND status = ?2")
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

    fn count_many(
        &mut self,
        entry: &Entry,
        now: OffsetDateTime,
        times: u64,
    ) -> Result<(), AuditError> {
        audit::count_many(&self.conn, entry, now, times)
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
    objects: ObjectPlace,
}

impl LocalTenantSpace {
    /// The space under `<data>/tenants`, made if new and made durable,
    /// with each tenant's objects kept in `objects`.
    pub(crate) fn open(
        data: &Path,
        fs: Arc<dyn Fs>,
        objects: ObjectPlace,
    ) -> Result<LocalTenantSpace, RegistryError> {
        let tenants = data.join("tenants");
        fs.create_dir_all(&tenants)
            .map_err(|e| RegistryError::failed(OpenStep::MakeTenantsDir, tenants.clone(), e))?;
        fs.sync_dir(data)
            .map_err(|e| RegistryError::failed(OpenStep::SyncDataDir, data.to_path_buf(), e))?;
        Ok(LocalTenantSpace {
            tenants,
            fs,
            objects,
        })
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
        drop(self.open(id, key, true, clock)?);
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
        match &self.objects {
            ObjectPlace::Local => {
                let open = if recover {
                    Store::open
                } else {
                    Store::open_beside
                };
                open(&self.dir(id), id, key, self.fs.clone(), clock)
            }
            #[cfg(feature = "s3")]
            ObjectPlace::S3(s3) => {
                let catalog_key = envelope::catalog_key(&key).map_err(|_| StoreError::Io)?;
                let durability = if recover {
                    Durability::Group
                } else {
                    Durability::Beside
                };
                let catalog =
                    LocalCatalog::open(&self.catalog(id), catalog_key, &*self.fs, durability)?;
                Store::open_on(
                    id,
                    key,
                    Box::new(catalog),
                    Arc::new(s3.tenant(id)),
                    Some(Arc::new(s3.packs(id))),
                    self.fs.clone(),
                    clock,
                    recover,
                )
            }
        }
    }

    /// A name that is not a tenant id is not ours and is left out. With
    /// a bucket, a tenant that has objects there but no directory here is
    /// present too: a create or removal cut off part way leaves one.
    fn present(&self) -> Result<Vec<TenantId>, TenantError> {
        let mut ids: Vec<TenantId> = self
            .fs
            .list(&self.tenants)
            .map_err(tenant_io)?
            .iter()
            .filter_map(|name| name.to_str().and_then(TenantId::parse))
            .collect();
        #[cfg(feature = "s3")]
        if let ObjectPlace::S3(s3) = &self.objects {
            ids.extend(s3.tenants().map_err(tenant_io)?);
        }
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        Ok(ids)
    }

    /// The bucket's objects go first: a directory left behind is found and
    /// removed at the next start, as is a tenant with objects but no
    /// directory.
    fn remove(&self, id: TenantId) -> Result<(), TenantError> {
        match &self.objects {
            ObjectPlace::Local => self.fs.remove_dir_all(&self.dir(id)).map_err(tenant_io),
            #[cfg(feature = "s3")]
            ObjectPlace::S3(s3) => {
                s3.remove_tenant(id).map_err(tenant_io)?;
                match self.fs.remove_dir_all(&self.dir(id)) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(tenant_io(e)),
                    _ => Ok(()),
                }
            }
        }
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
            .prepare_cached("SELECT id, wrapped_tenant_key, pending_wrapped_tenant_key FROM tenant")
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
            tx.cached_run(
                "UPDATE tenant SET wrapped_tenant_key = ?1, pending_wrapped_tenant_key = ?2
                 WHERE id = ?3",
                params![rewrap(&wrapped)?, pending, id.0],
            )
            .map_err(tenant_io)?;
        }
        tx.cached_run("DELETE FROM master_key_use", [])
            .map_err(tenant_io)?;
        tx.cached_run(
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
    match conn.cached_run("COMMIT", []) {
        Ok(_) => Ok(()),
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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::fs::OsFs;

    /// The catalog key under `parent`: the same key at each call.
    fn under(parent: &Key) -> Key {
        envelope::catalog_key(parent).expect("a catalog key")
    }

    /// A new tenant catalog in `dir`, and the key its catalog key is
    /// derived from.
    fn tenant_catalog(dir: &tempfile::TempDir) -> (std::path::PathBuf, Key, LocalCatalog) {
        let path = dir.path().join("catalog.sqlite");
        let key = Key::generate().expect("a key");
        let catalog = LocalCatalog::open(&path, under(&key), &OsFs, Durability::Group)
            .expect("the catalog opens");
        (path, key, catalog)
    }

    /// Whether the catalog at `conn` has a table `name`.
    fn has_table(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |r| r.get(0),
        )
        .expect("reads the schema")
    }

    #[test]
    fn a_new_tenant_catalog_has_no_pending_write() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.snapshot().expect("a snapshot");
        assert_eq!(tx.pending().expect("reads"), vec![], "no write is pending");
    }

    #[test]
    fn a_pending_write_reads_back_with_its_time_until_it_is_dropped() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let version = VersionId::generate();
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending(version, 42).expect("adds the row");
        tx.commit().expect("commits");
        let mut tx = catalog.immediate().expect("a write");
        assert_eq!(
            tx.pending().expect("reads"),
            vec![(version, 42)],
            "the pending row, with the time its write began"
        );
        assert!(
            tx.drop_pending(version).expect("drops"),
            "the row was there"
        );
        assert_eq!(tx.pending().expect("reads"), vec![], "no row is left");
        assert!(
            !tx.drop_pending(version).expect("drops"),
            "a second drop finds no row"
        );
    }

    #[test]
    fn pending_writes_read_back_by_version_id() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        let high = VersionId([0xff; 16]);
        let low = VersionId([0x01; 16]);
        tx.add_pending(high, 1).expect("adds");
        tx.add_pending(low, 2).expect("adds");
        assert_eq!(
            tx.pending().expect("reads"),
            vec![(low, 2), (high, 1)],
            "by version id"
        );
    }

    #[test]
    fn a_second_pending_row_for_one_version_is_refused() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let version = VersionId::generate();
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending(version, 1).expect("adds");
        assert!(
            matches!(tx.add_pending(version, 2), Err(StoreError::Io)),
            "a version's write begins once"
        );
    }

    #[test]
    fn a_pending_row_of_a_rolled_back_write_is_not_kept() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending(VersionId::generate(), 1).expect("adds");
        drop(tx);
        let mut tx = catalog.snapshot().expect("a snapshot");
        assert_eq!(tx.pending().expect("reads"), vec![], "nothing committed");
    }

    #[test]
    fn a_pending_row_keeps_a_time_at_either_end_of_the_range() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        let (a, b) = (VersionId([0x01; 16]), VersionId([0x02; 16]));
        tx.add_pending(a, i64::MIN).expect("adds");
        tx.add_pending(b, i64::MAX).expect("adds");
        assert_eq!(
            tx.pending().expect("reads"),
            vec![(a, i64::MIN), (b, i64::MAX)],
            "both ends of the range read back"
        );
    }

    #[test]
    fn the_pending_table_is_not_in_the_file_and_the_version_stays() {
        let dir = tempfile::tempdir().expect("a directory");
        let (path, key, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending(VersionId::generate(), 1).expect("adds");
        tx.commit().expect("commits");
        let conn = catalog::open(&path, &under(&key), "").expect("a second connection");
        assert!(
            !has_table(&conn, "pending_write"),
            "the file's schema has no pending table"
        );
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("the version");
        assert_eq!(version, 10, "the catalog's schema version is unchanged");
        assert_eq!(catalog::SCHEMA_VERSION, 10, "and this build's");
    }

    #[test]
    fn a_pending_row_goes_with_the_catalog_that_added_it() {
        let dir = tempfile::tempdir().expect("a directory");
        let (path, key, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending(VersionId::generate(), 1).expect("adds");
        tx.add_wrap(KeyId([7; 8])).expect("a row of the file");
        tx.commit().expect("commits");
        drop(catalog);
        let catalog =
            LocalCatalog::open(&path, under(&key), &OsFs, Durability::Group).expect("opens again");
        let mut tx = catalog.snapshot().expect("a snapshot");
        assert_eq!(
            tx.pending().expect("reads"),
            vec![],
            "a new process has no write under way"
        );
        assert_eq!(tx.wraps(KeyId([7; 8])).expect("reads"), 1, "the file's row");
    }

    #[test]
    fn a_new_tenant_catalog_has_no_pending_file() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.snapshot().expect("a snapshot");
        assert_eq!(
            tx.pending_files().expect("reads"),
            vec![],
            "no pack file is being written"
        );
    }

    #[test]
    fn a_pending_file_reads_back_with_its_time_until_it_is_dropped() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let file = FileId::generate();
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending_file(file, 42).expect("adds the row");
        tx.commit().expect("commits");
        let mut tx = catalog.immediate().expect("a write");
        assert_eq!(
            tx.pending_files().expect("reads"),
            vec![(file, 42)],
            "the pending row, with the time the file's writing began"
        );
        assert_eq!(
            tx.pending().expect("reads"),
            vec![],
            "a pending file is no pending write"
        );
        assert!(
            tx.drop_pending_file(file).expect("drops"),
            "the row was there"
        );
        assert_eq!(tx.pending_files().expect("reads"), vec![], "no row is left");
        assert!(
            !tx.drop_pending_file(file).expect("drops"),
            "a second drop finds no row"
        );
    }

    #[test]
    fn pending_files_read_back_by_file_id() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        let (high, low) = (FileId([0xff; 16]), FileId([0x01; 16]));
        tx.add_pending_file(high, 1).expect("adds");
        tx.add_pending_file(low, 2).expect("adds");
        assert_eq!(
            tx.pending_files().expect("reads"),
            vec![(low, 2), (high, 1)],
            "by file id"
        );
    }

    #[test]
    fn a_second_pending_row_for_one_file_is_refused() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let file = FileId::generate();
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending_file(file, 1).expect("adds");
        assert!(
            matches!(tx.add_pending_file(file, 2), Err(StoreError::Io)),
            "a file's writing begins once"
        );
    }

    #[test]
    fn a_pending_file_row_of_a_rolled_back_transaction_is_not_kept() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, _, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending_file(FileId::generate(), 1).expect("adds");
        drop(tx);
        let mut tx = catalog.snapshot().expect("a snapshot");
        assert_eq!(
            tx.pending_files().expect("reads"),
            vec![],
            "nothing committed"
        );
    }

    #[test]
    fn the_pending_file_table_is_not_in_the_file_and_goes_with_the_catalog() {
        let dir = tempfile::tempdir().expect("a directory");
        let (path, key, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending_file(FileId::generate(), 1).expect("adds");
        tx.commit().expect("commits");
        let conn = catalog::open(&path, &under(&key), "").expect("a second connection");
        assert!(
            !has_table(&conn, "pending_file"),
            "the file's schema has no pending file table"
        );
        drop(conn);
        drop(catalog);
        let catalog = LocalCatalog::open(&path, under(&key)).expect("opens again");
        let mut tx = catalog.snapshot().expect("a snapshot");
        assert_eq!(
            tx.pending_files().expect("reads"),
            vec![],
            "a new process has no pack file under way"
        );
    }

    #[test]
    fn a_registry_has_no_pending_table() {
        let dir = tempfile::tempdir().expect("a directory");
        let key = Key::generate().expect("a key");
        drop(LocalRegistry::open(dir.path(), under(&key)).expect("a registry"));
        let path = dir.path().join("server.sqlite");
        let conn = catalog::open(&path, &under(&key), "").expect("opens");
        assert!(
            !has_table(&conn, "pending_write"),
            "a registry has no pending table"
        );
    }

    #[test]
    fn a_backup_of_a_catalog_holds_no_pending_row() {
        let dir = tempfile::tempdir().expect("a directory");
        let (_, key, catalog) = tenant_catalog(&dir);
        let mut tx = catalog.immediate().expect("a write");
        tx.add_pending(VersionId::generate(), 1).expect("adds");
        tx.commit().expect("commits");
        let copy = dir.path().join("copy.sqlite");
        catalog.backup_into(&copy).expect("backs up");
        let conn = catalog::open(&copy, &under(&key), "").expect("opens the copy");
        assert!(
            !has_table(&conn, "pending_write"),
            "the copy has no pending table"
        );
    }

    /// Only an absent file is a missing object; every other error the
    /// system gives is a failure of the storage, as S3's are.
    #[test]
    fn only_an_absent_file_is_a_missing_object() {
        assert!(
            matches!(
                object_error(std::io::Error::from_raw_os_error(libc::ENOENT)),
                ObjectError::Missing
            ),
            "ENOENT is a missing object"
        );
        for errno in [libc::EIO, libc::EACCES, libc::EISDIR, libc::ENOSPC] {
            match object_error(std::io::Error::from_raw_os_error(errno)) {
                ObjectError::Io(e) => {
                    assert_eq!(e.raw_os_error(), Some(errno), "errno {errno} is kept");
                }
                ObjectError::Missing => panic!("errno {errno} is not a missing object"),
            }
        }
    }

    /// An object file the process may not read is a storage failure, with
    /// the system's error, not a missing object.
    #[test]
    fn an_object_file_the_process_may_not_read_is_a_failure_not_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let objects = LocalObjects::new(dir.path().to_owned(), Arc::new(OsFs));
        let version = VersionId::generate();
        objects.put(version, b"sealed bytes").expect("put");
        let file = dir.path().join(version.to_string());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).expect("chmod");
        let read = objects.get(version);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        match read {
            Err(ObjectError::Io(e)) => {
                assert_eq!(
                    e.kind(),
                    std::io::ErrorKind::PermissionDenied,
                    "the system's error: {e}"
                );
            }
            Err(ObjectError::Missing) => panic!("a file that is there is not missing"),
            Ok(_) => panic!("read a file without permission: is the test run as root?"),
        }
        assert_eq!(
            objects.get(version).expect("reads once allowed"),
            b"sealed bytes",
            "the file reads back once it may be read"
        );
        std::fs::remove_file(&file).expect("remove");
        assert!(
            matches!(objects.get(version), Err(ObjectError::Missing)),
            "a removed file is a missing object"
        );
    }

    /// A tenant catalog in a crash filesystem's root, whose WAL syncs the
    /// filesystem holds, fails and counts.
    struct Synced {
        dir: tempfile::TempDir,
        fs: Arc<crate::fs::CrashFs>,
        key: Key,
        catalog: LocalCatalog,
    }

    impl Synced {
        fn path(&self) -> PathBuf {
            self.dir.path().join("catalog.sqlite")
        }

        fn wal(&self) -> PathBuf {
            self.dir.path().join("catalog.sqlite-wal")
        }

        fn syncs(&self) -> usize {
            self.fs.syncs(&self.wal())
        }

        /// Another catalog on the same file, as another process opens it.
        fn open(&self, durability: Durability) -> LocalCatalog {
            LocalCatalog::open(&self.path(), under(&self.key), &*self.fs, durability)
                .expect("opens again")
        }
    }

    fn synced(durability: Durability) -> Synced {
        let dir = tempfile::tempdir().expect("a directory");
        let fs = Arc::new(crate::fs::CrashFs::new(dir.path()));
        let key = Key::generate().expect("a key");
        let catalog = LocalCatalog::open(
            &dir.path().join("catalog.sqlite"),
            under(&key),
            &*fs,
            durability,
        )
        .expect("the catalog opens");
        Synced {
            dir,
            fs,
            key,
            catalog,
        }
    }

    /// A commit that writes a frame to the WAL: one more wrap under a key.
    fn wrap(catalog: &LocalCatalog, key: KeyId) -> Result<(), CommitError> {
        let mut tx = catalog.immediate().expect("a write begins");
        tx.add_wrap(key).expect("adds a wrap");
        tx.commit()
    }

    fn a_key() -> KeyId {
        Key::generate().expect("a key").id()
    }

    fn synchronous(catalog: &LocalCatalog) -> i64 {
        catalog
            .local_conn()
            .pragma_query_value(None, "synchronous", |r| r.get(0))
            .expect("reads synchronous")
    }

    #[test]
    fn a_catalog_written_by_its_own_process_runs_with_synchronous_normal() {
        let s = synced(Durability::Group);
        assert_eq!(
            synchronous(&s.catalog),
            1,
            "NORMAL: COMMIT does not sync; the group sync does, after the connection is let go"
        );
    }

    #[test]
    fn a_catalog_opened_beside_a_server_keeps_synchronous_full() {
        let s = synced(Durability::Beside);
        assert_eq!(
            synchronous(&s.catalog),
            2,
            "FULL: the server never sees a commit of a command beside it before it is durable"
        );
    }

    #[test]
    fn opening_a_catalog_syncs_its_wal_then_its_directory() {
        let s = synced(Durability::Group);
        let order: Vec<crate::fs::Op> =
            s.fs.log()
                .into_iter()
                .filter(|op| matches!(op, crate::fs::Op::SyncData(_) | crate::fs::Op::SyncDir(_)))
                .collect();
        assert_eq!(
            order,
            [
                crate::fs::Op::SyncData(s.wal()),
                crate::fs::Op::SyncDir(s.dir.path().to_owned()),
            ],
            "the WAL a crash could otherwise lose with commits a restart saw, then its entry"
        );
    }

    #[test]
    fn a_catalog_whose_wal_cannot_be_synced_does_not_open() {
        let dir = tempfile::tempdir().expect("a directory");
        let fs = crate::fs::CrashFs::new(dir.path());
        fs.fail_syncs();
        let key = Key::generate().expect("a key");
        assert!(
            LocalCatalog::open(
                &dir.path().join("catalog.sqlite"),
                under(&key),
                &fs,
                Durability::Group
            )
            .is_err(),
            "commits a restart can read must be durable before anything is served"
        );
    }

    #[test]
    fn a_catalog_whose_directory_cannot_be_synced_does_not_open() {
        let dir = tempfile::tempdir().expect("a directory");
        let tenant = dir.path().join("tenant");
        std::fs::create_dir(&tenant).expect("made on disk, outside the model");
        let fs = crate::fs::CrashFs::new(dir.path());
        let key = Key::generate().expect("a key");
        assert!(
            LocalCatalog::open(
                &tenant.join("catalog.sqlite"),
                under(&key),
                &fs,
                Durability::Group
            )
            .is_err(),
            "the directory sync failed (the model has no such directory)"
        );
    }

    #[test]
    fn a_commit_returns_only_after_its_sync() {
        let s = synced(Durability::Group);
        let key = a_key();
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, key));
            s.fs.await_held(1);
            assert!(
                !committing.is_finished(),
                "no return while the sync is held"
            );
            s.fs.release_syncs();
            committing
                .join()
                .expect("the commit thread ends")
                .expect("commits once synced");
        });
        assert_eq!(s.syncs(), 2, "the open's sync and the commit's");
    }

    #[test]
    fn a_snapshot_reads_while_a_commit_waits_for_its_sync_and_ends_after_it() {
        let s = synced(Durability::Group);
        let key = a_key();
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, key));
            s.fs.await_held(1);
            let (read, seen) = std::sync::mpsc::channel();
            let catalog = &s.catalog;
            let ending = t.spawn(move || {
                let mut tx = catalog.snapshot()?;
                read.send(tx.wraps(key)?).expect("the test listens");
                tx.end()
            });
            assert_eq!(
                seen.recv_timeout(std::time::Duration::from_secs(20))
                    .expect("the snapshot reads while the commit's sync is held"),
                1,
                "the commit is visible before it is durable"
            );
            s.fs.release_syncs();
            committing.join().expect("ends").expect("commits");
            ending
                .join()
                .expect("ends")
                .expect("the snapshot ends once what it saw is durable");
        });
    }

    #[test]
    fn a_snapshot_that_saw_an_unsynced_commit_does_not_end_before_the_sync() {
        let s = synced(Durability::Group);
        let key = a_key();
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, key));
            s.fs.await_held(1);
            let ending = t.spawn(|| s.catalog.snapshot()?.end());
            // The syncing commit's wait is held; the snapshot's end waits
            // for it, and holds no sync of its own.
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while std::time::Instant::now() < deadline {
                assert!(!ending.is_finished(), "nothing it saw is durable yet");
                std::thread::yield_now();
            }
            s.fs.release_syncs();
            committing.join().expect("ends").expect("commits");
            ending.join().expect("ends").expect("ends after the sync");
        });
    }

    #[test]
    fn a_snapshot_that_saw_no_commit_ends_without_a_sync() {
        let s = synced(Durability::Group);
        let before = s.syncs();
        let mut tx = s.catalog.snapshot().expect("a snapshot");
        assert_eq!(tx.wraps(a_key()).expect("reads"), 0, "nothing there");
        tx.end().expect("ends");
        assert_eq!(s.syncs(), before, "everything it saw was durable already");
    }

    #[test]
    fn a_snapshot_after_a_synced_commit_ends_without_another_sync() {
        let s = synced(Durability::Group);
        wrap(&s.catalog, a_key()).expect("commits");
        let before = s.syncs();
        s.catalog
            .snapshot()
            .expect("a snapshot")
            .end()
            .expect("ends");
        assert_eq!(s.syncs(), before, "the commit's own sync covers it");
    }

    #[test]
    fn a_dropped_snapshot_still_waits_for_what_it_saw() {
        let s = synced(Durability::Group);
        let key = a_key();
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, key));
            s.fs.await_held(1);
            let dropping = t.spawn(|| drop(s.catalog.snapshot().expect("a snapshot")));
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while std::time::Instant::now() < deadline {
                assert!(!dropping.is_finished(), "a drop is an end too");
                std::thread::yield_now();
            }
            s.fs.release_syncs();
            committing.join().expect("ends").expect("commits");
            dropping.join().expect("the drop ends after the sync");
        });
    }

    #[test]
    fn a_write_dropped_without_commit_still_waits_for_what_it_saw() {
        let s = synced(Durability::Group);
        let (first, second) = (a_key(), a_key());
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, first));
            s.fs.await_held(1);
            let dropping = t.spawn(|| {
                let mut tx = s.catalog.immediate().expect("a write begins");
                tx.add_wrap(second).expect("adds");
                // Refused part-way, as a request that fails after a read.
                drop(tx);
            });
            s.catalog.durable.await_waiting(2);
            assert!(
                !dropping.is_finished(),
                "the drop ends only once what the write read is durable"
            );
            s.fs.release_syncs();
            committing.join().expect("ends").expect("commits");
            dropping.join().expect("the drop ends after the sync");
        });
        let mut tx = s.catalog.snapshot().expect("a snapshot");
        assert_eq!(
            (
                tx.wraps(first).expect("reads"),
                tx.wraps(second).expect("reads")
            ),
            (1, 0),
            "the commit is kept and the dropped write rolled back"
        );
    }

    /// The accepted decision for a write dropped without commit whose wait
    /// fails: Drop cannot return the error, so the request is refused with
    /// the error it already had, and the poisoned catalog refuses every
    /// later request until it is reopened.
    #[test]
    fn a_write_dropped_without_commit_after_a_failed_sync_leaves_later_requests_refused() {
        let s = synced(Durability::Group);
        let (first, second) = (a_key(), a_key());
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, first));
            s.fs.await_held(1);
            let dropping = t.spawn(|| {
                let mut tx = s.catalog.immediate().expect("a write begins");
                tx.add_wrap(second).expect("adds");
                drop(tx);
            });
            s.catalog.durable.await_waiting(2);
            s.fs.fail_syncs();
            assert_eq!(
                committing.join().expect("ends"),
                Err(CommitError::Unknown),
                "the commit it saw"
            );
            dropping
                .join()
                .expect("the drop ends; its failed wait is not lost");
        });
        s.fs.heal_syncs();
        s.fs.release_syncs();
        assert!(
            matches!(s.catalog.immediate(), Err(StoreError::Io)),
            "the next write is refused"
        );
        assert!(
            matches!(s.catalog.snapshot(), Err(StoreError::Io)),
            "the next read is refused"
        );
        let again = s.open(Durability::Group);
        let mut tx = again.snapshot().expect("a reopened catalog reads");
        assert_eq!(
            tx.wraps(second).expect("reads"),
            0,
            "the dropped write is not there"
        );
    }

    #[test]
    fn a_close_waits_for_the_sync_of_a_commit_made_before_it() {
        let s = synced(Durability::Group);
        let key = a_key();
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, key));
            s.fs.await_held(1);
            let closing = t.spawn(|| s.catalog.close());
            s.catalog.await_sync_waiters(2);
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while std::time::Instant::now() < deadline {
                assert!(
                    !closing.is_finished(),
                    "no close while the commit's sync is held"
                );
                std::thread::yield_now();
            }
            s.fs.release_syncs();
            assert_eq!(committing.join().expect("ends"), Ok(()), "committed");
            assert_eq!(
                closing.join().expect("ends"),
                Ok(()),
                "closed once the commit is durable"
            );
        });
    }

    #[test]
    fn a_close_whose_commit_sync_fails_while_it_waits_answers_the_error() {
        let s = synced(Durability::Group);
        let key = a_key();
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, key));
            s.fs.await_held(1);
            let closing = t.spawn(|| s.catalog.close());
            s.catalog.await_sync_waiters(2);
            s.fs.fail_syncs();
            assert_eq!(
                committing.join().expect("ends"),
                Err(CommitError::Unknown),
                "the commit's sync failed"
            );
            assert_eq!(
                closing.join().expect("ends"),
                Err(StoreError::Io),
                "the close says a commit may be lost"
            );
        });
    }

    #[test]
    fn a_close_whose_last_sync_failed_answers_the_error() {
        let s = synced(Durability::Group);
        s.fs.fail_syncs();
        assert!(wrap(&s.catalog, a_key()).is_err(), "the sync failed");
        s.fs.heal_syncs();
        assert_eq!(
            s.catalog.close(),
            Err(StoreError::Io),
            "a close after a failed sync answers it, though syncs work again"
        );
    }

    #[test]
    fn a_close_with_nothing_pending_answers_ok_without_a_sync() {
        let s = synced(Durability::Group);
        wrap(&s.catalog, a_key()).expect("committed and synced");
        let before = s.syncs();
        assert_eq!(s.catalog.close(), Ok(()), "every commit is durable");
        assert_eq!(s.syncs(), before, "nothing to sync: the close made none");
    }

    #[test]
    fn a_close_of_a_catalog_opened_beside_a_server_answers_a_failed_sync() {
        let s = synced(Durability::Group);
        let beside = s.open(Durability::Beside);
        assert_eq!(beside.close(), Ok(()), "nothing failed");
        s.fs.fail_syncs();
        assert!(
            beside.snapshot().and_then(|tx| tx.end()).is_err(),
            "its end's sync failed"
        );
        s.fs.heal_syncs();
        assert_eq!(beside.close(), Err(StoreError::Io), "the close answers it");
    }

    #[test]
    fn a_failed_sync_makes_the_commit_unknown() {
        let s = synced(Durability::Group);
        s.fs.fail_syncs();
        assert_eq!(
            wrap(&s.catalog, a_key()),
            Err(CommitError::Unknown),
            "the frames are in the WAL; whether they reach the disk is not known"
        );
    }

    #[test]
    fn after_a_failed_sync_no_transaction_begins_until_the_catalog_is_reopened() {
        let s = synced(Durability::Group);
        s.fs.fail_syncs();
        assert!(wrap(&s.catalog, a_key()).is_err(), "the sync failed");
        s.fs.heal_syncs();
        assert!(
            matches!(s.catalog.immediate(), Err(StoreError::Io)),
            "no write begins on a poisoned catalog"
        );
        assert!(
            matches!(s.catalog.snapshot(), Err(StoreError::Io)),
            "nor a read"
        );
        let again = s.open(Durability::Group);
        wrap(&again, a_key()).expect("a reopened catalog commits again");
    }

    #[test]
    fn a_snapshot_that_saw_a_commit_whose_sync_failed_ends_with_an_error() {
        let s = synced(Durability::Group);
        let key = a_key();
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, key));
            s.fs.await_held(1);
            let ending = t.spawn(|| s.catalog.snapshot()?.end());
            s.catalog.durable.await_waiting(2);
            s.fs.fail_syncs();
            assert_eq!(
                committing.join().expect("ends"),
                Err(CommitError::Unknown),
                "the commit"
            );
            assert!(
                matches!(ending.join().expect("ends"), Err(StoreError::Io)),
                "the snapshot read what may be lost: its end fails, nothing is answered"
            );
        });
    }

    #[test]
    fn a_commit_begun_on_a_poisoned_catalog_keeps_nothing() {
        let s = synced(Durability::Group);
        let (first, second) = (a_key(), a_key());
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, first));
            s.fs.await_held(1);
            // The connection is free while the first commit waits: a second
            // write begins, then the first one's sync fails.
            let mut tx = s.catalog.immediate().expect("begins before the failure");
            tx.add_wrap(second).expect("adds");
            s.fs.fail_syncs();
            assert_eq!(
                committing.join().expect("ends"),
                Err(CommitError::Unknown),
                "the first commit"
            );
            assert_eq!(
                tx.commit(),
                Err(CommitError::RolledBack),
                "a commit on a poisoned catalog rolls back: nothing of it is kept"
            );
        });
        s.fs.heal_syncs();
        s.fs.release_syncs();
        let again = s.open(Durability::Group);
        let mut tx = again.snapshot().expect("a snapshot");
        assert_eq!(
            tx.wraps(second).expect("reads"),
            0,
            "the second write is not there"
        );
    }

    #[test]
    fn a_catalog_beside_a_server_syncs_the_wal_at_a_snapshot_end() {
        let s = synced(Durability::Beside);
        let server = s.open(Durability::Group);
        let before = s.syncs();
        wrap(&server, a_key()).expect("the server commits");
        s.catalog
            .snapshot()
            .expect("a snapshot")
            .end()
            .expect("ends");
        assert_eq!(
            s.syncs(),
            before + 2,
            "the server's commit synced once, and the command's end syncs again: it cannot count the server's commits"
        );
    }

    #[test]
    fn a_catalog_beside_a_server_does_not_sync_again_when_nothing_changed() {
        let s = synced(Durability::Beside);
        s.catalog
            .snapshot()
            .expect("a snapshot")
            .end()
            .expect("ends");
        let before = s.syncs();
        s.catalog
            .snapshot()
            .expect("a snapshot")
            .end()
            .expect("ends");
        assert_eq!(
            s.syncs(),
            before,
            "no other connection committed since the last sync, which covered everything"
        );
    }

    #[test]
    fn a_catalog_beside_a_server_syncs_its_own_commit_inside_commit() {
        let s = synced(Durability::Beside);
        let before = s.syncs();
        wrap(&s.catalog, a_key()).expect("commits");
        assert_eq!(
            s.syncs(),
            before,
            "FULL syncs inside COMMIT, before the server can see it: no sync of ours"
        );
    }

    #[test]
    fn an_export_waits_for_the_records_it_may_write() {
        let s = synced(Durability::Group);
        s.fs.hold_syncs();
        std::thread::scope(|t| {
            let committing = t.spawn(|| wrap(&s.catalog, a_key()));
            s.fs.await_held(1);
            let exporting = t.spawn(|| {
                let mut out = Vec::new();
                s.catalog.export(0, &mut out).map(|()| out)
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
            while std::time::Instant::now() < deadline {
                assert!(!exporting.is_finished(), "no output before the sync");
                std::thread::yield_now();
            }
            s.fs.release_syncs();
            committing.join().expect("ends").expect("commits");
            exporting.join().expect("ends").expect("exports");
        });
    }

    #[test]
    fn an_export_beside_a_server_syncs_before_writing() {
        let s = synced(Durability::Beside);
        let server = s.open(Durability::Group);
        let before = s.syncs();
        wrap(&server, a_key()).expect("the server commits");
        let mut out = Vec::new();
        s.catalog.export(0, &mut out).expect("exports");
        assert_eq!(
            s.syncs(),
            before + 2,
            "the server's sync, then the export's"
        );
    }

    #[test]
    fn a_backup_beside_a_server_syncs_what_it_copied() {
        let s = synced(Durability::Beside);
        let server = s.open(Durability::Group);
        let before = s.syncs();
        wrap(&server, a_key()).expect("the server commits");
        let to = tempfile::tempdir().expect("a directory");
        s.catalog
            .backup_into(&to.path().join("catalog.sqlite"))
            .expect("backs up");
        assert_eq!(
            s.syncs(),
            before + 2,
            "the server's sync, then the backup's"
        );
    }

    #[test]
    fn a_backup_on_a_poisoned_catalog_fails() {
        let s = synced(Durability::Group);
        s.fs.fail_syncs();
        assert!(wrap(&s.catalog, a_key()).is_err(), "poisoned");
        let to = tempfile::tempdir().expect("a directory");
        assert!(
            s.catalog
                .backup_into(&to.path().join("catalog.sqlite"))
                .is_err(),
            "a copy could hold a commit that may be lost"
        );
    }
}
