//! The filesystem calls Store makes for object files, behind an interface
//! so tests can simulate a crash. The catalog is SQLite, which does its own
//! I/O and is durable once a commit returns.

use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

/// The mode of every file the server makes: the service account's alone,
/// whatever the umask of the process. The umask can only take bits away,
/// and these have none for group or others to lose.
pub const FILE_MODE: u32 = 0o600;

/// The mode of every directory the server makes, as `FILE_MODE`.
pub const DIR_MODE: u32 = 0o700;

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
    /// Creates a directory and its parents, without syncing. A new
    /// directory survives a crash once the directory holding it is synced.
    fn create_dir_all(&self, dir: &Path) -> io::Result<()>;
    /// Removes a directory and everything in it, without syncing. The
    /// removal survives a crash once the directory that held it is synced.
    fn remove_dir_all(&self, dir: &Path) -> io::Result<()>;
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
            .mode(FILE_MODE)
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
        DirBuilder::new().recursive(true).mode(DIR_MODE).create(dir)
    }

    fn remove_dir_all(&self, dir: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(dir)
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
        CreateDir(PathBuf),
        RemoveDir(PathBuf),
    }

    /// An in-memory filesystem that keeps what was synced apart from what
    /// was not, so a simulated crash can drop the latter. A file's data
    /// survives a crash once the file is synced; an entry in a directory, a
    /// file or a directory made, renamed or removed there, once that
    /// directory is synced. A directory lost in a crash takes everything
    /// in it.
    ///
    /// The model is rooted at a directory that exists on disk and is
    /// durable. A directory made in the model is made on disk too, because
    /// SQLite opens its catalogs there; a crash drops it only from the model.
    /// A directory is removed only from the model, since a crash may bring
    /// it back and SQLite's files in it must then still be there.
    pub(crate) struct CrashFs {
        state: Mutex<State>,
    }

    struct State {
        root: PathBuf,
        dirs: BTreeMap<PathBuf, Dir>,
        files: Vec<File>,
        log: Vec<Op>,
        /// From this many changes on, every change fails.
        fail_from: Option<usize>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Entry {
        /// An index into `State::files`.
        File(usize),
        Dir,
    }

    /// A directory's entries, as they are and as a crash would leave them.
    #[derive(Default)]
    struct Dir {
        now: BTreeMap<OsString, Entry>,
        synced: BTreeMap<OsString, Entry>,
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
        /// Fails once the fail point is reached, as a dead process would.
        fn alive(&self) -> io::Result<()> {
            match self.fail_from {
                Some(n) if self.log.len() >= n => Err(io::Error::other("the process has died")),
                _ => Ok(()),
            }
        }

        /// Whether `dir` is the root or is reached from it through
        /// entries as they are now.
        fn exists(&self, dir: &Path) -> bool {
            if dir == self.root {
                return true;
            }
            let Ok((parent, name)) = split(dir) else {
                return false;
            };
            self.exists(&parent)
                && self
                    .dirs
                    .get(&parent)
                    .is_some_and(|p| p.now.get(&name) == Some(&Entry::Dir))
        }

        fn dir(&mut self, dir: &Path) -> io::Result<&mut Dir> {
            if !self.exists(dir) {
                return Err(not_found());
            }
            self.dirs.get_mut(dir).ok_or_else(not_found)
        }

        fn file(&mut self, path: &Path) -> io::Result<usize> {
            let (dir, name) = split(path)?;
            match self.dir(&dir)?.now.get(&name) {
                Some(Entry::File(index)) => Ok(*index),
                _ => Err(not_found()),
            }
        }
    }

    impl CrashFs {
        /// A model rooted at `root`, an existing directory.
        pub(crate) fn new(root: &Path) -> CrashFs {
            CrashFs {
                state: Mutex::new(State {
                    root: root.to_owned(),
                    dirs: BTreeMap::from([(root.to_owned(), Dir::default())]),
                    files: Vec::new(),
                    log: Vec::new(),
                    fail_from: None,
                }),
            }
        }

        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state
                .lock()
                .expect("no test panicked holding the lock")
        }

        /// Loses everything not yet synced.
        pub(crate) fn crash(&self) {
            let mut state = self.state();
            state.fail_from = None;
            for dir in state.dirs.values_mut() {
                dir.now = dir.synced.clone();
            }
            for file in &mut state.files {
                file.now = file.synced.clone();
            }
        }

        /// From the `n`th change on (counting from 0 over the whole log),
        /// every change fails and changes nothing, as for a process that
        /// has died. `crash` clears it.
        pub(crate) fn fail_from(&self, n: usize) {
            self.state().fail_from = Some(n);
        }

        /// Every change so far, in order.
        pub(crate) fn log(&self) -> Vec<Op> {
            self.state().log.clone()
        }
    }

    impl Fs for CrashFs {
        fn write_new(&self, path: &Path, data: &[u8]) -> io::Result<()> {
            let (dir, name) = split(path)?;
            let mut state = self.state();
            state.alive()?;
            let index = state.files.len();
            let entries = &mut state.dir(&dir)?.now;
            if entries.contains_key(&name) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            entries.insert(name, Entry::File(index));
            state.files.push(File {
                now: data.to_vec(),
                synced: Vec::new(),
            });
            state.log.push(Op::Write(path.to_owned()));
            Ok(())
        }

        fn sync_file(&self, path: &Path) -> io::Result<()> {
            let mut state = self.state();
            state.alive()?;
            let index = state.file(path)?;
            let file = &mut state.files[index];
            file.synced = file.now.clone();
            state.log.push(Op::SyncFile(path.to_owned()));
            Ok(())
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            let ((from_dir, from_name), (to_dir, to_name)) = (split(from)?, split(to)?);
            let mut state = self.state();
            state.alive()?;
            let index = state.file(from)?;
            state.dir(&to_dir)?;
            state.dir(&from_dir)?.now.remove(&from_name);
            state.dir(&to_dir)?.now.insert(to_name, Entry::File(index));
            state.log.push(Op::Rename(from.to_owned(), to.to_owned()));
            Ok(())
        }

        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            let mut state = self.state();
            state.alive()?;
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
            state.alive()?;
            state.file(path)?;
            state.dir(&dir)?.now.remove(&name);
            state.log.push(Op::Remove(path.to_owned()));
            Ok(())
        }

        fn list(&self, dir: &Path) -> io::Result<Vec<OsString>> {
            Ok(self.state().dir(dir)?.now.keys().cloned().collect())
        }

        fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
            let mut state = self.state();
            state.alive()?;
            let below = dir
                .strip_prefix(&state.root)
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?
                .to_owned();
            // Find what is missing before touching the disk, so a refusal
            // makes nothing there either.
            let mut path = state.root.clone();
            let mut missing = Vec::new();
            for part in &below {
                let next = path.join(part);
                let parent = std::mem::replace(&mut path, next);
                if !missing.is_empty() {
                    missing.push(path.clone());
                    continue;
                }
                match state.dir(&parent)?.now.get(part) {
                    Some(Entry::Dir) => {}
                    Some(Entry::File(_)) => {
                        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
                    }
                    None => missing.push(path.clone()),
                }
            }
            if missing.is_empty() {
                return Ok(());
            }
            std::fs::create_dir_all(dir)?;
            for path in missing {
                let (parent, name) = split(&path)?;
                state.dir(&parent)?.now.insert(name, Entry::Dir);
                state.dirs.insert(path, Dir::default());
            }
            state.log.push(Op::CreateDir(dir.to_owned()));
            Ok(())
        }

        fn remove_dir_all(&self, dir: &Path) -> io::Result<()> {
            let (parent, name) = split(dir)?;
            let mut state = self.state();
            state.alive()?;
            if !state.exists(dir) {
                return Err(not_found());
            }
            state.dir(&parent)?.now.remove(&name);
            state.log.push(Op::RemoveDir(dir.to_owned()));
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
        fs.create_dir_all(&dir).expect("create it again");
        fs.write_new(&a, b"one").expect("write");
        assert_eq!(
            fs.write_new(&a, b"two").map_err(|e| e.kind()),
            Err(AlreadyExists),
            "write_new never replaces a file"
        );
        assert!(fs.create_dir_all(&a).is_err(), "a file has the name");
        assert!(
            fs.create_dir_all(&a.join("d")).is_err(),
            "a file stands in the way"
        );
        assert!(!a.join("d").exists(), "and nothing is made on disk");
        assert_eq!(
            fs.read(&a).expect("read"),
            b"one",
            "the refused write left the first one"
        );
        fs.sync_file(&a).expect("sync the file");
        fs.sync_dir(&dir).expect("sync the dir");
        fs.rename(&a, &b).expect("rename");
        assert_eq!(
            fs.read(&a).map_err(|e| e.kind()),
            Err(NotFound),
            "rename moves the file away"
        );
        assert_eq!(
            fs.read(&b).expect("read"),
            b"one",
            "the file is under its new name"
        );
        fs.rename(&b, &b).expect("rename onto itself");
        assert_eq!(
            fs.read(&b).expect("read"),
            b"one",
            "renaming onto itself changes nothing"
        );
        fs.write_new(&a, b"three").expect("write again");
        fs.rename(&a, &b).expect("rename over");
        assert_eq!(
            fs.read(&b).expect("read"),
            b"three",
            "rename replaces the target"
        );
        fs.write_new(&a, b"four").expect("write");
        fs.create_dir_all(&dir.join("c"))
            .expect("a directory inside");
        assert_eq!(
            fs.list(&dir).expect("list"),
            ["a", "b", "c"].map(OsString::from),
            "files and directories, sorted"
        );
        assert_eq!(
            fs.write_new(&dir.join("c"), b"x").map_err(|e| e.kind()),
            Err(AlreadyExists),
            "a directory's name is taken"
        );
        assert!(
            fs.remove(&dir.join("c")).is_err(),
            "remove takes only a file"
        );
        assert!(
            fs.remove_dir_all(&a).is_err(),
            "remove_dir_all takes only a directory"
        );
        fs.create_dir_all(&dir.join("c/d"))
            .expect("a directory inside c");
        fs.write_new(&dir.join("c/f"), b"x")
            .expect("a file inside c");
        fs.remove_dir_all(&dir.join("c"))
            .expect("remove c and all in it");
        assert_eq!(
            fs.list(&dir.join("c")).map_err(|e| e.kind()),
            Err(NotFound),
            "remove_dir_all removed c itself"
        );
        assert_eq!(
            fs.remove_dir_all(&dir.join("c")).map_err(|e| e.kind()),
            Err(NotFound),
            "a directory that is gone cannot be removed again"
        );
        fs.create_dir_all(&dir.join("c")).expect("c again");
        fs.remove(&a).expect("remove");
        fs.remove(&b).expect("remove");
        assert_eq!(
            fs.list(&dir).expect("list"),
            [OsString::from("c")],
            "the removed files are gone and c is left"
        );
        assert_eq!(
            fs.remove(&b).map_err(|e| e.kind()),
            Err(NotFound),
            "b was removed"
        );
        assert_eq!(
            fs.sync_file(&b).map_err(|e| e.kind()),
            Err(NotFound),
            "b was removed"
        );
        assert_eq!(
            fs.sync_dir(&root.join("none")).map_err(|e| e.kind()),
            Err(NotFound),
            "no directory has that name"
        );
        assert_eq!(
            fs.list(&root.join("none")).map_err(|e| e.kind()),
            Err(NotFound),
            "no directory has that name"
        );
    }

    #[test]
    fn the_os_filesystem_behaves_like_a_filesystem() {
        let root = tempfile::tempdir().expect("temp dir");
        behaves_like_a_filesystem(&OsFs, root.path());
    }

    #[test]
    fn the_crash_filesystem_behaves_like_a_filesystem() {
        let root = tempfile::tempdir().expect("temp dir");
        behaves_like_a_filesystem(&CrashFs::new(root.path()), root.path());
    }

    /// A crash filesystem rooted at a temporary directory.
    struct Model {
        root: tempfile::TempDir,
        fs: CrashFs,
    }

    impl Model {
        fn new() -> Model {
            let root = tempfile::tempdir().expect("temp dir");
            let fs = CrashFs::new(root.path());
            Model { root, fs }
        }

        fn root(&self) -> &Path {
            self.root.path()
        }

        /// The directory the file tests use, made and synced.
        fn dir(&self) -> PathBuf {
            self.root().join("objects")
        }

        fn f(&self) -> PathBuf {
            self.dir().join("f")
        }
    }

    /// A model with a durable `dir()`, after `steps` on its file `f()`.
    fn after(steps: impl FnOnce(&CrashFs, &Path)) -> Model {
        let m = Model::new();
        m.fs.create_dir_all(&m.dir()).expect("create the dir");
        m.fs.sync_dir(m.root()).expect("sync the root");
        steps(&m.fs, &m.f());
        m
    }

    fn synced(fs: &CrashFs, f: &Path) {
        fs.write_new(f, b"data").expect("write");
        fs.sync_file(f).expect("sync the file");
        fs.sync_dir(f.parent().expect("a dir"))
            .expect("sync the dir");
    }

    #[test]
    fn a_crash_drops_a_file_whose_entry_was_not_synced() {
        for sync_data in [false, true] {
            let m = after(|fs, f| {
                fs.write_new(f, b"data").expect("write");
                if sync_data {
                    fs.sync_file(f).expect("sync the file");
                }
            });
            m.fs.crash();
            assert_eq!(
                m.fs.read(&m.f()).map_err(|e| e.kind()),
                Err(NotFound),
                "the entry was never synced, so the crash loses the file (data synced: {sync_data})"
            );
        }
    }

    #[test]
    fn a_crash_empties_a_file_whose_data_was_not_synced() {
        let m = after(|fs, f| {
            fs.write_new(f, b"data").expect("write");
            fs.sync_dir(f.parent().expect("a dir"))
                .expect("sync the dir");
        });
        m.fs.crash();
        assert_eq!(
            m.fs.read(&m.f()).expect("the entry survives"),
            b"",
            "the entry was synced but the data was not, so the file comes back empty"
        );
    }

    #[test]
    fn a_crash_keeps_a_file_synced_in_both_ways() {
        let m = after(synced);
        m.fs.crash();
        assert_eq!(
            m.fs.read(&m.f()).expect("survives"),
            b"data",
            "data and entry were both synced, so the crash keeps the file whole"
        );
    }

    #[test]
    fn a_rename_or_remove_survives_a_crash_only_once_the_dir_is_synced() {
        for sync in [false, true] {
            let renamed = after(|fs, f| {
                synced(fs, f);
                fs.rename(f, &f.with_file_name("g")).expect("rename");
                if sync {
                    fs.sync_dir(f.parent().expect("a dir"))
                        .expect("sync the dir");
                }
            });
            renamed.fs.crash();
            let names = if sync { ["g"] } else { ["f"] };
            assert_eq!(
                renamed.fs.list(&renamed.dir()).expect("list"),
                names.map(OsString::from),
                "rename, sync {sync}"
            );
            assert_eq!(
                renamed
                    .fs
                    .read(&renamed.dir().join(names[0]))
                    .expect("read"),
                b"data",
                "whichever name survives, the synced data is under it"
            );

            let removed = after(|fs, f| {
                synced(fs, f);
                fs.remove(f).expect("remove");
                if sync {
                    fs.sync_dir(f.parent().expect("a dir"))
                        .expect("sync the dir");
                }
            });
            removed.fs.crash();
            let left = removed.fs.list(&removed.dir()).expect("list").len();
            assert_eq!(left, usize::from(!sync), "remove, sync {sync}");
        }
    }

    #[test]
    fn a_new_directory_survives_a_crash_only_once_its_parent_is_synced() {
        let m = Model::new();
        let (a, b) = (m.root().join("a"), m.root().join("a/b"));
        m.fs.create_dir_all(&b).expect("create a/b");
        m.fs.crash();
        assert_eq!(
            m.fs.list(m.root()).expect("list"),
            Vec::<OsString>::new(),
            "nothing was synced, so the crash loses a"
        );
        assert_eq!(
            m.fs.list(&a).map_err(|e| e.kind()),
            Err(NotFound),
            "a is gone after the crash"
        );

        m.fs.create_dir_all(&b).expect("create a/b");
        m.fs.sync_dir(&a).expect("sync a, not the root");
        m.fs.crash();
        assert_eq!(
            m.fs.list(&b).map_err(|e| e.kind()),
            Err(NotFound),
            "b is lost with a"
        );

        m.fs.create_dir_all(&b).expect("create a/b");
        m.fs.sync_dir(m.root()).expect("sync the root");
        m.fs.crash();
        assert_eq!(
            m.fs.list(m.root()).expect("list"),
            [OsString::from("a")],
            "the root was synced, so its entry for a survives"
        );
        assert_eq!(
            m.fs.list(&a).expect("a survives"),
            Vec::<OsString>::new(),
            "b's entry in a was never synced"
        );

        m.fs.create_dir_all(&b).expect("create b");
        m.fs.sync_dir(&a).expect("sync a");
        m.fs.crash();
        assert_eq!(
            m.fs.list(&b).expect("b survives"),
            Vec::<OsString>::new(),
            "a was synced, so its entry for b survives, and b is empty"
        );
    }

    #[test]
    fn a_lost_directory_takes_its_synced_files_with_it() {
        let m = Model::new();
        let a = m.root().join("a");
        m.fs.create_dir_all(&a).expect("create a");
        synced(&m.fs, &a.join("f"));
        m.fs.crash();
        assert_eq!(
            m.fs.list(m.root()).expect("list"),
            Vec::<OsString>::new(),
            "the root was never synced, so a is lost with its synced file"
        );
        m.fs.create_dir_all(&a).expect("create a again");
        assert_eq!(
            m.fs.list(&a).expect("list"),
            Vec::<OsString>::new(),
            "a new directory starts empty"
        );
    }

    #[test]
    fn a_directory_removal_survives_a_crash_only_once_its_parent_is_synced() {
        for sync in [false, true] {
            let m = Model::new();
            let a = m.root().join("a");
            m.fs.create_dir_all(&a).expect("create a");
            synced(&m.fs, &a.join("f"));
            m.fs.sync_dir(m.root()).expect("sync the root");
            m.fs.remove_dir_all(&a).expect("remove a");
            if sync {
                m.fs.sync_dir(m.root()).expect("sync the root");
            }
            m.fs.crash();
            if sync {
                assert_eq!(
                    m.fs.list(&a).map_err(|e| e.kind()),
                    Err(NotFound),
                    "the root was synced after the removal, so it holds"
                );
            } else {
                assert_eq!(
                    m.fs.read(&a.join("f")).expect("back"),
                    b"data",
                    "the removal was never synced, so a comes back with its file"
                );
            }
        }
    }

    /// A crash may bring a removed directory back, and SQLite's files in
    /// it must then still be there, so removal touches only the model.
    #[test]
    fn a_directory_is_removed_only_from_the_model() {
        let m = Model::new();
        let a = m.root().join("a");
        m.fs.create_dir_all(&a).expect("create a");
        m.fs.remove_dir_all(&a).expect("remove a");
        assert!(
            a.is_dir(),
            "the directory stays on disk, for a crash that brings it back"
        );
    }

    /// SQLite opens its files on disk, so a directory made in the model is
    /// made on disk too.
    #[test]
    fn a_directory_is_also_made_on_disk() {
        let m = Model::new();
        let b = m.root().join("a/b");
        m.fs.create_dir_all(&b).expect("create a/b");
        assert!(
            b.is_dir(),
            "the directory is on disk, where SQLite will open its files"
        );
    }

    #[test]
    fn nothing_outside_the_root_can_be_made() {
        let m = Model::new();
        let outside = m.root().with_file_name("elsewhere-5c1e");
        assert!(
            m.fs.create_dir_all(&outside).is_err(),
            "the model refuses a path outside its root"
        );
        assert!(!outside.exists(), "and makes nothing there on disk");
    }

    #[test]
    fn every_call_that_changes_state_is_logged_in_order() {
        let m = Model::new();
        let (dir, f, g) = (m.dir(), m.f(), m.dir().join("g"));
        m.fs.create_dir_all(&dir).expect("create the dir");
        m.fs.create_dir_all(&dir).expect("already there: no change");
        synced(&m.fs, &f);
        m.fs.rename(&f, &g).expect("rename");
        m.fs.remove(&g).expect("remove");
        m.fs.remove_dir_all(&dir).expect("remove the dir");
        assert_eq!(
            m.fs.log(),
            [
                Op::CreateDir(dir.clone()),
                Op::Write(f.clone()),
                Op::SyncFile(f.clone()),
                Op::SyncDir(dir.clone()),
                Op::Rename(f, g.clone()),
                Op::Remove(g),
                Op::RemoveDir(dir),
            ],
            "one entry per change, in order; the create that changed nothing is not logged"
        );
    }

    #[test]
    fn from_the_fail_point_on_no_change_happens_until_a_crash() {
        let m = after(|fs, f| fs.write_new(f, b"data").expect("write"));
        let ops = m.fs.log().len();
        m.fs.fail_from(ops);
        let (f, g) = (m.f(), m.dir().join("g"));
        assert!(m.fs.sync_file(&f).is_err(), "sync");
        assert!(m.fs.write_new(&g, b"x").is_err(), "write");
        assert!(m.fs.rename(&f, &g).is_err(), "rename");
        assert!(m.fs.sync_dir(&m.dir()).is_err(), "sync dir");
        assert!(m.fs.remove(&f).is_err(), "remove");
        assert!(
            m.fs.create_dir_all(&m.dir().join("d")).is_err(),
            "create dir"
        );
        assert!(m.fs.remove_dir_all(&m.dir()).is_err(), "remove dir");
        assert!(!m.dir().join("d").exists(), "not made on disk either");
        assert_eq!(m.fs.log().len(), ops, "nothing changed");
        assert_eq!(
            m.fs.read(&f).expect("reads still work"),
            b"data",
            "the fail point stops changes, not reads"
        );
        m.fs.crash();
        m.fs.write_new(&g, b"x")
            .expect("works again after the crash");
    }
}
