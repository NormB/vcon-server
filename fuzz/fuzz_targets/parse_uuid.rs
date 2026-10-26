//! Any text as a uuid in a request path. Only the RFC 9562 text form is
//! taken, and it is taken back as written, in either case.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    if let Some(uuid) = vcon::parse_uuid(text) {
        assert!(
            uuid.hyphenated().to_string().eq_ignore_ascii_case(text),
            "a uuid read back otherwise than written"
        );
    }
});
