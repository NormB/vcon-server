//! The `key-wrap` format: what a blob holds comes back from its bytes, the
//! layout is fixed byte for byte, and every malformed blob is refused with
//! the first thing wrong named, the format first.

use super::{BackendKind, Blob, BlobError, FORMAT, MAX_CIPHERTEXT, MAX_KEY_REF, WrapRef};

const ARN: &str = "arn:aws:kms:us-east-1:111122223333:key/1234abcd-12ab-34cd-56ef-1234567890ab";

fn reference(kind: BackendKind, key: &str) -> WrapRef {
    WrapRef::new(kind, key).expect("a valid key reference")
}

fn blob(kind: BackendKind, key: &str, ciphertext: &[u8]) -> Blob {
    Blob::new(reference(kind, key), ciphertext.to_vec()).expect("a valid blob")
}

/// A blob of format 1 with the given fields as raw bytes, lengths as given.
fn raw(kind: u8, key_len: u16, key: &[u8], ct_len: u32, ct: &[u8]) -> Vec<u8> {
    let mut out = vec![FORMAT, kind];
    out.extend_from_slice(&key_len.to_be_bytes());
    out.extend_from_slice(key);
    out.extend_from_slice(&ct_len.to_be_bytes());
    out.extend_from_slice(ct);
    out
}

// ---- Success ----

#[test]
fn a_blob_of_every_kind_comes_back_from_its_bytes() {
    for kind in BackendKind::ALL {
        let made = blob(kind, "transit/vcon-server#3", &[9, 8, 7, 6]);
        let back = Blob::decode(&made.encode()).expect("decodes");
        assert_eq!(back, made, "{kind} round-trips");
    }
}

#[test]
fn the_layout_is_fixed_byte_for_byte() {
    let made = blob(BackendKind::VaultTransit, "t/k", &[0xaa, 0xbb]);
    assert_eq!(
        made.encode(),
        vec![1, 4, 0, 3, b't', b'/', b'k', 0, 0, 0, 2, 0xaa, 0xbb],
        "format, kind, reference length and bytes, ciphertext length and bytes"
    );
}

#[test]
fn each_kind_has_its_fixed_byte() {
    let bytes: Vec<u8> = BackendKind::ALL
        .iter()
        .map(|k| blob(*k, "k", &[1]).encode()[1])
        .collect();
    assert_eq!(
        bytes,
        vec![1, 2, 3, 4, 5, 6],
        "the kinds' bytes never change"
    );
}

#[test]
fn each_kind_has_its_fixed_name() {
    let names: Vec<&str> = BackendKind::ALL.iter().map(|k| k.name()).collect();
    assert_eq!(
        names,
        vec![
            "aws-kms",
            "gcp-kms",
            "azure-key-vault",
            "vault-transit",
            "openbao-transit",
            "pkcs11"
        ],
        "the configuration's names"
    );
}

#[test]
fn every_name_gives_its_kind_back() {
    for kind in BackendKind::ALL {
        assert_eq!(
            BackendKind::from_name(kind.name()),
            Some(kind),
            "{kind} by its name"
        );
        assert_eq!(kind.to_string(), kind.name(), "{kind} displays its name");
    }
}

#[test]
fn a_reference_comes_back_from_its_text() {
    let r = reference(BackendKind::OpenbaoTransit, "transit/vcon#2");
    assert_eq!(
        r.to_string(),
        "openbao-transit:transit/vcon#2",
        "kind, colon, key"
    );
    assert_eq!(WrapRef::parse(&r.to_string()), Ok(r), "parses back");
}

#[test]
fn a_key_with_colons_of_its_own_parses_whole() {
    let r = WrapRef::parse(&format!("aws-kms:{ARN}")).expect("an ARN");
    assert_eq!(
        r.kind(),
        BackendKind::AwsKms,
        "the kind is before the first colon"
    );
    assert_eq!(r.key(), ARN, "every colon after it is the key's");
}

