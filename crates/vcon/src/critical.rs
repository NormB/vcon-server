//! Extensions a vCon declares it cannot be understood without.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::read::Rejection;

/// The member names that list required extensions: `critical`, and
/// `must_support`, its name before syntax version 0.4.0.
const MEMBERS: [&str; 2] = ["critical", "must_support"];

/// Refuses a vCon that requires an extension not in `supported`.
pub(crate) fn check_critical(
    object: &Map<String, Value>,
    supported: &BTreeSet<String>,
) -> Result<(), Rejection> {
    let mut required: Vec<&str> = Vec::new();
    for member in MEMBERS {
        let Some(value) = object.get(member) else {
            continue;
        };
        let Value::Array(names) = value else {
            return Err(Rejection::BadCritical);
        };
        for name in names {
            let Value::String(name) = name else {
                return Err(Rejection::BadCritical);
            };
            if !required.contains(&name.as_str()) {
                required.push(name);
            }
        }
    }
    let unsupported: Vec<String> = required
        .into_iter()
        .filter(|name| !supported.contains(*name))
        .map(str::to_owned)
        .collect();
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(Rejection::UnsupportedCritical(unsupported))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(body: &str, supported: &[&str]) -> Result<(), Rejection> {
        let Value::Object(map) = serde_json::from_str(body).expect("test JSON") else {
            panic!("test body must be an object")
        };
        let supported = supported.iter().map(|s| s.to_string()).collect();
        check_critical(&map, &supported)
    }

    fn unsupported(names: &[&str]) -> Result<(), Rejection> {
        Err(Rejection::UnsupportedCritical(
            names.iter().map(|s| s.to_string()).collect(),
        ))
    }

    #[test]
    fn passes_when_nothing_unsupported_is_required() {
        let cases: &[(&str, &str, &[&str])] = &[
            ("neither member", "{}", &[]),
            ("empty critical", r#"{"critical": []}"#, &[]),
            ("empty must_support", r#"{"must_support": []}"#, &[]),
            (
                "supported critical",
                r#"{"critical": ["cc"]}"#,
                &["cc", "wtf"],
            ),
            (
                "supported must_support",
                r#"{"must_support": ["wtf"]}"#,
                &["cc", "wtf"],
            ),
            ("other members ignored", r#"{"extensions": ["zzz"]}"#, &[]),
        ];
        for (what, body, supported) in cases {
            assert_eq!(check(body, supported), Ok(()), "{what}");
        }
    }

    #[test]
    fn names_every_unsupported_extension() {
        let cases: &[(&str, &str, &[&str], &[&str])] = &[
            ("one in critical", r#"{"critical": ["x"]}"#, &[], &["x"]),
            (
                "one in must_support",
                r#"{"must_support": ["x"]}"#,
                &[],
                &["x"],
            ),
            (
                "only the unsupported",
                r#"{"critical": ["cc", "x"]}"#,
                &["cc"],
                &["x"],
            ),
            (
                "both members, in order, once each",
                r#"{"critical": ["b", "a", "b"], "must_support": ["c", "a"]}"#,
                &[],
                &["b", "a", "c"],
            ),
            (
                "names are case-sensitive",
                r#"{"critical": ["CC"]}"#,
                &["cc"],
                &["CC"],
            ),
        ];
        for (what, body, supported, names) in cases {
            assert_eq!(check(body, supported), unsupported(names), "{what}");
        }
    }

    #[test]
    fn refuses_a_value_that_is_not_an_array_of_strings() {
        let cases = [
            r#"{"critical": "cc"}"#,
            r#"{"critical": null}"#,
            r#"{"critical": {}}"#,
            r#"{"critical": ["cc", 1]}"#,
            r#"{"must_support": [null]}"#,
            r#"{"critical": [], "must_support": "x"}"#,
            r#"{"critical": ["x"], "must_support": 3}"#,
        ];
        for body in cases {
            assert_eq!(check(body, &[]), Err(Rejection::BadCritical), "{body}");
        }
    }
}
