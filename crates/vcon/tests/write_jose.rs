//! The signed and encrypted forms written by `vcon::write`, with test-side
//! signers and encrypters: deterministic, and simple enough that a test
//! can open what they make. The real algorithms are tested with the
//! `aws-lc` feature.

use std::collections::BTreeSet;
use std::io::Read;
use std::sync::Mutex;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use flate2::read::GzDecoder;
use proptest::prelude::{any, prop_assert_eq, proptest};
use serde_json::{Value, json};
use uuid::Uuid;
use vcon::Form;
use vcon::read::{Limits, MAX_SIGNED_LAYERS, inspect};
use vcon::write::{
    Chain, Compression, ContentEncrypter, CryptoError, Date, KeyEncrypter, Party, Sealed, Signer,
    Vcon, WriteError, Zeroizing, encrypt, sign, sign_again,
};

const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
const CREATED: &str = "2022-06-21T13:53:00-04:00";
const SIGNED_AT: &str = "2022-06-21T14:00:00-04:00";
const LIMITS: Limits = Limits::new(1 << 24);
/// A five-byte DER value standing in for a certificate.
const CERT: [u8; 5] = [0x30, 0x03, 0x02, 0x01, 0x05];

fn id() -> Uuid {
    ID.parse().expect("test uuid")
}

fn date(text: &str) -> Date {
    text.parse().expect("a test date is RFC 3339")
}

fn base() -> Vcon {
    Vcon::new(id(), date(CREATED))
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &Value) -> Vec<u8> {
    URL_SAFE_NO_PAD
        .decode(text.as_str().expect("a base64url string"))
        .expect("base64url without padding")
}

fn json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).expect("the output is JSON")
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .expect("the bytes are gzip");
    out
}

/// What `inspect` finds, with no extension supported.
fn found(body: &[u8]) -> (Form, bool, Uuid) {
    let i = inspect(body, None, &LIMITS, &BTreeSet::new()).expect("inspect accepts it");
    (i.form, i.gzip, i.uuid)
}

/// Signs with a signature that names the input's length, and records each
/// input it was given.
struct TestSigner {
    chain: Chain,
    inputs: Mutex<Vec<Vec<u8>>>,
    fails: bool,
}

impl TestSigner {
    fn new(chain: Chain) -> TestSigner {
        TestSigner {
            chain,
            inputs: Mutex::new(Vec::new()),
            fails: false,
        }
    }

    fn x5c() -> TestSigner {
        TestSigner::new(Chain::X5c(vec![CERT.to_vec()]))
    }

    fn failing() -> TestSigner {
        TestSigner {
            fails: true,
            ..TestSigner::x5c()
        }
    }

    fn inputs(&self) -> Vec<Vec<u8>> {
        self.inputs
            .lock()
            .expect("the lock is not poisoned")
            .clone()
    }
}

impl Signer for TestSigner {
    fn alg(&self) -> &str {
        "RS256"
    }

    fn chain(&self) -> &Chain {
        &self.chain
    }

    fn sign(&self, input: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if self.fails {
            return Err(CryptoError::new("the test signer has no key"));
        }
        self.inputs
            .lock()
            .expect("the lock is not poisoned")
            .push(input.to_vec());
        Ok(format!("signed {} bytes", input.len()).into_bytes())
    }
}

/// Wraps a key by XOR with one byte, so a test can unwrap it.
struct TestKey {
    byte: u8,
    fails: bool,
}

impl TestKey {
    fn new(byte: u8) -> TestKey {
        TestKey { byte, fails: false }
    }
}

impl KeyEncrypter for TestKey {
    fn alg(&self) -> &str {
        "RSA-OAEP"
    }

    fn encrypt_key(&self, cek: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if self.fails {
            return Err(CryptoError::new("the test recipient has no key"));
        }
        Ok(cek.iter().map(|b| b ^ self.byte).collect())
    }
}

const CEK: [u8; 64] = [0x11; 64];
const IV: [u8; 16] = [0x22; 16];
const TAG: [u8; 32] = [0x33; 32];

