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
use crate::ids::{FileId, KeyId, TenantId, UserId, VersionId};
use crate::store::{OpenError, Store, StoreError};
use crate::tenants::TenantError;

#[cfg(test)]
mod conformance;
pub mod local;
// Moving objects into a bucket, so only a build with S3 has it.
#[cfg(feature = "s3")]
pub(crate) mod migrate;
pub(crate) mod pack;
// The catalogs in PostgreSQL, a preview: the connector and the pool.
#[cfg(feature = "postgres")]
pub(crate) mod postgres;
#[cfg(feature = "s3")]
pub(crate) mod s3;

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
    /// Where its sealed blob is when it is packed; None: in its object.
    pub place: Option<Packed>,
}

/// A packed version's row in its pack file, and the file as its catalog
/// row records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub file: FileId,
    /// The UTC store date of the file's rows, as its name writes it.
    pub date: String,
    /// The file's length in bytes.
    pub bytes: u64,
    pub place: pack::Place,
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

/// A kept version as the export reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedRow {
    pub version: VersionId,
    pub uuid: Uuid,
    pub form: String,
    pub gzip: bool,
    pub size: i64,
    pub stored_at: i64,
    /// Its uuid's current version, and the uuid not deleted.
    pub live: bool,
    pub wrapped: Vec<u8>,
    /// Where its sealed blob is when it is packed; None: in its object.
    pub place: Option<Packed>,
}

/// A version still in its own object, as a pack run plans it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnpackedRow {
    pub version: VersionId,
    pub stored_at: i64,
    /// The body's size in bytes, as received.
    pub size: i64,
}

/// A recorded pack file, as its row says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFileRow {
    pub date: String,
    pub bytes: u64,
    /// When no place named it any more; None while one may.
    pub unnamed_at: Option<i64>,
}

