//! Any bytes as the payload of a JWS: the base64url decoding, a gzipped
//! payload, nested signed layers and the header uuids. The first byte says
//! which header uuid the signature carries. A payload must be read as it
//! is read on its own.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vcon_fuzz::{LIMITS, OTHER_UUID, jws, signed_agrees, supported};

fuzz_target!(|data: &[u8]| {
    let Some((&choice, payload)) = data.split_first() else {
        return;
    };
    let supported = supported();
    let plain = vcon::inspect(payload, None, &LIMITS, &supported);
    let own = plain.as_ref().ok().map(|found| found.uuid.to_string());
    let header = match choice % 3 {
        0 => None,
        1 => own.as_deref(),
        _ => Some(OTHER_UUID),
    };
    let signed = vcon::inspect(&jws(payload, header), None, &LIMITS, &supported);
    assert!(
        signed_agrees(&plain, header, &signed),
        "signing changed the answer"
    );
});
