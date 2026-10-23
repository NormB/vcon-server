//! The filesystem calls Store makes for object files, behind an interface
//! so tests can simulate a crash. The catalog is SQLite, which does its own
//! I/O and is durable once a commit returns.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

/// The filesystem calls Store makes.
pub trait Fs: Send + Sync {
    /// Creates a file that must not exist yet and writes `data` to it,
    /// without syncing.
    fn write_new(&self, path: &Path, data: &[u8]) -> io::Result<()>;
    /// Makes a file's data durable.
    fn sync_file(&self, path: &Path) -> io::Result<()>;
    /// Renames a file, replacing any file at `to`.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Makes a directory's entries durable: files created, renamed or
    /// removed in it.
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
    /// Reads a whole file.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;
    /// Removes a file.
    fn remove(&self, path: &Path) -> io::Result<()>;
    /// The names in a directory, sorted.
    fn list(&self, dir: &Path) -> io::Result<Vec<OsString>>;
    /// Creates a directory and its parents.
    fn create_dir_all(&self, dir: &Path) -> io::Result<()>;
}

/// The operating system's filesystem. No test can see whether a sync
/// reached the disk without cutting power, so Store's crash tests use the
/// crash filesystem's model of syncing instead.
pub struct OsFs;

impl Fs for OsFs {
    fn write_new(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(data)
    }

    fn sync_file(&self, path: &Path) -> io::Result<()> {
        File::open(path)?.sync_all()
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        File::open(dir)?.sync_all()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<OsString>> {
        let mut names = std::fs::read_dir(dir)?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }

    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }
}

#[cfg(test)]
pub(crate) use crash::{CrashFs, Op};

#[cfg(test)]
mod crash {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::Fs;

