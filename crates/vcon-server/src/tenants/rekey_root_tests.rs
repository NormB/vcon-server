//! A move of the registry root from one key-wrapping backend to another:
//! the master key file to a key backend, back, and from one key backend to
//! another. Every tenant key, a pending one too, moves with the root, in a
//! copy of server.sqlite that replaces it only once complete, and the root
//! file follows; a run stopped at any step is finished by running it
//! again, and one run after completion changes nothing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use time::macros::datetime;
use uuid::Uuid;

use super::rekey::{MasterStep, Moved, RekeyStep, rekey_root, rekey_root_stopping};
use super::{ROOT_FILE, RegistryError, TenantError, Tenants};
use crate::audit::Actor;
use crate::backend::{Catalogs, ObjectPlace};
use crate::clock::ManualClock;
use crate::envelope::Key;
use crate::fs::OsFs;
use crate::ids::TenantId;
use crate::key_wrap::blob::BackendKind;
use crate::key_wrap::testing::{Fail, Scripted};
use crate::key_wrap::{BackendWrap, FileWrap, KeyWrap};
use crate::store::Caller;
use crate::store::testing::vcon;

const NOW: time::OffsetDateTime = datetime!(2026-11-26 12:00 UTC);

/// The next root file a move writes, before it replaces registry.key.
const NEXT: &str = "registry.key.next";

struct Dir {
    dir: tempfile::TempDir,
}

