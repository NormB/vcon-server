//! One tenant's vCons: a sealed object per kept version and a catalog row
//! for it. The catalog is the truth: a version exists once its row is
//! committed, and an object with no row is garbage. Every rule is here;
//! the catalog and the objects are reached through the backend traits.

use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::{Mutex, MutexGuard};

#[cfg(test)]
use crate::backend::local::LocalConn;

use uuid::Uuid;
use vcon::read::Inspection;

use crate::audit::{self, Action, Actor, Entry};
use crate::backend::local::{LocalCatalog, LocalObjects};
use crate::backend::{Catalog, CatalogTx, CommitError, NewVersion, ObjectError, Objects};
use crate::catalog;
use crate::clock::{Clock, nanos};
use crate::envelope::{self, Context, Key, WrapLimit};
use crate::fs::Fs;
use crate::ids::{KeyId, TenantId, VersionId};
use crate::retention::Retention;

mod backup;
mod examine;
mod legal;
mod list;
mod purge;
mod rekey;

pub use backup::{BackupError, Copied, TRIES};
pub use examine::Examination;
pub use legal::{Legal, VersionInfo};
pub use list::{Cursor, ListQuery, Listed, Listing};
pub use purge::{Kept, PurgeReport, due};

/// Who is calling, for the audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub key: KeyId,
}

/// What `create` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Created {
    /// Stored as a new vCon.
    New(VersionId),
    /// The same bytes were already current; nothing was written.
    Same(VersionId),
}

/// A version as `get` returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub version: VersionId,
    pub form: vcon::Form,
    /// Whether the body is gzipped, as it arrived.
    pub gzip: bool,
    /// The body exactly as it arrived.
    pub body: Vec<u8>,
}

/// What a tenant's Store keeps, as removal checks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Versions kept, hidden ones included.
    pub versions: u64,
    /// Uuids under a legal hold.
    pub held: u64,
}

/// Why a Store call was refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    Conflict,
    Gone,
    NotFound,
    NotDeleted,
    PreconditionFailed,
    Immutable,
    /// The tenant key is at its wrap limit and must be rotated.
    LimitReached,
    /// A row's object file is missing or does not open.
    Damaged,
    /// The audit record could not be written, so nothing was done.
    AuditFailed,
    Io,
}

/// Why a tenant's Store did not open: kept apart so a key that does not
/// open its catalog is never confused with files this process may not use.
#[derive(Debug)]
pub enum OpenError {
    /// The catalog does not open under the tenant's catalog key.
    WrongKey,
    /// This process may not use `path`, a file or directory the Store
    /// needs; `error` is the system's.
    Access {
        path: PathBuf,
        error: std::io::Error,
    },
    /// Anything else.
    Store(StoreError),
}

impl From<OpenError> for StoreError {
    /// How the Store's callers see a Store that did not open.
    fn from(_: OpenError) -> StoreError {
        StoreError::Io
    }
}

impl From<StoreError> for OpenError {
    fn from(e: StoreError) -> OpenError {
        OpenError::Store(e)
    }
}

impl From<catalog::CatalogError> for OpenError {
    fn from(e: catalog::CatalogError) -> OpenError {
        match e {
            catalog::CatalogError::WrongKey => OpenError::WrongKey,
            catalog::CatalogError::Access { path, error } => OpenError::Access { path, error },
            _ => OpenError::Store(StoreError::Io),
        }
    }
}

/// A catalog transaction.
type Tx<'a> = Box<dyn CatalogTx + 'a>;

/// One tenant's vCons.
pub struct Store {
    tenant: TenantId,
    tenant_key: Key,
    catalog: Box<dyn Catalog>,
    objects: Arc<dyn Objects>,
    /// The filesystem a backup is written to.
    fs: Arc<dyn Fs>,
    clock: Arc<dyn Clock>,
    /// The local objects directory, for tests that look at its files.
    #[cfg(test)]
    objects_dir: PathBuf,
    #[cfg(test)]
    hooks: tests::Hooks,
}

impl Store {
    /// Opens the tenant's local catalog under its catalog key, making it
    /// if new, then runs start-up recovery.
    pub fn open(
        dir: &Path,
        tenant: TenantId,
        tenant_key: Key,
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
    ) -> Result<Store, OpenError> {
        let store = Store::open_beside(dir, tenant, tenant_key, fs, clock)?;
        store.recover()?;
        Ok(store)
    }

