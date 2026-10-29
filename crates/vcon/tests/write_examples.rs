//! The vCon core draft's own examples, built with `vcon::write`: A.4 with
//! the builder, A.8 by redacting it and A.9 by amending it. Each output
//! equals the draft's file as a JSON value; member order and whitespace
//! are the draft authors' and are not compared.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;
use uuid::Uuid;
use vcon::Form;
use vcon::read::{Limits, inspect};
use vcon::write::{
    Analysis, Body, Channel, Content, ContentHash, Date, Dialog, DialogIndex, Duration, Parties,
    Party, PartyIndex, Reference, Vcon,
};

fn fixture(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ietf")
        .join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&bytes).expect("the fixture is JSON")
}

fn uuid(text: &str) -> Uuid {
    text.parse().expect("example uuid")
}

fn date(text: &str) -> Date {
    text.parse().expect("an example date is RFC 3339")
}

const A4_UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
const RECORDING_HASH: &str =
    "sha512-GLy6IPaIUM1GqzZqfIPZlWjaDsNgNvZM0iCONNThnH0a75fhUM6cYzLZ5GynSURREvZwmOh54-2lRRieyj82UQ";
const RECORDING_URL: &str = "https://github.com/ietf-wg-vcon/draft-ietf-vcon-vcon-core/raw/refs/heads/main/examples/ab_call.mp3";

fn hash() -> ContentHash {
    RECORDING_HASH
        .parse::<ContentHash>()
        .expect("the draft's hash")
}

/// The recording of A.4 with its content left to the caller.
fn call(content: Content) -> Dialog {
    Dialog::recording(content)
        .start(date("2022-06-21T17:53:26.000+00:00"))
        .duration(Duration::try_from_secs_f64(33.12).expect("duration"))
        .parties(Parties::Channels(vec![
            Channel::One(PartyIndex::new(0)),
            Channel::One(PartyIndex::new(1)),
        ]))
        .mediatype("audio/x-mp3")
        .filename("ab_call.mp3")
}

/// Party `i`'s telephone number in A.4. The draft's illustrative numbers
/// are allowed only in the fixture directory, so they are read from there.
fn tel(i: usize) -> String {
    fixture("ab_call_ext_rec.vcon")["parties"][i]["tel"]
        .as_str()
        .expect("a tel")
        .to_owned()
}

/// A.4, Two Party Call vCon With Externally Referenced Recording.
fn a4() -> Vcon {
    let mut v = Vcon::new(uuid(A4_UUID), date("2022-06-21T13:53:00-04:00"));
    v.add_party(Party::new().tel(tel(0)).name("Alice"));
    v.add_party(Party::new().tel(tel(1)).name("Bob"));
    v.add_dialog(call(Content::External {
        url: RECORDING_URL.into(),
        hashes: vec![hash()],
    }));
    v
}

fn written(v: &Vcon) -> Value {
    serde_json::from_slice(&v.to_json().expect("written")).expect("JSON")
}

fn read_uuid(v: &Vcon) -> Uuid {
    let bytes = v.to_json().expect("written");
    let found = inspect(&bytes, None, &Limits::new(1 << 20), &BTreeSet::new()).expect("a vCon");
    assert_eq!(
        (found.form, found.gzip),
        (Form::Unsigned, false),
        "an unsigned vCon, not gzipped"
    );
    found.uuid
}

#[test]
fn a4_built_with_the_builder_equals_the_drafts_file() {
    let v = a4();
    assert_eq!(
        written(&v),
        fixture("ab_call_ext_rec.vcon"),
        "A.4 built with the builder equals the draft's file"
    );
    assert_eq!(read_uuid(&v), uuid(A4_UUID), "inspect reads A.4's uuid");
}

/// A.8, Redacted vCon: A.4 with the telephone numbers and the recording's
/// url redacted and a redacted transcript added. The transcript's body is
/// taken from the draft's file, since it is the analysis vendor's data, not
/// something this crate builds.
#[test]
fn a8_made_by_redacting_a4_equals_the_drafts_file() {
    let expected = fixture("ab_call_ext_rec_redacted.vcon");
    let mut v = Vcon::redact(
        &a4(),
        uuid("01928e10-193e-8231-b9a2-279e0d16bc46"),
        "PII Redaction",
        Reference::by_uuid(uuid(A4_UUID)),
    )
    .expect("redacted");
    v.replace_party(PartyIndex::new(0), Party::new().name("Alice"))
        .expect("party");
    v.replace_party(PartyIndex::new(1), Party::new().name("Bob"))
        .expect("party");
    v.replace_dialog(DialogIndex::new(0), call(Content::HashOnly(vec![hash()])))
        .expect("dialog");
    let transcript = expected["analysis"][0]["body"].clone();
    v.add_analysis(
        Analysis::new(
            "transcript-redacted",
            "CapitalOne",
            Content::Inline(Body::Json(transcript)),
        )
        .dialog(DialogIndex::new(0))
        .schema("data_labeler_schema")
        .product("dataprofiler"),
    );
    assert_eq!(
        written(&v),
        expected,
        "A.8 made by redacting A.4 equals the draft's file"
    );
    assert_eq!(
        read_uuid(&v),
        uuid("01928e10-193e-8231-b9a2-279e0d16bc46"),
        "inspect reads A.8's uuid"
    );
}

/// A.9, Amended Signed vCon: A.4 with a text dialog added.
#[test]
fn a9_made_by_amending_a4_equals_the_drafts_file() {
    let mut v = Vcon::amend(
        &a4(),
        uuid("01a07da8-c5c1-8db4-a20f-d188127fad29"),
        Reference::by_uuid(uuid(A4_UUID)),
    )
    .expect("amended");
    v.add_dialog(
        Dialog::text(Content::Inline(Body::Text("Good day!".into())))
            .start(date("2022-06-21T17:53:26.000+00:00"))
            .parties(Parties::Channels(vec![
                Channel::One(PartyIndex::new(1)),
                Channel::One(PartyIndex::new(0)),
            ]))
            .mediatype("text/plain"),
    );
    assert_eq!(
        written(&v),
        fixture("ab_call_ext_rec_amended.vcon"),
        "A.9 made by amending A.4 equals the draft's file"
    );
    assert_eq!(
        read_uuid(&v),
        uuid("01a07da8-c5c1-8db4-a20f-d188127fad29"),
        "inspect reads A.9's uuid"
    );
}

#[test]
fn the_link_objects_are_written_as_the_draft_shows_them() {
    let a8 = Vcon::redact(
        &a4(),
        uuid("01928e10-193e-8231-b9a2-279e0d16bc46"),
        "PII Redaction",
        Reference::by_uuid(uuid(A4_UUID)),
    )
    .expect("redacted");
    assert_eq!(
        written(&a8)["redacted"],
        fixture("ab_call_ext_rec_redacted.vcon")["redacted"],
        "the Redacted Object as A.8 shows it"
    );
    let a9 = Vcon::amend(
        &a4(),
        uuid("01a07da8-c5c1-8db4-a20f-d188127fad29"),
        Reference::by_uuid(uuid(A4_UUID)),
    )
    .expect("amended");
    assert_eq!(
        written(&a9)["amended"],
        fixture("ab_call_ext_rec_amended.vcon")["amended"],
        "the Amended Object as A.9 shows it"
    );
}
