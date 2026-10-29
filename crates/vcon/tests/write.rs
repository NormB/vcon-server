//! `vcon::write` through the crate's public API: each rule of the writer,
//! and every output read back with `read::inspect`.

use std::collections::BTreeSet;
use std::io::Read;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::read::GzDecoder;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};
use uuid::Uuid;
use vcon::Form;
use vcon::read::{Limits, MAX_JSON_DEPTH, Rejection, inspect};
use vcon::write::{
    Analysis, AnalysisIndex, Attachment, AttachmentIndex, Body, Channel, CivicAddress, Content,
    ContentHash, Date, Dialog, DialogIndex, DialogType, Disposition, Duration, Event, Parties,
    Party, PartyHistory, PartyIndex, Reference, SessionChannel, SessionId, SessionIds, Vcon,
    WriteError, uuid_v8,
};

const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
const PRIOR: &str = "01a07da8-c5c1-8db4-a20f-d188127fad29";
const CREATED: &str = "2022-06-21T13:53:00-04:00";
const LIMITS: Limits = Limits::new(1 << 20);

fn id() -> Uuid {
    ID.parse().expect("test uuid")
}

fn prior() -> Uuid {
    PRIOR.parse().expect("test uuid")
}

fn date(text: &str) -> Date {
    text.parse().expect("a test date is RFC 3339")
}

fn base() -> Vcon {
    Vcon::new(id(), date(CREATED))
}

fn sha512(bytes: &[u8]) -> ContentHash {
    ContentHash::try_new("sha512", bytes).expect("test hash")
}

fn sha256(bytes: &[u8]) -> ContentHash {
    ContentHash::try_new("sha256", bytes).expect("test hash")
}

fn external(url: &str) -> Content {
    Content::External {
        url: url.to_owned(),
        hashes: vec![sha512(&[1, 2, 3])],
    }
}

fn written(v: &Vcon) -> Result<Value, WriteError> {
    v.to_json()
        .map(|bytes| serde_json::from_slice(&bytes).expect("the output is JSON"))
}

fn json_of(v: &Vcon) -> Value {
    written(v).expect("the vCon is written")
}

/// What `inspect` finds in `body`, with no extension supported.
fn found(body: &[u8]) -> Result<(Form, bool, Uuid), Rejection> {
    inspect(body, None, &LIMITS, &BTreeSet::new()).map(|i| (i.form, i.gzip, i.uuid))
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .expect("the output is gzip");
    out
}

/// A vCon with two parties and one recording, which index tests refer to.
fn with_two_parties_and_a_recording() -> Vcon {
    let mut v = base();
    v.add_party(Party::new().name("Alice"));
    v.add_party(Party::new().name("Bob"));
    v.add_dialog(Dialog::recording(external("https://example.com/a.mp3")));
    v
}

// --- WRT-1: uuid, created_at and dates ---

#[test]
fn a_new_vcon_holds_its_uuid_its_creation_time_and_the_four_arrays() {
    let bytes = base().to_json().expect("written");
    assert_eq!(
        String::from_utf8(bytes).expect("UTF-8"),
        format!(
            r#"{{"uuid":"{ID}","created_at":"{CREATED}","parties":[],"dialog":[],"analysis":[],"attachments":[]}}"#
        ),
        "only uuid, created_at and the four empty arrays"
    );
}

#[test]
fn the_uuid_is_written_in_lower_case_and_read_back_unchanged() {
    let upper: Uuid = ID.to_uppercase().parse().expect("upper-case uuid");
    let v = Vcon::new(upper, date(CREATED));
    let bytes = v.to_json().expect("written");
    assert_eq!(
        json_of(&v)["uuid"],
        json!(ID),
        "the uuid is written in lower case"
    );
    assert_eq!(
        found(&bytes),
        Ok((Form::Unsigned, false, id())),
        "inspect reads the same uuid back"
    );
}

#[test]
fn a_date_is_kept_exactly_as_given() {
    for text in [
        "2022-06-21T17:53:26.000+00:00",
        "2022-06-21T13:53:00-04:00",
        "2022-06-21T17:53:27.200000+00:00",
        "2026-10-27T00:00:00Z",
    ] {
        assert_eq!(
            text.parse::<Date>().expect(text).as_str(),
            text,
            "the date text is kept as given"
        );
    }
}

#[test]
fn a_date_without_an_offset_or_not_rfc3339_is_refused() {
    for text in [
        "2022-06-21T13:53:00",
        "2022-06-21",
        "13:53:00Z",
        "2022-13-01T00:00:00Z",
        "2022-06-21T13:53:00+0400",
        "yesterday",
        "",
    ] {
        assert_eq!(text.parse::<Date>(), Err(WriteError::BadDate), "{text:?}");
    }
}

#[test]
fn a_date_from_a_time_is_rfc3339_with_its_offset() {
    let utc = OffsetDateTime::from_unix_timestamp(1_655_833_980).expect("time");
    let eastern = utc.to_offset(UtcOffset::from_hms(-4, 0, 0).expect("offset"));
    assert_eq!(
        Date::try_from(eastern).expect("written").as_str(),
        "2022-06-21T13:53:00-04:00",
        "an OffsetDateTime is written with its offset"
    );
    let with_fraction =
        OffsetDateTime::from_unix_timestamp_nanos(1_655_833_980_250_000_000).expect("time");
    let text = Date::try_from(with_fraction).expect("written");
    let back = OffsetDateTime::parse(text.as_str(), &Rfc3339).expect("RFC 3339");
    assert_eq!(back, with_fraction, "the written date is the same instant");
    assert_eq!(
        text.as_str().parse::<Date>().as_ref(),
        Ok(&text),
        "the written date parses back to itself"
    );
}

#[test]
fn a_time_whose_offset_rfc3339_cannot_write_is_refused() {
    let odd = OffsetDateTime::from_unix_timestamp(0)
        .expect("time")
        .to_offset(UtcOffset::from_hms(1, 0, 30).expect("offset"));
    assert_eq!(
        Date::try_from(odd),
        Err(WriteError::BadDate),
        "an offset with seconds cannot be written in RFC 3339"
    );
}

// --- WRT-2: uuid_v8 ---

/// The expected values were computed with Python's hashlib, not with this
/// crate: rand_b = int.from_bytes(sha1(fqhn).digest()[:8], "big") >> 2.
#[test]
fn uuid_v8_matches_a_vector_computed_independently() {
    let u = uuid_v8(0x01a0_7da8_c2bb, 0x3e5, "vcon.example.com");
    assert_eq!(
        u.to_string(),
        "01a07da8-c2bb-83e5-b7dd-b0a177be8d34",
        "the vector computed with Python's hashlib"
    );
    assert_eq!(u.get_version_num(), 8, "version 8");
    assert_eq!(
        u.get_variant(),
        uuid::Variant::RFC4122,
        "the RFC 9562 variant"
    );
}

#[test]
fn two_hosts_differ_only_in_rand_b() {
    let a = uuid_v8(0x01a0_7da8_c2bb, 0x3e5, "vcon.example.com");
    let b = uuid_v8(0x01a0_7da8_c2bb, 0x3e5, "calls.example.org");
    assert_eq!(
        b.to_string(),
        "01a07da8-c2bb-83e5-aee2-8091e87f6e9a",
        "the second host's vector, computed with Python's hashlib"
    );
    assert_eq!(
        a.as_bytes()[..8],
        b.as_bytes()[..8],
        "time, version and rand_a are the same for both hosts"
    );
    assert_ne!(
        a.as_bytes()[8..],
        b.as_bytes()[8..],
        "rand_b differs between the hosts"
    );
}

#[test]
fn uuid_v8_uses_only_the_low_48_bits_of_time_and_12_of_rand_a() {
    assert_eq!(
        uuid_v8(0, 0, "vcon.example.com").to_string(),
        "00000000-0000-8000-b7dd-b0a177be8d34",
        "the lowest time and rand_a"
    );
    assert_eq!(
        uuid_v8(u64::MAX, u16::MAX, "vcon.example.com").to_string(),
        "ffffffff-ffff-8fff-b7dd-b0a177be8d34",
        "only the low 48 bits of time and 12 of rand_a are used"
    );
}

// --- WRT-3, WRT-4: compact output in the draft's order ---

/// Every member the first golden model sets, in the order core-04 defines
/// them, then the extension members in the order they were set.
const GOLDEN_A: &str = concat!(
    r#"{"uuid":"01a07da8-c2bb-83e5-b9a2-279e0d16bc46","extensions":["example_ext"],"#,
    r#""created_at":"2022-06-21T13:53:00-04:00","updated_at":"2022-06-21T14:00:00-04:00","#,
    r#""subject":"Golden","amended":{"uuid":"01a07da8-c5c1-8db4-a20f-d188127fad29"},"#,
    r#""parties":[{"tel":"+15555550100","name":"Alice","validation":"none","type":"person"},"#,
    r#"{"mailto":"bob@example.com","civicaddress":{"country":"US","a1":"CA"},"org":"Example","dept":"Sales"}],"#,
    r#""dialog":[{"type":"recording","start":"2022-06-21T17:53:26.000+00:00","duration":33.12,"#,
    r#""parties":[0,1],"mediatype":"audio/x-mp3","filename":"a.mp3","url":"https://example.com/a.mp3","#,
    r#""content_hash":"sha512-AQID"},"#,
    r#"{"type":"text","start":"2022-06-21T17:54:00Z","parties":1,"mediatype":"text/plain","#,
    r#""body":"Hi","encoding":"none","message_id":"<m1@example.com>"}],"#,
    r#""analysis":[{"type":"summary","dialog":[0,1],"attachment":0,"vendor":"Example","#,
    r#""product":"p","schema":"s","body":{"text":"ok"},"encoding":"json"}],"#,
    r#""attachments":[{"purpose":"notes","start":"2022-06-21T17:55:00Z","party":0,"dialog":0,"#,
    r#""mediatype":"application/octet-stream","body":"AQID","encoding":"base64url"}],"#,
    r#""example_ext_b":1,"example_ext_a":{"k":[true,null]}}"#
);