    /// Opens the store as a command running beside a server does: without
    /// start-up recovery, which would take the server's writes in progress
    /// for a crash's leftovers.
    pub fn open_beside(
        dir: &Path,
        tenant: TenantId,
        tenant_key: Key,
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
    ) -> Result<Store, OpenError> {
        let catalog_key = envelope::catalog_key(&tenant_key).map_err(|_| StoreError::Io)?;
        let catalog = LocalCatalog::open(&dir.join("catalog.sqlite"), catalog_key)?;
        let objects = dir.join("objects");
        fs.create_dir_all(&objects).map_err(|_| StoreError::Io)?;
        // Each version is written into objects/, so it must be writable too.
        catalog::may_use(&objects, libc::W_OK | libc::X_OK)?;
        // A new directory survives a crash only once the directory that
        // holds it is synced; without this a crash could take the objects
        // directory, synced files and all, while the catalog keeps its rows.
        fs.sync_dir(dir).map_err(|_| StoreError::Io)?;
        #[cfg_attr(
            not(test),
            allow(unused_mut, reason = "only tests set the objects directory after")
        )]
        let mut store = Store::with_backends(
            tenant,
            tenant_key,
            Box::new(catalog),
            Arc::new(LocalObjects::new(objects.clone(), fs.clone())),
            fs,
            clock,
        );
        #[cfg(test)]
        {
            store.objects_dir = objects;
        }
        Ok(store)
    }

    /// A Store over the given catalog and objects, opened as `open_beside`
    /// opens one: without start-up recovery.
    pub(crate) fn with_backends(
        tenant: TenantId,
        tenant_key: Key,
        catalog: Box<dyn Catalog>,
        objects: Arc<dyn Objects>,
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
    ) -> Store {
        Store {
            tenant,
            tenant_key,
            catalog,
            objects,
            fs,
            clock,
            #[cfg(test)]
            objects_dir: PathBuf::new(),
            #[cfg(test)]
            hooks: tests::Hooks::default(),
        }
    }

    /// Start-up recovery: the catalog recovers itself, and temporary
    /// objects and objects with no row are removed. A name that is not a
    /// version id, with or without `.tmp`, is not ours and is left alone.
    fn recover(&self) -> Result<(), StoreError> {
        self.catalog.recover()?;
        let mut removed = false;
        for name in self.objects.list().map_err(io)? {
            let (id, temporary) = match name.strip_suffix(".tmp") {
                Some(id) => (id, true),
                None => (name.as_str(), false),
            };
            let Some(version) = VersionId::parse(id) else {
                continue;
            };
            let has_row = {
                let mut tx = self.catalog.snapshot()?;
                tx.version_row(version)?.is_some()
            };
            if temporary || !has_row {
                self.objects.remove_name(&name).map_err(io)?;
                removed = true;
            }
        }
        if removed {
            self.objects.sync().map_err(io)?;
        }
        Ok(())
    }

    /// Stores a new vCon. The uuid's state is checked before anything is
    /// sealed, the body is written in the order below, and the
    /// commit checks the state again, so of two racing writers one wins
    /// and the other's object is removed.
    pub fn create(
        &self,
        who: &Caller,
        found: &Inspection,
        body: &[u8],
    ) -> Result<Created, StoreError> {
        let uuid = found.uuid;
        let entry = |status, version| Entry {
            actor: Actor::Key(who.key),
            action: Action::Create,
            target: Some(uuid.to_string()),
            version,
            status,
        };
        let key_id = self.tenant_key.id();
        {
            let mut tx = self.catalog.immediate()?;
            let refused = match state(&mut tx, uuid)? {
                State::Gone => Some((410, None, StoreError::Gone)),
                State::Live(current) => match self.read_version(&mut tx, uuid, current) {
                    Ok(stored) if stored == body => {
                        self.record(&mut tx, &entry(200, Some(current)))?;
                        commit(tx)?;
                        return Ok(Created::Same(current));
                    }
                    Ok(_) => Some((409, None, StoreError::Conflict)),
                    Err(StoreError::Damaged) => Some((500, Some(current), StoreError::Damaged)),
                    Err(e) => return Err(e),
                },
                State::Absent if envelope::wrap_limit(tx.wraps(key_id)?) == WrapLimit::Refuse => {
                    Some((503, None, StoreError::LimitReached))
                }
                // The wrap is counted before it is made, so every wrap is
                // counted once whether this write wins or loses.
                State::Absent => {
                    tx.add_wrap(key_id)?;
                    None
                }
            };
            if let Some((status, version, error)) = refused {
                self.record(&mut tx, &entry(status, version))?;
                commit(tx)?;
                return Err(error);
            }
            commit(tx)?;
        }

        let (version, wrapped) = self.write_version(uuid, body)?;
        self.pause();

        let unknown = std::cell::Cell::new(false);
        let committed = (|| {
            let mut tx = self.catalog.immediate()?;
            match state(&mut tx, uuid)? {
                State::Absent => {}
                lost => {
                    let (status, error) = match lost {
                        State::Gone => (410, StoreError::Gone),
                        _ => (409, StoreError::Conflict),
                    };
                    self.record(&mut tx, &entry(status, None))?;
                    commit(tx)?;
                    return Err(error);
                }
            }
            // The version's time, and the uuid's first: the list keeps
            // its place by it through replace, delete and restore.
            let stored_at = nanos(self.clock.now());
            tx.insert_version(&NewVersion {
                version,
                uuid,
                form: form_name(found.form),
                gzip: found.gzip,
                size: i64::try_from(body.len()).map_err(io)?,
                stored_at,
                key: key_id,
                wrapped: &wrapped,
            })?;
            tx.insert_vcon(uuid, version, stored_at)?;
            self.record(&mut tx, &entry(201, Some(version)))?;
            kept(tx.commit(), &unknown)?;
            Ok(Created::New(version))
        })();
        self.drop_unless_kept(committed.is_err(), unknown.get(), version);
        committed
    }

    /// Reads the current version of `uuid`. The body is decrypted whole,
    /// then one transaction checks that the version is still current and
    /// commits the audit record; only then is the body returned. If the
    /// version changed meanwhile, the read starts again.
    pub fn get(&self, who: &Caller, uuid: Uuid) -> Result<Stored, StoreError> {
        let entry = |status, version| Entry {
            actor: Actor::Key(who.key),
            action: Action::Read,
            target: Some(uuid.to_string()),
            version,
            status,
        };
        let refuse = |mut tx: Tx, state: State| {
            let (status, error) = match state {
                State::Gone => (410, StoreError::Gone),
                _ => (404, StoreError::NotFound),
            };
            self.record(&mut tx, &entry(status, None))?;
            commit(tx)?;
            Err(error)
        };
        loop {
            let (version, row) = {
                let mut tx = self.catalog.immediate()?;
                match state(&mut tx, uuid)? {
                    State::Live(version) => (version, version_row(&mut tx, version)),
                    other => return refuse(tx, other),
                }
            };
            let opened = row.and_then(|row| {
                let body = self.open_version(uuid, version, &row.wrapped)?;
                Ok(Stored {
                    version,
                    form: row.form,
                    gzip: row.gzip,
                    body,
                })
            });
            self.pause();
            let mut tx = self.catalog.immediate()?;
            match state(&mut tx, uuid)? {
                State::Live(now) if now == version => {
                    let status = if opened.is_ok() { 200 } else { 500 };
                    self.record(&mut tx, &entry(status, Some(version)))?;
                    commit(tx)?;
                    return opened;
                }
                State::Live(_) => continue,
                other => return refuse(tx, other),
            }
        }
    }

    /// Replaces the current version of `found.uuid`, if it is `if_match`
    /// and neither signed nor encrypted. The caller has checked that the
    /// body's uuid is the one asked for. The old version is hidden, and
    /// kept for legal reads until it is purged. As in `create`, the state
    /// is checked before sealing and again at the commit, so of two racing
    /// replaces one wins and the other's object is removed.
    pub fn replace(
        &self,
        who: &Caller,
        if_match: VersionId,
        found: &Inspection,
        body: &[u8],
    ) -> Result<VersionId, StoreError> {
        let uuid = found.uuid;
        let entry = |status, version| Entry {
            actor: Actor::Key(who.key),
            action: Action::Replace,
            target: Some(uuid.to_string()),
            version,
            status,
        };
        let key_id = self.tenant_key.id();
        // Why the state does not allow this replace, if it does not.
        let refusal = |tx: &mut Tx| -> Result<Option<(u16, StoreError)>, StoreError> {
            Ok(match state(tx, uuid)? {
                State::Absent => Some((404, StoreError::NotFound)),
                State::Gone => Some((410, StoreError::Gone)),
                State::Live(current) if version_row(tx, current)?.form != vcon::Form::Unsigned => {
                    Some((409, StoreError::Immutable))
                }
                State::Live(current) if current != if_match => {
                    Some((412, StoreError::PreconditionFailed))
                }
                State::Live(_) => None,
            })
        };
        {
            let mut tx = self.catalog.immediate()?;
            let refused = match refusal(&mut tx)? {
                None if envelope::wrap_limit(tx.wraps(key_id)?) == WrapLimit::Refuse => {
                    Some((503, StoreError::LimitReached))
                }
                other => other,
            };
            if let Some((status, error)) = refused {
                self.record(&mut tx, &entry(status, None))?;
                commit(tx)?;
                return Err(error);
            }
            tx.add_wrap(key_id)?;
            commit(tx)?;
        }

        let (version, wrapped) = self.write_version(uuid, body)?;
        self.pause();

        let unknown = std::cell::Cell::new(false);
        let committed = (|| {
            let mut tx = self.catalog.immediate()?;
            if let Some((status, error)) = refusal(&mut tx)? {
                self.record(&mut tx, &entry(status, None))?;
                commit(tx)?;
                return Err(error);
            }
            let now = nanos(self.clock.now());
            tx.insert_version(&NewVersion {
                version,
                uuid,
                form: form_name(found.form),
                gzip: found.gzip,
                size: i64::try_from(body.len()).map_err(io)?,
                stored_at: now,
                key: key_id,
                wrapped: &wrapped,
            })?;
            tx.hide_version(if_match, now)?;
            tx.set_current(uuid, version)?;
            self.record(&mut tx, &entry(200, Some(version)))?;
            kept(tx.commit(), &unknown)?;
            Ok(version)
        })();
        self.drop_unless_kept(committed.is_err(), unknown.get(), version);
        committed
    }

    /// Deletes `uuid`: one transaction marks it deleted, empties its
    /// current version and hides every kept version, with the audit
    /// record. Its versions stay for legal reads until they are purged;
    /// any already as old as `retention` is purged at once, so a zero
    /// period erases the vCon inside the request.
    /// Deleting a deleted or purged uuid succeeds again. A legal hold does
    /// not stop a delete, only a purge.
    pub fn delete(&self, who: &Caller, uuid: Uuid, retention: Retention) -> Result<(), StoreError> {
        let entry = |status| Entry {
            actor: Actor::Key(who.key),
            action: Action::Delete,
            target: Some(uuid.to_string()),
            version: None,
            status,
        };
        {
            let mut tx = self.catalog.immediate()?;
            let outcome = match state(&mut tx, uuid)? {
                State::Absent => Err(StoreError::NotFound),
                State::Gone => Ok(()),
                State::Live(_) => {
                    tx.mark_deleted(uuid, nanos(self.clock.now()))?;
                    Ok(())
                }
            };
            let status = if outcome.is_ok() { 204 } else { 404 };
            self.record(&mut tx, &entry(status))?;
            commit(tx)?;
            outcome?;
        }
        // A version already as old as `retention` is erased now, not at
        // the next sweep.
        self.purge_hidden(Actor::Key(who.key), retention, Some(uuid))?;
        Ok(())
    }

    /// Every version whose object is missing. Such a version answers 500
    /// when read; it is reported, never dropped.
    pub fn check(&self) -> Result<Vec<VersionId>, StoreError> {
        let versions = self.catalog.snapshot()?.version_ids()?;
        let present: std::collections::BTreeSet<String> =
            self.objects.list().map_err(io)?.into_iter().collect();
        Ok(versions
            .into_iter()
            .filter(|v| !present.contains(&v.to_string()))
            .collect())
    }

    /// Seals `body` as a new version of `uuid` under a new data key, wraps
    /// that key under the tenant key, and writes the object. Returns the
    /// version id and the wrapped key.
    fn write_version(&self, uuid: Uuid, body: &[u8]) -> Result<(VersionId, Vec<u8>), StoreError> {
        let version = VersionId::generate();
        let ctx = Context::Version {
            tenant: self.tenant,
            uuid,
            version,
        };
        let data_key = Key::generate().map_err(|_| StoreError::Io)?;
        let blob = envelope::seal(&data_key, &ctx, body);
        let wrapped = envelope::wrap(&self.tenant_key, &ctx, &data_key);
        self.objects.put(version, &blob).map_err(io)?;
        Ok((version, wrapped))
    }

    /// Removes a new version's object when the write failed and its rows
    /// were surely not kept. After a commit whose outcome is unknown the
    /// object stays: its rows may be kept, and a row without its object is
    /// damage, while an object without a row is a leftover that start-up
    /// recovery removes.
    fn drop_unless_kept(&self, failed: bool, unknown: bool, version: VersionId) {
        if failed && !unknown {
            // Best effort: start-up recovery removes any object left over.
            let _ = self.objects.remove(version);
        }
    }

    /// In tests, runs the hook set for the gap between work done outside
    /// the catalog lock and the transaction that confirms it.
    fn pause(&self) {
        #[cfg(test)]
        {
            let hook = self.hooks.between.lock().expect("hook").take();
            if let Some(hook) = hook {
                hook();
            }
        }
    }

    /// The plaintext of a stored version. A missing row or object, or one
    /// that does not open, is damage.
    fn read_version(
        &self,
        tx: &mut Tx,
        uuid: Uuid,
        version: VersionId,
    ) -> Result<Vec<u8>, StoreError> {
        let row = version_row(tx, version)?;
        self.open_version(uuid, version, &row.wrapped)
    }

    /// Opens a version's object with its wrapped data key.
    fn open_version(
        &self,
        uuid: Uuid,
        version: VersionId,
        wrapped: &[u8],
    ) -> Result<Vec<u8>, StoreError> {
        let ctx = Context::Version {
            tenant: self.tenant,
            uuid,
            version,
        };
        let data_key =
            envelope::unwrap(&self.tenant_key, &ctx, wrapped).map_err(|_| StoreError::Damaged)?;
        let blob = self.objects.get(version).map_err(|_| StoreError::Damaged)?;
        envelope::open(&data_key, &ctx, &blob).map_err(|_| StoreError::Damaged)
    }

    /// How many versions the Store keeps and how many uuids are held.
    pub fn counts(&self) -> Result<Counts, StoreError> {
        let mut tx = self.catalog.snapshot()?;
        Ok(Counts {
            versions: tx.count_versions()?,
            held: tx.count_held()?,
        })
    }

    /// Writes the tenant's log after `since` as JSON lines, oldest first.
    /// Not recorded: reading the log changes nothing.
    pub fn export(
        &self,
        since: Option<i64>,
        out: &mut dyn std::io::Write,
    ) -> Result<(), audit::AuditError> {
        self.catalog.export(since.unwrap_or(0), out)
    }

    /// A page of the tenant's log as JSON lines, the lines `export` writes
    /// for the same records. The read is recorded as audit_read in the same
    /// log and transaction, after the page is read, so a page never holds
    /// its own record.
    pub fn audit_page(
        &self,
        who: &Caller,
        query: &audit::AuditQuery,
    ) -> Result<Vec<u8>, StoreError> {
        let mut tx = self.catalog.immediate()?;
        let mut out = Vec::new();
        let read = tx.audit_lines(query.after, Some(query.limit), &mut out);
        self.record(
            &mut tx,
            &Entry {
                actor: Actor::Key(who.key),
                action: Action::AuditRead,
                target: None,
                version: None,
                status: if read.is_ok() { 200 } else { 500 },
            },
        )?;
        commit(tx)?;
        read.map(|()| out).map_err(io)
    }

    /// Records, in a transaction of its own, an action on the tenant taken
    /// elsewhere, such as a change to its retention period.
    pub fn log(&self, entry: &Entry) -> Result<(), StoreError> {
        let mut tx = self.catalog.immediate()?;
        self.record(&mut tx, entry)?;
        commit(tx)
    }

    #[cfg(test)]
    pub(crate) fn break_audit(&self) {
        self.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
            )
            .expect("trigger");
    }

    #[cfg(test)]
    pub(crate) fn on_backup_copied(&self, hook: tests::BackupHook) {
        *self.hooks.copied.lock().expect("hook") = Some(hook);
    }

    #[cfg(test)]
    pub(crate) fn conn_for_test(&self) -> MutexGuard<'_, LocalConn> {
        self.conn()
    }

    #[cfg(test)]
    pub(crate) fn records(&self) -> Vec<serde_json::Value> {
        crate::audit::testing::exported(&self.conn(), None)
    }

    /// Counts, in a transaction of its own, an event the tenant's log
    /// keeps one record per minute for, such as a failed check of one of
    /// its keys.
    pub fn count(&self, entry: &Entry) -> Result<(), StoreError> {
        let mut tx = self.catalog.immediate()?;
        tx.count(entry, self.clock.now())
            .map_err(|_| StoreError::AuditFailed)?;
        commit(tx)
    }

    /// Writes an audit record in `tx`; if it cannot, nothing is done.
    fn record(&self, tx: &mut Tx, entry: &Entry) -> Result<(), StoreError> {
        tx.record(entry, self.clock.now())
            .map_err(|_| StoreError::AuditFailed)
    }

    /// The local catalog's connection, for tests.
    #[cfg(test)]
    fn conn(&self) -> MutexGuard<'_, LocalConn> {
        self.catalog.local_conn()
    }
}