/// Encrypts by XOR with the content key, with a fixed key, IV and tag; it
/// records the additional authenticated data it was given.
struct TestContent {
    aads: Mutex<Vec<Vec<u8>>>,
    fails_key: bool,
    fails_seal: bool,
}

impl TestContent {
    fn new() -> TestContent {
        TestContent {
            aads: Mutex::new(Vec::new()),
            fails_key: false,
            fails_seal: false,
        }
    }
}

fn xor(bytes: &[u8], key: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .zip(key.iter().cycle())
        .map(|(b, k)| b ^ k)
        .collect()
}

impl ContentEncrypter for TestContent {
    fn enc(&self) -> &str {
        "A256CBC-HS512"
    }

    fn new_key(&self) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        if self.fails_key {
            return Err(CryptoError::new("no random source"));
        }
        Ok(Zeroizing::new(CEK.to_vec()))
    }

    fn seal(&self, cek: &[u8], plaintext: &[u8], aad: &[u8]) -> Result<Sealed, CryptoError> {
        if self.fails_seal {
            return Err(CryptoError::new("the cipher refused"));
        }
        self.aads
            .lock()
            .expect("the lock is not poisoned")
            .push(aad.to_vec());
        Ok(Sealed::new(IV.to_vec(), xor(plaintext, cek), TAG.to_vec()))
    }
}

/// Opens a JWE the test encrypters made, as the recipient whose key byte is
/// `byte`.
fn open(jwe: &Value, recipient: usize, byte: u8) -> Vec<u8> {
    let wrapped = unb64(&jwe["recipients"][recipient]["encrypted_key"]);
    let cek: Vec<u8> = wrapped.iter().map(|b| b ^ byte).collect();
    xor(&unb64(&jwe["ciphertext"]), &cek)
}

fn signed_base() -> Vec<u8> {
    let signer = TestSigner::x5c();
    sign(&base(), date(SIGNED_AT), Compression::None, &[&signer]).expect("signed")
}

// --- WRT-11: the signed form ---

/// Computed with Python from RFC 7515's rules, not with this crate: the
/// payload is base64url of the unsigned JSON with updated_at = signed_at,
/// protected is base64url of {"cty":"application/vcon"}, the signing input
/// is 283 bytes, and x5c is standard base64 of the DER bytes.
const GOLDEN_JWS: &str = concat!(
    r#"{"payload":"eyJ1dWlkIjoiMDFhMDdkYTgtYzJiYi04M2U1LWI5YTItMjc5ZTBkMTZiYzQ2IiwiY3JlYXRlZF9hdCI6"#,
    r#"IjIwMjItMDYtMjFUMTM6NTM6MDAtMDQ6MDAiLCJ1cGRhdGVkX2F0IjoiMjAyMi0wNi0yMVQxNDowMDowMC0wNDowMCIs"#,
    r#"InBhcnRpZXMiOltdLCJkaWFsb2ciOltdLCJhbmFseXNpcyI6W10sImF0dGFjaG1lbnRzIjpbXX0","signatures":"#,
    r#"[{"protected":"eyJjdHkiOiJhcHBsaWNhdGlvbi92Y29uIn0","header":{"alg":"RS256","x5c":["MAMCAQU="],"#,
    r#""uuid":"01a07da8-c2bb-83e5-b9a2-279e0d16bc46"},"signature":"c2lnbmVkIDI4MyBieXRlcw"}]}"#
);

#[test]
fn a_vcon_is_signed_byte_for_byte_as_rfc_7515_writes_it() {
    let bytes = signed_base();
    assert_eq!(
        String::from_utf8(bytes).expect("UTF-8"),
        GOLDEN_JWS,
        "the JWS computed independently"
    );
}

