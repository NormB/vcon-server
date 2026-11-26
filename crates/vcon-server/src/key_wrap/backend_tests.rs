//! The backend over a key backend: the root is a random key the backend
//! wraps, each tenant key is wrapped locally under a key-encryption key of
//! its own, bound to the tenant, and the backend wraps that key. A row
//! that names another key is refused without asking the backend.

use std::sync::Arc;

use super::blob::{BackendKind, Blob, BlobError, MAX_CIPHERTEXT, WrapRef};
use super::testing::{Call, Fail, Scripted};
use super::{BackendWrap, KeyWrap, WrapContext, WrapError, Wrapped, WrappedKek};
use crate::envelope::{self, Context, FORMAT, Key};
use crate::ids::TenantId;

const TENANT: TenantId = TenantId([7; 16]);
const OTHER: TenantId = TenantId([8; 16]);

fn scripted() -> Scripted {
    Scripted::new(BackendKind::Pkcs11, "vcon-1")
}

/// A backend over a new root, and its root blob.
fn made(backend: &Scripted) -> (BackendWrap<Scripted>, Vec<u8>) {
    BackendWrap::create(backend.clone()).expect("the root is made")
}

fn key() -> Key {
    Key::generate().expect("a key")
}

fn kek_of(wrapped: &Wrapped) -> &WrappedKek {
    wrapped.kek.as_ref().expect("a key-encryption key")
}

/// A key-encryption key recorded with another reference than its blob's.
fn renamed(kek: &WrappedKek, wrap_ref: &str) -> WrappedKek {
    WrappedKek {
        wrap_ref: wrap_ref.to_owned(),
        blob: kek.blob.clone(),
    }
}

// Success.

#[test]
fn a_root_made_then_opened_is_the_same_key() {
    let backend = scripted();
    let (wrap, blob) = made(&backend);
    let opened = BackendWrap::open(backend.clone(), &blob).expect("the root opens");
    assert_eq!(
        opened.root().id(),
        wrap.root().id(),
        "the same root after an open"
    );
}

#[test]
fn a_wrapped_tenant_key_unwraps_to_the_same_key() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let tenant_key = key();
    let wrapped = wrap.wrap(TENANT, &tenant_key).expect("wraps");
    let opened = wrap
        .unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref())
        .expect("unwraps");
    assert_eq!(opened.id(), tenant_key.id(), "the same tenant key");
}

#[test]
fn the_key_encryption_key_names_the_backends_key() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    let kek = kek_of(&wrapped);
    assert_eq!(kek.wrap_ref, "pkcs11:vcon-1", "the reference as text");
    let blob = Blob::decode(&kek.blob).expect("a key-wrap blob");
    assert_eq!(
        blob.reference(),
        &WrapRef::new(BackendKind::Pkcs11, "vcon-1").expect("a reference"),
        "the blob names the same key"
    );
}

#[test]
fn the_tenant_key_is_wrapped_locally_under_its_key_encryption_key() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let tenant_key = key();
    let wrapped = wrap.wrap(TENANT, &tenant_key).expect("wraps");
    let blob = Blob::decode(&kek_of(&wrapped).blob).expect("a key-wrap blob");
    let kek = super::Backend::unwrap(&backend, WrapContext::Tenant(TENANT), &blob)
        .expect("the backend unwraps the key-encryption key");
    let opened = envelope::unwrap(&kek, &Context::Tenant { tenant: TENANT }, &wrapped.key)
        .expect("the envelope opens the tenant key, bound to the tenant");
    assert_eq!(opened.id(), tenant_key.id(), "under the key-encryption key");
}

#[test]
fn each_wrap_makes_a_new_key_encryption_key() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let tenant_key = key();
    let first = wrap.wrap(TENANT, &tenant_key).expect("wraps");
    let second = wrap.wrap(TENANT, &tenant_key).expect("wraps");
    assert_ne!(kek_of(&first).blob, kek_of(&second).blob, "two keys");
    for wrapped in [&first, &second] {
        let opened = wrap
            .unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref())
            .expect("unwraps");
        assert_eq!(opened.id(), tenant_key.id(), "each opens");
    }
}

#[test]
fn the_backend_is_asked_with_the_tenants_context() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    backend.clear_calls();
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    wrap.unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref())
        .expect("unwraps");
    assert_eq!(
        backend.calls(),
        vec![
            Call::Wrap(WrapContext::Tenant(TENANT)),
            Call::Unwrap(WrapContext::Tenant(TENANT)),
        ],
        "one wrap and one unwrap, for the tenant"
    );
}

