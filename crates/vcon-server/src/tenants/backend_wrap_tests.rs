//! The registry over a key backend: tenants made, opened and rotated
//! through `BackendWrap` keep their key-encryption keys in their rows, and
//! a registry opened with the master key file refuses a data directory
//! whose root a key backend wraps, naming the backend.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use time::macros::datetime;

use super::{OpenStep, ROOT_FILE, RegistryError, Tenants};
use crate::audit::Actor;
use crate::backend::{Catalogs, ObjectPlace};
use crate::catalog;
use crate::clock::ManualClock;
use crate::envelope::{self, Key};
use crate::fs::CrashFs;
use crate::ids::TenantId;
use crate::key_wrap::blob::BackendKind;
use crate::key_wrap::testing::{Call, Fail, Scripted};
use crate::key_wrap::{BackendWrap, KeyWrap, WrapContext};
use crate::store::Caller;
use crate::store::testing::vcon;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

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
    data: tempfile::TempDir,
    _keys: tempfile::TempDir,
    master: PathBuf,
    fs: Arc<CrashFs>,
    clock: Arc<ManualClock>,
    backend: Scripted,
    /// The root's key-wrap blob, as registry.key holds it.
    root: Vec<u8>,
}

impl World {
    fn new() -> World {
        let data = tempfile::tempdir().expect("temp dir");
        let keys = tempfile::tempdir().expect("temp dir");
        let master = master_file(keys.path());
        let backend = Scripted::new(BackendKind::Pkcs11, "vcon-1");
        let (_, root) = BackendWrap::create(backend.clone()).expect("a root");
        std::fs::write(data.path().join(ROOT_FILE), &root).expect("write registry.key");
        World {
            fs: Arc::new(CrashFs::new(data.path())),
            clock: Arc::new(ManualClock::at(datetime!(2026-11-25 12:00 UTC))),
            data,
            _keys: keys,
            master,
            backend,
            root,
        }
    }

    /// The registry over the backend, opened as a start opens it.
    fn open(&self) -> Tenants {
        let wrap = BackendWrap::open(self.backend.clone(), &self.root).expect("the root opens");
        let tenants = Tenants::opened(
            self.data.path(),
            Box::new(wrap),
            self.fs.clone(),
            self.clock.clone(),
            false,
            ObjectPlace::Local,
            &Catalogs::Local,
        )
        .expect("opens");
        tenants
            .recover(crate::store::pending::Writers::Maybe)
            .expect("recovers");
        tenants
    }

    /// The registry opened with the master key file, as every release so
    /// far opens one.
    fn open_with_file(&self) -> Result<Tenants, RegistryError> {
        Tenants::open(
            self.data.path(),
            Key::load_master(&self.master).expect("the master key"),
            self.fs.clone(),
            self.clock.clone(),
        )
    }

    fn open_beside_with_file(&self) -> Result<Tenants, RegistryError> {
        Tenants::open_beside(
            self.data.path(),
            Key::load_master(&self.master).expect("the master key"),
            self.fs.clone(),
            self.clock.clone(),
        )
    }

    /// server.sqlite under the root's catalog key.
    fn registry(&self, tenants: &Tenants) -> rusqlite::Connection {
        let key = envelope::catalog_key(tenants.wrap.root()).expect("catalog key");
        catalog::open(&self.data.path().join("server.sqlite"), &key, "").expect("server.sqlite")
    }

    fn row(&self, tenants: &Tenants, id: TenantId) -> crate::backend::TenantRow {
        tenants
            .registry
            .snapshot()
            .expect("a snapshot")
            .tenant(id)
            .expect("reads")
            .expect("the row")
    }

    fn write_root(&self, bytes: &[u8]) {
        std::fs::write(self.data.path().join(ROOT_FILE), bytes).expect("write registry.key");
    }
}

fn caller() -> Caller {
    Caller {
        key: Key::generate().expect("key").id(),
    }
}

fn store_one(tenants: &Tenants, id: TenantId) {
    let store = tenants.store(id).expect("opens");
    let (found, body) = vcon(UUID, "kept");
    store.create(&caller(), &found, &body).expect("stores");
}

fn read_one(tenants: &Tenants, id: TenantId) -> Vec<u8> {
    tenants
        .store(id)
        .expect("opens")
        .get(&caller(), UUID.parse().expect("uuid"))
        .expect("reads back")
        .body
}

// Success.

#[test]
fn a_tenant_made_under_a_backend_keeps_its_kek_in_its_row() {
    let w = World::new();
    let tenants = w.open();
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    let row = w.row(&tenants, id);
    let kek = row.kek.expect("the key-encryption key is recorded");
    assert_eq!(kek.wrap_ref, "pkcs11:vcon-1", "under the backend's key");
    assert_eq!(row.pending_kek, None, "nothing pending");
}

