//! `inspect` through the crate's public API only.

use std::collections::BTreeSet;
use std::io::Write;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use vcon_form::{Form, Inspection, Limits, Rejection, inspect};

const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
const LIMITS: Limits = Limits { inflated: 1 << 20 };

fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::default());
    e.write_all(bytes).expect("compress");
    e.finish().expect("finish")
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unsigned() -> Value {
    json!({"uuid": ID, "created_at": "2026-10-23T00:00:00Z", "parties": []})
}

fn sign(inner: &[u8]) -> Value {
    json!({"payload": b64(inner), "signatures": [{"header": {"uuid": ID}}]})
}

fn run(body: &[u8]) -> Result<Inspection, Rejection> {
    inspect(body, None, &LIMITS, &BTreeSet::new())
}

fn ok(form: Form, gzip: bool) -> Result<Inspection, Rejection> {
    Ok(Inspection {
        form,
        gzip,
        uuid: ID.parse().expect("test uuid"),
    })
}

#[test]
fn reads_each_form() {
    let text = unsigned().to_string();
    assert_eq!(run(text.as_bytes()), ok(Form::Unsigned, false));
    assert_eq!(run(&gz(text.as_bytes())), ok(Form::Unsigned, true));
    assert_eq!(
        run(sign(text.as_bytes()).to_string().as_bytes()),
        ok(Form::Signed, false)
    );
    let encrypted = json!({"unprotected": {"uuid": ID}, "ciphertext": "x", "recipients": []});
    assert_eq!(
        run(encrypted.to_string().as_bytes()),
        ok(Form::Encrypted, false)
    );
}

#[test]
fn reads_the_ietf_examples() {
    let cases: [(&[u8], Form); 3] = [
        (
            include_bytes!("fixtures/ietf/ab_call_ext_rec.vcon"),
            Form::Unsigned,
        ),
        (
            include_bytes!("fixtures/ietf/ab_call_ext_rec_signed.vcon"),
            Form::Signed,
        ),
        (
            include_bytes!("fixtures/ietf/ab_call_ext_rec_encrypted.vcon"),
            Form::Encrypted,
        ),
    ];
    for (body, form) in cases {
        assert_eq!(run(body), ok(form, false), "{form:?}");
    }
}

#[test]
fn every_rejection_is_reachable() {
    let text = unsigned().to_string();
    let critical = json!({"uuid": ID, "created_at": "x", "critical": ["x"]}).to_string();
    let cases: Vec<(&str, Vec<u8>, Rejection)> = vec![
        ("too large", gz(&vec![b' '; (LIMITS.inflated + 1) as usize]), Rejection::TooLarge),
        ("bad gzip", vec![0x1f, 0x8b, 0], Rejection::BadGzip),
        ("not JSON", b"[]".to_vec(), Rejection::NotJson),
        ("duplicate key", br#"{"uuid": "a", "uuid": "b"}"#.to_vec(), Rejection::DuplicateKey),
        (
            "ambiguous form",
            br#"{"payload": "", "signatures": [], "ciphertext": "", "recipients": []}"#.to_vec(),
            Rejection::AmbiguousForm,
        ),
        ("no form", b"{}".to_vec(), Rejection::NoForm),
        ("missing uuid", br#"{"parties": []}"#.to_vec(), Rejection::MissingUuid),
        ("bad uuid", br#"{"uuid": "x"}"#.to_vec(), Rejection::BadUuid),
        ("missing created_at", format!(r#"{{"uuid": "{ID}"}}"#).into_bytes(), Rejection::MissingCreatedAt),
        ("bad critical", format!(r#"{{"uuid": "{ID}", "created_at": "x", "critical": 1}}"#).into_bytes(), Rejection::BadCritical),
        ("unsupported critical", critical.into_bytes(), Rejection::UnsupportedCritical(vec!["x".to_owned()])),
        ("bad payload", br#"{"payload": "%", "signatures": []}"#.to_vec(), Rejection::BadPayload),
        (
            "uuid mismatch",
            json!({"payload": b64(text.as_bytes()), "signatures": [{"header": {"uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc47"}}]})
                .to_string()
                .into_bytes(),
            Rejection::UuidMismatch,
        ),
    ];
    for (what, body, expected) in cases {
        assert_eq!(run(&body), Err(expected), "{what}");
    }
}

#[test]
fn the_media_type_and_supported_list_are_used() {
    let text = unsigned().to_string();
    assert_eq!(
        inspect(
            text.as_bytes(),
            Some("application/vcon+gzip"),
            &LIMITS,
            &BTreeSet::new()
        ),
        Err(Rejection::BadGzip)
    );
    let critical = json!({"uuid": ID, "created_at": "x", "critical": ["x"]}).to_string();
    let supported = BTreeSet::from(["x".to_owned()]);
    assert_eq!(
        inspect(critical.as_bytes(), None, &LIMITS, &supported),
        ok(Form::Unsigned, false)
    );
}

#[test]
fn the_body_and_every_payload_share_one_inflation_budget() {
    let padded = json!({"uuid": ID, "created_at": "x", "pad": " ".repeat(3000)}).to_string();
    let signed = json!({"payload": b64(&gz(padded.as_bytes())), "signatures": []}).to_string();
    let body = gz(signed.as_bytes());
    let total = (padded.len() + signed.len()) as u64;
    assert_eq!(
        inspect(&body, None, &Limits { inflated: total }, &BTreeSet::new()),
        ok(Form::Signed, true)
    );
    assert_eq!(
        inspect(
            &body,
            None,
            &Limits {
                inflated: total - 1
            },
            &BTreeSet::new()
        ),
        Err(Rejection::TooLarge)
    );
}
