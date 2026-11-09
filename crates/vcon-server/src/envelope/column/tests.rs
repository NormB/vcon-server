//! The column codec's tests: what must work, what must be refused, and
//! the limits. The vectors were computed independently with Python's hmac,
//! hashlib and the cryptography package's AESGCM.

use super::{
    ColumnContext, ColumnKeys, FORMAT, Key, KeyError, LockedPage, NONCE_LEN, REGISTRY,
    SEALED_COLUMN_OVERHEAD, associated_data, gcm_open, gcm_seal, hmac_sha256, mac, open_column,
    seal_column, seal_column_with_nonce, subkey, subkey_info,
};
use crate::ids::TenantId;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn key_of(bytes: [u8; 32]) -> Key {
    let mut page = LockedPage::new().expect("page");
    page.bytes_mut()[..32].copy_from_slice(&bytes);
    Key { page }
}

const CATALOG: [u8; 32] = [0x44; 32];

fn keys() -> ColumnKeys {
    ColumnKeys::derive(&key_of(CATALOG)).expect("derives")
}

const TENANT: TenantId = TenantId([0x11; 16]);
const ROW: [u8; 32] = [0xaa; 32];

const CTX: ColumnContext<'static> = ColumnContext {
    tenant: TENANT,
    table: "version",
    column: "form_sealed",
    row: &ROW,
};

fn sealed() -> Vec<u8> {
    seal_column(&keys(), &CTX, b"application/vcon+json").expect("seals")
}

/// A blob that does open under [`CTX`], so a test that expects another
/// context to fail is not passing on a codec that opens nothing.
fn sealed_and_opening() -> Vec<u8> {
    let blob = sealed();
    assert_eq!(
        open_column(&keys(), &CTX, &blob).expect("opens where it was sealed"),
        b"application/vcon+json",
        "the control: it opens under its own context"
    );
    blob
}

fn with_tenant(tenant: TenantId) -> ColumnContext<'static> {
    ColumnContext { tenant, ..CTX }
}

// ---- What must work.

#[test]
fn the_column_key_is_hkdf_of_the_catalog_key_with_the_columns_label() {
    assert_eq!(
        keys().column.bytes(),
        hex("72a74904c6815e2ecd9a482249fa077d06fd63f11fbd70275cc0fa8df74a179b"),
        "the value Python's HKDF gives for this catalog key and label"
    );
}

#[test]
fn the_index_key_is_hkdf_of_the_catalog_key_with_the_index_label() {
    assert_eq!(
        keys().index.bytes(),
        hex("2ef3eae3fad0cf977c566c9e37c9767f311029416bec5f904a220e8284674a11"),
        "the value Python's HKDF gives for this catalog key and label"
    );
}

#[test]
fn a_row_subkey_is_hkdf_of_the_column_key_with_table_column_and_row() {
    let subkey = subkey(&keys(), &CTX).expect("derives");
    assert_eq!(
        subkey.bytes(),
        hex("e6591863b41a29f56f10f7663c6c3d10ef075709a6c199f6166a4fc9ebd9724d"),
        "the subkey Python's HKDF gives for this table, column and row"
    );
}

#[test]
fn the_associated_data_is_written_out_field_by_field() {
    let mut expected = b"vcon-server/v1".to_vec();
    expected.extend([1, 3]);
    expected.extend([0x11; 16]);
    expected.extend(hex("0000000776657273696f6e"));
    expected.extend(hex("0000000b666f726d5f7365616c6564"));
    expected.extend(hex("00000020"));
    expected.extend([0xaa; 32]);
    assert_eq!(
        associated_data(&CTX, FORMAT),
        expected,
        "the label, format byte, kind 3, tenant, then length-prefixed table, column, row"
    );
}

