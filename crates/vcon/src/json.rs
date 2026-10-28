//! The first step of reading a vCon: the body must be one JSON object.

use std::cell::Cell;
use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

use crate::read::{MAX_JSON_DEPTH, Rejection};

/// Parses `body` as exactly one JSON object in which no object, at any
/// depth, repeats a member name, and no more than [`MAX_JSON_DEPTH`] arrays
/// and objects are open at once.
pub(crate) fn parse_object(body: &[u8]) -> Result<Map<String, Value>, Rejection> {
    let too_deep = Cell::new(false);
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let parsed = Strict {
        open: 0,
        too_deep: &too_deep,
    }
    .deserialize(&mut deserializer)
    .and_then(|value| deserializer.end().map(|()| value));
    match parsed {
        _ if too_deep.get() => Err(Rejection::TooDeep),
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(Rejection::NotJson),
        // Syntax and end-of-input errors come from the parser; the only data
        // error left is the duplicate name raised below.
        Err(e) if e.is_data() => Err(Rejection::DuplicateKey),
        Err(_) => Err(Rejection::NotJson),
    }
}

/// Reads one JSON value like `serde_json::Value`, except that a repeated
/// member name is an error instead of the last value silently winning, and
/// an array or object past [`MAX_JSON_DEPTH`] is an error that sets
/// `too_deep`. `open` is how many arrays and objects enclose the value.
#[derive(Clone, Copy)]
struct Strict<'a> {
    open: usize,
    too_deep: &'a Cell<bool>,
}

impl Strict<'_> {
    /// The reader for a value inside the array or object being entered, or
    /// an error if entering it would pass the limit.
    fn enter<E: de::Error>(self) -> Result<Self, E> {
        if self.open >= MAX_JSON_DEPTH {
            self.too_deep.set(true);
            return Err(E::custom("nested too deep"));
        }
        Ok(Strict {
            open: self.open + 1,
            ..self
        })
    }
}

impl<'de> DeserializeSeed<'de> for Strict<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Strict<'_> {
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
        let inside = self.enter()?;
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(inside)? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let inside = self.enter()?;
        let mut map = Map::new();
        while let Some(key) = access.next_key::<String>()? {
            if map.contains_key(&key) {
                return Err(de::Error::custom("duplicate member name"));
            }
            let value = access.next_value_seed(inside)?;
            map.insert(key, value);
        }
        Ok(Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_whitespace_around_the_object() {
        assert_eq!(parse_object(b" \n\t{}\r\n "), Ok(Map::new()));
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
    }

    /// `{"a": [[...]]}` with `open` arrays and objects open at the deepest
    /// point, counting the outer object.
    fn nested(open: usize) -> String {
        let inner = open - 1;
        format!("{{\"a\": {}0{}}}", "[".repeat(inner), "]".repeat(inner))
    }

    #[test]
    fn reads_json_nested_up_to_the_limit() {
        let map = parse_object(nested(MAX_JSON_DEPTH).as_bytes()).expect("at the limit");
        assert!(map.contains_key("a"));
        let objects = format!(
            "{}0{}",
            "{\"a\": ".repeat(MAX_JSON_DEPTH),
            "}".repeat(MAX_JSON_DEPTH)
        );
        assert!(parse_object(objects.as_bytes()).is_ok(), "objects only");
    }

    #[test]
    fn refuses_json_nested_past_the_limit() {
        let cases = [
            ("one array past", nested(MAX_JSON_DEPTH + 1)),
            (
                "one object past",
                format!(
                    "{}0{}",
                    "{\"a\": ".repeat(MAX_JSON_DEPTH + 1),
                    "}".repeat(MAX_JSON_DEPTH + 1)
                ),
            ),
            ("past the parser's own bound", nested(200)),
            (
                "past the limit, then broken",
                format!("{{\"a\": {}", "[".repeat(MAX_JSON_DEPTH)),
            ),
            (
                "past the limit in a later member",
                format!("{{\"a\": 1, \"b\": {}0{}}}", "[".repeat(80), "]".repeat(80)),
            ),
        ];
        for (what, body) in cases {
            assert_eq!(
                parse_object(body.as_bytes()),
                Err(Rejection::TooDeep),
                "{what}"
            );
        }
        let shallow_then_broken = format!("{{\"a\": {}", "[".repeat(MAX_JSON_DEPTH - 1));
        assert_eq!(
            parse_object(shallow_then_broken.as_bytes()),
            Err(Rejection::NotJson),
            "broken within the limit"
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
