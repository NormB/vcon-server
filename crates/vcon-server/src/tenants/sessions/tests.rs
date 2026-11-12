// Sessions in the registry, by kind: what must work, what must be refused,
// and the limits and odd inputs. A session is started here through
// `Tenants::start_session`, the seam a sign-in method will call.

use serde_json::{Value, json};
use time::OffsetDateTime;
use time::macros::datetime;

use super::{Proof, SessionError, SessionInfo, Started};
use crate::access::{Scope, Scopes};
use crate::audit::Actor;
use crate::audit::testing::exported;
use crate::ids::{TenantId, UserId};
use crate::roles::{Role, Roles};
use crate::session::{csrf_token, hash, new_secret, well_formed};
use crate::tenants::tests::{Fixture, fixture};
use crate::tenants::{Removal, TenantError};

const CLI: Actor = Actor::Cli;

/// The fixture's clock starts here.
const START: OffsetDateTime = datetime!(2026-10-23 12:00 UTC);

fn tenant(f: &Fixture, name: &str) -> TenantId {
    f.tenants.create(CLI, name, None).expect("made")
}

fn user(f: &Fixture, label: &str) -> UserId {
    f.tenants.add_user(CLI, label, None).expect("a local user")
}

fn grant(f: &Fixture, user: UserId, tenant: Option<TenantId>, roles: &[Role]) {
    f.tenants
        .grant_membership(CLI, user, tenant, Roles::of(roles))
        .expect("granted");
}

/// A tenant, and a user who reads and writes in it.
fn member(f: &Fixture) -> (TenantId, UserId) {
    let (t, u) = (tenant(f, "acme"), user(f, "Member"));
    grant(f, u, Some(t), &[Role::Reader, Role::Producer]);
    (t, u)
}

fn start(f: &Fixture, user: UserId, tenant: Option<TenantId>) -> Started {
    f.tenants
        .start_session(user, tenant, &["hwk"])
        .expect("a session")
}

/// The session a request with the cookie and the CSRF header reaches.
fn asked(f: &Fixture, s: &Started) -> Result<SessionInfo, SessionError> {
    f.tenants.session(&s.id, Proof::Csrf(Some(&s.csrf)))
}

// The statements here go through the fixture: only the backend speaks SQL
// (tests/storage_seam.rs), and this file is not inside a test module that
// rule can see.
fn rows(f: &Fixture) -> i64 {
    f.count("SELECT count(*) FROM session")
}

/// The records of `action` in a log: actor, target and status.
fn records(log: &[Value], action: &str) -> Vec<(Value, Value, Value)> {
    log.iter()
        .filter(|r| r["action"] == action)
        .map(|r| (r["actor"].clone(), r["target"].clone(), r["status"].clone()))
        .collect()
}

fn admin(f: &Fixture, action: &str) -> Vec<(Value, Value, Value)> {
    records(&exported(&f.conn(), None), action)
}

fn in_tenant(f: &Fixture, tenant: TenantId, action: &str) -> Vec<(Value, Value, Value)> {
    records(&exported(&f.tenant_catalog(tenant), None), action)
}

mod works {
    use super::*;

