//! The master key file's backend: what it writes is what the envelope wrote
//! before the trait, it opens what older releases wrote, and anything it
//! did not wrap for this tenant under this master key is refused.

use std::io;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use uuid::Uuid;

use super::{FileWrap, KeyWrap, WrapError, WrappedKek};
use crate::envelope::{self, Context, FORMAT, Key, SEALED_OVERHEAD};
use crate::ids::{TenantId, VersionId};

const TENANT: TenantId = TenantId([7; 16]);

/// A directory holding one master key file of 32 random bytes, mode 0600.
fn master_file() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("master.key");
    let bytes: [u8; 32] = aws_lc_rs::rand::generate(&aws_lc_rs::rand::SystemRandom::new())
        .expect("random bytes")
        .expose();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("create the key file")
        .write_all(&bytes)
        .expect("write the key file");
    (dir, path)
}

fn load(path: &Path) -> Key {
    Key::load_master(path).expect("a master key file")
}

/// The backend over the key in `path`, with a second copy of the same key
/// for the envelope to use directly.
fn backend_and_master(path: &Path) -> (FileWrap, Key) {
    (FileWrap::new(load(path)), load(path))
}

fn key() -> Key {
    Key::generate().expect("a key")
}

fn tenant_ctx(tenant: TenantId) -> Context {
    Context::Tenant { tenant }
}

fn is_damaged(result: Result<Key, WrapError>) -> bool {
    matches!(result, Err(WrapError::Damaged))
}

// Success.

#[test]
fn a_wrapped_key_opens_with_the_envelope_as_before() {
    let (_dir, path) = master_file();
    let (backend, master) = backend_and_master(&path);
    let tenant_key = key();
    let wrapped = backend.wrap(TENANT, &tenant_key).expect("wraps").key;
    let opened = envelope::unwrap(&master, &tenant_ctx(TENANT), &wrapped)
        .expect("the envelope opens what the backend wrapped");
    assert_eq!(opened.id(), tenant_key.id(), "the same tenant key");
}

#[test]
fn a_key_the_envelope_wrapped_unwraps_through_the_backend() {
    let (_dir, path) = master_file();
    let (backend, master) = backend_and_master(&path);
    let tenant_key = key();
    // What every release before the trait wrote into a tenant's row.
    let wrapped = envelope::wrap(&master, &tenant_ctx(TENANT), &tenant_key);
    let opened = backend
        .unwrap(TENANT, &wrapped, None)
        .expect("the backend opens what the envelope wrapped");
    assert_eq!(opened.id(), tenant_key.id(), "the same tenant key");
}

#[test]
fn a_wrapped_key_unwraps_to_the_same_key() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let tenant_key = key();
    let wrapped = backend.wrap(TENANT, &tenant_key).expect("wraps").key;
    let opened = backend.unwrap(TENANT, &wrapped, None).expect("unwraps");
    assert_eq!(opened.id(), tenant_key.id(), "the same tenant key");
}

#[test]
fn the_key_id_is_the_master_keys() {
    let (_dir, path) = master_file();
    let (backend, master) = backend_and_master(&path);
    assert_eq!(
        backend.key_id(),
        master.id(),
        "wraps are counted against the master key"
    );
}

#[test]
fn the_root_is_the_master_key() {
    let (_dir, path) = master_file();
    let (backend, master) = backend_and_master(&path);
    assert_eq!(
        backend.root().id(),
        master.id(),
        "the root is the master key"
    );
}

#[test]
fn the_server_catalog_key_is_the_one_derived_from_the_master_key() {
    let (_dir, path) = master_file();
    let (backend, master) = backend_and_master(&path);
    let from_root = envelope::catalog_key(backend.root()).expect("catalog key");
    let from_master = envelope::catalog_key(&master).expect("catalog key");
    assert_eq!(
        from_root.id(),
        from_master.id(),
        "server.sqlite opens under the same key as before"
    );
}

