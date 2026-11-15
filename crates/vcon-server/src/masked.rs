//! The masked view of a vCon (`GET /vcons/{uuid}?view=masked`): what it
//! holds, without any name, subject, full number, address or content, so a
//! client can show a vCon before anyone reveals a part of it.
//!
//! THE RULE is the console's: `summary` in console/format.js, which the
//! browser applies to a vCon it has read whole. This module is its port, and
//! tests/data/masked-cases.json holds the cases both are tested against
//! (here and in console-tests/tests/format.spec.js), so the two give the
//! same answer. A change to one is a change to both and to the cases.
//!
//! The parts a reveal shows (`section`, `links`) are ported here too, for
//! the terminal UI, which shows a reveal as the console does.

use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::{Engine, alphabet};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use time::{Date, Month, PrimitiveDateTime, Time, UtcOffset};

use crate::query::{self, BadQuery};
use crate::reveal::Part;

/// The query parameter that asks a read for a view of the vCon.
pub const PARAMETER: &str = "view";

/// The one view there is: the vCon masked.
pub const MASKED: &str = "masked";

/// Whether a read's query asks for the masked view. A query that does not
/// name the parameter is the read it always was.
///
/// # Errors
///
/// [`BadQuery`] if the query names the parameter and it is not exactly
/// `view=masked`, alone and once.
pub fn from_query(query: Option<&str>) -> Result<bool, BadQuery> {
    if !query::names(query, PARAMETER) {
        return Ok(false);
    }
    let read = query::pairs(query, &[PARAMETER])?;
    match read.get(PARAMETER).map(String::as_str) {
        Some(MASKED) => Ok(true),
        _ => Err(BadQuery),
    }
}

/// The form of each kind of body, and what the subject of an unread one is.
const UNSIGNED: &str = "unsigned";
const SIGNED: &str = "signed (JWS)";
const ENCRYPTED: &str = "encrypted (JWE)";
const NOT_JSON: &str = "not JSON";
const NOT_OBJECT: &str = "not a JSON object";
const NOT_READABLE: &str = "not readable";

/// The dialog types the vCon format names; any other is not shown.
const DIALOG_TYPES: [&str; 4] = ["recording", "text", "transfer", "incomplete"];

/// The fewest digits a number needs before its last four may be shown.
const MASKABLE_DIGITS: usize = 8;

/// The encodings under which a body is media, not text to read.
const MEDIA_ENCODINGS: [&str; 2] = ["base64url", "base64"];

/// The parts of a vCon that may hold a link, and what one item is called.
const LINKED: [(&str, &str); 3] = [
    ("dialog", "dialog"),
    ("attachments", "attachment"),
    ("analysis", "analysis"),
];

/// Base64url as a browser's `atob` reads it: no padding needed, and bits
/// after the last whole byte ignored.
const FORGIVING: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

/// A value's items when it is an array, or none.
fn items(value: Option<&Value>) -> &[Value] {
    value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// A field of an object when it is text.
fn text_field<'a>(object: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    object.get(name).and_then(Value::as_str)
}

/// Base64url JSON as an object, or none when it is anything else. A
/// leading byte order mark is dropped, as a browser's text decoder drops it.
fn decoded(value: Option<&Value>) -> Option<Map<String, Value>> {
    let text = value?.as_str()?;
    if text.is_empty()
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let bytes = FORGIVING.decode(text).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    match serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(text)).ok()? {
        Value::Object(object) => Some(object),
        _ => None,
    }
}

/// The vCon object a body holds, for reading its parts: the body itself,
/// or a signed vCon's payload. None for an encrypted vCon and for anything
/// that is not a JSON object.
fn readable(text: &str) -> Option<Map<String, Value>> {
    let Ok(Value::Object(vcon)) = serde_json::from_str::<Value>(text) else {
        return None;
    };
    if vcon.contains_key("ciphertext") {
        return None;
    }
    if vcon.contains_key("payload") && vcon.contains_key("signatures") {
        return decoded(vcon.get("payload"));
    }
    Some(vcon)
}

/// A summary of nothing, in the given form.
fn empty(form: &str, subject: &str) -> Summary {
    Summary {
        form: form.to_owned(),
        created: String::new(),
        subject: subject.to_owned(),
        parties: Vec::new(),
        dialogs: 0,
        dialog: Vec::new(),
        seconds: 0.0,
        attachments: 0,
        analyses: 0,
        links: 0,
        signature: None,
    }
}

