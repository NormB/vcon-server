// The crate's page on docs.rs is its README, so the example there runs as
// a doc test.
#![doc = include_str!("../README.md")]

use uuid::Uuid;

mod critical;
mod form;
mod gzip;
mod identity;
#[cfg(test)]
mod ietf_examples;
mod json;
pub mod read;
mod signed;

/// Reads a uuid written in RFC 9562 text form, 8-4-4-4-12 hex digits in
/// either case, as a vCon's uuid must be; no other spelling is taken.
pub fn parse_uuid(text: &str) -> Option<Uuid> {
    identity::parse_uuid_text(text).ok()
}

/// The three forms a vCon can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Plain JSON.
    Unsigned,
    /// A JWS whose payload is the vCon.
    Signed,
    /// A JWE whose ciphertext is a signed vCon.
    Encrypted,
}

/// The crate's name is not part of its API:
///
/// ```compile_fail,E0425
/// let _ = vcon::CRATE_NAME;
/// ```
#[cfg(doctest)]
pub struct NoCrateName;

#[cfg(test)]
mod tests {
    #[test]
    fn the_crate_is_named_vcon() {
        assert_eq!(env!("CARGO_PKG_NAME"), "vcon");
    }
}