fn golden_a() -> Vcon {
    let mut v = base();
    v.use_extension("example_ext");
    v.set_updated_at(date("2022-06-21T14:00:00-04:00"));
    v.set_subject("Golden");
    v.set_amended(Reference::by_uuid(prior()));
    let alice = v.add_party(
        Party::new()
            .tel("+15555550100")
            .name("Alice")
            .validation("none")
            .kind("person"),
    );
    let bob = v.add_party(
        Party::new()
            .mailto("bob@example.com")
            .civicaddress(CivicAddress::new().country("US").a1("CA"))
            .org("Example")
            .dept("Sales"),
    );
    let call = v.add_dialog(
        Dialog::recording(external("https://example.com/a.mp3"))
            .start(date("2022-06-21T17:53:26.000+00:00"))
            .duration(Duration::try_from_secs_f64(33.12).expect("duration"))
            .parties(Parties::Channels(vec![
                Channel::One(alice),
                Channel::One(bob),
            ]))
            .mediatype("audio/x-mp3")
            .filename("a.mp3"),
    );
    let text = v.add_dialog(
        Dialog::text(Content::Inline(Body::Text("Hi".into())))
            .start(date("2022-06-21T17:54:00Z"))
            .parties(Parties::One(bob))
            .mediatype("text/plain")
            .message_id("<m1@example.com>"),
    );
    let notes = v.add_attachment(
        Attachment::new(
            date("2022-06-21T17:55:00Z"),
            alice,
            call,
            Content::Inline(Body::Bytes(vec![1, 2, 3])),
        )
        .purpose("notes")
        .mediatype("application/octet-stream"),
    );
    v.add_analysis(
        Analysis::new(
            "summary",
            "Example",
            Content::Inline(Body::Json(json!({"text": "ok"}))),
        )
        .dialogs(vec![call, text])
        .attachment(notes)
        .product("p")
        .schema("s"),
    );
    v.set_member("example_ext_b", json!(1)).expect("member");
    v.set_member("example_ext_a", json!({"k": [true, null]}))
        .expect("member");
    v
}

/// The members the first golden model leaves out: critical, redacted, the
/// other dialog types, nested parties, session ids, party history, a
/// placeholder, several hashes and an extension member on a dialog.
const GOLDEN_B: &str = concat!(
    r#"{"uuid":"01a07da8-c2bb-83e5-b9a2-279e0d16bc46","extensions":["ext_c"],"critical":["ext_c"],"#,
    r#""created_at":"2022-06-21T13:53:00-04:00","#,
    r#""redacted":{"uuid":"01a07da8-c5c1-8db4-a20f-d188127fad29","type":"PII Redaction","#,
    r#""url":"https://example.com/prior.vcon","content_hash":["sha512-AQID","sha256-BAUG"]},"#,
    r#""parties":[{},{"name":"anonymous"},"#,
    r#"{"sip":"sip:carol@example.com","stir":"a.b.c","did":"did:example:123","gmlpos":"37.0 -122.0","uuid":"agent-7"}],"#,
    r#""dialog":[{"type":"recording","parties":[[0,1],null],"originator":1,"recording_set":1,"#,
    r#""content_hash":"sha512-AQID","session_id":[[{"local":"L","remote":"R"},{}],{}],"#,
    r#""party_history":[{"party":0,"time":"2022-06-21T17:53:27Z","event":"join"},"#,
    r#"{"party":1,"time":"2022-06-21T17:53:27Z","event":"keydown","button":"5"}],"#,
    r#""application":"Example Meet","ext_c_flag":true},"#,
    r#"{"type":"recording-set","start":"2022-06-21T17:53:00Z","duration":60,"parties":[0,1,2],"#,
    r#""recordings":[0],"session_id":{"local":"L","remote":"R"}},"#,
    r#"{"type":"incomplete","duration":2.0,"parties":[2,0],"disposition":"no-answer"},"#,
    r#"{"type":"transfer","transferee":0,"transferor":1,"transfer_target":2,"original":0,"#,
    r#""consultation":2,"target_dialog":1},"#,
    r#"{"type":"recording"}],"#,
    r#""analysis":[{"type":"transcript","dialog":0,"mediatype":"application/json","filename":"t.json","#,
    r#""vendor":"Example","url":"https://example.com/t.json","content_hash":["sha512-AQID","sha256-BAUG"]}],"#,
    r#""attachments":[{"start":"2022-06-21T17:53:00Z","party":2,"dialog":0,"filename":"x.pdf"}]}"#
);

fn golden_b() -> Vcon {
    let two = || vec![sha512(&[1, 2, 3]), sha256(&[4, 5, 6])];
    let t = || date("2022-06-21T17:53:27Z");
    let s = || date("2022-06-21T17:53:00Z");
    let mut v = base();
    v.use_critical_extension("ext_c");
    v.set_redacted(
        "PII Redaction",
        Reference::by_uuid(prior()).with_url("https://example.com/prior.vcon", two()),
    );
    let p0 = v.add_party(Party::new());
    let p1 = v.add_party(Party::new().name("anonymous"));
    let p2 = v.add_party(
        Party::new()
            .sip("sip:carol@example.com")
            .stir("a.b.c")
            .did("did:example:123")
            .gmlpos("37.0 -122.0")
            .uuid("agent-7"),
    );
    let d0 = v.add_dialog(
        Dialog::recording(Content::HashOnly(vec![sha512(&[1, 2, 3])]))
            .parties(Parties::Channels(vec![
                Channel::Mix(vec![p0, p1]),
                Channel::Unrecorded,
            ]))
            .originator(p1)
            .in_recording_set(DialogIndex::new(1))
            .session_id(SessionIds::Channels(vec![
                SessionChannel::Mix(vec![SessionId::new("L", "R"), SessionId::none()]),
                SessionChannel::One(SessionId::none()),
            ]))
            .party_history(vec![
                PartyHistory::new(p0, t(), Event::Join),
                PartyHistory::new(p1, t(), Event::Keydown("5".into())),
            ])
            .application("Example Meet")
            .set_member("ext_c_flag", json!(true))
            .expect("member"),
    );
    let d1 = v.add_dialog(
        Dialog::recording_set(vec![d0])
            .start(s())
            .duration(Duration::try_from_secs(60).expect("duration"))
            .parties(Parties::Channels(vec![
                Channel::One(p0),
                Channel::One(p1),
                Channel::One(p2),
            ]))
            .session_id(SessionIds::One(SessionId::new("L", "R"))),
    );
    let d2 = v.add_dialog(
        Dialog::incomplete(Disposition::NoAnswer)
            .parties(Parties::Channels(vec![Channel::One(p2), Channel::One(p0)]))
            .duration(Duration::try_from_secs_f64(2.0).expect("duration")),
    );
    v.add_dialog(
        Dialog::transfer()
            .transferee(p0)
            .transferor(p1)
            .transfer_target(p2)
            .original(d0)
            .consultation(d2)
            .target_dialog(d1),
    );
    v.add_dialog(Dialog::try_placeholder(DialogType::Recording).expect("placeholder"));
    v.add_analysis(
        Analysis::new(
            "transcript",
            "Example",
            Content::External {
                url: "https://example.com/t.json".into(),
                hashes: two(),
            },
        )
        .dialog(d0)
        .mediatype("application/json")
        .filename("t.json"),
    );
    v.add_attachment(Attachment::new(s(), p2, d0, Content::Absent).filename("x.pdf"));
    v
}

#[test]
fn a_model_is_written_byte_for_byte_in_the_drafts_order() {
    for (name, v, golden) in [("A", golden_a(), GOLDEN_A), ("B", golden_b(), GOLDEN_B)] {
        let bytes = v.to_json().expect(name);
        assert_eq!(
            String::from_utf8(bytes).expect("UTF-8"),
            golden,
            "golden {name}"
        );
    }
}

#[test]
fn every_golden_output_reads_back_as_an_unsigned_vcon() {
    let supported: BTreeSet<String> = ["ext_c".to_owned()].into();
    for v in [golden_a(), golden_b()] {
        let bytes = v.to_json().expect("written");
        let i = inspect(&bytes, None, &LIMITS, &supported).expect("inspect accepts it");
        assert_eq!(
            (i.form, i.gzip, i.uuid),
            (Form::Unsigned, false, id()),
            "inspect reads the golden output as an unsigned vCon"
        );
    }
}

#[test]
fn no_output_holds_the_deprecated_vcon_member() {
    for v in [base(), golden_a(), golden_b()] {
        assert!(
            json_of(&v).get("vcon").is_none(),
            "the deprecated vcon member is never written"
        );
    }
}

