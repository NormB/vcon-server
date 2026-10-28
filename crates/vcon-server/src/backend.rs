//! Where the server's data is kept: each tenant's catalog of rows and its
//! sealed objects, the registry of tenants and keys, and the places
//! tenants are made in.
//!
//! These are traits because Store and Tenants are written once over more
//! than one backend: the local data directory now, and a PostgreSQL
//! catalog and S3-compatible objects next. Store and Tenants hold every
//! rule and call these for each statement they need; a backend implements
//! statements, never a rule, so the rules cannot drift between backends.
//!
//! The local backend, SQLCipher catalog files and directories of object
//! files, is in `local`.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit::{Action, AuditError, Entry};
use crate::clock::Clock;
use crate::envelope::Key;
use crate::ids::{KeyId, TenantId, VersionId};
use crate::store::{OpenError, Store, StoreError};
use crate::tenants::TenantError;

#[cfg(test)]
mod conformance;
pub mod local;

/// A tenant's catalog row for a uuid, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VconRow {
    pub current: Option<VersionId>,
    pub deleted_at: Option<i64>,
}

/// A version row's columns that reading it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRow {
    /// The form's name as stored: unsigned, signed or encrypted.
    pub form: String,
    pub gzip: bool,
    pub wrapped: Vec<u8>,
}

/// A version row to add.
#[derive(Debug, Clone, Copy)]
pub struct NewVersion<'a> {
    pub version: VersionId,
    pub uuid: Uuid,
    pub form: &'static str,
    pub gzip: bool,
    pub size: i64,
    pub stored_at: i64,
    pub key: KeyId,
    pub wrapped: &'a [u8],
}

/// A hidden version as the purge rule reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenRow {
    pub uuid: Uuid,
    pub stored_at: i64,
    pub held: bool,
    pub deleted: bool,
}

/// A kept version as a legal listing reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeptRow {
    pub version: VersionId,
    pub stored_at: i64,
    pub hidden_at: Option<i64>,
}

/// A live uuid as the list reads it: its current version, the uuid's first
/// store time, and the current version's store time if its row is there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListedRow {
    pub uuid: Uuid,
    pub version: VersionId,
    pub stored_at: i64,
    pub updated_at: Option<i64>,
}

/// What a page of the list asks the catalog for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageQuery {
    /// Rows strictly before this (stored_at, uuid) in the list's order.
    pub before: Option<(i64, Uuid)>,
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// How many rows at most.
    pub rows: i64,
}

/// A version row as `check` and `examine` read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExaminedRow {
    pub version: VersionId,
    pub uuid: Uuid,
    pub stored_at: i64,
    pub hidden_at: Option<i64>,
    pub held: bool,
}

/// A data key to rewrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedRow {
    pub version: VersionId,
    pub uuid: Uuid,
    pub wrapped: Vec<u8>,
}

/// How a commit that did not succeed ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum CommitError {
    /// Nothing of the transaction was kept.
    RolledBack,
    /// The transaction may or may not have been kept.
    Unknown,
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CommitError::RolledBack => "the transaction was rolled back",
            CommitError::Unknown => "the transaction may or may not have been kept",
        })
    }
}

impl std::error::Error for CommitError {}

