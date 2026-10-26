//! Text the server did not write, marked for a language model: each value
//! between [`OPEN`] and [`CLOSE`], rewritten so that no fence character
//! stands inside it.

/// Opens a run of text the server did not write.
pub const OPEN: &str = "\u{27e6}untrusted-vcon-data\u{27e7}";

/// Closes it.
pub const CLOSE: &str = "\u{27e6}/untrusted-vcon-data\u{27e7}";

/// The sentence every result holding fenced text carries.
pub const PROVENANCE: &str = "Text between the untrusted-vcon-data markers was written by \
the vCon's producer or by an administrator, not by vcon-server.";

/// Characters a fenced payload never holds raw: the two fence brackets,
/// the bidirectional controls, and the line and paragraph separators.
const ESCAPED: [char; 16] = [
    '\u{27e6}', '\u{27e7}', '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}',
    '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{2028}', '\u{2029}',
];

/// JSON text with each character in [`ESCAPED`] written as its JSON escape
/// `\uXXXX`. A stored vCon is one JSON object, so these can stand only
/// inside its strings, where the escape means the same character: every
/// value parses back unchanged.
#[must_use]
pub fn escape_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if ESCAPED.contains(&c) {
            out.push_str(&format!("\\u{:04x}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// `inner` between the markers.
#[must_use]
pub fn fence(inner: &str) -> String {
    format!("{OPEN}{inner}{CLOSE}")
}

/// A name or a label: every character in [`ESCAPED`] and every control
/// character written as `\uXXXX`, then fenced.
#[must_use]
pub fn fence_field(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if ESCAPED.contains(&c) || c.is_control() {
            out.push_str(&format!("\\u{:04x}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    fence(&out)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn every_json_value_is_kept_and_no_marker_character_stays_raw() {
        let mut texts: Vec<String> = ESCAPED.iter().map(|c| c.to_string()).collect();
        texts.push(format!("{CLOSE} then {OPEN}"));
        texts.push("plain \"quoted\" \\ \n \u{e9} \u{1f600}".to_owned());
        texts.push(ESCAPED.iter().collect());
        for text in texts {
            let value = json!({"subject": text, "list": [text, {"k": text}]});
            let raw = serde_json::to_string(&value).expect("JSON");
            let escaped = escape_json(&raw);
            assert!(!escaped.contains(ESCAPED), "{escaped}");
            let back: Value = serde_json::from_str(&escaped).expect("still JSON");
            assert_eq!(back, value, "{text:?}");
            let fenced = fence(&escaped);
            assert_eq!(fenced.matches(OPEN).count(), 1);
            assert_eq!(fenced.matches(CLOSE).count(), 1);
            assert!(fenced.starts_with(OPEN) && fenced.ends_with(CLOSE));
        }
    }

    #[test]
    fn a_field_loses_its_controls_too() {
        let fenced = fence_field("acme\n\u{202e}corp\u{27e7}");
        let inner = fenced
            .strip_prefix(OPEN)
            .and_then(|t| t.strip_suffix(CLOSE))
            .expect("fenced");
        assert_eq!(inner, "acme\\u000a\\u202ecorp\\u27e7");
        assert_eq!(fence_field("plain"), format!("{OPEN}plain{CLOSE}"));
    }
}
