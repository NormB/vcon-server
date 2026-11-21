//! Synthetic vCons: made from a seed and an index, with reserved values only
//! (example.com mail addresses, 555-01xx numbers).

use serde_json::json;

use crate::mix::SizeMix;
use crate::rng::{self, SplitMix64};

/// The one creation time every synthetic vCon states.
const CREATED_AT: &str = "2026-10-25T12:00:00Z";

/// One synthetic vCon: its uuid and its JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synthetic {
    /// The uuid, in RFC 9562 text form.
    pub id: String,
    /// The JSON text, of the size the mix picked (or the smallest a vCon of
    /// this shape can be, when the mix picked less).
    pub body: Vec<u8>,
}

/// A random (version 4) uuid from `rng`. The name holds no "uid": CodeQL's
/// rust/cleartext-logging reads such a name as an account id, and a vCon's
/// id is public.
pub fn random_id(rng: &mut SplitMix64) -> String {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&rng.next_u64().to_be_bytes());
    b[8..].copy_from_slice(&rng.next_u64().to_be_bytes());
    b[6] = b[6] & 0x0f | 0x40;
    b[8] = b[8] & 0x3f | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The generator of vCon number `index` in the run seeded `seed`.
fn generator(seed: u64, index: u64) -> SplitMix64 {
    SplitMix64::new(rng::finalize(seed ^ rng::finalize(index ^ 0x5ca1_e000)))
}

/// One party with a reserved mail address and number.
fn party(rng: &mut SplitMix64) -> serde_json::Value {
    let n = rng.below(100);
    json!({
        "mailto": format!("party{n:02}@example.com"),
        "tel": format!("+1-555-01{n:02}"),
    })
}

/// The JSON text of a vCon whose one dialog holds `body`.
fn text(id: &str, index: u64, parties: &[serde_json::Value], body: &str) -> Vec<u8> {
    json!({
        "created_at": CREATED_AT,
        "dialog": [{
            "body": body,
            "encoding": "none",
            "mediatype": "text/plain",
            "parties": [0, 1],
            "start": CREATED_AT,
            "type": "text",
        }],
        "parties": parties,
        "subject": format!("scale harness vCon {index}"),
        "uuid": id,
    })
    .to_string()
    .into_bytes()
}

/// The uuid of vCon number `index` of the run seeded `seed`, without making
/// its text: the reads name stored vCons by it.
#[must_use]
pub fn id(seed: u64, index: u64) -> String {
    random_id(&mut generator(seed, index))
}

/// The index of the vCon read number `j` names, of `count` stored: even
/// reads are uniform over all of them, odd reads over the newest hundredth
/// (at least one).
#[must_use]
pub fn read_index(seed: u64, j: u64, count: u64) -> u64 {
    let mut rng = SplitMix64::new(rng::finalize(seed ^ 0x7ead ^ rng::finalize(j)));
    if j.is_multiple_of(2) {
        rng.below(count)
    } else {
        let newest = (count / 100).max(1).min(count);
        count - newest + rng.below(newest)
    }
}

/// The vCon number `index` of the run seeded `seed`; the same pair always
/// makes the same vCon.
#[must_use]
pub fn vcon(seed: u64, index: u64, mix: &SizeMix) -> Synthetic {
    // The uuid is the generator's first draw, so id() makes the same one.
    let mut rng = generator(seed, index);
    let id = random_id(&mut rng);
    let target = mix.pick(rng.below(mix.total_weight()));
    let parties = [party(&mut rng), party(&mut rng)];
    let empty = text(&id, index, &parties, "");
    // Lowercase letters and spaces need no JSON escape, so each one adds
    // exactly one byte to the text.
    let filler: String = (0..target.saturating_sub(empty.len()))
        .map(|_| match rng.below(27) {
            26 => ' ',
            k => char::from(b'a' + u8::try_from(k).unwrap_or(0)),
        })
        .collect();
    let body = if filler.is_empty() {
        empty
    } else {
        text(&id, index, &parties, &filler)
    };
    Synthetic { id, body }
}
