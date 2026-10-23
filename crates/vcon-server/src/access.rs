//! Turns the bearer token on a request into a caller, or refuses it.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::ids::KeyId;

/// The fixed start of every token, so secret scanners can spot a leaked
/// one.
const PREFIX: &str = "vcs_";

/// A secret is 256 random bits.
const SECRET_LEN: usize = 32;

/// Why a request's token was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// Not a token as `Token` writes one.
    Malformed,
}

/// Reads a token written as `vcs_<key id>_<secret>`: the key id is 16
/// lowercase hex digits, and the secret is 256 bits in base64url without
/// padding. The secret may hold `_`, so the parts are found by position.
pub fn parse(token: &str) -> Result<KeyId, AuthError> {
    let malformed = AuthError::Malformed;
    let rest = token.strip_prefix(PREFIX).ok_or(malformed)?;
    let (id, rest) = rest.split_at_checked(16).ok_or(malformed)?;
    let secret = rest.strip_prefix('_').ok_or(malformed)?;
    let id = KeyId::parse(id).ok_or(malformed)?;
    // The decoding refuses padding, other alphabets and stray trailing
    // bits, so each secret has one spelling.
    let secret = URL_SAFE_NO_PAD.decode(secret).map_err(|_| malformed)?;
    if secret.len() != SECRET_LEN {
        return Err(malformed);
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef";

    fn secret(byte: u8) -> String {
        URL_SAFE_NO_PAD.encode([byte; 32])
    }

    #[test]
    fn a_well_formed_token_gives_its_key_id() {
        let id = KeyId([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        let secrets = [
            secret(0x00),
            secret(0xff),
            // 0xfb gives '-' and 0xff gives '_' in base64url.
            URL_SAFE_NO_PAD.encode([0xfb; 32]),
            URL_SAFE_NO_PAD.encode([[0xff; 16], [0x00; 16]].concat()),
        ];
        for secret in secrets {
            assert_eq!(secret.len(), 43);
            let token = format!("vcs_{ID}_{secret}");
            assert_eq!(parse(&token), Ok(id), "{token}");
        }
        assert!(
            secret(0xff).contains('_'),
            "a secret may hold the separator"
        );
    }

    #[test]
    fn a_malformed_token_is_refused() {
        let good = secret(0x42);
        let malformed = [
            String::new(),
            "vcs_".to_owned(),
            format!("{ID}_{good}"),
            format!("vck_{ID}_{good}"),
            format!("VCS_{ID}_{good}"),
            format!("vcs_{}_{good}", &ID[..15]),
            format!("vcs_{ID}0_{good}"),
            format!("vcs_{}_{good}", ID.to_uppercase()),
            format!("vcs_0123456789abcdeg_{good}"),
            format!("vcs_{ID}{good}"),
            format!("vcs_{ID}-{good}"),
            format!("vcs_{ID}_{}", &good[..42]),
            format!("vcs_{ID}_{good}A"),
            format!("vcs_{ID}_{good}="),
            format!("vcs_{ID}_{}", good.replace('Q', "+")),
            format!("vcs_{ID}_{}+", &good[..42]),
            format!("vcs_{ID}_{}B", &good[..42]),
            format!(" vcs_{ID}_{good}"),
            format!("vcs_{ID}_{good} "),
            format!("vcs_{ID}__{good}"),
            format!("vcs_\u{0661}123456789abcdef_{good}"),
        ];
        for token in malformed {
            assert_eq!(parse(&token), Err(AuthError::Malformed), "{token:?}");
        }
    }
}
