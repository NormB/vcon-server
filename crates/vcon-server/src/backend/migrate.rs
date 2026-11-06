//! Moving a tenant's sealed objects from one storage to another, as
//! `vcon-server storage move-objects` does from the data directory to a
//! bucket. Each object is written to the new storage, read back and
//! compared byte for byte, and only then removed from the old one, so a run
//! cut off at any point loses nothing: the next run writes the rest again.
//! A name that is not a version id, a temporary one among them, is not
//! moved and stays where it is.

use super::{ObjectError, Objects};
use crate::ids::VersionId;

/// Why a move stopped.
#[derive(Debug)]
pub(crate) enum MoveError {
    /// The old storage could not be read, listed or written.
    From(ObjectError),
    /// The new storage could not be written or read.
    To(ObjectError),
    /// The copy of the version read back from the new storage is not the
    /// object; the object stays in the old storage.
    Differs(VersionId),
}

/// Moves every version object of `from` to `to`, as the module says, and
/// answers how many moved.
pub(crate) fn move_objects(from: &dyn Objects, to: &dyn Objects) -> Result<u64, MoveError> {
    let mut moved = 0;
    for name in from.list().map_err(MoveError::From)? {
        // A temporary name, and any other that is not a version id, is
        // not a stored version.
        let Some(version) = VersionId::parse(&name) else {
            continue;
        };
        let bytes = from.get(version).map_err(MoveError::From)?;
        to.put(version, &bytes).map_err(MoveError::To)?;
        if to.get(version).map_err(MoveError::To)? != bytes {
            return Err(MoveError::Differs(version));
        }
        from.remove_name(&name).map_err(MoveError::From)?;
        moved += 1;
    }
    if moved > 0 {
        from.sync().map_err(MoveError::From)?;
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::backend::local::LocalObjects;
    use crate::fs::OsFs;

    /// Objects in a new directory, and the directory.
    fn objects() -> (tempfile::TempDir, LocalObjects) {
        let dir = tempfile::tempdir().expect("temp dir");
        let objects = LocalObjects::new(dir.path().to_owned(), Arc::new(OsFs));
        (dir, objects)
    }

    /// Objects that write each object with its last byte changed after the
    /// first `good` writes, and fail every write after `fail_after`.
    struct Faulty {
        inner: LocalObjects,
        good: usize,
        fail_after: usize,
        writes: AtomicUsize,
    }

    impl Objects for Faulty {
        fn put(&self, version: VersionId, blob: &[u8]) -> Result<(), ObjectError> {
            let n = self.writes.fetch_add(1, Ordering::SeqCst);
            if n >= self.fail_after {
                return Err(ObjectError::Io(std::io::Error::other("the storage failed")));
            }
            let mut bytes = blob.to_vec();
            if n >= self.good {
                *bytes.last_mut().expect("a byte") ^= 1;
            }
            self.inner.put(version, &bytes)
        }
        fn get(&self, version: VersionId) -> Result<Vec<u8>, ObjectError> {
            self.inner.get(version)
        }
        fn remove(&self, version: VersionId) -> Result<(), ObjectError> {
            self.inner.remove(version)
        }
        fn list(&self) -> Result<Vec<String>, ObjectError> {
            self.inner.list()
        }
        fn remove_name(&self, name: &str) -> Result<(), ObjectError> {
            self.inner.remove_name(name)
        }
        fn sync(&self) -> Result<(), ObjectError> {
            self.inner.sync()
        }
    }

    /// Three version objects, a temporary one and a name not ours, in
    /// `dir`; the versions and their bytes.
    fn planted(dir: &std::path::Path) -> Vec<(VersionId, Vec<u8>)> {
        let kept: Vec<(VersionId, Vec<u8>)> = (0..3u8)
            .map(|n| (VersionId::generate(), vec![0x1f, 0x8b, n, 0xff, n]))
            .collect();
        for (version, bytes) in &kept {
            std::fs::write(dir.join(version.to_string()), bytes).expect("written");
        }
        std::fs::write(dir.join(format!("{}.tmp", VersionId::generate())), b"half")
            .expect("written");
        std::fs::write(dir.join("not-ours"), b"someone else's").expect("written");
        kept
    }

    /// Every version object moves, byte for byte; the temporary name and the
    /// one not ours stay, and a second run moves nothing.
    #[test]
    fn every_version_object_moves_byte_for_byte_and_nothing_else() {
        let (from_dir, from) = objects();
        let (_to_dir, to) = objects();
        let kept = planted(from_dir.path());
        assert_eq!(
            move_objects(&from, &to).expect("moved"),
            3,
            "three versions"
        );
        for (version, bytes) in &kept {
            assert_eq!(&to.get(*version).expect("moved"), bytes, "byte for byte");
            assert!(
                matches!(from.get(*version), Err(ObjectError::Missing)),
                "gone from the old storage"
            );
        }
        let mut left = from.list().expect("listed");
        left.retain(|n| VersionId::parse(n).is_none());
        assert_eq!(
            left.len(),
            2,
            "the temporary name and the one not ours: {left:?}"
        );
        assert_eq!(
            move_objects(&from, &to).expect("moved"),
            0,
            "nothing left to move"
        );
    }

    /// A copy that does not read back as the object stops the move with the
    /// version's id, and the object stays in the old storage.
    #[test]
    fn a_copy_that_reads_back_otherwise_keeps_the_object() {
        let (from_dir, from) = objects();
        let (_to_dir, to) = objects();
        let kept = planted(from_dir.path());
        let faulty = Faulty {
            inner: to,
            good: 1,
            fail_after: usize::MAX,
            writes: AtomicUsize::new(0),
        };
        let stopped = move_objects(&from, &faulty);
        let Err(MoveError::Differs(version)) = stopped else {
            panic!("the move stops at the copy that differs: {stopped:?}")
        };
        let bytes = &kept.iter().find(|(v, _)| *v == version).expect("ours").1;
        assert_eq!(
            &from.get(version).expect("kept"),
            bytes,
            "kept, byte for byte"
        );
        let moved = kept
            .iter()
            .filter(|(v, _)| matches!(from.get(*v), Err(ObjectError::Missing)))
            .count();
        assert_eq!(moved, 1, "only the one that read back the same moved");
    }

    /// A move cut off by the new storage failing loses nothing; the next
    /// run moves the rest.
    #[test]
    fn a_move_cut_off_loses_nothing_and_the_next_run_finishes() {
        let (from_dir, from) = objects();
        let (to_dir, to) = objects();
        let kept = planted(from_dir.path());
        let faulty = Faulty {
            inner: to,
            good: usize::MAX,
            fail_after: 2,
            writes: AtomicUsize::new(0),
        };
        assert!(
            matches!(move_objects(&from, &faulty), Err(MoveError::To(_))),
            "the third write fails"
        );
        let to = LocalObjects::new(to_dir.path().to_owned(), Arc::new(OsFs));
        for (version, bytes) in &kept {
            let held = [from.get(*version).ok(), to.get(*version).ok()];
            assert!(
                held.iter().flatten().any(|b| b == bytes),
                "each version is in one storage or the other"
            );
        }
        assert_eq!(move_objects(&from, &to).expect("moved"), 1, "the rest");
        for (version, bytes) in &kept {
            assert_eq!(&to.get(*version).expect("moved"), bytes, "all moved");
        }
    }
}
