//! A library for vCons. Its current scope is reading and checking a vCon:
//! which form it is in, whether it is gzipped, its uuid, and whether it
//! needs extensions the caller does not support. Otherwise it says why the
//! vCon is refused. [`inspect`] does no I/O and returns no bytes, so the
//! caller can store the body exactly as received.

use std::collections::BTreeSet;

use uuid::Uuid;

mod critical;
mod form;
mod gzip;
mod identity;
#[cfg(test)]
mod ietf_examples;
mod json;
mod signed;

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Bounds on the work one inspection may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most bytes all gzip inflation in one body may produce, counting
    /// the body itself and every gzipped payload layer inside it.
    pub inflated: u64,
}

/// Reads a uuid written in RFC 9562 text form, 8-4-4-4-12 hex digits in
/// either case, as a vCon's uuid must be; no other spelling is taken.
pub fn parse_uuid(text: &str) -> Option<Uuid> {
    identity::parse_uuid_text(text).ok()
}

/// What a received vCon is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inspection {
    /// Its form.
    pub form: Form,
    /// Whether the body as received is gzipped.
    pub gzip: bool,
    /// Its uuid: from the vCon itself, from the innermost signed payload, or
    /// from the unprotected header of an encrypted vCon.
    pub uuid: Uuid,
}

/// Reads `body` without changing it. `content_type` is the media type it
/// arrived with, if any, and `supported` the extension names this server
/// accepts in `critical`. An encrypted vCon's `critical` cannot be read and
/// is not checked.
pub fn inspect(
    body: &[u8],
    content_type: Option<&str>,
    limits: &Limits,
    supported: &BTreeSet<String>,
) -> Result<Inspection, Rejection> {
    let (inflated, gzip) = gzip::inflate(body, content_type, limits.inflated)?;
    let mut budget = limits.inflated;
    if gzip {
        budget -= inflated.len() as u64;
    }
    let object = json::parse_object(&inflated)?;
    let form = form::form_of(&object)?;
    let uuid = match form {
        Form::Unsigned => {
            let uuid = identity::unsigned_identity(&object)?;
            critical::check_critical(&object, supported)?;
            uuid
        }
        Form::Signed => signed::signed_identity(&object, &mut budget, supported)?,
        Form::Encrypted => identity::encrypted_identity(&object)?,
    };
    Ok(Inspection { form, gzip, uuid })
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
