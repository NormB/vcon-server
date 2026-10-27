//! The crate's public names: what is at the root, what is under `read`.

use std::collections::BTreeSet;

#[test]
fn the_shared_names_are_at_the_root_and_reading_is_under_read() {
    let _: Option<uuid::Uuid> = vcon::parse_uuid("01a07da8-c2bb-83e5-b9a2-279e0d16bc46");
    let _ = vcon::Form::Unsigned;
    let limits = vcon::read::Limits::new(1 << 20);
    assert_eq!(limits.inflated, 1 << 20);
    let _: usize = vcon::read::MAX_SIGNED_LAYERS + vcon::read::MAX_JSON_DEPTH;
    let found: Result<vcon::read::Inspection, vcon::read::Rejection> =
        vcon::read::inspect(b"[]", None, &limits, &BTreeSet::new());
    assert_eq!(found, Err(vcon::read::Rejection::NotJson));
}