/// `depth` arrays, one inside the other.
fn nested(depth: usize) -> Value {
    (0..depth).fold(json!(0), |inner, _| Value::Array(vec![inner]))
}

#[test]
fn an_extension_member_may_bring_the_output_to_the_depth_limit_and_no_further() {
    // The vCon object itself is one level, so a top-level member's value
    // may have MAX_JSON_DEPTH - 1.
    let mut fits = base();
    fits.set_member("deep", nested(MAX_JSON_DEPTH - 1))
        .expect("member");
    let bytes = fits.to_json().expect("at the limit");
    assert_eq!(
        found(&bytes),
        Ok((Form::Unsigned, false, id())),
        "inspect accepts output at the depth limit"
    );

    let mut over = base();
    over.set_member("deep", nested(MAX_JSON_DEPTH))
        .expect("member");
    assert_eq!(
        over.to_json(),
        Err(WriteError::TooDeep),
        "one level more is refused"
    );
}

#[test]
fn a_json_body_counts_toward_the_depth_limit() {
    // vCon, the dialog array and the dialog object are three levels.
    let at = |depth| {
        let mut v = base();
        v.add_dialog(
            Dialog::text(Content::Inline(Body::Json(nested(depth)))).mediatype("application/json"),
        );
        v
    };
    let bytes = at(MAX_JSON_DEPTH - 3).to_json().expect("at the limit");
    assert_eq!(
        found(&bytes),
        Ok((Form::Unsigned, false, id())),
        "inspect accepts a body at the depth limit"
    );
    assert_eq!(
        at(MAX_JSON_DEPTH - 2).to_json(),
        Err(WriteError::TooDeep),
        "one level more in a body is refused"
    );
}

#[test]
fn the_same_model_is_written_as_the_same_bytes() {
    for v in [golden_a(), golden_b()] {
        assert_eq!(
            v.to_json(),
            v.clone().to_json(),
            "the same model gives the same JSON"
        );
        assert_eq!(
            v.to_gzip(),
            v.clone().to_gzip(),
            "the same model gives the same gzip"
        );
    }
}

// --- gzip ---

#[test]
fn the_gzip_form_inflates_to_the_unsigned_form_and_reads_back_as_gzip() {
    let v = golden_a();
    let gz = v.to_gzip().expect("gzip");
    assert_eq!(
        gunzip(&gz),
        v.to_json().expect("json"),
        "the gzip form inflates to the unsigned form"
    );
    assert_eq!(
        found(&gz),
        Ok((Form::Unsigned, true, id())),
        "inspect reads the gzip form as gzip with the same uuid"
    );
}

#[test]
fn the_gzip_header_records_no_time() {
    let gz = base().to_gzip().expect("gzip");
    assert_eq!(&gz[..4], &[0x1f, 0x8b, 8, 0], "magic, deflate, no flags");
    assert_eq!(&gz[4..8], &[0, 0, 0, 0], "MTIME is zero");
}

// --- WRT-5: indices ---

type Build = fn(&mut Vcon);

/// One case per member that holds an index: each sets index 9, which
/// refers to nothing, and names the path the refusal must give.
const OUT_OF_RANGE: &[(&str, Build)] = &[
    ("/dialog/1/parties", |v| {
        v.add_dialog(
            Dialog::recording(external("https://example.com/b.mp3"))
                .parties(Parties::One(PartyIndex::new(9))),
        );
    }),
    ("/dialog/1/parties/1", |v| {
        v.add_dialog(
            Dialog::recording(external("https://example.com/b.mp3")).parties(Parties::Channels(
                vec![
                    Channel::One(PartyIndex::new(0)),
                    Channel::One(PartyIndex::new(9)),
                ],
            )),
        );
    }),
    ("/dialog/1/parties/0/1", |v| {
        v.add_dialog(
            Dialog::recording(external("https://example.com/b.mp3")).parties(Parties::Channels(
                vec![Channel::Mix(vec![PartyIndex::new(1), PartyIndex::new(9)])],
            )),
        );
    }),
    ("/dialog/1/originator", |v| {
        v.add_dialog(
            Dialog::recording(external("https://example.com/b.mp3")).originator(PartyIndex::new(9)),
        );
    }),
    ("/dialog/1/party_history/1/party", |v| {
        v.add_dialog(
            Dialog::recording(external("https://example.com/b.mp3")).party_history(vec![
                PartyHistory::new(
                    PartyIndex::new(0),
                    date("2022-06-21T17:53:27Z"),
                    Event::Join,
                ),
                PartyHistory::new(
                    PartyIndex::new(9),
                    date("2022-06-21T17:53:28Z"),
                    Event::Drop,
                ),
            ]),
        );
    }),
    ("/dialog/1/recordings/1", |v| {
        v.add_dialog(Dialog::recording_set(vec![
            DialogIndex::new(0),
            DialogIndex::new(9),
        ]));
    }),
    ("/dialog/1/recording_set", |v| {
        v.add_dialog(
            Dialog::recording(external("https://example.com/b.mp3"))
                .in_recording_set(DialogIndex::new(9)),
        );
    }),
    ("/dialog/1/transferee", |v| {
        v.add_dialog(Dialog::transfer().transferee(PartyIndex::new(9)));
    }),
    ("/dialog/1/transferor", |v| {
        v.add_dialog(Dialog::transfer().transferor(PartyIndex::new(9)));
    }),
    ("/dialog/1/transfer_target", |v| {
        v.add_dialog(Dialog::transfer().transfer_target(PartyIndex::new(9)));
    }),
    ("/dialog/1/original", |v| {
        v.add_dialog(Dialog::transfer().original(DialogIndex::new(9)));
    }),
    ("/dialog/1/consultation", |v| {
        v.add_dialog(Dialog::transfer().consultation(DialogIndex::new(9)));
    }),
    ("/dialog/1/target_dialog", |v| {
        v.add_dialog(Dialog::transfer().target_dialog(DialogIndex::new(9)));
    }),
    ("/attachments/0/party", |v| {
        v.add_attachment(Attachment::new(
            date(CREATED),
            PartyIndex::new(9),
            DialogIndex::new(0),
            external("https://example.com/x.pdf"),
        ));
    }),
    ("/attachments/0/dialog", |v| {
        v.add_attachment(Attachment::new(
            date(CREATED),
            PartyIndex::new(0),
            DialogIndex::new(9),
            external("https://example.com/x.pdf"),
        ));
    }),
    ("/analysis/0/dialog", |v| {
        v.add_analysis(
            Analysis::new("summary", "Example", external("https://example.com/s.json"))
                .dialog(DialogIndex::new(9)),
        );
    }),
    ("/analysis/0/dialog/1", |v| {
        v.add_analysis(
            Analysis::new("summary", "Example", external("https://example.com/s.json"))
                .dialogs(vec![DialogIndex::new(0), DialogIndex::new(9)]),
        );
    }),
    ("/analysis/0/attachment", |v| {
        v.add_analysis(
            Analysis::new("summary", "Example", external("https://example.com/s.json"))
                .attachment(AttachmentIndex::new(9)),
        );
    }),
    ("/analysis/0/attachment/0", |v| {
        v.add_analysis(
            Analysis::new("summary", "Example", external("https://example.com/s.json"))
                .attachments(vec![AttachmentIndex::new(9)]),
        );
    }),
];

#[test]
fn an_index_that_refers_to_nothing_is_refused_with_its_path() {
    for (path, build) in OUT_OF_RANGE {
        let mut v = with_two_parties_and_a_recording();
        build(&mut v);
        assert_eq!(
            v.to_json(),
            Err(WriteError::UnknownIndex {
                path: (*path).to_owned()
            }),
            "{path}"
        );
    }
}

/// The first index past the end is refused too, not only one far past it.
#[test]
fn an_index_equal_to_the_number_of_entries_is_refused() {
    let unknown = |path: &str| {
        Err(WriteError::UnknownIndex {
            path: path.to_owned(),
        })
    };
    // Two parties: index 2 is one past the last.
    let mut party = with_two_parties_and_a_recording();
    party.add_dialog(
        Dialog::recording(external("https://example.com/b.mp3"))
            .parties(Parties::One(PartyIndex::new(2))),
    );
    assert_eq!(
        party.to_json(),
        unknown("/dialog/1/parties"),
        "party 2 of two does not exist"
    );
    // The recording and this recording-set: index 2 is one past the last.
    let mut dialog = with_two_parties_and_a_recording();
    dialog.add_dialog(Dialog::recording_set(vec![DialogIndex::new(2)]));
    assert_eq!(
        dialog.to_json(),
        unknown("/dialog/1/recordings/0"),
        "dialog 2 of two does not exist"
    );
    // No attachments: index 0 is one past the last.
    let mut attachment = with_two_parties_and_a_recording();
    attachment.add_analysis(
        Analysis::new("summary", "Example", external("https://example.com/s.json"))
            .attachment(AttachmentIndex::new(0)),
    );
    assert_eq!(
        attachment.to_json(),
        unknown("/analysis/0/attachment"),
        "attachment 0 of none does not exist"
    );
}

