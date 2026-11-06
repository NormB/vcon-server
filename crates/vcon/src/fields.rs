//! The members of an unsigned vCon, or of the vCon inside a signed one,
//! that an export may copy into clear columns: its times, its subject, its
//! parties' addresses and names, and how many dialogs it has. An encrypted
//! vCon has none that a reader without its key can see.

use serde_json::{Map, Value};

use crate::read::{Fields, Party};

/// The fields of `object`, an unsigned vCon. A member that is missing, or
/// is not of the type the vCon core draft gives it, is left out: these are
/// copies for analytics, never the vCon itself.
pub(crate) fn fields_of(object: &Map<String, Value>) -> Fields {
    let text = |object: &Map<String, Value>, name: &str| {
        object.get(name).and_then(Value::as_str).map(str::to_owned)
    };
    let parties = object.get("parties").and_then(Value::as_array).map(|all| {
        all.iter()
            .map(|party| match party {
                Value::Object(party) => Party {
                    tel: text(party, "tel"),
                    mailto: text(party, "mailto"),
                    name: text(party, "name"),
                },
                _ => Party::default(),
            })
            .collect()
    });
    Fields {
        created_at: text(object, "created_at"),
        updated_at: text(object, "updated_at"),
        subject: text(object, "subject"),
        parties,
        dialogs: object.get("dialog").and_then(Value::as_array).map(Vec::len),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::json;

    use crate::read::{Limits, Rejection, fields};

    const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
    const LIMITS: Limits = Limits::new(1 << 20);

    fn full() -> Value {
        json!({
            "uuid": ID,
            "created_at": "2026-10-23T00:00:00Z",
            "updated_at": "2026-10-24T00:00:00Z",
            "subject": "a call about an order",
            "parties": [
                {"tel": "+15555550101", "name": "Alice Example"},
                {"mailto": "bob@example.com"},
                "not an object"
            ],
            "dialog": [{"type": "text"}, {"type": "text"}, {"type": "text"}]
        })
    }

    use serde_json::Value;

    fn expected() -> Fields {
        Fields {
            created_at: Some("2026-10-23T00:00:00Z".to_owned()),
            updated_at: Some("2026-10-24T00:00:00Z".to_owned()),
            subject: Some("a call about an order".to_owned()),
            parties: Some(vec![
                Party {
                    tel: Some("+15555550101".to_owned()),
                    mailto: None,
                    name: Some("Alice Example".to_owned()),
                },
                Party {
                    tel: None,
                    mailto: Some("bob@example.com".to_owned()),
                    name: None,
                },
                Party::default(),
            ]),
            dialogs: Some(3),
        }
    }

    use super::{Fields, Party};

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(bytes).expect("compress");
        e.finish().expect("finish")
    }

    fn signed(payload: &[u8]) -> Vec<u8> {
        json!({
            "payload": URL_SAFE_NO_PAD.encode(payload),
            "signatures": [{"header": {"uuid": ID}, "protected": "", "signature": ""}]
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn an_unsigned_vcon_gives_each_field() {
        let body = full().to_string().into_bytes();
        assert_eq!(fields(&body, &LIMITS), Ok(Some(expected())), "plain");
        assert_eq!(fields(&gz(&body), &LIMITS), Ok(Some(expected())), "gzipped");
    }

    /// The vCon inside every signed layer is read, gzipped or not.
    #[test]
    fn a_signed_vcon_gives_the_fields_of_the_vcon_it_signs() {
        let inner = full().to_string().into_bytes();
        let once = signed(&inner);
        let twice = signed(&gz(&once));
        for body in [once.clone(), gz(&once), signed(&gz(&inner)), twice] {
            assert_eq!(
                fields(&body, &LIMITS),
                Ok(Some(expected())),
                "{}",
                String::from_utf8_lossy(&body)
            );
        }
    }

    #[test]
    fn an_encrypted_vcon_gives_no_fields() {
        let body = json!({
            "unprotected": {"uuid": ID},
            "recipients": [],
            "ciphertext": "",
            "iv": "",
            "tag": "",
            "protected": ""
        })
        .to_string()
        .into_bytes();
        assert_eq!(fields(&body, &LIMITS), Ok(None), "nothing can be read");
    }

    /// Members of another type are left out, as are missing ones.
    #[test]
    fn a_member_of_the_wrong_type_is_left_out() {
        let body = json!({
            "uuid": ID,
            "created_at": 7,
            "subject": ["x"],
            "parties": {"tel": "+15555550101"},
            "dialog": "none"
        })
        .to_string()
        .into_bytes();
        assert_eq!(
            fields(&body, &LIMITS),
            Ok(Some(Fields::default())),
            "no field of the wrong type"
        );
        let bare = json!({"uuid": ID, "created_at": "2026-10-23T00:00:00Z"})
            .to_string()
            .into_bytes();
        assert_eq!(
            fields(&bare, &LIMITS),
            Ok(Some(Fields {
                created_at: Some("2026-10-23T00:00:00Z".to_owned()),
                ..Fields::default()
            })),
            "missing members are absent"
        );
    }

    /// The same bounds as `inspect`: a body past the inflation limit is
    /// refused, never read in full.
    #[test]
    fn the_inflation_limit_holds() {
        let body = gz(&vec![b' '; (LIMITS.inflated + 1) as usize]);
        assert_eq!(
            fields(&body, &LIMITS),
            Err(Rejection::TooLarge),
            "a body past the inflation limit is refused"
        );
        assert_eq!(
            fields(b"[]", &LIMITS),
            Err(Rejection::NotJson),
            "a body that is not a JSON object is refused"
        );
    }
}