    #[test]
    fn a_session_starts_with_a_new_id_its_token_and_what_the_membership_gives() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        assert!(well_formed(&s.id), "a 256-bit id");
        assert_eq!(s.csrf, csrf_token(&s.id), "the token of that id");
        assert_eq!(
            s.session,
            SessionInfo {
                user: u,
                tenant: Some(t),
                scopes: Scopes::of(&[Scope::Read, Scope::Write]),
                amr: "hwk".to_owned(),
                created_at: START,
                last_seen_at: START,
                idle_expires_at: datetime!(2026-10-23 12:15 UTC),
                expires_at: datetime!(2026-10-23 20:00 UTC),
            },
            "15 minutes idle and 8 hours in all unless the tenant set less"
        );
    }

    #[test]
    fn each_start_gives_another_id() {
        let f = fixture();
        let (t, u) = member(&f);
        let (one, two) = (start(&f, u, Some(t)), start(&f, u, Some(t)));
        assert_ne!(one.id, two.id, "an id is never given twice");
        assert_ne!(one.csrf, two.csrf, "nor a token");
        assert_eq!(rows(&f), 2, "both are live");
    }

    #[test]
    fn a_started_session_is_shown_for_debugging_without_its_id_and_token() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let shown = format!("{s:?}");
        assert!(
            !shown.contains(&s.id) && !shown.contains(&s.csrf),
            "a log line or a failed assertion that prints it shows neither"
        );
        assert!(
            shown.contains("Started") && shown.contains("hwk"),
            "the rest of it is shown"
        );
    }

    #[test]
    fn the_table_keeps_the_hashes_and_neither_the_id_nor_the_token() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        // The row is read as the server reads it, by the hash of the id.
        let row = f
            .tenants
            .registry
            .snapshot()
            .expect("a transaction")
            .session(&hash(&s.id))
            .expect("reads")
            .expect("found by the SHA-256 of the id");
        assert_eq!(row.id_hash, hash(&s.id), "SHA-256 of the id");
        assert_eq!(row.csrf_hash, hash(&s.csrf), "SHA-256 of the token");
        let texts = [row.scopes.as_bytes(), row.amr.as_bytes()];
        let blobs = [row.id_hash.as_slice(), row.csrf_hash.as_slice()];
        for kept in texts.into_iter().chain(blobs) {
            assert!(
                kept != s.id.as_bytes() && kept != s.csrf.as_bytes(),
                "no column holds the id or the token"
            );
        }
        assert_eq!(rows(&f), 1, "the one row");
    }

    #[test]
    fn a_request_with_the_token_reaches_the_session_and_moves_its_idle_limit() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.clock.set(datetime!(2026-10-23 12:10 UTC));
        let seen = asked(&f, &s).expect("the session");
        assert_eq!(
            seen.last_seen_at,
            datetime!(2026-10-23 12:10 UTC),
            "seen now"
        );
        assert_eq!(
            seen.idle_expires_at,
            datetime!(2026-10-23 12:25 UTC),
            "15 minutes from this request"
        );
        assert_eq!(
            seen.expires_at, s.session.expires_at,
            "the end does not move"
        );
        assert_eq!(seen.created_at, START, "nor the start");
        f.clock.set(datetime!(2026-10-23 12:24 UTC));
        assert!(
            asked(&f, &s).is_ok(),
            "past the first limit, within the second"
        );
    }

    #[test]
    fn the_hand_out_reaches_the_session_with_no_token() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let seen = f
            .tenants
            .session(&s.id, Proof::Handout)
            .expect("the session");
        assert_eq!(seen, s.session, "the session as it started");
    }

    #[test]
    fn a_session_takes_its_tenant_s_limits_when_it_starts() {
        let f = fixture();
        let (t, u) = member(&f);
        let limits = json!({ "session.idle_seconds": 60, "session.absolute_seconds": 300 });
        f.tenants.set_settings(CLI, t, &limits).expect("set");
        let s = start(&f, u, Some(t));
        assert_eq!(
            (s.session.idle_expires_at, s.session.expires_at),
            (
                datetime!(2026-10-23 12:01 UTC),
                datetime!(2026-10-23 12:05 UTC)
            ),
            "one minute idle, five in all"
        );
    }

    #[test]
    fn an_operator_s_session_has_no_tenant_the_admin_scope_and_the_defaults() {
        let f = fixture();
        let u = user(&f, "Operator");
        grant(&f, u, None, &[Role::Operator]);
        let s = start(&f, u, None);
        assert_eq!(s.session.tenant, None, "the operator plane");
        assert_eq!(
            s.session.scopes,
            Role::Operator.scopes(),
            "the role's scopes"
        );
        assert_eq!(
            (s.session.idle_expires_at, s.session.expires_at),
            (
                datetime!(2026-10-23 12:15 UTC),
                datetime!(2026-10-23 20:00 UTC)
            ),
            "the defaults"
        );
    }

    #[test]
    fn a_session_s_scopes_are_those_of_its_roles_under_the_tenant_s_cap() {
        let f = fixture();
        let (t, u) = member(&f);
        f.tenants
            .set_role_cap(CLI, t, Some(Roles::of(&[Role::Reader])))
            .expect("capped");
        let s = start(&f, u, Some(t));
        assert_eq!(
            s.session.scopes,
            Scopes::of(&[Scope::Read]),
            "producer is capped"
        );
    }

    #[test]
    fn ending_a_session_removes_it_and_answers_what_it_was() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let ended = f.tenants.end_session(&s.id).expect("ended");
        assert_eq!((ended.user, ended.tenant), (u, Some(t)), "whose it was");
        assert_eq!(rows(&f), 0, "the row is gone");
    }

    #[test]
    fn a_start_is_recorded_under_the_user_in_the_admin_log_and_the_tenant_s() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let record = (
            json!(format!("user:{u}")),
            json!(format!("{u}/{t}")),
            json!(201),
        );
        assert_eq!(
            admin(&f, "session_start"),
            std::slice::from_ref(&record),
            "the admin log"
        );
        assert_eq!(
            in_tenant(&f, t, "session_start"),
            [record],
            "the tenant's copy"
        );
        let log = serde_json::to_string(&exported(&f.conn(), None)).expect("text");
        assert!(!log.contains(&s.id), "the id is in no record");
        assert!(!log.contains(&s.csrf), "nor the token");
    }

    #[test]
    fn an_end_is_recorded_under_the_user_in_the_admin_log_and_the_tenant_s() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.tenants.end_session(&s.id).expect("ended");
        let record = (
            json!(format!("user:{u}")),
            json!(format!("{u}/{t}")),
            json!(204),
        );
        assert_eq!(
            admin(&f, "session_end"),
            std::slice::from_ref(&record),
            "the admin log"
        );
        assert_eq!(
            in_tenant(&f, t, "session_end"),
            [record],
            "the tenant's copy"
        );
    }

    #[test]
    fn an_operator_s_start_is_recorded_in_the_admin_log_alone() {
        let f = fixture();
        let t = tenant(&f, "acme");
        let u = user(&f, "Operator");
        grant(&f, u, None, &[Role::Operator]);
        start(&f, u, None);
        assert_eq!(
            admin(&f, "session_start"),
            [(
                json!(format!("user:{u}")),
                json!(format!("{u}/operator")),
                json!(201)
            )],
            "the admin log"
        );
        assert_eq!(in_tenant(&f, t, "session_start"), [], "no tenant's log");
    }

    #[test]
    fn a_step_up_gives_a_new_id_and_token_and_keeps_the_start_and_the_end() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.clock.set(datetime!(2026-10-23 12:10 UTC));
        let stepped = f
            .tenants
            .step_up_session(&s.id, &["hwk", "mfa"])
            .expect("stepped up");
        assert_ne!(stepped.id, s.id, "a new id");
        assert_eq!(stepped.csrf, csrf_token(&stepped.id), "and its token");
        assert_eq!(stepped.session.amr, "hwk mfa", "what the step-up showed");
        assert_eq!(
            stepped.session.created_at, START,
            "the same session's start"
        );
        assert_eq!(stepped.session.expires_at, s.session.expires_at, "and end");
        assert_eq!(
            stepped.session.idle_expires_at,
            datetime!(2026-10-23 12:25 UTC),
            "seen at the step-up"
        );
        assert_eq!(
            asked(&f, &s),
            Err(SessionError::NoSession),
            "the old id is gone"
        );
        assert!(asked(&f, &stepped).is_ok(), "the new one is live");
        assert_eq!(rows(&f), 1, "one session, not two");
    }

    #[test]
    fn ending_one_of_a_user_s_sessions_leaves_the_other() {
        let f = fixture();
        let (t, u) = member(&f);
        let (one, two) = (start(&f, u, Some(t)), start(&f, u, Some(t)));
        f.tenants.end_session(&one.id).expect("ended");
        assert_eq!(asked(&f, &one), Err(SessionError::NoSession), "ended");
        assert!(asked(&f, &two).is_ok(), "the other browser's stays");
    }

    #[test]
    fn a_session_is_there_after_a_restart() {
        let mut f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.restart();
        assert_eq!(
            asked(&f, &s),
            Ok(s.session.clone()),
            "kept in server.sqlite"
        );
    }

    #[test]
    fn a_start_removes_the_sessions_that_have_ended() {
        let f = fixture();
        let (t, u) = member(&f);
        start(&f, u, Some(t));
        start(&f, u, Some(t));
        f.clock.set(datetime!(2026-10-23 12:15 UTC));
        start(&f, u, Some(t));
        assert_eq!(rows(&f), 1, "the two that passed their idle limit are gone");
    }
}

