//! The one rule for a secret file the configuration names: the master key
//! (`master_key`, and `rekey master --new-master`), the TLS private key
//! (`tls.key`), the S3 secret access keys (`storage.s3` and `export.s3`
//! `secret_access_key_file`), the PostgreSQL catalog's password and client
//! key (`catalog.postgres`), and the MCP stdio key (`key_file`).
//!
//! Until 2026-11-09 only the PostgreSQL files were checked (#147); a master
//! key, a TLS key or an S3 secret that every user on the host could read was
//! used without a word. Each is now refused unless it is:
//!
//! - a regular file (a FIFO, a device or a directory is refused before it is
//!   opened, so a FIFO cannot block the read);
//! - owned by the user the process runs as, or by root (or the process is
//!   root), so no other account can replace it;
//! - closed to other users entirely, and not writable by its group: no bit
//!   of [`REFUSED_BITS`]. `0600`, `0400` and `0640` (for a file such as
//!   `0640 root:vcon`) pass; `0644`, `0604` and `0660` do not.
//!
//! A symbolic link is followed, and the file it names is the one checked.
//! The owner and the mode are read from the file opened, so the file read is
//! the file checked. An error names the key, the file and what to run, and
//! never what the file holds.

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use zeroize::Zeroizing;

/// The mode bits a secret file may not have: write for its group, and any
/// access for other users.
pub(crate) const REFUSED_BITS: u32 = 0o027;

/// Why a secret file is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Not a regular file.
    NotAFile,
    /// Owned by another user, not root.
    Owner {
        /// The file's owner.
        uid: u32,
        /// The user the process runs as.
        process: u32,
    },
    /// A bit of [`REFUSED_BITS`] is set.
    Mode {
        /// The file's mode, its permission and special bits.
        mode: u32,
    },
}

/// Whether a file of this kind, mode and owner may hold a secret for a
/// process running as `process_uid`. A pure function, so every case can be
/// tested without root.
pub(crate) fn judge(is_file: bool, mode: u32, file_uid: u32, process_uid: u32) -> Option<Refusal> {
    if !is_file {
        return Some(Refusal::NotAFile);
    }
    if file_uid != process_uid && file_uid != 0 && process_uid != 0 {
        return Some(Refusal::Owner {
            uid: file_uid,
            process: process_uid,
        });
    }
    let mode = mode & 0o7777;
    if mode & REFUSED_BITS != 0 {
        return Some(Refusal::Mode { mode });
    }
    None
}

/// Why a secret file was not opened: the system's error, or the rule's.
#[derive(Debug)]
pub(crate) enum Problem {
    /// It could not be looked at or opened.
    Io(std::io::Error),
    /// It breaks the rule.
    Refused(Refusal),
}

/// The secret file `path` opened, once it passes the rule.
///
/// # Errors
///
/// [`Problem::Io`] when it cannot be looked at or opened, and
/// [`Problem::Refused`] when it breaks the rule.
pub(crate) fn check_open(path: &Path) -> Result<File, Problem> {
    // Before the open: opening a FIFO would block until a writer came.
    if !std::fs::metadata(path).map_err(Problem::Io)?.is_file() {
        return Err(Problem::Refused(Refusal::NotAFile));
    }
    let file = File::open(path).map_err(Problem::Io)?;
    // The opened file's own metadata, so the file read is the file checked.
    let meta = file.metadata().map_err(Problem::Io)?;
    // SAFETY: geteuid takes nothing and cannot fail.
    let process = unsafe { libc::geteuid() };
    match judge_metadata(&meta, process) {
        Some(refusal) => Err(Problem::Refused(refusal)),
        None => Ok(file),
    }
}

/// [`judge`] of a file's metadata, for a process running as `process_uid`.
pub(crate) fn judge_metadata(meta: &std::fs::Metadata, process_uid: u32) -> Option<Refusal> {
    judge(meta.is_file(), meta.mode(), meta.uid(), process_uid)
}

/// The error for `refusal` of the file `path` that `key` names.
pub(crate) fn describe(key: &str, path: &Path, refusal: Refusal) -> String {
    let file = path.display();
    match refusal {
        Refusal::NotAFile => format!("{key} {file}: not a regular file"),
        Refusal::Owner { uid, process } => format!(
            "{key} {file}: the file is owned by uid {uid}, not by the user this runs as (uid \
             {process}) or root; run `chown {process} {file}` (or `chown root`)"
        ),
        Refusal::Mode { mode } => format!(
            "{key} {file}: other users may read or change the file (mode {mode:04o}); run \
             `chmod 600 {file}`, or `chmod 640 {file}` for a group that only reads it"
        ),
    }
}