#[test]
fn the_signer_signs_protected_dot_payload() {
    let signer = TestSigner::x5c();
    let bytes = sign(&base(), date(SIGNED_AT), Compression::None, &[&signer]).expect("signed");
    let jws = json(&bytes);
    let expected = format!(
        "{}.{}",
        jws["signatures"][0]["protected"]
            .as_str()
            .expect("protected"),
        jws["payload"].as_str().expect("payload")
    );
    assert_eq!(
        signer.inputs(),
        vec![expected.into_bytes()],
        "one signing input, ASCII(protected) . ASCII(payload)"
    );
}

#[test]
fn the_payload_is_the_vcon_with_updated_at_set_to_the_signing_time() {
    let mut v = base();
    v.add_party(Party::new().name("Alice"));
    let before = v.to_json().expect("unsigned");
    let signer = TestSigner::x5c();
    let bytes = sign(&v, date(SIGNED_AT), Compression::None, &[&signer]).expect("signed");
    let mut expected = v.clone();
    expected.set_updated_at(date(SIGNED_AT));
    assert_eq!(
        unb64(&json(&bytes)["payload"]),
        expected.to_json().expect("unsigned"),
        "the payload is the unsigned form with updated_at = signed_at"
    );
    assert_eq!(
        v.to_json().expect("unsigned"),
        before,
        "the caller's model is unchanged"
    );
    assert_eq!(
        found(&bytes),
        (Form::Signed, false, id()),
        "inspect reads it as signed"
    );
}

#[test]
fn a_gzip_payload_inflates_to_the_vcon_and_is_marked_gzip() {
    let signer = TestSigner::x5c();
    let bytes = sign(&base(), date(SIGNED_AT), Compression::Gzip, &[&signer]).expect("signed");
    let jws = json(&bytes);
    let mut expected = base();
    expected.set_updated_at(date(SIGNED_AT));
    assert_eq!(
        gunzip(&unb64(&jws["payload"])),
        expected.to_json().expect("unsigned"),
        "the payload is the gzip of the unsigned form"
    );
    assert_eq!(
        unb64(&jws["signatures"][0]["protected"]),
        br#"{"cty":"application/vcon+gzip"}"#,
        "the protected header marks it gzip"
    );
    assert_eq!(
        found(&bytes),
        (Form::Signed, false, id()),
        "inspect reads it through the gzip"
    );
}

#[test]
fn an_x5u_chain_is_written_as_its_url() {
    let signer = TestSigner::new(Chain::X5u("https://example.com/chain.pem".into()));
    let bytes = sign(&base(), date(SIGNED_AT), Compression::None, &[&signer]).expect("signed");
    assert_eq!(
        json(&bytes)["signatures"][0]["header"],
        json!({"alg": "RS256", "x5u": "https://example.com/chain.pem", "uuid": ID}),
        "alg, x5u and uuid in the header"
    );
}

#[test]
fn several_certificates_are_written_in_order_in_standard_base64() {
    let second = vec![0xfb, 0xff, 0x00];
    let signer = TestSigner::new(Chain::X5c(vec![CERT.to_vec(), second.clone()]));
    let bytes = sign(&base(), date(SIGNED_AT), Compression::None, &[&signer]).expect("signed");
    assert_eq!(
        json(&bytes)["signatures"][0]["header"]["x5c"],
        json!([STANDARD.encode(CERT), STANDARD.encode(&second)]),
        "standard base64, with padding and + and /"
    );
}

#[test]
fn the_protected_header_holds_only_cty_so_no_name_repeats() {
    let bytes = signed_base();
    let jws = json(&bytes);
    let protected = json(&unb64(&jws["signatures"][0]["protected"]));
    let header = &jws["signatures"][0]["header"];
    assert_eq!(
        protected,
        json!({"cty": "application/vcon"}),
        "only cty is protected"
    );
    for name in ["alg", "x5c", "uuid"] {
        assert!(header.get(name).is_some(), "{name} is in the header");
        assert!(
            protected.get(name).is_none(),
            "{name} is not also protected"
        );
    }
}

