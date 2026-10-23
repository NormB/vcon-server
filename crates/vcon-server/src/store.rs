//! One tenant's vCons: a sealed object file per kept version and a catalog
//! row for it. The catalog is the truth: a version exists once its row is
//! committed, and an object file with no row is garbage.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;
use vcon_form::Inspection;

use crate::audit::{self, Action, Actor, Entry};
use crate::catalog;
use crate::clock::Clock;
use crate::envelope::{self, Context, Key, KeyId, WrapLimit};
use crate::fs::Fs;
use crate::ids::{TenantId, VersionId};

/// The tables of a tenant's catalog, besides its audit log.
const TABLES: &str = "
CREATE TABLE vcon (
    uuid BLOB PRIMARY KEY,
    current BLOB,
    deleted_at INTEGER,
    hold INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE version (
    version BLOB PRIMARY KEY,
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
    uuid BLOB PRIMARY KEY,
    purged_at INTEGER NOT NULL
);
CREATE TABLE key_use (
    tenant_key_id BLOB PRIMARY KEY,
    wraps INTEGER NOT NULL
);
";

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

/// One tenant's vCons.
pub struct Store {
    tenant: TenantId,
    tenant_key: Key,
    objects: PathBuf,
    conn: Mutex<Connection>,
    fs: Arc<dyn Fs>,
    clock: Arc<dyn Clock>,
    #[cfg(test)]
    hooks: tests::Hooks,
}

impl Store {
    /// Opens the tenant's catalog under its catalog key, making it if new.
    pub fn open(
        dir: &Path,
        tenant: TenantId,
        tenant_key: Key,
        fs: Arc<dyn Fs>,
        clock: Arc<dyn Clock>,
    ) -> Result<Store, StoreError> {
        let catalog_key = envelope::catalog_key(&tenant_key).map_err(|_| StoreError::Io)?;
        let schema = format!("{TABLES}{}", audit::SCHEMA);
        let conn = catalog::open(&dir.join("catalog.sqlite"), &catalog_key, &schema)
            .map_err(|_| StoreError::Io)?;
        let objects = dir.join("objects");
        fs.create_dir_all(&objects).map_err(|_| StoreError::Io)?;
        Ok(Store {
            tenant,
            tenant_key,
            objects,
            conn: Mutex::new(conn),
            fs,
            clock,
            #[cfg(test)]
            hooks: tests::Hooks::default(),
        })
    }

    /// Stores a new vCon. The uuid's state is checked before anything is
    /// sealed, the body is written in the order below, and the
    /// commit checks the state again, so of two racing writers one wins
    /// and the other's file is removed.
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
            let mut conn = self.conn();
            let tx = immediate(&mut conn)?;
            let refused = match state(&tx, uuid)? {
                State::Gone => Some((410, None, StoreError::Gone)),
                State::Live(current) => match self.read_version(&tx, uuid, current) {
                    Ok(stored) if stored == body => {
                        self.record(&tx, &entry(200, Some(current)))?;
                        commit(tx)?;
                        return Ok(Created::Same(current));
                    }
                    Ok(_) => Some((409, None, StoreError::Conflict)),
                    Err(StoreError::Damaged) => Some((500, Some(current), StoreError::Damaged)),
                    Err(e) => return Err(e),
                },
                State::Absent if envelope::wrap_limit(wraps(&tx, key_id)?) == WrapLimit::Refuse => {
                    Some((503, None, StoreError::LimitReached))
                }
                // The wrap is counted before it is made, so every wrap is
                // counted once whether this write wins or loses.
                State::Absent => {
                    add_wrap(&tx, key_id)?;
                    None
                }
            };
            if let Some((status, version, error)) = refused {
                self.record(&tx, &entry(status, version))?;
                commit(tx)?;
                return Err(error);
            }
            commit(tx)?;
        }

        let version = VersionId::generate();
        let ctx = Context::Version {
            tenant: self.tenant,
            uuid,
            version,
        };
        let data_key = Key::generate().map_err(|_| StoreError::Io)?;
        let blob = envelope::seal(&data_key, &ctx, body);
        let wrapped = envelope::wrap(&self.tenant_key, &ctx, &data_key);
        let file = self.write_object(version, &blob)?;
        #[cfg(test)]
        {
            let hook = self.hooks.before_commit.lock().expect("hook").take();
            if let Some(hook) = hook {
                hook();
            }
        }

        let committed = (|| {
            let mut conn = self.conn();
            let tx = immediate(&mut conn)?;
            match state(&tx, uuid)? {
                State::Absent => {}
                lost => {
                    let (status, error) = match lost {
                        State::Gone => (410, StoreError::Gone),
                        _ => (409, StoreError::Conflict),
                    };
                    self.record(&tx, &entry(status, None))?;
                    commit(tx)?;
                    return Err(error);
                }
            }
            tx.execute(
                "INSERT INTO version
                     (version, uuid, form, gzip, size, stored_at, hidden_at, tenant_key_id, wrapped_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8)",
                params![
                    version.0,
                    uuid.as_bytes(),
                    form_name(found.form),
                    found.gzip,
                    i64::try_from(body.len()).map_err(io)?,
                    nanos(self.clock.now()),
                    key_id.bytes(),
                    wrapped,
                ],
            )
            .map_err(io)?;
            tx.execute(
                "INSERT INTO vcon (uuid, current) VALUES (?1, ?2)",
                params![uuid.as_bytes(), version.0],
            )
            .map_err(io)?;
            self.record(&tx, &entry(201, Some(version)))?;
            commit(tx)?;
            Ok(Created::New(version))
        })();
        if committed.is_err() {
            // Best effort: start-up recovery removes any file left over.
            let _ = self.fs.remove(&file);
        }
        committed
    }

    /// Writes a sealed body in order: a temporary file, synced,
    /// renamed to its version id, and the directory synced. Returns the
    /// file's path.
    fn write_object(&self, version: VersionId, blob: &[u8]) -> Result<PathBuf, StoreError> {
        let tmp = self.objects.join(format!("{version}.tmp"));
        let file = self.objects.join(version.to_string());
        let written = self
            .fs
            .write_new(&tmp, blob)
            .and_then(|()| self.fs.sync_file(&tmp))
            .and_then(|()| self.fs.rename(&tmp, &file))
            .and_then(|()| self.fs.sync_dir(&self.objects));
        if written.is_err() {
            // Best effort: start-up recovery removes any file left over.
            let _ = self.fs.remove(&tmp);
            let _ = self.fs.remove(&file);
            return Err(StoreError::Io);
        }
        Ok(file)
    }

    /// The plaintext of a stored version. A missing row or file, or one
    /// that does not open, is damage.
    fn read_version(
        &self,
        tx: &Transaction,
        uuid: Uuid,
        version: VersionId,
    ) -> Result<Vec<u8>, StoreError> {
        let wrapped: Vec<u8> = tx
            .query_row(
                "SELECT wrapped_key FROM version WHERE version = ?1",
                [version.0],
                |r| r.get(0),
            )
            .optional()
            .map_err(io)?
            .ok_or(StoreError::Damaged)?;
        let ctx = Context::Version {
            tenant: self.tenant,
            uuid,
            version,
        };
        let data_key =
            envelope::unwrap(&self.tenant_key, &ctx, &wrapped).map_err(|_| StoreError::Damaged)?;
        let blob = self
            .fs
            .read(&self.objects.join(version.to_string()))
            .map_err(|_| StoreError::Damaged)?;
        envelope::open(&data_key, &ctx, &blob).map_err(|_| StoreError::Damaged)
    }

    /// Writes an audit record in `tx`; if it cannot, nothing is done.
    fn record(&self, tx: &Transaction, entry: &Entry) -> Result<(), StoreError> {
        audit::record(tx, entry, self.clock.now()).map_err(|_| StoreError::AuditFailed)
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .expect("no thread panicked holding the catalog")
    }
}

