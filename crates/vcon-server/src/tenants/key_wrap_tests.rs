//! The registry reaches tenant keys only through its key-wrapping backend:
//! it derives server.sqlite's key from the backend's root, counts wraps
//! against the backend's key id, and asks the backend to wrap and unwrap
//! each tenant key. A backend that refuses leaves nothing half made.

use std::collections::VecDeque;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{OptionalExtension, params};
use time::macros::datetime;

use super::{RegistryError, Removal, TenantError, Tenants, rekey};
use crate::audit::Actor;
use crate::backend::{Catalogs, ObjectPlace};
use crate::catalog;
use crate::clock::{ManualClock, nanos};
use crate::envelope::{self, Context, Key};
use crate::fs::CrashFs;
use crate::ids::{KeyId, TenantId};
use crate::key_wrap::{FileWrap, KeyWrap, WrapError, Wrapped, WrappedKek};
use crate::store::Caller;
use crate::store::testing::vcon;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

/// What the registry asked of the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    Wrap(TenantId),
    Unwrap(TenantId),
}

/// A failure the backend is told to give at its next call of a kind.
#[derive(Debug, Clone, Copy)]
enum Fail {
    Damaged,
    Memory,
}

impl Fail {
    fn error(self) -> WrapError {
        match self {
            Fail::Damaged => WrapError::Damaged,
            Fail::Memory => WrapError::Memory(std::io::Error::from_raw_os_error(libc::ENOMEM)),
        }
    }
}

/// The file backend, with each call logged and failures given in the order
/// a test queues them (`None` lets that call through). Its key id and its
/// root may be other keys than the master key, to show which one the
/// registry uses for what.
#[derive(Debug)]
struct Fake {
    inner: FileWrap,
    key_id: Option<KeyId>,
    root: Option<Key>,
    calls: Mutex<Vec<Call>>,
    wraps: Mutex<VecDeque<Option<Fail>>>,
    unwraps: Mutex<VecDeque<Option<Fail>>>,
}

impl Fake {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("calls").clone()
    }

    fn unwraps_of(&self, id: TenantId) -> usize {
        self.calls()
            .iter()
            .filter(|c| **c == Call::Unwrap(id))
            .count()
    }

    fn fail_wraps(&self, fails: &[Option<Fail>]) {
        self.wraps.lock().expect("wraps").extend(fails);
    }

    fn fail_unwraps(&self, fails: &[Option<Fail>]) {
        self.unwraps.lock().expect("unwraps").extend(fails);
    }
}

/// The backend as the registry holds it, while the test keeps a handle.
#[derive(Debug)]
struct Shared(Arc<Fake>);

impl KeyWrap for Shared {
    fn key_id(&self) -> KeyId {
        self.0.key_id.unwrap_or_else(|| self.0.inner.key_id())
    }

    fn root(&self) -> &Key {
        self.0.root.as_ref().unwrap_or_else(|| self.0.inner.root())
    }

    fn wrap(&self, tenant: TenantId, key: &Key) -> Result<Wrapped, WrapError> {
        self.0.calls.lock().expect("calls").push(Call::Wrap(tenant));
        match self.0.wraps.lock().expect("wraps").pop_front().flatten() {
            Some(fail) => Err(fail.error()),
            None => self.0.inner.wrap(tenant, key),
        }
    }

    fn unwrap(
        &self,
        tenant: TenantId,
        wrapped: &[u8],
        kek: Option<&WrappedKek>,
    ) -> Result<Key, WrapError> {
        self.0
            .calls
            .lock()
            .expect("calls")
            .push(Call::Unwrap(tenant));
        match self
            .0
            .unwraps
            .lock()
            .expect("unwraps")
            .pop_front()
            .flatten()
        {
            Some(fail) => Err(fail.error()),
            None => self.0.inner.unwrap(tenant, wrapped, kek),
        }
    }
}