#[test]
fn the_root_is_wrapped_once_and_unwrapped_once_with_the_registry_context() {
    let backend = scripted();
    let (_, blob) = made(&backend);
    assert_eq!(
        backend.calls(),
        vec![Call::Wrap(WrapContext::Registry)],
        "create wraps the root"
    );
    backend.clear_calls();
    BackendWrap::open(backend.clone(), &blob).expect("opens");
    assert_eq!(
        backend.calls(),
        vec![Call::Unwrap(WrapContext::Registry)],
        "open unwraps the root"
    );
}

#[test]
fn the_root_blob_is_a_key_wrap_blob_of_the_backends_key() {
    let backend = scripted();
    let (_, blob) = made(&backend);
    assert_eq!(blob[0], super::blob::FORMAT, "key-wrap format 1");
    let decoded = Blob::decode(&blob).expect("a key-wrap blob");
    assert_eq!(
        decoded.reference().to_string(),
        "pkcs11:vcon-1",
        "the backend's key"
    );
}

#[test]
fn the_key_id_is_the_roots() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    assert_eq!(
        wrap.key_id(),
        wrap.root().id(),
        "wraps count against the root"
    );
}

#[test]
fn the_server_catalog_key_derives_from_the_root_the_backend_holds() {
    let backend = scripted();
    let (wrap, blob) = made(&backend);
    let decoded = Blob::decode(&blob).expect("a key-wrap blob");
    let root = super::Backend::unwrap(&backend, WrapContext::Registry, &decoded)
        .expect("the backend unwraps the root");
    assert_eq!(
        envelope::catalog_key(wrap.root()).expect("a key").id(),
        envelope::catalog_key(&root).expect("a key").id(),
        "the catalog key of the root in the blob"
    );
}

#[test]
fn the_backend_wraps_and_unwraps_from_several_threads() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrap = Arc::new(wrap);
    let threads: Vec<_> = (0..4_u8)
        .map(|n| {
            let wrap = wrap.clone();
            std::thread::spawn(move || {
                let tenant = TenantId([n; 16]);
                let tenant_key = key();
                let wrapped = wrap.wrap(tenant, &tenant_key).expect("wraps");
                let opened = wrap
                    .unwrap(tenant, &wrapped.key, wrapped.kek.as_ref())
                    .expect("unwraps");
                opened.id() == tenant_key.id()
            })
        })
        .collect();
    for thread in threads {
        assert!(thread.join().expect("the thread"), "each thread's key");
    }
}

#[test]
fn debug_shows_the_key_and_the_roots_id_only() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let text = format!("{wrap:?}");
    assert!(text.contains("vcon-1"), "the backend's key: {text}");
    assert!(
        text.contains(&wrap.root().id().to_string()),
        "the root's id: {text}"
    );
}

// Failure.

#[test]
fn a_key_under_the_master_key_file_is_elsewhere_and_asks_nothing() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    backend.clear_calls();
    let result = wrap.unwrap(TENANT, &[1, 2, 3], None);
    assert!(
        matches!(result, Err(WrapError::Elsewhere { wrap_ref: None })),
        "under the master key file: {result:?}"
    );
    assert_eq!(backend.calls(), Vec::new(), "no backend call");
}

#[test]
fn a_key_under_another_backend_key_is_elsewhere_and_asks_nothing() {
    let backend = scripted();
    let other = Scripted::new(BackendKind::Pkcs11, "vcon-2");
    let (wrap, _) = made(&backend);
    let (theirs, _) = made(&other);
    let wrapped = theirs.wrap(TENANT, &key()).expect("wraps");
    backend.clear_calls();
    let result = wrap.unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref());
    assert!(
        matches!(&result, Err(WrapError::Elsewhere { wrap_ref: Some(r) }) if r == "pkcs11:vcon-2"),
        "under the other key: {result:?}"
    );
    assert_eq!(backend.calls(), Vec::new(), "no backend call");
}

#[test]
fn a_key_of_another_backend_kind_is_elsewhere_and_asks_nothing() {
    let backend = scripted();
    let other = Scripted::new(BackendKind::VaultTransit, "transit/vcon-1");
    let (wrap, _) = made(&backend);
    let (theirs, _) = made(&other);
    let wrapped = theirs.wrap(TENANT, &key()).expect("wraps");
    backend.clear_calls();
    let result = wrap.unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref());
    assert!(
        matches!(&result, Err(WrapError::Elsewhere { wrap_ref: Some(r) }) if r == "vault-transit:transit/vcon-1"),
        "under the other kind: {result:?}"
    );
    assert_eq!(backend.calls(), Vec::new(), "no backend call");
}

