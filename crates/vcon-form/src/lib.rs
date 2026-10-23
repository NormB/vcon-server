//! Reads a vCon without changing it.

// Used by `inspect`, which a later change adds; until then only tests call them.
#[allow(dead_code)]
mod form;
#[allow(dead_code)]
mod gzip;
#[allow(dead_code)]
mod json;

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

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

/// Why a received body cannot be stored as a vCon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// Inflating a gzip body passed the configured limit.
    TooLarge,
    /// The body is marked as gzip but is not a valid gzip stream.
    BadGzip,
    /// The body is not exactly one JSON object.
    NotJson,
    /// An object in the body repeats a member name.
    DuplicateKey,
    /// The body has the members of both the signed and the encrypted form.
    AmbiguousForm,
    /// The body has the members of no form.
    NoForm,
}