/// The counts and masked parties of an unsigned vCon object.
fn counted(vcon: &Map<String, Value>, form: &str) -> Summary {
    let dialog = items(vcon.get("dialog"));
    let seconds = dialog
        .iter()
        .filter_map(|d| d.get("duration").and_then(Value::as_f64))
        .filter(|duration| *duration > 0.0)
        .fold(0.0, |sum, duration| sum + duration);
    let subject = match text_field(vcon, "subject") {
        Some(subject) if !subject.is_empty() => "present, hidden",
        _ => "none",
    };
    Summary {
        form: form.to_owned(),
        created: vcon.get("created_at").map(instant).unwrap_or_default(),
        subject: subject.to_owned(),
        parties: items(vcon.get("parties"))
            .iter()
            .enumerate()
            .map(|(index, party)| party_line(party, index))
            .collect(),
        dialogs: dialog.len(),
        dialog: dialog
            .iter()
            .enumerate()
            .map(|(index, d)| dialog_line(d, index))
            .collect(),
        seconds,
        attachments: items(vcon.get("attachments")).len(),
        analyses: items(vcon.get("analysis")).len(),
        links: links_of(vcon).len(),
        signature: None,
    }
}

/// Every link in a vCon object, with where it is.
fn links_of(vcon: &Map<String, Value>) -> Vec<Link> {
    let mut found = Vec::new();
    for (part, noun) in LINKED {
        for (index, item) in items(vcon.get(part)).iter().enumerate() {
            if let Some(url) = item.as_object().and_then(|item| text_field(item, "url"))
                && !url.is_empty()
            {
                found.push(Link {
                    place: format!("{noun} {}", index.saturating_add(1)),
                    url: url.to_owned(),
                });
            }
        }
    }
    found
}

/// A vCon's form and counts, its parties masked: the answer of the masked
/// view, with the fields and the words of the console's summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// `unsigned`, `signed (JWS)`, `encrypted (JWE)`, `not JSON` or `not a
    /// JSON object`.
    pub form: String,
    /// When it was made, in UTC (see [`instant`]); empty when not a time.
    pub created: String,
    /// `present, hidden`, `none`, or `not readable`.
    pub subject: String,
    /// One line for each party, masked (see [`party_line`]).
    pub parties: Vec<String>,
    /// How many dialogs it holds.
    pub dialogs: usize,
    /// One line for each dialog, with no content (see [`dialog_line`]).
    pub dialog: Vec<String>,
    /// The sum of the dialogs' positive lengths, in seconds.
    #[serde(serialize_with = "as_js_number")]
    pub seconds: f64,
    /// How many attachments it holds.
    pub attachments: usize,
    /// How many analyses it holds.
    pub analyses: usize,
    /// How many links to outside the server it holds.
    pub links: usize,
    /// A signed vCon's first signature header, else none.
    pub signature: Option<Signature>,
}

/// The fields of a signature's protected header the view names; each is
/// empty when the header does not hold it as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// The signing algorithm, such as `ES256`.
    pub alg: String,
    /// The id of the signing key.
    pub kid: String,
    /// The thumbprint of the signing certificate.
    pub x5t: String,
}

/// A number as JSON writes it in JavaScript: a whole number without a
/// fraction, so `30` and not `30.0`.
fn as_js_number<S: Serializer>(value: &f64, to: S) -> Result<S::Ok, S::Error> {
    /// Below this every whole number is exact in a JavaScript number.
    const EXACT: f64 = 9_007_199_254_740_992.0;
    if value.fract() == 0.0 && value.abs() < EXACT {
        // The value is whole and within the exact range, so the
        // conversion keeps it.
        #[expect(clippy::cast_possible_truncation, reason = "whole and below 2^53")]
        to.serialize_i64(*value as i64)
    } else {
        to.serialize_f64(*value)
    }
}

/// A body as the text a browser reads from it: UTF-8, any byte that is not
/// replaced, and a leading byte order mark dropped.
#[must_use]
pub fn text_of(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
}

