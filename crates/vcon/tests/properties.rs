//! Properties of `inspect` over arbitrary input.

use std::collections::BTreeSet;
use std::io::Write;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::Compression;
use flate2::write::GzEncoder;
use proptest::arbitrary::any;
use proptest::strategy::{Just, Strategy};
use proptest::{collection, prop_assert, prop_assert_eq, prop_assume, prop_oneof, proptest};
use serde_json::{Map, Value, json};
use uuid::Uuid;
use vcon::Form;
use vcon::read::{Limits, Rejection, inspect};

const LIMIT: u64 = 4096;

fn gz(bytes: &[u8]) -> Vec<u8> {
    let mut e = GzEncoder::new(Vec::new(), Compression::fast());
    e.write_all(bytes).expect("compress");
    e.finish().expect("finish")
}

fn media_type() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        Just(Some("application/vcon".to_owned())),
        Just(Some("application/vcon+gzip".to_owned())),
        ".{0,40}".prop_map(Some),
    ]
}

/// JSON values whose objects use vCon member names often, so generated
/// bodies reach the form, identity and critical checks.
fn json_value() -> impl Strategy<Value = Value> {
    let name = prop_oneof![
        Just("uuid"),
        Just("created_at"),
        Just("parties"),
        Just("dialog"),
        Just("payload"),
        Just("signatures"),
        Just("ciphertext"),
        Just("recipients"),
        Just("unprotected"),
        Just("critical"),
        Just("must_support"),
        Just("header"),
        Just("x"),
    ]
    .prop_map(str::to_owned);
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        Just(Value::from("01a07da8-c2bb-83e5-b9a2-279e0d16bc46")),
        ".{0,12}".prop_map(Value::from),
    ];
    leaf.prop_recursive(4, 48, 6, move |inner| {
        prop_oneof![
            collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            collection::btree_map(name.clone(), inner, 0..6)
                .prop_map(|m| Value::Object(m.into_iter().collect::<Map<_, _>>())),
        ]
    })
}

const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
const OTHER_ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc47";

/// A generated vCon and the answer `inspect` must give for it, with no
/// supported extensions.
#[derive(Debug, Clone)]
struct Case {
    body: Value,
    expected: Result<(Form, Uuid), Rejection>,
}

fn id() -> Uuid {
    Uuid::parse_str(ID).expect("test uuid")
}

/// An unsigned vCon, valid or broken in one or more ways. When several
/// checks fail, the expected answer is the first in the order `inspect`
/// checks them: form, uuid, created_at, critical.
fn unsigned_case() -> impl Strategy<Value = Case> {
    let uuid = prop_oneof![
        3 => Just((Some(Value::from(ID)), None)),
        1 => Just((None, Some(Rejection::MissingUuid))),
        1 => Just((Some(Value::from("not-a-uuid")), Some(Rejection::BadUuid))),
        1 => Just((Some(Value::from(7)), Some(Rejection::BadUuid))),
    ];
    let created_at = prop_oneof![4 => Just(true), 1 => Just(false)];
    let critical = prop_oneof![
        3 => Just((None, None)),
        1 => Just((Some(json!(["x"])), Some(Rejection::UnsupportedCritical(vec!["x".to_owned()])))),
        1 => Just((Some(json!([])), None)),
        1 => Just((Some(json!("x")), Some(Rejection::BadCritical))),
    ];
    (uuid, created_at, critical, any::<bool>()).prop_map(
        |((uuid, uuid_err), created_at, (critical, critical_err), parties)| {
            let mut m = Map::new();
            let has_uuid = uuid.is_some();
            if let Some(u) = uuid {
                m.insert("uuid".into(), u);
            }
            if created_at {
                m.insert("created_at".into(), Value::from("2026-10-23T00:00:00Z"));
            }
            if let Some(c) = critical {
                m.insert("critical".into(), c);
            }
            if parties {
                m.insert("parties".into(), json!([]));
            }
            let expected = if !has_uuid && !parties {
                Err(Rejection::NoForm)
            } else if let Some(e) = uuid_err {
                Err(e)
            } else if !created_at {
                Err(Rejection::MissingCreatedAt)
            } else if let Some(e) = critical_err {
                Err(e)
            } else {
                Ok((Form::Unsigned, id()))
            };
            Case {
                body: Value::Object(m),
                expected,
            }
        },
    )
}

