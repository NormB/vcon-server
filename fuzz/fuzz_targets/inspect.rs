//! Any body, read as a vCon with no media type: the gzip check by magic
//! number, the JSON reader and all three forms. It must give an answer,
//! never a panic, and the same answer once gzipped.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vcon_fuzz::{LIMITS, gzip, gzip_agrees, supported};

fuzz_target!(|body: &[u8]| {
    let supported = supported();
    let plain = vcon::inspect(body, None, &LIMITS, &supported);
    let gzipped = vcon::inspect(&gzip(body), None, &LIMITS, &supported);
    assert!(
        gzip_agrees(body, &plain, &gzipped),
        "gzip changed the answer"
    );
});