#[test]
fn each_signer_adds_a_signature_over_the_same_input() {
    let a = TestSigner::x5c();
    let b = TestSigner::new(Chain::X5u("https://example.com/b.pem".into()));
    let bytes = sign(&base(), date(SIGNED_AT), Compression::None, &[&a, &b]).expect("signed");
    assert_eq!(
        json(&bytes)["signatures"]
            .as_array()
            .expect("signatures")
            .len(),
        2,
        "one signature per signer"
    );
    assert_eq!(a.inputs(), b.inputs(), "both signed the same input");
}

#[test]
fn signing_needs_a_signer_and_a_usable_chain() {
    assert_eq!(
        sign(&base(), date(SIGNED_AT), Compression::None, &[]),
        Err(WriteError::NoSigner),
        "no signer"
    );
    for (case, chain) in [
        ("an empty x5c", Chain::X5c(vec![])),
        (
            "an http x5u",
            Chain::X5u("http://example.com/chain.pem".into()),
        ),
        ("an x5u with no host", Chain::X5u("https://".into())),
    ] {
        let signer = TestSigner::new(chain);
        assert_eq!(
            sign(&base(), date(SIGNED_AT), Compression::None, &[&signer]),
            Err(WriteError::BadChain),
            "{case} is refused"
        );
    }
}

#[test]
fn a_signer_that_fails_is_reported_in_its_own_words() {
    let signer = TestSigner::failing();
    assert_eq!(
        sign(&base(), date(SIGNED_AT), Compression::None, &[&signer]),
        Err(WriteError::Crypto(CryptoError::new(
            "the test signer has no key"
        ))),
        "the signer's error"
    );
}

#[test]
fn a_vcon_that_cannot_be_written_is_not_signed() {
    let mut v = base();
    v.set_redacted_without_prior("PII Redaction");
    v.set_amended(vcon::write::Reference::by_uuid(Uuid::nil()));
    let signer = TestSigner::x5c();
    assert_eq!(
        sign(&v, date(SIGNED_AT), Compression::None, &[&signer]),
        Err(WriteError::BothLinks),
        "the unsigned form's refusal"
    );
    assert!(signer.inputs().is_empty(), "nothing was signed");
}

// --- WRT-12: signing again ---

#[test]
fn a_signed_vcon_signed_again_reads_back_with_its_innermost_uuid() {
    let inner = signed_base();
    let signer = TestSigner::new(Chain::X5u("https://example.com/archive.pem".into()));
    let outer = sign_again(&inner, Compression::None, &signer, LIMITS).expect("signed again");
    let jws = json(&outer);
    assert_eq!(
        unb64(&jws["payload"]),
        inner,
        "the payload is the signed vCon as given"
    );
    assert_eq!(
        jws["signatures"][0]["header"]["uuid"],
        json!(ID),
        "the header names the innermost uuid"
    );
    assert_eq!(
        found(&outer),
        (Form::Signed, false, id()),
        "inspect reads both layers"
    );
}

#[test]
fn a_gzip_layer_is_marked_gzip() {
    let signer = TestSigner::x5c();
    let outer = sign_again(&signed_base(), Compression::Gzip, &signer, LIMITS).expect("signed");
    let jws = json(&outer);
    assert_eq!(
        gunzip(&unb64(&jws["payload"])),
        signed_base(),
        "the payload is the gzip of the signed vCon"
    );
    assert_eq!(
        unb64(&jws["signatures"][0]["protected"]),
        br#"{"cty":"application/vcon+gzip"}"#,
        "marked gzip"
    );
}

#[test]
fn only_a_signed_vcon_is_signed_again() {
    let signer = TestSigner::x5c();
    let unsigned = base().to_json().expect("unsigned");
    let encrypted = encrypt(
        &signed_base(),
        Compression::None,
        &TestContent::new(),
        &[&TestKey::new(0x5a)],
        LIMITS,
    )
    .expect("encrypted");
    for (case, input) in [
        ("unsigned", unsigned),
        ("encrypted", encrypted),
        ("not JSON", b"not a vCon".to_vec()),
    ] {
        assert_eq!(
            sign_again(&input, Compression::None, &signer, LIMITS),
            Err(WriteError::NotSigned),
            "{case} is refused"
        );
    }
}

