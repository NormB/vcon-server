//! Any text as a key, tenant or version id, as request paths carry them.
//! An id that parses is written back exactly as it was read.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vcon_server::ids::{KeyId, TenantId, VersionId};

fuzz_target!(|text: &str| {
    if let Some(id) = KeyId::parse(text) {
        assert_eq!(id.to_string(), text);
    }
    if let Some(id) = TenantId::parse(text) {
        assert_eq!(id.to_string(), text);
    }
    if let Some(id) = VersionId::parse(text) {
        assert_eq!(id.to_string(), text);
    }
});
