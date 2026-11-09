//! The master key is read from a file that must hold exactly 32 bytes.
//! The expected ids were computed with sha256sum.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use vcon_server::envelope::{Key, KeyError};

fn file(dir: &tempfile::TempDir, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join("master.key");
    fs::write(&path, bytes).expect("write the key file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("chmod");
    path
}

fn with_mode(dir: &tempfile::TempDir, mode: u32) -> PathBuf {
    let path = file(dir, &[7; 32]);
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("chmod");
    path
}

/// The refusal's text, or a panic naming what came back instead.
fn refusal(path: &std::path::Path) -> String {
    match Key::load_master(path) {
        Err(KeyError::Refused(text)) => text,
        Err(other) => panic!("not a refusal: {other:?}"),
        Ok(_) => panic!("loaded"),
    }
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
        matches!(Key::load_master(dir.path()), Err(KeyError::Refused(t)) if t.ends_with("not a regular file")),
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

// The one secret-file rule (crate::secret_file), on the master key.

#[test]
fn a_0644_master_key_is_refused_with_the_chmod_to_run() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = with_mode(&dir, 0o644);
    let text = refusal(&path);
    assert!(
        text.starts_with(&format!(
            "master_key {}: other users may read or change the file (mode 0644)",
            path.display()
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("run `chmod 600 {}`", path.display())),
        "{text}"
    );
}

#[test]
fn a_0604_master_key_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(
        refusal(&with_mode(&dir, 0o604)).contains("(mode 0604)"),
        "others may read a 0604 key"
    );
}

#[test]
fn a_0660_master_key_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(
        refusal(&with_mode(&dir, 0o660)).contains("(mode 0660)"),
        "the group may replace a 0660 key"
    );
}

#[test]
fn a_0640_master_key_loads() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(
        Key::load_master(&with_mode(&dir, 0o640)).is_ok(),
        "0640 lets only the group read the key"
    );
}

#[test]
fn a_0400_master_key_loads() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(
        Key::load_master(&with_mode(&dir, 0o400)).is_ok(),
        "0400 is closed to all but the owner"
    );
}

#[test]
fn a_symlink_to_a_0600_master_key_loads() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = with_mode(&dir, 0o600);
    let link = dir.path().join("link.key");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    assert!(
        Key::load_master(&link).is_ok(),
        "a symlink's target is the file checked, and it is 0600"
    );
}

#[test]
fn a_refused_master_key_displays_its_reason() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = with_mode(&dir, 0o644);
    let err = Key::load_master(&path).expect_err("refused");
    assert!(
        err.to_string()
            .starts_with(&format!("master_key {}: other users", path.display())),
        "{err}"
    );
}

#[test]
fn a_master_key_of_another_name_is_refused_under_that_name() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = with_mode(&dir, 0o644);
    match Key::load_master_named(&path, "--new-master") {
        Err(KeyError::Refused(text)) => assert!(text.starts_with("--new-master "), "{text}"),
        other => panic!("{:?}", other.err()),
    }
}
