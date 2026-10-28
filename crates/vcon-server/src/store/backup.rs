//! One tenant's Store copied into a backup while a server may be writing
//! to it: the catalog with SQLite's online backup API, then the object file
//! of every version row that copy holds.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use super::Store;
use crate::backend::ObjectError;
use crate::ids::VersionId;

/// How many times the catalog is copied while versions in each copy are
/// purged before their object files are copied.
pub const TRIES: usize = 5;

/// What `backup_to` copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Copied {
    /// Versions kept, hidden ones included.
    pub versions: u64,
    /// How many times the catalog was copied.
    pub tries: usize,
}

/// Why a backup of a Store failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupError {
    /// A version in each of `TRIES` copies of the catalog was purged before
    /// its object file could be copied.
    KeptChanging,
    /// A file could not be read, written or synced, or the catalog could
    /// not be copied.
    Io,
}

impl Store {
    /// Copies this tenant's catalog and object files into `dir`, made if
    /// new. Anything already in `dir` from an earlier try is replaced or
    /// removed.
    ///
    /// Each try copies the catalog first, in one step inside one read
    /// transaction, then the object file of each version row in that copy.
    /// An object file is written once and never changed, so a file copied
    /// at any try is still right; a row whose file is gone was purged after
    /// the copy, and the next try copies the catalog again. Files the final
    /// copy has no row for are removed. Every file and directory written is
    /// synced, so the backup is whole once this returns.
    pub fn backup_to(&self, dir: &Path) -> Result<Copied, BackupError> {
        let fail = |_| BackupError::Io;
        let objects = dir.join("objects");
        self.fs.create_dir_all(&objects).map_err(fail)?;
        let catalog = dir.join("catalog.sqlite");
        for tries in 1..=TRIES {
            for file in ["catalog.sqlite", "catalog.sqlite-wal", "catalog.sqlite-shm"] {
                remove_if_there(&dir.join(file)).map_err(fail)?;
            }
            let versions: BTreeSet<String> = self
                .catalog
                .backup_into(&catalog)
                .map_err(|_| BackupError::Io)?
                .into_iter()
                .map(|v| v.to_string())
                .collect();
            #[cfg(test)]
            if let Some(hook) = self.hooks.copied.lock().expect("hook").as_mut() {
                hook(self);
            }
            let present: BTreeSet<String> = self
                .fs
                .list(&objects)
                .map_err(fail)?
                .into_iter()
                .filter_map(|n| n.into_string().ok())
                .collect();
            let mut missing = false;
            for name in &versions {
                if present.contains(name) {
                    continue;
                }
                let Some(version) = VersionId::parse(name) else {
                    return Err(BackupError::Io);
                };
                match self.objects.get(version) {
                    Ok(bytes) => {
                        let tmp = objects.join(format!("{name}.tmp"));
                        if present.contains(&format!("{name}.tmp")) {
                            self.fs.remove(&tmp).map_err(fail)?;
                        }
                        self.fs.write_new(&tmp, &bytes).map_err(fail)?;
                        self.fs.sync_file(&tmp).map_err(fail)?;
                        self.fs.rename(&tmp, &objects.join(name)).map_err(fail)?;
                    }
                    Err(ObjectError::Missing) => missing = true,
                    Err(ObjectError::Io(e)) => return Err(fail(e)),
                }
            }
            if missing {
                continue;
            }
            for name in self.fs.list(&objects).map_err(fail)? {
                let name = name.into_string().unwrap_or_default();
                if !versions.contains(&name) {
                    self.fs.remove(&objects.join(&name)).map_err(fail)?;
                }
            }
            self.fs.sync_dir(&objects).map_err(fail)?;
            self.fs.sync_dir(dir).map_err(fail)?;
            // A new directory survives a crash once the one holding it is
            // synced too.
            if let Some(parent) = dir.parent() {
                self.fs.sync_dir(parent).map_err(fail)?;
            }
            return Ok(Copied {
                versions: versions.len() as u64,
                tries,
            });
        }
        Err(BackupError::KeptChanging)
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::clock::Clock;
    use crate::fs::Fs;
    use crate::retention::Retention;
    use crate::store::testing::vcon;
    use crate::store::tests::{Fixture, fixture};

    const A: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
    const B: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc47";

    /// Stores A (then replaces it, so one version is hidden) and B.
    fn stored(f: &Fixture) -> VersionId {
        let (found, body) = vcon(A, "first");
        let first = match f.store.create(&f.caller, &found, &body).expect("create") {
            crate::store::Created::New(v) => v,
            other => panic!("{other:?}"),
        };
        let (found, body) = vcon(A, "second");
        f.store
            .replace(&f.caller, first, &found, &body)
            .expect("replace");
        let (found, body) = vcon(B, "other");
        f.store.create(&f.caller, &found, &body).expect("create");
        first
    }

    /// The backup in `dir` opened as a Store, and examined.
    fn examined(f: &Fixture, dir: &Path) -> (Store, crate::store::Examination) {
        let store = Store::open_beside(
            dir,
            f.tenant(),
            f.tenant_key(),
            f.fs.clone(),
            f.clock.clone(),
        )
        .expect("the backup opens under the tenant key");
        let now = f.clock.now();
        let found = store
            .examine(Retention::Years(7), now, now)
            .expect("examines");
        (store, found)
    }

    fn body_of(store: &Store, f: &Fixture, uuid: &str) -> Vec<u8> {
        store
            .get(&f.caller, uuid.parse().expect("uuid"))
            .expect("reads")
            .body
    }

    #[test]
    fn a_backup_opens_whole_with_every_version() {
        let f = fixture();
        stored(&f);
        let dir = f.dir.path().join("backup");
        let copied = f.store.backup_to(&dir).expect("backs up");
        assert_eq!(
            copied,
            Copied {
                versions: 3,
                tries: 1
            }
        );
        let (store, found) = examined(&f, &dir);
        assert_eq!(found.versions, 3);
        assert!(found.damaged.is_empty(), "{:?}", found.damaged);
        assert!(found.leftovers.is_empty(), "{:?}", found.leftovers);
        assert_eq!(body_of(&store, &f, A), body_of(&f.store, &f, A));
        assert_eq!(body_of(&store, &f, B), body_of(&f.store, &f, B));
    }

    #[test]
    fn a_version_purged_after_the_catalog_is_copied_makes_another_try() {
        let f = fixture();
        let first = stored(&f);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        *f.store.hooks.copied.lock().expect("hook") = Some(Box::new(move |store: &Store| {
            // Once: the hidden version is purged, row and file, after the
            // catalog copy holds its row.
            if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                let purged = store.purge_due(Retention::Days(0)).expect("purges");
                assert_eq!(purged.purged, 1);
            }
        }));
        let dir = f.dir.path().join("backup");
        let copied = f.store.backup_to(&dir).expect("backs up");
        assert_eq!(
            copied,
            Copied {
                versions: 2,
                tries: 2
            }
        );
        let (_, found) = examined(&f, &dir);
        assert_eq!(found.versions, 2);
        assert!(found.damaged.is_empty(), "{:?}", found.damaged);
        assert!(found.leftovers.is_empty(), "{:?}", found.leftovers);
        assert!(
            !dir.join("objects").join(first.to_string()).exists()
                && f.fs
                    .read(&dir.join("objects").join(first.to_string()))
                    .is_err(),
            "the purged version's file is not in the backup"
        );
    }