#[test]
fn a_null_channel_stands_for_a_channel_with_no_party_recorded() {
    let mut v = with_two_parties_and_a_recording();
    v.add_dialog(
        Dialog::recording(external("https://example.com/b.mp3")).parties(Parties::Channels(vec![
            Channel::One(PartyIndex::new(0)),
            Channel::Unrecorded,
            Channel::Mix(vec![PartyIndex::new(0), PartyIndex::new(1)]),
        ])),
    );
    assert_eq!(
        json_of(&v)["dialog"][1]["parties"],
        json!([0, null, [0, 1]]),
        "a null stands for the unrecorded channel"
    );
}

#[test]
fn every_index_member_in_range_is_written_as_its_number() {
    let mut v = with_two_parties_and_a_recording();
    let (p0, p1) = (PartyIndex::new(0), PartyIndex::new(1));
    let d0 = DialogIndex::new(0);
    v.add_dialog(Dialog::recording_set(vec![d0]));
    v.add_dialog(
        Dialog::recording(external("https://example.com/b.mp3"))
            .parties(Parties::One(p1))
            .originator(p0)
            .in_recording_set(DialogIndex::new(1))
            .party_history(vec![PartyHistory::new(
                p1,
                date("2022-06-21T17:53:27Z"),
                Event::Hold,
            )]),
    );
    v.add_dialog(
        Dialog::transfer()
            .transferee(p0)
            .transferor(p1)
            .transfer_target(p0)
            .original(d0)
            .consultation(DialogIndex::new(1))
            .target_dialog(DialogIndex::new(2)),
    );
    let a0 = v.add_attachment(Attachment::new(
        date(CREATED),
        p1,
        d0,
        external("https://example.com/x.pdf"),
    ));
    v.add_analysis(
        Analysis::new("summary", "Example", external("https://example.com/s.json"))
            .dialogs(vec![d0, DialogIndex::new(2)])
            .attachments(vec![a0]),
    );
    let out = json_of(&v);
    assert_eq!(
        out["dialog"][1]["recordings"],
        json!([0]),
        "recordings written as numbers"
    );
    let rec = &out["dialog"][2];
    assert_eq!(
        (&rec["parties"], &rec["originator"], &rec["recording_set"]),
        (&json!(1), &json!(0), &json!(1)),
        "parties, originator and recording_set written as numbers"
    );
    assert_eq!(
        rec["party_history"][0]["party"],
        json!(1),
        "party_history party written as a number"
    );
    let t = &out["dialog"][3];
    for (name, n) in [
        ("transferee", 0),
        ("transferor", 1),
        ("transfer_target", 0),
        ("original", 0),
        ("consultation", 1),
        ("target_dialog", 2),
    ] {
        assert_eq!(t[name], json!(n), "{name}");
    }
    assert_eq!(
        (
            &out["attachments"][0]["party"],
            &out["attachments"][0]["dialog"]
        ),
        (&json!(1), &json!(0)),
        "attachment party and dialog written as numbers"
    );
    assert_eq!(
        out["analysis"][0]["dialog"],
        json!([0, 2]),
        "analysis dialog written as an array"
    );
    assert_eq!(
        out["analysis"][0]["attachment"],
        json!([0]),
        "analysis attachment written as an array"
    );
}

// --- WRT-6: Table 1 ---

const ALL_TYPES: [DialogType; 5] = [
    DialogType::Recording,
    DialogType::RecordingSet,
    DialogType::Text,
    DialogType::Transfer,
    DialogType::Incomplete,
];

/// A dialog of `kind` that holds what Table 1 marks M for it and nothing
/// else, referring to party 0 and dialog 0.
fn minimal(kind: DialogType) -> Dialog {
    match kind {
        DialogType::Recording => Dialog::recording(external("https://example.com/b.mp3")),
        DialogType::RecordingSet => Dialog::recording_set(vec![DialogIndex::new(0)]),
        DialogType::Text => {
            Dialog::text(Content::Inline(Body::Text("Hi".into()))).mediatype("text/plain")
        }
        DialogType::Transfer => Dialog::transfer(),
        DialogType::Incomplete => Dialog::incomplete(Disposition::Failed),
        _ => unreachable!("every type is listed"),
    }
}

type Set = fn(Dialog) -> Dialog;

/// Each member Table 1 marks X for some type, the setter that adds it, and
/// the types it is marked X for.
const FORBIDDEN: &[(&str, Set, &[DialogType])] = &[
    (
        "parties",
        |d| d.parties(Parties::One(PartyIndex::new(0))),
        &[DialogType::Transfer],
    ),
    (
        "originator",
        |d| d.originator(PartyIndex::new(0)),
        &[DialogType::Transfer],
    ),
    (
        "recordings",
        |d| d.recordings(vec![DialogIndex::new(0)]),
        &[
            DialogType::Recording,
            DialogType::Text,
            DialogType::Transfer,
            DialogType::Incomplete,
        ],
    ),
    (
        "recording_set",
        |d| d.in_recording_set(DialogIndex::new(0)),
        &[
            DialogType::RecordingSet,
            DialogType::Text,
            DialogType::Transfer,
            DialogType::Incomplete,
        ],
    ),
    (
        "mediatype",
        |d| d.mediatype("text/plain"),
        &[
            DialogType::RecordingSet,
            DialogType::Transfer,
            DialogType::Incomplete,
        ],
    ),
    (
        "filename",
        |d| d.filename("a.txt"),
        &[
            DialogType::RecordingSet,
            DialogType::Transfer,
            DialogType::Incomplete,
        ],
    ),
    (
        "body",
        |d| d.content(Content::Inline(Body::Text("Hi".into()))),
        &[
            DialogType::RecordingSet,
            DialogType::Transfer,
            DialogType::Incomplete,
        ],
    ),
    (
        "url",
        |d| d.content(external("https://example.com/b.mp3")),
        &[
            DialogType::RecordingSet,
            DialogType::Transfer,
            DialogType::Incomplete,
        ],
    ),
    (
        "content_hash",
        |d| d.content(Content::HashOnly(vec![sha512(&[1])])),
        &[
            DialogType::RecordingSet,
            DialogType::Transfer,
            DialogType::Incomplete,
        ],
    ),
    (
        "session_id",
        |d| d.session_id(SessionIds::One(SessionId::none())),
        &[DialogType::Transfer],
    ),
    (
        "party_history",
        |d| d.party_history(vec![]),
        &[DialogType::Transfer],
    ),
    (
        "transferee",
        |d| d.transferee(PartyIndex::new(0)),
        &[
            DialogType::Recording,
            DialogType::RecordingSet,
            DialogType::Text,
            DialogType::Incomplete,
        ],
    ),
    (
        "transferor",
        |d| d.transferor(PartyIndex::new(0)),
        &[
            DialogType::Recording,
            DialogType::RecordingSet,
            DialogType::Text,
            DialogType::Incomplete,
        ],
    ),
    (
        "transfer_target",
        |d| d.transfer_target(PartyIndex::new(0)),
        &[
            DialogType::Recording,
            DialogType::RecordingSet,
            DialogType::Text,
            DialogType::Incomplete,
        ],
    ),
    (
        "original",
        |d| d.original(DialogIndex::new(0)),
        &[
            DialogType::Recording,
            DialogType::RecordingSet,
            DialogType::Text,
            DialogType::Incomplete,
        ],
    ),
    (
        "consultation",
        |d| d.consultation(DialogIndex::new(0)),
        &[
            DialogType::Recording,
            DialogType::RecordingSet,
            DialogType::Text,
            DialogType::Incomplete,
        ],
    ),
    (
        "target_dialog",
        |d| d.target_dialog(DialogIndex::new(0)),
        &[
            DialogType::Recording,
            DialogType::RecordingSet,
            DialogType::Text,
            DialogType::Incomplete,
        ],
    ),
    (
        "message_id",
        |d| d.message_id("<m@example.com>"),
        &[
            DialogType::RecordingSet,
            DialogType::Transfer,
            DialogType::Incomplete,
        ],
    ),
];

/// In a redacted vCon, so that content the setters replace (a text body by
/// a hash only) is allowed wherever Table 1 allows content at all.
fn redacted_with_a_recording() -> Vcon {
    let mut v = with_two_parties_and_a_recording();
    v.set_redacted_without_prior("PII Redaction");
    v
}

#[test]
fn every_member_table_1_marks_x_is_refused_for_that_type() {
    for (name, set, types) in FORBIDDEN {
        for kind in types.iter() {
            let mut v = redacted_with_a_recording();
            v.add_dialog(set(minimal(*kind)));
            assert_eq!(
                v.to_json(),
                Err(WriteError::ForbiddenParameter {
                    path: "/dialog/1".into(),
                    name
                }),
                "{name} on {kind:?}"
            );
        }
    }
}

#[test]
fn every_member_table_1_does_not_mark_x_is_written_for_that_type() {
    for (name, set, types) in FORBIDDEN {
        for kind in ALL_TYPES.iter().filter(|k| !types.contains(k)) {
            let mut v = redacted_with_a_recording();
            // An inline body on a recording needs its mediatype (WRT-6).
            let dialog = match kind {
                DialogType::Recording => minimal(*kind).mediatype("audio/x-wav"),
                _ => minimal(*kind),
            };
            v.add_dialog(set(dialog));
            let out = written(&v).unwrap_or_else(|e| panic!("{name} on {kind:?}: {e:?}"));
            assert!(out["dialog"][1].get(*name).is_some(), "{name} on {kind:?}");
        }
    }
}

