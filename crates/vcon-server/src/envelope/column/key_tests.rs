//! A key sealed as a column value: what must work, what must be refused,
//! and the limits. An audit log's key is stored this way, under the keys
//! of the catalog it belongs to.

use super::{
    ColumnContext, ColumnKeys, Key, KeyError, LockedPage, REGISTRY, SEALED_COLUMN_OVERHEAD,
    open_column, seal_column, unwrap_key, wrap_key,
};
use crate::ids::TenantId;

fn key_of(bytes: [u8; 32]) -> Key {
    let mut page = LockedPage::new().expect("page");
    page.bytes_mut()[..32].copy_from_slice(&bytes);
    Key { page }
}

fn keys_of(byte: u8) -> ColumnKeys {
    ColumnKeys::derive(&key_of([byte; 32])).expect("derives")
}

fn keys() -> ColumnKeys {
    keys_of(0x44)
}

const TENANT: TenantId = TenantId([0x11; 16]);
const ROW: [u8; 8] = [0xaa; 8];

const CTX: ColumnContext<'static> = ColumnContext {
    tenant: TENANT,
    table: "audit_key",
    column: "wrapped",
    row: &ROW,
};

const SECRET: [u8; 32] = [0x77; 32];

fn wrapped() -> Vec<u8> {
    wrap_key(&keys(), &CTX, &key_of(SECRET)).expect("wraps")
}

fn refused(result: Result<Key, KeyError>, what: &str) {
    assert!(
        matches!(result, Err(KeyError::Damaged)),
        "{what}: {:?}",
        result.map(|key| key.id())
    );
}

// What must work.

#[test]
fn a_wrapped_key_unwraps_to_the_same_key() {
    let key = unwrap_key(&keys(), &CTX, &wrapped()).expect("unwraps");
    assert_eq!(key.id(), key_of(SECRET).id(), "the same key, by its id");
    assert_eq!(key.bytes(), SECRET, "and by its bytes");
}

#[test]
fn a_wrapped_key_is_the_key_s_length_and_the_overhead_of_a_sealed_column() {
    assert_eq!(
        wrapped().len(),
        32 + SEALED_COLUMN_OVERHEAD,
        "the format byte, the nonce, 32 bytes and the tag"
    );
}

#[test]
fn a_wrapped_key_is_a_sealed_column_value_of_the_key_s_bytes() {
    assert_eq!(
        open_column(&keys(), &CTX, &wrapped()).expect("opens"),
        SECRET,
        "the column codec opens it to the key"
    );
}

#[test]
fn keys_derived_again_from_the_same_catalog_key_unwrap_it() {
    let blob = wrapped();
    let again = keys_of(0x44);
    assert_eq!(
        unwrap_key(&again, &CTX, &blob).expect("unwraps").id(),
        key_of(SECRET).id(),
        "the keys are derived, never stored"
    );
}

#[test]
fn a_registry_context_wraps_and_unwraps_a_key_too() {
    let ctx = ColumnContext {
        tenant: REGISTRY,
        ..CTX
    };
    let blob = wrap_key(&keys(), &ctx, &key_of(SECRET)).expect("wraps");
    assert_eq!(
        unwrap_key(&keys(), &ctx, &blob).expect("unwraps").id(),
        key_of(SECRET).id(),
        "the registry's tenant field is a context like another"
    );
}

// What must be refused.

#[test]
fn the_keys_of_another_catalog_key_do_not_unwrap_it() {
    refused(
        unwrap_key(&keys_of(0x45), &CTX, &wrapped()),
        "another catalog key",
    );
}

#[test]
fn another_row_does_not_unwrap_it() {
    let ctx = ColumnContext {
        row: &[0xab; 8],
        ..CTX
    };
    refused(
        unwrap_key(&keys(), &ctx, &wrapped()),
        "the id of another key",
    );
}

#[test]
fn another_tenant_does_not_unwrap_it() {
    let ctx = ColumnContext {
        tenant: TenantId([0x12; 16]),
        ..CTX
    };
    refused(unwrap_key(&keys(), &ctx, &wrapped()), "another tenant");
}

#[test]
fn another_table_or_column_does_not_unwrap_it() {
    for (table, column) in [("audit", "wrapped"), ("audit_key", "target_sealed")] {
        let ctx = ColumnContext {
            table,
            column,
            ..CTX
        };
        refused(
            unwrap_key(&keys(), &ctx, &wrapped()),
            &format!("{table}.{column}"),
        );
    }
}

#[test]
fn a_blob_with_any_one_byte_changed_does_not_unwrap() {
    let blob = wrapped();
    for at in 0..blob.len() {
        let mut changed = blob.clone();
        changed[at] ^= 0x01;
        refused(unwrap_key(&keys(), &CTX, &changed), &format!("byte {at}"));
    }
}

#[test]
fn a_sealed_value_that_is_not_32_bytes_is_not_a_key() {
    for len in [0, 1, 31, 33, 64] {
        let blob = seal_column(&keys(), &CTX, &vec![0x77; len]).expect("seals");
        refused(
            unwrap_key(&keys(), &CTX, &blob),
            &format!("a value of {len} bytes"),
        );
    }
}

// The edges.

#[test]
fn an_empty_blob_and_every_cut_of_a_blob_are_refused() {
    let blob = wrapped();
    for len in 0..blob.len() {
        refused(
            unwrap_key(&keys(), &CTX, &blob[..len]),
            &format!("the first {len} bytes"),
        );
    }
}

#[test]
fn a_blob_longer_than_a_page_is_refused_and_does_not_panic() {
    let blob = seal_column(&keys(), &CTX, &vec![0x77; 70_000]).expect("seals");
    refused(unwrap_key(&keys(), &CTX, &blob), "70,000 bytes");
}

#[test]
fn every_wrap_of_the_same_key_gives_a_new_blob() {
    let (first, second) = (wrapped(), wrapped());
    assert_ne!(first, second, "a new nonce each time");
    for blob in [first, second] {
        assert_eq!(
            unwrap_key(&keys(), &CTX, &blob).expect("unwraps").id(),
            key_of(SECRET).id(),
            "each unwraps to the key"
        );
    }
}

#[test]
fn a_key_wrapped_with_an_empty_row_unwraps_only_with_an_empty_row() {
    let ctx = ColumnContext { row: &[], ..CTX };
    let blob = wrap_key(&keys(), &ctx, &key_of(SECRET)).expect("wraps");
    assert_eq!(
        unwrap_key(&keys(), &ctx, &blob).expect("unwraps").id(),
        key_of(SECRET).id(),
        "the same context"
    );
    refused(unwrap_key(&keys(), &CTX, &blob), "a row of eight bytes");
}
