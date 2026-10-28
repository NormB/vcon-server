//! Turns the bearer token on a request into a caller, or refuses it.

use std::fmt;

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
    /// Reads the tenant's own audit log.
    Audit,
    Admin,
}

impl Scope {
    /// Every scope, in the order a key's scopes are written.
    const ALL: [Scope; 6] = [
        Scope::Write,
        Scope::Read,
        Scope::Delete,
        Scope::Legal,
        Scope::Audit,
        Scope::Admin,
    ];

    /// The scope with this exact name.
    pub fn named(name: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|s| s.name() == name)
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }

    fn name(self) -> &'static str {
        match self {
            Scope::Write => "write",
            Scope::Read => "read",
            Scope::Delete => "delete",
            Scope::Legal => "legal",
            Scope::Audit => "audit",
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

    /// The scopes a list names, or None when one is unknown or named twice,
    /// so a list means one thing. Which scopes a key may hold is Tenants'
    /// rule.
    pub fn named(names: &[String]) -> Option<Scopes> {
        let mut held = Vec::new();
        for name in names {
            let scope = Scope::named(name)?;
            if held.contains(&scope) {
                return None;
            }
            held.push(scope);
        }
        Some(Scopes::of(&held))
    }

    /// Reads scopes written by `Display`: names in the fixed order, comma
    /// separated, each once.
    pub fn parse(text: &str) -> Option<Scopes> {
        let scopes: Vec<Scope> = text.split(',').map(Scope::named).collect::<Option<_>>()?;
        let read = Scopes::of(&scopes);
        (read.to_string() == text).then_some(read)
    }
}

impl Scopes {
    /// The names of the scopes held, in the fixed order of `Scope::ALL`,
    /// as the API and the command line list them.
    pub fn names(self) -> Vec<&'static str> {
        Scope::ALL
            .into_iter()
            .filter(|s| self.contains(*s))
            .map(Scope::name)
            .collect()
    }
}

impl std::fmt::Display for Scopes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.names().join(","))
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
    /// The key does not hold the scope the request needs.
    Forbidden(Scope),
    /// Not `Bearer` and a token as `new_token` writes one.
    Malformed,
    /// No key has this id, or the secret is not its own.
    Unknown,
    /// The key was revoked.
    Revoked,
    /// The key table could not be read, so nothing is let through.
    Io,
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::Missing => f.write_str("no Authorization header"),
            AuthError::Forbidden(scope) => {
                write!(f, "the key does not hold the {} scope", scope.name())
            }
            AuthError::Malformed => {
                f.write_str("the Authorization header is not Bearer and a token")
            }
            AuthError::Unknown => f.write_str("no key has this id and secret"),
            AuthError::Revoked => f.write_str("the key was revoked"),
            AuthError::Io => f.write_str("the key table could not be read"),
        }
    }
}

impl std::error::Error for AuthError {}

/// The caller a request's Authorization header names. The key is looked up
/// on every call, with no cache, so a revoked key fails on its next
/// request.
///
/// A failed check is counted in the audit (`Tenants::count_auth_failure`),
/// except when the key table could not be read.
pub fn authenticate(keys: &Tenants, authorization: Option<&str>) -> Result<Caller, AuthError> {
    match check(keys, authorization) {
        Ok(caller) => Ok(caller),
        Err((error, known)) => {
            if error != AuthError::Io {
                keys.count_auth_failure(known);
            }
            Err(error)
        }
    }
}

/// A failed check, with the key it named if that key is known.
type Failure = (AuthError, Option<(KeyId, Option<TenantId>)>);

fn check(keys: &Tenants, authorization: Option<&str>) -> Result<Caller, Failure> {
    let unknown = |error| (error, None);
    let header = authorization.ok_or(unknown(AuthError::Missing))?;
    let token = bearer(header).ok_or(unknown(AuthError::Malformed))?;
    let id = parse(token).map_err(unknown)?;
    let key = keys
        .stored_key(id)
        .map_err(|_| unknown(AuthError::Io))?
        .ok_or(unknown(AuthError::Unknown))?;
    let known = |error| (error, Some((id, key.tenant)));
    // The secret is checked before revocation, so a wrong secret learns
    // nothing about the key.
    constant_time::verify_slices_are_equal(&token_hash(token), &key.hash)
        .map_err(|_| known(AuthError::Unknown))?;
    if key.revoked {
        return Err(known(AuthError::Revoked));
    }
    Ok(Caller {
        key: id,
        tenant: key.tenant,
        scopes: key.scopes,
    })
}