/// What a vCon body holds, without any name, subject, full number or
/// address: its form, counts and masked parties. A signed vCon's payload is
/// read; an encrypted one is not decoded.
#[must_use]
pub fn summary(text: &str) -> Summary {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return empty(NOT_JSON, NOT_READABLE);
    };
    let Value::Object(vcon) = value else {
        return empty(NOT_OBJECT, NOT_READABLE);
    };
    if vcon.contains_key("ciphertext") {
        return empty(ENCRYPTED, NOT_READABLE);
    }
    if vcon.contains_key("payload") && vcon.contains_key("signatures") {
        let header = items(vcon.get("signatures"))
            .first()
            .and_then(Value::as_object)
            .and_then(|first| decoded(first.get("protected")));
        let field = |name: &str| {
            header
                .as_ref()
                .and_then(|header| text_field(header, name))
                .unwrap_or_default()
                .to_owned()
        };
        let mut made = decoded(vcon.get("payload")).map_or_else(
            || empty(SIGNED, NOT_READABLE),
            |payload| counted(&payload, SIGNED),
        );
        made.signature = Some(Signature {
            alg: field("alg"),
            kid: field("kid"),
            x5t: field("x5t"),
        });
        return made;
    }
    counted(&vcon, UNSIGNED)
}

/// The numbers of an RFC 3339 date-time, `YYYY-MM-DDTHH:MM:SS`, any
/// fraction, then `Z` or `+HH:MM`/`-HH:MM` (either letter in either case):
/// year, month, day, hour, minute, second and the offset in minutes. None
/// for text of any other shape.
fn date_time_numbers(text: &str) -> Option<([u16; 6], i32)> {
    let bytes = text.as_bytes();
    let number = |from: usize, to: usize| -> Option<u16> {
        let digits = bytes.get(from..to)?;
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| std::str::from_utf8(digits).ok()?.parse().ok())?
    };
    let at = |index: usize, wanted: &[u8]| bytes.get(index).is_some_and(|b| wanted.contains(b));
    if !(at(4, b"-") && at(7, b"-") && at(10, b"Tt") && at(13, b":") && at(16, b":")) {
        return None;
    }
    let fields = [
        number(0, 4)?,
        number(5, 7)?,
        number(8, 10)?,
        number(11, 13)?,
        number(14, 16)?,
        number(17, 19)?,
    ];
    let mut rest = 19;
    if at(rest, b".") {
        let digits = bytes
            .get(rest.saturating_add(1)..)?
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        rest = rest.saturating_add(1).saturating_add(digits);
    }
    let zone = bytes.get(rest..)?;
    let offset = match zone {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let hours = number(rest.saturating_add(1), rest.saturating_add(3))?;
            let minutes = number(rest.saturating_add(4), rest.saturating_add(6))?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let minutes = i32::from(hours) * 60 + i32::from(minutes);
            if *sign == b'-' { -minutes } else { minutes }
        }
        _ => return None,
    };
    Some((fields, offset))
}

/// A vCon's time as `YYYY-MM-DD HH:MM:SS UTC+00:00`, or empty when it is
/// not an RFC 3339 date-time of a day that exists, in the years 1 to 9999
/// once in UTC. Text that is not a time is never shown.
#[must_use]
pub fn instant(value: &Value) -> String {
    let read = || {
        let (fields, offset) = date_time_numbers(value.as_str()?)?;
        let [year, month, day, hour, minute, second] = fields;
        let date = Date::from_calendar_date(
            i32::from(year),
            Month::try_from(u8::try_from(month).ok()?).ok()?,
            u8::try_from(day).ok()?,
        )
        .ok()?;
        let time = Time::from_hms(
            u8::try_from(hour).ok()?,
            u8::try_from(minute).ok()?,
            u8::try_from(second).ok()?,
        )
        .ok()?;
        let offset = UtcOffset::from_whole_seconds(offset.checked_mul(60)?).ok()?;
        let at = PrimitiveDateTime::new(date, time)
            .assume_offset(offset)
            .checked_to_offset(UtcOffset::UTC)?;
        (1..=9999).contains(&at.year()).then(|| {
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC+00:00",
                at.year(),
                u8::from(at.month()),
                at.day(),
                at.hour(),
                at.minute(),
                at.second()
            )
        })
    };
    read().unwrap_or_default()
}

/// A telephone number as its last four digits, or hidden whole.
#[must_use]
pub fn mask_tel(value: &Value) -> String {
    let digits: Vec<char> = value
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    match digits.len().checked_sub(4) {
        Some(from) if digits.len() >= MASKABLE_DIGITS => {
            let last: String = digits.get(from..).unwrap_or_default().iter().collect();
            format!("number ending {last}")
        }
        _ => "number hidden".to_owned(),
    }
}