impl Dir {
    fn new() -> Dir {
        let d = Dir {
            dir: tempfile::tempdir().expect("temp dir"),
        };
        for (name, byte) in [("old", 0x41), ("new", 0x42), ("other", 0x43)] {
            let path = d.key_file(name);
            std::fs::write(&path, [byte; 32]).expect("a key file");
            std::fs::set_permissions(
                &path,
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
            )
            .expect("chmod 0600");
        }
        std::fs::create_dir(d.data()).expect("the data directory");
        d
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn key_file(&self, name: &str) -> PathBuf {
        self.dir.path().join(format!("{name}.key"))
    }

    fn key(&self, name: &str) -> Key {
        Key::load_master(&self.key_file(name)).expect("a key")
    }

    fn file(&self, name: &str) -> FileWrap {
        FileWrap::new(self.key(name))
    }

    fn open_wrap(&self, wrap: Box<dyn KeyWrap>) -> Tenants {
        let tenants = Tenants::opened(
            &self.data(),
            wrap,
            Arc::new(OsFs),
            Arc::new(ManualClock::at(NOW)),
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

    /// The registry under the master key file `name`, as a start opens it.
    fn open_file(&self, name: &str) -> Result<Tenants, RegistryError> {
        Tenants::open(
            &self.data(),
            self.key(name),
            Arc::new(OsFs),
            Arc::new(ManualClock::at(NOW)),
        )
    }

    /// The backend over the root registry.key holds.
    fn backend_wrap(&self, backend: &Scripted) -> BackendWrap<Scripted> {
        BackendWrap::open(backend.clone(), &self.root().expect("registry.key"))
            .expect("the root opens")
    }

    fn root(&self) -> Option<Vec<u8>> {
        read_if_there(&self.data().join(ROOT_FILE))
    }

    fn next(&self) -> Option<Vec<u8>> {
        read_if_there(&self.data().join(NEXT))
    }

    fn server_file(&self) -> Vec<u8> {
        std::fs::read(self.data().join("server.sqlite")).expect("server.sqlite")
    }
}

fn read_if_there(path: &Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => panic!("{}: {e}", path.display()),
    }
}

const CALLER: Caller = Caller {
    key: crate::ids::KeyId([5; 8]),
};

fn backend(key: &str) -> Scripted {
    Scripted::new(BackendKind::Pkcs11, key)
}

/// Two tenants with one vCon each, and the second's key rotation stopped
/// with its new key pending, in the registry `tenants`.
fn fill(tenants: &Tenants) -> (Vec<(TenantId, Uuid)>, TenantId) {
    let mut made = Vec::new();
    for (n, name) in ["acme", "beta"].into_iter().enumerate() {
        let id = tenants.create(Actor::Cli, name, None).expect("made");
        let (found, body) = vcon(&format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}"), name);
        tenants
            .store(id)
            .expect("store")
            .create(&CALLER, &found, &body)
            .expect("stored");
        made.push((id, found.uuid));
    }
    let pending = made[1].0;
    *tenants.hooks.rekey_stop.lock().expect("hook") = Some(RekeyStep::Reencrypted);
    assert_eq!(
        tenants.rekey_tenant(Actor::Cli, pending),
        Err(TenantError::Io),
        "the rotation stops with its key pending"
    );
    (made, pending)
}

/// Every vCon reads, and the stopped rotation finishes, in `tenants`.
fn readable(tenants: &Tenants, made: &[(TenantId, Uuid)], pending: TenantId) {
    tenants
        .rekey_tenant(Actor::Cli, pending)
        .expect("the pending key moved with the root");
    for (id, uuid) in made {
        let store = tenants.store(*id).expect("store");
        assert!(store.get(&CALLER, *uuid).is_ok(), "{id} reads");
    }
}

/// A data directory under the master key file "old", filled.
fn under_file() -> (Dir, Vec<(TenantId, Uuid)>, TenantId) {
    let d = Dir::new();
    let tenants = d.open_file("old").expect("opens");
    let (made, pending) = fill(&tenants);
    drop(tenants);
    (d, made, pending)
}

/// A data directory under `backend`, filled.
fn under_backend(backend: &Scripted) -> (Dir, Vec<(TenantId, Uuid)>, TenantId) {
    let d = Dir::new();
    let (wrap, root) = BackendWrap::create(backend.clone()).expect("a root");
    std::fs::write(d.data().join(ROOT_FILE), &root).expect("registry.key");
    let tenants = d.open_wrap(Box::new(wrap));
    let (made, pending) = fill(&tenants);
    drop(tenants);
    (d, made, pending)
}

/// Each tenant's two key-encryption key references, by tenant.
fn wrap_refs(tenants: &Tenants) -> Vec<(Option<String>, Option<String>)> {
    tenants
        .registry
        .snapshot()
        .expect("a snapshot")
        .tenants()
        .expect("rows")
        .into_iter()
        .map(|row| {
            (
                row.kek.map(|k| k.wrap_ref),
                row.pending_kek.map(|k| k.wrap_ref),
            )
        })
        .collect()
}

// Success.

#[test]
fn a_move_from_the_master_key_file_to_a_backend_moves_every_tenant() {
    let (d, made, pending) = under_file();
    let b = backend("vcon-1");
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    let moved = rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW).expect("moves");
    assert_eq!(moved, Moved::Tenants(2), "two tenants");
    assert_eq!(d.root(), Some(root), "registry.key holds the new root");
    assert_eq!(d.next(), None, "no next root is left");
    let tenants = d.open_wrap(Box::new(d.backend_wrap(&b)));
    assert_eq!(
        wrap_refs(&tenants)
            .iter()
            .filter(|(kek, _)| kek.as_deref() == Some("pkcs11:vcon-1"))
            .count(),
        2,
        "each tenant key is under the backend"
    );
    readable(&tenants, &made, pending);
}

#[test]
fn after_a_move_to_a_backend_the_master_key_file_opens_nothing() {
    let (d, _, _) = under_file();
    let (to, root) = BackendWrap::create(backend("vcon-1")).expect("a root");
    rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW).expect("moves");
    assert!(
        matches!(
            d.open_file("old"),
            Err(RegistryError::RootWrapped(Some(BackendKind::Pkcs11), _))
        ),
        "the old master key file is refused"
    );
}

#[test]
fn a_move_from_a_backend_to_a_master_key_file_moves_every_tenant() {
    let b = backend("vcon-1");
    let (d, made, pending) = under_backend(&b);
    let from = d.backend_wrap(&b);
    let moved = rekey_root(&d.data(), &from, &d.file("new"), None, NOW).expect("moves");
    assert_eq!(moved, Moved::Tenants(2), "two tenants");
    assert_eq!(d.root(), None, "registry.key is removed");
    let tenants = d
        .open_file("new")
        .expect("opens under the new master key file");
    assert_eq!(
        wrap_refs(&tenants),
        vec![(None, None), (None, None)],
        "no tenant names a key-encryption key"
    );
    readable(&tenants, &made, pending);
}

#[test]
fn a_move_from_one_backend_to_another_moves_every_tenant() {
    let a = backend("vcon-1");
    let (d, made, pending) = under_backend(&a);
    let b = backend("vcon-2");
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    rekey_root(&d.data(), &d.backend_wrap(&a), &to, Some(&root), NOW).expect("moves");
    let tenants = d.open_wrap(Box::new(d.backend_wrap(&b)));
    assert!(
        wrap_refs(&tenants)
            .iter()
            .all(|(kek, _)| kek.as_deref() == Some("pkcs11:vcon-2")),
        "every tenant is under the second backend"
    );
    readable(&tenants, &made, pending);
}

