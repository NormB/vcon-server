//! The identity every unsigned vCon must carry: `uuid` and `created_at`.

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::Rejection;

/// Returns the vCon's uuid. `uuid` must be a string in RFC 9562 text form,
/// of any version, and `created_at` must be present.
pub(crate) fn unsigned_identity(object: &Map<String, Value>) -> Result<Uuid, Rejection> {
    let uuid = match object.get("uuid") {
        None => return Err(Rejection::MissingUuid),
        Some(Value::String(text)) => parse_uuid_text(text)?,
        Some(_) => return Err(Rejection::BadUuid),
    };
    if !object.contains_key("created_at") {
        return Err(Rejection::MissingCreatedAt);
    }
    Ok(uuid)
}

/// Returns the uuid of an encrypted vCon, the only member a server without
/// the key can read: `unprotected.uuid`, in RFC 9562 text form.
pub(crate) fn encrypted_identity(object: &Map<String, Value>) -> Result<Uuid, Rejection> {
    match object.get("unprotected").and_then(|u| u.get("uuid")) {
        None => Err(Rejection::MissingUuid),
        Some(Value::String(text)) => parse_uuid_text(text),
        Some(_) => Err(Rejection::BadUuid),
    }
}

/// Parses the RFC 9562 text form only: 8-4-4-4-12 hex digits, either case.
pub(crate) fn parse_uuid_text(text: &str) -> Result<Uuid, Rejection> {
    // `Uuid::parse_str` also accepts the simple (32 characters), braced (38)
    // and URN (45) forms; the RFC 9562 text form is the only one of 36.
    if text.len() != 36 {
        return Err(Rejection::BadUuid);
    }
    Uuid::parse_str(text).map_err(|_| Rejection::BadUuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

    fn identity(body: &str) -> Result<Uuid, Rejection> {
        let Value::Object(map) = serde_json::from_str(body).expect("test JSON") else {
            panic!("test body must be an object")
        };
        unsigned_identity(&map)
    }

    #[test]
    fn returns_the_uuid() {
        let body = format!(r#"{{"uuid": "{ID}", "created_at": "2026-10-23T00:00:00Z"}}"#);
        assert_eq!(identity(&body), Ok(ID.parse().expect("test uuid")));
    }

    #[test]
    fn refuses_a_missing_or_malformed_identity() {
        let cases = [
            (
                "no uuid",
                r#"{"created_at": "x"}"#.to_owned(),
                Rejection::MissingUuid,
            ),
            (
                "uuid not a string",
                r#"{"uuid": 7, "created_at": "x"}"#.to_owned(),
                Rejection::BadUuid,
            ),
            (
                "uuid null",
                r#"{"uuid": null, "created_at": "x"}"#.to_owned(),
                Rejection::BadUuid,
            ),
            (
                "no created_at",
                format!(r#"{{"uuid": "{ID}"}}"#),
                Rejection::MissingCreatedAt,
            ),
            (
                "bad uuid beats missing created_at",
                r#"{"uuid": "x"}"#.to_owned(),
                Rejection::BadUuid,
            ),
        ];
        for (what, body, expected) in cases {
            assert_eq!(identity(&body), Err(expected), "{what}");
        }
    }

    fn encrypted(body: &str) -> Result<Uuid, Rejection> {
        let Value::Object(map) = serde_json::from_str(body).expect("test JSON") else {
            panic!("test body must be an object")
        };
        encrypted_identity(&map)
    }

    #[test]
    fn reads_the_unprotected_uuid_of_an_encrypted_vcon() {
        let body = format!(
            r#"{{"unprotected": {{"uuid": "{ID}", "cty": "application/vcon"}}, "ciphertext": "x", "recipients": []}}"#
        );
        assert_eq!(encrypted(&body), Ok(ID.parse().expect("test uuid")));
        let no_created_at = format!(r#"{{"unprotected": {{"uuid": "{ID}"}}}}"#);
        assert!(
            encrypted(&no_created_at).is_ok(),
            "nothing else is required"
        );
    }

    #[test]
    fn refuses_an_encrypted_vcon_without_a_readable_uuid() {
        let cases = [
            (
                "no unprotected",
                r#"{"ciphertext": "x"}"#.to_owned(),
                Rejection::MissingUuid,
            ),
            (
                "unprotected not an object",
                r#"{"unprotected": "x"}"#.to_owned(),
                Rejection::MissingUuid,
            ),
            (
                "no uuid",
                r#"{"unprotected": {"cty": "x"}}"#.to_owned(),
                Rejection::MissingUuid,
            ),
            (
                "uuid not a string",
                r#"{"unprotected": {"uuid": 1}}"#.to_owned(),
                Rejection::BadUuid,
            ),
            (
                "uuid malformed",
                r#"{"unprotected": {"uuid": "x"}}"#.to_owned(),
                Rejection::BadUuid,
            ),
            (
                "uuid not in the text form",
                format!(r#"{{"unprotected": {{"uuid": "{{{ID}}}"}}}}"#),
                Rejection::BadUuid,
            ),
            (
                "top-level uuid does not count",
                format!(r#"{{"uuid": "{ID}", "unprotected": {{}}}}"#),
                Rejection::MissingUuid,
            ),
        ];
        for (what, body, expected) in cases {
            assert_eq!(encrypted(&body), Err(expected), "{what}");
        }
    }

    #[test]
    fn accepts_only_the_rfc_9562_text_form() {
        let good = [
            ID,
            "01A07DA8-C2BB-83E5-B9A2-279E0D16BC46",
            "01a07DA8-c2BB-83e5-B9a2-279e0d16BC46",
            "00000000-0000-0000-0000-000000000000",
            "f81d4fae-7dec-11d0-a765-00a0c91e6bf6",
        ];
        for text in good {
            assert_eq!(
                parse_uuid_text(text),
                Ok(Uuid::parse_str(text).expect("valid")),
                "{text}"
            );
        }
        let bad = [
            "",
            "01a07da8c2bb83e5b9a2279e0d16bc46",
            "{01a07da8-c2bb-83e5-b9a2-279e0d16bc46}",
            "urn:uuid:01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
            " 01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc46 ",
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc4",
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc466",
            "01a07da8-c2bb-83e5-b9a2_279e0d16bc46",
            "01a07da8c-2bb-83e5-b9a2-279e0d16bc46",
            "g1a07da8-c2bb-83e5-b9a2-279e0d16bc46",
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc4\u{e9}",
        ];
        for text in bad {
            assert_eq!(parse_uuid_text(text), Err(Rejection::BadUuid), "{text:?}");
        }
    }
}
