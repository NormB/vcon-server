//! Bodies nested far deeper than any vCon: signed layers inside signed
//! layers, and JSON arrays and objects inside each other. Reading one must
//! not overflow the stack, and nesting past the fixed limits is refused as
//! too deep. A stack overflow aborts the whole process, so each deep read
//! runs in a child process, on a thread with a small stack, and this process
//! looks at how the child ended.

use std::collections::BTreeSet;
use std::io::Write;
use std::process::Command;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use uuid::Uuid;
use vcon::Form;
use vcon::read::{Limits, MAX_JSON_DEPTH, MAX_SIGNED_LAYERS, Rejection, inspect};

const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
const OTHER: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc47";

/// No inflation limit to stop the read early, as with a server configured
/// with a high `limits.inflated`.
const UNLIMITED: Limits = Limits::new(u64::MAX);

/// Far past the limit, and deep enough that reading one stack frame per
/// layer overflows `SMALL_STACK` in a debug build, as the commit hook and
/// CI run the tests.
const DEEP: usize = 500;

/// A quarter of the 2 MiB a Rust thread gets by default: room for one read
/// at the limits in a debug build on x86_64 (64 KiB was not), far less than
/// a frame per layer of `DEEP` needs.
const SMALL_STACK: usize = 512 << 10;

fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::fast());
    e.write_all(bytes).expect("compress");
    e.finish().expect("finish")
}

fn unsigned() -> Value {
    json!({"uuid": ID, "created_at": "2026-10-26T00:00:00Z"})
}

/// A JWS whose payload is `inner`, gzipped or not, with one signature
/// whose header names `header_uuid`, or no header.
fn sign(inner: &[u8], zip: bool, header_uuid: Option<&str>) -> Vec<u8> {
    let payload = if zip { gz(inner) } else { inner.to_vec() };
    let signature = match header_uuid {
        Some(uuid) => json!({"header": {"uuid": uuid}}),
        None => json!({}),
    };
    json!({"payload": URL_SAFE_NO_PAD.encode(payload), "signatures": [signature]})
        .to_string()
        .into_bytes()
}

/// An unsigned vCon inside `layers` signed layers, each payload gzipped and
/// each signature empty, so the body grows only a little with each layer.
fn nested(layers: usize) -> Vec<u8> {
    let mut body = unsigned().to_string().into_bytes();
    for _ in 0..layers {
        body = sign(&body, true, None);
    }
    body
}