enum State {
    Absent,
    Live(VersionId),
    Gone,
}

/// A uuid's state: purged or deleted is Gone. A row that is neither
/// deleted nor current is damage.
fn state(tx: &mut Tx, uuid: Uuid) -> Result<State, StoreError> {
    if tx.tombstoned(uuid)? {
        return Ok(State::Gone);
    }
    match tx.vcon_row(uuid)? {
        None => Ok(State::Absent),
        Some(row) if row.deleted_at.is_some() => Ok(State::Gone),
        Some(row) => row.current.map(State::Live).ok_or(StoreError::Damaged),
    }
}

struct VersionRow {
    form: vcon::Form,
    gzip: bool,
    wrapped: Vec<u8>,
}

/// A version's row; one missing, or with a form this build does not
/// know, is damage.
fn version_row(tx: &mut Tx, version: VersionId) -> Result<VersionRow, StoreError> {
    let row = tx.version_row(version)?.ok_or(StoreError::Damaged)?;
    let form = match row.form.as_str() {
        "unsigned" => vcon::Form::Unsigned,
        "signed" => vcon::Form::Signed,
        "encrypted" => vcon::Form::Encrypted,
        _ => return Err(StoreError::Damaged),
    };
    Ok(VersionRow {
        form,
        gzip: row.gzip,
        wrapped: row.wrapped,
    })
}

fn commit(tx: Tx) -> Result<(), StoreError> {
    tx.commit().map_err(io)
}

/// The commit that adds a version's rows: an error is Io, and one whose
/// outcome is unknown is noted in `unknown`, since its rows may be kept.
fn kept(
    outcome: Result<(), CommitError>,
    unknown: &std::cell::Cell<bool>,
) -> Result<(), StoreError> {
    outcome.map_err(|e| {
        unknown.set(e == CommitError::Unknown);
        StoreError::Io
    })
}

fn io<E>(_: E) -> StoreError {
    StoreError::Io
}

fn form_name(form: vcon::Form) -> &'static str {
    match form {
        vcon::Form::Unsigned => "unsigned",
        vcon::Form::Signed => "signed",
        vcon::Form::Encrypted => "encrypted",
    }
}