#[test]
fn every_type_is_written_with_only_what_table_1_requires() {
    for kind in ALL_TYPES {
        let mut v = with_two_parties_and_a_recording();
        v.add_dialog(minimal(kind));
        let out = written(&v).unwrap_or_else(|e| panic!("{kind:?}: {e:?}"));
        assert_eq!(
            out["dialog"][1]["type"],
            json!(kind.as_str()),
            "the dialog type written as the draft spells it"
        );
    }
}

#[test]
fn a_recording_set_without_recordings_is_refused() {
    let mut v = with_two_parties_and_a_recording();
    v.add_dialog(Dialog::new(DialogType::RecordingSet));
    assert_eq!(
        v.to_json(),
        Err(WriteError::MissingParameter {
            path: "/dialog/1".into(),
            name: "recordings"
        }),
        "a recording-set needs recordings"
    );
}

#[test]
fn an_incomplete_dialog_without_a_disposition_is_refused() {
    let mut v = with_two_parties_and_a_recording();
    v.add_dialog(Dialog::new(DialogType::Incomplete));
    assert_eq!(
        v.to_json(),
        Err(WriteError::MissingParameter {
            path: "/dialog/1".into(),
            name: "disposition"
        }),
        "an incomplete dialog needs a disposition"
    );
}

#[test]
fn inline_recording_or_text_content_without_a_mediatype_is_refused() {
    for kind in [DialogType::Recording, DialogType::Text] {
        let mut v = with_two_parties_and_a_recording();
        v.add_dialog(Dialog::new(kind).content(Content::Inline(Body::Bytes(vec![1]))));
        assert_eq!(
            v.to_json(),
            Err(WriteError::MissingParameter {
                path: "/dialog/1".into(),
                name: "mediatype"
            }),
            "{kind:?}"
        );
    }
}

#[test]
fn external_content_needs_no_mediatype() {
    let mut v = with_two_parties_and_a_recording();
    v.add_dialog(Dialog::text(external("https://example.com/t.txt")));
    assert!(
        json_of(&v)["dialog"][1].get("mediatype").is_none(),
        "external content needs no mediatype"
    );
}

#[test]
fn a_recording_or_incomplete_placeholder_holds_only_its_type() {
    let mut v = base();
    v.add_dialog(Dialog::try_placeholder(DialogType::Recording).expect("recording"));
    v.add_dialog(Dialog::try_placeholder(DialogType::Incomplete).expect("incomplete"));
    assert_eq!(
        json_of(&v)["dialog"],
        json!([{"type": "recording"}, {"type": "incomplete", "disposition": "failed"}]),
        "a placeholder holds only its type, and failed for incomplete"
    );
}

#[test]
fn no_placeholder_is_made_of_the_other_types() {
    for kind in [
        DialogType::Text,
        DialogType::Transfer,
        DialogType::RecordingSet,
    ] {
        assert_eq!(
            Dialog::try_placeholder(kind),
            Err(WriteError::NoPlaceholder(kind)),
            "only recording and incomplete placeholders are made"
        );
    }
}

// --- WRT-7: content ---

#[test]
fn each_body_kind_is_written_with_its_encoding_and_round_trips() {
    let bytes = vec![0, 1, 2, 250, 251, 255];
    let mut v = base();
    for body in [
        Body::Bytes(bytes.clone()),
        Body::Text("Good day!".into()),
        Body::Json(json!({"a": [1, 2.5, "x"]})),
    ] {
        v.add_dialog(Dialog::text(Content::Inline(body)).mediatype("text/plain"));
    }
    let out = json_of(&v);
    let d = &out["dialog"];
    assert_eq!(
        d[0]["encoding"],
        json!("base64url"),
        "bytes are written with encoding base64url"
    );
    let text = d[0]["body"].as_str().expect("a string");
    assert!(!text.contains('='), "no padding");
    assert_eq!(
        URL_SAFE_NO_PAD.decode(text).expect("base64url"),
        bytes,
        "the body decodes back to the bytes"
    );
    assert_eq!(
        (&d[1]["body"], &d[1]["encoding"]),
        (&json!("Good day!"), &json!("none")),
        "text is written with encoding none"
    );
    assert_eq!(
        (&d[2]["body"], &d[2]["encoding"]),
        (&json!({"a": [1, 2.5, "x"]}), &json!("json")),
        "a JSON value is written with encoding json"
    );
}

#[test]
fn one_hash_is_written_as_a_string_and_several_as_an_array() {
    let mut v = base();
    v.add_dialog(Dialog::recording(external("https://example.com/a.mp3")));
    v.add_dialog(Dialog::recording(Content::External {
        url: "https://example.com/b.mp3".into(),
        hashes: vec![sha512(&[1, 2, 3]), sha256(&[4, 5, 6])],
    }));
    let out = json_of(&v);
    assert_eq!(
        out["dialog"][0]["content_hash"],
        json!("sha512-AQID"),
        "one hash is written as a string"
    );
    assert_eq!(
        out["dialog"][1]["content_hash"],
        json!(["sha512-AQID", "sha256-BAUG"]),
        "several hashes are written as an array"
    );
}

#[test]
fn external_content_needs_an_https_url_with_a_host_and_a_hash() {
    for (case, content) in [
        ("http", external("http://example.com/a.mp3")),
        ("no scheme", external("example.com/a.mp3")),
        ("no host", external("https://")),
        ("ftp", external("ftp://example.com/a.mp3")),
        (
            "no hash",
            Content::External {
                url: "https://example.com/a.mp3".into(),
                hashes: vec![],
            },
        ),
    ] {
        let mut v = base();
        v.add_dialog(Dialog::recording(content));
        assert_eq!(
            v.to_json(),
            Err(WriteError::BadContent {
                path: "/dialog/0".into()
            }),
            "{case}"
        );
    }
}

#[test]
fn an_https_scheme_in_capitals_is_accepted_and_written_as_given() {
    let mut v = base();
    v.add_dialog(Dialog::recording(external("HTTPS://example.com/a.mp3")));
    assert_eq!(
        json_of(&v)["dialog"][0]["url"],
        json!("HTTPS://example.com/a.mp3"),
        "the url is written as given"
    );
}

#[test]
fn content_hash_new_writes_the_digest_in_base64url() {
    assert_eq!(
        ContentHash::try_new("sha512", &[0xfb, 0xff])
            .expect("hash")
            .as_str(),
        "sha512--_8",
        "the digest is written in base64url without padding"
    );
}

#[test]
fn a_malformed_hash_algorithm_or_digest_is_refused() {
    for alg in ["SHA512", "sha-512", "", "sha 512", "shä512"] {
        assert_eq!(
            ContentHash::try_new(alg, &[1]),
            Err(WriteError::BadHash),
            "{alg:?}"
        );
    }
    assert_eq!(
        ContentHash::try_new("sha512", &[]),
        Err(WriteError::BadHash),
        "an empty digest is refused"
    );
    for token in [
        "SHA512-AQID",
        "sha512",
        "sha512-",
        "-AQID",
        "sha512-AQ+D",
        "sha512-AQ/D",
        "sha512-AQI=",
        "sha512-A",
        "sha512-AQ ID",
    ] {
        assert_eq!(
            token.parse::<ContentHash>(),
            Err(WriteError::BadHash),
            "{token:?}"
        );
    }
    assert_eq!(
        "sha512-AQID".parse::<ContentHash>().expect("hash").as_str(),
        "sha512-AQID",
        "a well-formed token parses"
    );
}

/// core-04 2.2: an algorithm name has no hyphen and a hyphen follows it,
/// so the first hyphen ends it, and a digest may hold hyphens of its own.
#[test]
fn a_hash_reads_back_as_the_algorithm_and_digest_it_was_made_with() {
    for (alg, digest) in [
        ("sha512", vec![1, 2, 3]),
        ("sha256", vec![0xfb, 0xff]),
        ("sha3x512", vec![0xf8; 9]),
    ] {
        let made = ContentHash::try_new(alg, &digest).expect("hash");
        let read = made.as_str().parse::<ContentHash>().expect("its own token");
        assert_eq!(
            read, made,
            "a token the writer makes reads back as the same hash"
        );
        assert_eq!(
            (read.algorithm(), read.digest()),
            (alg, digest.clone()),
            "{alg}"
        );
    }
    let hyphens = "sha-512-AQID"
        .parse::<ContentHash>()
        .expect("algorithm sha");
    assert_eq!(
        hyphens.algorithm(),
        "sha",
        "the algorithm ends at the first hyphen"
    );
    assert_eq!(
        hyphens.digest(),
        URL_SAFE_NO_PAD.decode("512-AQID").expect("base64url"),
        "the digest keeps its own hyphens"
    );
}

