//! `.bestpractices.json` in the repository root proposes answers to the
//! OpenSSF Best Practices criteria. The site skips the whole file when it is
//! not a JSON object or is over 100,000 bytes, and skips any status it does
//! not recognise, without saying so; these tests keep it readable.

use std::path::Path;

use serde_json::{Map, Value};

/// The largest file the site reads.
const MAX_FILE: usize = 100_000;
/// The longest justification the site stores.
const MAX_TEXT: usize = 8192;

fn answers() -> Map<String, Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.bestpractices.json");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(bytes.len() <= MAX_FILE, "{} bytes", bytes.len());
    match serde_json::from_slice(&bytes).expect("valid JSON") {
        Value::Object(map) => map,
        other => panic!("not a JSON object: {other}"),
    }
}

#[test]
fn every_status_is_one_the_site_accepts() {
    let map = answers();
    let statuses: Vec<_> = map.iter().filter(|(k, _)| k.ends_with("_status")).collect();
    assert!(!statuses.is_empty(), "no answers");
    for (key, value) in statuses {
        let status = value
            .as_str()
            .unwrap_or_else(|| panic!("{key} is not text"));
        assert!(
            ["Met", "Unmet", "N/A"].contains(&status),
            "{key} is {status:?}"
        );
    }
}

#[test]
fn every_status_has_its_justification_and_no_key_is_stray() {
    let map = answers();
    for (key, value) in &map {
        if let Some(criterion) = key.strip_suffix("_status") {
            let justification = map
                .get(&format!("{criterion}_justification"))
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{criterion} has no justification"));
            assert!(!justification.trim().is_empty(), "{criterion}: empty");
            assert!(justification.len() <= MAX_TEXT, "{criterion}: too long");
        } else if let Some(criterion) = key.strip_suffix("_justification") {
            assert!(value.is_string(), "{key} is not text");
            assert!(
                map.contains_key(&format!("{criterion}_status")),
                "{key} has no status"
            );
        } else {
            panic!("{key} is neither a status nor a justification");
        }
    }
}