/// A new pack file's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewPackFile<'a> {
    pub file: FileId,
    pub date: &'a str,
    pub rows: i64,
    pub bytes: i64,
    pub created_at: i64,
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
    /// version ids that copy holds, each with its place in that copy.
    fn backup_into(&self, to: &Path) -> Result<Vec<(VersionId, Option<Packed>)>, StoreError>;
    /// Makes the copy `backup_into` wrote at `copy` say that every version
    /// is in its own object: no place, no pack file, no purge left to
    /// finish.
    fn unpack_copy(&self, copy: &Path) -> Result<(), StoreError>;
    /// Writes every audit record after the seq `after` as JSON lines,
    /// oldest first.
    fn export(&self, after: i64, out: &mut dyn Write) -> Result<(), AuditError>;
    /// The local connection, for tests that read or break the catalog
    /// directly.
    /// Makes the `nth` commit from now fail as `how` says: kept and then
    /// reported unknown, or rolled back.
    #[cfg(test)]
    fn fail_commit(&self, nth: usize, how: CommitError);
    /// For tests: `busy` is sent a message each time a checkpoint after a
    /// purge finds a reader in its way and is about to wait.
    #[cfg(test)]
    fn on_checkpoint_wait(&self, busy: std::sync::mpsc::Sender<()>);
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
    /// Every version still in its own object, by store time and then
    /// version id.
    fn unpacked(&mut self) -> Result<Vec<UnpackedRow>, StoreError>;
    /// Whether `version` is kept and still in its own object.
    fn is_unpacked(&mut self, version: VersionId) -> Result<bool, StoreError>;
    fn insert_pack_file(&mut self, row: &NewPackFile) -> Result<(), StoreError>;
    /// Moves `version`'s place to `place` in `file`.
    fn set_place(
        &mut self,
        version: VersionId,
        file: FileId,
        place: pack::Place,
    ) -> Result<(), StoreError>;
    /// Every pack file the catalog records.
    fn pack_files(&mut self) -> Result<Vec<FileId>, StoreError>;
    /// A recorded pack file's row; None if it has none.
    fn pack_file(&mut self, file: FileId) -> Result<Option<PackFileRow>, StoreError>;
    /// Every kept version packed in `file`, with its store time and place,
    /// by row group and row.
    fn placed_in(&mut self, file: FileId) -> Result<Vec<(VersionId, i64, Packed)>, StoreError>;
    /// Lists `file` for a purge's rewrite; one listed already stays listed.
    fn add_purge_pending(&mut self, file: FileId) -> Result<(), StoreError>;
    fn purge_pending(&mut self) -> Result<Vec<FileId>, StoreError>;
    fn delete_purge_pending(&mut self, file: FileId) -> Result<(), StoreError>;
    /// Marks `file` as no longer named by any place, at `at`.
    fn unname_pack_file(&mut self, file: FileId, at: i64) -> Result<(), StoreError>;
    /// Every recorded file marked unnamed.
    fn unnamed_files(&mut self) -> Result<Vec<FileId>, StoreError>;
    fn delete_pack_file(&mut self, file: FileId) -> Result<(), StoreError>;
    /// Takes the next manifest number, never one given before.
    fn allocate_manifest(&mut self) -> Result<i64, StoreError>;
    /// The manifest numbers taken and not yet marked written, in order.
    fn unpublished_manifests(&mut self) -> Result<Vec<i64>, StoreError>;
    fn mark_published(&mut self, seq: i64, at: i64) -> Result<(), StoreError>;
    /// Every manifest marked written, with when, by number.
    fn published_manifests(&mut self) -> Result<Vec<(i64, i64)>, StoreError>;
    fn delete_manifest(&mut self, seq: i64) -> Result<(), StoreError>;
    /// Every pack file the catalog names (not unnamed), with its date, its
    /// rows and its length, by date and then file id.
    fn named_files(&mut self) -> Result<Vec<(FileId, PackFileRow, i64)>, StoreError>;
    /// The versions a read returns: current, of a uuid not deleted, in
    /// version order.
    fn live_versions(&mut self) -> Result<Vec<VersionId>, StoreError>;
    /// Every version id with its place, in version order; a place that is
    /// not whole is `Damaged` for that version alone.
    #[expect(clippy::type_complexity, reason = "one result per version")]
    fn places(
        &mut self,
    ) -> Result<Vec<(VersionId, Result<Option<Packed>, StoreError>)>, StoreError>;
    fn count_versions(&mut self) -> Result<u64, StoreError>;
    /// The versions whose row names a pack file.
    fn count_packed(&mut self) -> Result<u64, StoreError>;
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
    /// Every kept version stored at or after `since` and before `until`,
    /// by store time and then version id.
    fn exported(
        &mut self,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<ExportedRow>, StoreError>;
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
    /// Removes an object. One already absent is `Missing`, or Ok where the
    /// storage cannot tell (S3 answers success either way); callers treat
    /// both as removed.
    fn remove(&self, version: VersionId) -> Result<(), ObjectError>;
    /// Every name held, sorted, leftovers included.
    fn list(&self) -> Result<Vec<String>, ObjectError>;
    /// Removes a name `list` returned.
    fn remove_name(&self, name: &str) -> Result<(), ObjectError>;
    /// Makes removals durable.
    fn sync(&self) -> Result<(), ObjectError>;
}

/// One tenant's pack files, each named by `pack::name` under the tenant's
/// own directory. A file never changes once written.
pub trait Packs: Send + Sync + std::fmt::Debug {
    /// `len` bytes of the file `name` from byte `start`; fewer if the file
    /// ends sooner. `Missing` if no file has the name.
    fn read(&self, name: &str, start: u64, len: u64) -> Result<Vec<u8>, ObjectError>;
    /// The file's length; None if no file has the name.
    fn size(&self, name: &str) -> Result<Option<u64>, ObjectError>;
    /// Writes a new file, durable once this returns, only if no file has
    /// the name (`If-None-Match: *` on S3). A name already taken is
    /// `Ok(false)` and writes nothing.
    fn put_new(&self, name: &str, bytes: &[u8]) -> Result<bool, ObjectError>;
    /// Removes a file; one already gone is removed too.
    fn remove(&self, name: &str) -> Result<(), ObjectError>;
    /// Every file name under the tenant's `parquet/`, as `pack::name`
    /// writes them and any other, sorted.
    fn list(&self) -> Result<Vec<String>, ObjectError>;
}