#[test]
fn absent_content_is_refused_outside_a_redaction_or_placeholder() {
    let mut text = base();
    text.add_dialog(Dialog::new(DialogType::Text));
    let mut recording = base();
    recording.add_dialog(Dialog::recording(Content::Absent).start(date(CREATED)));
    let mut attachment = with_two_parties_and_a_recording();
    attachment.add_attachment(Attachment::new(
        date(CREATED),
        PartyIndex::new(0),
        DialogIndex::new(0),
        Content::Absent,
    ));
    let mut analysis = base();
    analysis.add_analysis(Analysis::new("summary", "Example", Content::Absent));
    let mut hash_only = base();
    hash_only.add_dialog(Dialog::recording(Content::HashOnly(vec![sha512(&[1])])));
    for (case, mut v, path) in [
        ("text", text, "/dialog/0"),
        ("recording", recording, "/dialog/0"),
        ("attachment", attachment, "/attachments/0"),
        ("analysis", analysis, "/analysis/0"),
        ("hash only", hash_only, "/dialog/0"),
    ] {
        assert_eq!(
            v.to_json(),
            Err(WriteError::BadContent { path: path.into() }),
            "{case}"
        );
        v.set_redacted_without_prior("PII Redaction");
        written(&v).unwrap_or_else(|e| panic!("{case} in a redacted vCon: {e:?}"));
    }
}

#[test]
fn a_hash_only_list_must_hold_a_hash() {
    let mut v = base();
    v.set_redacted_without_prior("PII Redaction");
    v.add_dialog(Dialog::recording(Content::HashOnly(vec![])));
    assert_eq!(
        v.to_json(),
        Err(WriteError::BadContent {
            path: "/dialog/0".into()
        }),
        "a hash-only list must hold a hash"
    );
}

// --- WRT-8: links ---

#[test]
fn amended_and_redacted_are_never_both_written() {
    let mut v = base();
    v.set_amended(Reference::by_uuid(prior()));
    v.set_redacted_without_prior("PII Redaction");
    assert_eq!(
        v.to_json(),
        Err(WriteError::BothLinks),
        "amended and redacted together are refused"
    );
}

#[test]
fn a_reference_url_needs_https_and_a_hash() {
    for (case, reference) in [
        (
            "a url without a hash",
            Reference::by_url("https://example.com/prior.vcon", vec![]),
        ),
        (
            "an http url",
            Reference::by_url("http://example.com/prior.vcon", vec![sha512(&[1])]),
        ),
        (
            "a uuid and a url without a hash",
            Reference::by_uuid(prior()).with_url("https://example.com/prior.vcon", vec![]),
        ),
    ] {
        let mut amended = base();
        amended.set_amended(reference.clone());
        assert_eq!(
            amended.to_json(),
            Err(WriteError::BadReference),
            "amended: {case} is refused"
        );
        let mut redacted = base();
        redacted.set_redacted("PII Redaction", reference.clone());
        assert_eq!(
            redacted.to_json(),
            Err(WriteError::BadReference),
            "redacted: {case} is refused"
        );
    }
}

#[test]
fn each_reference_shape_is_written() {
    let mut by_url = base();
    by_url.set_amended(Reference::by_url(
        "https://example.com/prior.vcon",
        vec![sha512(&[1, 2, 3])],
    ));
    assert_eq!(
        json_of(&by_url)["amended"],
        json!({"url": "https://example.com/prior.vcon", "content_hash": "sha512-AQID"}),
        "a url reference is written with its hash"
    );
    let mut unavailable = base();
    unavailable.set_redacted_without_prior("PII Redaction");
    assert_eq!(
        json_of(&unavailable)["redacted"],
        json!({"type": "PII Redaction"}),
        "a redaction without a prior holds only its type"
    );
}

#[test]
fn a_link_to_the_vcons_own_uuid_is_refused() {
    let mut amended = base();
    amended.set_amended(Reference::by_uuid(id()));
    assert_eq!(
        amended.to_json(),
        Err(WriteError::SameUuid),
        "an amended link to the vCon's own uuid is refused"
    );
    let mut redacted = base();
    redacted.set_redacted("PII Redaction", Reference::by_uuid(id()));
    assert_eq!(
        redacted.to_json(),
        Err(WriteError::SameUuid),
        "a redacted link to the vCon's own uuid is refused"
    );
}

// --- WRT-9: amend, redact and removal ---

fn without(mut value: Value, names: &[&str]) -> Value {
    for name in names {
        value.as_object_mut().expect("an object").remove(*name);
    }
    value
}

#[test]
fn amend_copies_the_prior_vcon_under_a_new_uuid_and_links_it() {
    let original = golden_a();
    let before = json_of(&original);
    let mut new = Vcon::amend(&original, prior(), Reference::by_uuid(id())).expect("amended");
    let next = new.add_dialog(minimal(DialogType::Text));
    assert_eq!(next, DialogIndex::new(2), "after the prior's two dialogs");
    let out = json_of(&new);
    assert_eq!(
        out["uuid"],
        json!(PRIOR),
        "the amended copy has the new uuid"
    );
    assert_eq!(
        out["amended"],
        json!({"uuid": ID}),
        "the amended link names the prior vCon"
    );
    let mut expected = without(before.clone(), &["uuid", "amended"]);
    expected["dialog"]
        .as_array_mut()
        .expect("dialog")
        .push(json!({"type": "text", "mediatype": "text/plain", "body": "Hi", "encoding": "none"}));
    assert_eq!(
        without(out, &["uuid", "amended"]),
        expected,
        "the copy holds the prior content and the new dialog"
    );
    assert_eq!(json_of(&original), before, "the prior vCon is unchanged");
}

#[test]
fn redact_copies_the_prior_vcon_and_links_it_with_its_type() {
    let original = golden_a();
    let new = Vcon::redact(
        &original,
        prior(),
        "PII Redaction",
        Reference::by_uuid(id()),
    )
    .expect("redacted");
    let out = json_of(&new);
    assert_eq!(
        out["redacted"],
        json!({"uuid": ID, "type": "PII Redaction"}),
        "the redacted link names the prior vCon and the kind"
    );
    assert!(
        out.get("amended").is_none(),
        "the prior's amended link is not copied"
    );
    assert_eq!(
        without(out, &["uuid", "redacted"]),
        without(json_of(&original), &["uuid", "amended"]),
        "the redacted copy holds the prior content"
    );
}

#[test]
fn amend_drops_a_redacted_link_the_prior_had() {
    let mut prior_redaction = base();
    prior_redaction.set_redacted_without_prior("PII Redaction");
    let new = Vcon::amend(&prior_redaction, prior(), Reference::by_uuid(id())).expect("amended");
    let out = json_of(&new);
    assert!(
        out.get("redacted").is_none(),
        "the prior redacted link is not copied"
    );
    assert_eq!(
        out["amended"],
        json!({"uuid": ID}),
        "the amended link names the prior vCon"
    );
}

#[test]
fn a_new_version_under_the_prior_uuid_is_refused() {
    let original = golden_a();
    assert_eq!(
        Vcon::amend(&original, id(), Reference::by_uuid(id())).err(),
        Some(WriteError::SameUuid),
        "amend under the prior uuid is refused"
    );
    assert_eq!(
        Vcon::redact(
            &original,
            id(),
            "PII Redaction",
            Reference::by_uuid(prior())
        )
        .err(),
        Some(WriteError::SameUuid),
        "redact under the prior uuid is refused"
    );
}

#[test]
fn a_removed_party_attachment_or_analysis_leaves_an_empty_object() {
    let mut v = golden_a();
    v.remove_party(PartyIndex::new(0)).expect("party");
    v.remove_attachment(AttachmentIndex::new(0))
        .expect("attachment");
    v.remove_analysis(AnalysisIndex::new(0)).expect("analysis");
    let out = json_of(&v);
    let before = json_of(&golden_a());
    assert_eq!(
        out["parties"][0],
        json!({}),
        "a removed party leaves an empty object"
    );
    assert_eq!(
        out["parties"][1], before["parties"][1],
        "later indices unchanged"
    );
    assert_eq!(
        out["attachments"],
        json!([{}]),
        "a removed attachment leaves an empty object"
    );
    assert_eq!(
        out["analysis"],
        json!([{}]),
        "a removed analysis leaves an empty object"
    );
    assert_eq!(
        out["dialog"], before["dialog"],
        "indices to party 0 still written"
    );
}

#[test]
fn a_removed_dialog_leaves_a_placeholder_of_its_own_type() {
    let mut v = golden_b();
    for i in 0..5 {
        v.remove_dialog(DialogIndex::new(i)).expect("dialog");
    }
    assert_eq!(
        json_of(&v)["dialog"],
        json!([
            {"type": "recording"},
            {"type": "recording-set"},
            {"type": "incomplete", "disposition": "no-answer"},
            {"type": "transfer"},
            {"type": "recording"}
        ]),
        "each removed dialog leaves a placeholder of its own type"
    );
}

#[test]
fn a_removed_text_dialog_is_written_in_a_vcon_that_is_not_redacted() {
    let mut v = base();
    v.add_dialog(minimal(DialogType::Text));
    v.remove_dialog(DialogIndex::new(0)).expect("dialog");
    assert_eq!(
        json_of(&v)["dialog"],
        json!([{"type": "text"}]),
        "a removed text dialog leaves its type"
    );
}