/// Refuses a caller whose key does not hold `scope`. Admin belongs only to
/// an operator key, and the vCon, legal and audit scopes only to a tenant
/// key, so a key holding a scope of the other kind is still refused it.
pub fn require(caller: &Caller, scope: Scope) -> Result<(), AuthError> {
    let of_its_kind = (scope == Scope::Admin) == caller.tenant.is_none();
    if of_its_kind && caller.scopes.contains(scope) {
        Ok(())
    } else {
        Err(AuthError::Forbidden(scope))
    }
}

/// The tenant a vCon or legal request acts on: the caller's own, from its
/// key, never from the request.
pub fn tenant_for(caller: &Caller, scope: Scope) -> Result<TenantId, AuthError> {
    require(caller, scope)?;
    caller.tenant.ok_or(AuthError::Forbidden(scope))
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
            assert_eq!(
                secret.len(),
                43,
                "32 bytes are 43 base64url characters without padding"
            );
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
        assert!(none.is_empty(), "no scopes given, none held");
        let scopes = Scopes::of(&[Scope::Delete, Scope::Write, Scope::Delete]);
        assert!(
            !scopes.is_empty(),
            "two scopes given, so the set is not empty"
        );
        for scope in Scope::ALL {
            assert_eq!(
                scopes.contains(scope),
                matches!(scope, Scope::Write | Scope::Delete),
                "{scope:?}"
            );
        }
        assert_eq!(
            scopes.to_string(),
            "write,delete",
            "a repeated scope is held once, and written in the fixed order"
        );
        assert_eq!(
            Scopes::of(&Scope::ALL).to_string(),
            "write,read,delete,legal,audit,admin",
            "every scope, in the fixed order"
        );
        assert_eq!(
            Scopes::of(&[Scope::Legal, Scope::Write, Scope::Read]).names(),
            ["write", "read", "legal"],
            "names come in Scope::ALL's order, not the order given"
        );
        for scope in Scope::ALL {
            let one = Scopes::of(&[scope]);
            assert_eq!(Scopes::parse(&one.to_string()), Some(one), "{scope:?}");
        }
        let all = Scopes::of(&Scope::ALL);
        assert_eq!(
            Scopes::parse(&all.to_string()),
            Some(all),
            "parse reads every scope back from its written form"
        );
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

    /// audit is a tenant key's scope: it reads the tenant's own log.
    #[test]
    fn audit_is_required_of_a_tenant_key_only() {
        let tenant = Caller {
            key: KeyId::generate(),
            tenant: Some(TenantId::generate()),
            scopes: Scopes::of(&[Scope::Audit]),
        };
        assert_eq!(
            Scope::named("audit"),
            Some(Scope::Audit),
            "the scope is named audit"
        );
        assert_eq!(
            require(&tenant, Scope::Audit),
            Ok(()),
            "a tenant key with the audit scope may read its log"
        );
        assert_eq!(
            tenant_for(&tenant, Scope::Audit),
            Ok(tenant.tenant.expect("one")),
            "and the log it reads is its own tenant's"
        );
        assert_eq!(
            require(&tenant, Scope::Read),
            Err(AuthError::Forbidden(Scope::Read)),
            "audit grants no other scope"
        );
        let operator = Caller {
            tenant: None,
            scopes: Scopes::of(&[Scope::Admin, Scope::Audit]),
            ..tenant
        };
        assert_eq!(
            require(&operator, Scope::Audit),
            Err(AuthError::Forbidden(Scope::Audit)),
            "an operator key has no tenant log to read, even with the audit scope"
        );
    }

    #[test]
    fn a_scope_is_found_by_its_exact_name() {
        for scope in Scope::ALL {
            assert_eq!(Scope::named(scope.name()), Some(scope), "{scope:?}");
        }
        for name in ["", "Read", "READ", " read", "read,write", "superuser"] {
            assert_eq!(Scope::named(name), None, "{name:?}");
        }
    }

    #[test]
    fn a_new_token_is_well_formed_and_new_each_time() {
        let (id, token) = new_token();
        assert_eq!(
            parse(&token),
            Ok(id),
            "a new token parses to the id it was made with"
        );
        let (other_id, other) = new_token();
        assert_ne!(other_id, id, "each new token has a new random key id");
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
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "the hash is plain SHA-256, as the RFC's vector shows"
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
                }),
                "an operator key's caller has no tenant"
            );
        }

        #[test]
        fn a_missing_or_malformed_header_is_refused() {
            let f = fixture();
            let (_, key) = key(&f);
            assert_eq!(
                authenticate(&f.tenants, None),
                Err(AuthError::Missing),
                "no Authorization header is told apart from a bad one"
            );
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
                Err(AuthError::Unknown),
                "a well-formed token whose key id no catalog holds"
            );
            let (_, other) = new_token();
            let forged = format!("{}{}", &key.token[..21], &other[21..]);
            assert_eq!(
                auth(&f, &format!("Bearer {forged}")),
                Err(AuthError::Unknown),
                "a real key id with another secret is answered as an unknown key"
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
            assert!(
                auth(&f, &header).is_ok(),
                "the key works before it is revoked"
            );
            f.tenants.revoke_key(Actor::Cli, key.id).expect("revoked");
            assert_eq!(
                auth(&f, &header),
                Err(AuthError::Revoked),
                "the next call after revocation is refused: no cache keeps it alive"
            );
        }

        fn admin_failures(f: &Fixture) -> Vec<serde_json::Value> {
            f.admin_records("auth_failed")
        }

        fn tenant_failures(f: &Fixture, t: TenantId) -> Vec<serde_json::Value> {
            crate::audit::testing::exported(&f.tenant_catalog(t), None)
                .into_iter()
                .filter(|r| r["action"] == "auth_failed")
                .collect()
        }

        fn forged(key: &NewKey) -> String {
            let (_, other) = new_token();
            format!("Bearer {}{}", &key.token[..21], &other[21..])
        }

        #[test]
        fn a_burst_of_bad_tokens_leaves_one_counted_record() {
            let f = fixture();
            let (t, _) = key(&f);
            for _ in 0..20 {
                assert!(auth(&f, "Bearer not-a-token").is_err(), "a malformed token");
            }
            for _ in 0..5 {
                assert!(authenticate(&f.tenants, None).is_err(), "no header");
                let (_, made_up) = new_token();
                assert!(
                    auth(&f, &format!("Bearer {made_up}")).is_err(),
                    "a key id no catalog holds"
                );
            }
            let records = admin_failures(&f);
            assert_eq!(records.len(), 1, "{records:?}");
            assert_eq!(
                records[0]["actor"], "",
                "no key is known, so no actor is named"
            );
            assert_eq!(
                records[0]["status"], 401,
                "each was refused as unauthenticated"
            );
            assert_eq!(
                records[0]["count"], 30,
                "20 malformed, 5 missing, 5 unknown"
            );
            assert_eq!(
                records[0]["at"], "2026-10-23T12:00:00Z",
                "the record is stamped with the fixture clock's minute"
            );
            assert_eq!(
                tenant_failures(&f, t),
                Vec::<serde_json::Value>::new(),
                "no failure named this tenant's key"
            );

            f.clock
                .set(time::macros::datetime!(2026-10-23 12:01:30 UTC));
            assert!(auth(&f, "Bearer not-a-token").is_err(), "a malformed token");
            let records = admin_failures(&f);
            assert_eq!(records.len(), 2, "a new minute, a new record");
            assert_eq!(
                records[1]["count"], 1,
                "the new record counts only the new failure"
            );
        }

        #[test]
        fn a_known_key_that_fails_is_counted_in_its_tenants_log() {
            let f = fixture();
            let (t, key) = key(&f);
            for _ in 0..3 {
                assert_eq!(
                    auth(&f, &forged(&key)),
                    Err(AuthError::Unknown),
                    "a wrong secret for a real key"
                );
            }
            f.tenants.revoke_key(Actor::Cli, key.id).expect("revoked");
            for _ in 0..2 {
                assert_eq!(
                    auth(&f, &format!("Bearer {}", key.token)),
                    Err(AuthError::Revoked),
                    "the right secret for a revoked key"
                );
            }
            let records = tenant_failures(&f, t);
            assert_eq!(records.len(), 1, "{records:?}");
            assert_eq!(
                records[0]["actor"],
                key.id.to_string(),
                "the failures name the key they tried"
            );
            assert_eq!(
                records[0]["status"], 401,
                "each was refused as unauthenticated"
            );
            assert_eq!(
                records[0]["count"], 5,
                "3 wrong secrets and 2 calls after revocation"
            );
            assert_eq!(
                admin_failures(&f),
                Vec::<serde_json::Value>::new(),
                "a tenant key's failures go to its tenant's log only"
            );
        }

        #[test]
        fn an_operator_key_that_fails_is_counted_in_the_admin_log() {
            let f = fixture();
            let operator = f
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made");
            assert!(auth(&f, &forged(&operator)).is_err(), "a wrong secret");
            let records = admin_failures(&f);
            assert_eq!(
                records.len(),
                1,
                "an operator key has no tenant log: {records:?}"
            );
            assert_eq!(
                records[0]["actor"],
                operator.id.to_string(),
                "the record names the operator key tried"
            );
        }

        #[test]
        fn a_known_key_whose_tenant_log_is_gone_is_counted_in_the_admin_log() {
            let f = fixture();
            let (t, key) = key(&f);
            f.tenants
                .remove(Actor::Cli, t, crate::tenants::Removal::Force)
                .expect("removed");
            assert_eq!(
                auth(&f, &format!("Bearer {}", key.token)),
                Err(AuthError::Revoked),
                "removing a tenant revokes its keys"
            );
            let records = admin_failures(&f);
            assert_eq!(records.len(), 1, "with the tenant's log gone: {records:?}");
            assert_eq!(
                records[0]["actor"],
                key.id.to_string(),
                "the record still names the key tried"
            );
        }

        #[test]
        fn a_failure_that_cannot_be_counted_is_refused_all_the_same() {
            let f = fixture();
            f.conn()
                .execute_batch(
                    "CREATE TEMP TRIGGER no_audit BEFORE INSERT ON audit
                     BEGIN SELECT RAISE(ABORT, 'no audit'); END;",
                )
                .expect("trigger");
            assert_eq!(
                auth(&f, "Bearer not-a-token"),
                Err(AuthError::Malformed),
                "a failed audit write does not turn a refusal into a pass"
            );
        }

        #[test]
        fn a_successful_check_counts_nothing() {
            let f = fixture();
            let (t, key) = key(&f);
            assert!(
                auth(&f, &format!("Bearer {}", key.token)).is_ok(),
                "the right token"
            );
            assert_eq!(
                admin_failures(&f),
                Vec::<serde_json::Value>::new(),
                "a success is not a failure to count"
            );
            assert_eq!(
                tenant_failures(&f, t),
                Vec::<serde_json::Value>::new(),
                "a success is not a failure to count"
            );
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
            assert_eq!(
                admin_failures(&f),
                Vec::<serde_json::Value>::new(),
                "an unreadable table is not a failed check"
            );
        }
    }

    mod scopes {
        use super::*;

        const TENANT: TenantId = TenantId([1; 16]);

        fn caller(tenant: Option<TenantId>, scopes: &[Scope]) -> Caller {
            Caller {
                key: KeyId([2; 8]),
                tenant,
                scopes: Scopes::of(scopes),
            }
        }

        /// Which scopes each key may use: (key, scopes it allows).
        #[test]
        fn each_key_may_use_only_the_scopes_it_holds_of_its_kind() {
            let vcon = [Scope::Write, Scope::Read, Scope::Delete, Scope::Legal];
            let matrix = [
                (caller(Some(TENANT), &[Scope::Write]), vec![Scope::Write]),
                (
                    caller(Some(TENANT), &[Scope::Read, Scope::Delete]),
                    vec![Scope::Read, Scope::Delete],
                ),
                (caller(Some(TENANT), &[Scope::Legal]), vec![Scope::Legal]),
                (caller(Some(TENANT), &vcon), vcon.to_vec()),
                (caller(None, &[Scope::Admin]), vec![Scope::Admin]),
                // A key of one kind holding a scope of the other, as only a
                // damaged table could, is refused it.
                (
                    caller(Some(TENANT), &[Scope::Admin, Scope::Read]),
                    vec![Scope::Read],
                ),
                (
                    caller(None, &[Scope::Admin, Scope::Write]),
                    vec![Scope::Admin],
                ),
            ];
            for (who, allowed) in matrix {
                for scope in Scope::ALL {
                    let expected = if allowed.contains(&scope) {
                        Ok(())
                    } else {
                        Err(AuthError::Forbidden(scope))
                    };
                    assert_eq!(require(&who, scope), expected, "{who:?} {scope:?}");
                }
            }
        }

        #[test]
        fn a_request_acts_on_the_tenant_of_its_key() {
            let tenant_key = caller(Some(TENANT), &[Scope::Write]);
            assert_eq!(
                tenant_for(&tenant_key, Scope::Write),
                Ok(TENANT),
                "a tenant key acts on its own tenant"
            );
            assert_eq!(
                tenant_for(&tenant_key, Scope::Read),
                Err(AuthError::Forbidden(Scope::Read)),
                "only with a scope it holds"
            );
        }

        #[test]
        fn an_operator_key_acts_on_no_tenant() {
            let operator = caller(None, &[Scope::Admin, Scope::Write]);
            for scope in Scope::ALL {
                assert_eq!(
                    tenant_for(&operator, scope),
                    Err(AuthError::Forbidden(scope)),
                    "{scope:?}"
                );
            }
        }
    }
}