/// A tenant's row in the registry, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRow {
    pub id: TenantId,
    pub name: String,
    /// The period as `Retention` writes it; None is the server default.
    pub retention: Option<String>,
    /// The metadata level as `Metadata` writes it; None is `none`.
    pub metadata: Option<String>,
    /// The role cap as `Roles` writes it; None is no cap.
    pub role_cap: Option<String>,
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

/// A user's row in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRow {
    pub id: UserId,
    pub label: String,
    /// The identity provider's issuer and its subject for the user; None
    /// for a local account.
    pub identity: Option<(String, String)>,
    pub created_at: i64,
    pub deactivated_at: Option<i64>,
}

/// A membership's row in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipRow {
    pub user: UserId,
    /// None for an operator membership.
    pub tenant: Option<TenantId>,
    /// The roles, as `Roles` writes them.
    pub roles: String,
    pub granted_at: i64,
}

/// A session's row in the registry: one for each live session of a user
/// who signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    /// The SHA-256 of the session's id; the id itself is not kept.
    pub id_hash: Vec<u8>,
    pub user: UserId,
    /// The tenant of the membership the session is for; None for an
    /// operator membership.
    pub tenant: Option<TenantId>,
    /// The scopes the session was given when it started, as `Scopes`
    /// writes them.
    pub scopes: String,
    pub created_at: i64,
    pub last_seen_at: i64,
    /// When the session ends unless a request is seen before.
    pub idle_expires_at: i64,
    /// When the session ends whatever is seen.
    pub expires_at: i64,
    /// The time a request adds to `idle_expires_at`.
    pub idle_seconds: i64,
    /// The authentication methods the sign-in showed, joined with spaces.
    pub amr: String,
    /// The SHA-256 of the session's CSRF token.
    pub csrf_hash: Vec<u8>,
}

