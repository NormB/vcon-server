//! The master key is read from a file that must hold exactly 32 bytes.
//! The expected ids were computed with sha256sum.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use vcon_server::envelope::{Key, KeyError};

fn file(dir: &tempfile::TempDir, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join("master.key");
    fs::write(&path, bytes).expect("write the key file");
    path
}

#[test]
fn a_32_byte_file_loads_and_its_id_starts_its_sha_256() {
    let dir = tempfile::tempdir().expect("temp dir");
    let counting: Vec<u8> = (0..32).collect();
    let key = Key::load_master(&file(&dir, &counting)).expect("loads");
    assert_eq!(key.id().to_string(), "630dcd2966c43366");
    let key = Key::load_master(&file(&dir, &[0; 32])).expect("loads");
    assert_eq!(key.id().to_string(), "66687aadf862bd77");
}

#[test]
fn a_file_of_any_other_size_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    for len in [0, 1, 31, 33, 64, 4096] {
        let path = file(&dir, &vec![7; len]);
        assert!(
            matches!(Key::load_master(&path), Err(KeyError::WrongSize)),
            "{len} bytes"
        );
    }
}

#[test]
fn a_missing_or_unreadable_file_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("absent.key");
    assert!(
        matches!(Key::load_master(&missing), Err(KeyError::Read(e)) if e.kind() == std::io::ErrorKind::NotFound),
        "missing"
    );
    assert!(
        matches!(Key::load_master(dir.path()), Err(KeyError::Read(_))),
        "a directory"
    );
    let locked = file(&dir, &[7; 32]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    // SAFETY: geteuid has no preconditions.
    let euid = unsafe { libc::geteuid() };
    assert_ne!(euid, 0, "root reads any file; run as another user");
    assert!(
        matches!(Key::load_master(&locked), Err(KeyError::Read(e)) if e.kind() == std::io::ErrorKind::PermissionDenied),
        "no read permission"
    );
}
