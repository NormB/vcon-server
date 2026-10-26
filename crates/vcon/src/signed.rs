//! The signed (JWS) form: the vCon is the base64url payload, possibly
//! gzipped, possibly itself a JWS when a vCon has been signed again.

use std::collections::BTreeSet;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::critical::check_critical;
use crate::form::form_of;
use crate::gzip::inflate;
use crate::identity::{parse_uuid_text, unsigned_identity};
use crate::json::parse_object;
use crate::{Form, Rejection};

/// Returns the uuid of the vCon inside a signed body, opening nested JWS
/// layers. `budget` is what is left of the inflation limit and shrinks with
/// every gzip layer, so all layers together stay within one limit.
pub(crate) fn signed_identity(
    object: &Map<String, Value>,
    budget: &mut u64,
    supported: &BTreeSet<String>,
) -> Result<Uuid, Rejection> {
    let header_uuids = header_uuids(object)?;
    let Some(Value::String(payload)) = object.get("payload") else {
        return Err(Rejection::BadPayload);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| Rejection::BadPayload)?;
    let (inner, gzip) = inflate(&bytes, None, *budget)?;
    if gzip {
        *budget -= inner.len() as u64;
    }
    let inner = parse_object(&inner).map_err(|r| match r {
        Rejection::NotJson => Rejection::BadPayload,
        other => other,
    })?;
    let uuid = match form_of(&inner) {
        Ok(Form::Signed) => signed_identity(&inner, budget, supported)?,
        Ok(Form::Unsigned) => {
            let uuid = unsigned_identity(&inner)?;
            check_critical(&inner, supported)?;
            uuid
        }
        _ => return Err(Rejection::BadPayload),
    };
    for header_uuid in header_uuids {
        match header_uuid {
            Value::String(text) if parse_uuid_text(text) == Ok(uuid) => {}
            _ => return Err(Rejection::UuidMismatch),
        }
    }
    Ok(uuid)
}

