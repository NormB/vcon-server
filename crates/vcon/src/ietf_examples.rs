//! The IETF core draft's own examples, kept byte for byte in
//! tests/fixtures/ietf, must read as the form and uuid the draft gives them.

use std::collections::BTreeSet;

use uuid::Uuid;

use crate::Form;
use crate::critical::check_critical;
use crate::form::form_of;
use crate::identity::{encrypted_identity, unsigned_identity};
use crate::json::parse_object;
use crate::signed::signed_identity;

const UNSIGNED: &[u8] = include_bytes!("../tests/fixtures/ietf/ab_call_ext_rec.vcon");
const SIGNED: &[u8] = include_bytes!("../tests/fixtures/ietf/ab_call_ext_rec_signed.vcon");
const ENCRYPTED: &[u8] = include_bytes!("../tests/fixtures/ietf/ab_call_ext_rec_encrypted.vcon");

/// The uuid all three examples share (core-04, Appendix A.4 to A.7).
const ID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

fn id() -> Uuid {
    ID.parse().expect("the draft's uuid")
}

#[test]
fn the_unsigned_example_reads_as_unsigned() {
    let object = parse_object(UNSIGNED).expect("A.4 is one JSON object");
    assert_eq!(
        form_of(&object),
        Ok(Form::Unsigned),
        "A.4 has neither the JWS nor the JWE members"
    );
    assert_eq!(
        unsigned_identity(&object),
        Ok(id()),
        "A.4 carries the uuid all three examples share"
    );
    assert_eq!(
        check_critical(&object, &BTreeSet::new()),
        Ok(()),
        "A.4 needs no extension, so a reader that supports none accepts it"
    );
}

#[test]
fn the_signed_example_reads_as_signed() {
    let object = parse_object(SIGNED).expect("A.6 is one JSON object");
    assert_eq!(
        form_of(&object),
        Ok(Form::Signed),
        "A.6 has a JWS's payload and signatures"
    );
    let mut budget = 1 << 20;
    assert_eq!(
        signed_identity(&object, &mut budget, &BTreeSet::new()),
        Ok(id()),
        "A.6's payload is A.4, so it has A.4's uuid"
    );
}

#[test]
fn the_encrypted_example_reads_as_encrypted() {
    let object = parse_object(ENCRYPTED).expect("A.7 is one JSON object");
    assert_eq!(
        form_of(&object),
        Ok(Form::Encrypted),
        "A.7 has a JWE's ciphertext and recipients"
    );
    assert_eq!(
        encrypted_identity(&object),
        Ok(id()),
        "A.7's unprotected header carries the uuid the three examples share"
    );
}