/// A master key file of 32 random bytes, mode 0600, in `dir`.
fn master_file(dir: &Path) -> PathBuf {
    let path = dir.join("master.key");
    let bytes: [u8; 32] = aws_lc_rs::rand::generate(&aws_lc_rs::rand::SystemRandom::new())
        .expect("random bytes")
        .expose();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("create the key file")
        .write_all(&bytes)
        .expect("write the key file");
    path
}

struct World {
    /// The data directory.
    data: tempfile::TempDir,
    /// Where the master key file is, apart from the data.
    _keys: tempfile::TempDir,
    master: PathBuf,
    fs: Arc<CrashFs>,
    clock: Arc<ManualClock>,
}

impl World {
    fn new() -> World {
        let data = tempfile::tempdir().expect("temp dir");
        let keys = tempfile::tempdir().expect("temp dir");
        let master = master_file(keys.path());
        let fs = Arc::new(CrashFs::new(data.path()));
        let clock = Arc::new(ManualClock::at(datetime!(2026-11-24 12:00 UTC)));
        World {
            data,
            _keys: keys,
            master,
            fs,
            clock,
        }
    }

    fn master(&self) -> Key {
        Key::load_master(&self.master).expect("the master key")
    }

    fn fake(&self) -> Arc<Fake> {
        self.fake_with(None, None)
    }

    fn fake_with(&self, key_id: Option<KeyId>, root: Option<Key>) -> Arc<Fake> {
        Arc::new(Fake {
            inner: FileWrap::new(self.master()),
            key_id,
            root,
            calls: Mutex::new(Vec::new()),
            wraps: Mutex::new(VecDeque::new()),
            unwraps: Mutex::new(VecDeque::new()),
        })
    }

    /// The registry over `fake`, opened as a start opens it.
    fn open(&self, fake: &Arc<Fake>) -> Result<Tenants, RegistryError> {
        self.open_as(fake, false)
    }

    fn open_as(&self, fake: &Arc<Fake>, beside: bool) -> Result<Tenants, RegistryError> {
        let tenants = Tenants::opened(
            self.data.path(),
            Box::new(Shared(fake.clone())),
            self.fs.clone(),
            self.clock.clone(),
            beside,
            ObjectPlace::Local,
            &Catalogs::Local,
        )?;
        if !beside {
            tenants
                .recover(crate::store::pending::Writers::None)
                .map_err(RegistryError::Recovery)?;
        }
        Ok(tenants)
    }

    fn server_sqlite(&self) -> PathBuf {
        self.data.path().join("server.sqlite")
    }

    /// A connection to server.sqlite under the catalog key of `parent`.
    fn registry_under(&self, parent: &Key) -> Result<rusqlite::Connection, catalog::CatalogError> {
        let key = envelope::catalog_key(parent).expect("catalog key");
        catalog::open(&self.server_sqlite(), &key, "")
    }

    /// The tenant's row's (wrapped, pending) keys, read under the master
    /// key's catalog key.
    fn row_keys(&self, id: TenantId) -> (Vec<u8>, Option<Vec<u8>>) {
        self.registry_under(&self.master())
            .expect("server.sqlite under the master key")
            .query_row(
                "SELECT wrapped_tenant_key, pending_wrapped_tenant_key FROM tenant WHERE id = ?1",
                [id.0],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("the tenant's row")
    }

    fn wraps_of(&self, conn: &rusqlite::Connection, key: KeyId) -> Option<i64> {
        conn.query_row(
            "SELECT wraps FROM master_key_use WHERE master_key_id = ?1",
            [key.0],
            |r| r.get(0),
        )
        .optional()
        .expect("query")
    }

    fn tenant_dirs(&self) -> usize {
        match std::fs::read_dir(self.data.path().join("tenants")) {
            Ok(entries) => entries.count(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
            Err(e) => panic!("read tenants/: {e}"),
        }
    }
}

fn caller() -> Caller {
    Caller {
        key: Key::generate().expect("key").id(),
    }
}

fn opened_under(key: &Key, id: TenantId, wrapped: &[u8]) -> Key {
    envelope::unwrap(key, &Context::Tenant { tenant: id }, wrapped)
        .expect("the row's key opens under the master key, as before the trait")
}

// Success.

#[test]
fn a_create_wraps_the_new_tenant_key_once_for_its_tenant() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    assert_eq!(
        fake.calls(),
        vec![Call::Wrap(id)],
        "one wrap, for this tenant"
    );
    let (wrapped, pending) = w.row_keys(id);
    let _ = opened_under(&w.master(), id, &wrapped);
    assert_eq!(pending, None, "nothing pending");
}

#[test]
fn a_tenants_key_is_unwrapped_once_and_its_store_kept() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    let a = tenants.store(id).expect("opens");
    let b = tenants.store(id).expect("opens");
    assert!(Arc::ptr_eq(&a, &b), "the same Store");
    assert_eq!(
        fake.unwraps_of(id),
        1,
        "one unwrap for the life of the Store"
    );
}