#[test]
fn the_accessors_give_what_was_made() {
    let made = blob(BackendKind::Pkcs11, "0a1b2c", &[5; 40]);
    assert_eq!(made.reference().kind(), BackendKind::Pkcs11, "kind");
    assert_eq!(made.reference().key(), "0a1b2c", "key");
    assert_eq!(made.ciphertext(), &[5; 40][..], "ciphertext");
}

#[test]
fn the_encoded_length_is_the_fields_and_their_lengths() {
    let made = blob(BackendKind::AwsKms, ARN, &[3; 184]);
    assert_eq!(
        made.encode().len(),
        1 + 1 + 2 + ARN.len() + 4 + 184,
        "no padding, nothing after"
    );
}

#[test]
fn the_smallest_blob_round_trips() {
    let made = blob(BackendKind::GcpKms, "k", &[0]);
    assert_eq!(
        made.encode().len(),
        10,
        "1 + 1 + 2 + 1 + 4 + 1: one byte of key and of ciphertext"
    );
    assert_eq!(Blob::decode(&made.encode()), Ok(made), "decodes");
}

#[test]
fn the_largest_blob_round_trips() {
    let key = "k".repeat(MAX_KEY_REF);
    let made = blob(BackendKind::AzureKeyVault, &key, &vec![7; MAX_CIPHERTEXT]);
    assert_eq!(
        Blob::decode(&made.encode()),
        Ok(made),
        "decodes at both limits"
    );
}

#[test]
fn debug_shows_the_reference_and_the_ciphertext_length_only() {
    let made = blob(BackendKind::VaultTransit, "t/k", &[0xc3, 0x5a, 0x99]);
    let shown = format!("{made:?}");
    assert!(shown.contains("t/k"), "the reference: {shown}");
    assert!(shown.contains("ciphertext_len: 3"), "the length: {shown}");
    for needle in ["195", "90", "153", "c3", "C3", "5a", "99"] {
        assert!(
            !shown.contains(needle),
            "no ciphertext byte {needle}: {shown}"
        );
    }
}

// ---- Failure ----

#[test]
fn an_empty_blob_is_refused() {
    assert_eq!(Blob::decode(&[]), Err(BlobError::Empty), "no bytes");
}

#[test]
fn format_zero_is_refused_and_named() {
    let err = Blob::decode(&[0, 4, 0, 1, b'k', 0, 0, 0, 1, 1]).expect_err("format 0");
    assert_eq!(err, BlobError::UnknownFormat(0), "format 0");
    assert_eq!(
        err.to_string(),
        "key-wrap format 0 is not one this build knows",
        "the format is named"
    );
}

#[test]
fn a_newer_format_is_refused_and_named() {
    let mut bytes = blob(BackendKind::VaultTransit, "k", &[1]).encode();
    bytes[0] = 2;
    let err = Blob::decode(&bytes).expect_err("format 2");
    assert_eq!(err, BlobError::UnknownFormat(2), "format 2");
    assert!(err.to_string().contains("format 2"), "named: {err}");
}

#[test]
fn kind_zero_is_refused() {
    assert_eq!(
        Blob::decode(&raw(0, 1, b"k", 1, &[1])),
        Err(BlobError::UnknownKind(0)),
        "no kind has byte 0"
    );
}

#[test]
fn a_kind_after_the_last_is_refused_and_named() {
    let err = Blob::decode(&raw(7, 1, b"k", 1, &[1])).expect_err("kind 7");
    assert_eq!(err, BlobError::UnknownKind(7), "kind 7");
    assert_eq!(
        err.to_string(),
        "a key-wrap blob names key backend 7, which this build does not know",
        "the kind's byte is named"
    );
}

#[test]
fn every_blob_cut_short_is_refused() {
    let whole = blob(BackendKind::AwsKms, ARN, &[3; 20]).encode();
    for len in 1..whole.len() {
        assert_eq!(
            Blob::decode(&whole[..len]),
            Err(BlobError::Truncated),
            "cut to {len} of {} bytes",
            whole.len()
        );
    }
}

