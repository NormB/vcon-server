//! Any bytes as a sealed vCon or a wrapped key read back from disk. A blob
//! not made by `seal` or `wrap` with this key and context never opens.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use vcon_server::envelope::{self, Context, Key};
use vcon_server::ids::{TenantId, VersionId};

fn key() -> &'static Key {
    static KEY: OnceLock<Key> = OnceLock::new();
    KEY.get_or_init(|| Key::generate().expect("one locked page"))
}

fuzz_target!(|blob: &[u8]| {
    let tenant = TenantId([7; 16]);
    let version = Context::Version {
        tenant,
        uuid: uuid::Uuid::from_bytes([9; 16]),
        version: VersionId([8; 16]),
    };
    assert!(
        envelope::open(key(), &version, blob).is_err(),
        "only a blob sealed with this key and context opens, and the fuzzer cannot forge one"
    );
    assert!(
        envelope::unwrap(key(), &Context::Tenant { tenant }, blob).is_err(),
        "only a key wrapped with this key and context unwraps, and the fuzzer cannot forge one"
    );
});