#[test]
fn the_registry_is_under_the_backends_root() {
    let w = World::new();
    let root = Key::generate().expect("a root other than the master key");
    let root_id = root.id();
    let fake = w.fake_with(None, Some(root));
    drop(w.open(&fake).expect("opens"));
    assert!(
        w.registry_under(fake.root.as_ref().expect("root")).is_ok(),
        "server.sqlite opens under the root's catalog key ({root_id})"
    );
    assert!(
        w.registry_under(&w.master()).is_err(),
        "and not under the master key's, which only wraps tenant keys here"
    );
}

#[test]
fn wraps_are_counted_against_the_backends_key_id() {
    let w = World::new();
    let counted = KeyId([9; 8]);
    let fake = w.fake_with(Some(counted), None);
    let tenants = w.open(&fake).expect("opens");
    tenants.create(Actor::Cli, "acme", None).expect("made");
    tenants.create(Actor::Cli, "globex", None).expect("made");
    let conn = w.registry_under(&w.master()).expect("server.sqlite");
    assert_eq!(
        w.wraps_of(&conn, counted),
        Some(2),
        "both wraps, under the backend's key id"
    );
    assert_eq!(
        w.wraps_of(&conn, w.master().id()),
        None,
        "none under another id"
    );
}

#[test]
fn a_rekeys_wrap_is_counted_against_the_backends_key_id() {
    let w = World::new();
    let counted = KeyId([9; 8]);
    let fake = w.fake_with(Some(counted), None);
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    tenants.rekey_tenant(Actor::Cli, id).expect("rotates");
    let conn = w.registry_under(&w.master()).expect("server.sqlite");
    assert_eq!(
        w.wraps_of(&conn, counted),
        Some(2),
        "the create's wrap and the rotation's, under the backend's key id"
    );
    assert_eq!(
        w.wraps_of(&conn, w.master().id()),
        None,
        "none under another id"
    );
}

#[test]
fn key_order_unwraps_each_tenant_through_the_backend() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let a = tenants.create(Actor::Cli, "acme", None).expect("made");
    let b = tenants.create(Actor::Cli, "globex", None).expect("made");
    let order = tenants.key_order().expect("read");
    assert_eq!(order.wrapped_elsewhere, 0, "every key opens");
    assert!(order.is_sound(), "the start may go on");
    assert!(
        fake.unwraps_of(a) >= 1 && fake.unwraps_of(b) >= 1,
        "each tenant's key was asked for"
    );
}

#[test]
fn a_rekey_unwraps_the_old_key_and_wraps_the_new_one() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    let before = opened_under(&w.master(), id, &w.row_keys(id).0).id();
    let made = fake.calls().len();
    tenants.rekey_tenant(Actor::Cli, id).expect("rotates");
    // The rotation drains the tenant's Store, and its record in the
    // tenant's audit log opens the Store again under the promoted key
    // (`copy_to_tenant`): the third call.
    assert_eq!(
        fake.calls()[made..],
        [Call::Unwrap(id), Call::Wrap(id), Call::Unwrap(id)],
        "the old key unwrapped, the new one wrapped as pending, then unwrapped once promoted"
    );
    let (wrapped, pending) = w.row_keys(id);
    let after = opened_under(&w.master(), id, &wrapped).id();
    assert_ne!(after, before, "a new tenant key");
    assert_eq!(pending, None, "promoted");
}