/// A body with the members of both the signed and the encrypted form is
/// ambiguous to `inspect`, so it is not a signed vCon here either.
#[test]
fn a_body_with_both_signed_and_encrypted_members_is_refused() {
    let mut jws = json(&signed_base());
    jws["ciphertext"] = json!("AA");
    jws["recipients"] = json!([]);
    let bytes = serde_json::to_vec(&jws).expect("JSON");
    let signer = TestSigner::x5c();
    assert_eq!(
        sign_again(&bytes, Compression::None, &signer, LIMITS),
        Err(WriteError::NotSigned),
        "an ambiguous body is refused for signing again"
    );
    assert_eq!(
        encrypt(
            &bytes,
            Compression::None,
            &TestContent::new(),
            &[&TestKey::new(1)],
            LIMITS
        ),
        Err(WriteError::NotSigned),
        "and for encrypting"
    );
}

/// `inspect` refuses a JWS whose header names another uuid than the vCon
/// it signs, so it is not a signed vCon to sign again or encrypt.
#[test]
fn a_signed_vcon_whose_header_names_another_uuid_is_refused() {
    let mut jws = json(&signed_base());
    jws["signatures"][0]["header"]["uuid"] = json!("01a07da8-c2bb-83e5-b9a2-279e0d16bc47");
    let bytes = serde_json::to_vec(&jws).expect("JSON");
    let signer = TestSigner::x5c();
    assert_eq!(
        sign_again(&bytes, Compression::None, &signer, LIMITS),
        Err(WriteError::NotSigned),
        "a header uuid that is not the vCon's is refused for signing again"
    );
    assert_eq!(
        encrypt(
            &bytes,
            Compression::None,
            &TestContent::new(),
            &[&TestKey::new(1)],
            LIMITS
        ),
        Err(WriteError::NotSigned),
        "and for encrypting"
    );
}

/// The input is read under the caller's limits, as `inspect` reads it.
#[test]
fn the_input_is_read_within_the_callers_limits() {
    let signer = TestSigner::x5c();
    let gzipped = sign(&base(), date(SIGNED_AT), Compression::Gzip, &[&signer]).expect("signed");
    let small = Limits::new(16);
    assert_eq!(
        sign_again(&gzipped, Compression::None, &signer, small),
        Err(WriteError::NotSigned),
        "a payload that inflates past the limit is refused for signing again"
    );
    assert_eq!(
        encrypt(
            &gzipped,
            Compression::None,
            &TestContent::new(),
            &[&TestKey::new(1)],
            small
        ),
        Err(WriteError::NotSigned),
        "and for encrypting"
    );
    assert!(
        sign_again(&gzipped, Compression::None, &signer, LIMITS).is_ok(),
        "within the limit it is signed again"
    );
}

#[test]
fn the_sixty_fourth_layer_is_written_and_the_sixty_fifth_refused() {
    let signer = TestSigner::x5c();
    let mut layers = signed_base();
    // Bounded in size and in time: with gzip a layer adds a few hundred
    // bytes, while a layer written without it is 4/3 the size of the one it
    // signs, so 64 of them would take gigabytes; the size bound stops such a
    // run within a few layers, and the deadline stops any other slow one.
    let started = std::time::Instant::now();
    for n in 2..=MAX_SIGNED_LAYERS {
        layers = sign_again(&layers, Compression::Gzip, &signer, LIMITS)
            .unwrap_or_else(|_| panic!("layer {n} is refused"));
        assert!(
            layers.len() < 256 * 1024,
            "layer {n} passed 256 KiB: each gzip layer adds a few hundred bytes"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(120),
            "64 small layers are written in far less than 120 s"
        );
    }
    assert_eq!(
        found(&layers),
        (Form::Signed, false, id()),
        "inspect reads all 64 layers"
    );
    assert_eq!(
        sign_again(&layers, Compression::Gzip, &signer, LIMITS),
        Err(WriteError::TooManyLayers),
        "a 65th layer is refused"
    );
}

