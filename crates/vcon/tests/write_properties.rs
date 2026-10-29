//! Properties of `vcon::write` over generated object models: every output
//! reads back with `read::inspect` as the form, gzip flag and uuid it was
//! written with, and the unsigned JSON parsed back equals the model.
//!
//! Each generated model is built twice: once with the writer, and once as
//! the JSON value core-04 says it stands for, built here member by member.

use std::collections::BTreeSet;
use std::io::Read;

use flate2::read::GzDecoder;
use proptest::prelude::{Just, Strategy, any, prop, prop_assert_eq, prop_oneof, proptest};
use serde_json::{Map, Value, json};
use uuid::Uuid;
use vcon::Form;
use vcon::read::{Limits, inspect};
use vcon::write::{
    Analysis, Attachment, Body, Content, ContentHash, Date, Dialog, DialogIndex, DialogType,
    Disposition, Parties, Party, PartyIndex, Vcon,
};

const DATES: [&str; 3] = [
    "2022-06-21T13:53:00-04:00",
    "2026-10-27T00:00:00Z",
    "2022-06-21T17:53:26.000+00:00",
];
const MEMBER_NAMES: [&str; 3] = ["x_one", "x_two", "x_three"];
const EXTENSIONS: [&str; 2] = ["ext_one", "ext_two"];

#[derive(Debug, Clone)]
struct PartyDesc {
    name: Option<String>,
    tel: Option<String>,
    mailto: Option<String>,
}

#[derive(Debug, Clone)]
enum DialogDesc {
    Text { body: String, party: u8 },
    Recording { path: String },
    Incomplete { disposition: u8 },
    Transfer { party: u8 },
    Placeholder,
}

#[derive(Debug, Clone)]
struct Model {
    uuid: u128,
    created_at: usize,
    parties: Vec<PartyDesc>,
    dialog: Vec<DialogDesc>,
    attachments: Vec<(String, u8, u8)>,
    analysis: Vec<(String, String, Value)>,
    extensions: Vec<(usize, bool)>,
    members: Vec<(usize, Value)>,
}

fn text() -> impl Strategy<Value = String> {
    // Any characters, so escaping is exercised: quotes, controls, and
    // characters outside the Basic Multilingual Plane.
    ".{0,12}"
}

fn value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        text().prop_map(Value::from),
    ];
    leaf.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..3).prop_map(Value::Array),
            prop::collection::btree_map("[a-z]{1,4}", inner, 0..3)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn model() -> impl Strategy<Value = Model> {
    let party = (
        prop::option::of(text()),
        prop::option::of(text()),
        prop::option::of(text()),
    )
        .prop_map(|(name, tel, mailto)| PartyDesc { name, tel, mailto });
    let dialog = prop_oneof![
        (text(), any::<u8>()).prop_map(|(body, party)| DialogDesc::Text { body, party }),
        "[a-z]{1,8}".prop_map(|path| DialogDesc::Recording { path }),
        any::<u8>().prop_map(|disposition| DialogDesc::Incomplete { disposition }),
        any::<u8>().prop_map(|party| DialogDesc::Transfer { party }),
        Just(DialogDesc::Placeholder),
    ];
    (
        any::<u128>(),
        0..DATES.len(),
        prop::collection::vec(party, 0..4),
        prop::collection::vec(dialog, 0..5),
        prop::collection::vec((text(), any::<u8>(), any::<u8>()), 0..3),
        prop::collection::vec((text(), text(), value()), 0..3),
        prop::collection::vec((0..EXTENSIONS.len(), any::<bool>()), 0..3),
        prop::collection::vec((0..MEMBER_NAMES.len(), value()), 0..4),
    )
        .prop_map(
            |(uuid, created_at, parties, dialog, attachments, analysis, extensions, members)| {
                Model {
                    uuid,
                    created_at,
                    parties,
                    dialog,
                    attachments,
                    analysis,
                    extensions,
                    members,
                }
            },
        )
}

const DISPOSITIONS: [(Disposition, &str); 6] = [
    (Disposition::NoAnswer, "no-answer"),
    (Disposition::Congestion, "congestion"),
    (Disposition::Failed, "failed"),
    (Disposition::Busy, "busy"),
    (Disposition::HungUp, "hung-up"),
    (Disposition::VoicemailNoMessage, "voicemail-no-message"),
];

fn hash() -> ContentHash {
    ContentHash::try_new("sha512", &[1, 2, 3]).expect("hash")
}