#[test]
fn a_reference_that_disagrees_with_its_blob_is_damaged_and_asks_nothing() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    backend.clear_calls();
    let kek = renamed(kek_of(&wrapped), "pkcs11:vcon-2");
    let result = wrap.unwrap(TENANT, &wrapped.key, Some(&kek));
    assert!(
        matches!(result, Err(WrapError::Damaged)),
        "a row whose two columns disagree: {result:?}"
    );
    assert_eq!(backend.calls(), Vec::new(), "no backend call");
}

#[test]
fn a_blob_of_an_unknown_format_is_refused_with_the_format_named() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    let mut kek = kek_of(&wrapped).clone();
    kek.blob[0] = 2;
    backend.clear_calls();
    let result = wrap.unwrap(TENANT, &wrapped.key, Some(&kek));
    assert!(
        matches!(result, Err(WrapError::Blob(BlobError::UnknownFormat(2)))),
        "format 2: {result:?}"
    );
    assert_eq!(backend.calls(), Vec::new(), "no backend call");
}

#[test]
fn a_backend_that_refuses_the_unwrap_gives_damaged() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    backend.fail_unwraps(&[Some(Fail::Damaged)]);
    let result = wrap.unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref());
    assert!(matches!(result, Err(WrapError::Damaged)), "{result:?}");
}

#[test]
fn a_backend_memory_error_is_passed_on() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    backend.fail_unwraps(&[Some(Fail::Memory)]);
    let result = wrap.unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref());
    assert!(matches!(result, Err(WrapError::Memory(_))), "{result:?}");
}

#[test]
fn a_backend_that_refuses_the_wrap_gives_no_key() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    backend.fail_wraps(&[Some(Fail::Damaged)]);
    let result = wrap.wrap(TENANT, &key());
    assert!(matches!(result, Err(WrapError::Damaged)), "{result:?}");
}

#[test]
fn a_key_encryption_key_moved_to_another_tenant_does_not_open() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    let result = wrap.unwrap(OTHER, &wrapped.key, wrapped.kek.as_ref());
    assert!(
        matches!(result, Err(WrapError::Damaged)),
        "another tenant's row: {result:?}"
    );
}

#[test]
fn a_tenant_key_moved_beside_another_tenants_kek_does_not_open() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let ours = wrap.wrap(TENANT, &key()).expect("wraps");
    let theirs = wrap.wrap(OTHER, &key()).expect("wraps");
    let result = wrap.unwrap(TENANT, &theirs.key, ours.kek.as_ref());
    assert!(
        matches!(result, Err(WrapError::Damaged)),
        "a key under another key-encryption key: {result:?}"
    );
}

#[test]
fn a_root_blob_of_another_backend_key_is_elsewhere_and_asks_nothing() {
    let other = Scripted::new(BackendKind::Pkcs11, "vcon-2");
    let (_, blob) = made(&other);
    let backend = scripted();
    let result = BackendWrap::open(backend.clone(), &blob);
    assert!(
        matches!(&result, Err(WrapError::Elsewhere { wrap_ref: Some(r) }) if r == "pkcs11:vcon-2"),
        "the root is under the other key: {:?}",
        result.err()
    );
    assert_eq!(backend.calls(), Vec::new(), "no backend call");
}

#[test]
fn a_root_the_backend_does_not_open_is_damaged() {
    let backend = scripted();
    let (_, blob) = made(&backend);
    backend.fail_unwraps(&[Some(Fail::Damaged)]);
    let result = BackendWrap::open(backend.clone(), &blob);
    assert!(
        matches!(result, Err(WrapError::Damaged)),
        "{:?}",
        result.err()
    );
}

#[test]
fn an_empty_root_blob_is_refused_as_a_blob() {
    let result = BackendWrap::open(scripted(), &[]);
    assert!(
        matches!(result, Err(WrapError::Blob(BlobError::Empty))),
        "{:?}",
        result.err()
    );
}

#[test]
fn a_backend_ciphertext_too_long_for_a_blob_is_refused() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    backend.fail_wraps(&[Some(Fail::Ciphertext(MAX_CIPHERTEXT + 1))]);
    let result = wrap.wrap(TENANT, &key());
    assert!(
        matches!(result, Err(WrapError::Blob(BlobError::Ciphertext))),
        "{result:?}"
    );
}