/// The `uuid` members of every signature's header. `signatures` must be an
/// array of objects, and each `header`, where present, an object.
fn header_uuids(object: &Map<String, Value>) -> Result<Vec<&Value>, Rejection> {
    let Some(Value::Array(signatures)) = object.get("signatures") else {
        return Err(Rejection::BadPayload);
    };
    let mut uuids = Vec::new();
    for signature in signatures {
        let Value::Object(signature) = signature else {
            return Err(Rejection::BadPayload);
        };
        match signature.get("header") {
            None => {}
            Some(Value::Object(header)) => uuids.extend(header.get("uuid")),
            Some(_) => return Err(Rejection::BadPayload),
        }
    }
    Ok(uuids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
    use flate2::{Compression, write::GzEncoder};
    use serde_json::json;
    use std::io::Write;

    const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
    const OTHER: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc47";
    const LIMIT: u64 = 1 << 20;

    fn unsigned() -> Value {
        json!({"uuid": ID, "created_at": "2026-10-23T00:00:00Z", "parties": []})
    }

    fn jws(payload: String, header: Value) -> Value {
        json!({"payload": payload, "signatures": [{"header": header, "protected": "", "signature": ""}]})
    }

    fn b64(bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(bytes).expect("compress");
        e.finish().expect("finish")
    }

    fn sign(inner: &Value, header: Value) -> Value {
        jws(b64(inner.to_string().as_bytes()), header)
    }

    fn run(outer: &Value, limit: u64, supported: &[&str]) -> Result<Uuid, Rejection> {
        let Value::Object(map) = outer else {
            panic!("object")
        };
        let supported = supported.iter().map(|s| s.to_string()).collect();
        let mut budget = limit;
        signed_identity(map, &mut budget, &supported)
    }

    fn id() -> Uuid {
        ID.parse().expect("test uuid")
    }

    #[test]
    fn reads_the_uuid_of_the_signed_vcon() {
        let cases = [
            (
                "header uuid matches",
                sign(&unsigned(), json!({"alg": "RS256", "uuid": ID})),
            ),
            (
                "header uuid in upper case",
                sign(&unsigned(), json!({"uuid": ID.to_uppercase()})),
            ),
            ("no header uuid", sign(&unsigned(), json!({"alg": "RS256"}))),
            (
                "no header at all",
                json!({"payload": b64(unsigned().to_string().as_bytes()), "signatures": [{}]}),
            ),
            (
                "gzipped payload",
                jws(
                    b64(&gz(unsigned().to_string().as_bytes())),
                    json!({"uuid": ID}),
                ),
            ),
            (
                "two layers, re-signed",
                sign(&sign(&unsigned(), json!({"uuid": ID})), json!({"uuid": ID})),
            ),
            (
                "three layers, middle one gzipped",
                sign(
                    &jws(
                        b64(&gz(sign(&unsigned(), json!({})).to_string().as_bytes())),
                        json!({"uuid": ID}),
                    ),
                    json!({}),
                ),
            ),
        ];
        for (what, outer) in cases {
            assert_eq!(run(&outer, LIMIT, &[]), Ok(id()), "{what}");
        }
    }

    #[test]
    fn refuses_a_header_uuid_that_differs() {
        let cases = [
            ("outer header", sign(&unsigned(), json!({"uuid": OTHER}))),
            ("not a uuid", sign(&unsigned(), json!({"uuid": "nope"}))),
            ("not a string", sign(&unsigned(), json!({"uuid": 5}))),
            (
                "second signature",
                json!({"payload": b64(unsigned().to_string().as_bytes()),
                       "signatures": [{"header": {"uuid": ID}}, {"header": {"uuid": OTHER}}]}),
            ),
            (
                "inner layer",
                sign(
                    &sign(&unsigned(), json!({"uuid": OTHER})),
                    json!({"uuid": ID}),
                ),
            ),
            (
                "outer layer",
                sign(
                    &sign(&unsigned(), json!({"uuid": ID})),
                    json!({"uuid": OTHER}),
                ),
            ),
        ];
        for (what, outer) in cases {
            assert_eq!(
                run(&outer, LIMIT, &[]),
                Err(Rejection::UuidMismatch),
                "{what}"
            );
        }
    }

    #[test]
    fn refuses_a_payload_that_is_not_a_vcon() {
        let text = unsigned().to_string();
        let cases = [
            (
                "payload not a string",
                json!({"payload": 1, "signatures": []}),
            ),
            (
                "padding",
                jws(URL_SAFE.encode(text.as_bytes() as &[u8]) + "==", json!({})),
            ),
            (
                "standard alphabet",
                jws(STANDARD.encode([0xfb_u8, 0xff, 0xbf]), json!({})),
            ),
            (
                "whitespace",
                jws(format!(" {}", b64(text.as_bytes())), json!({})),
            ),
            ("not JSON", jws(b64(b"hello"), json!({}))),
            ("JSON array", jws(b64(b"[]"), json!({}))),
            ("no form inside", jws(b64(b"{}"), json!({}))),
            (
                "encrypted inside",
                jws(
                    b64(json!({"ciphertext": "x", "recipients": []})
                        .to_string()
                        .as_bytes()),
                    json!({}),
                ),
            ),
            (
                "signatures not an array",
                json!({"payload": b64(text.as_bytes()), "signatures": {}}),
            ),
            (
                "a signature not an object",
                json!({"payload": b64(text.as_bytes()), "signatures": [1]}),
            ),
            (
                "a header not an object",
                json!({"payload": b64(text.as_bytes()), "signatures": [{"header": []}]}),
            ),
        ];
        for (what, outer) in cases {
            assert_eq!(
                run(&outer, LIMIT, &[]),
                Err(Rejection::BadPayload),
                "{what}"
            );
        }
    }

    #[test]
    fn checks_the_inner_vcon_like_an_unsigned_one() {
        let no_uuid = json!({"created_at": "x", "parties": []});
        assert_eq!(
            run(&sign(&no_uuid, json!({})), LIMIT, &[]),
            Err(Rejection::MissingUuid)
        );
        let critical = json!({"uuid": ID, "created_at": "x", "critical": ["x"]});
        assert_eq!(
            run(&sign(&critical, json!({})), LIMIT, &[]),
            Err(Rejection::UnsupportedCritical(vec!["x".to_owned()]))
        );
        assert_eq!(run(&sign(&critical, json!({})), LIMIT, &["x"]), Ok(id()));
        let dup = jws(b64(br#"{"uuid": "a", "uuid": "b"}"#), json!({}));
        assert_eq!(run(&dup, LIMIT, &[]), Err(Rejection::DuplicateKey));
    }

    #[test]
    fn all_layers_share_one_inflation_budget() {
        let padded = json!({"uuid": ID, "created_at": "x", "pad": " ".repeat(3000)});
        let inner = jws(b64(&gz(padded.to_string().as_bytes())), json!({}));
        let outer = jws(b64(&gz(inner.to_string().as_bytes())), json!({}));
        let inner_size = padded.to_string().len() as u64;
        let outer_size = inner.to_string().len() as u64;
        assert_eq!(run(&outer, inner_size + outer_size, &[]), Ok(id()));
        assert_eq!(
            run(&outer, inner_size + outer_size - 1, &[]),
            Err(Rejection::TooLarge)
        );
        assert!(inner_size.max(outer_size) < inner_size + outer_size - 1);
    }
}