#[test]
fn a_tenants_data_reads_back_after_a_restart_of_the_backend() {
    let mut w = World::new();
    let id = {
        let tenants = w.open();
        let id = tenants.create(Actor::Cli, "acme", None).expect("made");
        store_one(&tenants, id);
        id
    };
    w.backend = w.backend.restarted();
    let tenants = w.open();
    assert_eq!(
        read_one(&tenants, id),
        vcon(UUID, "kept").1,
        "the same body"
    );
    assert!(
        w.backend
            .calls()
            .contains(&Call::Unwrap(WrapContext::Tenant(id))),
        "the tenant's key-encryption key was unwrapped by the backend"
    );
}

#[test]
fn a_tenant_rekey_under_a_backend_gives_a_new_kek() {
    let w = World::new();
    let tenants = w.open();
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    store_one(&tenants, id);
    let before = w.row(&tenants, id).kek.expect("a kek");
    tenants.rekey_tenant(Actor::Cli, id).expect("rotates");
    let row = w.row(&tenants, id);
    let after = row.kek.expect("a kek");
    assert_ne!(after.blob, before.blob, "a new key-encryption key");
    assert_eq!(row.pending_kek, None, "promoted");
    assert_eq!(read_one(&tenants, id), vcon(UUID, "kept").1, "still reads");
}

#[test]
fn key_order_under_a_backend_finds_every_tenant_in_order() {
    let w = World::new();
    let tenants = w.open();
    for name in ["acme", "globex"] {
        tenants.create(Actor::Cli, name, None).expect("made");
    }
    let order = tenants.key_order().expect("reads");
    assert_eq!(order.wrapped_elsewhere, 0, "every key opens: {order:?}");
    assert!(order.is_sound(), "in order: {order:?}");
}

#[test]
fn the_registry_is_under_the_roots_catalog_key() {
    let w = World::new();
    let tenants = w.open();
    tenants.create(Actor::Cli, "acme", None).expect("made");
    let conn = w.registry(&tenants);
    let count: i64 = conn
        .query_row("SELECT count(*) FROM tenant", [], |r| r.get(0))
        .expect("counts");
    assert_eq!(count, 1, "server.sqlite opens under the root's catalog key");
    let master = Key::load_master(&w.master).expect("the master key");
    assert!(
        catalog::open(
            &w.data.path().join("server.sqlite"),
            &envelope::catalog_key(&master).expect("catalog key"),
            ""
        )
        .is_err(),
        "and not under the master key's"
    );
}

#[test]
fn a_pending_key_under_a_backend_is_recorded_with_its_kek() {
    let w = World::new();
    let tenants = w.open();
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    *tenants.hooks.rekey_stop.lock().expect("hook") = Some(super::rekey::RekeyStep::Pending);
    assert!(tenants.rekey_tenant(Actor::Cli, id).is_err(), "stopped");
    let row = w.row(&tenants, id);
    assert!(row.pending.is_some(), "a pending key");
    assert_eq!(
        row.pending_kek.map(|k| k.wrap_ref),
        Some("pkcs11:vcon-1".to_owned()),
        "under its own key-encryption key"
    );
    tenants
        .rekey_tenant(Actor::Cli, id)
        .expect("the run again finishes");
    assert_eq!(w.row(&tenants, id).pending_kek, None, "promoted");
}

// Failure.

#[test]
fn the_master_key_file_refuses_a_root_a_backend_wraps_naming_it() {
    let w = World::new();
    let result = w.open_with_file();
    assert!(
        matches!(
            result,
            Err(RegistryError::RootWrapped(
                Some(BackendKind::Pkcs11),
                "registry.key"
            ))
        ),
        "the backend's kind: {:?}",
        result.err()
    );
}

#[test]
fn the_refusal_says_which_backend_wraps_the_root() {
    let w = World::new();
    let text = w.open_with_file().err().expect("refused").to_string();
    assert_eq!(
        text,
        "the registry's root key is wrapped by the key backend pkcs11 (registry.key), not by the \
         master key file",
        "the text"
    );
}

#[test]
fn a_command_beside_a_server_refuses_it_too() {
    let w = World::new();
    let result = w.open_beside_with_file();
    assert!(
        matches!(
            result,
            Err(RegistryError::RootWrapped(
                Some(BackendKind::Pkcs11),
                "registry.key"
            ))
        ),
        "{:?}",
        result.err()
    );
}

#[test]
fn the_refusal_makes_no_registry() {
    let w = World::new();
    assert!(w.open_with_file().is_err(), "refused");
    assert!(
        !w.data.path().join("server.sqlite").exists(),
        "nothing was made"
    );
}