mod refused {
    use super::*;

    fn refused_start(f: &Fixture, user: UserId, tenant: Option<TenantId>) -> SessionError {
        f.tenants
            .start_session(user, tenant, &["hwk"])
            .expect_err("refused")
    }

    #[test]
    fn a_user_no_one_made_gets_no_session_and_the_refusal_is_recorded() {
        let f = fixture();
        let t = tenant(&f, "acme");
        let nobody = UserId([9; 16]);
        assert_eq!(
            refused_start(&f, nobody, Some(t)),
            SessionError::NoUser,
            "no such user"
        );
        assert_eq!(rows(&f), 0, "nothing made");
        assert_eq!(
            admin(&f, "session_start"),
            [(
                json!(format!("user:{nobody}")),
                json!(format!("{nobody}/{t}")),
                json!(404)
            )],
            "recorded"
        );
    }

    #[test]
    fn a_deactivated_user_gets_no_session() {
        let f = fixture();
        let (t, u) = member(&f);
        f.tenants.deactivate_user(CLI, u).expect("deactivated");
        assert_eq!(
            refused_start(&f, u, Some(t)),
            SessionError::Deactivated,
            "deactivated"
        );
        assert_eq!(rows(&f), 0, "nothing made");
    }

    #[test]
    fn a_user_with_no_membership_in_the_tenant_gets_no_session_there() {
        let f = fixture();
        let (t, u) = member(&f);
        let other = tenant(&f, "other");
        assert_eq!(
            refused_start(&f, u, Some(other)),
            SessionError::NoMembership,
            "a tenant the user is not in"
        );
        assert_eq!(
            refused_start(&f, u, None),
            SessionError::NoMembership,
            "not an operator"
        );
        let outsider = user(&f, "Outsider");
        assert_eq!(
            refused_start(&f, outsider, Some(t)),
            SessionError::NoMembership,
            "a user with no membership"
        );
        assert_eq!(rows(&f), 0, "nothing made");
    }

