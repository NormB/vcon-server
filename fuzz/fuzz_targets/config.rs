//! Any text as the configuration file the server reads at start.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = vcon_server::config::parse(text);
});
