//! The first step of reading a vCon: the body must be one JSON object.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use crate::Rejection;

/// Parses `body` as exactly one JSON object in which no object, at any
/// depth, repeats a member name.
pub(crate) fn parse_object(body: &[u8]) -> Result<Map<String, Value>, Rejection> {
    match serde_json::from_slice::<Strict>(body) {
        Ok(Strict(Value::Object(map))) => Ok(map),
        Ok(_) => Err(Rejection::NotJson),
        // Syntax and end-of-input errors come from the parser; the only data
        // error is the duplicate name raised below.
        Err(e) if e.is_data() => Err(Rejection::DuplicateKey),
        Err(_) => Err(Rejection::NotJson),
    }
}

/// A JSON value read like `serde_json::Value`, except that a repeated
/// member name is an error instead of the last value silently winning.
struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor).map(Strict)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Number::from_f64(v).map_or(Value::Null, Value::Number))
    }

    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(Strict(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            if map.contains_key(&key) {
                return Err(de::Error::custom("duplicate member name"));
            }
            let Strict(value) = access.next_value()?;
            map.insert(key, value);
        }
        Ok(Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_one_object() {
        let map = parse_object(br#"{"uuid": "x", "n": 1}"#).expect("an object");
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn accepts_whitespace_around_the_object() {
        assert!(parse_object(b" \n\t{}\r\n ").is_ok());
    }

    #[test]
    fn refuses_everything_that_is_not_one_object() {
        let cases: &[(&str, &[u8])] = &[
            ("empty", b""),
            ("only whitespace", b"  \n"),
            ("array", b"[{}]"),
            ("number", b"42"),
            ("string", br#""text""#),
            ("null", b"null"),
            ("invalid UTF-8", b"{\"a\": \"\xff\"}"),
            ("truncated", b"{\"a\": 1"),
            ("two objects", b"{} {}"),
            ("trailing text", b"{}x"),
            ("number out of range", b"{\"a\": 1e400}"),
        ];
        for (what, body) in cases {
            assert_eq!(parse_object(body), Err(Rejection::NotJson), "{what}");
        }
        let deep = format!("{{\"a\": {}{}}}", "[".repeat(200), "]".repeat(200));
        assert_eq!(
            parse_object(deep.as_bytes()),
            Err(Rejection::NotJson),
            "too deep"
        );
    }

    #[test]
    fn refuses_a_repeated_member_name_at_any_depth() {
        let cases: &[(&str, &[u8])] = &[
            ("top level", br#"{"uuid": "a", "uuid": "b"}"#),
            ("same value twice", br#"{"uuid": "a", "uuid": "a"}"#),
            ("nested object", br#"{"parties": {"tel": "1", "tel": "2"}}"#),
            (
                "object inside an array",
                br#"{"dialog": [{}, {"type": 1, "type": 2}]}"#,
            ),
            (
                "spelled with an escape",
                br#"{"uuid": "a", "\u0075uid": "b"}"#,
            ),
        ];
        for (what, body) in cases {
            assert_eq!(parse_object(body), Err(Rejection::DuplicateKey), "{what}");
        }
    }

    #[test]
    fn keeps_every_value_as_serde_json_reads_it() {
        let body = br#"{
            "plain": "text",
            "escaped": "tab\there \u00e9 \"quoted\"",
            "int": 7, "negative": -42, "big": 18446744073709551615, "float": 1.5e3,
            "yes": true, "no": false, "nothing": null,
            "list": [1, "two", [3], {"four": 4}],
            "nested": {"deeper": {"deepest": []}}
        }"#;
        let expected: Value = serde_json::from_slice(body).expect("valid JSON");
        assert_eq!(
            Value::Object(parse_object(body).expect("an object")),
            expected
        );
    }

    #[test]
    fn allows_the_same_name_in_different_objects() {
        let body = br#"{"a": {"uuid": 1}, "b": [{"uuid": 2}, {"uuid": 3}], "uuid": 4}"#;
        assert!(parse_object(body).is_ok());
    }
}
