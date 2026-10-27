//! Any body sent as `application/vcon+gzip`: the inflater meets hostile
//! streams (bad headers, bad checksums, several members, bombs) and must
//! stop at the limit.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vcon_fuzz::{LIMITS, supported};

fuzz_target!(|body: &[u8]| {
    let found = vcon::read::inspect(body, Some("application/vcon+gzip"), &LIMITS, &supported());
    if let Ok(found) = found {
        assert!(found.gzip, "a body sent as gzip was read as plain");
    }
});