/// What the catalog says about a uuid.
enum State {
    /// Never stored.
    Absent,
    /// Stored, with this current version.
    Live(VersionId),
    /// Deleted or purged.
    Gone,
}

fn state(tx: &Transaction, uuid: Uuid) -> Result<State, StoreError> {
    let purged = tx
        .query_row(
            "SELECT 1 FROM tombstone WHERE uuid = ?1",
            [uuid.as_bytes()],
            |_| Ok(()),
        )
        .optional()
        .map_err(io)?;
    if purged.is_some() {
        return Ok(State::Gone);
    }
    let row: Option<(Option<[u8; 16]>, Option<i64>)> = tx
        .query_row(
            "SELECT current, deleted_at FROM vcon WHERE uuid = ?1",
            [uuid.as_bytes()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(io)?;
    match row {
        None => Ok(State::Absent),
        Some((_, Some(_))) => Ok(State::Gone),
        Some((Some(current), None)) => Ok(State::Live(VersionId(current))),
        // A live row always names its current version.
        Some((None, None)) => Err(StoreError::Damaged),
    }
}

/// Data keys wrapped so far under the tenant key `key`.
fn wraps(tx: &Transaction, key: KeyId) -> Result<u64, StoreError> {
    let wraps: Option<i64> = tx
        .query_row(
            "SELECT wraps FROM key_use WHERE tenant_key_id = ?1",
            [key.bytes()],
            |r| r.get(0),
        )
        .optional()
        .map_err(io)?;
    u64::try_from(wraps.unwrap_or(0)).map_err(io)
}

fn add_wrap(tx: &Transaction, key: KeyId) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO key_use (tenant_key_id, wraps) VALUES (?1, 1)
         ON CONFLICT (tenant_key_id) DO UPDATE SET wraps = wraps + 1",
        [key.bytes()],
    )
    .map_err(io)?;
    Ok(())
}

