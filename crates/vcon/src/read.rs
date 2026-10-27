//! Reading and checking a vCon: which form it is in, whether it is
//! gzipped, its uuid, and whether it needs extensions the caller does not
//! support. [`inspect`] does no I/O and returns no bytes.
//!
//! [`Inspection`], [`Limits`] and [`Rejection`] are `#[non_exhaustive]`, so
//! a later release can add a field or a refusal without breaking a caller.
//! A caller reads an [`Inspection`] but does not build one:
//!
//! ```compile_fail,E0639
//! let found = vcon::read::Inspection {
//!     form: vcon::Form::Unsigned,
//!     gzip: false,
//!     uuid: "01a07da8-c2bb-83e5-b9a2-279e0d16bc46".parse().unwrap(),
//! };
//! ```
//!
//! makes [`Limits`] with [`Limits::new`]:
//!
//! ```compile_fail,E0639
//! let limits = vcon::read::Limits { inflated: 1 << 20 };
//! ```
//!
//! and, matching a [`Rejection`], has an arm for refusals it does not name:
//!
//! ```compile_fail,E0004
//! fn named(r: &vcon::read::Rejection) -> &'static str {
//!     use vcon::read::Rejection::*;
//!     match r {
//!         TooLarge => "too-large", TooDeep => "too-deep", BadGzip => "bad-gzip",
//!         NotJson => "not-json", DuplicateKey => "duplicate", AmbiguousForm => "ambiguous",
//!         NoForm => "no-form", MissingUuid => "missing-uuid", BadUuid => "bad-uuid",
//!         MissingCreatedAt => "missing-created-at", BadPayload => "bad-payload",
//!         UuidMismatch => "uuid-mismatch", BadCritical => "bad-critical",
//!         UnsupportedCritical(_) => "unsupported",
//!     }
//! }
//! ```

use std::collections::BTreeSet;

use uuid::Uuid;

use crate::Form;
use crate::{critical, form, gzip, identity, json, signed};

/// The most signed (JWS) layers a body may have, counting from the body to
/// the unsigned vCon: one for a vCon signed once. A layer is added each time
/// a signed vCon is signed again, as the vCon core draft describes for
/// archives kept longer than their signing certificates; 64 allows one a
/// year for 63 years. More is refused as [`Rejection::TooDeep`] before the
/// next layer is read.
pub const MAX_SIGNED_LAYERS: usize = 64;

/// The most arrays and objects open at once in any one JSON text read: the
/// body, a signed payload, the vCon itself counting as one. More is refused
/// as [`Rejection::TooDeep`]. It is below the JSON parser's own bound of 127,
/// so too deep is told apart from not JSON.
pub const MAX_JSON_DEPTH: usize = 64;

/// Bounds on the work one inspection may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// The most bytes all gzip inflation in one body may produce, counting
    /// the body itself and every gzipped payload layer inside it.
    pub inflated: u64,
}

impl Limits {
    /// Limits that let gzip inflation in one body produce at most
    /// `inflated` bytes.
    pub const fn new(inflated: u64) -> Limits {
        Limits { inflated }
    }
}

/// What a received vCon is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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

/// Why a received body cannot be stored as a vCon.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Rejection {
    /// Inflating a gzip body passed the configured limit.
    TooLarge,
    /// The body nests deeper than [`MAX_SIGNED_LAYERS`] signed layers, or a
    /// JSON text in it deeper than [`MAX_JSON_DEPTH`].
    TooDeep,
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