/// One tenant's catalog.
pub trait Catalog: Send + Sync {
    /// A transaction that writes: one at a time per tenant, so a tenant's
    /// writes queue here.
    fn immediate(&self) -> Result<Box<dyn CatalogTx + '_>, StoreError>;
    /// A transaction that only reads, seeing one consistent state.
    fn snapshot(&self) -> Result<Box<dyn CatalogTx + '_>, StoreError>;
    /// Start-up recovery of the catalog itself (local: the WAL is
    /// checkpointed with truncation).
    fn recover(&self) -> Result<(), StoreError>;
    /// What follows purges (local: the WAL is checkpointed with truncation,
    /// asking without waiting and waiting between tries).
    fn after_purge(&self) -> Result<(), StoreError>;
    /// Copies the catalog, as it is in one transaction, into a local
    /// catalog file at `to` under the same catalog key, and returns the
    /// version ids that copy holds.
    fn backup_into(&self, to: &Path) -> Result<Vec<VersionId>, StoreError>;
    /// Writes every audit record after the seq `after` as JSON lines,
    /// oldest first.
    fn export(&self, after: i64, out: &mut dyn Write) -> Result<(), AuditError>;
    /// The local connection, for tests that read or break the catalog
    /// directly.
    /// Makes the `nth` commit from now fail as `how` says: kept and then
    /// reported unknown, or rolled back.
    #[cfg(test)]
    fn fail_commit(&self, nth: usize, how: CommitError);
    #[cfg(test)]
    fn local_conn(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection>;
}

/// The statements Store runs in a catalog transaction. Dropping one
/// without `commit` rolls it back.
pub trait CatalogTx {
    /// Whether the uuid has a tombstone: it was purged.
    fn tombstoned(&mut self, uuid: Uuid) -> Result<bool, StoreError>;
    fn vcon_row(&mut self, uuid: Uuid) -> Result<Option<VconRow>, StoreError>;
    fn version_row(&mut self, version: VersionId) -> Result<Option<VersionRow>, StoreError>;
    /// Whether `version` is a kept version of `uuid`.
    fn has_version(&mut self, version: VersionId, uuid: Uuid) -> Result<bool, StoreError>;
    fn insert_version(&mut self, row: &NewVersion) -> Result<(), StoreError>;
    fn insert_vcon(
        &mut self,
        uuid: Uuid,
        current: VersionId,
        stored_at: i64,
    ) -> Result<(), StoreError>;
    fn set_current(&mut self, uuid: Uuid, current: VersionId) -> Result<(), StoreError>;
    fn hide_version(&mut self, version: VersionId, at: i64) -> Result<(), StoreError>;
    /// Marks the uuid deleted at `at` with no current version, and hides
    /// every version of it not hidden yet.
    fn mark_deleted(&mut self, uuid: Uuid, at: i64) -> Result<(), StoreError>;
    /// Wraps counted against a tenant key.
    fn wraps(&mut self, key: KeyId) -> Result<u64, StoreError>;
    fn add_wrap(&mut self, key: KeyId) -> Result<(), StoreError>;
    /// Every version id, in order.
    fn version_ids(&mut self) -> Result<Vec<VersionId>, StoreError>;
    fn count_versions(&mut self) -> Result<u64, StoreError>;
    fn count_held(&mut self) -> Result<u64, StoreError>;
    /// The hidden versions, of one uuid or of all.
    fn hidden_versions(&mut self, uuid: Option<Uuid>) -> Result<Vec<VersionId>, StoreError>;
    /// A hidden version with its uuid's hold and deletion; None if it is
    /// gone or not hidden.
    fn hidden_row(&mut self, version: VersionId) -> Result<Option<HiddenRow>, StoreError>;
    fn delete_version(&mut self, version: VersionId) -> Result<(), StoreError>;
    fn versions_of(&mut self, uuid: Uuid) -> Result<u64, StoreError>;
    /// Removes the uuid's row and adds its tombstone.
    fn tombstone(&mut self, uuid: Uuid, at: i64) -> Result<(), StoreError>;
    /// The latest kept version of a uuid, by store time and then by the
    /// order the rows were added.
    fn latest(&mut self, uuid: Uuid) -> Result<Option<VersionId>, StoreError>;
    /// Every kept version of a uuid, oldest first, in the same order.
    fn kept(&mut self, uuid: Uuid) -> Result<Vec<KeptRow>, StoreError>;
    fn unhide(&mut self, version: VersionId) -> Result<(), StoreError>;
    /// Makes `version` current and clears the uuid's deletion.
    fn undelete(&mut self, uuid: Uuid, version: VersionId) -> Result<(), StoreError>;
    fn set_hold(&mut self, uuid: Uuid, on: bool) -> Result<(), StoreError>;
    fn page(&mut self, query: &PageQuery) -> Result<Vec<ListedRow>, StoreError>;
    /// Up to `limit` data keys wrapped under `key`, in version order.
    fn wrapped_under(&mut self, key: KeyId, limit: usize) -> Result<Vec<WrappedRow>, StoreError>;
    fn set_wrapped(
        &mut self,
        version: VersionId,
        key: KeyId,
        wrapped: &[u8],
    ) -> Result<(), StoreError>;
    fn count_not_under(&mut self, key: KeyId) -> Result<u64, StoreError>;
    /// Every version row with its uuid's hold, in version order.
    fn examined(&mut self) -> Result<Vec<ExaminedRow>, StoreError>;
    /// The times of every successful purge recorded, as stored.
    fn purge_times(&mut self) -> Result<Vec<String>, StoreError>;
    /// Starts a nested part that `rollback_part` undoes and `release_part`
    /// keeps.
    fn begin_part(&mut self) -> Result<(), StoreError>;
    fn release_part(&mut self) -> Result<(), StoreError>;
    fn rollback_part(&mut self) -> Result<(), StoreError>;
    /// Appends an audit record.
    fn record(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError>;
    /// Adds one to a counted audit record.
    fn count(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError>;
    /// Writes at most `limit` audit records after the seq `after` as JSON
    /// lines, oldest first; None is no limit.
    fn audit_lines(
        &mut self,
        after: i64,
        limit: Option<u32>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError>;
    fn commit(self: Box<Self>) -> Result<(), CommitError>;
}

/// Why an object call failed.
#[derive(Debug)]
#[must_use]
pub enum ObjectError {
    /// No object has the name.
    Missing,
    /// The storage failed; the error says how.
    Io(std::io::Error),
}

impl std::fmt::Display for ObjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectError::Missing => f.write_str("no object has the name"),
            ObjectError::Io(e) => write!(f, "the object storage failed: {e}"),
        }
    }
}

impl std::error::Error for ObjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ObjectError::Missing => None,
            ObjectError::Io(e) => Some(e),
        }
    }
}

