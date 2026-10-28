//! The crate's public names: what is at the root, what is under `read`.

use std::collections::BTreeSet;

#[test]
fn the_shared_names_are_at_the_root_and_reading_is_under_read() {
    assert_eq!(
        vcon::parse_uuid("01a07da8-c2bb-83e5-b9a2-279e0d16bc46"),
        Some(uuid::Uuid::from_u128(
            0x01a07da8_c2bb_83e5_b9a2_279e0d16bc46
        )),
        "the RFC 9562 text form reads as its 128 bits"
    );
    assert_eq!(
        vcon::parse_uuid("{01a07da8-c2bb-83e5-b9a2-279e0d16bc46}"),
        None,
        "only the RFC 9562 text form"
    );
    let _ = vcon::Form::Unsigned;
    let limits = vcon::read::Limits::new(1 << 20);
    let _: usize = vcon::read::MAX_SIGNED_LAYERS + vcon::read::MAX_JSON_DEPTH;
    let found: Result<vcon::read::Inspection, vcon::read::Rejection> =
        vcon::read::inspect(b"[]", None, &limits, &BTreeSet::new());
    assert_eq!(found, Err(vcon::read::Rejection::NotJson));
}
