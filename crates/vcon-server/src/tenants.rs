//! The registry of tenants: each one's id, name, retention period and
//! wrapped tenant key, kept in `server.sqlite` under a catalog key derived
//! from the master key, with the admin audit log beside them. Each tenant's
//! vCons live in `tenants/<id>/`, in a Store of their own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::audit::{self, Action, Actor, Entry};
use crate::catalog::{self, CatalogError};
use crate::clock::{Clock, nanos};
use crate::envelope::{self, Context, Key, KeyId, WrapLimit};
use crate::fs::Fs;
use crate::ids::TenantId;
use crate::retention::Retention;
use crate::store::Store;

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
";

/// Why a Tenants call was refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TenantError {
    /// No tenant has the id.
    NotFound,
    /// Another tenant has the name.
    NameTaken,
    /// The master key is at its wrap limit and must be rotated.
    LimitReached,
    /// The audit record could not be written, so nothing was done.
    AuditFailed,
    /// The tenant is being removed.
    Closing,
    /// server.sqlite was made under another master key.
    WrongKey,
    Io,
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
        let registry = Tenants {
            tenants,
            master,
            conn: Mutex::new(conn),
            stores: Mutex::new(HashMap::new()),
            fs,
            clock,
            #[cfg(test)]
            hooks: tests::Hooks::default(),
        };
        registry.recover()?;
        Ok(registry)
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
            let refused = if name_taken(&tx, name)? {
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
        // durable: the directory's entry in `tenants/` by syncing it.
        let dir = self.tenants.join(id.to_string());
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
        let now = self.clock.now();
        let result = if name_taken(&tx, name)? {
            self.record(&tx, &entry(409, None))
                .and(Err(TenantError::NameTaken))
        } else {
            tx.execute(
                "INSERT INTO tenant (id, name, retention, created_at, tenant_key_id,
                     wrapped_tenant_key, key_since)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?4)",
                params![
                    id.0,
                    name,
                    retention.map(|r| r.to_string()),
                    nanos(now),
                    key_id.bytes(),
                    wrapped
                ],
            )
            .map_err(io)?;
            self.record(&tx, &entry(201, Some(id))).map(|()| id)
        };
        match result {
            Ok(id) => {
                tx.commit().map_err(io)?;
                Ok(id)
            }
            Err(error) => {
                // A refusal's record commits; a failed record leaves nothing.
                if error == TenantError::NameTaken {
                    tx.commit().map_err(io)?;
                } else {
                    drop(tx);
                }
                drop(conn);
                let _ = self.fs.remove_dir_all(&dir);
                Err(error)
            }
        }
    }

    /// The tenant's Store, shared by every request to it.
    pub fn store(&self, id: TenantId) -> Result<Arc<Store>, TenantError> {
        // The Store is opened under this lock, so it is opened once: a
        // second open's recovery could remove a write's temporary file.
        let mut stores = self.stores();
        match stores.get(&id) {
            Some(Slot::Open(store)) => return Ok(store.clone()),
            Some(Slot::Closing) => return Err(TenantError::Closing),
            None => {}
        }
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
        let store = Arc::new(
            Store::open(
                &self.tenants.join(id.to_string()),
                id,
                tenant_key,
                self.fs.clone(),
                self.clock.clone(),
            )
            .map_err(io)?,
        );
        stores.insert(id, Slot::Open(store.clone()));
        Ok(store)
    }

    /// Marks the tenant closing, so no new handle is handed out, and waits
    /// until every handle given out before is dropped. Handles are plain
    /// `Arc`s, so the wait watches the count; requests are short and
    /// removal is rare.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "tenant removal, which drains, comes next")
    )]
    pub(crate) fn drain(&self, id: TenantId) {
        let slot = self.stores().insert(id, Slot::Closing);
        if let Some(Slot::Open(store)) = slot {
            while Arc::strong_count(&store) > 1 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }

    fn stores(&self) -> MutexGuard<'_, HashMap<TenantId, Slot>> {
        self.stores
            .lock()
            .expect("no thread panicked holding the stores")
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
    use crate::store::Caller;
    use crate::store::testing::vcon;

    const MARKER: &str = "plain-record-marker-8d21e4";
    const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

    fn caller() -> Caller {
        Caller {
            key: Key::generate().expect("key").id(),
        }
    }

    #[derive(Default)]
    pub(super) struct Hooks {
        /// Runs once, between a create's directory and catalog and the
        /// transaction that commits its row.
        pub(super) between: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        /// Runs once, as `store` opens a tenant's Store.
        pub(super) opening: Mutex<Option<Box<dyn FnOnce() + Send>>>,
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

    struct Fixture {
        dir: tempfile::TempDir,
        fs: Arc<CrashFs>,
        clock: Arc<ManualClock>,
        master: Master,
        tenants: Tenants,
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

        fn conn(&self) -> MutexGuard<'_, Connection> {
            self.tenants.conn()
        }

        fn tenants_dir(&self) -> PathBuf {
            self.dir.path().join("tenants")
        }

        /// The tenant directories, as the filesystem shows them now.
        fn dirs(&self) -> Vec<OsString> {
            self.fs.list(&self.tenants_dir()).expect("list tenants")
        }

        fn count(&self, sql: &str) -> i64 {
            self.conn().query_row(sql, [], |r| r.get(0)).expect(sql)
        }

        fn master_wraps(&self) -> i64 {
            self.count("SELECT coalesce(sum(wraps), 0) FROM master_key_use")
        }

        fn admin_log(&self) -> Vec<Value> {
            exported(&self.conn(), None)
        }

        /// The tenant's key, unwrapped from its row.
        fn tenant_key(&self, id: TenantId) -> Key {
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
        fn store(&self, id: TenantId) -> Store {
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

    fn fixture() -> Fixture {
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
            ["audit", "master_key_use", "sqlite_sequence", "tenant"]
        );
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
        let mut may_be_empty = columns_taking_null(&conn, "tenant", &tenant);
        may_be_empty.extend(columns_taking_null(&conn, "master_key_use", &key_use));
        assert_eq!(
            may_be_empty,
            ["tenant.retention", "tenant.pending_wrapped_tenant_key"]
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
                tenants.drain(id);
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
        f.tenants.drain(id);
        assert_eq!(f.tenants.store(id).err(), Some(TenantError::Closing));
    }
}