#[test]
fn two_wraps_of_one_key_differ_and_both_unwrap() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let tenant_key = key();
    let first = backend.wrap(TENANT, &tenant_key).expect("wraps").key;
    let second = backend.wrap(TENANT, &tenant_key).expect("wraps").key;
    assert_ne!(first, second, "each wrap draws its own nonce");
    for wrapped in [&first, &second] {
        let opened = backend.unwrap(TENANT, wrapped, None).expect("unwraps");
        assert_eq!(opened.id(), tenant_key.id(), "the same tenant key");
    }
}

#[test]
fn a_wrapped_key_has_the_envelopes_layout() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let wrapped = backend.wrap(TENANT, &key()).expect("wraps").key;
    assert_eq!(
        wrapped.len(),
        32 + SEALED_OVERHEAD,
        "a 32-byte key, the format byte, the nonce and the tag"
    );
    assert_eq!(wrapped[0], FORMAT, "the envelope's format byte");
}

#[test]
fn the_backend_wraps_and_unwraps_from_several_threads() {
    let (_dir, path) = master_file();
    let backend = Arc::new(FileWrap::new(load(&path)));
    let threads: Vec<_> = (0..4u8)
        .map(|n| {
            let backend = backend.clone();
            std::thread::spawn(move || {
                let tenant = TenantId([n; 16]);
                let tenant_key = key();
                let wrapped = backend.wrap(tenant, &tenant_key).expect("wraps").key;
                let opened = backend.unwrap(tenant, &wrapped, None).expect("unwraps");
                opened.id() == tenant_key.id()
            })
        })
        .collect();
    for thread in threads {
        assert!(
            thread.join().expect("the thread ends"),
            "the same tenant key"
        );
    }
}

#[test]
fn the_key_id_is_the_first_8_bytes_of_the_files_sha256() {
    let (_dir, path) = master_file();
    let bytes = std::fs::read(&path).expect("read the key file");
    let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &bytes);
    let backend = FileWrap::new(load(&path));
    assert_eq!(
        backend.key_id().0[..],
        hash.as_ref()[..8],
        "the id `check` and the rotation commands print"
    );
}

// Failure.

#[test]
fn a_key_wrapped_under_another_master_is_damaged() {
    let (_dir, path) = master_file();
    let (_other_dir, other_path) = master_file();
    let backend = FileWrap::new(load(&path));
    let wrapped = FileWrap::new(load(&other_path))
        .wrap(TENANT, &key())
        .expect("wraps")
        .key;
    assert!(
        is_damaged(backend.unwrap(TENANT, &wrapped, None)),
        "another master key's wrap"
    );
}

#[test]
fn a_key_wrapped_for_another_tenant_is_damaged() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let wrapped = backend.wrap(TENANT, &key()).expect("wraps").key;
    assert!(
        is_damaged(backend.unwrap(TenantId([8; 16]), &wrapped, None)),
        "a row's key moved to another tenant does not open"
    );
}

#[test]
fn an_empty_blob_is_damaged() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    assert!(
        is_damaged(backend.unwrap(TENANT, &[], None)),
        "an empty blob"
    );
}

#[test]
fn every_shorter_blob_is_damaged() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let wrapped = backend.wrap(TENANT, &key()).expect("wraps").key;
    for len in 0..wrapped.len() {
        assert!(
            is_damaged(backend.unwrap(TENANT, &wrapped[..len], None)),
            "a blob cut to {len} bytes"
        );
    }
}

#[test]
fn changing_any_byte_is_damaged() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let wrapped = backend.wrap(TENANT, &key()).expect("wraps").key;
    for i in 0..wrapped.len() {
        let mut changed = wrapped.clone();
        changed[i] ^= 0x01;
        assert!(
            is_damaged(backend.unwrap(TENANT, &changed, None)),
            "byte {i} changed"
        );
    }
}