/// Whether JavaScript's `trim` removes `c`: its white space and line
/// terminators, the byte order mark among them.
fn trimmed_by_javascript(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

/// An email address as its domain, or hidden whole.
#[must_use]
pub fn mask_mail(value: &Value) -> String {
    let Some(address) = value.as_str() else {
        return "address hidden".to_owned();
    };
    match address.rfind('@') {
        Some(at) if at > 0 => {
            let domain = address
                .get(at.saturating_add(1)..)
                .unwrap_or_default()
                .trim_matches(trimmed_by_javascript)
                .to_lowercase();
            if domain.is_empty() {
                "address hidden".to_owned()
            } else {
                format!("address at {domain}")
            }
        }
        _ => "address hidden".to_owned(),
    }
}

/// Party `index` (from 0) as a line that names what it holds and shows none
/// of it.
#[must_use]
pub fn party_line(party: &Value, index: usize) -> String {
    let label = format!("Party {}", index.saturating_add(1));
    let Some(party) = party.as_object() else {
        return format!("{label}: not an object");
    };
    let mut parts = Vec::new();
    if let Some(tel) = party.get("tel") {
        parts.push(mask_tel(tel));
    }
    if let Some(mailto) = party.get("mailto") {
        parts.push(mask_mail(mailto));
    }
    if party.contains_key("name") {
        parts.push("name hidden".to_owned());
    }
    if parts.is_empty() {
        format!("{label}: no number, address or name")
    } else {
        format!("{label}: {}", parts.join("; "))
    }
}

/// A value as a party's place in the parties list, if it is one: a whole
/// number, zero or more.
fn place(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|number| number.fract() == 0.0 && *number >= 0.0)
}

/// A dialog's parties as their numbers, counted from one.
fn parties_of(parties: Option<&Value>) -> String {
    if let Some(one) = parties.and_then(place) {
        return format!("party {}", js_number(one + 1.0));
    }
    let places: Option<Vec<f64>> = parties
        .and_then(Value::as_array)
        .filter(|list| !list.is_empty())
        .and_then(|list| list.iter().map(place).collect());
    match places {
        Some(places) => {
            let numbers: Vec<String> = places.iter().map(|p| js_number(p + 1.0)).collect();
            format!("parties {}", numbers.join(", "))
        }
        None => "no parties".to_owned(),
    }
}

/// Dialog `index` (from 0) as a line with its type, start, length and
/// parties, and whether it has content, never the content or a link.
#[must_use]
pub fn dialog_line(dialog: &Value, index: usize) -> String {
    let label = format!("Dialog {}", index.saturating_add(1));
    let Some(dialog) = dialog.as_object() else {
        return format!("{label}: not an object");
    };
    let kind = text_field(dialog, "type")
        .filter(|kind| DIALOG_TYPES.contains(kind))
        .unwrap_or("other type");
    let start = dialog.get("start").map(instant).unwrap_or_default();
    let start = if start.is_empty() { "no start" } else { &start };
    let length = dialog
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|duration| *duration > 0.0)
        .map_or_else(|| "no length".to_owned(), |d| format!("{} s", js_number(d)));
    let present = |name: &str| text_field(dialog, name).is_some_and(|text| !text.is_empty());
    let content = if present("body") {
        "content hidden"
    } else if present("url") {
        "content at a link outside the server"
    } else {
        "no content"
    };
    format!(
        "{label}: {kind}, {start}, {length}, {}; {content}",
        parties_of(dialog.get("parties"))
    )
}

/// A number as JavaScript writes it in text: `30`, `12.5`, `1e+21`.
#[must_use]
pub fn js_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let size = value.abs();
    if size >= 1e21 || size < 1e-6 {
        let written = format!("{value:e}");
        return match written.split_once('e') {
            Some((digits, power)) if !power.starts_with('-') => format!("{digits}e+{power}"),
            _ => written,
        };
    }
    format!("{value}")
}

/// A link of a vCon: where it is (`dialog 1`, `attachment 2`, `analysis
/// 3`) and the address, which is never loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Where the link is, as `dialog 1`.
    pub place: String,
    /// Where it points.
    pub url: String,
}

/// Every link in a vCon body, in the order of the console; none when the
/// body cannot be read in parts.
#[must_use]
pub fn links(text: &str) -> Vec<Link> {
    readable(text)
        .map(|vcon| links_of(&vcon))
        .unwrap_or_default()
}

/// The most characters of a vCon shown at once: 2 MiB, as in the console.
pub const DISPLAY_LIMIT: usize = 2 * 1024 * 1024;