#[test]
fn a_pending_key_moves_under_the_backend_too() {
    let (d, _, pending) = under_file();
    let b = backend("vcon-1");
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW).expect("moves");
    let tenants = d.open_wrap(Box::new(d.backend_wrap(&b)));
    let row = tenants
        .registry
        .snapshot()
        .expect("a snapshot")
        .tenant(pending)
        .expect("reads")
        .expect("the row");
    assert_eq!(
        row.pending_kek.map(|k| k.wrap_ref),
        Some("pkcs11:vcon-1".to_owned()),
        "the pending key is under its own key-encryption key"
    );
}

#[test]
fn a_move_to_a_file_counts_each_wrap_against_the_new_key() {
    let b = backend("vcon-1");
    let (d, _, _) = under_backend(&b);
    rekey_root(&d.data(), &d.backend_wrap(&b), &d.file("new"), None, NOW).expect("moves");
    let tenants = d.open_file("new").expect("opens");
    let uses: Vec<(Vec<u8>, i64)> = tenants
        .conn_for_test()
        .prepare("SELECT master_key_id, wraps FROM master_key_use")
        .expect("prepare")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(
        uses,
        [(d.key("new").id().bytes().to_vec(), 3)],
        "two tenant keys and one pending one, under the new key only"
    );
}

#[test]
fn a_move_is_recorded_once_in_the_admin_log() {
    let (d, _, _) = under_file();
    let b = backend("vcon-1");
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW).expect("moves");
    let tenants = d.open_wrap(Box::new(d.backend_wrap(&b)));
    let records = tenants.admin_records();
    let moves: Vec<_> = records
        .iter()
        .filter(|r| r["action"] == "rekey_master")
        .collect();
    assert_eq!(moves.len(), 1, "one record");
    assert_eq!(moves[0]["status"], 200, "it succeeded");
}

#[test]
fn a_move_between_two_master_key_files_is_the_master_rotation() {
    let (d, made, pending) = under_file();
    let moved = rekey_root(&d.data(), &d.file("old"), &d.file("new"), None, NOW).expect("rotates");
    assert_eq!(moved, Moved::Tenants(2), "two tenants");
    assert_eq!(d.root(), None, "no root file");
    readable(&d.open_file("new").expect("opens"), &made, pending);
}

// Failure.

#[test]
fn a_backend_that_does_not_wrap_changes_nothing() {
    let (d, _, _) = under_file();
    let b = backend("vcon-1");
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    let before = d.server_file();
    b.fail_wraps(&[None, Some(Fail::Damaged)]);
    assert!(
        rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW).is_err(),
        "the second wrap fails"
    );
    assert_eq!(d.server_file(), before, "server.sqlite is untouched");
    assert_eq!(d.root(), None, "no registry.key");
    assert!(d.open_file("old").is_ok(), "still under the old key");
}

#[test]
fn a_backend_that_does_not_unwrap_changes_nothing() {
    let b = backend("vcon-1");
    let (d, _, _) = under_backend(&b);
    let from = d.backend_wrap(&b);
    let (root, before) = (d.root(), d.server_file());
    b.fail_unwraps(&[Some(Fail::Damaged)]);
    assert!(
        rekey_root(&d.data(), &from, &d.file("new"), None, NOW).is_err(),
        "the unwrap fails"
    );
    assert_eq!(d.server_file(), before, "server.sqlite is untouched");
    assert_eq!(d.root(), root, "registry.key is untouched");
}

#[test]
fn a_key_that_opens_neither_way_is_refused_and_changes_nothing() {
    let (d, _, _) = under_file();
    let before = d.server_file();
    assert_eq!(
        rekey_root(&d.data(), &d.file("other"), &d.file("new"), None, NOW),
        Err(TenantError::WrongKey),
        "under neither key"
    );
    assert_eq!(d.server_file(), before, "untouched");
}

#[test]
fn a_source_under_another_backend_key_is_refused() {
    let a = backend("vcon-1");
    let (d, _, _) = under_backend(&a);
    // The source names a root server.sqlite is not under.
    let other = backend("vcon-9");
    let (from, _) = BackendWrap::create(other).expect("a root");
    let before = d.server_file();
    assert!(
        rekey_root(&d.data(), &from, &d.file("new"), None, NOW).is_err(),
        "refused"
    );
    assert_eq!(d.server_file(), before, "untouched");
}