    /// One call, as the crash filesystem logs it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum Op {
        Write(PathBuf),
        SyncFile(PathBuf),
        Rename(PathBuf, PathBuf),
        SyncDir(PathBuf),
        Remove(PathBuf),
    }

    /// An in-memory filesystem that keeps what was synced apart from what
    /// was not, so a simulated crash can drop the latter. A file's data
    /// survives a crash once the file is synced; a file's creation, rename
    /// or removal once its directory is synced. Directories themselves are
    /// durable as soon as they are made, and hold only files.
    #[derive(Default)]
    pub(crate) struct CrashFs {
        state: Mutex<State>,
    }

    #[derive(Default)]
    struct State {
        dirs: BTreeMap<PathBuf, Dir>,
        files: Vec<File>,
        log: Vec<Op>,
    }

    /// A directory's entries, each naming an index into `State::files`.
    #[derive(Default)]
    struct Dir {
        now: BTreeMap<OsString, usize>,
        synced: BTreeMap<OsString, usize>,
    }

    struct File {
        now: Vec<u8>,
        synced: Vec<u8>,
    }

    fn not_found() -> io::Error {
        io::Error::from(io::ErrorKind::NotFound)
    }

    /// A path's directory and name.
    fn split(path: &Path) -> io::Result<(PathBuf, OsString)> {
        match (path.parent(), path.file_name()) {
            (Some(dir), Some(name)) => Ok((dir.to_owned(), name.to_owned())),
            _ => Err(io::Error::from(io::ErrorKind::InvalidInput)),
        }
    }

    impl State {
        fn dir(&mut self, dir: &Path) -> io::Result<&mut Dir> {
            self.dirs.get_mut(dir).ok_or_else(not_found)
        }

        fn file(&mut self, path: &Path) -> io::Result<usize> {
            let (dir, name) = split(path)?;
            self.dir(&dir)?
                .now
                .get(&name)
                .copied()
                .ok_or_else(not_found)
        }
    }

    impl CrashFs {
        pub(crate) fn new() -> CrashFs {
            CrashFs::default()
        }

        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state
                .lock()
                .expect("no test panicked holding the lock")
        }

        /// Loses everything not yet synced.
        pub(crate) fn crash(&self) {
            let mut state = self.state();
            for dir in state.dirs.values_mut() {
                dir.now = dir.synced.clone();
            }
            for file in &mut state.files {
                file.now = file.synced.clone();
            }
        }

        /// Every write, sync, rename and remove so far, in order.
        pub(crate) fn log(&self) -> Vec<Op> {
            self.state().log.clone()
        }
    }

    impl Fs for CrashFs {
        fn write_new(&self, path: &Path, data: &[u8]) -> io::Result<()> {
            let (dir, name) = split(path)?;
            let mut state = self.state();
            let index = state.files.len();
            let entries = &mut state.dir(&dir)?.now;
            if entries.contains_key(&name) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            entries.insert(name, index);
            state.files.push(File {
                now: data.to_vec(),
                synced: Vec::new(),
            });
            state.log.push(Op::Write(path.to_owned()));
            Ok(())
        }

        fn sync_file(&self, path: &Path) -> io::Result<()> {
            let mut state = self.state();
            let index = state.file(path)?;
            let file = &mut state.files[index];
            file.synced = file.now.clone();
            state.log.push(Op::SyncFile(path.to_owned()));
            Ok(())
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            let ((from_dir, from_name), (to_dir, to_name)) = (split(from)?, split(to)?);
            let mut state = self.state();
            let index = state.file(from)?;
            state.dir(&to_dir)?;
            state.dir(&from_dir)?.now.remove(&from_name);
            state.dir(&to_dir)?.now.insert(to_name, index);
            state.log.push(Op::Rename(from.to_owned(), to.to_owned()));
            Ok(())
        }

        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            let mut state = self.state();
            let entries = state.dir(dir)?;
            entries.synced = entries.now.clone();
            state.log.push(Op::SyncDir(dir.to_owned()));
            Ok(())
        }

        fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
            let mut state = self.state();
            let index = state.file(path)?;
            Ok(state.files[index].now.clone())
        }

        fn remove(&self, path: &Path) -> io::Result<()> {
            let (dir, name) = split(path)?;
            let mut state = self.state();
            state.dir(&dir)?.now.remove(&name).ok_or_else(not_found)?;
            state.log.push(Op::Remove(path.to_owned()));
            Ok(())
        }

        fn list(&self, dir: &Path) -> io::Result<Vec<OsString>> {
            Ok(self.state().dir(dir)?.now.keys().cloned().collect())
        }

        fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
            self.state().dirs.entry(dir.to_owned()).or_default();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind::{AlreadyExists, NotFound};
    use std::path::PathBuf;

    use super::*;

    /// What every implementation must do, crash or no crash.
    fn behaves_like_a_filesystem(fs: &dyn Fs, root: &Path) {
        let dir = root.join("objects");
        let (a, b) = (dir.join("a"), dir.join("b"));
        assert_eq!(
            fs.write_new(&a, b"x").map_err(|e| e.kind()),
            Err(NotFound),
            "no dir yet"
        );
        fs.create_dir_all(&dir).expect("create the dir");
        fs.write_new(&a, b"one").expect("write");
        assert_eq!(
            fs.write_new(&a, b"two").map_err(|e| e.kind()),
            Err(AlreadyExists)
        );
        assert_eq!(fs.read(&a).expect("read"), b"one");
        fs.sync_file(&a).expect("sync the file");
        fs.sync_dir(&dir).expect("sync the dir");
        fs.rename(&a, &b).expect("rename");
        assert_eq!(fs.read(&a).map_err(|e| e.kind()), Err(NotFound));
        assert_eq!(fs.read(&b).expect("read"), b"one");
        fs.rename(&b, &b).expect("rename onto itself");
        assert_eq!(fs.read(&b).expect("read"), b"one");
        fs.write_new(&a, b"three").expect("write again");
        fs.rename(&a, &b).expect("rename over");
        assert_eq!(fs.read(&b).expect("read"), b"three");
        fs.write_new(&a, b"four").expect("write");
        assert_eq!(
            fs.list(&dir).expect("list"),
            [OsString::from("a"), OsString::from("b")]
        );
        fs.remove(&a).expect("remove");
        fs.remove(&b).expect("remove");
        assert_eq!(fs.list(&dir).expect("list"), Vec::<OsString>::new());
        assert_eq!(fs.remove(&b).map_err(|e| e.kind()), Err(NotFound));
        assert_eq!(fs.sync_file(&b).map_err(|e| e.kind()), Err(NotFound));
        assert_eq!(
            fs.sync_dir(&root.join("none")).map_err(|e| e.kind()),
            Err(NotFound)
        );
        assert_eq!(
            fs.list(&root.join("none")).map_err(|e| e.kind()),
            Err(NotFound)
        );
    }

    #[test]
    fn the_os_filesystem_behaves_like_a_filesystem() {
        let root = tempfile::tempdir().expect("temp dir");
        behaves_like_a_filesystem(&OsFs, root.path());
    }

    #[test]
    fn the_crash_filesystem_behaves_like_a_filesystem() {
        behaves_like_a_filesystem(&CrashFs::new(), Path::new("/data"));
    }

    fn dir() -> PathBuf {
        PathBuf::from("/data/objects")
    }

    /// A crash filesystem with one file `f` in `dir()`, after `steps`.
    fn after(steps: impl FnOnce(&CrashFs, &Path)) -> CrashFs {
        let fs = CrashFs::new();
        fs.create_dir_all(&dir()).expect("create the dir");
        steps(&fs, &dir().join("f"));
        fs
    }

    fn synced(fs: &CrashFs, f: &Path) {
        fs.write_new(f, b"data").expect("write");
        fs.sync_file(f).expect("sync the file");
        fs.sync_dir(&dir()).expect("sync the dir");
    }

    #[test]
    fn a_crash_drops_a_file_whose_entry_was_not_synced() {
        for sync_data in [false, true] {
            let fs = after(|fs, f| {
                fs.write_new(f, b"data").expect("write");
                if sync_data {
                    fs.sync_file(f).expect("sync the file");
                }
            });
            fs.crash();
            assert_eq!(
                fs.read(&dir().join("f")).map_err(|e| e.kind()),
                Err(NotFound)
            );
        }
    }

    #[test]
    fn a_crash_empties_a_file_whose_data_was_not_synced() {
        let fs = after(|fs, f| {
            fs.write_new(f, b"data").expect("write");
            fs.sync_dir(&dir()).expect("sync the dir");
        });
        fs.crash();
        assert_eq!(fs.read(&dir().join("f")).expect("the entry survives"), b"");
    }

    #[test]
    fn a_crash_keeps_a_file_synced_in_both_ways() {
        let fs = after(synced);
        fs.crash();
        assert_eq!(fs.read(&dir().join("f")).expect("survives"), b"data");
    }

    #[test]
    fn a_rename_or_remove_survives_a_crash_only_once_the_dir_is_synced() {
        for sync in [false, true] {
            let renamed = after(|fs, f| {
                synced(fs, f);
                fs.rename(f, &dir().join("g")).expect("rename");
                if sync {
                    fs.sync_dir(&dir()).expect("sync the dir");
                }
            });
            renamed.crash();
            let names = if sync { ["g"] } else { ["f"] };
            assert_eq!(
                renamed.list(&dir()).expect("list"),
                names.map(OsString::from),
                "rename, sync {sync}"
            );
            assert_eq!(renamed.read(&dir().join(names[0])).expect("read"), b"data");

            let removed = after(|fs, f| {
                synced(fs, f);
                fs.remove(f).expect("remove");
                if sync {
                    fs.sync_dir(&dir()).expect("sync the dir");
                }
            });
            removed.crash();
            let left = removed.list(&dir()).expect("list").len();
            assert_eq!(left, usize::from(!sync), "remove, sync {sync}");
        }
    }

    #[test]
    fn every_call_that_changes_state_is_logged_in_order() {
        let fs = after(|fs, f| {
            synced(fs, f);
            fs.rename(f, &dir().join("g")).expect("rename");
            fs.remove(&dir().join("g")).expect("remove");
        });
        let (f, g) = (dir().join("f"), dir().join("g"));
        assert_eq!(
            fs.log(),
            [
                Op::Write(f.clone()),
                Op::SyncFile(f.clone()),
                Op::SyncDir(dir()),
                Op::Rename(f, g.clone()),
                Op::Remove(g),
            ]
        );
    }
}