/// `text` cut at [`DISPLAY_LIMIT`] characters, and how many were left out.
#[must_use]
pub fn shown(text: &str) -> (&str, usize) {
    match text.char_indices().nth(DISPLAY_LIMIT) {
        Some((cut, _)) => {
            let (kept, left) = text.split_at(cut);
            (kept, left.chars().count())
        }
        None => (text, 0),
    }
}

/// Items as indented JSON, with inline media named by its size, not shown;
/// `none` for no items.
fn listed(value: Option<&Value>) -> String {
    let made = items(value);
    if made.is_empty() {
        return "none".to_owned();
    }
    let readable: Vec<Value> = made
        .iter()
        .map(|item| {
            let mut item = item.clone();
            if let Some(object) = item.as_object_mut()
                && let Some(encoding) = text_field(object, "encoding")
                    .filter(|encoding| MEDIA_ENCODINGS.contains(encoding))
                    .map(str::to_owned)
                && let Some(length) =
                    text_field(object, "body").map(|body| body.encode_utf16().count())
            {
                object.insert(
                    "body".to_owned(),
                    Value::String(format!("[{length} characters of {encoding}, not shown]")),
                );
            }
            item
        })
        .collect();
    let text = serde_json::to_string_pretty(&readable).unwrap_or_default();
    shown(&text).0.to_owned()
}