#[test]
fn a_registry_opened_beside_a_server_unwraps_through_the_backend() {
    let w = World::new();
    let fake = w.fake();
    let id = {
        let tenants = w.open(&fake).expect("opens");
        let id = tenants.create(Actor::Cli, "acme", None).expect("made");
        let store = tenants.store(id).expect("opens");
        let (found, body) = vcon(UUID, "kept");
        store.create(&caller(), &found, &body).expect("stores");
        id
    };
    let beside = w.fake();
    let tenants = w.open_as(&beside, true).expect("opens beside");
    let store = tenants.store(id).expect("opens");
    let read = store
        .get(&caller(), UUID.parse().expect("uuid"))
        .expect("reads back");
    assert_eq!(read.body, vcon(UUID, "kept").1, "the same body");
    assert_eq!(
        beside.calls(),
        vec![Call::Unwrap(id)],
        "through the backend"
    );
}

// Failure.

#[test]
fn a_create_whose_wrap_fails_makes_no_tenant() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    fake.fail_wraps(&[Some(Fail::Memory)]);
    assert_eq!(
        tenants.create(Actor::Cli, "acme", None),
        Err(TenantError::Io),
        "refused"
    );
    assert!(tenants.list(Actor::Cli).expect("list").is_empty(), "no row");
    assert_eq!(w.tenant_dirs(), 0, "no directory");
    let conn = w.registry_under(&w.master()).expect("server.sqlite");
    assert_eq!(
        w.wraps_of(&conn, w.master().id()),
        Some(1),
        "the wrap was counted before it was tried, as every wrap is"
    );
}

#[test]
fn a_store_whose_key_does_not_unwrap_does_not_open() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    fake.fail_unwraps(&[Some(Fail::Damaged)]);
    assert_eq!(tenants.store(id).err(), Some(TenantError::Io), "refused");
}

#[test]
fn a_store_that_did_not_open_is_tried_again() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    fake.fail_unwraps(&[Some(Fail::Damaged)]);
    assert!(tenants.store(id).is_err(), "the first open fails");
    assert!(tenants.store(id).is_ok(), "the next one opens");
    assert_eq!(fake.unwraps_of(id), 2, "a failure is not kept");
}

#[test]
fn a_key_whose_memory_cannot_be_locked_opens_no_store() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    fake.fail_unwraps(&[Some(Fail::Memory)]);
    assert_eq!(tenants.store(id).err(), Some(TenantError::Io), "refused");
}

#[test]
fn key_order_counts_a_key_the_backend_cannot_open() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    tenants.create(Actor::Cli, "acme", None).expect("made");
    fake.fail_unwraps(&[Some(Fail::Damaged)]);
    let order = tenants.key_order().expect("read");
    assert_eq!(order.wrapped_elsewhere, 1, "the key that did not open");
    assert!(!order.is_sound(), "a start refuses");
}

#[test]
fn a_rekey_whose_old_key_does_not_unwrap_changes_nothing() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    let before = w.row_keys(id);
    fake.fail_unwraps(&[Some(Fail::Damaged)]);
    assert_eq!(
        tenants.rekey_tenant(Actor::Cli, id),
        Err(TenantError::Io),
        "refused"
    );
    assert_eq!(w.row_keys(id), before, "the row is as it was");
    let conn = w.registry_under(&w.master()).expect("server.sqlite");
    assert_eq!(
        w.wraps_of(&conn, w.master().id()),
        Some(1),
        "no new wrap counted"
    );
}