fn immediate(conn: &mut Connection) -> Result<Transaction<'_>, StoreError> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(io)
}

fn commit(tx: Transaction) -> Result<(), StoreError> {
    tx.commit().map_err(io)
}

fn io<E>(_: E) -> StoreError {
    StoreError::Io
}

fn form_name(form: vcon_form::Form) -> &'static str {
    match form {
        vcon_form::Form::Unsigned => "unsigned",
        vcon_form::Form::Signed => "signed",
        vcon_form::Form::Encrypted => "encrypted",
    }
}

/// A time as nanoseconds since 1970, as catalogs store it.
fn nanos(at: time::OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos()).expect("a time before 2262")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;
    use time::macros::datetime;
    use vcon_form::{Form, Limits, inspect};

    use super::*;
    use crate::clock::ManualClock;
    use crate::fs::{CrashFs, Op};

    const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

    /// Test-only points where a test can step in.
    #[derive(Default)]
    pub(super) struct Hooks {
        /// Runs once, after a write's files are synced and before its
        /// catalog transaction.
        pub(super) before_commit: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        fs: Arc<CrashFs>,
        store: Store,
        caller: Caller,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("temp dir");
        let fs = Arc::new(CrashFs::new());
        let clock = Arc::new(ManualClock::at(datetime!(2026-10-23 12:00 UTC)));
        let store = Store::open(
            dir.path(),
            TenantId([0x11; 16]),
            Key::generate().expect("tenant key"),
            fs.clone(),
            clock,
        )
        .expect("opens");
        let caller = Caller {
            key: Key::generate().expect("caller key").id(),
        };
        Fixture {
            _dir: dir,
            fs,
            store,
            caller,
        }
    }

    /// An unsigned vCon with this uuid, and what `inspect` makes of it.
    fn vcon(uuid: &str, subject: &str) -> (Inspection, Vec<u8>) {
        let body = format!(
            r#"{{"uuid": "{uuid}", "created_at": "2026-10-23T12:00:00Z", "subject": "{subject}", "parties": []}}"#
        )
        .into_bytes();
        let found = inspect(&body, None, &Limits { inflated: 1 << 20 }, &BTreeSet::new())
            .expect("a valid vCon");
        assert_eq!(found.form, Form::Unsigned);
        (found, body)
    }

    fn audit_log(store: &Store) -> Vec<Value> {
        let mut out = Vec::new();
        audit::export(&store.conn(), None, &mut out).expect("export");
        String::from_utf8(out)
            .expect("UTF-8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("JSON"))
            .collect()
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
            f.fs.read(&f.store.objects.join(version.to_string()))
                .expect("file");
        envelope::open(&data_key, &ctx, &blob).expect("opens")
    }

    #[test]
    fn a_new_vcon_is_written_in_order_and_recorded() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        let (tmp, done) = (
            f.store.objects.join(format!("{version}.tmp")),
            f.store.objects.join(version.to_string()),
        );
        assert_eq!(
            f.fs.log(),
            [
                Op::Write(tmp.clone()),
                Op::SyncFile(tmp.clone()),
                Op::Rename(tmp, done),
                Op::SyncDir(f.store.objects.clone()),
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
            assert_eq!(
                f.store.create(&f.caller, &found, &body),
                Err(StoreError::Gone),
                "{sql}"
            );
            assert_eq!(f.fs.log(), [], "no file written");
            assert_eq!(wraps(&f.store), 0, "nothing wrapped");
            assert_eq!(audit_log(&f.store)[0]["status"], 410);
        }
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
        assert_eq!(
            f.store.create(&f.caller, &found, &body),
            Err(StoreError::LimitReached)
        );
        assert_eq!(f.fs.log(), [], "no file written");
        assert_eq!(audit_log(&f.store)[0]["status"], 503);
    }

    #[test]
    fn a_missing_current_file_is_damage_not_a_conflict() {
        let f = fixture();
        let (found, body) = vcon(UUID, "first");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("a new vCon")
        };
        f.fs.remove(&f.store.objects.join(version.to_string()))
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
        *f.store.hooks.before_commit.lock().expect("hook") = Some(Box::new(move || {
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
            f.fs.list(&f.store.objects).expect("list").len(),
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
            f.fs.list(&f.store.objects).expect("list"),
            Vec::<std::ffi::OsString>::new()
        );
    }
}