#[test]
fn a_byte_after_the_ciphertext_is_refused() {
    let mut bytes = blob(BackendKind::GcpKms, "k", &[1, 2]).encode();
    bytes.push(0);
    assert_eq!(
        Blob::decode(&bytes),
        Err(BlobError::TrailingBytes),
        "one byte more"
    );
}

#[test]
fn an_empty_key_reference_is_refused() {
    assert_eq!(
        WrapRef::new(BackendKind::Pkcs11, ""),
        Err(BlobError::KeyRef),
        "new"
    );
    assert_eq!(
        Blob::decode(&raw(6, 0, b"", 1, &[1])),
        Err(BlobError::KeyRef),
        "decode"
    );
}

#[test]
fn a_key_reference_that_is_not_printable_ascii_is_refused() {
    for bad in ["a b", "a\tb", "a\nb", "a\u{7f}b", "caf\u{e9}", "\u{0}"] {
        assert_eq!(
            WrapRef::new(BackendKind::VaultTransit, bad),
            Err(BlobError::KeyRef),
            "new refuses {bad:?}"
        );
        let key = bad.as_bytes();
        let len = u16::try_from(key.len()).expect("short");
        assert_eq!(
            Blob::decode(&raw(4, len, key, 1, &[1])),
            Err(BlobError::KeyRef),
            "decode refuses {bad:?}"
        );
    }
}

#[test]
fn an_empty_ciphertext_is_refused() {
    let r = reference(BackendKind::AwsKms, "k");
    assert_eq!(Blob::new(r, Vec::new()), Err(BlobError::Ciphertext), "new");
    assert_eq!(
        Blob::decode(&raw(1, 1, b"k", 0, &[])),
        Err(BlobError::Ciphertext),
        "decode"
    );
}

#[test]
fn a_reference_text_without_a_known_kind_is_refused() {
    for bad in [
        "",
        "vault-transit",
        "file:master.key",
        "vault:t/k",
        ":k",
        "AWS-KMS:k",
    ] {
        assert_eq!(WrapRef::parse(bad), Err(BlobError::RefText), "{bad:?}");
    }
}

#[test]
fn a_reference_text_with_a_bad_key_is_refused() {
    assert_eq!(
        WrapRef::parse("pkcs11:"),
        Err(BlobError::KeyRef),
        "empty key"
    );
    assert_eq!(
        WrapRef::parse("pkcs11:a b"),
        Err(BlobError::KeyRef),
        "a space"
    );
}

#[test]
fn the_file_backend_has_no_kind_in_this_format() {
    assert_eq!(
        BackendKind::from_name("file"),
        None,
        "file wraps no key-wrap blob"
    );
}

#[test]
fn no_error_text_holds_the_blobs_bytes() {
    let ct: Vec<u8> = (0..64).map(|i| (i * 37 + 11) as u8).collect();
    let hex: String = ct.iter().map(|b| format!("{b:02x}")).collect();
    let mut cases = vec![
        Blob::decode(&raw(9, 1, b"k", 64, &ct)).expect_err("kind"),
        Blob::decode(&raw(1, 1, b"k", 64, &ct[..10])).expect_err("cut"),
        Blob::decode(&[ct.as_slice(), &[1]].concat()).expect_err("format"),
    ];
    let mut long = blob(BackendKind::GcpKms, "k", &ct).encode();
    long.extend_from_slice(&ct);
    cases.push(Blob::decode(&long).expect_err("trailing"));
    for err in cases {
        let text = err.to_string();
        assert!(!text.contains(&hex[..8]), "no ciphertext in {text:?}");
        assert!(!text.is_empty(), "{err:?} says what is wrong");
    }
}

// ---- Edge ----

#[test]
fn a_key_reference_of_the_limit_is_kept_and_one_more_refused() {
    let at = "r".repeat(MAX_KEY_REF);
    assert!(
        WrapRef::new(BackendKind::AwsKms, at).is_ok(),
        "{MAX_KEY_REF} bytes"
    );
    let over = "r".repeat(MAX_KEY_REF + 1);
    assert_eq!(
        WrapRef::new(BackendKind::AwsKms, over.clone()),
        Err(BlobError::KeyRef),
        "one byte over"
    );
    let len = u16::try_from(over.len()).expect("fits");
    assert_eq!(
        Blob::decode(&raw(1, len, over.as_bytes(), 1, &[1])),
        Err(BlobError::KeyRef),
        "decode refuses it too"
    );
}