#[test]
fn removing_or_replacing_an_entry_that_does_not_exist_is_refused() {
    let unknown = |path: &str| {
        Err(WriteError::UnknownIndex {
            path: path.to_owned(),
        })
    };
    let mut v = golden_a();
    assert_eq!(
        v.remove_party(PartyIndex::new(2)),
        unknown("/parties/2"),
        "remove_party out of range is refused"
    );
    assert_eq!(
        v.remove_dialog(DialogIndex::new(2)),
        unknown("/dialog/2"),
        "remove_dialog out of range is refused"
    );
    assert_eq!(
        v.remove_attachment(AttachmentIndex::new(1)),
        unknown("/attachments/1"),
        "remove_attachment out of range is refused"
    );
    assert_eq!(
        v.remove_analysis(AnalysisIndex::new(1)),
        unknown("/analysis/1"),
        "remove_analysis out of range is refused"
    );
    assert_eq!(
        v.replace_party(PartyIndex::new(2), Party::new()),
        unknown("/parties/2"),
        "replace_party out of range is refused"
    );
    assert_eq!(
        v.replace_dialog(DialogIndex::new(2), minimal(DialogType::Transfer)),
        unknown("/dialog/2"),
        "replace_dialog out of range is refused"
    );
    let attachment = Attachment::new(
        date(CREATED),
        PartyIndex::new(0),
        DialogIndex::new(0),
        external("https://example.com/x.pdf"),
    );
    assert_eq!(
        v.replace_attachment(AttachmentIndex::new(1), attachment),
        unknown("/attachments/1"),
        "replace_attachment out of range is refused"
    );
    let analysis = Analysis::new("summary", "Example", external("https://example.com/s.json"));
    assert_eq!(
        v.replace_analysis(AnalysisIndex::new(1), analysis),
        unknown("/analysis/1"),
        "replace_analysis out of range is refused"
    );
    assert_eq!(json_of(&v), json_of(&golden_a()), "nothing changed");
}

#[test]
fn a_replaced_entry_is_written_in_its_place() {
    let mut v = golden_a();
    v.replace_party(PartyIndex::new(1), Party::new().name("Robert"))
        .expect("party");
    v.replace_dialog(DialogIndex::new(1), minimal(DialogType::Transfer))
        .expect("dialog");
    v.replace_attachment(
        AttachmentIndex::new(0),
        Attachment::new(
            date(CREATED),
            PartyIndex::new(1),
            DialogIndex::new(1),
            external("https://example.com/x.pdf"),
        ),
    )
    .expect("attachment");
    v.replace_analysis(
        AnalysisIndex::new(0),
        Analysis::new("summary", "Other", external("https://example.com/s.json")),
    )
    .expect("analysis");
    let out = json_of(&v);
    assert_eq!(
        out["parties"][1],
        json!({"name": "Robert"}),
        "the replaced party is written in its place"
    );
    assert_eq!(
        out["dialog"][1],
        json!({"type": "transfer"}),
        "the replaced dialog is written in its place"
    );
    assert_eq!(
        out["attachments"][0]["party"],
        json!(1),
        "the replaced attachment is written in its place"
    );
    assert_eq!(
        out["analysis"][0]["vendor"],
        json!("Other"),
        "the replaced analysis is written in its place"
    );
}

// --- WRT-10: extensions and critical ---

#[test]
fn a_critical_extension_is_listed_once_in_both_lists() {
    let mut v = base();
    v.use_critical_extension("b_ext");
    v.use_extension("a_ext");
    v.use_critical_extension("b_ext");
    v.use_critical_extension("a_ext");
    let out = json_of(&v);
    assert_eq!(
        out["extensions"],
        json!(["b_ext", "a_ext"]),
        "each extension listed once, in order"
    );
    assert_eq!(
        out["critical"],
        json!(["b_ext", "a_ext"]),
        "each critical extension listed once, in order"
    );
}

#[test]
fn an_extension_that_is_not_critical_is_only_in_extensions() {
    let mut v = base();
    v.use_extension("a_ext");
    let out = json_of(&v);
    assert_eq!(
        out["extensions"],
        json!(["a_ext"]),
        "the extension is listed in extensions"
    );
    assert!(
        out.get("critical").is_none(),
        "an extension that is not critical is not in critical"
    );
}

#[test]
fn inspect_refuses_a_critical_extension_the_reader_does_not_support() {
    let mut v = base();
    v.use_critical_extension("b_ext");
    let bytes = v.to_json().expect("written");
    assert_eq!(
        found(&bytes),
        Err(Rejection::UnsupportedCritical(vec!["b_ext".into()])),
        "a reader without the extension refuses the vCon"
    );
    let supported: BTreeSet<String> = ["b_ext".to_owned()].into();
    assert!(
        inspect(&bytes, None, &LIMITS, &supported).is_ok(),
        "a reader with the extension accepts the vCon"
    );
}

// --- Party: civicaddress ---

/// core-04 4.2.9 lists the Civicaddress members in an order; they are
/// written in that order, not in the order they were set.
#[test]
fn a_civic_address_is_written_in_the_drafts_order_whatever_order_it_was_set_in() {
    let mut v = base();
    v.add_party(Party::new().civicaddress(CivicAddress::new().pc("94000").a1("CA").country("US")));
    let text = String::from_utf8(v.to_json().expect("written")).expect("UTF-8");
    assert!(
        text.contains(r#""civicaddress":{"country":"US","a1":"CA","pc":"94000"}"#),
        "{text}"
    );
}

// --- WRT-20: an inline attachment's mediatype ---

#[test]
fn an_inline_attachment_needs_a_mediatype_and_an_external_one_does_not() {
    let attachment = |content| {
        Attachment::new(
            date(CREATED),
            PartyIndex::new(0),
            DialogIndex::new(0),
            content,
        )
    };
    let mut without = with_two_parties_and_a_recording();
    without.add_attachment(attachment(Content::Inline(Body::Text("notes".into()))));
    assert_eq!(
        without.to_json(),
        Err(WriteError::MissingParameter {
            path: "/attachments/0".into(),
            name: "mediatype"
        }),
        "an inline attachment needs a mediatype"
    );
    let mut with = with_two_parties_and_a_recording();
    with.add_attachment(
        attachment(Content::Inline(Body::Text("notes".into()))).mediatype("text/plain"),
    );
    assert_eq!(
        json_of(&with)["attachments"][0]["mediatype"],
        json!("text/plain"),
        "the mediatype is written"
    );
    let mut external = with_two_parties_and_a_recording();
    external.add_attachment(attachment(external_content()));
    assert!(
        json_of(&external)["attachments"][0]
            .get("mediatype")
            .is_none(),
        "an external attachment needs no mediatype"
    );
}

/// External content for an attachment.
fn external_content() -> Content {
    external("https://example.com/x.pdf")
}

// --- Common traits (Rust API Guidelines C-COMMON-TRAITS, C-GOOD-ERR) ---

/// A WriteError goes into a `Box<dyn Error + Send + Sync>`, so a caller can
/// return it with `?` from any thread.
#[test]
fn a_write_error_is_an_error_that_crosses_threads() {
    fn boxed(e: WriteError) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(e)
    }
    let e = boxed(WriteError::BadHash);
    assert_eq!(
        e.to_string(),
        "a content hash is malformed",
        "its Display, read through the box"
    );
}

#[test]
fn every_refusal_says_what_is_wrong_in_lower_case_without_a_full_stop() {
    let cases = [
        (
            WriteError::UnknownIndex {
                path: "/dialog/0/parties/1".into(),
            },
            "the index at /dialog/0/parties/1 refers to no entry",
        ),
        (
            WriteError::MissingParameter {
                path: "/dialog/1".into(),
                name: "recordings",
            },
            "/dialog/1 lacks recordings, which its type requires",
        ),
        (
            WriteError::ForbiddenParameter {
                path: "/dialog/1".into(),
                name: "parties",
            },
            "/dialog/1 has parties, which its type does not allow",
        ),
        (
            WriteError::BadContent {
                path: "/attachments/0".into(),
            },
            "the content of /attachments/0 cannot be written there",
        ),
        (WriteError::BadHash, "a content hash is malformed"),
        (WriteError::BadDate, "a date is not RFC 3339 with an offset"),
        (
            WriteError::BadDuration,
            "a duration is not an unsigned number of seconds",
        ),
        (
            WriteError::BadReference,
            "a reference to a prior vCon is incomplete or its url is not https",
        ),
        (WriteError::BothLinks, "both amended and redacted are set"),
        (
            WriteError::SameUuid,
            "a vCon refers to a vCon with its own uuid",
        ),
        (
            WriteError::CoreMember("uuid".into()),
            "the member name uuid is reserved",
        ),
        (
            WriteError::NoPlaceholder(DialogType::Text),
            "no placeholder is made of type text",
        ),
        (
            WriteError::TooDeep,
            "the output would nest more than 64 arrays and objects",
        ),
    ];
    for (e, text) in cases {
        assert_eq!(e.to_string(), text, "the message of {e:?}");
    }
}

#[test]
fn the_text_types_display_as_they_are_written() {
    let created = date(CREATED);
    assert_eq!(created.to_string(), CREATED, "a Date displays its text");
    let hash = sha512(&[1, 2, 3]);
    assert_eq!(
        hash.to_string(),
        "sha512-AQID",
        "a ContentHash displays its token"
    );
    for kind in ALL_TYPES {
        assert_eq!(
            kind.to_string(),
            kind.as_str(),
            "{kind:?} displays as written"
        );
    }
    for d in [
        Disposition::NoAnswer,
        Disposition::Congestion,
        Disposition::Failed,
        Disposition::Busy,
        Disposition::HungUp,
        Disposition::VoicemailNoMessage,
    ] {
        assert_eq!(d.to_string(), d.as_str(), "{d:?} displays as written");
    }
}