/// One part of a vCon body as text, as the console shows a reveal of it:
/// the parties, the dialog or the analysis as indented JSON with inline
/// media named by its size, or the subject. None for any other part and for
/// a body that cannot be read in parts.
#[must_use]
pub fn section(text: &str, part: Part) -> Option<String> {
    let vcon = readable(text)?;
    match part {
        Part::Parties => Some(listed(vcon.get("parties"))),
        Part::Dialog => Some(listed(vcon.get("dialog"))),
        Part::Analysis => Some(listed(vcon.get("analysis"))),
        Part::Subject => Some(
            text_field(&vcon, "subject")
                .filter(|subject| !subject.is_empty())
                .unwrap_or("none")
                .to_owned(),
        ),
        Part::Links | Part::All | Part::Audio => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The cases format.js is tested against too.
    const CASES: &str = include_str!("../tests/data/masked-cases.json");

    fn cases() -> Vec<(String, String, Value)> {
        let cases: Vec<Value> = serde_json::from_str(CASES).expect("the cases are JSON");
        cases
            .into_iter()
            .map(|case| {
                (
                    case["name"].as_str().expect("a name").to_owned(),
                    case["body"].as_str().expect("a body").to_owned(),
                    case["summary"].clone(),
                )
            })
            .collect()
    }

    fn masked(vcon: &Value) -> Summary {
        summary(&vcon.to_string())
    }

    // Success.

    #[test]
    fn each_case_is_summarized_as_format_js_summarizes_it() {
        let cases = cases();
        assert!(cases.len() > 20, "there are cases to compare");
        for (name, body, wanted) in cases {
            let made = serde_json::to_value(summary(&body)).expect("a summary is JSON");
            assert_eq!(made, wanted, "the case {name}");
        }
    }

    #[test]
    fn view_masked_alone_asks_for_the_masked_view() {
        assert_eq!(from_query(Some("view=masked")), Ok(true), "the view");
    }

    #[test]
    fn a_read_without_the_parameter_is_not_the_masked_view() {
        for query in [None, Some(""), Some("reveal=all"), Some("viewed=masked")] {
            assert_eq!(from_query(query), Ok(false), "{query:?}");
        }
    }

    #[test]
    fn a_number_shows_its_last_four_digits() {
        assert_eq!(
            mask_tel(&json!("+1 212 555 0142")),
            "number ending 0142",
            "masked"
        );
    }

    #[test]
    fn an_address_shows_its_domain_in_lower_case() {
        assert_eq!(
            mask_mail(&json!("ada@Example.COM")),
            "address at example.com",
            "masked"
        );
    }

    #[test]
    fn a_party_line_names_number_address_and_name_in_that_order() {
        let party =
            json!({"name": "Ada Example", "mailto": "ada@example.com", "tel": "+12125550142"});
        assert_eq!(
            party_line(&party, 2),
            "Party 3: number ending 0142; address at example.com; name hidden",
            "the line"
        );
    }

    #[test]
    fn a_dialog_line_shows_type_start_length_and_parties_and_no_content() {
        let dialog = json!({"type": "text", "start": "2026-11-10T09:15:05Z", "duration": 30,
            "parties": [0, 1], "body": "Ada said hello"});
        let line = dialog_line(&dialog, 0);
        assert_eq!(
            line,
            "Dialog 1: text, 2026-11-10 09:15:05 UTC+00:00, 30 s, parties 1, 2; content hidden",
            "the line"
        );
        assert!(!line.contains("hello"), "no content");
    }

    #[test]
    fn a_time_with_an_offset_is_shown_in_utc() {
        assert_eq!(
            instant(&json!("2026-11-10T11:15:00+02:00")),
            "2026-11-10 09:15:00 UTC+00:00",
            "in UTC"
        );
    }

    #[test]
    fn a_whole_number_of_seconds_is_written_without_a_fraction() {
        let made =
            serde_json::to_string(&masked(&json!({"dialog": [{"duration": 30}]}))).expect("JSON");
        assert!(made.contains(r#""seconds":30,"#), "{made}");
    }

    #[test]
    fn numbers_are_written_as_javascript_writes_them() {
        for (number, text) in [
            (30.0, "30"),
            (12.5, "12.5"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (1e20, "100000000000000000000"),
            (0.1 + 0.2, "0.30000000000000004"),
        ] {
            assert_eq!(js_number(number), text, "{number}");
        }
    }

    #[test]
    fn a_signed_vcons_header_names_its_algorithm_key_and_thumbprint() {
        let cases = cases();
        let (_, body, _) = cases
            .iter()
            .find(|(name, _, _)| name == "a signed vCon")
            .expect("the signed case");
        let made = summary(body);
        assert_eq!(made.form, "signed (JWS)", "the form");
        assert_eq!(
            made.signature,
            Some(Signature {
                alg: "ES256".to_owned(),
                kid: "key-1".to_owned(),
                x5t: "x5t-example".to_owned()
            }),
            "the header"
        );
    }

    #[test]
    fn links_say_where_they_are_in_the_consoles_order() {
        let vcon = json!({"analysis": [{"url": "https://a.example.com/t"}],
            "dialog": [{"url": "https://media.example.com/a.wav"}, {"body": "x"}],
            "attachments": [{}, {"url": "https://files.example.com/x.pdf"}]});
        let found: Vec<(String, String)> = links(&vcon.to_string())
            .into_iter()
            .map(|link| (link.place, link.url))
            .collect();
        assert_eq!(
            found,
            [
                (
                    "dialog 1".to_owned(),
                    "https://media.example.com/a.wav".to_owned()
                ),
                (
                    "attachment 2".to_owned(),
                    "https://files.example.com/x.pdf".to_owned()
                ),
                (
                    "analysis 1".to_owned(),
                    "https://a.example.com/t".to_owned()
                ),
            ],
            "the links"
        );
    }

    #[test]
    fn a_section_shows_the_part_with_inline_media_named_by_its_size() {
        let vcon = json!({"dialog": [{"type": "recording", "encoding": "base64url", "body": "AAAABBBB"}],
            "subject": "Renewal", "parties": [{"name": "Ada Example"}]});
        let text = vcon.to_string();
        let dialog = section(&text, Part::Dialog).expect("a dialog section");
        assert!(
            dialog.contains("[8 characters of base64url, not shown]"),
            "{dialog}"
        );
        assert!(!dialog.contains("AAAABBBB"), "{dialog}");
        assert_eq!(
            section(&text, Part::Subject).as_deref(),
            Some("Renewal"),
            "the subject"
        );
        let parties = section(&text, Part::Parties).expect("a parties section");
        assert!(parties.contains("Ada Example"), "{parties}");
    }

    // Failure.

    #[test]
    fn a_view_other_than_masked_is_refused() {
        for query in ["view=whole", "view=MASKED", "view=masked2", "view="] {
            assert_eq!(from_query(Some(query)), Err(BadQuery), "{query}");
        }
    }

    #[test]
    fn the_view_given_twice_is_refused() {
        for query in ["view=masked&view=masked", "view=masked&view=whole"] {
            assert_eq!(from_query(Some(query)), Err(BadQuery), "{query}");
        }
    }

    /// The masked view and a reveal are two answers: one read cannot ask
    /// for both.
    #[test]
    fn the_view_beside_a_reveal_or_another_parameter_is_refused() {
        for query in [
            "view=masked&reveal=all",
            "reveal=parties&view=masked",
            "limit=3&view=masked",
        ] {
            assert_eq!(from_query(Some(query)), Err(BadQuery), "{query}");
        }
    }

    #[test]
    fn the_parameter_with_no_value_is_refused() {
        for query in ["view", "view&x=1"] {
            assert_eq!(from_query(Some(query)), Err(BadQuery), "{query}");
        }
    }

    #[test]
    fn text_that_is_not_json_is_not_read() {
        let made = summary("{\"subject\":");
        assert_eq!(
            (made.form.as_str(), made.subject.as_str()),
            ("not JSON", "not readable"),
            "not read"
        );
    }

    #[test]
    fn an_encrypted_vcon_is_not_decoded() {
        let made =
            masked(&json!({"ciphertext": "x", "subject": "Renewal", "parties": [{"name": "Ada"}]}));
        assert_eq!(made.form, "encrypted (JWE)", "the form");
        assert!(made.parties.is_empty(), "no party is read");
    }

    #[test]
    fn a_number_too_short_or_not_text_is_hidden_whole() {
        for value in [
            json!("555-0101"),
            json!(2_125_550_142_u64),
            json!(null),
            json!(""),
        ] {
            assert_eq!(mask_tel(&value), "number hidden", "{value}");
        }
    }

    #[test]
    fn an_address_without_a_local_part_or_a_domain_is_hidden_whole() {
        for value in [
            json!("@example.com"),
            json!("ada@"),
            json!("ada@  "),
            json!("ada"),
            json!(7),
        ] {
            assert_eq!(mask_mail(&value), "address hidden", "{value}");
        }
    }

    #[test]
    fn text_that_is_not_a_date_time_is_never_shown() {
        for value in [
            "Ada called about her account",
            "1",
            "Jan 2 2026",
            "2026-11-10T09:15Z",
            "2026-11-10 09:15:00Z",
            "2026-11-10T09:15:00",
            "2026-11-10T09:15:00.Z",
            "x UTC+00:00",
        ] {
            assert_eq!(instant(&json!(value)), "", "{value}");
        }
    }

    #[test]
    fn a_day_hour_second_or_offset_that_does_not_exist_is_no_time() {
        for value in [
            "2026-02-30T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:60:00Z",
            "2026-01-01T23:59:60Z",
            "2026-01-01T00:00:00+24:00",
            "2026-01-01T00:00:00+01:60",
        ] {
            assert_eq!(instant(&json!(value)), "", "{value}");
        }
    }

    #[test]
    fn a_party_or_dialog_that_is_not_an_object_is_named_and_not_read() {
        assert_eq!(
            party_line(&json!("Ada Example"), 0),
            "Party 1: not an object",
            "party"
        );
        assert_eq!(
            dialog_line(&json!(["hello"]), 1),
            "Dialog 2: not an object",
            "dialog"
        );
    }

    #[test]
    fn a_body_that_cannot_be_read_in_parts_has_no_section_and_no_links() {
        for text in ["{\"ciphertext\":\"x\"}", "[1]", "not JSON"] {
            assert_eq!(section(text, Part::Parties), None, "{text}");
            assert_eq!(links(text), [], "{text}");
        }
    }

    #[test]
    fn only_the_four_parts_have_a_section() {
        let text = json!({"subject": "Renewal"}).to_string();
        for part in [Part::Links, Part::All, Part::Audio] {
            assert_eq!(section(&text, part), None, "{part:?}");
        }
    }

    // Edge.

    #[test]
    fn an_encoded_parameter_is_the_parameter() {
        for query in ["%76iew=masked", "view=%6Dasked", "vi%65w=mask%65d"] {
            assert_eq!(from_query(Some(query)), Ok(true), "{query}");
        }
    }

    #[test]
    fn an_empty_piece_beside_the_view_names_nothing() {
        for query in ["view=masked&", "&view=masked", "&&view=masked&&"] {
            assert_eq!(from_query(Some(query)), Ok(true), "{query}");
        }
    }

    #[test]
    fn a_fraction_of_a_second_is_dropped_not_rounded() {
        assert_eq!(
            instant(&json!("2026-11-10T09:15:00.999999999Z")),
            "2026-11-10 09:15:00 UTC+00:00",
            "dropped"
        );
    }

    #[test]
    fn lower_case_t_and_z_are_read() {
        assert_eq!(
            instant(&json!("2026-11-10t09:15:00z")),
            "2026-11-10 09:15:00 UTC+00:00",
            "read"
        );
    }

    #[test]
    fn a_time_outside_the_years_1_to_9999_in_utc_is_no_time() {
        for value in [
            "9999-12-31T23:30:00-01:00",
            "0001-01-01T00:30:00+01:00",
            "0000-06-01T00:00:00Z",
        ] {
            assert_eq!(instant(&json!(value)), "", "{value}");
        }
        assert_eq!(
            instant(&json!("0001-01-01T00:00:00Z")),
            "0001-01-01 00:00:00 UTC+00:00",
            "year 1"
        );
        assert_eq!(
            instant(&json!("9999-12-31T23:59:59Z")),
            "9999-12-31 23:59:59 UTC+00:00",
            "year 9999"
        );
    }

    #[test]
    fn a_leap_day_is_a_day() {
        assert_eq!(
            instant(&json!("2024-02-29T12:00:00Z")),
            "2024-02-29 12:00:00 UTC+00:00",
            "2024"
        );
        assert_eq!(instant(&json!("2026-02-29T12:00:00Z")), "", "2026 has none");
    }

    #[test]
    fn a_place_written_with_a_fraction_of_zero_is_a_place() {
        let line = dialog_line(&json!({"type": "text", "parties": [0.0, 2.0]}), 0);
        assert!(line.contains("parties 1, 3"), "{line}");
        let line = dialog_line(&json!({"type": "text", "parties": [0, 1.5]}), 0);
        assert!(line.contains("no parties"), "{line}");
    }

    #[test]
    fn a_body_with_a_byte_order_mark_is_read_as_a_browser_reads_it() {
        let text = text_of(b"\xef\xbb\xbf{\"subject\":\"Renewal\"}");
        assert_eq!(
            summary(&text).subject,
            "present, hidden",
            "the mark is dropped"
        );
        assert_eq!(
            text_of(b"{\"a\":\"\xff\"}"),
            "{\"a\":\"\u{fffd}\"}",
            "a bad byte is replaced"
        );
    }

    #[test]
    fn a_domain_loses_the_spaces_javascript_trims() {
        for value in [
            "ada@ example.com\u{a0}",
            "ada@\u{feff}example.com",
            "ada@\texample.com\n",
        ] {
            assert_eq!(
                mask_mail(&json!(value)),
                "address at example.com",
                "{value:?}"
            );
        }
    }

    #[test]
    fn digits_are_counted_across_punctuation_and_only_ascii_digits_count() {
        assert_eq!(
            mask_tel(&json!("+1 (212) 555-0199")),
            "number ending 0199",
            "punctuation"
        );
        assert_eq!(
            mask_tel(&json!("\u{661}\u{662}\u{663}\u{664}5550199")),
            "number hidden",
            "Arabic-Indic digits"
        );
    }

    #[test]
    fn the_summary_holds_no_name_subject_number_or_address() {
        let vcon = json!({"subject": "Ada's divorce", "created_at": "Ada's birthday",
            "parties": [{"name": "Ada Example", "tel": "+12125550142", "mailto": "ada@example.com"}],
            "dialog": [{"type": "text", "start": "Ada's call", "body": "Ada said hello"}]});
        let made = serde_json::to_string(&masked(&vcon)).expect("JSON");
        for hidden in ["Ada", "divorce", "2125550142", "ada@", "hello"] {
            assert!(!made.contains(hidden), "{hidden} in {made}");
        }
    }

    #[test]
    fn a_long_text_is_cut_at_the_limit_and_says_what_was_left_out() {
        let text = "x".repeat(DISPLAY_LIMIT + 5);
        let (kept, left) = shown(&text);
        assert_eq!((kept.len(), left), (DISPLAY_LIMIT, 5), "cut");
        assert_eq!(shown("short"), ("short", 0), "kept whole");
    }

    #[test]
    fn a_summary_read_back_is_the_summary() {
        let made =
            masked(&json!({"dialog": [{"duration": 12.5}], "parties": [{"tel": "+12125550142"}]}));
        let text = serde_json::to_string(&made).expect("JSON");
        let back: Summary = serde_json::from_str(&text).expect("read back");
        assert_eq!(back, made, "the same");
    }

    #[test]
    fn an_empty_list_section_says_none() {
        let text = json!({"analysis": []}).to_string();
        assert_eq!(
            section(&text, Part::Analysis).as_deref(),
            Some("none"),
            "no analysis"
        );
        assert_eq!(
            section(&text, Part::Subject).as_deref(),
            Some("none"),
            "no subject"
        );
    }
}