/// Wraps a case in one signed layer: its payload, gzipped or not, under a
/// header with no uuid, the same uuid or another uuid.
fn sign(inner: Case, zip: bool, header: u8) -> Case {
    let text = inner.body.to_string();
    let bytes = if zip {
        gz(text.as_bytes())
    } else {
        text.into_bytes()
    };
    let (header, matches) = match header {
        0 => (json!({}), true),
        1 => (json!({"uuid": ID}), true),
        _ => (json!({"uuid": OTHER_ID}), false),
    };
    let expected = match inner.expected {
        // A broken inner vCon is reported before any outer header.
        Err(Rejection::NoForm) => Err(Rejection::BadPayload),
        Err(e) => Err(e),
        Ok(_) if !matches => Err(Rejection::UuidMismatch),
        Ok((_, uuid)) => Ok((Form::Signed, uuid)),
    };
    Case {
        body: json!({"payload": URL_SAFE_NO_PAD.encode(bytes), "signatures": [{"header": header}]}),
        expected,
    }
}

/// An unsigned vCon in zero to two signed layers, or an encrypted vCon
/// with or without a readable uuid.
fn vcon_case() -> impl Strategy<Value = Case> {
    let layers = collection::vec((any::<bool>(), 0..3u8), 0..3);
    let signed = (unsigned_case(), layers).prop_map(|(inner, layers)| {
        layers
            .into_iter()
            .fold(inner, |inner, (zip, header)| sign(inner, zip, header))
    });
    let encrypted = prop_oneof![
        Just((json!({"uuid": ID}), Ok((Form::Encrypted, id())))),
        Just((json!({}), Err(Rejection::MissingUuid))),
        Just((json!({"uuid": 1}), Err(Rejection::BadUuid))),
    ]
    .prop_map(|(unprotected, expected)| Case {
        body: json!({"unprotected": unprotected, "ciphertext": "x", "recipients": []}),
        expected,
    });
    prop_oneof![4 => signed, 1 => encrypted]
}

proptest! {
    #[test]
    fn never_panics_on_arbitrary_bytes(body in collection::vec(any::<u8>(), 0..2048), ct in media_type()) {
        let _ = inspect(&body, ct.as_deref(), &Limits::new(LIMIT), &BTreeSet::new());
    }

    #[test]
    fn never_panics_on_arbitrary_gzip(body in collection::vec(any::<u8>(), 0..2048)) {
        let _ = inspect(&gz(&body), None, &Limits::new(LIMIT), &BTreeSet::new());
    }

    #[test]
    fn never_panics_on_vcon_like_json(value in json_value(), ct in media_type()) {
        let body = value.to_string();
        let _ = inspect(body.as_bytes(), ct.as_deref(), &Limits::new(LIMIT), &BTreeSet::new());
    }

    #[test]
    fn anything_inflating_past_the_limit_is_too_large(body in collection::vec(any::<u8>(), (LIMIT as usize + 1)..(LIMIT as usize * 4))) {
        prop_assert_eq!(
            inspect(&gz(&body), None, &Limits::new(LIMIT), &BTreeSet::new()),
            Err(Rejection::TooLarge),
            "the body inflates to more than LIMIT bytes, whatever they are"
        );
    }

    #[test]
    fn gzip_changes_only_the_gzip_flag(value in json_value()) {
        let body = value.to_string();
        prop_assume!((body.len() as u64) <= LIMIT);
        let limits = Limits::new(LIMIT);
        let plain = inspect(body.as_bytes(), None, &limits, &BTreeSet::new());
        let zipped = inspect(&gz(body.as_bytes()), None, &limits, &BTreeSet::new());
        match (plain, zipped) {
            (Ok(p), Ok(z)) => {
                prop_assert!(!p.gzip && z.gzip, "only the gzipped body is marked as gzip");
                prop_assert_eq!(
                    (p.form, p.uuid),
                    (z.form, z.uuid),
                    "gzip does not change what the vCon is"
                );
            }
            (p, z) => prop_assert_eq!(p, z, "gzip does not change why a body is refused"),
        }
    }

    #[test]
    fn gives_the_answer_a_generated_vcon_was_built_to_get(case in vcon_case(), zip in any::<bool>()) {
        let text = case.body.to_string();
        let body = if zip { gz(text.as_bytes()) } else { text.into_bytes() };
        let got = inspect(&body, None, &Limits::new(LIMIT), &BTreeSet::new())
            .map(|i| (i.form, i.uuid, i.gzip));
        let expected = case.expected.map(|(form, uuid)| (form, uuid, zip));
        prop_assert_eq!(got, expected, "{}", case.body);
    }
}
