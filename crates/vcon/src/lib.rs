// The crate's page on docs.rs is its README, so the example there runs as
// a doc test.
#![doc = include_str!("../README.md")]

use std::fmt;

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
pub mod write;

/// Reads a uuid written in RFC 9562 text form, 8-4-4-4-12 hex digits in
/// either case, as a vCon's uuid must be; no other spelling is taken.
///
/// ```
/// let text = "01A07DA8-C2BB-83E5-B9A2-279E0D16BC46";
/// let uuid = vcon::parse_uuid(text).expect("8-4-4-4-12 hex digits");
/// assert_eq!(
///     uuid.to_string(),
///     "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
///     "upper case hex reads as the same uuid"
/// );
/// let bare = "01a07da8c2bb83e5b9a2279e0d16bc46";
/// assert_eq!(vcon::parse_uuid(bare), None, "without hyphens it is not RFC 9562 text form");
/// ```
#[must_use]
pub fn parse_uuid(text: &str) -> Option<Uuid> {
    identity::parse_uuid_text(text).ok()
}

/// The three forms a vCon can take. A later release may add one, so a
/// `match` on it outside this crate needs a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Form {
    /// Plain JSON.
    Unsigned,
    /// A JWS whose payload is the vCon.
    Signed,
    /// A JWE whose ciphertext is a signed vCon.
    Encrypted,
}

impl Form {
    /// The form's name in lower case: `unsigned`, `signed` or `encrypted`.
    ///
    /// ```
    /// assert_eq!(vcon::Form::Signed.as_str(), "signed", "a JWS is the signed form");
    /// ```
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Form::Unsigned => "unsigned",
            Form::Signed => "signed",
            Form::Encrypted => "encrypted",
        }
    }
}

/// Writes [`Form::as_str`].
impl fmt::Display for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
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
    use super::Form;

    #[test]
    fn each_form_has_its_lower_case_name() {
        let names = [
            (Form::Unsigned, "unsigned"),
            (Form::Signed, "signed"),
            (Form::Encrypted, "encrypted"),
        ];
        for (form, name) in names {
            assert_eq!(
                form.as_str(),
                name,
                "as_str gives the form's name in lower case: {form:?}"
            );
            assert_eq!(
                form.to_string(),
                name,
                "Display writes what as_str gives: {form:?}"
            );
        }
    }
}
