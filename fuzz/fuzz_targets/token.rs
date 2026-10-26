//! Any text as an API token: `vcs_<key id>_<secret>`. A token that parses
//! starts with the key id it gives.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vcon_server::access;

fuzz_target!(|token: &str| {
    if let Ok(id) = access::parse(token) {
        assert!(
            token.starts_with(&format!("vcs_{id}_")),
            "{token:?} gave {id}"
        );
    }
});