    #[test]
    fn a_membership_whose_roles_are_all_outside_the_cap_gets_no_session() {
        let f = fixture();
        let (t, u) = member(&f);
        f.tenants
            .set_role_cap(CLI, t, Some(Roles::of(&[Role::Auditor])))
            .expect("capped");
        assert_eq!(
            refused_start(&f, u, Some(t)),
            SessionError::NoMembership,
            "no role counts under the cap"
        );
    }

    #[test]
    fn a_start_with_methods_that_cannot_be_kept_is_refused() {
        let f = fixture();
        let (t, u) = member(&f);
        for amr in [&[][..], &["two words"][..], &["hwk", "hwk"][..]] {
            assert_eq!(
                f.tenants.start_session(u, Some(t), amr).err(),
                Some(SessionError::BadAmr),
                "{amr:?}"
            );
        }
        assert_eq!(rows(&f), 0, "nothing made");
    }

    #[test]
    fn an_id_no_session_has_reaches_none() {
        let f = fixture();
        let (t, u) = member(&f);
        start(&f, u, Some(t));
        let unknown = new_secret();
        let ids = [unknown.as_str(), "", "short", "not/base64url"];
        for (place, id) in ids.into_iter().enumerate() {
            assert_eq!(
                f.tenants.session(id, Proof::Handout),
                Err(SessionError::NoSession),
                "id {place}"
            );
        }
    }

