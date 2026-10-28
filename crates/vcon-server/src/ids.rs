//! Identifiers, written in lowercase hex: a tenant or a version is 128
//! random bits, a key 64 bits.

use std::fmt;

/// Names a key without revealing it: an encryption key by the start of
/// its SHA-256 (`Key::id`), an access key by 64 random bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyId(pub [u8; 8]);

impl KeyId {
    /// A new random key id.
    #[must_use]
    pub fn generate() -> KeyId {
        KeyId(random())
    }

    /// Reads a key id written by `Display`: exactly 16 lowercase hex
    /// digits.
    pub fn parse(text: &str) -> Option<KeyId> {
        unhex(text).map(KeyId)
    }

    /// The id's 8 bytes, as catalogs store them.
    pub(crate) fn bytes(self) -> [u8; 8] {
        self.0
    }
}

/// A tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TenantId(pub [u8; 16]);

impl TenantId {
    /// A new random tenant id.
    #[must_use]
    pub fn generate() -> TenantId {
        TenantId(random())
    }

    /// Reads a tenant id written by `Display`: exactly 32 lowercase hex
    /// digits.
    pub fn parse(text: &str) -> Option<TenantId> {
        unhex(text).map(TenantId)
    }
}

/// One stored version of a vCon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VersionId(pub [u8; 16]);

impl VersionId {
    /// A new random version id.
    #[must_use]
    pub fn generate() -> VersionId {
        VersionId(random())
    }
}

impl VersionId {
    /// Reads a version id written by `Display`: exactly 32 lowercase hex
    /// digits.
    pub fn parse(text: &str) -> Option<VersionId> {
        unhex(text).map(VersionId)
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    // AWS-LC's RAND_bytes aborts the process when it cannot get enough
    // randomness, so `fill` never returns its error.
    aws_lc_rs::rand::fill(&mut bytes).expect("AWS-LC aborts instead of failing");
    bytes
}

/// Reads exactly 2N lowercase hex digits.
fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let digits = text.as_bytes();
    if digits.len() != 2 * N {
        return None;
    }
    let digit = |d: u8| match d {
        b'0'..=b'9' => Some(d - b'0'),
        b'a'..=b'f' => Some(d - b'a' + 10),
        _ => None,
    };
    let mut bytes = [0; N];
    let (pairs, _) = digits.as_chunks::<2>();
    for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
        *byte = digit(*high)? << 4 | digit(*low)?;
    }
    Some(bytes)
}

fn hex(bytes: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    bytes.iter().try_for_each(|b| write!(f, "{b:02x}"))
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex(&self.0, f)
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex(&self.0, f)
    }
}

impl fmt::Display for VersionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_written_as_32_lowercase_hex_digits() {
        let mut bytes = [0; 16];
        bytes[0] = 0x0a;
        bytes[15] = 0xff;
        let expected = format!("0a{}ff", "00".repeat(14));
        assert_eq!(
            TenantId(bytes).to_string(),
            expected,
            "two lower-case hex digits per byte, in order, zeros kept"
        );
        assert_eq!(
            VersionId(bytes).to_string(),
            expected,
            "two lower-case hex digits per byte, in order, zeros kept"
        );
    }

    /// Every byte of `ids` varies across them.
    fn varies(ids: &[[u8; 16]]) {
        for i in 0..16 {
            assert!(
                ids.iter().any(|id| id[i] != ids[0][i]),
                "byte {i} never varies"
            );
        }
    }

    #[test]
    fn a_key_id_is_64_random_bits_written_as_16_hex_digits() {
        let ids: Vec<[u8; 8]> = (0..8).map(|_| KeyId::generate().0).collect();
        for i in 0..8 {
            assert!(
                ids.iter().any(|id| id[i] != ids[0][i]),
                "byte {i} never varies"
            );
        }
        let id = KeyId([0x0a, 0, 0, 0, 0, 0, 0, 0xff]);
        assert_eq!(
            id.to_string(),
            "0a000000000000ff",
            "two lower-case hex digits per byte, in order, zeros kept"
        );
        assert_eq!(
            KeyId::parse("0a000000000000ff"),
            Some(id),
            "parse reads the spelling Display writes"
        );
        for bad in [
            "0A000000000000FF",
            "0a000000000000f",
            "0a000000000000ff0",
            "",
        ] {
            assert_eq!(KeyId::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn ids_are_random() {
        varies(&(0..8).map(|_| VersionId::generate().0).collect::<Vec<_>>());
        varies(&(0..8).map(|_| TenantId::generate().0).collect::<Vec<_>>());
    }

    /// The spellings parse refuses, given one id's own spelling `text`.
    fn misspelled(text: &str) -> [String; 7] {
        [
            String::new(),
            text[..31].to_owned(),
            format!("{text}0"),
            text.to_uppercase(),
            format!("g{}", &text[1..]),
            format!("{text}.tmp"),
            format!("+{}", &text[1..]),
        ]
    }

    #[test]
    fn tenant_and_version_ids_parse_only_from_their_own_spelling() {
        let tenant = TenantId::generate();
        let version = VersionId::generate();
        assert_eq!(
            TenantId::parse(&tenant.to_string()),
            Some(tenant),
            "parse reads the spelling Display writes"
        );
        assert_eq!(
            VersionId::parse(&version.to_string()),
            Some(version),
            "parse reads the spelling Display writes"
        );
        for (text, parsed) in [
            (
                tenant.to_string(),
                (|t: &str| TenantId::parse(t).is_some()) as fn(&str) -> bool,
            ),
            (version.to_string(), |t: &str| VersionId::parse(t).is_some()),
        ] {
            for bad in misspelled(&text) {
                // An id with no letter is its own upper-case spelling.
                if bad != text {
                    assert!(!parsed(&bad), "not an id's own spelling: {bad:?}");
                }
            }
        }
    }
}