// --- WRT-13: the encrypted form ---

#[test]
fn a_signed_vcon_is_encrypted_as_rfc_7516_writes_it() {
    let signed = signed_base();
    let content = TestContent::new();
    let bytes = encrypt(
        &signed,
        Compression::None,
        &content,
        &[&TestKey::new(0x5a)],
        LIMITS,
    )
    .expect("encrypted");
    let jwe = json(&bytes);
    let members: Vec<&str> = jwe
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        members.len(),
        5,
        "unprotected, recipients, iv, ciphertext and tag, no protected"
    );
    assert_eq!(
        jwe["unprotected"],
        json!({"cty": "application/vcon", "enc": "A256CBC-HS512", "uuid": ID}),
        "cty, enc and the innermost uuid"
    );
    assert_eq!(
        jwe["recipients"],
        json!([{"header": {"alg": "RSA-OAEP"}, "encrypted_key": b64(&xor(&CEK, &[0x5a]))}]),
        "the key wrapped for the one recipient"
    );
    assert_eq!(
        (&jwe["iv"], &jwe["tag"]),
        (&json!(b64(&IV)), &json!(b64(&TAG))),
        "the sealer's iv and tag"
    );
    assert_eq!(
        open(&jwe, 0, 0x5a),
        signed,
        "the plaintext is the signed vCon"
    );
    assert_eq!(
        content
            .aads
            .lock()
            .expect("the lock is not poisoned")
            .clone(),
        vec![Vec::<u8>::new()],
        "with no protected header the AAD is empty"
    );
    assert_eq!(
        found(&bytes),
        (Form::Encrypted, false, id()),
        "inspect reads its uuid"
    );
}

#[test]
fn each_recipient_opens_the_same_plaintext() {
    let signed = signed_base();
    let bytes = encrypt(
        &signed,
        Compression::None,
        &TestContent::new(),
        &[&TestKey::new(0x5a), &TestKey::new(0xa5)],
        LIMITS,
    )
    .expect("encrypted");
    let jwe = json(&bytes);
    assert_eq!(open(&jwe, 0, 0x5a), signed, "the first recipient opens it");
    assert_eq!(open(&jwe, 1, 0xa5), signed, "the second recipient opens it");
}

#[test]
fn a_gzip_plaintext_inflates_to_the_signed_vcon_and_is_marked_gzip() {
    let signed = signed_base();
    let bytes = encrypt(
        &signed,
        Compression::Gzip,
        &TestContent::new(),
        &[&TestKey::new(0x5a)],
        LIMITS,
    )
    .expect("encrypted");
    let jwe = json(&bytes);
    assert_eq!(
        gunzip(&open(&jwe, 0, 0x5a)),
        signed,
        "the plaintext is the gzip"
    );
    assert_eq!(
        jwe["unprotected"]["cty"],
        json!("application/vcon+gzip"),
        "marked gzip"
    );
}

#[test]
fn only_a_signed_vcon_is_encrypted() {
    let unsigned = base().to_json().expect("unsigned");
    assert_eq!(
        encrypt(
            &unsigned,
            Compression::None,
            &TestContent::new(),
            &[&TestKey::new(1)],
            LIMITS
        ),
        Err(WriteError::NotSigned),
        "an unsigned vCon is signed first"
    );
}