#[test]
fn a_row_under_another_backend_key_is_counted_elsewhere_without_a_call() {
    let w = World::new();
    let tenants = w.open();
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    // A second tenant stays under this backend's key: its row's
    // key-encryption key reaches the backend, and its key opens.
    let ours = tenants.create(Actor::Cli, "globex", None).expect("made");
    let other = Scripted::new(BackendKind::Pkcs11, "vcon-2");
    let (theirs, _) = BackendWrap::create(other).expect("a root");
    let wrapped = theirs
        .wrap(id, &Key::generate().expect("key"))
        .expect("wraps");
    let kek = wrapped.kek.expect("a kek");
    w.registry(&tenants)
        .execute(
            "UPDATE tenant SET wrapped_tenant_key = ?1, wrap_ref = ?2, wrapped_kek = ?3 \
             WHERE id = ?4",
            rusqlite::params![wrapped.key, kek.wrap_ref, kek.blob, id.0],
        )
        .expect("moves the row under the other key");
    w.backend.clear_calls();
    let order = tenants.key_order().expect("reads");
    assert_eq!(order.wrapped_elsewhere, 1, "only acme: {order:?}");
    let calls = w.backend.calls();
    assert!(
        calls.contains(&Call::Unwrap(WrapContext::Tenant(ours))),
        "the backend was asked for globex's key: {calls:?}"
    );
    assert!(
        !calls.contains(&Call::Unwrap(WrapContext::Tenant(id))),
        "and not for acme's: {calls:?}"
    );
}

#[test]
fn a_store_whose_key_the_backend_refuses_does_not_open() {
    let w = World::new();
    let tenants = w.open();
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    w.backend.fail_unwraps(&[Some(Fail::Damaged)]);
    assert!(tenants.store(id).is_err(), "refused");
    assert!(tenants.store(id).is_ok(), "the next try opens");
}

#[test]
fn a_create_the_backend_refuses_leaves_no_tenant() {
    let w = World::new();
    let tenants = w.open();
    w.backend.fail_wraps(&[Some(Fail::Damaged)]);
    assert!(tenants.create(Actor::Cli, "acme", None).is_err(), "refused");
    let count: i64 = w
        .registry(&tenants)
        .query_row("SELECT count(*) FROM tenant", [], |r| r.get(0))
        .expect("counts");
    assert_eq!(count, 0, "no row");
    let dirs = std::fs::read_dir(w.data.path().join("tenants"))
        .map(Iterator::count)
        .unwrap_or(0);
    assert_eq!(dirs, 0, "no directory");
}

// Edge.

#[test]
fn a_root_blob_that_does_not_read_is_still_refused() {
    let w = World::new();
    w.write_root(&[9, 9, 9]);
    let result = w.open_with_file();
    assert!(
        matches!(
            result,
            Err(RegistryError::RootWrapped(None, "registry.key"))
        ),
        "{:?}",
        result.err()
    );
    assert_eq!(
        result.err().expect("refused").to_string(),
        "the registry's root key is wrapped by a key backend this build does not know \
         (registry.key), not by the master key file",
        "the text"
    );
}

#[test]
fn an_empty_registry_key_file_is_refused() {
    let w = World::new();
    w.write_root(&[]);
    assert!(
        matches!(
            w.open_with_file(),
            Err(RegistryError::RootWrapped(None, "registry.key"))
        ),
        "an empty file is not taken for no file"
    );
}

#[test]
fn a_registry_key_that_cannot_be_read_names_the_step_and_file() {
    let w = World::new();
    std::fs::remove_file(w.data.path().join(ROOT_FILE)).expect("remove");
    std::fs::create_dir(w.data.path().join(ROOT_FILE)).expect("a directory in its place");
    let result = w.open_with_file();
    match result {
        Err(RegistryError::Failed { step, path, .. }) => {
            assert_eq!(step, OpenStep::RootKey, "reading the root");
            assert_eq!(path, w.data.path().join(ROOT_FILE), "registry.key");
        }
        other => panic!("expected a failed read, got {:?}", other.err()),
    }
}

#[test]
fn without_registry_key_the_master_key_file_opens_as_before() {
    let w = World::new();
    std::fs::remove_file(w.data.path().join(ROOT_FILE)).expect("remove");
    let tenants = w.open_with_file().expect("opens under the master key file");
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    assert_eq!(w.row(&tenants, id).kek, None, "under the master key file");
}

#[test]
fn a_backend_that_fails_once_at_open_leaves_the_tenant_closed_until_it_succeeds() {
    let w = World::new();
    let tenants = w.open();
    let id = tenants.create(Actor::Cli, "acme", None).expect("made");
    store_one(&tenants, id);
    drop(tenants);
    let tenants = w.open();
    w.backend.fail_unwraps(&[Some(Fail::Memory)]);
    assert!(tenants.store(id).is_err(), "the memory error refuses");
    assert_eq!(read_one(&tenants, id), vcon(UUID, "kept").1, "then reads");
}
