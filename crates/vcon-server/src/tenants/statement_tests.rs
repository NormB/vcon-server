//! server.sqlite's connection keeps the statements it prepares: every
//! request reads its key there, so a kept statement is parsed once, not
//! once per request.

use crate::access::{self, AuthError, Scope, Scopes};
use crate::audit::Actor;
use crate::catalog::testing::{keeps, prepared, reprepared};
use crate::tenants::NewKey;
use crate::tenants::tests::{Fixture, fixture};

fn key(f: &Fixture) -> NewKey {
    let tenant = f.tenants.create(Actor::Cli, "acme", None).expect("made");
    f.tenants
        .create_key(
            Actor::Cli,
            Some(tenant),
            Scopes::of(&[Scope::Read]),
            "reader",
        )
        .expect("made")
}

fn auth(f: &Fixture, token: &str) -> Result<access::Caller, AuthError> {
    access::authenticate(&f.tenants, Some(&format!("Bearer {token}")))
}

/// A second and a third authentication find the statements the first
/// left, and add none.
#[test]
fn an_authentication_reuses_the_statements_of_the_one_before() {
    let f = fixture();
    let made = key(&f);
    auth(&f, &made.token).expect("first");
    let after_one = prepared(&f.conn());
    for sql in ["BEGIN DEFERRED", "SELECT id, tenant, token_hash, scopes"] {
        assert!(
            keeps(&f.conn(), sql),
            "the authentication keeps `{sql}` prepared: {after_one:?}"
        );
    }
    auth(&f, &made.token).expect("second");
    auth(&f, &made.token).expect("third");
    assert_eq!(
        prepared(&f.conn()),
        after_one,
        "later authentications add no statement"
    );
}

/// A wrong secret is refused, and the right one is taken after it, through
/// the same kept statements.
#[test]
fn a_refused_secret_leaves_the_kept_statements_working() {
    let f = fixture();
    let made = key(&f);
    auth(&f, &made.token).expect("first");
    // A character inside the secret, so the token still decodes: the last
    // one carries spare bits a decoder may refuse.
    let mut wrong: Vec<char> = made.token.chars().collect();
    let at = wrong.len() - 5;
    wrong[at] = if wrong[at] == 'A' { 'B' } else { 'A' };
    let wrong: String = wrong.into_iter().collect();
    assert_eq!(
        auth(&f, &wrong).map(|c| c.key),
        Err(AuthError::Unknown),
        "a wrong secret is unknown"
    );
    assert_eq!(
        auth(&f, &made.token).map(|c| c.key),
        Ok(made.id),
        "the right secret is still taken"
    );
}

/// Authentications by several keys bind new values to the kept
/// statements, and SQLite prepares none of them again.
#[test]
fn no_kept_statement_is_prepared_again_for_another_key() {
    let f = fixture();
    let tenant = f.tenants.create(Actor::Cli, "acme", None).expect("made");
    for i in 0..6 {
        let made = f
            .tenants
            .create_key(
                Actor::Cli,
                Some(tenant),
                Scopes::of(&[Scope::Read]),
                &format!("reader {i}"),
            )
            .expect("made");
        auth(&f, &made.token).expect("taken");
        auth(&f, &made.token).expect("taken again");
    }
    assert_eq!(
        reprepared(&f.conn()),
        Vec::<(String, i32)>::new(),
        "statements SQLite prepared again, with how many times"
    );
}

/// A kept statement reads the key table as it is now: a key revoked
/// between two authentications is refused at once.
#[test]
fn a_kept_statement_sees_a_revocation_at_once() {
    let f = fixture();
    let made = key(&f);
    auth(&f, &made.token).expect("before the revocation");
    f.tenants.revoke_key(Actor::Cli, made.id).expect("revoked");
    assert_eq!(
        auth(&f, &made.token).map(|c| c.key),
        Err(AuthError::Revoked),
        "the revoked key is refused on its next use"
    );
}