    #[test]
    fn a_store_whose_versions_keep_being_purged_fails_after_its_tries() {
        let f = fixture();
        stored(&f);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let current = Arc::new(Mutex::new(None::<VersionId>));
        *f.store.hooks.copied.lock().expect("hook") = Some(Box::new(move |store: &Store| {
            // Each try: B's current version is replaced and purged at
            // once, so the copy's row for it never has its file.
            let n = seen.fetch_add(1, Ordering::SeqCst);
            let mut current = current.lock().expect("current");
            let caller = crate::store::Caller {
                key: crate::envelope::Key::generate().expect("key").id(),
            };
            let now = current.unwrap_or_else(|| {
                store
                    .get(&caller, B.parse().expect("uuid"))
                    .expect("B")
                    .version
            });
            let (found, body) = vcon(B, &format!("change {n}"));
            *current = Some(store.replace(&caller, now, &found, &body).expect("replace"));
            store.purge_due(Retention::Days(0)).expect("purges");
        }));
        let dir = f.dir.path().join("backup");
        assert_eq!(f.store.backup_to(&dir), Err(BackupError::KeptChanging));
        assert_eq!(calls.load(Ordering::SeqCst), TRIES);
    }

    #[test]
    fn files_an_earlier_try_left_are_replaced_or_removed() {
        let f = fixture();
        stored(&f);
        let dir = f.dir.path().join("backup");
        let objects = dir.join("objects");
        f.fs.create_dir_all(&objects).expect("dir");
        let stray = VersionId::generate().to_string();
        f.fs.write_new(&objects.join(&stray), b"from an earlier try")
            .expect("write");
        f.fs.write_new(&objects.join(format!("{stray}.tmp")), b"half")
            .expect("write");
        std::fs::write(dir.join("catalog.sqlite"), b"an earlier copy").expect("write");
        std::fs::write(dir.join("catalog.sqlite-wal"), b"its wal").expect("write");
        f.store.backup_to(&dir).expect("backs up");
        let (_, found) = examined(&f, &dir);
        assert_eq!(found.versions, 3);
        assert!(found.damaged.is_empty(), "{:?}", found.damaged);
        assert!(found.leftovers.is_empty(), "{:?}", found.leftovers);
        let names = f.fs.list(&objects).expect("list");
        assert_eq!(names.len(), 3, "{names:?}");
    }