#[test]
fn a_blob_holding_other_than_32_bytes_is_damaged() {
    let (_dir, path) = master_file();
    let (backend, master) = backend_and_master(&path);
    for len in [0, 16, 31, 33, 64] {
        let sealed = envelope::seal(&master, &tenant_ctx(TENANT), &vec![5; len]);
        assert!(
            is_damaged(backend.unwrap(TENANT, &sealed, None)),
            "a sealed value of {len} bytes is not a key"
        );
    }
}

#[test]
fn a_blob_of_another_format_is_damaged() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let mut wrapped = backend.wrap(TENANT, &key()).expect("wraps").key;
    for format in [0, 2, 0xff] {
        wrapped[0] = format;
        assert!(
            is_damaged(backend.unwrap(TENANT, &wrapped, None)),
            "format byte {format}"
        );
    }
}

#[test]
fn a_data_key_wrapped_for_a_version_is_damaged() {
    let (_dir, path) = master_file();
    let (backend, master) = backend_and_master(&path);
    let version = Context::Version {
        tenant: TENANT,
        uuid: Uuid::from_bytes([3; 16]),
        version: VersionId([4; 16]),
    };
    let wrapped = envelope::wrap(&master, &version, &key());
    assert!(
        is_damaged(backend.unwrap(TENANT, &wrapped, None)),
        "a version's context is not a tenant's"
    );
}

#[test]
fn damaged_says_only_that_the_key_did_not_open() {
    let error = WrapError::Damaged;
    assert_eq!(
        error.to_string(),
        "a wrapped tenant key did not open",
        "the text"
    );
    assert!(
        std::error::Error::source(&error).is_none(),
        "no cause to give"
    );
}

#[test]
fn a_memory_error_gives_its_cause() {
    let error = WrapError::Memory(io::Error::from_raw_os_error(libc::ENOMEM));
    assert_eq!(
        error.to_string(),
        "the key's memory could not be locked or kept out of core dumps",
        "the text"
    );
    let cause = std::error::Error::source(&error).expect("the I/O error");
    assert_eq!(
        cause.to_string(),
        io::Error::from_raw_os_error(libc::ENOMEM).to_string(),
        "the cause is the system's error"
    );
}

// Edge.

#[test]
fn debug_shows_the_key_id_only() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let id = backend.key_id().to_string();
    assert_eq!(
        format!("{backend:?}"),
        format!("FileWrap {{ master: Key {{ id: {id}, .. }} }}"),
        "no byte of the key"
    );
}

#[test]
fn the_all_zero_tenant_id_is_bound_like_any_other() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let zero = TenantId([0; 16]);
    let wrapped = backend.wrap(zero, &key()).expect("wraps").key;
    assert!(
        backend.unwrap(zero, &wrapped, None).is_ok(),
        "opens for its own tenant"
    );
    let mut one = [0; 16];
    one[15] = 1;
    assert!(
        is_damaged(backend.unwrap(TenantId(one), &wrapped, None)),
        "not for a tenant one bit away"
    );
}

#[test]
fn a_key_wrapped_before_a_restart_unwraps_after_it() {
    let (_dir, path) = master_file();
    let tenant_key = key();
    let wrapped = FileWrap::new(load(&path))
        .wrap(TENANT, &tenant_key)
        .expect("wraps")
        .key;
    // The master key read again from its file, as the next start does.
    let opened = FileWrap::new(load(&path))
        .unwrap(TENANT, &wrapped, None)
        .expect("unwraps after a restart");
    assert_eq!(opened.id(), tenant_key.id(), "the same tenant key");
}

#[test]
fn the_root_is_the_same_key_at_every_call() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    assert!(
        std::ptr::eq(backend.root(), backend.root()),
        "one root, never a copy per call"
    );
}

#[test]
fn a_tenant_ids_byte_order_is_bound() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let mut bytes = [0; 16];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::try_from(i).expect("small");
    }
    let mut reversed = bytes;
    reversed.reverse();
    let wrapped = backend.wrap(TenantId(bytes), &key()).expect("wraps").key;
    assert!(
        is_damaged(backend.unwrap(TenantId(reversed), &wrapped, None)),
        "the same bytes in another order are another tenant"
    );
}