#[test]
fn a_memory_error_of_the_target_changes_nothing() {
    let b = backend("vcon-2");
    let a = backend("vcon-1");
    let (d, _, _) = under_backend(&a);
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    let (old_root, before) = (d.root(), d.server_file());
    b.fail_wraps(&[Some(Fail::Memory)]);
    assert!(
        rekey_root(&d.data(), &d.backend_wrap(&a), &to, Some(&root), NOW).is_err(),
        "refused"
    );
    assert_eq!(d.server_file(), before, "server.sqlite untouched");
    assert_eq!(d.root(), old_root, "registry.key untouched");
}

#[test]
fn a_tenant_key_too_damaged_to_unwrap_stops_the_move() {
    let (d, _, _) = under_file();
    let tenants = d.open_file("old").expect("opens");
    tenants
        .conn_for_test()
        .execute("UPDATE tenant SET wrapped_tenant_key = x'00'", [])
        .expect("damages the rows");
    drop(tenants);
    let before = d.server_file();
    let (to, root) = BackendWrap::create(backend("vcon-1")).expect("a root");
    assert!(
        rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW).is_err(),
        "refused"
    );
    assert_eq!(d.server_file(), before, "untouched");
    assert_eq!(d.root(), None, "no registry.key");
}

// Edge.

#[test]
fn stopped_before_the_rename_a_move_to_a_backend_leaves_the_old_registry() {
    for step in [MasterStep::Copied, MasterStep::Rewrapped] {
        let (d, made, pending) = under_file();
        let b = backend("vcon-1");
        let (to, root) = BackendWrap::create(b.clone()).expect("a root");
        let before = d.server_file();
        assert_eq!(
            rekey_root_stopping(&d.data(), &d.file("old"), &to, Some(&root), NOW, Some(step)),
            Err(TenantError::Io),
            "{step:?}"
        );
        assert_eq!(d.server_file(), before, "{step:?}: untouched");
        assert_eq!(d.root(), None, "{step:?}: no registry.key yet");
        assert!(
            d.open_file("old").is_ok(),
            "{step:?}: still under the old key"
        );
        rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW).expect("finishes");
        readable(&d.open_wrap(Box::new(d.backend_wrap(&b))), &made, pending);
    }
}

#[test]
fn stopped_after_the_rename_a_move_to_a_backend_is_finished_by_a_rerun() {
    let (d, made, pending) = under_file();
    let b = backend("vcon-1");
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    assert_eq!(
        rekey_root_stopping(
            &d.data(),
            &d.file("old"),
            &to,
            Some(&root),
            NOW,
            Some(MasterStep::Renamed)
        ),
        Err(TenantError::Io),
        "stopped"
    );
    assert_eq!(d.root(), None, "registry.key not yet in place");
    assert_eq!(d.next(), Some(root.clone()), "the next root is kept");
    // The run again takes its root from the next root file.
    let again = BackendWrap::open(b.clone(), &d.next().expect("next")).expect("opens");
    let after = d.server_file();
    assert_eq!(
        rekey_root(&d.data(), &d.file("old"), &again, Some(&root), NOW),
        Ok(Moved::Already),
        "only the root file was left"
    );
    assert_eq!(d.server_file(), after, "server.sqlite not rewritten");
    assert_eq!(d.root(), Some(root), "registry.key in place");
    assert_eq!(d.next(), None, "the next root file is gone");
    readable(&d.open_wrap(Box::new(d.backend_wrap(&b))), &made, pending);
}

#[test]
fn stopped_after_the_rename_a_move_to_a_file_is_finished_by_a_rerun() {
    let b = backend("vcon-1");
    let (d, made, pending) = under_backend(&b);
    assert_eq!(
        rekey_root_stopping(
            &d.data(),
            &d.backend_wrap(&b),
            &d.file("new"),
            None,
            NOW,
            Some(MasterStep::Renamed)
        ),
        Err(TenantError::Io),
        "stopped"
    );
    assert!(d.root().is_some(), "registry.key still there");
    assert_eq!(
        rekey_root(&d.data(), &d.backend_wrap(&b), &d.file("new"), None, NOW),
        Ok(Moved::Already),
        "only the root file was left"
    );
    assert_eq!(d.root(), None, "registry.key removed");
    readable(&d.open_file("new").expect("opens"), &made, pending);
}