#[test]
fn a_root_the_backend_does_not_wrap_is_not_made() {
    let backend = scripted();
    backend.fail_wraps(&[Some(Fail::Memory)]);
    let result = BackendWrap::create(backend.clone());
    assert!(
        matches!(result, Err(WrapError::Memory(_))),
        "{:?}",
        result.err()
    );
}

// Edge.

#[test]
fn a_blob_with_bytes_after_its_end_is_refused() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    let mut kek = kek_of(&wrapped).clone();
    kek.blob.push(0);
    let result = wrap.unwrap(TENANT, &wrapped.key, Some(&kek));
    assert!(
        matches!(result, Err(WrapError::Blob(BlobError::TrailingBytes))),
        "{result:?}"
    );
}

#[test]
fn the_all_zero_tenant_id_is_bound_like_any_other() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let zero = TenantId([0; 16]);
    let wrapped = wrap.wrap(zero, &key()).expect("wraps");
    assert!(
        wrap.unwrap(zero, &wrapped.key, wrapped.kek.as_ref())
            .is_ok(),
        "the zero id opens its own"
    );
    assert!(
        matches!(
            wrap.unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref()),
            Err(WrapError::Damaged)
        ),
        "and not another's"
    );
}

#[test]
fn a_key_wrapped_before_a_restart_unwraps_after_it() {
    let backend = scripted();
    let (wrap, blob) = made(&backend);
    let tenant_key = key();
    let wrapped = wrap.wrap(TENANT, &tenant_key).expect("wraps");
    drop(wrap);
    let again = BackendWrap::open(backend.restarted(), &blob).expect("the root opens again");
    let opened = again
        .unwrap(TENANT, &wrapped.key, wrapped.kek.as_ref())
        .expect("unwraps after a restart");
    assert_eq!(opened.id(), tenant_key.id(), "the same tenant key");
}

#[test]
fn the_same_reference_with_another_key_is_damaged() {
    // Two backends that name one key but hold different ones: the blob
    // reaches the backend, which does not open it.
    let backend = scripted();
    let (_, blob) = made(&backend);
    let result = BackendWrap::open(scripted(), &blob);
    assert!(
        matches!(result, Err(WrapError::Damaged)),
        "{:?}",
        result.err()
    );
}

#[test]
fn a_tenant_key_has_the_envelopes_layout() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    let file = envelope::wrap(&key(), &Context::Tenant { tenant: TENANT }, &key());
    assert_eq!(wrapped.key[0], FORMAT, "envelope format 1");
    assert_eq!(wrapped.key.len(), file.len(), "the length of a file wrap");
}

#[test]
fn two_tenants_keks_each_open_only_their_own() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let a = wrap.wrap(TENANT, &key()).expect("wraps");
    let b = wrap.wrap(OTHER, &key()).expect("wraps");
    assert_ne!(kek_of(&a).blob, kek_of(&b).blob, "a key each");
    assert!(
        wrap.unwrap(TENANT, &a.key, a.kek.as_ref()).is_ok()
            && wrap.unwrap(OTHER, &b.key, b.kek.as_ref()).is_ok(),
        "each opens its own"
    );
}

#[test]
fn a_root_blob_given_as_a_tenants_kek_does_not_open() {
    let backend = scripted();
    let (wrap, blob) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    let kek = WrappedKek {
        wrap_ref: "pkcs11:vcon-1".to_owned(),
        blob,
    };
    let result = wrap.unwrap(TENANT, &wrapped.key, Some(&kek));
    assert!(
        matches!(result, Err(WrapError::Damaged)),
        "the registry context is not a tenant's: {result:?}"
    );
}

#[test]
fn a_tenants_kek_given_as_the_root_does_not_open() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    let wrapped = wrap.wrap(TENANT, &key()).expect("wraps");
    let result = BackendWrap::open(backend.clone(), &kek_of(&wrapped).blob);
    assert!(
        matches!(result, Err(WrapError::Damaged)),
        "a tenant's context is not the registry's: {:?}",
        result.err()
    );
}

#[test]
fn a_backend_ciphertext_of_the_largest_size_fits() {
    let backend = scripted();
    let (wrap, _) = made(&backend);
    backend.fail_wraps(&[Some(Fail::Ciphertext(MAX_CIPHERTEXT))]);
    let wrapped = wrap
        .wrap(TENANT, &key())
        .expect("the largest ciphertext fits");
    let blob = Blob::decode(&kek_of(&wrapped).blob).expect("a key-wrap blob");
    assert_eq!(blob.ciphertext().len(), MAX_CIPHERTEXT, "kept whole");
}
