//! Who may use the data directory at once. `serve` runs alone among
//! servers; a command that rewrites the registry or the tenants' catalogs
//! runs alone; the other commands run beside a server and beside each
//! other, but never beside a command that runs alone.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::{fmt, io};

/// How a process shares the data directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sharing {
    /// A server: never beside another server or a command that runs alone.
    /// Two servers on one directory would each recover its tenants at open,
    /// and one's recovery could remove a write the other has under way.
    Serve,
    /// A command that runs with nothing else in the data directory.
    Alone,
    /// A command that runs beside a server and other such commands.
    Beside,
}

/// The work a process does in the data directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Serve,
    TenantCreate,
    TenantList,
    TenantSetRetention,
    TenantRemove,
    KeyCreate,
    KeyList,
    KeyRevoke,
    RekeyTenant,
    RekeyMaster,
    AuditExport,
    Check,
    Backup,
}

impl Work {
    /// How this work shares the data directory.
    pub fn sharing(self) -> Sharing {
        match self {
            Work::Serve => Sharing::Serve,
            Work::TenantCreate
            | Work::TenantRemove
            | Work::RekeyTenant
            | Work::RekeyMaster
            | Work::Check => Sharing::Alone,
            Work::TenantList
            | Work::TenantSetRetention
            | Work::KeyCreate
            | Work::KeyList
            | Work::KeyRevoke
            | Work::AuditExport
            | Work::Backup => Sharing::Beside,
        }
    }
}

/// The data directory's locks, held while this value lives: the open files
/// are kept only for their locks.
#[derive(Debug)]
pub struct DataDirLock {
    _held: Vec<File>,
}

/// Why the data directory could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Something that may not run beside this `Sharing` holds it.
    InUse(Sharing),
    /// A lock file could not be opened or locked.
    Io(io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::InUse(Sharing::Serve) => f.write_str(
                "the data directory is in use by another vcon-server or a command that runs alone",
            ),
            LockError::InUse(Sharing::Alone) => f.write_str(
                "the data directory is in use; stop the server and any other command first",
            ),
            LockError::InUse(Sharing::Beside) => {
                f.write_str("the data directory is in use by a command that runs alone")
            }
            LockError::Io(e) => write!(f, "the data directory cannot be locked: {e}"),
        }
    }
}

/// Takes the data directory for work that shares it as `sharing`, or says
/// what holds it.
///
/// Two lock files carry the rule: `lock`, taken exclusively by a server and
/// by a command that runs alone, and `lock.commands`, shared by commands
/// that run beside a server and taken exclusively by one that runs alone.
pub fn lock(dir: &Path, sharing: Sharing) -> Result<DataDirLock, LockError> {
    let take = |name: &str, how| {
        flock(&dir.join(name), how).map_err(|e| match e.kind() {
            io::ErrorKind::WouldBlock => LockError::InUse(sharing),
            _ => LockError::Io(e),
        })
    };
    let held = match sharing {
        Sharing::Serve => vec![take(SERVERS, libc::LOCK_EX)?],
        Sharing::Alone => vec![
            take(SERVERS, libc::LOCK_EX)?,
            take(COMMANDS, libc::LOCK_EX)?,
        ],
        Sharing::Beside => vec![take(COMMANDS, libc::LOCK_SH)?],
    };
    Ok(DataDirLock { _held: held })
}

const SERVERS: &str = "lock";
const COMMANDS: &str = "lock.commands";

/// Opens `path`, made if missing, and locks it without waiting.
fn flock(path: &Path, how: libc::c_int) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: the descriptor is this open file's, and flock takes only it
    // and flags.
    if unsafe { libc::flock(file.as_raw_fd(), how | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rekey_check_and_tenant_create_and_remove_run_alone() {
        for (work, sharing) in [
            (Work::Serve, Sharing::Serve),
            (Work::TenantCreate, Sharing::Alone),
            (Work::TenantList, Sharing::Beside),
            (Work::TenantSetRetention, Sharing::Beside),
            (Work::TenantRemove, Sharing::Alone),
            (Work::KeyCreate, Sharing::Beside),
            (Work::KeyList, Sharing::Beside),
            (Work::KeyRevoke, Sharing::Beside),
            (Work::RekeyTenant, Sharing::Alone),
            (Work::RekeyMaster, Sharing::Alone),
            (Work::AuditExport, Sharing::Beside),
            (Work::Check, Sharing::Alone),
        ] {
            assert_eq!(work.sharing(), sharing, "{work:?}");
        }
    }

    /// Each pair: what holds the directory, what asks for it next, and
    /// whether that is allowed.
    #[test]
    fn who_may_run_beside_whom() {
        use Sharing::{Alone, Beside, Serve};
        for (first, second, allowed) in [
            (Serve, Serve, false),
            (Serve, Alone, false),
            (Serve, Beside, true),
            (Alone, Serve, false),
            (Alone, Alone, false),
            (Alone, Beside, false),
            (Beside, Serve, true),
            (Beside, Alone, false),
            (Beside, Beside, true),
        ] {
            let dir = tempfile::tempdir().expect("temp dir");
            let held = lock(dir.path(), first).expect("the first lock");
            match lock(dir.path(), second) {
                Ok(_) => assert!(allowed, "{second:?} beside {first:?}"),
                Err(LockError::InUse(asked)) => {
                    assert!(!allowed, "{second:?} refused beside {first:?}");
                    assert_eq!(asked, second);
                }
                Err(e) => panic!("{first:?} then {second:?}: {e}"),
            }
            drop(held);
            assert!(lock(dir.path(), second).is_ok(), "{second:?} once free");
        }
    }

    /// A refused command that runs alone leaves nothing held: a server can
    /// still start once the beside command ends.
    #[test]
    fn a_refused_lock_holds_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let beside = lock(dir.path(), Sharing::Beside).expect("beside");
        assert!(lock(dir.path(), Sharing::Alone).is_err());
        let server = lock(dir.path(), Sharing::Serve).expect("serve beside");
        drop((beside, server));
        assert!(lock(dir.path(), Sharing::Alone).is_ok());
    }

    #[test]
    fn each_refusal_says_what_holds_the_directory() {
        for (sharing, words) in [
            (Sharing::Serve, "another vcon-server or a command"),
            (Sharing::Alone, "stop the server"),
            (Sharing::Beside, "runs alone"),
        ] {
            let text = LockError::InUse(sharing).to_string();
            assert!(text.contains(words), "{sharing:?}: {text}");
        }
    }

    #[test]
    fn a_data_directory_that_cannot_hold_a_lock_file_is_an_error() {
        let missing = Path::new("/nonexistent/vcon-server-data");
        for sharing in [Sharing::Serve, Sharing::Alone, Sharing::Beside] {
            assert!(matches!(lock(missing, sharing), Err(LockError::Io(_))));
        }
    }
}