#[test]
fn a_sealed_value_matches_the_vector_computed_independently() {
    let nonce: [u8; 12] = hex("555555555555555555555555")
        .try_into()
        .expect("12 bytes");
    let blob =
        seal_column_with_nonce(&keys(), &CTX, nonce, b"application/vcon+json").expect("seals");
    assert_eq!(
        blob,
        hex(concat!(
            "01555555555555555555555555",
            "9d7bd4ff7cb6d39387149ff9b4d610cbf4f388c0d0",
            "0a1380da97637ec519f9f041adb330e9"
        )),
        "format byte, nonce, ciphertext and tag as Python's AESGCM gives them"
    );
}

/// RFC 4231, test cases 1, 2 and 3 (HMAC-SHA-256).
#[test]
fn hmac_sha256_matches_rfc_4231() {
    let cases: [(Vec<u8>, Vec<u8>, &str); 3] = [
        (
            vec![0x0b; 20],
            b"Hi There".to_vec(),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        ),
        (
            b"Jefe".to_vec(),
            b"what do ya want for nothing?".to_vec(),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        ),
        (
            vec![0xaa; 20],
            vec![0xdd; 50],
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
        ),
    ];
    for (n, (key, data, mac)) in cases.into_iter().enumerate() {
        assert_eq!(
            hmac_sha256(&key, &data).to_vec(),
            hex(mac),
            "case {}",
            n + 1
        );
    }
}

#[test]
fn a_mac_matches_the_vector_computed_independently() {
    let uuid = hex("018bcfe5000070008000000000000001");
    assert_eq!(
        mac(&keys(), &uuid).to_vec(),
        hex("07f2b53f73d808c65a4e68692fb2e72bb31da8e2059065fbc653efbef91ef0dd"),
        "HMAC-SHA256 of the uuid's 16 bytes under the index key"
    );
    assert_eq!(
        mac(&keys(), b"acme-example").to_vec(),
        hex("b01121d6652754fb316d0c0b2a1f4714093a5f2c6fb4d0b08bfe3eb7ce29f706"),
        "and of a tenant name as written"
    );
}

#[test]
fn a_sealed_value_opens_with_the_same_keys_and_context() {
    assert_eq!(
        open_column(&keys(), &CTX, &sealed()).expect("opens"),
        b"application/vcon+json",
        "seal then open gives the plaintext back"
    );
}

#[test]
fn every_seal_of_the_same_value_gives_a_new_blob_and_nonce() {
    let blobs: Vec<Vec<u8>> = (0..64).map(|_| sealed()).collect();
    let nonces: std::collections::HashSet<_> =
        blobs.iter().map(|b| b[1..1 + NONCE_LEN].to_vec()).collect();
    assert_eq!(
        nonces.len(),
        blobs.len(),
        "a nonce repeated within 64 seals"
    );
    assert_ne!(blobs[0], blobs[1], "the same value seals differently");
    for blob in &blobs {
        assert_eq!(
            open_column(&keys(), &CTX, blob).expect("opens"),
            b"application/vcon+json",
            "each opens"
        );
    }
}

#[test]
fn the_registry_context_opens_like_a_tenant_context() {
    let ctx = with_tenant(REGISTRY);
    let blob = seal_column(&keys(), &ctx, b"operator key label").expect("seals");
    assert_eq!(
        open_column(&keys(), &ctx, &blob).expect("opens"),
        b"operator key label",
        "the registry's scope opens with the same context"
    );
}

#[test]
fn a_blob_is_exactly_its_plaintext_and_the_stated_overhead() {
    assert_eq!(
        SEALED_COLUMN_OVERHEAD, 29,
        "one format byte, a 12-byte nonce and a 16-byte tag"
    );
    assert_eq!(
        sealed().len(),
        b"application/vcon+json".len() + SEALED_COLUMN_OVERHEAD,
        "ciphertext is as long as the plaintext"
    );
    assert_eq!(sealed()[0], FORMAT, "the first byte is the format");
}

#[test]
fn equal_values_have_equal_macs_and_other_values_do_not() {
    assert_eq!(
        mac(&keys(), b"same"),
        mac(&keys(), b"same"),
        "equal values have equal MACs, which SQL finds them by"
    );
    assert_ne!(
        mac(&keys(), b"same"),
        mac(&keys(), b"other"),
        "other values do not"
    );
}