/// An unsigned vCon whose JSON nests `depth` deep, the vCon object itself
/// counting as one: a member holding arrays and objects in turn.
fn deep_json(depth: usize) -> Vec<u8> {
    let inner = depth.checked_sub(1).expect("the vCon object is one level");
    let (mut open, mut close) = (String::new(), String::new());
    for level in 0..inner {
        if level % 2 == 0 {
            open.push('[');
            close.push(']');
        } else {
            open.push_str(r#"{"a":"#);
            close.push('}');
        }
    }
    let close: String = close.chars().rev().collect();
    format!(r#"{{"uuid":"{ID}","created_at":"2026-10-26T00:00:00Z","x":{open}0{close}}}"#)
        .into_bytes()
}

/// What `inspect` found, as its fields, which a test outside the crate can
/// read but not build.
fn run(body: &[u8]) -> Result<(Form, bool, Uuid), Rejection> {
    inspect(body, None, &UNLIMITED, &BTreeSet::new()).map(|i| (i.form, i.gzip, i.uuid))
}

fn found(form: Form, gzip: bool) -> Result<(Form, bool, Uuid), Rejection> {
    Ok((form, gzip, ID.parse().expect("test uuid")))
}

/// Reads `body` in a thread with a small stack; returns only if the read
/// did not overflow it, and fails unless it refused the body as too deep.
fn read_on_a_small_stack(body: Vec<u8>) {
    let reader = std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(move || run(&body))
        .expect("spawn the reader");
    assert_eq!(
        reader.join().expect("the reader returns"),
        Err(Rejection::TooDeep),
        "both bodies are past a nesting limit"
    );
}

/// Runs the ignored test `name` of this binary in a child process and
/// fails unless it ran and passed.
fn passes_in_a_child(name: &str) {
    let child = Command::new(std::env::current_exe().expect("this test binary"))
        .args(["--ignored", "--exact", name, "--test-threads=1"])
        .output()
        .expect("run the child");
    let stdout = String::from_utf8_lossy(&child.stdout);
    let stderr = String::from_utf8_lossy(&child.stderr);
    assert!(
        child.status.success() && stdout.contains("1 passed"),
        "{name} ended with {}:\n{stderr}",
        child.status
    );
}

#[test]
#[ignore = "run in a child process by a_deep_signed_body_does_not_overflow_the_stack"]
fn child_reads_deep_signed_layers_on_a_small_stack() {
    read_on_a_small_stack(nested(DEEP));
}

#[test]
#[ignore = "run in a child process by deep_json_does_not_overflow_the_stack"]
fn child_reads_deep_json_on_a_small_stack() {
    read_on_a_small_stack(deep_json(100_000));
}

#[test]
fn a_deep_signed_body_does_not_overflow_the_stack() {
    passes_in_a_child("child_reads_deep_signed_layers_on_a_small_stack");
}

#[test]
fn deep_json_does_not_overflow_the_stack() {
    passes_in_a_child("child_reads_deep_json_on_a_small_stack");
}

#[test]
fn signed_layers_up_to_the_limit_are_read() {
    assert_eq!(
        run(&nested(1)),
        found(Form::Signed, false),
        "one signed layer is the plain signed form"
    );
    let full = nested(MAX_SIGNED_LAYERS);
    assert_eq!(
        run(&full),
        found(Form::Signed, false),
        "exactly MAX_SIGNED_LAYERS layers is within the limit"
    );
    assert_eq!(run(&gz(&full)), found(Form::Signed, true), "body gzipped");
}

#[test]
fn one_signed_layer_past_the_limit_is_too_deep() {
    let over = nested(MAX_SIGNED_LAYERS + 1);
    assert_eq!(
        run(&over),
        Err(Rejection::TooDeep),
        "one layer more than MAX_SIGNED_LAYERS is refused"
    );
    assert_eq!(run(&gz(&over)), Err(Rejection::TooDeep), "body gzipped");
    assert_eq!(
        run(&nested(2 * MAX_SIGNED_LAYERS)),
        Err(Rejection::TooDeep),
        "far past the limit is refused the same way"
    );
}

#[test]
fn the_depth_is_refused_before_what_lies_deeper() {
    // Past the limit nothing more is read: not the inner vCon, and not the
    // header uuids, which are compared only once the vCon's uuid is known.
    let mut no_uuid = json!({"created_at": "2026-10-26T00:00:00Z"})
        .to_string()
        .into_bytes();
    let mut mismatch = unsigned().to_string().into_bytes();
    for _ in 0..=MAX_SIGNED_LAYERS {
        no_uuid = sign(&no_uuid, true, None);
        mismatch = sign(&mismatch, true, Some(OTHER));
    }
    assert_eq!(
        run(&no_uuid),
        Err(Rejection::TooDeep),
        "the missing uuid lies past the limit, so it is never reached"
    );
    assert_eq!(
        run(&mismatch),
        Err(Rejection::TooDeep),
        "header uuids are compared once the vCon's uuid is read, which is past the limit"
    );
    // Within the limit, the same vCons are refused for what they are.
    let mut within = unsigned().to_string().into_bytes();
    for _ in 0..MAX_SIGNED_LAYERS {
        within = sign(&within, true, Some(OTHER));
    }
    assert_eq!(
        run(&within),
        Err(Rejection::UuidMismatch),
        "within the limit the inner vCon is read: its uuid differs from the headers'"
    );
}

#[test]
fn json_up_to_the_limit_is_read() {
    assert_eq!(
        run(&deep_json(MAX_JSON_DEPTH)),
        found(Form::Unsigned, false),
        "JSON nested exactly MAX_JSON_DEPTH deep is within the limit"
    );
    let signed = sign(&deep_json(MAX_JSON_DEPTH), true, Some(ID));
    assert_eq!(
        run(&signed),
        found(Form::Signed, false),
        "the limit counts within each JSON text, so a payload as deep is read too"
    );
}

#[test]
fn json_one_level_past_the_limit_is_too_deep() {
    let over = deep_json(MAX_JSON_DEPTH + 1);
    assert_eq!(
        run(&over),
        Err(Rejection::TooDeep),
        "one level past MAX_JSON_DEPTH is refused"
    );
    assert_eq!(run(&gz(&over)), Err(Rejection::TooDeep), "body gzipped");
    let signed = sign(&over, true, Some(ID));
    assert_eq!(run(&signed), Err(Rejection::TooDeep), "in a signed payload");
    let unprotected = String::from_utf8(deep_json(MAX_JSON_DEPTH)).expect("text");
    let encrypted = format!(r#"{{"ciphertext":"","recipients":[],"unprotected":{unprotected}}}"#);
    assert_eq!(
        run(encrypted.as_bytes()),
        Err(Rejection::TooDeep),
        "encrypted"
    );
}

#[test]
fn the_json_limit_is_below_the_parsers_own() {
    // serde_json refuses its 128th nested array or object as a syntax
    // error. The limit must refuse first, so depth gets its own answer.
    const {
        assert!(
            MAX_JSON_DEPTH < 127,
            "the depth limit must refuse before serde_json's own bound"
        )
    };
    assert_eq!(
        run(&deep_json(127)),
        Err(Rejection::TooDeep),
        "at serde_json's bound the answer is still too deep, not not JSON"
    );
    assert_eq!(
        run(&deep_json(100_000)),
        Err(Rejection::TooDeep),
        "a depth that would overflow a recursive parser is refused as too deep"
    );
}