/// The registry: tenants, access keys, users, their memberships and
/// sessions, master-key use and the admin log.
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
    /// Sets a tenant's metadata level; false if no tenant has the id.
    fn set_metadata(&mut self, id: TenantId, level: Option<&str>) -> Result<bool, TenantError>;
    /// Sets a tenant's role cap, or with None removes it; false if no tenant
    /// has the id.
    fn set_role_cap(&mut self, id: TenantId, cap: Option<&str>) -> Result<bool, TenantError>;
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
    fn user(&mut self, id: UserId) -> Result<Option<UserRow>, TenantError>;
    /// Every user, oldest first.
    fn users(&mut self) -> Result<Vec<UserRow>, TenantError>;
    /// Whether an active user has this identity.
    fn identity_taken(&mut self, issuer: &str, subject: &str) -> Result<bool, TenantError>;
    fn insert_user(&mut self, row: &UserRow) -> Result<(), TenantError>;
    /// Deactivates a user at `at` unless it is deactivated already.
    fn deactivate_user(&mut self, id: UserId, at: i64) -> Result<(), TenantError>;
    /// The user's membership in `tenant`, or as an operator for None.
    fn membership(
        &mut self,
        user: UserId,
        tenant: Option<TenantId>,
    ) -> Result<Option<MembershipRow>, TenantError>;
    /// Every membership, each user's oldest first.
    fn memberships(&mut self) -> Result<Vec<MembershipRow>, TenantError>;
    fn insert_membership(&mut self, row: &MembershipRow) -> Result<(), TenantError>;
    /// Sets the roles of the membership `row` names by its user and
    /// tenant.
    fn set_roles(&mut self, row: &MembershipRow) -> Result<(), TenantError>;
    fn delete_membership(
        &mut self,
        user: UserId,
        tenant: Option<TenantId>,
    ) -> Result<(), TenantError>;
    /// Removes every membership a user holds.
    fn delete_memberships_of_user(&mut self, user: UserId) -> Result<(), TenantError>;
    /// Removes every membership in a tenant.
    fn delete_memberships_in(&mut self, tenant: TenantId) -> Result<(), TenantError>;
    /// The session whose id has this SHA-256, ended or not.
    fn session(&mut self, id_hash: &[u8]) -> Result<Option<SessionRow>, TenantError>;
    fn insert_session(&mut self, row: &SessionRow) -> Result<(), TenantError>;
    /// Sets when a session was last seen and when it ends unless seen
    /// again.
    fn touch_session(
        &mut self,
        id_hash: &[u8],
        last_seen_at: i64,
        idle_expires_at: i64,
    ) -> Result<(), TenantError>;
    /// Removes one session, if it is there.
    fn delete_session(&mut self, id_hash: &[u8]) -> Result<(), TenantError>;
    /// Removes every session of a user, in every tenant and as an
    /// operator.
    fn delete_sessions_of_user(&mut self, user: UserId) -> Result<(), TenantError>;
    /// Removes every session in a tenant.
    fn delete_sessions_in(&mut self, tenant: TenantId) -> Result<(), TenantError>;
    /// Removes every session either of whose limits is at or before `at`.
    fn delete_sessions_ended(&mut self, at: i64) -> Result<(), TenantError>;
    /// The settings a tenant has set, as (name, value) by name. The table
    /// does not know tenants or names: the caller checks both.
    fn settings(&mut self, tenant: TenantId) -> Result<Vec<(String, String)>, TenantError>;
    /// Sets one of a tenant's settings, over the value it had.
    fn put_setting(&mut self, tenant: TenantId, name: &str, value: &str)
    -> Result<(), TenantError>;
    /// Removes one of a tenant's settings, if it was set.
    fn delete_setting(&mut self, tenant: TenantId, name: &str) -> Result<(), TenantError>;
    /// Removes every setting of a tenant.
    fn delete_settings_in(&mut self, tenant: TenantId) -> Result<(), TenantError>;
    /// The times of the admin records of `action` with `status`, as
    /// stored.
    fn record_times(&mut self, action: Action, status: u16) -> Result<Vec<String>, TenantError>;
    fn record(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError>;
    fn count(&mut self, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError>;
    /// Adds `times` to a counted audit record in the minute of `now`.
    fn count_many(
        &mut self,
        entry: &Entry,
        now: OffsetDateTime,
        times: u64,
    ) -> Result<(), AuditError>;
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

/// Where each tenant's sealed objects are kept, as the configuration says.
#[derive(Debug, Clone, Default)]
pub(crate) enum ObjectPlace {
    /// Files in each tenant's directory.
    #[default]
    Local,
    /// An S3-compatible bucket, under each tenant's own name.
    #[cfg(feature = "s3")]
    S3(Arc<s3::S3>),
}

/// The object place `storage` names, reached and checked: for a bucket,
/// the secret key is read from its file, the client made, and the bucket's
/// versioning read, which also shows that the bucket can be reached.
///
/// # Errors
///
/// A `StorageError` that says which setting or step failed.
pub(crate) fn objects_for(
    storage: &crate::config::Storage,
    #[cfg_attr(
        not(feature = "s3"),
        expect(
            unused_variables,
            reason = "only a bucket is bound to a data directory"
        )
    )]
    data_dir: &Path,
) -> Result<ObjectPlace, crate::config::StorageError> {
    match &storage.objects {
        crate::config::ObjectStore::Local => Ok(ObjectPlace::Local),
        #[cfg(feature = "s3")]
        crate::config::ObjectStore::S3(settings) => {
            s3::place(settings, storage.parquet.is_some(), data_dir)
        }
        #[cfg(not(feature = "s3"))]
        crate::config::ObjectStore::S3(_) => Err(crate::config::StorageError(
            "storage.objects is \"s3\", but this build has no S3 support (cargo feature s3)"
                .to_owned(),
        )),
    }
}

