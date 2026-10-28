//! A copy of the whole data directory, made while a server may be running:
//! server.sqlite with SQLite's online backup API, then each tenant it lists,
//! as `Store::backup_to` copies one.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use super::{TenantError, Tenants};
use crate::audit::{Action, Actor, Entry};
use crate::ids::TenantId;
use crate::store::{self, TRIES};

/// What `backup` copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backup {
    pub tenants: u64,
    pub versions: u64,
    /// How many times server.sqlite was copied.
    pub tries: usize,
}

/// Why a backup was refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupError {
    /// Something is already at the backup's path.
    Exists,
    /// Tenants were removed, or versions purged, after each try's copy.
    KeptChanging,
    /// A file could not be read, written or synced, or a catalog could not
    /// be copied or read.
    Io,
    /// The admin log could not record the backup.
    AuditFailed,
}

impl Tenants {
    /// Copies the data directory into `to`, which must not exist, and
    /// records the run in the admin log with its outcome.
    ///
    /// Each try copies server.sqlite in one step inside one read
    /// transaction, then each tenant that copy lists, as `Store::backup_to`
    /// copies one. A tenant the copy lists that is gone, or whose versions
    /// kept being purged, was removed meanwhile, and the next try copies
    /// server.sqlite again. Tenant directories the final copy does not
    /// list are removed. Tenants are copied one after another, each as it
    /// was at its own copy.
    pub fn backup(&self, who: Actor, to: &Path) -> Result<Backup, BackupError> {
        let done = self.copy_all(to);
        let status = match done {
            Ok(_) => 200,
            Err(BackupError::Exists) => 409,
            Err(BackupError::KeptChanging) => 503,
            Err(BackupError::Io | BackupError::AuditFailed) => 500,
        };
        self.record_alone(&Entry {
            actor: who,
            action: Action::Backup,
            target: None,
            version: None,
            status,
        })
        .map_err(|_| BackupError::AuditFailed)?;
        done
    }

    fn copy_all(&self, to: &Path) -> Result<Backup, BackupError> {
        let fail = |_| BackupError::Io;
        match self.fs.list(to) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => return Err(BackupError::Exists),
            Err(e) if e.kind() == io::ErrorKind::NotADirectory => return Err(BackupError::Exists),
            Err(_) => return Err(BackupError::Io),
        }
        let tenants = to.join("tenants");
        self.fs.create_dir_all(&tenants).map_err(fail)?;
        // Only the service account may enter the copy, as with the data
        // directory, which is restored by copying this one with its modes.
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(to, std::fs::Permissions::from_mode(crate::fs::DIR_MODE))
                .map_err(fail)?;
        }
        let registry = to.join("server.sqlite");
        'tries: for tries in 1..=TRIES {
            for file in ["server.sqlite", "server.sqlite-wal", "server.sqlite-shm"] {
                remove_if_there(&to.join(file)).map_err(fail)?;
            }
            let listed = self
                .registry
                .backup_into(&registry)
                .map_err(|_| BackupError::Io)?;
            #[cfg(test)]
            if let Some(hook) = self.hooks.registry_copied.lock().expect("hook").as_mut() {
                hook(self);
            }
            let mut versions = 0;
            for id in &listed {
                let copied = match self.store(*id) {
                    Ok(store) => store.backup_to(&tenants.join(id.to_string())),
                    Err(TenantError::NotFound | TenantError::Closing) => continue 'tries,
                    Err(_) => return Err(BackupError::Io),
                };
                match copied {
                    Ok(copied) => versions += copied.versions,
                    // A tenant removed while it was copied: its files went
                    // with it, so no try of its own could finish.
                    Err(store::BackupError::KeptChanging) if !self.listed(*id)? => {
                        self.stores().remove(id);
                        continue 'tries;
                    }
                    Err(store::BackupError::KeptChanging) => return Err(BackupError::KeptChanging),
                    Err(store::BackupError::Io) => return Err(BackupError::Io),
                }
            }
            let names: BTreeSet<String> = listed.iter().map(ToString::to_string).collect();
            for name in self.fs.list(&tenants).map_err(fail)? {
                let name = name.into_string().unwrap_or_default();
                if !names.contains(&name) {
                    self.fs.remove_dir_all(&tenants.join(&name)).map_err(fail)?;
                }
            }
            self.fs.sync_dir(&tenants).map_err(fail)?;
            self.fs.sync_dir(to).map_err(fail)?;
            if let Some(parent) = to.parent() {
                self.fs.sync_dir(parent).map_err(fail)?;
            }
            return Ok(Backup {
                tenants: listed.len() as u64,
                versions,
                tries,
            });
        }
        Err(BackupError::KeptChanging)
    }

    /// Whether server.sqlite, as it is now, lists the tenant.
    fn listed(&self, id: TenantId) -> Result<bool, BackupError> {
        let mut tx = self.registry.snapshot().map_err(|_| BackupError::Io)?;
        Ok(tx.tenant(id).map_err(|_| BackupError::Io)?.is_some())
    }
}

