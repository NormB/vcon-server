//! Reads a vCon without changing it.

// Used by `inspect`, which a later change adds; until then only tests call them.
#[allow(dead_code)]
mod critical;
#[allow(dead_code)]
mod form;
#[allow(dead_code)]
mod gzip;
#[allow(dead_code)]
mod identity;
#[cfg(test)]
mod ietf_examples;
#[allow(dead_code)]
mod json;
#[allow(dead_code)]
mod signed;

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
    /// The vCon has no uuid.
    MissingUuid,
    /// The uuid is not a string in RFC 9562 text form.
    BadUuid,
    /// The vCon has no created_at.
    MissingCreatedAt,
    /// A signed vCon's payload or signatures cannot be read as a JWS
    /// carrying a vCon.
    BadPayload,
    /// A `uuid` in a JWS header differs from the uuid of the signed vCon.
    UuidMismatch,
    /// `critical` or `must_support` is not an array of strings.
    BadCritical,
    /// `critical` or `must_support` names extensions this server does not
    /// support; they are listed in order of first appearance.
    UnsupportedCritical(Vec<String>),
}