    #[test]
    fn a_session_not_seen_for_its_idle_limit_has_ended_and_its_row_is_removed() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.clock.set(datetime!(2026-10-23 12:15 UTC));
        assert_eq!(asked(&f, &s), Err(SessionError::NoSession), "at the limit");
        assert_eq!(rows(&f), 0, "removed when found ended");
    }

    #[test]
    fn a_session_in_use_ends_at_its_absolute_limit_all_the_same() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let mut at = START;
        while at + time::Duration::minutes(10) < s.session.expires_at {
            at += time::Duration::minutes(10);
            f.clock.set(at);
            assert!(asked(&f, &s).is_ok(), "in use at {at}");
        }
        f.clock.set(datetime!(2026-10-23 20:00 UTC));
        assert_eq!(asked(&f, &s), Err(SessionError::NoSession), "8 hours on");
        assert_eq!(rows(&f), 0, "removed");
    }

    #[test]
    fn a_request_with_no_token_or_a_wrong_one_is_refused() {
        let f = fixture();
        let (t, u) = member(&f);
        let (s, other) = (start(&f, u, Some(t)), start(&f, u, Some(t)));
        let tokens = [
            None,
            Some(""),
            Some(other.csrf.as_str()),
            Some(s.id.as_str()),
            Some(&s.csrf[..42]),
        ];
        for (place, token) in tokens.into_iter().enumerate() {
            assert_eq!(
                f.tenants.session(&s.id, Proof::Csrf(token)),
                Err(SessionError::BadCsrf),
                "token {place}"
            );
        }
        assert_eq!(rows(&f), 2, "a refused token ends nothing");
    }

    #[test]
    fn a_request_refused_for_its_token_does_not_move_the_idle_limit() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.clock.set(datetime!(2026-10-23 12:10 UTC));
        assert_eq!(
            f.tenants.session(&s.id, Proof::Csrf(None)),
            Err(SessionError::BadCsrf),
            "refused for its token"
        );
        f.clock.set(datetime!(2026-10-23 12:15 UTC));
        assert_eq!(
            asked(&f, &s),
            Err(SessionError::NoSession),
            "idle since 12:00"
        );
    }

    #[test]
    fn a_cookie_replayed_after_the_session_ended_reaches_nothing() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.tenants.end_session(&s.id).expect("ended");
        assert_eq!(
            asked(&f, &s),
            Err(SessionError::NoSession),
            "with its token"
        );
        assert_eq!(
            f.tenants.session(&s.id, Proof::Handout),
            Err(SessionError::NoSession),
            "the hand-out"
        );
        assert_eq!(
            f.tenants.end_session(&s.id).err(),
            Some(SessionError::NoSession),
            "ending it again"
        );
    }

    #[test]
    fn a_grant_of_a_role_ends_the_user_s_sessions() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        grant(&f, u, Some(t), &[Role::Auditor]);
        assert_eq!(
            asked(&f, &s),
            Err(SessionError::NoSession),
            "ended by the grant"
        );
        let again = start(&f, u, Some(t));
        assert!(
            again.session.scopes.contains(Scope::Audit),
            "a new one has the role"
        );
    }

    #[test]
    fn taking_a_role_ends_the_user_s_sessions() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.tenants
            .revoke_membership(CLI, u, Some(t), Some(Roles::of(&[Role::Producer])))
            .expect("taken");
        assert_eq!(asked(&f, &s), Err(SessionError::NoSession), "ended");
        assert_eq!(rows(&f), 0, "the row is gone");
    }

    #[test]
    fn removing_a_membership_ends_the_user_s_sessions() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.tenants
            .revoke_membership(CLI, u, Some(t), None)
            .expect("removed");
        assert_eq!(asked(&f, &s), Err(SessionError::NoSession), "ended");
    }

    #[test]
    fn deactivating_a_user_ends_its_sessions() {
        let f = fixture();
        let (t, u) = member(&f);
        grant(&f, u, None, &[Role::Operator]);
        let (in_tenant, as_operator) = (start(&f, u, Some(t)), start(&f, u, None));
        f.tenants.deactivate_user(CLI, u).expect("deactivated");
        assert_eq!(
            asked(&f, &in_tenant),
            Err(SessionError::NoSession),
            "the tenant's"
        );
        assert_eq!(
            asked(&f, &as_operator),
            Err(SessionError::NoSession),
            "the operator's"
        );
        assert_eq!(rows(&f), 0, "no row is left");
    }

    #[test]
    fn a_change_of_a_tenant_s_role_cap_ends_the_sessions_in_it() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.tenants
            .set_role_cap(CLI, t, Some(Roles::of(&[Role::Reader])))
            .expect("capped");
        assert_eq!(
            asked(&f, &s),
            Err(SessionError::NoSession),
            "ended by the cap"
        );
    }

    #[test]
    fn removing_a_tenant_ends_the_sessions_in_it() {
        let f = fixture();
        let (t, u) = member(&f);
        start(&f, u, Some(t));
        f.tenants.remove(CLI, t, Removal::Empty).expect("removed");
        assert_eq!(rows(&f), 0, "no session outlives its tenant");
    }

    #[test]
    fn a_start_whose_record_cannot_be_written_makes_no_session() {
        let f = fixture();
        let (t, u) = member(&f);
        f.tenants.break_audit();
        assert_eq!(
            refused_start(&f, u, Some(t)),
            SessionError::Registry(TenantError::AuditFailed),
            "the record failed"
        );
        assert_eq!(rows(&f), 0, "nothing made");
    }

    #[test]
    fn an_end_whose_record_cannot_be_written_leaves_the_session() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.tenants.break_audit();
        assert_eq!(
            f.tenants.end_session(&s.id).err(),
            Some(SessionError::Registry(TenantError::AuditFailed)),
            "the record failed"
        );
        assert_eq!(rows(&f), 1, "still there");
    }

    #[test]
    fn a_step_up_of_a_session_that_has_ended_is_refused() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        assert_eq!(
            f.tenants.step_up_session(&new_secret(), &["mfa"]).err(),
            Some(SessionError::NoSession),
            "an unknown id"
        );
        assert_eq!(
            f.tenants.step_up_session(&s.id, &[]).err(),
            Some(SessionError::BadAmr),
            "no method"
        );
        f.clock.set(datetime!(2026-10-23 12:15 UTC));
        assert_eq!(
            f.tenants.step_up_session(&s.id, &["mfa"]).err(),
            Some(SessionError::NoSession),
            "idle too long"
        );
    }
}