impl From<ObjectError> for StoreError {
    fn from(_: ObjectError) -> StoreError {
        StoreError::Io
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::BTreeSet;

    use vcon::Form;
    use vcon::read::{Inspection, Limits, inspect};

    /// An unsigned vCon with this uuid, and what `inspect` makes of it.
    pub(crate) fn vcon(uuid: &str, subject: &str) -> (Inspection, Vec<u8>) {
        let body = format!(
            r#"{{"uuid": "{uuid}", "created_at": "2026-10-23T12:00:00Z", "subject": "{subject}", "parties": []}}"#
        )
        .into_bytes();
        let found =
            inspect(&body, None, &Limits::new(1 << 20), &BTreeSet::new()).expect("a valid vCon");
        assert_eq!(found.form, Form::Unsigned);
        (found, body)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;
    use time::macros::datetime;
    use vcon::read::{Limits, inspect};

    use super::testing::vcon;
    use super::*;
    use crate::audit::testing::exported;

    /// Each version is written into objects/: a Store that may not write
    /// there does not open, and says so with the path and the system's
    /// error, as it does for its catalog.
    #[test]
    fn an_objects_directory_the_process_may_not_write_is_named() {
        use std::os::unix::fs::PermissionsExt;
        let f = fixture();
        let objects = f.dir.path().join("objects");
        let mode = |m| {
            std::fs::set_permissions(&objects, std::fs::Permissions::from_mode(m)).expect("chmod")
        };
        mode(0o500);
        let refused = f.open_or_why();
        mode(0o700);
        match refused {
            Err(OpenError::Access { path, error }) => {
                assert_eq!(path, objects);
                assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("{:?}", other.map(|_| "opened")),
        }
        assert!(f.open_or_why().is_ok(), "opens once allowed");
    }
    use crate::catalog::testing::columns_taking_null;
    use crate::clock::ManualClock;
    use rusqlite::{Connection, params};

    use crate::catalog;
    use crate::fs::{CrashFs, Op};

    const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

    /// What a backup runs at each try, after it copies the catalog.
    pub(crate) type BackupHook = Box<dyn FnMut(&Store) + Send>;

    /// Test-only points where a test can step in.
    #[derive(Default)]
    pub(super) struct Hooks {
        /// Runs once, between work done outside the catalog lock (a
        /// write's files, a read's decryption) and the transaction that
        /// confirms it.
        pub(super) between: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        /// Runs at each try of a backup, after the catalog is copied and
        /// before its versions' files are.
        pub(super) copied: Mutex<Option<BackupHook>>,
    }

    pub(super) struct Fixture {
        pub(super) dir: tempfile::TempDir,
        pub(super) fs: Arc<CrashFs>,
        pub(super) clock: Arc<ManualClock>,
        /// The tenant key, wrapped under `master`, so a test can open the
        /// store again with the same key.
        master: Key,
        wrapped: Vec<u8>,
        pub(super) store: Store,
        pub(super) caller: Caller,
    }

    pub(super) const TENANT: TenantId = TenantId([0x11; 16]);

    impl Fixture {
        pub(super) fn open(&self) -> Result<Store, StoreError> {
            self.open_or_why().map_err(StoreError::from)
        }

        /// `open`, saying why a Store that did not open did not.
        pub(super) fn open_or_why(&self) -> Result<Store, OpenError> {
            let ctx = Context::Tenant { tenant: TENANT };
            let tenant_key = envelope::unwrap(&self.master, &ctx, &self.wrapped).expect("unwraps");
            Store::open(
                self.dir.path(),
                TENANT,
                tenant_key,
                self.fs.clone(),
                self.clock.clone(),
            )
        }

        /// The tenant key the Store was made under.
        pub(super) fn tenant_key(&self) -> Key {
            let ctx = Context::Tenant { tenant: TENANT };
            envelope::unwrap(&self.master, &ctx, &self.wrapped).expect("unwraps")
        }

        pub(super) fn tenant(&self) -> TenantId {
            TENANT
        }

        /// Opens the store again as a command beside a server would.
        fn open_beside(&self) -> Result<Store, StoreError> {
            let ctx = Context::Tenant { tenant: TENANT };
            let tenant_key = envelope::unwrap(&self.master, &ctx, &self.wrapped).expect("unwraps");
            Store::open_beside(
                self.dir.path(),
                TENANT,
                tenant_key,
                self.fs.clone(),
                self.clock.clone(),
            )
            .map_err(StoreError::from)
        }

        /// Closes the store and opens it again, as a restart would.
        pub(super) fn reopen(&mut self) {
            let placeholder = self.open().expect("opens");
            drop(std::mem::replace(&mut self.store, placeholder));
            self.store = self.open().expect("reopens");
        }
    }

    pub(super) fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("temp dir");
        let fs = Arc::new(CrashFs::new(dir.path()));
        let clock = Arc::new(ManualClock::at(datetime!(2026-10-23 12:00 UTC)));
        let master = Key::generate().expect("master key");
        let tenant_key = Key::generate().expect("tenant key");
        let wrapped = envelope::wrap(&master, &Context::Tenant { tenant: TENANT }, &tenant_key);
        let store =
            Store::open(dir.path(), TENANT, tenant_key, fs.clone(), clock.clone()).expect("opens");
        let caller = Caller {
            key: Key::generate().expect("caller key").id(),
        };
        Fixture {
            dir,
            fs,
            clock,
            master,
            wrapped,
            store,
            caller,
        }
    }

    fn audit_log(store: &Store) -> Vec<Value> {
        exported(&store.conn(), None)
    }

    fn count(store: &Store, sql: &str) -> i64 {
        store.conn().query_row(sql, [], |r| r.get(0)).expect(sql)
    }

    fn wraps(store: &Store) -> i64 {
        store
            .conn()
            .query_row("SELECT coalesce(sum(wraps), 0) FROM key_use", [], |r| {
                r.get(0)
            })
            .expect("wraps")
    }

    /// The body the store holds for `version`, opened as `get` will.
    fn stored_body(f: &Fixture, uuid: Uuid, version: VersionId) -> Vec<u8> {
        let wrapped: Vec<u8> = f
            .store
            .conn()
            .query_row(
                "SELECT wrapped_key FROM version WHERE version = ?1",
                [version.0],
                |r| r.get(0),
            )
            .expect("row");
        let ctx = Context::Version {
            tenant: f.store.tenant,
            uuid,
            version,
        };
        let data_key = envelope::unwrap(&f.store.tenant_key, &ctx, &wrapped).expect("unwraps");
        let blob =
            f.fs.read(&f.store.objects_dir.join(version.to_string()))
                .expect("file");
        envelope::open(&data_key, &ctx, &blob).expect("opens")
    }

    fn audit_page(f: &Fixture, after: i64, limit: u32) -> Result<Vec<u8>, StoreError> {
        f.store
            .audit_page(&f.caller, &audit::AuditQuery { after, limit })
    }

    fn export_text(store: &Store, since: Option<i64>) -> Vec<u8> {
        let mut out = Vec::new();
        store.export(since, &mut out).expect("export");
        out
    }

    #[test]
    fn an_audit_page_is_export_s_lines_and_records_its_read_after_them() {
        let f = fixture();
        for n in 0..3 {
            let (found, body) = vcon(&format!("01a07da8-c2bb-83e5-b9a2-27900000000{n}"), "x");
            f.store.create(&f.caller, &found, &body).expect("stored");
        }
        let before = export_text(&f.store, None);
        assert_eq!(audit_page(&f, 0, 1000), Ok(before.clone()));
        let read = audit_log(&f.store);
        assert_eq!(read.len(), 4, "three creates and the read");
        assert_eq!(read[3]["action"], "audit_read");
        assert_eq!(read[3]["actor"], f.caller.key.to_string());
        assert_eq!(read[3]["status"], 200);
        assert_eq!(read[3]["seq"], 4);
        assert_eq!(read[3]["target"], Value::Null);
        // A page of two from seq 1: the second and third creates.
        let lines: Vec<&[u8]> = before.split_inclusive(|b| *b == b'\n').collect();
        assert_eq!(audit_page(&f, 1, 2), Ok(lines[1..3].concat()));
        // Past the end: nothing but the earlier reads' records.
        let tail = audit_page(&f, 4, 1000).expect("a page");
        assert_eq!(tail, export_text(&f.store, Some(4))[..tail.len()]);
        assert_eq!(String::from_utf8(tail).expect("UTF-8").lines().count(), 1);
    }

    /// A record that cannot be read back (a status past any HTTP status)
    /// fails the page, and the read is still recorded, as 500.
    #[test]
    fn an_audit_page_that_cannot_be_read_is_a_failure_and_recorded() {
        let f = fixture();
        f.store
            .conn_for_test()
            .execute(
                "INSERT INTO audit (at, actor, action, status, count)
                 VALUES ('2026-10-23T12:00:00Z', 'cli', 'read', 70000, 1)",
                [],
            )
            .expect("damaged");
        assert_eq!(audit_page(&f, 0, 10), Err(StoreError::Io));
        let mut recorded = Vec::new();
        f.store.export(Some(1), &mut recorded).expect("export");
        let recorded: Value = serde_json::from_slice(&recorded).expect("one record");
        assert_eq!(recorded["action"], "audit_read");
        assert_eq!(recorded["status"], 500);
    }

    #[test]
    fn an_audit_page_whose_record_fails_is_refused() {
        let f = fixture();
        f.store.break_audit();
        assert_eq!(audit_page(&f, 0, 10), Err(StoreError::AuditFailed));
    }

    #[test]
    fn a_new_vcon_is_written_in_order_and_recorded() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let ops = f.fs.log().len();
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let (tmp, done) = (
            f.store.objects_dir.join(format!("{version}.tmp")),
            f.store.objects_dir.join(version.to_string()),
        );
        assert_eq!(
            f.fs.log()[ops..],
            [
                Op::Write(tmp.clone()),
                Op::SyncFile(tmp.clone()),
                Op::Rename(tmp, done),
                Op::SyncDir(f.store.objects_dir.clone()),
            ]
        );
        assert_eq!(stored_body(&f, found.uuid, version), body);
        assert_eq!(
            count(
                &f.store,
                "SELECT count(*) FROM version WHERE hidden_at IS NULL"
            ),
            1
        );
        let current: Vec<u8> = f
            .store
            .conn()
            .query_row(
                "SELECT current FROM vcon WHERE uuid = ?1",
                [found.uuid.as_bytes()],
                |r| r.get(0),
            )
            .expect("vcon row");
        assert_eq!(current, version.0);
        assert_eq!(wraps(&f.store), 1, "one data key wrapped");
        let log = audit_log(&f.store);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0]["action"], "create");
        assert_eq!(log[0]["actor"], f.caller.key.to_string());
        assert_eq!(log[0]["target"], UUID);
        assert_eq!(log[0]["version"], version.to_string());
        assert_eq!(log[0]["status"], 201);
    }

    #[test]
    fn the_same_body_again_is_same_and_writes_nothing() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let ops = f.fs.log().len();
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Ok(Created::Same(version))
        );
        assert_eq!(f.fs.log().len(), ops, "no file written");
        assert_eq!(count(&f.store, "SELECT count(*) FROM version"), 1);
        assert_eq!(wraps(&f.store), 1, "nothing wrapped");
        let log = audit_log(&f.store);
        assert_eq!(
            (log[1]["status"].clone(), log[1]["version"].clone()),
            (200.into(), version.to_string().into())
        );
    }

    #[test]
    fn another_body_for_a_stored_uuid_is_a_conflict_and_writes_nothing() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        f.store.create(&f.caller, &found, &body).expect("stored");
        let ops = f.fs.log().len();
        // The same length as the first body, so only the bytes differ.
        let (found, other) = vcon(UUID, "other");
        assert_eq!(
            f.store.create(&f.caller, &found, &other),
            Err(StoreError::Conflict)
        );
        assert_eq!(f.fs.log().len(), ops, "no file written");
        assert_eq!(wraps(&f.store), 1, "nothing wrapped");
        assert_eq!(audit_log(&f.store)[1]["status"], 409);
    }

    #[test]
    fn a_deleted_or_purged_uuid_is_gone_and_writes_nothing() {
        for sql in [
            "INSERT INTO vcon (uuid, current, deleted_at) VALUES (?1, NULL, 1)",
            "INSERT INTO tombstone (uuid, purged_at) VALUES (?1, 1)",
        ] {
            let f = fixture();
            let (found, body) = vcon(UUID, "first");
            f.store
                .conn()
                .execute(sql, [found.uuid.as_bytes()])
                .expect(sql);
            let ops = f.fs.log().len();
            assert_eq!(
                f.store.create(&f.caller, &found, &body),
                Err(StoreError::Gone),
                "{sql}"
            );
            assert_eq!(f.fs.log().len(), ops, "no file written");
            assert_eq!(wraps(&f.store), 0, "nothing wrapped");
            assert_eq!(audit_log(&f.store)[0]["status"], 410);
        }
    }

    #[test]
    fn counts_include_hidden_versions_and_held_uuids() {
        let f = fixture();
        let none = Counts {
            versions: 0,
            held: 0,
        };
        assert_eq!(f.store.counts(), Ok(none));
        let (found, old) = stored(&f, "first");
        let (_, newer) = vcon(UUID, "second");
        f.store
            .replace(&f.caller, old, &found, &newer)
            .expect("replaced");
        f.store
            .delete(&f.caller, found.uuid, Retention::Years(7))
            .expect("deleted");
        let (other, body) = vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bc47", "other");
        f.store.create(&f.caller, &other, &body).expect("stored");
        assert_eq!(
            f.store.counts(),
            Ok(Counts {
                versions: 3,
                held: 0
            }),
            "two hidden versions and a live one"
        );
        f.store
            .conn()
            .execute(
                "UPDATE vcon SET hold = 1 WHERE uuid = ?1",
                [found.uuid.as_bytes()],
            )
            .expect("a hold");
        assert_eq!(f.store.counts().map(|c| c.held), Ok(1));
    }

    #[test]
    fn a_negative_wrap_count_is_damage_and_stores_nothing() {
        let f = fixture();
        f.store
            .conn()
            .execute(
                "INSERT INTO key_use (tenant_key_id, wraps) VALUES (?1, -1)",
                [f.store.tenant_key.id().bytes()],
            )
            .expect("a damaged count");
        let (found, body) = vcon(UUID, "first");
        let ops = f.fs.log().len();
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::Io)
        );
        assert_eq!(f.fs.log().len(), ops, "no file written");
    }

    #[test]
    fn a_tenant_key_at_its_wrap_limit_refuses_new_vcons() {
        let f = fixture();
        f.store
            .conn()
            .execute(
                "INSERT INTO key_use (tenant_key_id, wraps) VALUES (?1, ?2)",
                params![f.store.tenant_key.id().bytes(), 1_i64 << 32],
            )
            .expect("at the limit");
        let (found, body) = vcon(UUID, "first");
        let ops = f.fs.log().len();
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::LimitReached)
        );
        assert_eq!(f.fs.log().len(), ops, "no file written");
        assert_eq!(audit_log(&f.store)[0]["status"], 503);
    }

    #[test]
    fn a_missing_current_file_is_damage_not_a_conflict() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        f.fs.remove(&f.store.objects_dir.join(version.to_string()))
            .expect("remove");
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::Damaged)
        );
        assert_eq!(audit_log(&f.store)[1]["status"], 500);
    }

    #[test]
    fn a_racing_create_that_loses_gets_a_conflict_and_leaves_no_file() {
        let f = Arc::new(fixture());
        let (found, body) = vcon(UUID, "first");
        let winner = f.clone();
        *f.store.hooks.between.lock().expect("hook") = Some(Box::new(move || {
            let (found, other) = vcon(UUID, "second");
            assert!(matches!(
                winner.store.create(&winner.caller, &found, &other),
                Ok(Created::New(_))
            ));
        }));
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::Conflict)
        );
        assert_eq!(
            count(&f.store, "SELECT count(*) FROM version"),
            1,
            "only the winner"
        );
        assert_eq!(
            f.fs.list(&f.store.objects_dir).expect("list").len(),
            1,
            "the loser's file is gone"
        );
        assert_eq!(wraps(&f.store), 2, "both wraps are counted");
        let statuses: Vec<_> = audit_log(&f.store)
            .iter()
            .map(|r| r["status"].clone())
            .collect();
        assert_eq!(statuses, [201, 409]);
    }

    #[test]
    fn a_create_whose_audit_record_fails_leaves_no_row_and_no_file() {
        let f = fixture();
        // Every audit insert now fails, as a full disk or a broken log would.
        f.store
            .conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'audit fails'); END;",
            )
            .expect("trigger");
        let (found, body) = vcon(UUID, "first");
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::AuditFailed)
        );
        assert_eq!(count(&f.store, "SELECT count(*) FROM version"), 0);
        assert_eq!(count(&f.store, "SELECT count(*) FROM vcon"), 0);
        assert_eq!(
            f.fs.list(&f.store.objects_dir).expect("list"),
            Vec::<std::ffi::OsString>::new()
        );
    }

    #[test]
    fn a_create_whose_commit_outcome_is_unknown_keeps_its_object() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        // The wrap count commits first; the second commit, which adds the
        // rows, is kept and then reported as failed, as a lost connection
        // reports a commit the database made.
        f.store.catalog.fail_commit(2, CommitError::Unknown);
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::Io),
            "a_create_whose_commit_outcome_is_unknown_keeps_its_object: f.store.create(&f.caller, &found, &body)"
        );
        let stored = f
            .store
            .get(&f.caller, found.uuid)
            .expect("the kept row reads");
        assert_eq!(
            stored.body, body,
            "a_create_whose_commit_outcome_is_unknown_keeps_its_object: stored.body"
        );
        assert_eq!(f.store.check(), Ok(Vec::new()), "no row without its object");
        drop(f.open().expect("reopens"));
        assert_eq!(
            f.store.get(&f.caller, found.uuid).map(|s| s.body),
            Ok(body),
            "recovery keeps an object a row names"
        );
    }

    #[test]
    fn a_replace_whose_commit_outcome_is_unknown_keeps_its_object() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(first)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let (found, second) = vcon(UUID, "second");
        f.store.catalog.fail_commit(2, CommitError::Unknown);
        assert_eq!(
            f.store.replace(&f.caller, first, &found, &second),
            Err(StoreError::Io),
            "a_replace_whose_commit_outcome_is_unknown_keeps_its_object: f.store.replace(&f.caller, first, &found, &second)"
        );
        let stored = f
            .store
            .get(&f.caller, found.uuid)
            .expect("the kept row reads");
        assert_eq!(
            stored.body, second,
            "a_replace_whose_commit_outcome_is_unknown_keeps_its_object: stored.body"
        );
        assert_eq!(f.store.check(), Ok(Vec::new()), "no row without its object");
    }

    #[test]
    fn a_create_whose_commit_rolls_back_leaves_no_object() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        f.store.catalog.fail_commit(2, CommitError::RolledBack);
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::Io),
            "a_create_whose_commit_rolls_back_leaves_no_object: f.store.create(&f.caller, &found, &body)"
        );
        assert_eq!(
            count(&f.store, "SELECT count(*) FROM version"),
            0,
            "a_create_whose_commit_rolls_back_leaves_no_object: count(&f.store, \"SELECT count(*) FROM version\")"
        );
        assert_eq!(
            f.fs.list(&f.store.objects_dir).expect("list"),
            Vec::<std::ffi::OsString>::new(),
            "a_create_whose_commit_rolls_back_leaves_no_object: f.fs.list(&f.store.objects_dir).expect(\"list\")"
        );
    }

    fn get_entry_status(f: &Fixture) -> Value {
        audit_log(&f.store).last().expect("a record")["status"].clone()
    }

    #[test]
    fn a_stored_vcon_reads_back_byte_for_byte_and_is_recorded() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let stored = f.store.get(&f.caller, found.uuid).expect("reads");
        assert_eq!(
            stored,
            Stored {
                version,
                form: vcon::Form::Unsigned,
                gzip: false,
                body
            }
        );
        let log = audit_log(&f.store);
        let read = log.last().expect("a record");
        assert_eq!(read["action"], "read");
        assert_eq!(read["status"], 200);
        assert_eq!(read["version"], version.to_string());
        assert_eq!(read["target"], UUID);
    }

    #[test]
    fn a_uuid_never_stored_is_not_found() {
        let f = fixture();
        let (found, _) = vcon(UUID, "first");
        assert_eq!(
            f.store.get(&f.caller, found.uuid),
            Err(StoreError::NotFound)
        );
        assert_eq!(get_entry_status(&f), 404);
    }

    #[test]
    fn a_deleted_or_purged_uuid_is_gone() {
        for sql in [
            "UPDATE vcon SET current = NULL, deleted_at = 1 WHERE uuid = ?1",
            "DELETE FROM vcon WHERE uuid = ?1; INSERT INTO tombstone (uuid, purged_at) VALUES (?1, 1)",
        ] {
            let f = fixture();
            let (found, body) = vcon(UUID, "first");
            f.store.create(&f.caller, &found, &body).expect("stored");
            for statement in sql.split("; ") {
                f.store
                    .conn()
                    .execute(statement, [found.uuid.as_bytes()])
                    .expect(statement);
            }
            assert_eq!(
                f.store.get(&f.caller, found.uuid),
                Err(StoreError::Gone),
                "{sql}"
            );
            assert_eq!(get_entry_status(&f), 410);
        }
    }

    #[test]
    fn a_missing_or_damaged_file_is_a_500_never_an_empty_body() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let file = f.store.objects_dir.join(version.to_string());
        let blob = f.fs.read(&file).expect("file");
        f.fs.remove(&file).expect("remove");
        assert_eq!(
            f.store.get(&f.caller, found.uuid),
            Err(StoreError::Damaged),
            "missing"
        );
        assert_eq!(get_entry_status(&f), 500);
        let mut bad = blob.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        f.fs.write_new(&file, &bad).expect("write");
        assert_eq!(
            f.store.get(&f.caller, found.uuid),
            Err(StoreError::Damaged),
            "tampered"
        );
    }

    #[test]
    fn a_read_racing_a_delete_never_returns_the_body() {
        let f = Arc::new(fixture());
        let (found, body) = vcon(UUID, "first");
        f.store.create(&f.caller, &found, &body).expect("stored");
        let deleter = f.clone();
        *f.store.hooks.between.lock().expect("hook") = Some(Box::new(move || {
            deleter
                .store
                .conn()
                .execute(
                    "UPDATE vcon SET current = NULL, deleted_at = 1 WHERE uuid = ?1",
                    [found.uuid.as_bytes()],
                )
                .expect("delete");
        }));
        assert_eq!(f.store.get(&f.caller, found.uuid), Err(StoreError::Gone));
        assert_eq!(get_entry_status(&f), 410);
    }

    #[test]
    fn a_read_racing_a_new_version_starts_again_and_returns_the_new_one() {
        let f = Arc::new(fixture());
        let (found, body) = vcon(UUID, "first");
        f.store.create(&f.caller, &found, &body).expect("stored");
        let (_, newer) = vcon(UUID, "newer");
        let (version, wrapped) = f.store.write_version(found.uuid, &newer).expect("written");
        let writer = f.clone();
        *f.store.hooks.between.lock().expect("hook") = Some(Box::new(move || {
            let conn = writer.store.conn();
            conn.execute(
                "INSERT INTO version
                     (version, uuid, form, gzip, size, stored_at, tenant_key_id, wrapped_key)
                 VALUES (?1, ?2, 'unsigned', 0, 0, 0, ?3, ?4)",
                params![
                    version.0,
                    found.uuid.as_bytes(),
                    writer.store.tenant_key.id().bytes(),
                    wrapped
                ],
            )
            .expect("new version");
            conn.execute(
                "UPDATE vcon SET current = ?1 WHERE uuid = ?2",
                params![version.0, found.uuid.as_bytes()],
            )
            .expect("made current");
        }));
        let stored = f.store.get(&f.caller, found.uuid).expect("reads");
        assert_eq!((stored.version, stored.body), (version, newer));
        let reads = audit_log(&f.store)
            .iter()
            .filter(|r| r["action"] == "read")
            .count();
        assert_eq!(reads, 1, "one record for the one request");
    }

    #[test]
    fn every_form_and_the_gzip_flag_read_back_as_stored() {
        use base64::Engine;
        use std::io::Write;
        let f = fixture();
        let uuids = [
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc01",
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc02",
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc03",
        ];
        let (_, unsigned) = vcon(uuids[0], "plain");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&unsigned).expect("gzip");
        let gzipped = gz.finish().expect("gzip");
        let (_, inner) = vcon(uuids[1], "signed");
        let signed = format!(
            r#"{{"payload": "{}", "signatures": [{{"header": {{}}}}]}}"#,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(inner)
        )
        .into_bytes();
        let encrypted = format!(
            r#"{{"unprotected": {{"uuid": "{}"}}, "ciphertext": "x", "recipients": []}}"#,
            uuids[2]
        )
        .into_bytes();
        let cases = [
            (gzipped, vcon::Form::Unsigned, true),
            (signed, vcon::Form::Signed, false),
            (encrypted, vcon::Form::Encrypted, false),
        ];
        for (body, form, gzip) in cases {
            let found = inspect(&body, None, &Limits::new(1 << 20), &BTreeSet::new())
                .expect("a valid vCon");
            assert_eq!((found.form, found.gzip), (form, gzip));
            f.store.create(&f.caller, &found, &body).expect("stored");
            let stored = f.store.get(&f.caller, found.uuid).expect("reads");
            assert_eq!((stored.form, stored.gzip, stored.body), (form, gzip, body));
        }
    }

    /// Stores `subject` under UUID and returns its version.
    fn stored(f: &Fixture, subject: &str) -> (Inspection, VersionId) {
        let (found, body) = vcon(UUID, subject);
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        (found, version)
    }

    #[test]
    fn a_replace_makes_a_new_current_version_and_hides_the_old_one() {
        let f = fixture();
        let (found, old) = stored(&f, "first");
        let (_, newer) = vcon(UUID, "second");
        let version = f
            .store
            .replace(&f.caller, old, &found, &newer)
            .expect("replaced");
        assert_ne!(version, old);
        let stored = f.store.get(&f.caller, found.uuid).expect("reads");
        assert_eq!((stored.version, stored.body), (version, newer));
        let hidden: Option<i64> = f
            .store
            .conn()
            .query_row(
                "SELECT hidden_at FROM version WHERE version = ?1",
                [old.0],
                |r| r.get(0),
            )
            .expect("the old row is kept");
        assert!(hidden.is_some(), "the old version is hidden");
        assert!(
            f.fs.read(&f.store.objects_dir.join(old.to_string()))
                .is_ok(),
            "its file is kept"
        );
        let record = audit_log(&f.store)
            .into_iter()
            .find(|r| r["action"] == "replace")
            .expect("a replace record");
        assert_eq!(
            (record["status"].clone(), record["version"].clone()),
            (200.into(), version.to_string().into())
        );
    }

    #[test]
    fn a_replace_of_a_missing_or_gone_uuid_is_refused() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let any = VersionId([7; 16]);
        assert_eq!(
            f.store.replace(&f.caller, any, &found, &body),
            Err(StoreError::NotFound)
        );
        assert_eq!(get_entry_status(&f), 404);
        let (found, old) = stored(&f, "first");
        f.store
            .conn()
            .execute(
                "UPDATE vcon SET current = NULL, deleted_at = 1 WHERE uuid = ?1",
                [found.uuid.as_bytes()],
            )
            .expect("delete");
        assert_eq!(
            f.store.replace(&f.caller, old, &found, &body),
            Err(StoreError::Gone)
        );
        assert_eq!(get_entry_status(&f), 410);
    }

    #[test]
    fn a_stale_if_match_is_refused_and_writes_nothing() {
        let f = fixture();
        let (found, _) = stored(&f, "first");
        let ops = f.fs.log().len();
        let (_, newer) = vcon(UUID, "second");
        assert_eq!(
            f.store
                .replace(&f.caller, VersionId([7; 16]), &found, &newer),
            Err(StoreError::PreconditionFailed)
        );
        assert_eq!(f.fs.log().len(), ops, "no file written");
        assert_eq!(wraps(&f.store), 1, "nothing wrapped");
        assert_eq!(get_entry_status(&f), 412);
    }

    #[test]
    fn a_signed_or_encrypted_current_version_is_immutable() {
        use base64::Engine;
        let f = fixture();
        let (found, old) = stored(&f, "first");
        let (_, inner) = vcon(UUID, "signed");
        let signed = format!(
            r#"{{"payload": "{}", "signatures": [{{"header": {{}}}}]}}"#,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(inner)
        )
        .into_bytes();
        let signed_found =
            inspect(&signed, None, &Limits::new(1 << 20), &BTreeSet::new()).expect("a signed vCon");
        let version = f
            .store
            .replace(&f.caller, old, &signed_found, &signed)
            .expect("an unsigned vCon may be replaced by a signed one");
        let (_, newer) = vcon(UUID, "second");
        let ops = f.fs.log().len();
        assert_eq!(
            f.store.replace(&f.caller, version, &found, &newer),
            Err(StoreError::Immutable)
        );
        assert_eq!(f.fs.log().len(), ops, "no file written");
        assert_eq!(get_entry_status(&f), 409);

        let g = fixture();
        let encrypted = format!(
            r#"{{"unprotected": {{"uuid": "{UUID}"}}, "ciphertext": "x", "recipients": []}}"#
        )
        .into_bytes();
        let enc_found = inspect(&encrypted, None, &Limits::new(1 << 20), &BTreeSet::new())
            .expect("an encrypted vCon");
        let Ok(Created::New(enc_version)) = g.store.create(&g.caller, &enc_found, &encrypted)
        else {
            panic!("stored")
        };
        assert_eq!(
            g.store.replace(&g.caller, enc_version, &found, &newer),
            Err(StoreError::Immutable)
        );
    }

    #[test]
    fn a_replace_at_the_wrap_limit_is_refused() {
        let f = fixture();
        let (found, old) = stored(&f, "first");
        f.store
            .conn()
            .execute("UPDATE key_use SET wraps = ?1", [1_i64 << 32])
            .expect("at the limit");
        let (_, newer) = vcon(UUID, "second");
        assert_eq!(
            f.store.replace(&f.caller, old, &found, &newer),
            Err(StoreError::LimitReached)
        );
        assert_eq!(get_entry_status(&f), 503);
    }

    #[test]
    fn of_two_replaces_racing_on_one_version_the_second_to_commit_loses() {
        let f = Arc::new(fixture());
        let (found, old) = stored(&f, "first");
        let winner = f.clone();
        let racer = found;
        *f.store.hooks.between.lock().expect("hook") = Some(Box::new(move || {
            let (_, body) = vcon(UUID, "winner");
            winner
                .store
                .replace(&winner.caller, old, &racer, &body)
                .expect("wins");
        }));
        let (_, body) = vcon(UUID, "loser");
        assert_eq!(
            f.store.replace(&f.caller, old, &found, &body),
            Err(StoreError::PreconditionFailed)
        );
        assert_eq!(
            f.fs.list(&f.store.objects_dir).expect("list").len(),
            2,
            "first and winner only"
        );
        assert_eq!(wraps(&f.store), 3, "every wrap counted");
        assert_eq!(get_entry_status(&f), 412);
    }

    #[test]
    fn two_threads_replacing_with_the_same_if_match_one_wins() {
        let f = Arc::new(fixture());
        let (_, old) = stored(&f, "first");
        let threads: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|subject| {
                let f = f.clone();
                std::thread::spawn(move || {
                    let (found, body) = vcon(UUID, subject);
                    f.store.replace(&f.caller, old, &found, &body)
                })
            })
            .collect();
        let results: Vec<_> = threads
            .into_iter()
            .map(|t| t.join().expect("thread"))
            .collect();
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        assert!(
            results.contains(&Err(StoreError::PreconditionFailed)),
            "{results:?}"
        );
    }

    #[test]
    fn a_replace_whose_audit_record_fails_changes_nothing() {
        let f = fixture();
        let (found, old) = stored(&f, "first");
        f.store
            .conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'audit fails'); END;",
            )
            .expect("trigger");
        let (_, newer) = vcon(UUID, "second");
        assert_eq!(
            f.store.replace(&f.caller, old, &found, &newer),
            Err(StoreError::AuditFailed)
        );
        f.store
            .conn()
            .execute_batch("DROP TRIGGER no_audit")
            .expect("drop trigger");
        assert_eq!(
            f.store.get(&f.caller, found.uuid).expect("reads").version,
            old
        );
        assert_eq!(
            f.fs.list(&f.store.objects_dir).expect("list").len(),
            1,
            "only the first file"
        );
    }

    fn hidden_versions(f: &Fixture) -> i64 {
        count(
            &f.store,
            "SELECT count(*) FROM version WHERE hidden_at IS NOT NULL",
        )
    }

    #[test]
    fn a_deleted_uuid_answers_gone_and_keeps_its_versions_hidden() {
        let f = fixture();
        let (found, old) = stored(&f, "first");
        let (_, newer) = vcon(UUID, "second");
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        let current = f
            .store
            .replace(&f.caller, old, &found, &newer)
            .expect("replaced");
        f.clock.set(datetime!(2026-10-23 14:00 UTC));
        assert_eq!(
            f.store.delete(&f.caller, found.uuid, Retention::Years(7)),
            Ok(())
        );
        let record = audit_log(&f.store).pop().expect("a record");
        assert_eq!(
            (record["action"].clone(), record["status"].clone()),
            ("delete".into(), 204.into())
        );
        let hidden_at = |version: VersionId| -> i64 {
            f.store
                .conn()
                .query_row(
                    "SELECT hidden_at FROM version WHERE version = ?1",
                    [version.0],
                    |r| r.get(0),
                )
                .expect("hidden")
        };
        assert_eq!(
            hidden_at(old),
            nanos(datetime!(2026-10-23 13:00 UTC)),
            "hidden by the replace"
        );
        assert_eq!(
            hidden_at(current),
            nanos(datetime!(2026-10-23 14:00 UTC)),
            "hidden by the delete"
        );
        assert_eq!(f.store.get(&f.caller, found.uuid), Err(StoreError::Gone));
        let (_, body) = vcon(UUID, "third");
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::Gone)
        );
        assert_eq!(hidden_versions(&f), 2, "both versions hidden");
        assert_eq!(
            f.fs.list(&f.store.objects_dir).expect("list").len(),
            2,
            "files kept for legal reads"
        );
        let (current, deleted_at): (Option<Vec<u8>>, Option<i64>) = f
            .store
            .conn()
            .query_row(
                "SELECT current, deleted_at FROM vcon WHERE uuid = ?1",
                [found.uuid.as_bytes()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("row");
        assert_eq!(current, None);
        assert_eq!(deleted_at, Some(nanos(datetime!(2026-10-23 14:00 UTC))));
    }

    #[test]
    fn deleting_a_deleted_or_purged_uuid_succeeds_again() {
        let f = fixture();
        let (found, _) = stored(&f, "first");
        f.store
            .delete(&f.caller, found.uuid, Retention::Years(7))
            .expect("deleted");
        assert_eq!(
            f.store.delete(&f.caller, found.uuid, Retention::Years(7)),
            Ok(())
        );
        assert_eq!(get_entry_status(&f), 204);
        let g = fixture();
        g.store
            .conn()
            .execute(
                "INSERT INTO tombstone (uuid, purged_at) VALUES (?1, 1)",
                [found.uuid.as_bytes()],
            )
            .expect("purged");
        assert_eq!(
            g.store.delete(&g.caller, found.uuid, Retention::Years(7)),
            Ok(())
        );
        assert_eq!(get_entry_status(&g), 204);
    }

    #[test]
    fn deleting_a_uuid_never_stored_is_not_found() {
        let f = fixture();
        let (found, _) = vcon(UUID, "first");
        assert_eq!(
            f.store.delete(&f.caller, found.uuid, Retention::Years(7)),
            Err(StoreError::NotFound)
        );
        assert_eq!(get_entry_status(&f), 404);
    }

    #[test]
    fn a_hold_does_not_stop_a_delete() {
        let f = fixture();
        let (found, _) = stored(&f, "first");
        f.store
            .conn()
            .execute(
                "UPDATE vcon SET hold = 1 WHERE uuid = ?1",
                [found.uuid.as_bytes()],
            )
            .expect("hold");
        assert_eq!(
            f.store.delete(&f.caller, found.uuid, Retention::Years(7)),
            Ok(())
        );
        assert_eq!(f.store.get(&f.caller, found.uuid), Err(StoreError::Gone));
    }

    #[test]
    fn a_delete_whose_audit_record_fails_changes_nothing() {
        let f = fixture();
        let (found, version) = stored(&f, "first");
        f.store
            .conn()
            .execute_batch(
                "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'audit fails'); END;",
            )
            .expect("trigger");
        assert_eq!(
            f.store.delete(&f.caller, found.uuid, Retention::Years(7)),
            Err(StoreError::AuditFailed)
        );
        f.store
            .conn()
            .execute_batch("DROP TRIGGER no_audit")
            .expect("drop trigger");
        assert_eq!(
            f.store
                .get(&f.caller, found.uuid)
                .expect("still live")
                .version,
            version
        );
        assert_eq!(hidden_versions(&f), 0);
    }

    #[test]
    fn only_the_optional_catalog_columns_may_be_empty() {
        let f = fixture();
        let conn = f.store.conn();
        let tables: [(&str, &[(&str, &str)]); 5] = [
            (
                "vcon",
                &[
                    ("uuid", "x'01'"),
                    ("current", "x'02'"),
                    ("deleted_at", "1"),
                    ("hold", "0"),
                ],
            ),
            (
                "version",
                &[
                    ("version", "x'02'"),
                    ("uuid", "x'01'"),
                    ("form", "'unsigned'"),
                    ("gzip", "0"),
                    ("size", "1"),
                    ("stored_at", "1"),
                    ("hidden_at", "1"),
                    ("tenant_key_id", "x'03'"),
                    ("wrapped_key", "x'04'"),
                ],
            ),
            ("tombstone", &[("uuid", "x'01'"), ("purged_at", "1")]),
            ("key_use", &[("tenant_key_id", "x'03'"), ("wraps", "1")]),
            (
                "audit",
                &[
                    ("at", "'2026-10-23T12:00Z'"),
                    ("actor", "'cli'"),
                    ("action", "'create'"),
                    ("target", "'x'"),
                    ("version", "'y'"),
                    ("status", "201"),
                    ("count", "1"),
                ],
            ),
        ];
        let may_be_empty: Vec<String> = tables
            .iter()
            .flat_map(|(table, row)| columns_taking_null(&conn, table, row))
            .collect();
        assert_eq!(
            may_be_empty,
            [
                "vcon.current",
                "vcon.deleted_at",
                "version.hidden_at",
                "audit.target",
                "audit.version"
            ]
        );
    }

    #[test]
    fn a_new_stores_objects_directory_survives_a_crash() {
        let f = fixture();
        f.fs.crash();
        assert_eq!(
            f.fs.list(f.dir.path()).expect("list"),
            [std::ffi::OsString::from("objects")]
        );
    }

    #[test]
    fn opening_removes_temporary_and_orphaned_files_and_nothing_else() {
        let mut f = fixture();
        let (found, version) = stored(&f, "first");
        let orphan = VersionId([0x0e; 16]);
        let objects = f.store.objects_dir.clone();
        for name in [
            format!("{orphan}"),
            format!("{orphan}.tmp"),
            format!("{version}.tmp"),
            "notes.txt".to_owned(),
        ] {
            f.fs.write_new(&objects.join(name), b"x").expect("write");
        }
        f.reopen();
        let names: Vec<String> =
            f.fs.list(&objects)
                .expect("list")
                .into_iter()
                .map(|n| n.to_string_lossy().into_owned())
                .collect();
        assert_eq!(names, [version.to_string(), "notes.txt".to_owned()]);
        assert_eq!(
            f.store
                .get(&f.caller, found.uuid)
                .expect("still reads")
                .version,
            version
        );
        let log = f.fs.log();
        let last_removal = log
            .iter()
            .rposition(|op| matches!(op, Op::Remove(_)))
            .expect("files were removed");
        assert!(
            log[last_removal..].contains(&Op::SyncDir(objects)),
            "the removals are synced"
        );
    }

    /// A command beside a running server must not take the server's
    /// writes in progress for a crash's leftovers.
    #[test]
    fn opening_beside_a_server_removes_nothing() {
        let f = fixture();
        let (found, version) = stored(&f, "first");
        let orphan = VersionId([0x0e; 16]);
        let objects = f.store.objects_dir.clone();
        let mut written = vec![version.to_string()];
        for name in [
            format!("{orphan}"),
            format!("{orphan}.tmp"),
            format!("{version}.tmp"),
        ] {
            f.fs.write_new(&objects.join(&name), b"x").expect("write");
            written.push(name);
        }
        written.sort();
        let ops = f.fs.log().len();
        let beside = f.open_beside().expect("opens beside");
        let mut names: Vec<String> =
            f.fs.list(&objects)
                .expect("list")
                .into_iter()
                .map(|n| n.to_string_lossy().into_owned())
                .collect();
        names.sort();
        assert_eq!(names, written);
        assert!(
            !f.fs.log()[ops..]
                .iter()
                .any(|op| matches!(op, Op::Remove(_))),
            "nothing removed"
        );
        assert_eq!(
            beside.get(&f.caller, found.uuid).expect("reads").version,
            version
        );
    }

    #[test]
    fn opening_checkpoints_the_wal_with_truncation() {
        let mut f = fixture();
        stored(&f, "first");
        let wal = f.dir.path().join("catalog.sqlite-wal");
        assert!(
            std::fs::metadata(&wal).expect("a WAL").len() > 0,
            "the WAL holds the write"
        );
        f.reopen();
        assert_eq!(std::fs::metadata(&wal).expect("a WAL").len(), 0);
    }

    #[test]
    fn check_reports_a_version_whose_file_is_missing() {
        let f = fixture();
        let (_, version) = stored(&f, "first");
        assert_eq!(f.store.check(), Ok(vec![]));
        f.fs.remove(&f.store.objects_dir.join(version.to_string()))
            .expect("remove");
        assert_eq!(f.store.check(), Ok(vec![version]));
    }

    #[test]
    fn opening_refuses_while_a_reader_stops_the_wal_from_being_truncated() {
        let f = fixture();
        stored(&f, "first");
        let ctx = Context::Tenant { tenant: TENANT };
        let tenant_key = envelope::unwrap(&f.master, &ctx, &f.wrapped).expect("unwraps");
        let catalog_key = envelope::catalog_key(&tenant_key).expect("catalog key");
        let mut reader = catalog::open(&f.dir.path().join("catalog.sqlite"), &catalog_key, "")
            .expect("a second connection");
        let read = reader.transaction().expect("read transaction");
        let _: i64 = read
            .query_row("SELECT count(*) FROM vcon", [], |r| r.get(0))
            .expect("a read holds a snapshot");
        assert!(matches!(f.open(), Err(StoreError::Io)));
        drop(read);
        assert!(f.open().is_ok(), "opens once the reader is done");
    }

    /// The filesystem changes a create or replace makes when nothing
    /// fails: write, sync, rename, sync the directory.
    const WRITE_STEPS: usize = 4;

    /// After a restart, no row lacks its file and every file has a row.
    fn assert_whole(f: &Fixture, step: usize) {
        assert_eq!(
            f.store.check(),
            Ok(vec![]),
            "a row without its file at step {step}"
        );
        for name in f.fs.list(&f.store.objects_dir).expect("list") {
            let name = name.to_string_lossy().into_owned();
            let version = VersionId::parse(&name)
                .unwrap_or_else(|| panic!("{name} left over at step {step}"));
            let rows = count(
                &f.store,
                &format!("SELECT count(*) FROM version WHERE hex(version) = upper('{version}')"),
            );
            assert_eq!(rows, 1, "{name} has no row at step {step}");
        }
    }

    /// Makes the next audit insert fail for every connection, as if the
    /// process died before its commit; `unblock` removes it again.
    pub(super) fn block_commits(f: &Fixture) -> Connection {
        let ctx = Context::Tenant { tenant: TENANT };
        let tenant_key = envelope::unwrap(&f.master, &ctx, &f.wrapped).expect("unwraps");
        let catalog_key = envelope::catalog_key(&tenant_key).expect("catalog key");
        catalog::open(&f.dir.path().join("catalog.sqlite"), &catalog_key, "")
            .expect("second connection")
    }

    fn die_before_commit(f: &Fixture) {
        let other = block_commits(f);
        let fs = f.fs.clone();
        *f.store.hooks.between.lock().expect("hook") = Some(Box::new(move || {
            fs.fail_from(0);
            other
                .execute_batch(
                    "CREATE TRIGGER dead BEFORE INSERT ON audit
                     BEGIN SELECT RAISE(ABORT, 'the process has died'); END;",
                )
                .expect("trigger");
        }));
    }

    pub(super) fn restart(f: &mut Fixture) {
        f.fs.crash();
        f.store
            .conn()
            .execute_batch("DROP TRIGGER IF EXISTS dead")
            .expect("drop trigger");
        f.reopen();
    }

    #[test]
    fn a_create_cut_off_at_any_step_leaves_no_vcon_or_the_whole_vcon() {
        let (found, body) = vcon(UUID, "first");
        for step in 0..=WRITE_STEPS {
            let mut f = fixture();
            f.fs.fail_from(f.fs.log().len() + step);
            let result = f.store.create(&f.caller, &found, &body);
            assert_eq!(
                result.is_ok(),
                step == WRITE_STEPS,
                "step {step}: {result:?}"
            );
            restart(&mut f);
            match f.store.get(&f.caller, found.uuid) {
                Ok(stored) => assert_eq!(stored.body, body, "step {step}"),
                Err(StoreError::NotFound) => {
                    assert!(result.is_err(), "acknowledged, then lost, at step {step}")
                }
                other => panic!("half a vCon at step {step}: {other:?}"),
            }
            assert_whole(&f, step);
        }
    }

    #[test]
    fn a_create_cut_off_between_its_file_and_its_commit_leaves_nothing() {
        let mut f = fixture();
        let (found, body) = vcon(UUID, "first");
        die_before_commit(&f);
        assert!(f.store.create(&f.caller, &found, &body).is_err());
        restart(&mut f);
        assert_eq!(
            f.store.get(&f.caller, found.uuid),
            Err(StoreError::NotFound)
        );
        assert_whole(&f, WRITE_STEPS);
    }

    #[test]
    fn a_replace_cut_off_at_any_step_leaves_the_old_version_or_the_new() {
        let (found, old_body) = vcon(UUID, "first");
        let (_, new_body) = vcon(UUID, "second");
        for step in 0..=WRITE_STEPS {
            let mut f = fixture();
            let Ok(Created::New(old)) = f.store.create(&f.caller, &found, &old_body) else {
                panic!("stored")
            };
            f.fs.fail_from(f.fs.log().len() + step);
            let result = f.store.replace(&f.caller, old, &found, &new_body);
            assert_eq!(
                result.is_ok(),
                step == WRITE_STEPS,
                "step {step}: {result:?}"
            );
            restart(&mut f);
            let stored = f
                .store
                .get(&f.caller, found.uuid)
                .unwrap_or_else(|e| panic!("step {step}: {e:?}"));
            match result {
                Ok(new) => assert_eq!(
                    (stored.version, stored.body),
                    (new, new_body.clone()),
                    "step {step}"
                ),
                Err(_) => assert_eq!(
                    (stored.version, stored.body),
                    (old, old_body.clone()),
                    "step {step}"
                ),
            }
            assert_whole(&f, step);
        }
    }

    #[test]
    fn a_replace_cut_off_between_its_file_and_its_commit_keeps_the_old_version() {
        let mut f = fixture();
        let (found, old) = stored(&f, "first");
        let (_, newer) = vcon(UUID, "second");
        die_before_commit(&f);
        assert!(f.store.replace(&f.caller, old, &found, &newer).is_err());
        restart(&mut f);
        assert_eq!(
            f.store.get(&f.caller, found.uuid).expect("reads").version,
            old
        );
        assert_eq!(hidden_versions(&f), 0);
        assert_whole(&f, WRITE_STEPS);
    }

    #[test]
    fn a_delete_cut_off_before_its_commit_leaves_the_vcon_live() {
        let mut f = fixture();
        let (found, version) = stored(&f, "first");
        let other = block_commits(&f);
        other
            .execute_batch(
                "CREATE TRIGGER dead BEFORE INSERT ON audit
                 BEGIN SELECT RAISE(ABORT, 'the process has died'); END;",
            )
            .expect("trigger");
        assert!(
            f.store
                .delete(&f.caller, found.uuid, Retention::Years(7))
                .is_err()
        );
        drop(other);
        restart(&mut f);
        assert_eq!(
            f.store
                .get(&f.caller, found.uuid)
                .expect("still live")
                .version,
            version
        );
        assert_eq!(hidden_versions(&f), 0);
    }
}