/// The secret file `path`, named `key` in an error, opened once it passes
/// the rule.
///
/// # Errors
///
/// The error text: the key, the file, and why.
pub(crate) fn open(path: &Path, key: &str) -> Result<File, String> {
    check_open(path).map_err(|problem| match problem {
        Problem::Io(e) => format!("{key} {}: {e}", path.display()),
        Problem::Refused(refusal) => describe(key, path, refusal),
    })
}

/// The bytes of the secret file `path`, named `key` in an error, once it
/// passes the rule. They are read into a buffer of the file's size and
/// zeroed when dropped.
///
/// # Errors
///
/// The error text: the key, the file, and why.
pub(crate) fn read(path: &Path, key: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut file = open(path, key)?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut bytes = Zeroizing::new(Vec::with_capacity(
        usize::try_from(size).unwrap_or(0).saturating_add(1),
    ));
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("{key} {}: {e}", path.display()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const ME: u32 = 1000;
    const OTHER: u32 = 1234;
    const ROOT: u32 = 0;

    fn mode(m: u32) -> Option<Refusal> {
        judge(true, m, ME, ME)
    }

    // SUCCESS: the modes and owners a secret file may have.

    #[test]
    fn mode_0600_passes() {
        assert_eq!(mode(0o600), None, "0600 gives no one else access");
    }

    #[test]
    fn mode_0400_passes() {
        assert_eq!(mode(0o400), None, "0400 gives no one else access");
    }

    #[test]
    fn mode_0640_passes_for_a_group_that_reads_it() {
        assert_eq!(mode(0o640), None, "0640: the group may only read");
    }

    #[test]
    fn mode_0700_passes_since_owner_execute_gives_no_one_else_access() {
        assert_eq!(mode(0o700), None, "owner execute gives no one else access");
    }

    #[test]
    fn a_file_root_owns_passes_for_another_user() {
        assert_eq!(
            judge(true, 0o640, ROOT, ME),
            None,
            "root may own a secret another user reads"
        );
    }

    #[test]
    fn a_process_running_as_root_may_read_a_file_another_user_owns() {
        assert_eq!(
            judge(true, 0o600, OTHER, ROOT),
            None,
            "root may read a file any user owns"
        );
    }

    // FAILURE: the modes, owners and kinds refused.

    #[test]
    fn mode_0644_is_refused() {
        assert_eq!(
            mode(0o644),
            Some(Refusal::Mode { mode: 0o644 }),
            "others may read 0644"
        );
    }

    #[test]
    fn mode_0604_is_refused() {
        assert_eq!(
            mode(0o604),
            Some(Refusal::Mode { mode: 0o604 }),
            "others may read 0604"
        );
    }

    #[test]
    fn mode_0660_is_refused_since_the_group_could_replace_the_secret() {
        assert_eq!(
            mode(0o660),
            Some(Refusal::Mode { mode: 0o660 }),
            "the group may replace a 0660 file"
        );
    }

    #[test]
    fn mode_0620_is_refused_for_group_write_alone() {
        assert_eq!(
            mode(0o620),
            Some(Refusal::Mode { mode: 0o620 }),
            "the group may replace a 0620 file"
        );
    }

    #[test]
    fn mode_0602_is_refused_for_others_write_alone() {
        assert_eq!(
            mode(0o602),
            Some(Refusal::Mode { mode: 0o602 }),
            "others may write 0602"
        );
    }

    #[test]
    fn mode_0601_is_refused_for_others_execute_alone() {
        assert_eq!(
            mode(0o601),
            Some(Refusal::Mode { mode: 0o601 }),
            "others hold the execute bit of 0601"
        );
    }

    #[test]
    fn a_file_another_user_owns_is_refused() {
        assert_eq!(
            judge(true, 0o600, OTHER, ME),
            Some(Refusal::Owner {
                uid: OTHER,
                process: ME
            }),
            "another user could replace the file"
        );
    }

    #[test]
    fn a_file_that_is_not_a_regular_file_is_refused() {
        assert_eq!(
            judge(false, 0o600, ME, ME),
            Some(Refusal::NotAFile),
            "a directory, FIFO or device is not a file"
        );
    }

    // EDGE: the bits that do not matter, and which reason wins.

    #[test]
    fn group_execute_alone_passes() {
        assert_eq!(
            mode(0o610),
            None,
            "group execute alone gives no read or write"
        );
    }

    #[test]
    fn mode_0000_passes_the_rule_and_fails_at_the_read() {
        assert_eq!(
            mode(0o000),
            None,
            "no access for anyone else; the read fails later"
        );
    }

    #[test]
    fn a_setuid_bit_is_not_access_for_anyone_else() {
        assert_eq!(mode(0o4600), None, "setuid grants no one else access");
    }

    #[test]
    fn the_file_type_bits_of_a_full_st_mode_are_ignored() {
        assert_eq!(
            mode(0o100_600),
            None,
            "the file type bits of st_mode are not permission bits"
        );
        assert_eq!(
            mode(0o100_644),
            Some(Refusal::Mode { mode: 0o644 }),
            "the file type bits are dropped and 0644 is still refused"
        );
    }

    #[test]
    fn not_a_file_is_reported_before_the_owner_and_the_mode() {
        assert_eq!(
            judge(false, 0o666, OTHER, ME),
            Some(Refusal::NotAFile),
            "a non-file is refused before its owner or mode is looked at"
        );
    }

    #[test]
    fn the_owner_is_reported_before_the_mode() {
        assert_eq!(
            judge(true, 0o644, OTHER, ME),
            Some(Refusal::Owner {
                uid: OTHER,
                process: ME
            }),
            "the owner is checked before the mode"
        );
    }

    // The error text.

    #[test]
    fn a_mode_refusal_names_the_key_the_file_the_mode_and_the_chmod_to_run() {
        let text = describe(
            "master_key",
            Path::new("/etc/vcon-server/master.key"),
            Refusal::Mode { mode: 0o644 },
        );
        assert_eq!(
            text,
            "master_key /etc/vcon-server/master.key: other users may read or change the file \
             (mode 0644); run `chmod 600 /etc/vcon-server/master.key`, or `chmod 640 \
             /etc/vcon-server/master.key` for a group that only reads it",
            "the text names the key, the file, the mode and both fixes"
        );
    }

    #[test]
    fn an_owner_refusal_names_both_users_and_the_chown_to_run() {
        let text = describe(
            "tls.key",
            Path::new("/k.pem"),
            Refusal::Owner {
                uid: OTHER,
                process: ME,
            },
        );
        assert_eq!(
            text,
            "tls.key /k.pem: the file is owned by uid 1234, not by the user this runs as (uid \
             1000) or root; run `chown 1000 /k.pem` (or `chown root`)",
            "the text names both uids and the chown"
        );
    }

    #[test]
    fn a_not_a_file_refusal_names_the_key_and_the_file() {
        assert_eq!(
            describe("key_file", Path::new("/d"), Refusal::NotAFile),
            "key_file /d: not a regular file",
            "the text names the key and the file"
        );
    }

    #[test]
    fn a_setuid_mode_is_shown_with_its_special_bits() {
        let text = describe("k", Path::new("/f"), Refusal::Mode { mode: 0o4644 });
        assert!(
            text.contains("(mode 4644)"),
            "the mode is shown with its special bits"
        );
    }

    // Real files.

    fn file(dir: &tempfile::TempDir, name: &str, bytes: &[u8], m: u32) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(m)).expect("chmod");
        path
    }

    // A file another user owns cannot be made without root, so the reading
    // of a real file's owner is tested with a process uid that is not the
    // file's: the conversion from metadata, not the wire.
    #[test]
    fn a_real_file_s_owner_is_read_from_its_metadata() {
        let dir = tempfile::tempdir().expect("dir");
        let path = file(&dir, "s", b"x", 0o600);
        let meta = std::fs::metadata(&path).expect("metadata");
        let owner = std::os::unix::fs::MetadataExt::uid(&meta);
        let process = owner + 1;
        assert_eq!(
            judge_metadata(&meta, process),
            Some(Refusal::Owner {
                uid: owner,
                process
            }),
            "the file's own uid is read, not the process's"
        );
    }

    #[test]
    fn a_real_file_s_mode_is_read_from_its_metadata() {
        let dir = tempfile::tempdir().expect("dir");
        let path = file(&dir, "s", b"x", 0o604);
        let meta = std::fs::metadata(&path).expect("metadata");
        let owner = std::os::unix::fs::MetadataExt::uid(&meta);
        assert_eq!(
            judge_metadata(&meta, owner),
            Some(Refusal::Mode { mode: 0o604 }),
            "the file's own mode is read"
        );
    }

    #[test]
    fn a_0600_file_is_read_whole() {
        let dir = tempfile::tempdir().expect("dir");
        let path = file(&dir, "s", b"the secret\n", 0o600);
        assert_eq!(
            read(&path, "k").expect("read").as_slice(),
            b"the secret\n",
            "the file's bytes come back whole"
        );
    }

    #[test]
    fn a_0640_file_is_read() {
        let dir = tempfile::tempdir().expect("dir");
        let path = file(&dir, "s", b"x", 0o640);
        assert!(read(&path, "k").is_ok(), "a group may read a 0640 file");
    }

    #[test]
    fn a_0644_file_is_refused_with_its_key_and_path() {
        let dir = tempfile::tempdir().expect("dir");
        let path = file(&dir, "s", b"x", 0o644);
        let err = read(&path, "storage.s3.secret_access_key_file").expect_err("refused");
        assert!(
            err.starts_with(&format!(
                "storage.s3.secret_access_key_file {}: other users",
                path.display()
            )),
            "{err}"
        );
        assert!(err.contains("(mode 0644)"), "{err}");
    }

    #[test]
    fn a_refusal_never_holds_the_secret() {
        let dir = tempfile::tempdir().expect("dir");
        let path = file(&dir, "s", b"s3cr3t-value-0123", 0o644);
        let err = read(&path, "k").expect_err("refused");
        assert!(!err.contains("s3cr3t"), "{err}");
    }

    #[test]
    fn a_symlink_to_a_0600_file_is_followed_and_read() {
        let dir = tempfile::tempdir().expect("dir");
        let target = file(&dir, "real", b"via link", 0o600);
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert_eq!(
            read(&link, "k").expect("read").as_slice(),
            b"via link",
            "the link's target is read"
        );
    }

    #[test]
    fn a_symlink_to_a_0644_file_is_refused_by_the_target_s_mode() {
        let dir = tempfile::tempdir().expect("dir");
        let target = file(&dir, "real", b"x", 0o644);
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let err = read(&link, "k").expect_err("refused");
        assert!(
            err.starts_with(&format!("k {}: other users", link.display())),
            "{err}"
        );
    }

    #[test]
    fn a_dangling_symlink_is_refused_as_missing() {
        let dir = tempfile::tempdir().expect("dir");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("gone"), &link).expect("symlink");
        let err = read(&link, "k").expect_err("refused");
        assert!(err.contains("No such file or directory"), "{err}");
    }

    #[test]
    fn a_missing_file_names_the_key_the_path_and_the_os_error() {
        let err = read(Path::new("/nonexistent/vcon-secret"), "master_key").expect_err("refused");
        assert_eq!(
            err, "master_key /nonexistent/vcon-secret: No such file or directory (os error 2)",
            "the text names the key, the path and the OS error"
        );
    }

    #[test]
    fn a_directory_is_refused_as_not_a_regular_file() {
        let dir = tempfile::tempdir().expect("dir");
        let err = read(dir.path(), "k").expect_err("refused");
        assert_eq!(
            err,
            format!("k {}: not a regular file", dir.path().display()),
            "a directory is refused by kind"
        );
    }

    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let dir = tempfile::tempdir().expect("dir");
        let fifo = dir.path().join("fifo");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo");
        assert!(made.success(), "mkfifo made the FIFO");
        let err = read(&fifo, "k").expect_err("refused");
        assert!(err.ends_with("not a regular file"), "{err}");
    }

    #[test]
    fn a_0000_file_is_refused_at_the_open() {
        let dir = tempfile::tempdir().expect("dir");
        let path = file(&dir, "s", b"x", 0o000);
        // Root opens a 0000 file; anyone else is refused by the kernel.
        // SAFETY: geteuid takes nothing and cannot fail.
        if unsafe { libc::geteuid() } == 0 {
            assert!(read(&path, "k").is_ok(), "root opens a 0000 file");
        } else {
            let err = read(&path, "k").expect_err("refused");
            assert!(err.contains("Permission denied"), "{err}");
        }
    }
}