#[test]
fn encrypting_needs_a_recipient_and_working_encrypters() {
    let signed = signed_base();
    assert_eq!(
        encrypt(&signed, Compression::None, &TestContent::new(), &[], LIMITS),
        Err(WriteError::NoRecipient),
        "no recipient"
    );
    let no_key = TestContent {
        fails_key: true,
        ..TestContent::new()
    };
    assert_eq!(
        encrypt(
            &signed,
            Compression::None,
            &no_key,
            &[&TestKey::new(1)],
            LIMITS
        ),
        Err(WriteError::Crypto(CryptoError::new("no random source"))),
        "new_key's error"
    );
    let no_seal = TestContent {
        fails_seal: true,
        ..TestContent::new()
    };
    assert_eq!(
        encrypt(
            &signed,
            Compression::None,
            &no_seal,
            &[&TestKey::new(1)],
            LIMITS
        ),
        Err(WriteError::Crypto(CryptoError::new("the cipher refused"))),
        "seal's error"
    );
    let no_wrap = TestKey {
        fails: true,
        ..TestKey::new(1)
    };
    assert_eq!(
        encrypt(
            &signed,
            Compression::None,
            &TestContent::new(),
            &[&no_wrap],
            LIMITS
        ),
        Err(WriteError::Crypto(CryptoError::new(
            "the test recipient has no key"
        ))),
        "encrypt_key's error"
    );
}

// --- WRT-16: the same inputs, the same bytes ---

#[test]
fn the_same_inputs_are_signed_and_encrypted_as_the_same_bytes() {
    assert_eq!(signed_base(), signed_base(), "signing is deterministic");
    let encrypted = || {
        encrypt(
            &signed_base(),
            Compression::Gzip,
            &TestContent::new(),
            &[&TestKey::new(9)],
            LIMITS,
        )
        .expect("encrypted")
    };
    assert_eq!(
        encrypted(),
        encrypted(),
        "with the same key and IV from the encrypter, the same JWE"
    );
}

// --- Errors ---

#[test]
fn the_new_refusals_say_what_is_wrong() {
    let cases = [
        (WriteError::NotSigned, "the input is not a signed vCon"),
        (
            WriteError::TooManyLayers,
            "signing again would pass 64 signed layers",
        ),
        (WriteError::NoSigner, "no signer was given"),
        (WriteError::NoRecipient, "no recipient was given"),
        (
            WriteError::BadChain,
            "a signer's chain is an empty x5c or an x5u that is not https",
        ),
        (
            WriteError::Crypto(CryptoError::new("the key is not loaded")),
            "a signer or encrypter failed: the key is not loaded",
        ),
    ];
    for (e, text) in cases {
        assert_eq!(e.to_string(), text, "the message of {e:?}");
    }
}

#[test]
fn a_crypto_refusal_has_the_crypto_error_as_its_source() {
    let e = WriteError::Crypto(CryptoError::new("the key is not loaded"));
    let source = std::error::Error::source(&e).expect("a source");
    assert_eq!(
        source.to_string(),
        "the key is not loaded",
        "the encrypter's own message"
    );
    assert!(
        std::error::Error::source(&WriteError::BadHash).is_none(),
        "other refusals have none"
    );
}

// --- WRT-14: every form reads back ---

proptest! {
    #[test]
    fn every_form_reads_back_with_its_form_gzip_flag_and_uuid(
        bits in any::<u128>(),
        gzip in any::<bool>(),
        signers in 1usize..4,
        again in 0usize..3,
        encrypted in any::<bool>(),
    ) {
        let uuid = Uuid::from_u128(bits);
        let compression = if gzip { Compression::Gzip } else { Compression::None };
        let all: Vec<TestSigner> = (0..signers).map(|_| TestSigner::x5c()).collect();
        let refs: Vec<&dyn Signer> = all.iter().map(|s| s as &dyn Signer).collect();
        let mut bytes = sign(&Vcon::new(uuid, date(CREATED)), date(SIGNED_AT), compression, &refs)
            .expect("signed");
        for _ in 0..again {
            bytes = sign_again(&bytes, compression, &all[0], LIMITS).expect("signed again");
        }
        let mut form = Form::Signed;
        if encrypted {
            bytes = encrypt(&bytes, compression, &TestContent::new(), &[&TestKey::new(7)], LIMITS)
                .expect("encrypted");
            form = Form::Encrypted;
        }
        let i = inspect(&bytes, None, &LIMITS, &BTreeSet::new()).expect("inspect accepts it");
        prop_assert_eq!((i.form, i.gzip, i.uuid), (form, false, uuid), "read back as written");
    }
}