/// The catalog `catalog` names, checked before anything is opened. The
/// local catalog needs nothing more. A PostgreSQL catalog is a preview in
/// this release: its secret files are read, a connection is made and the
/// server checked, and then the start is refused with what was reached,
/// since the catalogs are not kept there yet.
///
/// # Errors
///
/// A `StorageError` for a PostgreSQL catalog: the step that refused it, or
/// the preview's refusal after a good connection.
pub(crate) async fn catalog_for(
    catalog: &crate::config::CatalogStore,
) -> Result<(), crate::config::StorageError> {
    match catalog {
        crate::config::CatalogStore::Local => Ok(()),
        #[cfg(feature = "postgres")]
        crate::config::CatalogStore::Postgres(settings) => Err(postgres::preview(settings).await),
        #[cfg(not(feature = "postgres"))]
        crate::config::CatalogStore::Postgres(_) => Err(crate::config::StorageError(
            "catalog.backend is \"postgres\", but this build has no PostgreSQL support (cargo \
             feature postgres)"
                .to_owned(),
        )),
    }
}

/// [`catalog_for`] for a command, which runs outside any runtime: a
/// PostgreSQL catalog is checked on a runtime of its own, made for it.
///
/// # Errors
///
/// As [`catalog_for`], or a `StorageError` if that runtime cannot start.
pub(crate) fn catalog_for_command(
    catalog: &crate::config::CatalogStore,
) -> Result<(), crate::config::StorageError> {
    if *catalog == crate::config::CatalogStore::Local {
        return Ok(());
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| crate::config::StorageError(format!("the runtime cannot start: {e}")))?
        .block_on(catalog_for(catalog))
}