#[test]
fn a_rekey_whose_new_key_cannot_be_wrapped_changes_nothing() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    let before = w.row_keys(id);
    fake.fail_wraps(&[Some(Fail::Memory)]);
    assert_eq!(
        tenants.rekey_tenant(Actor::Cli, id),
        Err(TenantError::Io),
        "refused"
    );
    assert_eq!(w.row_keys(id), before, "no pending key");
    let conn = w.registry_under(&w.master()).expect("server.sqlite");
    assert_eq!(
        w.wraps_of(&conn, w.master().id()),
        Some(1),
        "the count rolls back with the pending key"
    );
}

#[test]
fn a_create_at_the_backend_keys_wrap_limit_is_refused_unwrapped() {
    let w = World::new();
    let counted = KeyId([9; 8]);
    let fake = w.fake_with(Some(counted), None);
    let tenants = w.open(&fake).expect("opens");
    w.registry_under(&w.master())
        .expect("server.sqlite")
        .execute(
            "INSERT INTO master_key_use (master_key_id, wraps, since) VALUES (?1, ?2, 1)",
            params![counted.0, 1_i64 << 32],
        )
        .expect("at the limit");
    assert_eq!(
        tenants.create(Actor::Cli, "acme", None),
        Err(TenantError::LimitReached),
        "the backend's key is at its limit"
    );
    assert!(fake.calls().is_empty(), "nothing was wrapped");
}

#[test]
fn a_resumed_rekey_whose_pending_key_does_not_unwrap_keeps_it() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    *tenants.hooks.rekey_stop.lock().expect("hook") = Some(rekey::RekeyStep::Pending);
    assert!(
        tenants.rekey_tenant(Actor::Cli, id).is_err(),
        "stopped after the pending key"
    );
    let stopped = w.row_keys(id);
    assert!(stopped.1.is_some(), "a pending key is kept");
    // The old key opens, the pending one does not.
    fake.fail_unwraps(&[None, Some(Fail::Damaged)]);
    assert_eq!(
        tenants.rekey_tenant(Actor::Cli, id),
        Err(TenantError::Io),
        "refused"
    );
    assert_eq!(w.row_keys(id), stopped, "the pending key is not lost");
}

#[test]
fn a_registry_under_another_root_does_not_open() {
    let w = World::new();
    let fake = w.fake_with(None, Some(Key::generate().expect("root")));
    drop(w.open(&fake).expect("made under one root"));
    let other = w.fake_with(None, Some(Key::generate().expect("another root")));
    assert!(
        matches!(w.open(&other), Err(RegistryError::WrongKey)),
        "server.sqlite names another key"
    );
}

#[test]
fn a_restart_whose_backend_cannot_open_tenant_keys_opens_no_tenant() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    drop(tenants);
    // The same root, but a backend that opens no tenant key, as one holding
    // another wrapping key would.
    let other = w.fake();
    other.fail_unwraps(&[Some(Fail::Damaged), Some(Fail::Damaged)]);
    let tenants = w.open(&other).expect("the registry opens");
    assert_eq!(
        tenants.store(id).err(),
        Some(TenantError::Io),
        "the tenant does not"
    );
}

// Edge.

#[test]
fn a_resumed_rekey_unwraps_the_pending_key_and_wraps_nothing() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    *tenants.hooks.rekey_stop.lock().expect("hook") = Some(rekey::RekeyStep::Pending);
    assert!(
        tenants.rekey_tenant(Actor::Cli, id).is_err(),
        "stopped after the pending key"
    );
    let made = fake.calls().len();
    tenants.rekey_tenant(Actor::Cli, id).expect("finishes");
    // The third unwrap is the Store reopened under the promoted key for the
    // rotation's record in the tenant's audit log (`copy_to_tenant`).
    assert_eq!(
        fake.calls()[made..],
        [Call::Unwrap(id), Call::Unwrap(id), Call::Unwrap(id)],
        "the old and the pending key unwrapped, then the promoted one; no new key wrapped"
    );
}