#[test]
fn a_key_derived_again_opens_what_the_first_derivation_sealed() {
    let again = ColumnKeys::derive(&key_of(CATALOG)).expect("derives");
    assert_eq!(
        open_column(&again, &CTX, &sealed()).expect("opens"),
        b"application/vcon+json",
        "the keys are derived at open and never stored, so a new derivation must open it"
    );
    assert_eq!(
        mac(&again, b"same"),
        mac(&keys(), b"same"),
        "and gives the same MACs"
    );
}

// ---- What must be refused.

fn assert_damaged(result: Result<Vec<u8>, KeyError>, what: &str) {
    assert!(
        matches!(result, Err(KeyError::Damaged)),
        "{what}: must not open as Damaged"
    );
}

#[test]
fn a_value_moved_to_another_tenant_does_not_open() {
    let other = with_tenant(TenantId([0x12; 16]));
    assert_damaged(
        open_column(&keys(), &other, &sealed_and_opening()),
        "another tenant",
    );
}

#[test]
fn a_value_moved_to_the_registry_does_not_open() {
    assert_damaged(
        open_column(&keys(), &with_tenant(REGISTRY), &sealed_and_opening()),
        "a tenant's value read as the registry's",
    );
}

#[test]
fn a_value_moved_to_another_table_does_not_open() {
    let other = ColumnContext {
        table: "tombstone",
        ..CTX
    };
    assert_damaged(
        open_column(&keys(), &other, &sealed_and_opening()),
        "another table",
    );
}

#[test]
fn a_value_moved_to_another_column_does_not_open() {
    let other = ColumnContext {
        column: "gzip_sealed",
        ..CTX
    };
    assert_damaged(
        open_column(&keys(), &other, &sealed_and_opening()),
        "another column",
    );
}

#[test]
fn a_value_moved_to_another_row_does_not_open() {
    let other_row = [0xbb; 32];
    let other = ColumnContext {
        row: &other_row,
        ..CTX
    };
    assert_damaged(
        open_column(&keys(), &other, &sealed_and_opening()),
        "another row",
    );
}

#[test]
fn a_value_opened_under_another_catalog_key_does_not_open() {
    let other = ColumnKeys::derive(&key_of([0x45; 32])).expect("derives");
    assert_damaged(
        open_column(&other, &CTX, &sealed_and_opening()),
        "another catalog key",
    );
}

#[test]
fn changing_any_byte_of_a_sealed_value_fails() {
    let blob = sealed_and_opening();
    for i in 0..blob.len() {
        let mut bad = blob.clone();
        bad[i] ^= 0x01;
        assert_damaged(open_column(&keys(), &CTX, &bad), &format!("byte {i}"));
    }
}

#[test]
fn a_blob_cut_short_or_grown_does_not_open() {
    let blob = sealed_and_opening();
    for len in 0..blob.len() {
        assert_damaged(
            open_column(&keys(), &CTX, &blob[..len]),
            &format!("first {len} bytes"),
        );
    }
    let mut longer = blob.clone();
    longer.push(0);
    assert_damaged(open_column(&keys(), &CTX, &longer), "one byte more");
}

#[test]
fn a_blob_made_under_a_version_context_does_not_open_as_a_column() {
    let version = super::super::Context::Version {
        tenant: TENANT,
        uuid: uuid::Uuid::from_bytes([0x33; 16]),
        version: crate::ids::VersionId([0x22; 16]),
    };
    let key = subkey(&keys(), &CTX).expect("derives");
    let blob = super::super::seal(&key, &version, b"x");
    assert_eq!(
        super::super::open(&key, &version, &blob).as_deref(),
        Ok(&b"x"[..]),
        "the control: it opens as what it is"
    );
    assert_damaged(
        open_column(&keys(), &CTX, &blob),
        "a body blob is not a column value",
    );
}