// --- WRT-17: durations ---

#[test]
fn a_whole_duration_has_no_decimal_point_and_a_fractional_one_has_one() {
    let mut v = base();
    for duration in [
        Duration::try_from_secs(2).expect("2"),
        Duration::try_from_secs_f64(2.0).expect("2.0"),
        Duration::try_from_secs_f64(33.12).expect("33.12"),
        Duration::try_from_secs((1 << 53) - 1).expect("2^53 - 1"),
        Duration::try_from_secs_f64(0.0).expect("0.0"),
    ] {
        v.add_dialog(minimal(DialogType::Incomplete).duration(duration));
    }
    let text = String::from_utf8(v.to_json().expect("written")).expect("UTF-8");
    for written in [
        r#"{"type":"incomplete","duration":2,"disposition":"failed"}"#,
        r#"{"type":"incomplete","duration":2.0,"disposition":"failed"}"#,
        r#"{"type":"incomplete","duration":33.12,"disposition":"failed"}"#,
        r#"{"type":"incomplete","duration":9007199254740991,"disposition":"failed"}"#,
        r#"{"type":"incomplete","duration":0.0,"disposition":"failed"}"#,
    ] {
        assert!(text.contains(written), "{written} in {text}");
    }
}

#[test]
fn a_duration_that_is_not_an_unsigned_number_is_refused() {
    assert_eq!(
        Duration::try_from_secs(1 << 53),
        Err(WriteError::BadDuration),
        "2^53 is above the largest UnsignedInt"
    );
    for seconds in [f64::NAN, f64::INFINITY, -0.5, -0.0, 1e300, 1e-7] {
        assert_eq!(
            Duration::try_from_secs_f64(seconds),
            Err(WriteError::BadDuration),
            "{seconds}"
        );
    }
}

// --- WRT-18: extension member names ---

const RESERVED: [&str; 3] = ["vcon", "must_support", "appended"];

const TOP: &[&str] = &[
    "uuid",
    "extensions",
    "critical",
    "created_at",
    "updated_at",
    "subject",
    "redacted",
    "amended",
    "parties",
    "dialog",
    "analysis",
    "attachments",
    "payload",
    "signatures",
    "ciphertext",
    "recipients",
];
const PARTY: &[&str] = &[
    "tel",
    "sip",
    "stir",
    "mailto",
    "name",
    "did",
    "validation",
    "gmlpos",
    "civicaddress",
    "uuid",
    "type",
    "org",
    "dept",
];
const DIALOG: &[&str] = &[
    "type",
    "start",
    "duration",
    "parties",
    "originator",
    "recordings",
    "recording_set",
    "mediatype",
    "filename",
    "body",
    "encoding",
    "url",
    "content_hash",
    "disposition",
    "session_id",
    "party_history",
    "transferee",
    "transferor",
    "transfer_target",
    "original",
    "consultation",
    "target_dialog",
    "application",
    "message_id",
];
const ATTACHMENT: &[&str] = &[
    "purpose",
    "start",
    "party",
    "dialog",
    "mediatype",
    "filename",
    "body",
    "encoding",
    "url",
    "content_hash",
];
const ANALYSIS: &[&str] = &[
    "type",
    "dialog",
    "attachment",
    "mediatype",
    "filename",
    "vendor",
    "product",
    "schema",
    "body",
    "encoding",
    "url",
    "content_hash",
];

fn refused(name: &str) -> WriteError {
    WriteError::CoreMember(name.to_owned())
}

#[test]
fn an_extension_member_may_not_take_a_name_the_draft_reserves() {
    for name in TOP.iter().chain(&RESERVED) {
        assert_eq!(
            base().set_member(name, json!(1)),
            Err(refused(name)),
            "top {name}"
        );
    }
    for name in PARTY.iter().chain(&RESERVED) {
        assert_eq!(
            Party::new().set_member(name, json!(1)).err(),
            Some(refused(name)),
            "party {name}"
        );
    }
    for name in DIALOG.iter().chain(&RESERVED) {
        assert_eq!(
            Dialog::transfer().set_member(name, json!(1)).err(),
            Some(refused(name)),
            "dialog {name}"
        );
    }
    let attachment = || {
        Attachment::new(
            date(CREATED),
            PartyIndex::new(0),
            DialogIndex::new(0),
            Content::Absent,
        )
    };
    for name in ATTACHMENT.iter().chain(&RESERVED) {
        assert_eq!(
            attachment().set_member(name, json!(1)).err(),
            Some(refused(name)),
            "attachment {name}"
        );
    }
    for name in ANALYSIS.iter().chain(&RESERVED) {
        assert_eq!(
            Analysis::new("summary", "Example", Content::Absent)
                .set_member(name, json!(1))
                .err(),
            Some(refused(name)),
            "analysis {name}"
        );
    }
}

#[test]
fn a_name_another_object_reserves_is_allowed_where_it_is_not_reserved() {
    let party = Party::new()
        .set_member("vendor", json!("x"))
        .expect("vendor on a party");
    let mut v = base();
    v.add_party(party);
    v.set_member("tel", json!("x")).expect("tel at the top");
    let out = json_of(&v);
    assert_eq!(
        (&out["parties"][0]["vendor"], &out["tel"]),
        (&json!("x"), &json!("x")),
        "a name reserved elsewhere is allowed here"
    );
}

#[test]
fn a_member_set_again_is_written_once_in_its_first_place() {
    let mut v = base();
    v.set_member("first", json!(1)).expect("member");
    v.set_member("second", json!(2)).expect("member");
    v.set_member("first", json!(3)).expect("member");
    let text = String::from_utf8(v.to_json().expect("written")).expect("UTF-8");
    assert!(
        text.ends_with(r#""attachments":[],"first":3,"second":2}"#),
        "{text}"
    );
}

// --- WRT-19: required members and closed value sets ---

#[test]
fn every_closed_value_is_spelled_as_the_draft_spells_it() {
    let types: Vec<&str> = ALL_TYPES.iter().map(|t| t.as_str()).collect();
    assert_eq!(
        types,
        [
            "recording",
            "recording-set",
            "text",
            "transfer",
            "incomplete"
        ],
        "every dialog type spelled as the draft spells it"
    );
    let dispositions = [
        (Disposition::NoAnswer, "no-answer"),
        (Disposition::Congestion, "congestion"),
        (Disposition::Failed, "failed"),
        (Disposition::Busy, "busy"),
        (Disposition::HungUp, "hung-up"),
        (Disposition::VoicemailNoMessage, "voicemail-no-message"),
    ];
    for (d, text) in dispositions {
        assert_eq!(
            d.as_str(),
            text,
            "the disposition spelled as the draft spells it"
        );
        let mut v = base();
        v.add_dialog(Dialog::incomplete(d));
        assert_eq!(
            json_of(&v)["dialog"][0]["disposition"],
            json!(text),
            "the disposition written as the draft spells it"
        );
    }
}

#[test]
fn every_party_event_is_written_and_key_events_carry_their_button() {
    let events = [
        (Event::Join, json!({"event": "join"})),
        (Event::Drop, json!({"event": "drop"})),
        (Event::Hold, json!({"event": "hold"})),
        (Event::Unhold, json!({"event": "unhold"})),
        (Event::Mute, json!({"event": "mute"})),
        (Event::Unmute, json!({"event": "unmute"})),
        (
            Event::Keydown("5".into()),
            json!({"event": "keydown", "button": "5"}),
        ),
        (
            Event::Keyup("#".into()),
            json!({"event": "keyup", "button": "#"}),
        ),
    ];
    let mut v = with_two_parties_and_a_recording();
    let history = events
        .iter()
        .map(|(e, _)| {
            PartyHistory::new(PartyIndex::new(1), date("2022-06-21T17:53:27Z"), e.clone())
        })
        .collect();
    v.add_dialog(minimal(DialogType::Recording).party_history(history));
    let out = json_of(&v);
    for (i, (_, expected)) in events.iter().enumerate() {
        let mut want = json!({"party": 1, "time": "2022-06-21T17:53:27Z"});
        for (k, val) in expected.as_object().expect("object") {
            want[k] = val.clone();
        }
        assert_eq!(out["dialog"][1]["party_history"][i], want, "event {i}");
    }
}

#[test]
fn an_attachment_and_an_analysis_are_written_with_their_required_members() {
    let mut v = with_two_parties_and_a_recording();
    v.add_attachment(
        Attachment::new(
            date("2022-06-21T17:55:00Z"),
            PartyIndex::new(1),
            DialogIndex::new(0),
            Content::Inline(Body::Text("notes".into())),
        )
        .mediatype("text/plain"),
    );
    v.add_analysis(Analysis::new(
        "sentiment",
        "Example",
        Content::Inline(Body::Json(json!(0.5))),
    ));
    let out = json_of(&v);
    assert_eq!(
        out["attachments"][0],
        json!({"start": "2022-06-21T17:55:00Z", "party": 1, "dialog": 0,
               "mediatype": "text/plain", "body": "notes", "encoding": "none"}),
        "an attachment written with its required members"
    );
    assert_eq!(
        out["analysis"][0],
        json!({"type": "sentiment", "vendor": "Example", "body": 0.5, "encoding": "json"}),
        "an analysis written with its required members"
    );
}