/// One tenant's sealed objects, named by version id.
pub trait Objects: Send + Sync {
    /// Writes a new object; it is durable once this returns Ok. On an
    /// error nothing of it is left that `list` would call a version.
    fn put(&self, version: VersionId, blob: &[u8]) -> Result<(), ObjectError>;
    fn get(&self, version: VersionId) -> Result<Vec<u8>, ObjectError>;
    /// Removes an object; one already absent is `Missing`.
    fn remove(&self, version: VersionId) -> Result<(), ObjectError>;
    /// Every name held, sorted, leftovers included.
    fn list(&self) -> Result<Vec<String>, ObjectError>;
    /// Removes a name `list` returned.
    fn remove_name(&self, name: &str) -> Result<(), ObjectError>;
    /// Makes removals durable.
    fn sync(&self) -> Result<(), ObjectError>;
}

/// A tenant's row in the registry, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRow {
    pub id: TenantId,
    pub name: String,
    /// The period as `Retention` writes it; None is the server default.
    pub retention: Option<String>,
    pub created_at: i64,
    pub tenant_key_id: KeyId,
    pub wrapped: Vec<u8>,
    pub pending: Option<Vec<u8>>,
    pub key_since: i64,
}

/// An access key's row in the registry, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRow {
    pub id: KeyId,
    pub tenant: Option<TenantId>,
    pub token_hash: Vec<u8>,
    /// The scopes as `Scopes` writes them.
    pub scopes: String,
    pub label: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