#[test]
fn removals_before_the_backend_keys_use_began_wait_for_no_rotation() {
    let w = World::new();
    let counted = KeyId([9; 8]);
    let fake = w.fake_with(Some(counted), None);
    let tenants = w.open(&fake).expect("opens");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    w.clock.set(datetime!(2026-11-24 12:01 UTC));
    tenants
        .remove(Actor::Cli, id, Removal::Empty)
        .expect("removed");
    assert_eq!(
        tenants.rotation_backlog().expect("read").removals,
        1,
        "removed after the backend key's use began"
    );
    // As if the backend's key were rotated after the removal.
    let since = nanos(datetime!(2026-11-24 12:02 UTC));
    w.registry_under(&w.master())
        .expect("server.sqlite")
        .execute(
            "UPDATE master_key_use SET since = ?1 WHERE master_key_id = ?2",
            params![since, counted.0],
        )
        .expect("a later start of use");
    assert_eq!(
        tenants.rotation_backlog().expect("read").removals,
        0,
        "a removal before the backend key's use began waits for no rotation"
    );
}

#[test]
fn concurrent_opens_of_one_store_unwrap_once() {
    let w = World::new();
    let fake = w.fake();
    let tenants = Arc::new(w.open(&fake).expect("opens"));
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let tenants = tenants.clone();
            std::thread::spawn(move || tenants.store(id).is_ok())
        })
        .collect();
    for thread in threads {
        assert!(thread.join().expect("the thread ends"), "opens");
    }
    assert_eq!(
        fake.unwraps_of(id),
        1,
        "the open is made once, under the lock"
    );
}

#[test]
fn key_order_of_no_tenants_asks_nothing() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    let order = tenants.key_order().expect("read");
    assert!(order.is_sound(), "nothing out of order");
    assert!(fake.calls().is_empty(), "no call to the backend");
}

#[test]
fn a_failed_create_leaves_its_name_free() {
    let w = World::new();
    let fake = w.fake();
    let tenants = w.open(&fake).expect("opens");
    fake.fail_wraps(&[Some(Fail::Damaged)]);
    assert!(tenants.create(Actor::Cli, "acme", None).is_err(), "refused");
    let id = tenants
        .create(Actor::Cli, "acme", None)
        .expect("the name is free");
    assert!(tenants.store(id).is_ok(), "and the tenant opens");
}

#[test]
fn the_root_and_the_wrapping_key_may_be_different_keys() {
    let w = World::new();
    // One root other than the master key, kept wrapped so that each open of
    // the registry can be handed a copy of it.
    let holder = Key::generate().expect("holder");
    let ctx = Context::Tenant {
        tenant: TenantId([1; 16]),
    };
    let wrapped_root = envelope::wrap(&holder, &ctx, &Key::generate().expect("root"));
    let again = || envelope::unwrap(&holder, &ctx, &wrapped_root).expect("root");
    let fake = w.fake_with(None, Some(again()));
    let id = {
        let tenants = w.open(&fake).expect("opens");
        let id = tenants.create(Actor::Cli, "acme", None).expect("made");
        let store = tenants.store(id).expect("opens");
        let (found, body) = vcon(UUID, "kept");
        store.create(&caller(), &found, &body).expect("stores");
        id
    };
    let tenants = w
        .open(&w.fake_with(None, Some(again())))
        .expect("reopens under the same root");
    let read = tenants
        .store(id)
        .expect("the tenant opens through the master key")
        .get(&caller(), UUID.parse().expect("uuid"))
        .expect("reads back");
    assert_eq!(read.body, vcon(UUID, "kept").1, "the same body");
    assert!(
        w.registry_under(&w.master()).is_err(),
        "server.sqlite is under the root, not the master key"
    );
}

#[test]
fn a_tenant_made_through_the_file_backend_opens_under_tenants_open() {
    let w = World::new();
    let id = {
        let tenants = w.open(&w.fake()).expect("opens");
        tenants.create(Actor::Cli, "acme", None).expect("made")
    };
    // The public opener, which every command and the server use.
    let tenants = Tenants::open(w.data.path(), w.master(), w.fs.clone(), w.clock.clone())
        .expect("opens with the master key alone");
    assert!(tenants.store(id).is_ok(), "the tenant opens");
}