/// The model built with the writer, and the JSON value it stands for.
fn build(m: &Model) -> (Vcon, Value, BTreeSet<String>) {
    let uuid = Uuid::from_u128(m.uuid);
    let created = DATES[m.created_at];
    let mut v = Vcon::new(
        uuid,
        created
            .parse::<Date>()
            .expect("a generated date is RFC 3339"),
    );
    let mut top = Map::new();
    top.insert("uuid".into(), json!(uuid.to_string()));

    let mut extensions: Vec<String> = Vec::new();
    let mut critical: Vec<String> = Vec::new();
    for (i, is_critical) in &m.extensions {
        let name = EXTENSIONS[*i];
        if *is_critical {
            v.use_critical_extension(name);
        } else {
            v.use_extension(name);
        }
        if !extensions.iter().any(|e| e == name) {
            extensions.push(name.into());
        }
        if *is_critical && !critical.iter().any(|e| e == name) {
            critical.push(name.into());
        }
    }
    if !extensions.is_empty() {
        top.insert("extensions".into(), json!(extensions));
    }
    if !critical.is_empty() {
        top.insert("critical".into(), json!(critical));
    }
    top.insert("created_at".into(), json!(created));

    let mut parties = Vec::new();
    for p in &m.parties {
        let mut party = Party::new();
        let mut want = Map::new();
        if let Some(t) = &p.tel {
            party = party.tel(t.clone());
            want.insert("tel".into(), json!(t));
        }
        if let Some(t) = &p.mailto {
            party = party.mailto(t.clone());
            want.insert("mailto".into(), json!(t));
        }
        if let Some(t) = &p.name {
            party = party.name(t.clone());
            want.insert("name".into(), json!(t));
        }
        v.add_party(party);
        parties.push(Value::Object(want));
    }
    let n_parties = m.parties.len();

    let mut dialog = Vec::new();
    for d in &m.dialog {
        let (made, want) = match d {
            DialogDesc::Text { body, party } => {
                let mut made =
                    Dialog::text(Content::Inline(Body::Text(body.clone()))).mediatype("text/plain");
                let mut want = json!({"type": "text", "mediatype": "text/plain",
                                      "body": body, "encoding": "none"});
                if n_parties > 0 {
                    let i = usize::from(*party) % n_parties;
                    made = made.parties(Parties::One(PartyIndex::new(i)));
                    want["parties"] = json!(i);
                }
                (made, want)
            }
            DialogDesc::Recording { path } => {
                let url = format!("https://example.com/{path}.wav");
                let made = Dialog::recording(Content::External {
                    url: url.clone(),
                    hashes: vec![hash()],
                });
                let want = json!({"type": "recording", "url": url,
                                  "content_hash": hash().as_str()});
                (made, want)
            }
            DialogDesc::Incomplete { disposition } => {
                let (d, text) = DISPOSITIONS[usize::from(*disposition) % DISPOSITIONS.len()];
                (
                    Dialog::incomplete(d),
                    json!({"type": "incomplete", "disposition": text}),
                )
            }
            DialogDesc::Transfer { party } => {
                let mut made = Dialog::transfer();
                let mut want = json!({"type": "transfer"});
                if n_parties > 0 {
                    let i = usize::from(*party) % n_parties;
                    made = made.transferee(PartyIndex::new(i));
                    want["transferee"] = json!(i);
                }
                (made, want)
            }
            DialogDesc::Placeholder => (
                Dialog::try_placeholder(DialogType::Recording).expect("placeholder"),
                json!({"type": "recording"}),
            ),
        };
        v.add_dialog(made);
        dialog.push(want);
    }
    let n_dialog = m.dialog.len();

    let mut attachments = Vec::new();
    if n_parties > 0 && n_dialog > 0 {
        for (purpose, party, d) in &m.attachments {
            let (p, d) = (usize::from(*party) % n_parties, usize::from(*d) % n_dialog);
            v.add_attachment(
                Attachment::new(
                    created
                        .parse::<Date>()
                        .expect("a generated date is RFC 3339"),
                    PartyIndex::new(p),
                    DialogIndex::new(d),
                    Content::Inline(Body::Bytes(purpose.as_bytes().to_vec())),
                )
                .purpose(purpose.clone())
                .mediatype("application/octet-stream"),
            );
            use base64::Engine;
            let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(purpose);
            attachments.push(json!({"purpose": purpose, "start": created, "party": p,
                                    "dialog": d, "mediatype": "application/octet-stream",
                                    "body": body, "encoding": "base64url"}));
        }
    }

    let mut analysis = Vec::new();
    for (kind, vendor, body) in &m.analysis {
        v.add_analysis(Analysis::new(
            kind.clone(),
            vendor.clone(),
            Content::Inline(Body::Json(body.clone())),
        ));
        analysis.push(json!({"type": kind, "vendor": vendor, "body": body, "encoding": "json"}));
    }

    top.insert("parties".into(), Value::Array(parties));
    top.insert("dialog".into(), Value::Array(dialog));
    top.insert("analysis".into(), Value::Array(analysis));
    top.insert("attachments".into(), Value::Array(attachments));
    for (i, value) in &m.members {
        let name = MEMBER_NAMES[*i];
        v.set_member(name, value.clone()).expect("member");
        top.insert(name.into(), value.clone());
    }
    (v, Value::Object(top), critical.into_iter().collect())
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    GzDecoder::new(bytes).read_to_end(&mut out).expect("gzip");
    out
}

proptest! {
    #[test]
    fn every_model_is_written_as_the_json_it_stands_for_and_reads_back(m in model()) {
        let (v, expected, supported) = build(&m);
        let limits = Limits::new(1 << 20);
        let json = v.to_json().expect("every generated model is valid");
        let back: Value = serde_json::from_slice(&json).expect("JSON");
        prop_assert_eq!(&back, &expected, "the JSON parsed back equals the model");

        let unsigned = inspect(&json, None, &limits, &supported).expect("inspect accepts it");
        prop_assert_eq!(
            (unsigned.form, unsigned.gzip, unsigned.uuid),
            (Form::Unsigned, false, v.uuid()), "inspect reads the unsigned form with its uuid");

        let gz = v.to_gzip().expect("gzip");
        prop_assert_eq!(gunzip(&gz), json, "the gzip form inflates to the unsigned form");
        let gzipped = inspect(&gz, None, &limits, &supported).expect("inspect accepts it");
        prop_assert_eq!(
            (gzipped.form, gzipped.gzip, gzipped.uuid),
            (Form::Unsigned, true, v.uuid()), "inspect reads the gzip form with its uuid");
    }
}
