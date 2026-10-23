//! Reads a vCon without changing it.

// Used by `inspect`, which a later change adds; until then only tests call it.
#[allow(dead_code)]
mod json;

/// The name of this crate.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// Why a received body cannot be stored as a vCon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// The body is not exactly one JSON object.
    NotJson,
    /// An object in the body repeats a member name.
    DuplicateKey,
}