/// The subkey binding is checked apart from the associated data, so
/// removing one binding is not hidden by the other.
#[test]
fn the_subkey_differs_for_each_of_table_column_and_row() {
    let k = keys();
    let base = subkey(&k, &CTX).expect("derives");
    let other_row = [0xbb; 32];
    let others = [
        ColumnContext {
            table: "tombstone",
            ..CTX
        },
        ColumnContext {
            column: "gzip_sealed",
            ..CTX
        },
        ColumnContext {
            row: &other_row,
            ..CTX
        },
    ];
    for other in others {
        assert_ne!(
            subkey(&k, &other).expect("derives").bytes(),
            base.bytes(),
            "{other:?} gives the subkey of another place"
        );
    }
}

/// What opens under one associated data must not open under another with
/// one field changed, the key held fixed: this checks the associated data
/// by itself.
fn assert_ad_binds(other: &ColumnContext<'_>) {
    let random = aws_lc_rs::rand::SystemRandom::new();
    let key: [u8; 32] = aws_lc_rs::rand::generate(&random).expect("random").expose();
    let nonce: [u8; 12] = aws_lc_rs::rand::generate(&random).expect("random").expose();
    let mut text = b"value".to_vec();
    let tag = gcm_seal(&key, nonce, &associated_data(&CTX, FORMAT), &mut text);
    text.extend(tag);
    let mut sealed = text.clone();
    assert!(
        gcm_open(&key, nonce, &associated_data(&CTX, FORMAT), &mut sealed).is_ok(),
        "the same associated data opens it"
    );
    let mut sealed = text;
    assert!(
        gcm_open(&key, nonce, &associated_data(other, FORMAT), &mut sealed).is_err(),
        "{other:?} must not open a value bound to another place"
    );
}

#[test]
fn the_associated_data_binds_the_tenant() {
    assert_ad_binds(&with_tenant(TenantId([0x12; 16])));
}

#[test]
fn the_associated_data_binds_the_table() {
    assert_ad_binds(&ColumnContext {
        table: "tombstone",
        ..CTX
    });
}

#[test]
fn the_associated_data_binds_the_column() {
    assert_ad_binds(&ColumnContext {
        column: "gzip_sealed",
        ..CTX
    });
}

#[test]
fn the_associated_data_binds_the_row_key() {
    let row = [0xab; 32];
    assert_ad_binds(&ColumnContext { row: &row, ..CTX });
}

#[test]
fn the_associated_data_binds_the_format_byte() {
    assert_ne!(
        associated_data(&CTX, FORMAT),
        associated_data(&CTX, FORMAT + 1),
        "a blob of another format has other associated data"
    );
}

#[test]
fn the_associated_data_is_not_a_bodys() {
    let version = super::super::Context::Version {
        tenant: TENANT,
        uuid: uuid::Uuid::from_bytes([0x33; 16]),
        version: crate::ids::VersionId([0x22; 16]),
    };
    assert_ne!(
        associated_data(&CTX, FORMAT)[..16],
        version.associated_data(FORMAT)[..16],
        "the kind byte 3 differs from a body's 2"
    );
}

#[test]
fn a_mac_under_another_catalog_key_differs() {
    let other = ColumnKeys::derive(&key_of([0x45; 32])).expect("derives");
    assert_ne!(
        mac(&other, b"same"),
        mac(&keys(), b"same"),
        "a rotated key gives other MACs"
    );
}

// ---- The limits.

#[test]
fn an_empty_value_seals_and_opens() {
    let blob = seal_column(&keys(), &CTX, b"").expect("seals");
    assert_eq!(blob.len(), SEALED_COLUMN_OVERHEAD, "only the overhead");
    assert_eq!(
        open_column(&keys(), &CTX, &blob).expect("opens"),
        b"",
        "empty"
    );
}

#[test]
fn a_value_of_64_kib_seals_and_opens() {
    let long = vec![0xa5; 65_536];
    let blob = seal_column(&keys(), &CTX, &long).expect("seals");
    assert_eq!(
        open_column(&keys(), &CTX, &blob).expect("opens"),
        long,
        "a long value round trips"
    );
}