fn remove_if_there(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::clock::Clock;
    use crate::fs::Fs;
    use crate::ids::TenantId;
    use crate::store::Caller;
    use crate::store::testing::vcon;
    use crate::tenants::Removal;
    use crate::tenants::tests::{Fixture, fixture};

    /// Two tenants, each holding a vCon.
    fn two(f: &Fixture) -> [TenantId; 2] {
        let caller = Caller {
            key: crate::envelope::Key::generate().expect("key").id(),
        };
        let ids = ["acme", "globex"]
            .map(|name| f.tenants.create(Actor::Cli, name, None).expect("creates"));
        for (n, id) in ids.iter().enumerate() {
            let (found, body) = vcon(&format!("01a07da8-c2bb-83e5-b9a2-27900000000{n}"), "held");
            f.tenants
                .store(*id)
                .expect("store")
                .create(&caller, &found, &body)
                .expect("create");
        }
        ids
    }

    /// The backup at `to` opened as a registry under the same master key.
    fn opened(f: &Fixture, to: &Path) -> Tenants {
        Tenants::open_beside(to, f.master_key(), f.fs.clone(), f.clock.clone())
            .expect("the backup opens under the master key")
    }

    fn assert_whole(backup: &Tenants, tenants: usize) {
        let now = crate::clock::SystemClock.now();
        let found = backup
            .examine(crate::retention::Retention::Years(7), now)
            .expect("examines");
        assert_eq!(found.tenants.len(), tenants);
        for tenant in &found.tenants {
            let store = tenant
                .store
                .as_ref()
                .expect("each catalog opens under its key");
            assert!(store.damaged.is_empty(), "{:?}", store.damaged);
            assert!(store.leftovers.is_empty(), "{:?}", store.leftovers);
            assert_eq!(store.versions, 1);
        }
    }

    fn records(f: &Fixture, action: &str) -> Vec<serde_json::Value> {
        f.tenants
            .admin_records()
            .into_iter()
            .filter(|r| r["action"] == action)
            .collect()
    }

    #[test]
    fn a_backup_opens_whole_under_the_master_key_and_is_recorded() {
        let f = fixture();
        two(&f);
        let to = f.dir.path().join("backup");
        let done = f.tenants.backup(Actor::Cli, &to).expect("backs up");
        assert_eq!(
            done,
            Backup {
                tenants: 2,
                versions: 2,
                tries: 1
            }
        );
        let backup = opened(&f, &to);
        assert_whole(&backup, 2);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&to)
            .expect("the backup")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o700,
            "only the service account may enter the backup: {mode:o}"
        );
        let names: Vec<String> = backup
            .list(Actor::Cli)
            .expect("lists")
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["acme", "globex"]);
        let recorded = records(&f, "backup");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["status"], 200);
    }

    #[test]
    fn a_backup_survives_a_crash_once_it_returns() {
        let f = fixture();
        two(&f);
        let to = f.dir.path().join("backup");
        f.tenants.backup(Actor::Cli, &to).expect("backs up");
        f.fs.crash();
        assert_eq!(
            f.fs.list(&to.join("tenants"))
                .expect("the backup is there")
                .len(),
            2
        );
        assert_whole(&opened(&f, &to), 2);
    }

    #[test]
    fn a_path_already_there_is_refused_and_recorded() {
        let f = fixture();
        two(&f);
        let to = f.dir.path().join("backup");
        f.fs.create_dir_all(&to).expect("dir");
        assert_eq!(f.tenants.backup(Actor::Cli, &to), Err(BackupError::Exists));
        assert!(
            f.fs.list(&to).expect("list").is_empty(),
            "nothing is written"
        );
        assert_eq!(records(&f, "backup")[0]["status"], 409);
    }

    #[test]
    fn a_tenant_removed_after_the_registry_is_copied_starts_the_backup_again() {
        let f = fixture();
        let [acme, _] = two(&f);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        *f.tenants.hooks.registry_copied.lock().expect("hook") =
            Some(Box::new(move |tenants: &Tenants| {
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    tenants
                        .remove(Actor::Cli, acme, Removal::Force)
                        .expect("removes");
                }
            }));
        let to = f.dir.path().join("backup");
        let done = f.tenants.backup(Actor::Cli, &to).expect("backs up");
        assert_eq!(
            done,
            Backup {
                tenants: 1,
                versions: 1,
                tries: 2
            }
        );
        assert_whole(&opened(&f, &to), 1);
        let dirs = f.fs.list(&to.join("tenants")).expect("list");
        assert_eq!(
            dirs.len(),
            1,
            "no directory is left for the removed tenant: {dirs:?}"
        );
    }

    #[test]
    fn a_tenant_removed_while_its_data_is_copied_starts_the_backup_again() {
        let f = fixture();
        let [acme, _] = two(&f);
        // Another registry on the same directory, as the server would be,
        // removes the tenant while this one copies its files: after the
        // tenant's catalog is copied, before its object files are.
        let server =
            Tenants::open_beside(f.dir.path(), f.master_key(), f.fs.clone(), f.clock.clone())
                .expect("a second registry");
        let once = std::sync::Mutex::new(Some(server));
        f.tenants
            .store(acme)
            .expect("store")
            .on_backup_copied(Box::new(move |_: &crate::store::Store| {
                if let Some(server) = once.lock().expect("once").take() {
                    server
                        .remove(Actor::Cli, acme, Removal::Force)
                        .expect("removes");
                }
            }));
        let to = f.dir.path().join("backup");
        let done = f.tenants.backup(Actor::Cli, &to).expect("backs up");
        assert_eq!(done.tenants, 1);
        assert_eq!(done.tries, 2);
        assert_whole(&opened(&f, &to), 1);
        let dirs = f.fs.list(&to.join("tenants")).expect("list");
        assert_eq!(
            dirs.len(),
            1,
            "the removed tenant's partial copy is gone: {dirs:?}"
        );
    }

    #[test]
    fn a_registry_that_keeps_changing_fails_after_its_tries_and_is_recorded() {
        let f = fixture();
        let [acme, _] = two(&f);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let listed = std::sync::Mutex::new(acme);
        *f.tenants.hooks.registry_copied.lock().expect("hook") =
            Some(Box::new(move |tenants: &Tenants| {
                // Each try: a tenant the copy lists is removed before its
                // data is copied, and a new one is made, which the next
                // copy lists.
                let n = seen.fetch_add(1, Ordering::SeqCst);
                let mut listed = listed.lock().expect("listed");
                tenants
                    .remove(Actor::Cli, *listed, Removal::Force)
                    .expect("removes");
                *listed = tenants
                    .create(Actor::Cli, &format!("brief {n}"), None)
                    .expect("creates");
            }));
        let to = f.dir.path().join("backup");
        assert_eq!(
            f.tenants.backup(Actor::Cli, &to),
            Err(BackupError::KeptChanging)
        );
        assert_eq!(calls.load(Ordering::SeqCst), crate::store::TRIES);
        assert_eq!(records(&f, "backup")[0]["status"], 503);
    }
}