mod limits {
    use super::*;

    #[test]
    fn a_session_is_live_one_second_before_its_idle_limit() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.clock.set(datetime!(2026-10-23 12:14:59 UTC));
        assert!(asked(&f, &s).is_ok(), "14 minutes 59 seconds idle");
    }

    #[test]
    fn a_session_is_live_one_second_before_its_absolute_limit() {
        let f = fixture();
        let (t, u) = member(&f);
        let limits = json!({ "session.idle_seconds": 300, "session.absolute_seconds": 300 });
        f.tenants.set_settings(CLI, t, &limits).expect("set");
        let s = start(&f, u, Some(t));
        f.clock.set(datetime!(2026-10-23 12:04:59 UTC));
        let seen = asked(&f, &s).expect("live");
        assert_eq!(
            seen.idle_expires_at, seen.expires_at,
            "the idle limit is never shown past the end"
        );
        f.clock.set(datetime!(2026-10-23 12:05 UTC));
        assert_eq!(asked(&f, &s), Err(SessionError::NoSession), "at the end");
    }

    #[test]
    fn a_change_to_another_user_leaves_this_user_s_sessions() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let other = user(&f, "Other");
        grant(&f, other, Some(t), &[Role::Reader]);
        f.tenants
            .revoke_membership(CLI, other, Some(t), None)
            .expect("removed");
        f.tenants.deactivate_user(CLI, other).expect("deactivated");
        assert!(asked(&f, &s).is_ok(), "untouched");
    }

    #[test]
    fn a_refused_change_ends_no_session() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let key = Actor::Key(crate::ids::KeyId([7; 8]));
        assert!(
            f.tenants
                .grant_membership(key, u, Some(t), Roles::of(&[Role::Auditor]))
                .is_err(),
            "a key may not grant"
        );
        assert!(
            f.tenants
                .revoke_membership(CLI, u, Some(t), Some(Roles::default()))
                .is_err(),
            "no roles named"
        );
        assert!(f.tenants.deactivate_user(key, u).is_err(), "a key may not");
        assert!(
            f.tenants
                .set_role_cap(CLI, t, Some(Roles::of(&[Role::Operator])))
                .is_err(),
            "not a cap"
        );
        assert!(asked(&f, &s).is_ok(), "the session is live");
    }

    #[test]
    fn a_change_of_the_limits_reaches_new_sessions_and_not_live_ones() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let limits = json!({ "session.idle_seconds": 60 });
        f.tenants.set_settings(CLI, t, &limits).expect("set");
        f.clock.set(datetime!(2026-10-23 12:05 UTC));
        let seen = asked(&f, &s).expect("live under the limit it started with");
        assert_eq!(
            seen.idle_expires_at,
            datetime!(2026-10-23 12:20 UTC),
            "15 minutes"
        );
        let new = start(&f, u, Some(t));
        assert_eq!(
            new.session.idle_expires_at,
            datetime!(2026-10-23 12:06 UTC),
            "one minute"
        );
    }

    #[test]
    fn a_role_change_in_one_tenant_ends_the_user_s_sessions_in_every_tenant() {
        let f = fixture();
        let (t, u) = member(&f);
        let other = tenant(&f, "other");
        grant(&f, u, Some(other), &[Role::Reader]);
        let (here, there) = (start(&f, u, Some(t)), start(&f, u, Some(other)));
        grant(&f, u, Some(t), &[Role::Auditor]);
        assert_eq!(
            asked(&f, &here),
            Err(SessionError::NoSession),
            "where it changed"
        );
        assert_eq!(
            asked(&f, &there),
            Err(SessionError::NoSession),
            "and elsewhere"
        );
    }

    #[test]
    fn a_cap_change_in_one_tenant_leaves_other_tenants_and_operators_sessions() {
        let f = fixture();
        let (t, u) = member(&f);
        let other = tenant(&f, "other");
        grant(&f, u, Some(other), &[Role::Reader]);
        let operator = user(&f, "Operator");
        grant(&f, operator, None, &[Role::Operator]);
        let (there, op) = (start(&f, u, Some(other)), start(&f, operator, None));
        f.tenants.set_role_cap(CLI, t, None).expect("set");
        assert!(asked(&f, &there).is_ok(), "another tenant's");
        assert!(asked(&f, &op).is_ok(), "an operator's");
    }

    #[test]
    fn the_token_of_the_id_a_step_up_replaced_is_refused_with_the_new_id() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let stepped = f
            .tenants
            .step_up_session(&s.id, &["mfa"])
            .expect("stepped up");
        assert_eq!(
            f.tenants.session(&stepped.id, Proof::Csrf(Some(&s.csrf))),
            Err(SessionError::BadCsrf),
            "the old token"
        );
    }

    #[test]
    fn an_idle_limit_longer_than_what_is_left_is_shown_as_the_end() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        let mut at = START;
        while at + time::Duration::minutes(10) < datetime!(2026-10-23 19:50 UTC) {
            at += time::Duration::minutes(10);
            f.clock.set(at);
            asked(&f, &s).expect("in use");
        }
        f.clock.set(datetime!(2026-10-23 19:50 UTC));
        let seen = asked(&f, &s).expect("live");
        assert_eq!(
            seen.idle_expires_at, seen.expires_at,
            "10 minutes are left, not 15"
        );
    }

    #[test]
    fn a_session_shows_as_json_its_times_in_rfc_3339_and_the_token_given() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        assert_eq!(
            s.session.to_json(&s.csrf),
            Some(json!({
                "user": u.to_string(),
                "tenant": t.to_string(),
                "scopes": Scopes::of(&[Scope::Read, Scope::Write]).names(),
                "amr": ["hwk"],
                "created_at": "2026-10-23T12:00:00Z",
                "last_seen_at": "2026-10-23T12:00:00Z",
                "idle_expires_at": "2026-10-23T12:15:00Z",
                "expires_at": "2026-10-23T20:00:00Z",
                "csrf_token": s.csrf,
            })),
            "what GET /session answers"
        );
    }

    #[test]
    fn an_operator_s_session_shows_null_for_its_tenant() {
        let f = fixture();
        let u = user(&f, "Operator");
        grant(&f, u, None, &[Role::Operator]);
        let s = start(&f, u, None);
        let shown = s.session.to_json(&s.csrf).expect("JSON");
        assert_eq!(shown["tenant"], Value::Null, "no tenant");
    }

    #[test]
    fn stored_scopes_that_do_not_read_back_are_damage_not_no_scopes() {
        let f = fixture();
        let (t, u) = member(&f);
        let s = start(&f, u, Some(t));
        f.conn()
            .execute_batch("UPDATE session SET scopes = 'everything'")
            .expect("damaged");
        assert_eq!(
            asked(&f, &s),
            Err(SessionError::Registry(TenantError::Io)),
            "a failure, not a session with no scope"
        );
    }
}