/// The registry: tenants, access keys, master-key use and the admin log.
pub trait Registry: Send + Sync {
    /// A transaction that writes: one at a time.
    fn immediate(&self) -> Result<Box<dyn RegistryTx + '_>, TenantError>;
    /// A transaction that only reads, seeing one consistent state.
    fn snapshot(&self) -> Result<Box<dyn RegistryTx + '_>, TenantError>;
    /// Writes every admin record after the seq `after` as JSON lines,
    /// oldest first.
    fn export(&self, after: i64, out: &mut dyn Write) -> Result<(), AuditError>;
    /// Copies the registry, as it is in one transaction, into a local
    /// registry file at `to` under the same key, and returns the tenants
    /// that copy lists, by id.
    fn backup_into(&self, to: &Path) -> Result<Vec<TenantId>, TenantError>;
    /// Makes the `nth` commit from now fail as `how` says.
    #[cfg(test)]
    fn fail_commit(&self, nth: usize, how: CommitError);
    /// The local connection, for tests that read or break the registry
    /// directly.
    #[cfg(test)]
    fn local_conn(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection>;
}

/// The statements Tenants runs in a registry transaction. Dropping one
/// without `commit` rolls it back.
pub trait RegistryTx {
    fn tenant(&mut self, id: TenantId) -> Result<Option<TenantRow>, TenantError>;
    /// Every tenant, by name.
    fn tenants(&mut self) -> Result<Vec<TenantRow>, TenantError>;
    fn name_taken(&mut self, name: &str) -> Result<bool, TenantError>;
    fn insert_tenant(&mut self, row: &TenantRow) -> Result<(), TenantError>;
    /// Sets a tenant's retention; false if no tenant has the id.
    fn set_retention(&mut self, id: TenantId, retention: Option<&str>)
    -> Result<bool, TenantError>;
    /// Removes a tenant's row and revokes its access keys at `at`.
    fn delete_tenant(&mut self, id: TenantId, at: i64) -> Result<(), TenantError>;
    fn set_pending(&mut self, id: TenantId, pending: &[u8]) -> Result<(), TenantError>;
    /// Makes the pending key current, as `key` since `at`.
    fn promote_pending(&mut self, id: TenantId, key: KeyId, at: i64) -> Result<(), TenantError>;
    /// Wraps counted against a master key.
    fn master_wraps(&mut self, key: KeyId) -> Result<u64, TenantError>;
    fn add_master_wrap(&mut self, key: KeyId, at: i64) -> Result<(), TenantError>;
    /// When the master key's use began, if it is recorded.
    fn master_since(&mut self, key: KeyId) -> Result<Option<i64>, TenantError>;
    fn key(&mut self, id: KeyId) -> Result<Option<KeyRow>, TenantError>;
    /// Every access key, or one tenant's, by creation time then id.
    fn keys(&mut self, tenant: Option<TenantId>) -> Result<Vec<KeyRow>, TenantError>;
    fn insert_key(&mut self, row: &KeyRow) -> Result<(), TenantError>;
    /// Revokes a key at `at` unless it is revoked already.
    fn revoke_key(&mut self, id: KeyId, at: i64) -> Result<(), TenantError>;
    /// The times of the admin records of `action` with `status`, as
    /// stored.
    fn record_times(&mut self, action: Action, status: u16) -> Result<Vec<String>, TenantError>;
    fn record(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError>;
    fn count(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError>;
    fn audit_lines(
        &mut self,
        after: i64,
        limit: Option<u32>,
        out: &mut dyn Write,
    ) -> Result<(), AuditError>;
    fn commit(self: Box<Self>) -> Result<(), CommitError>;
}

/// Where each tenant's catalog and objects are made, opened and removed.
pub trait TenantSpace: Send + Sync {
    /// Makes a new tenant's catalog and objects, durably.
    fn make(&self, id: TenantId, key: Key, clock: Arc<dyn Clock>) -> Result<(), OpenError>;
    /// Opens a tenant's Store; with `recover`, runs start-up recovery.
    fn open(
        &self,
        id: TenantId,
        key: Key,
        recover: bool,
        clock: Arc<dyn Clock>,
    ) -> Result<Store, OpenError>;
    /// Every tenant that has a catalog or objects here, by id.
    fn present(&self) -> Result<Vec<TenantId>, TenantError>;
    /// Removes a tenant's catalog and objects.
    fn remove(&self, id: TenantId) -> Result<(), TenantError>;
    /// Which catalog key the tenant's catalog is under now, `old` or
    /// `new`; under neither is an error.
    fn catalog_under(&self, id: TenantId, old: &Key, new: &Key) -> Result<Under, TenantError>;
    /// Moves the tenant's catalog from the catalog key `old` to `new`.
    fn reseal(&self, id: TenantId, old: &Key, new: &Key) -> Result<(), TenantError>;
}

/// The key a tenant catalog is under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Under {
    Old,
    New,
}