#[test]
fn a_ciphertext_of_the_limit_is_kept_and_one_more_refused() {
    let r = reference(BackendKind::GcpKms, "k");
    assert!(
        Blob::new(r.clone(), vec![1; MAX_CIPHERTEXT]).is_ok(),
        "{MAX_CIPHERTEXT} bytes"
    );
    assert_eq!(
        Blob::new(r, vec![1; MAX_CIPHERTEXT + 1]),
        Err(BlobError::Ciphertext),
        "one byte over"
    );
}

#[test]
fn a_reference_length_past_the_limit_is_refused_before_its_bytes_are_read() {
    // The length says 1025; only one byte of it follows.
    assert_eq!(
        Blob::decode(&raw(1, 1025, b"k", 1, &[1])),
        Err(BlobError::KeyRef),
        "refused on the length, not reported as cut short"
    );
}

#[test]
fn a_huge_ciphertext_length_is_refused_without_reading_it() {
    assert_eq!(
        Blob::decode(&raw(1, 1, b"k", u32::MAX, &[1])),
        Err(BlobError::Ciphertext),
        "refused on the length"
    );
    let at = u32::try_from(MAX_CIPHERTEXT + 1).expect("fits");
    assert_eq!(
        Blob::decode(&raw(1, 1, b"k", at, &[1])),
        Err(BlobError::Ciphertext),
        "one past the limit"
    );
}

#[test]
fn the_format_is_judged_before_anything_else() {
    // Format 2 followed by bytes that would be cut short in format 1.
    assert_eq!(
        Blob::decode(&[2]),
        Err(BlobError::UnknownFormat(2)),
        "one byte"
    );
    assert_eq!(
        Blob::decode(&[2, 0, 0xff]),
        Err(BlobError::UnknownFormat(2)),
        "garbage after"
    );
}

#[test]
fn a_format_byte_alone_is_cut_short() {
    assert_eq!(
        Blob::decode(&[FORMAT]),
        Err(BlobError::Truncated),
        "only the format"
    );
}

#[test]
fn the_kind_is_judged_before_the_lengths() {
    assert_eq!(
        Blob::decode(&[FORMAT, 200]),
        Err(BlobError::UnknownKind(200)),
        "no length follows"
    );
}

#[test]
fn names_are_matched_exactly() {
    for name in ["AWS-KMS", "aws_kms", " aws-kms", "aws-kms ", "pkcs#11", ""] {
        assert_eq!(BackendKind::from_name(name), None, "{name:?}");
    }
}

#[test]
fn every_printable_ascii_byte_is_a_valid_key_character() {
    let key: String = (0x21_u8..=0x7e).map(char::from).collect();
    let r = WrapRef::new(BackendKind::Pkcs11, key.clone()).expect("printable");
    let made = Blob::new(r, vec![1]).expect("blob");
    assert_eq!(
        Blob::decode(&made.encode())
            .expect("decodes")
            .reference()
            .key(),
        key,
        "every byte from ! to ~ kept"
    );
}

#[test]
fn every_error_says_something_distinct() {
    let all = [
        BlobError::Empty,
        BlobError::UnknownFormat(3),
        BlobError::UnknownKind(9),
        BlobError::Truncated,
        BlobError::TrailingBytes,
        BlobError::KeyRef,
        BlobError::Ciphertext,
        BlobError::RefText,
    ];
    let texts: std::collections::BTreeSet<String> = all.iter().map(ToString::to_string).collect();
    assert_eq!(texts.len(), all.len(), "one text per error: {texts:?}");
    for err in &all {
        assert!(
            std::error::Error::source(err).is_none(),
            "{err:?} wraps no other error"
        );
    }
}
