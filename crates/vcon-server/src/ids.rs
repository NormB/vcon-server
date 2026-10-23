//! Identifiers the server makes for its own records: 128 random bits each,
//! written as 32 lowercase hex digits.

use std::fmt;

/// A tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TenantId(pub [u8; 16]);

/// One stored version of a vCon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VersionId(pub [u8; 16]);

impl VersionId {
    /// A new random version id.
    pub fn generate() -> VersionId {
        let mut bytes = [0; 16];
        // AWS-LC's RAND_bytes aborts the process when it cannot get enough
        // randomness, so `fill` never returns its error.
        aws_lc_rs::rand::fill(&mut bytes).expect("AWS-LC aborts instead of failing");
        VersionId(bytes)
    }
}

impl VersionId {
    /// Reads a version id written by `Display`: exactly 32 lowercase hex
    /// digits.
    pub fn parse(text: &str) -> Option<VersionId> {
        let digits = text.as_bytes();
        if digits.len() != 32 {
            return None;
        }
        let digit = |d: u8| match d {
            b'0'..=b'9' => Some(d - b'0'),
            b'a'..=b'f' => Some(d - b'a' + 10),
            _ => None,
        };
        let mut bytes = [0; 16];
        let (pairs, _) = digits.as_chunks::<2>();
        for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
            *byte = digit(*high)? << 4 | digit(*low)?;
        }
        Some(VersionId(bytes))
    }
}

fn hex(bytes: &[u8; 16], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    bytes.iter().try_for_each(|b| write!(f, "{b:02x}"))
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
        assert_eq!(TenantId(bytes).to_string(), expected);
        assert_eq!(VersionId(bytes).to_string(), expected);
    }

    #[test]
    fn version_ids_are_random() {
        let ids: Vec<VersionId> = (0..8).map(|_| VersionId::generate()).collect();
        for i in 0..16 {
            assert!(
                ids.iter().any(|id| id.0[i] != ids[0].0[i]),
                "byte {i} never varies"
            );
        }
    }

    #[test]
    fn a_version_id_parses_only_from_its_own_spelling() {
        let id = VersionId::generate();
        assert_eq!(VersionId::parse(&id.to_string()), Some(id));
        let text = id.to_string();
        let bad = [
            String::new(),
            text[..31].to_owned(),
            format!("{text}0"),
            text.to_uppercase(),
            format!("g{}", &text[1..]),
            format!("{}.tmp", text),
            format!("+{}", &text[1..]),
        ];
        for bad in bad {
            if bad != text {
                assert_eq!(VersionId::parse(&bad), None, "{bad:?}");
            }
        }
    }
}
