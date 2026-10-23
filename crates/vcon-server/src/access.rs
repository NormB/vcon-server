//! Turns the bearer token on a request into a caller, or refuses it.

use aws_lc_rs::constant_time;
use aws_lc_rs::digest::{SHA256, digest};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::ids::{KeyId, TenantId};
use crate::tenants::Tenants;

/// The fixed start of every token, so secret scanners can spot a leaked
/// one.
const PREFIX: &str = "vcs_";

/// A secret is 256 random bits.
const SECRET_LEN: usize = 32;

/// What a key may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Write,
    Read,
    Delete,
    Legal,
    Admin,
}

impl Scope {
    /// Every scope, in the order a key's scopes are written.
    const ALL: [Scope; 5] = [
        Scope::Write,
        Scope::Read,
        Scope::Delete,
        Scope::Legal,
        Scope::Admin,
    ];

    fn bit(self) -> u8 {
        1 << self as u8
    }

    fn name(self) -> &'static str {
        match self {
            Scope::Write => "write",
            Scope::Read => "read",
            Scope::Delete => "delete",
            Scope::Legal => "legal",
            Scope::Admin => "admin",
        }
    }
}

/// A set of scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Scopes(u8);

impl Scopes {
    pub fn of(scopes: &[Scope]) -> Scopes {
        Scopes(scopes.iter().fold(0, |bits, scope| bits | scope.bit()))
    }

    pub fn contains(self, scope: Scope) -> bool {
        self.0 & scope.bit() != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Reads scopes written by `Display`: names in the fixed order, comma
    /// separated, each once.
    pub fn parse(text: &str) -> Option<Scopes> {
        let scopes: Vec<Scope> = text
            .split(',')
            .map(|name| Scope::ALL.into_iter().find(|s| s.name() == name))
            .collect::<Option<_>>()?;
        let read = Scopes::of(&scopes);
        (read.to_string() == text).then_some(read)
    }
}

impl std::fmt::Display for Scopes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = Scope::ALL
            .into_iter()
            .filter(|s| self.contains(*s))
            .map(Scope::name)
            .collect();
        f.write_str(&names.join(","))
    }
}

/// A new token for a new key: `vcs_<key id>_<secret>`, with a random id
/// and a random 256-bit secret.
pub(crate) fn new_token() -> (KeyId, String) {
    let id = KeyId::generate();
    let mut secret = [0; SECRET_LEN];
    // AWS-LC's RAND_bytes aborts the process when it cannot get enough
    // randomness, so `fill` never returns its error.
    aws_lc_rs::rand::fill(&mut secret).expect("AWS-LC aborts instead of failing");
    let token = format!("{PREFIX}{id}_{}", URL_SAFE_NO_PAD.encode(secret));
    (id, token)
}

/// The SHA-256 of a whole token, as the key table stores it.
pub(crate) fn token_hash(token: &str) -> [u8; 32] {
    digest(&SHA256, token.as_bytes())
        .as_ref()
        .try_into()
        .expect("SHA-256 is 32 bytes")
}

/// Who a request comes from: a tenant's key, or with no tenant the
/// operator's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    pub key: KeyId,
    pub tenant: Option<TenantId>,
    pub scopes: Scopes,
}

/// Why a request's token was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// No Authorization header.
    Missing,
    /// Not `Bearer` and a token as `new_token` writes one.
    Malformed,
    /// No key has this id, or the secret is not its own.
    Unknown,
    /// The key was revoked.
    Revoked,
    /// The key table could not be read, so nothing is let through.
    Io,
}

/// The caller a request's Authorization header names. The key is looked up
/// on every call, with no cache, so a revoked key fails on its next
/// request.
pub fn authenticate(keys: &Tenants, authorization: Option<&str>) -> Result<Caller, AuthError> {
    let header = authorization.ok_or(AuthError::Missing)?;
    let token = bearer(header).ok_or(AuthError::Malformed)?;
    let id = parse(token)?;
    let key = keys
        .stored_key(id)
        .map_err(|_| AuthError::Io)?
        .ok_or(AuthError::Unknown)?;
    // The secret is checked before revocation, so a wrong secret learns
    // nothing about the key.
    constant_time::verify_slices_are_equal(&token_hash(token), &key.hash)
        .map_err(|_| AuthError::Unknown)?;
    if key.revoked {
        return Err(AuthError::Revoked);
    }
    Ok(Caller {
        key: id,
        tenant: key.tenant,
        scopes: key.scopes,
    })
}