#[test]
fn a_blob_with_bytes_appended_is_damaged() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let mut wrapped = backend.wrap(TENANT, &key()).expect("wraps").key;
    wrapped.push(0);
    assert!(
        is_damaged(backend.unwrap(TENANT, &wrapped, None)),
        "one byte more"
    );
}

#[test]
fn an_unwrapped_key_outlives_its_backend() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let tenant_key = key();
    let wrapped = backend.wrap(TENANT, &tenant_key).expect("wraps").key;
    let opened = backend.unwrap(TENANT, &wrapped, None).expect("unwraps");
    drop(backend);
    // The key is in a page of its own: it still wraps and unwraps.
    let inner = envelope::wrap(&opened, &tenant_ctx(TENANT), &tenant_key);
    let again = envelope::unwrap(&opened, &tenant_ctx(TENANT), &inner).expect("still works");
    assert_eq!(again.id(), tenant_key.id(), "the unwrapped key is whole");
}

/// A key-encryption key as a key backend would have recorded it.
fn backend_kek() -> WrappedKek {
    WrappedKek {
        wrap_ref: "pkcs11:vcon-1".to_owned(),
        blob: vec![1, 6, 0, 7],
    }
}

#[test]
fn the_file_backend_records_no_key_encryption_key() {
    let (_dir, path) = master_file();
    let wrapped = FileWrap::new(load(&path))
        .wrap(TENANT, &key())
        .expect("wraps");
    assert_eq!(
        wrapped.kek, None,
        "the master key file wraps the key itself"
    );
}

#[test]
fn a_key_under_a_key_backend_is_elsewhere_for_the_file_backend() {
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let result = backend.unwrap(TENANT, &[1, 2, 3], Some(&backend_kek()));
    assert!(
        matches!(
            &result,
            Err(WrapError::Elsewhere { wrap_ref: Some(r) }) if r == "pkcs11:vcon-1"
        ),
        "the key is under the backend's key: {result:?}"
    );
}

#[test]
fn a_key_that_would_open_is_still_elsewhere_when_a_kek_is_named() {
    // The row names a key-encryption key, so the file backend opens
    // nothing, even bytes its master key would open.
    let (_dir, path) = master_file();
    let backend = FileWrap::new(load(&path));
    let wrapped = backend.wrap(TENANT, &key()).expect("wraps").key;
    let result = backend.unwrap(TENANT, &wrapped, Some(&backend_kek()));
    assert!(
        matches!(result, Err(WrapError::Elsewhere { wrap_ref: Some(_) })),
        "a row that names a key-encryption key is not opened under the master key: {result:?}"
    );
}

#[test]
fn elsewhere_names_the_key_and_no_bytes() {
    let error = WrapError::Elsewhere {
        wrap_ref: Some("pkcs11:vcon-1".to_owned()),
    };
    assert_eq!(
        error.to_string(),
        "the key is wrapped under the key backend key pkcs11:vcon-1, not under this backend's key",
        "the key it is under"
    );
    assert!(
        std::error::Error::source(&error).is_none(),
        "nothing underneath"
    );
    let file = WrapError::Elsewhere { wrap_ref: None };
    assert_eq!(
        file.to_string(),
        "the key is wrapped under the master key file, not under the key backend's key",
        "the master key file"
    );
}

#[test]
fn a_blob_error_is_the_source_not_the_text() {
    let error = WrapError::Blob(super::blob::BlobError::UnknownFormat(9));
    assert_eq!(
        error.to_string(),
        "a key-wrap blob could not be read or made",
        "the error's own text"
    );
    assert_eq!(
        std::error::Error::source(&error).map(ToString::to_string),
        Some("key-wrap format 9 is not one this build knows".to_owned()),
        "the format is named underneath"
    );
}