    #[test]
    fn a_backup_survives_a_crash_once_it_returns() {
        let f = fixture();
        stored(&f);
        let dir = f.dir.path().join("backup");
        f.store.backup_to(&dir).expect("backs up");
        f.fs.crash();
        let (_, found) = examined(&f, &dir);
        assert_eq!(found.versions, 3);
        assert!(found.damaged.is_empty(), "{:?}", found.damaged);
    }

    #[test]
    fn a_backup_taken_while_writes_go_on_opens_whole() {
        let f = Arc::new(fixture());
        stored(&f);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // The writer says when its first create is done, so the backup
        // starts with writes under way whatever the scheduler does.
        let (first, written_once) = std::sync::mpsc::channel();
        let writer = {
            let (f, stop) = (f.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut n = 0u32;
                while n == 0 || !stop.load(Ordering::SeqCst) {
                    let uuid = format!("01a07da8-c2bb-83e5-b9a2-{n:012x}");
                    let (found, body) = vcon(&uuid, "load");
                    f.store.create(&f.caller, &found, &body).expect("create");
                    if n == 0 {
                        let _ = first.send(());
                    }
                    n += 1;
                }
                n
            })
        };
        written_once
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("the writer stores its first vCon");
        let dir = f.dir.path().join("backup");
        let copied = f.store.backup_to(&dir).expect("backs up");
        stop.store(true, Ordering::SeqCst);
        let written = writer.join().expect("writer");
        assert!(written > 0, "writes went on");
        let (_, found) = examined(&f, &dir);
        assert_eq!(
            found.versions, copied.versions,
            "the copy holds the versions the backup says it copied"
        );
        assert!(
            found.damaged.is_empty(),
            "every copied version opens: {:?}",
            found.damaged
        );
        assert!(
            found.leftovers.is_empty(),
            "the copy holds no file its catalog does not name: {:?}",
            found.leftovers
        );
    }
}