/// `vcon-server migrate`: applies the migrations of the PostgreSQL catalog
/// `catalog` names, on a runtime of its own, and gives the line to print.
///
/// # Errors
///
/// A `StorageError` when the catalog is local, this build has no PostgreSQL
/// support, or the step that refused it says why.
pub(crate) fn migrate_for_command(
    catalog: &crate::config::CatalogStore,
) -> Result<serde_json::Value, crate::config::StorageError> {
    match catalog {
        crate::config::CatalogStore::Local => Err(crate::config::StorageError(
            "migrate is for a PostgreSQL catalog: catalog.backend is \"local\"".to_owned(),
        )),
        #[cfg(feature = "postgres")]
        crate::config::CatalogStore::Postgres(settings) => {
            let migrated = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| crate::config::StorageError(format!("the runtime cannot start: {e}")))?
                .block_on(postgres::migrate(settings))?;
            Ok(serde_json::json!({
                "migrated": migrated.from != migrated.to,
                "from": migrated.from,
                "to": migrated.to,
                "message": migrated.message(),
            }))
        }
        #[cfg(not(feature = "postgres"))]
        crate::config::CatalogStore::Postgres(_) => Err(crate::config::StorageError(
            "catalog.backend is \"postgres\", but this build has no PostgreSQL support (cargo \
             feature postgres)"
                .to_owned(),
        )),
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use crate::config::CatalogStore;

    #[tokio::test]
    async fn the_local_catalog_needs_nothing_more() {
        assert_eq!(
            catalog_for(&CatalogStore::Local).await,
            Ok(()),
            "the SQLCipher files open as before"
        );
        assert_eq!(
            catalog_for_command(&CatalogStore::Local),
            Ok(()),
            "and a command makes no runtime for it"
        );
    }

    #[test]
    fn migrate_is_refused_with_a_local_catalog_and_says_why() {
        let crate::config::StorageError(message) =
            migrate_for_command(&CatalogStore::Local).expect_err("refused");
        assert!(
            message.contains("PostgreSQL catalog") && message.contains("\"local\""),
            "names the setting: {message}"
        );
    }

    /// Settings for a PostgreSQL catalog in `dir`, at a port of the
    /// loopback address nothing listens on.
    #[cfg(feature = "postgres")]
    fn unreachable(dir: &Path) -> crate::config::PostgresSettings {
        use std::os::unix::fs::PermissionsExt;
        let password = dir.join("pg.password");
        std::fs::write(&password, "p\n").expect("writes");
        std::fs::set_permissions(&password, std::fs::Permissions::from_mode(0o600)).expect("mode");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("binds")
            .local_addr()
            .expect("an address")
            .port();
        crate::config::PostgresSettings {
            hosts: vec!["127.0.0.1".to_owned()],
            port,
            dbname: "vcon".to_owned(),
            user: "vcon_app".to_owned(),
            ca_file: None,
            password_file: Some(password),
            client_cert_file: None,
            client_key_file: None,
            pool: 1,
            connect_timeout: std::time::Duration::from_secs(1),
            statement_timeout: std::time::Duration::from_secs(1),
            lock_timeout: std::time::Duration::from_secs(1),
        }
    }

    /// A command runs outside any runtime; the check makes its own.
    #[cfg(feature = "postgres")]
    #[test]
    fn a_command_with_a_postgres_catalog_is_refused_with_why() {
        let dir = tempfile::tempdir().expect("dir");
        let settings = unreachable(dir.path());
        let crate::config::StorageError(message) =
            catalog_for_command(&CatalogStore::Postgres(Box::new(settings.clone())))
                .expect_err("refused");
        assert!(
            message.starts_with("catalog.postgres: no server could be used: ")
                && message.contains(&format!("127.0.0.1:{}", settings.port)),
            "names the key and the server: {message}"
        );
    }

    #[cfg(not(feature = "postgres"))]
    #[test]
    fn a_build_without_postgres_refuses_migrate_by_its_feature() {
        let dir = tempfile::tempdir().expect("dir");
        let settings = crate::config::PostgresSettings {
            hosts: vec!["db1.example.com".to_owned()],
            port: 5432,
            dbname: "vcon".to_owned(),
            user: "vcon_migrate".to_owned(),
            ca_file: None,
            password_file: Some(dir.path().join("pg.password")),
            client_cert_file: None,
            client_key_file: None,
            pool: 1,
            connect_timeout: std::time::Duration::from_secs(1),
            statement_timeout: std::time::Duration::from_secs(1),
            lock_timeout: std::time::Duration::from_secs(1),
        };
        let crate::config::StorageError(message) =
            migrate_for_command(&CatalogStore::Postgres(Box::new(settings))).expect_err("refused");
        assert!(
            message.contains("cargo feature postgres"),
            "names the feature: {message}"
        );
    }

    #[cfg(not(feature = "postgres"))]
    #[tokio::test]
    async fn a_build_without_postgres_refuses_a_postgres_catalog_by_its_feature() {
        let settings = crate::config::PostgresSettings {
            hosts: vec!["db1.example.com".to_owned()],
            port: 5432,
            dbname: "vcon".to_owned(),
            user: "vcon_app".to_owned(),
            ca_file: None,
            password_file: Some("/etc/vcon-server/pg.password".into()),
            client_cert_file: None,
            client_key_file: None,
            pool: 1,
            connect_timeout: std::time::Duration::from_secs(1),
            statement_timeout: std::time::Duration::from_secs(1),
            lock_timeout: std::time::Duration::from_secs(1),
        };
        let crate::config::StorageError(message) =
            catalog_for(&CatalogStore::Postgres(Box::new(settings)))
                .await
                .expect_err("refused");
        assert!(
            message.contains("catalog.backend") && message.contains("cargo feature postgres"),
            "names the setting and the feature: {message}"
        );
    }
}