/// The token in `Bearer <token>`. The scheme's case does not matter, and
/// one or more spaces may follow it (RFC 7235).
fn bearer(header: &str) -> Option<&str> {
    let (scheme, rest) = header.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("Bearer")
        .then(|| rest.trim_start_matches(' '))
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

    #[test]
    fn scopes_are_a_set_written_in_a_fixed_order() {
        let none = Scopes::of(&[]);
        assert!(none.is_empty());
        let scopes = Scopes::of(&[Scope::Delete, Scope::Write, Scope::Delete]);
        assert!(!scopes.is_empty());
        for scope in Scope::ALL {
            assert_eq!(
                scopes.contains(scope),
                matches!(scope, Scope::Write | Scope::Delete),
                "{scope:?}"
            );
        }
        assert_eq!(scopes.to_string(), "write,delete");
        assert_eq!(
            Scopes::of(&Scope::ALL).to_string(),
            "write,read,delete,legal,admin"
        );
        for scope in Scope::ALL {
            let one = Scopes::of(&[scope]);
            assert_eq!(Scopes::parse(&one.to_string()), Some(one), "{scope:?}");
        }
        let all = Scopes::of(&Scope::ALL);
        assert_eq!(Scopes::parse(&all.to_string()), Some(all));
    }

    #[test]
    fn only_the_written_spelling_of_scopes_is_read() {
        for text in [
            "",
            "delete,write",
            "write,write",
            "write,",
            ",write",
            "write, read",
            "Write",
            "owner",
            "write,read,",
        ] {
            assert_eq!(Scopes::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_new_token_is_well_formed_and_new_each_time() {
        let (id, token) = new_token();
        assert_eq!(parse(&token), Ok(id));
        let (other_id, other) = new_token();
        assert_ne!(other_id, id);
        assert_ne!(other[21..], token[21..], "a new secret");
    }

    /// RFC 6234's SHA-256 test vector for "abc".
    #[test]
    fn a_token_is_stored_as_its_sha_256() {
        assert_eq!(
            token_hash("abc")
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    mod authenticate {
        use super::*;
        use crate::audit::Actor;
        use crate::tenants::{Fixture, NewKey, fixture};

        fn vcon_scopes() -> Scopes {
            Scopes::of(&[Scope::Write, Scope::Read])
        }

        fn key(f: &Fixture) -> (TenantId, NewKey) {
            let t = f.tenants.create(Actor::Cli, "acme", None).expect("made");
            let key = f
                .tenants
                .create_key(Actor::Cli, Some(t), vcon_scopes(), "x")
                .expect("made");
            (t, key)
        }

        fn auth(f: &Fixture, header: &str) -> Result<Caller, AuthError> {
            authenticate(&f.tenants, Some(header))
        }

        #[test]
        fn a_valid_token_names_its_caller() {
            let f = fixture();
            let (t, key) = key(&f);
            let caller = Caller {
                key: key.id,
                tenant: Some(t),
                scopes: vcon_scopes(),
            };
            for header in [
                format!("Bearer {}", key.token),
                format!("bearer {}", key.token),
                format!("BEARER   {}", key.token),
            ] {
                assert_eq!(auth(&f, &header), Ok(caller), "{header}");
            }
            let operator = f
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made");
            assert_eq!(
                auth(&f, &format!("Bearer {}", operator.token)),
                Ok(Caller {
                    key: operator.id,
                    tenant: None,
                    scopes: Scopes::of(&[Scope::Admin]),
                })
            );
        }

        #[test]
        fn a_missing_or_malformed_header_is_refused() {
            let f = fixture();
            let (_, key) = key(&f);
            assert_eq!(authenticate(&f.tenants, None), Err(AuthError::Missing));
            for header in [
                String::new(),
                "Bearer".to_owned(),
                "Bearer ".to_owned(),
                format!("Bearer{}", key.token),
                format!("Basic {}", key.token),
                format!("Bearers {}", key.token),
                format!("Bearer {} ", key.token),
                format!("Bearer\t{}", key.token),
                "Bearer vcs_0123456789abcdef_short".to_owned(),
            ] {
                assert_eq!(auth(&f, &header), Err(AuthError::Malformed), "{header:?}");
            }
        }

        #[test]
        fn an_unknown_key_or_a_wrong_secret_is_unknown() {
            let f = fixture();
            let (_, key) = key(&f);
            let (_, stranger) = new_token();
            assert_eq!(
                auth(&f, &format!("Bearer {stranger}")),
                Err(AuthError::Unknown)
            );
            let (_, other) = new_token();
            let forged = format!("{}{}", &key.token[..21], &other[21..]);
            assert_eq!(
                auth(&f, &format!("Bearer {forged}")),
                Err(AuthError::Unknown)
            );
            f.tenants.revoke_key(Actor::Cli, key.id).expect("revoked");
            assert_eq!(
                auth(&f, &format!("Bearer {forged}")),
                Err(AuthError::Unknown),
                "a wrong secret learns nothing about revocation"
            );
        }

        #[test]
        fn a_revoked_key_is_refused_on_its_next_call() {
            let f = fixture();
            let (_, key) = key(&f);
            let header = format!("Bearer {}", key.token);
            assert!(auth(&f, &header).is_ok());
            f.tenants.revoke_key(Actor::Cli, key.id).expect("revoked");
            assert_eq!(auth(&f, &header), Err(AuthError::Revoked));
        }

        #[test]
        fn a_key_table_that_cannot_be_read_lets_nothing_through() {
            let f = fixture();
            let (_, key) = key(&f);
            let header = format!("Bearer {}", key.token);
            f.conn()
                .execute(
                    "UPDATE api_key SET scopes = 'owner' WHERE id = ?1",
                    [key.id.bytes()],
                )
                .expect("damage");
            assert_eq!(auth(&f, &header), Err(AuthError::Io), "damaged scopes");
            f.conn()
                .execute_batch("ALTER TABLE api_key RENAME TO gone")
                .expect("lose the table");
            assert_eq!(auth(&f, &header), Err(AuthError::Io), "no table");
        }
    }
}