#[test]
fn an_empty_row_key_is_a_place_of_its_own() {
    let empty = ColumnContext { row: &[], ..CTX };
    let blob = seal_column(&keys(), &empty, b"v").expect("seals");
    assert_eq!(
        open_column(&keys(), &empty, &blob).expect("opens"),
        b"v",
        "opens"
    );
    assert_damaged(open_column(&keys(), &CTX, &blob), "another row key");
}

#[test]
fn a_row_key_of_a_megabyte_seals_and_opens() {
    let row = vec![0x5a; 1 << 20];
    let ctx = ColumnContext { row: &row, ..CTX };
    let blob = seal_column(&keys(), &ctx, b"v").expect("seals");
    assert_eq!(
        open_column(&keys(), &ctx, &blob).expect("opens"),
        b"v",
        "opens"
    );
}

#[test]
fn moving_bytes_between_table_and_column_gives_another_place() {
    let a = ColumnContext {
        table: "ab",
        column: "c",
        ..CTX
    };
    let b = ColumnContext {
        table: "a",
        column: "bc",
        ..CTX
    };
    assert_ne!(
        subkey_info(&a),
        subkey_info(&b),
        "the fields are length-prefixed, so a boundary change is another subkey"
    );
    assert_ne!(
        associated_data(&a, FORMAT),
        associated_data(&b, FORMAT),
        "and other associated data"
    );
}

#[test]
fn moving_bytes_between_column_and_row_gives_another_place() {
    let a = ColumnContext {
        column: "ab",
        row: b"c",
        ..CTX
    };
    let b = ColumnContext {
        column: "a",
        row: b"bc",
        ..CTX
    };
    assert_ne!(subkey_info(&a), subkey_info(&b), "another subkey");
    assert_ne!(
        associated_data(&a, FORMAT),
        associated_data(&b, FORMAT),
        "other associated data"
    );
}

#[test]
fn a_non_ascii_column_name_is_its_bytes() {
    let ctx = ColumnContext {
        table: "tablé",
        ..CTX
    };
    let blob = seal_column(&keys(), &ctx, b"v").expect("seals");
    assert_eq!(
        open_column(&keys(), &ctx, &blob).expect("opens"),
        b"v",
        "opens"
    );
    assert!(
        associated_data(&ctx, FORMAT)
            .windows("tablé".len())
            .any(|w| w == "tablé".as_bytes()),
        "the name goes in as its UTF-8 bytes"
    );
}

#[test]
fn a_mac_of_the_empty_value_is_32_bytes_and_not_zero() {
    let out = mac(&keys(), b"");
    assert_eq!(out.len(), 32, "the full 32 bytes");
    assert_ne!(out, [0; 32], "not the zero value");
}

#[test]
fn a_mac_is_the_full_32_bytes_and_every_bit_can_vary() {
    let macs: Vec<[u8; 32]> = (0u32..256)
        .map(|i| mac(&keys(), &i.to_be_bytes()))
        .collect();
    for byte in 0..32 {
        assert!(
            macs.iter().any(|m| m[byte] != macs[0][byte]),
            "byte {byte} of the MAC never changes"
        );
    }
}

#[test]
fn the_debug_of_column_keys_shows_none_of_their_bytes_or_ids() {
    let k = keys();
    let shown = format!("{k:?}");
    assert!(
        !shown.contains(&k.column.id().to_string()) && !shown.contains(&k.index.id().to_string()),
        "not even the ids: {shown}"
    );
    assert_eq!(shown, "ColumnKeys { .. }", "only the name: {shown}");
}

#[test]
fn the_tenant_field_is_a_fixed_16_bytes_in_the_associated_data() {
    let ad = associated_data(&with_tenant(REGISTRY), FORMAT);
    assert_eq!(
        ad[b"vcon-server/v1".len() + 2..b"vcon-server/v1".len() + 2 + 16],
        [0; 16],
        "the registry's tenant is 16 zero bytes, where a tenant's id goes"
    );
}