#[test]
fn stopped_before_the_rename_a_move_to_a_file_leaves_the_old_registry() {
    for step in [MasterStep::Copied, MasterStep::Rewrapped] {
        let b = backend("vcon-1");
        let (d, made, pending) = under_backend(&b);
        let (root, before) = (d.root(), d.server_file());
        assert_eq!(
            rekey_root_stopping(
                &d.data(),
                &d.backend_wrap(&b),
                &d.file("new"),
                None,
                NOW,
                Some(step)
            ),
            Err(TenantError::Io),
            "{step:?}"
        );
        assert_eq!(d.server_file(), before, "{step:?}: untouched");
        assert_eq!(d.root(), root, "{step:?}: registry.key untouched");
        rekey_root(&d.data(), &d.backend_wrap(&b), &d.file("new"), None, NOW).expect("finishes");
        readable(&d.open_file("new").expect("opens"), &made, pending);
    }
}

#[test]
fn a_move_run_again_after_it_completed_changes_nothing_and_says_so() {
    let (d, _, _) = under_file();
    rekey_root(&d.data(), &d.file("old"), &d.file("new"), None, NOW).expect("rotates");
    let after = d.server_file();
    assert_eq!(
        rekey_root(&d.data(), &d.file("old"), &d.file("new"), None, NOW),
        Ok(Moved::Already),
        "nothing left"
    );
    assert_eq!(d.server_file(), after, "not rewritten");
}

#[test]
fn a_registry_with_no_tenants_moves_its_root() {
    let d = Dir::new();
    drop(d.open_file("old").expect("makes an empty registry"));
    let b = backend("vcon-1");
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    assert_eq!(
        rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW),
        Ok(Moved::Tenants(0)),
        "no tenant"
    );
    let tenants = d.open_wrap(Box::new(d.backend_wrap(&b)));
    assert_eq!(wrap_refs(&tenants), Vec::new(), "still none");
}

#[test]
fn a_next_root_left_by_a_stopped_run_is_replaced() {
    let (d, made, pending) = under_file();
    let b = backend("vcon-1");
    let (first, first_root) = BackendWrap::create(b.clone()).expect("a root");
    assert_eq!(
        rekey_root_stopping(
            &d.data(),
            &d.file("old"),
            &first,
            Some(&first_root),
            NOW,
            Some(MasterStep::Copied)
        ),
        Err(TenantError::Io),
        "stopped"
    );
    assert_eq!(d.next(), Some(first_root), "the first run's next root");
    let (second, second_root) = BackendWrap::create(b.clone()).expect("a root");
    rekey_root(&d.data(), &d.file("old"), &second, Some(&second_root), NOW).expect("moves");
    assert_eq!(d.root(), Some(second_root), "the second run's root");
    readable(&d.open_wrap(Box::new(d.backend_wrap(&b))), &made, pending);
}

#[test]
fn a_move_from_a_file_with_a_borrowed_key_is_the_same_move() {
    let (d, made, pending) = under_file();
    let (old, new) = (d.key("old"), d.key("new"));
    rekey_root(
        &d.data(),
        &FileWrap::borrowing(&old),
        &FileWrap::borrowing(&new),
        None,
        NOW,
    )
    .expect("rotates");
    readable(&d.open_file("new").expect("opens"), &made, pending);
}

#[test]
fn a_move_to_a_file_removes_a_next_root_a_stopped_move_left() {
    let (d, made, pending) = under_file();
    let (to, root) = BackendWrap::create(backend("vcon-1")).expect("a root");
    assert_eq!(
        rekey_root_stopping(
            &d.data(),
            &d.file("old"),
            &to,
            Some(&root),
            NOW,
            Some(MasterStep::Copied)
        ),
        Err(TenantError::Io),
        "stopped"
    );
    assert!(d.next().is_some(), "a next root is left");
    rekey_root(&d.data(), &d.file("old"), &d.file("new"), None, NOW).expect("rotates");
    assert_eq!(d.next(), None, "the next root is removed");
    readable(&d.open_file("new").expect("opens"), &made, pending);
}

#[test]
fn a_rerun_whose_next_root_went_missing_is_refused() {
    let (d, _, _) = under_file();
    let b = backend("vcon-1");
    let (to, root) = BackendWrap::create(b.clone()).expect("a root");
    assert_eq!(
        rekey_root_stopping(
            &d.data(),
            &d.file("old"),
            &to,
            Some(&root),
            NOW,
            Some(MasterStep::Renamed)
        ),
        Err(TenantError::Io),
        "stopped"
    );
    std::fs::remove_file(d.data().join(NEXT)).expect("the next root goes");
    assert_eq!(
        rekey_root(&d.data(), &d.file("old"), &to, Some(&root), NOW),
        Err(TenantError::Io),
        "no root to put in place"
    );
    assert_eq!(d.root(), None, "no registry.key is made up");
}
