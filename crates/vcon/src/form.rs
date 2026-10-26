//! Which of the three vCon forms a parsed body is in.

use serde_json::{Map, Value};

use crate::{Form, Rejection};

/// Tells the form from the members present. A pair means signed or
/// encrypted; any member an unsigned vCon carries means unsigned.
pub(crate) fn form_of(object: &Map<String, Value>) -> Result<Form, Rejection> {
    let has = |name: &str| object.contains_key(name);
    let signed = has("payload") && has("signatures");
    let encrypted = has("ciphertext") && has("recipients");
    match (signed, encrypted) {
        (true, true) => Err(Rejection::AmbiguousForm),
        (true, false) => Ok(Form::Signed),
        (false, true) => Ok(Form::Encrypted),
        (false, false) => {
            if UNSIGNED_MEMBERS.iter().any(|m| has(m)) {
                Ok(Form::Unsigned)
            } else {
                Err(Rejection::NoForm)
            }
        }
    }
}

/// Members whose presence marks the unsigned form: the four arrays core-04
/// lists, plus `uuid`, which every vCon must carry.
const UNSIGNED_MEMBERS: [&str; 5] = ["parties", "dialog", "analysis", "attachments", "uuid"];

#[cfg(test)]
mod tests {
    use super::*;

    fn form(body: &str) -> Result<Form, Rejection> {
        let Value::Object(map) = serde_json::from_str(body).expect("test JSON") else {
            panic!("test body must be an object")
        };
        form_of(&map)
    }

    #[test]
    fn tells_each_form_apart() {
        let cases = [
            (
                "signed",
                r#"{"payload": "x", "signatures": []}"#,
                Ok(Form::Signed),
            ),
            (
                "encrypted",
                r#"{"ciphertext": "x", "recipients": []}"#,
                Ok(Form::Encrypted),
            ),
            (
                "unsigned by parties",
                r#"{"parties": []}"#,
                Ok(Form::Unsigned),
            ),
            (
                "unsigned by dialog",
                r#"{"dialog": []}"#,
                Ok(Form::Unsigned),
            ),
            (
                "unsigned by analysis",
                r#"{"analysis": []}"#,
                Ok(Form::Unsigned),
            ),
            (
                "unsigned by attachments",
                r#"{"attachments": []}"#,
                Ok(Form::Unsigned),
            ),
            (
                "unsigned by uuid alone",
                r#"{"uuid": "x"}"#,
                Ok(Form::Unsigned),
            ),
            (
                "signed even with unsigned members beside it",
                r#"{"payload": "x", "signatures": [], "parties": []}"#,
                Ok(Form::Signed),
            ),
            (
                "both pairs",
                r#"{"payload": "x", "signatures": [], "ciphertext": "x", "recipients": []}"#,
                Err(Rejection::AmbiguousForm),
            ),
            ("empty object", "{}", Err(Rejection::NoForm)),
            (
                "unrelated members",
                r#"{"subject": "x"}"#,
                Err(Rejection::NoForm),
            ),
            (
                "half a signed pair",
                r#"{"payload": "x"}"#,
                Err(Rejection::NoForm),
            ),
            (
                "other half of a signed pair",
                r#"{"signatures": []}"#,
                Err(Rejection::NoForm),
            ),
            (
                "half an encrypted pair",
                r#"{"ciphertext": "x"}"#,
                Err(Rejection::NoForm),
            ),
            (
                "other half of an encrypted pair",
                r#"{"recipients": []}"#,
                Err(Rejection::NoForm),
            ),
            (
                "half a pair beside a uuid",
                r#"{"payload": "x", "uuid": "y"}"#,
                Ok(Form::Unsigned),
            ),
        ];
        for (what, body, expected) in cases {
            assert_eq!(form(body), expected, "{what}");
        }
    }
}
