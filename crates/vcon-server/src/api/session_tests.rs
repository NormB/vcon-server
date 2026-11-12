//! Requests by session over HTTPS: `GET /session`, `POST /session/logout`,
//! and what every other route does with a session cookie. No sign-in
//! method is built yet, so each session here is started through
//! `Tenants::start_session`, the seam a sign-in method will call.

use reqwest::Method;
use serde_json::{Value, json};

use super::tests::pipeline::{START, Server, problem, server, server_on, server_without_origin};
use crate::access::{Plane, Scope, Scopes};
use crate::audit::Actor;
use crate::ids::UserId;
use crate::roles::{Role, Roles};
use crate::session::COOKIE;
use crate::tenants::Started;

/// The clear-cookie header a refused or ended session is answered with.
const CLEARED: &str = "__Host-vcs_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0";

/// A user with the roles reader and producer in the server's tenant.
fn member(s: &Server) -> UserId {
    let user = s
        .tenants
        .add_user(Actor::Cli, "Member", None)
        .expect("a user");
    s.tenants
        .grant_membership(
            Actor::Cli,
            user,
            Some(s.tenant),
            Roles::of(&[Role::Reader, Role::Producer]),
        )
        .expect("granted");
    user
}

/// A user signed in to the server's tenant.
fn signed_in(s: &Server) -> (UserId, Started) {
    let user = member(s);
    let started = s
        .tenants
        .start_session(user, Some(s.tenant), &["hwk"])
        .expect("a session");
    (user, started)
}

/// An operator signed in.
fn operator_signed_in(s: &Server) -> (UserId, Started) {
    let user = s
        .tenants
        .add_user(Actor::Cli, "Operator", None)
        .expect("a user");
    s.tenants
        .grant_membership(Actor::Cli, user, None, Roles::of(&[Role::Operator]))
        .expect("granted");
    let started = s
        .tenants
        .start_session(user, None, &["hwk"])
        .expect("a session");
    (user, started)
}

fn cookie_of(id: &str) -> String {
    format!("{COOKIE}={id}")
}

/// Sends a request with the headers given and no others of its own.
async fn send(
    s: &Server,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
) -> reqwest::Response {
    let mut request = s.client().request(method, s.url(path));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.expect("an answer")
}

/// `GET /session` as a browser's page makes it: the cookie and
/// `Sec-Fetch-Site: same-origin`.
async fn read(s: &Server, id: &str) -> reqwest::Response {
    let cookie = cookie_of(id);
    let headers = [
        ("cookie", cookie.as_str()),
        ("sec-fetch-site", "same-origin"),
    ];
    send(s, Method::GET, "/session", &headers).await
}

/// `POST /session/logout` as a browser's page makes it: the cookie, the
/// page's origin and the CSRF token.
async fn logout(s: &Server, started: &Started) -> reqwest::Response {
    let (cookie, origin) = (cookie_of(&started.id), s.url(""));
    let headers = [
        ("cookie", cookie.as_str()),
        ("origin", origin.as_str()),
        ("x-csrf-token", started.csrf.as_str()),
    ];
    send(s, Method::POST, "/session/logout", &headers).await
}

/// The status of a refusal, the last part of its problem type, and its
/// `Set-Cookie` header.
async fn refused(answer: reqwest::Response) -> (u16, String, Option<String>) {
    let status = answer.status().as_u16();
    let set = answer
        .headers()
        .get("set-cookie")
        .map(|v| v.to_str().expect("text").to_owned());
    (status, problem(answer).await, set)
}

fn no_session(cleared: bool) -> (u16, String, Option<String>) {
    (
        401,
        "no-session".to_owned(),
        cleared.then(|| CLEARED.to_owned()),
    )
}

fn bad_origin() -> (u16, String, Option<String>) {
    (403, "bad-origin".to_owned(), None)
}

fn bad_csrf() -> (u16, String, Option<String>) {
    (403, "bad-csrf".to_owned(), None)
}

/// The records of `action` in the admin log.
fn admin_log(s: &Server, action: &str) -> Vec<Value> {
    let mut out = Vec::new();
    s.tenants.export_admin(None, &mut out).expect("export");
    String::from_utf8(out)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("JSON"))
        .filter(|r| r["action"] == action)
        .collect()
}

/// Actor, status and count of each record given.
fn counted(records: &[Value]) -> Vec<(Value, Value, Value)> {
    records
        .iter()
        .map(|r| (r["actor"].clone(), r["status"].clone(), r["count"].clone()))
        .collect()
}

mod works {
    use super::*;

    #[tokio::test]
    async fn the_hand_out_answers_the_session_and_its_csrf_token() {
        let s = server().await;
        let (user, started) = signed_in(&s);
        let answer = read(&s, &started.id).await;
        assert_eq!(answer.status(), 200, "the session's own browser");
        assert_eq!(answer.headers()["cache-control"], "no-store", "never kept");
        assert_eq!(answer.headers()["content-type"], "application/json", "JSON");
        assert!(
            answer.headers().get("set-cookie").is_none(),
            "the cookie stays"
        );
        let body: Value = answer.json().await.expect("JSON");
        assert_eq!(body["user"], user.to_string(), "who");
        assert_eq!(body["tenant"], s.tenant.to_string(), "where");
        assert_eq!(
            body["scopes"],
            json!(Scopes::of(&[Scope::Read, Scope::Write]).names()),
            "what the membership gave"
        );
        assert_eq!(body["amr"], json!(["hwk"]), "how the user signed in");
        assert_eq!(body["csrf_token"], started.csrf, "the token for the page");
        assert!(
            !body.to_string().contains(&started.id),
            "the id is not shown"
        );
    }

    #[tokio::test]
    async fn the_token_handed_out_ends_the_session_and_the_cookie_is_cleared() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let shown: Value = read(&s, &started.id).await.json().await.expect("JSON");
        let handed = Started {
            csrf: shown["csrf_token"].as_str().expect("a token").to_owned(),
            ..started.clone()
        };
        let answer = logout(&s, &handed).await;
        assert_eq!(answer.status(), 204, "signed out");
        assert_eq!(
            answer.headers()["set-cookie"],
            CLEARED,
            "the browser drops it"
        );
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "the session is gone"
        );
    }

    #[tokio::test]
    async fn a_sign_out_is_recorded_under_the_user_and_reads_are_counted() {
        let s = server().await;
        let (user, started) = signed_in(&s);
        for _ in 0..3 {
            assert_eq!(read(&s, &started.id).await.status(), 200, "a read");
        }
        assert_eq!(logout(&s, &started).await.status(), 204, "signed out");
        let actor = json!(format!("user:{user}"));
        assert_eq!(
            counted(&s.tenant_log("session_read")),
            [(actor.clone(), json!(200), json!(3))],
            "one record a minute in the tenant's log"
        );
        assert_eq!(
            counted(&s.tenant_log("session_end")),
            [(actor.clone(), json!(204), json!(1))],
            "the tenant's log"
        );
        assert_eq!(
            counted(&admin_log(&s, "session_end")),
            [(actor, json!(204), json!(1))],
            "the admin log"
        );
    }

    #[tokio::test]
    async fn an_operator_s_session_is_served_with_no_tenant() {
        let s = server().await;
        let (user, started) = operator_signed_in(&s);
        let answer = read(&s, &started.id).await;
        assert_eq!(answer.status(), 200, "the one listener serves both planes");
        let body: Value = answer.json().await.expect("JSON");
        assert_eq!(body["tenant"], Value::Null, "the operator plane");
        assert_eq!(
            body["scopes"],
            json!(Role::Operator.scopes().names()),
            "the role's scopes"
        );
        assert_eq!(logout(&s, &started).await.status(), 204, "signed out");
        assert_eq!(
            counted(&admin_log(&s, "session_read")),
            [(json!(format!("user:{user}")), json!(200), json!(1))],
            "an operator's read is counted in the admin log"
        );
    }

    #[tokio::test]
    async fn each_listener_serves_the_sessions_of_its_plane() {
        let tenant = server_on(Plane::Tenant).await;
        let (_, started) = signed_in(&tenant);
        assert_eq!(read(&tenant, &started.id).await.status(), 200, "a tenant's");
        assert_eq!(logout(&tenant, &started).await.status(), 204, "and its end");
        let operator = server_on(Plane::Operator).await;
        let (_, started) = operator_signed_in(&operator);
        assert_eq!(
            read(&operator, &started.id).await.status(),
            200,
            "an operator's"
        );
        assert_eq!(
            logout(&operator, &started).await.status(),
            204,
            "and its end"
        );
    }

    #[tokio::test]
    async fn a_read_counts_as_seen_and_moves_the_idle_limit() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        s.clock.set(START + time::Duration::minutes(10));
        let body: Value = read(&s, &started.id).await.json().await.expect("JSON");
        // The test clock moves a microsecond at each reading, so a limit
        // is the minute named and some microseconds.
        let within = |name: &str, minutes: i64| {
            let minute = (START + time::Duration::minutes(minutes))
                .format(&time::format_description::well_known::Rfc3339)
                .expect("a time");
            let shown = body[name].as_str().expect("a time");
            assert!(
                shown.starts_with(minute.trim_end_matches('Z')),
                "{name}: {shown} is not in the second of {minute}"
            );
        };
        within("idle_expires_at", 25);
        within("expires_at", 8 * 60);
        s.clock.set(START + time::Duration::minutes(24));
        assert_eq!(read(&s, &started.id).await.status(), 200, "still live");
    }

    #[tokio::test]
    async fn a_key_is_served_beside_a_cookie_of_another_name() {
        let s = server().await;
        let token = s.key(&[Scope::Read]);
        let answer = s
            .client()
            .get(s.url("/key"))
            .bearer_auth(&token)
            .header("cookie", "theme=dark; vcs_session=abc")
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 200, "only the session cookie is refused");
    }

    #[tokio::test]
    async fn the_origin_may_come_with_the_hand_out() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let (cookie, origin) = (cookie_of(&started.id), s.url(""));
        let headers = [
            ("cookie", cookie.as_str()),
            ("sec-fetch-site", "same-origin"),
            ("origin", origin.as_str()),
        ];
        let answer = send(&s, Method::GET, "/session", &headers).await;
        assert_eq!(answer.status(), 200, "a browser may send both");
    }

    #[tokio::test]
    async fn two_browsers_of_one_user_are_signed_out_one_at_a_time() {
        let s = server().await;
        let (user, one) = signed_in(&s);
        let two = s
            .tenants
            .start_session(user, Some(s.tenant), &["hwk"])
            .expect("a second session");
        assert_eq!(logout(&s, &one).await.status(), 204, "the first signs out");
        assert_eq!(read(&s, &two.id).await.status(), 200, "the second stays");
    }

    #[tokio::test]
    async fn a_sign_out_takes_a_body_and_does_not_read_it() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let answer = s
            .client()
            .post(s.url("/session/logout"))
            .header("cookie", cookie_of(&started.id))
            .header("origin", s.url(""))
            .header("x-csrf-token", &started.csrf)
            .body("ignored")
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 204, "signed out");
    }

    #[tokio::test]
    async fn the_csrf_header_s_name_is_matched_in_any_case() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let (cookie, origin) = (cookie_of(&started.id), s.url(""));
        let headers = [
            ("cookie", cookie.as_str()),
            ("origin", origin.as_str()),
            ("X-CSRF-Token", started.csrf.as_str()),
        ];
        let answer = send(&s, Method::POST, "/session/logout", &headers).await;
        assert_eq!(answer.status(), 204, "a header name has no case");
    }

    #[tokio::test]
    async fn a_new_session_after_a_sign_out_is_served() {
        let s = server().await;
        let (user, started) = signed_in(&s);
        assert_eq!(logout(&s, &started).await.status(), 204, "signed out");
        let again = s
            .tenants
            .start_session(user, Some(s.tenant), &["hwk"])
            .expect("signed in again");
        assert_eq!(read(&s, &again.id).await.status(), 200, "the new session");
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "the old cookie stays dead"
        );
    }
}

mod refused {
    use super::*;

    #[tokio::test]
    async fn a_request_with_no_cookie_has_no_session() {
        let s = server().await;
        let answer = send(
            &s,
            Method::GET,
            "/session",
            &[("sec-fetch-site", "same-origin")],
        )
        .await;
        assert_eq!(
            answer.headers()["www-authenticate"],
            "Session",
            "a 401 names what is missing"
        );
        assert_eq!(
            refused(answer).await,
            no_session(false),
            "no cookie to clear"
        );
        let origin = s.url("");
        let answer = send(&s, Method::POST, "/session/logout", &[("origin", &origin)]).await;
        assert_eq!(refused(answer).await, no_session(false), "nor on sign-out");
    }

    #[tokio::test]
    async fn a_cookie_no_session_has_is_refused_and_cleared() {
        let s = server().await;
        signed_in(&s);
        let unknown = crate::session::new_secret();
        assert_eq!(
            refused(read(&s, &unknown).await).await,
            no_session(true),
            "unknown"
        );
        assert_eq!(
            refused(read(&s, "short").await).await,
            no_session(true),
            "malformed"
        );
        let two = format!("{COOKIE}={unknown}; {COOKIE}={unknown}");
        let headers = [("cookie", two.as_str()), ("sec-fetch-site", "same-origin")];
        let answer = send(&s, Method::GET, "/session", &headers).await;
        assert_eq!(refused(answer).await, no_session(true), "two cookies");
    }

    #[tokio::test]
    async fn the_hand_out_is_refused_to_a_request_that_is_not_a_same_origin_fetch() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let cookie = cookie_of(&started.id);
        let only_cookie = [("cookie", cookie.as_str())];
        let answer = send(&s, Method::GET, "/session", &only_cookie).await;
        assert_eq!(
            refused(answer).await,
            bad_origin(),
            "no Sec-Fetch-Site: a script"
        );
        for site in ["cross-site", "same-site", "none"] {
            let headers = [("cookie", cookie.as_str()), ("sec-fetch-site", site)];
            let answer = send(&s, Method::GET, "/session", &headers).await;
            assert_eq!(refused(answer).await, bad_origin(), "{site}");
        }
        let headers = [
            ("cookie", cookie.as_str()),
            ("sec-fetch-site", "same-origin"),
            ("origin", "https://evil.example.net"),
        ];
        let answer = send(&s, Method::GET, "/session", &headers).await;
        assert_eq!(refused(answer).await, bad_origin(), "another origin");
        assert_eq!(
            read(&s, &started.id).await.status(),
            200,
            "the session is live"
        );
    }

    #[tokio::test]
    async fn a_sign_out_from_another_origin_or_none_is_refused_and_ends_nothing() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let cookie = cookie_of(&started.id);
        let slash = format!("{}/", s.url(""));
        let plain = s.url("").replace("https://", "http://");
        for origin in [
            None,
            Some("https://evil.example.net"),
            Some("null"),
            Some(slash.as_str()),
            Some(plain.as_str()),
        ] {
            let mut headers = vec![
                ("cookie", cookie.as_str()),
                ("x-csrf-token", started.csrf.as_str()),
                ("sec-fetch-site", "same-origin"),
            ];
            if let Some(origin) = origin {
                headers.push(("origin", origin));
            }
            let answer = send(&s, Method::POST, "/session/logout", &headers).await;
            assert_eq!(refused(answer).await, bad_origin(), "{origin:?}");
        }
        assert_eq!(
            read(&s, &started.id).await.status(),
            200,
            "the session is live"
        );
    }

    #[tokio::test]
    async fn a_sign_out_with_no_csrf_token_or_a_wrong_one_is_refused_and_ends_nothing() {
        let s = server().await;
        let (user, started) = signed_in(&s);
        let other = s
            .tenants
            .start_session(user, Some(s.tenant), &["hwk"])
            .expect("a second session");
        let (cookie, origin) = (cookie_of(&started.id), s.url(""));
        for token in [
            None,
            Some(""),
            Some(other.csrf.as_str()),
            Some(started.id.as_str()),
        ] {
            let mut headers = vec![("cookie", cookie.as_str()), ("origin", origin.as_str())];
            if let Some(token) = token {
                headers.push(("x-csrf-token", token));
            }
            let answer = send(&s, Method::POST, "/session/logout", &headers).await;
            assert_eq!(refused(answer).await, bad_csrf(), "{token:?}");
        }
        assert_eq!(
            read(&s, &started.id).await.status(),
            200,
            "the session is live"
        );
    }

    #[tokio::test]
    async fn a_cookie_replayed_after_sign_out_is_refused_on_both_routes() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        assert_eq!(logout(&s, &started).await.status(), 204, "signed out");
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "a read"
        );
        assert_eq!(
            refused(logout(&s, &started).await).await,
            no_session(true),
            "a second sign-out"
        );
    }

    #[tokio::test]
    async fn a_request_with_a_key_and_a_session_cookie_is_refused_on_every_route() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let token = s.key(&[Scope::Read]);
        let cookie = cookie_of(&started.id);
        let mut asked = 0;
        for (method, path) in super::super::description::operations() {
            let by_session = path.starts_with("/session");
            if !super::super::description::keyed(&method, &path) && !by_session {
                continue;
            }
            let path = path
                .replace("{uuid}", "0199e5c6-0d7a-7c11-8000-0000000000aa")
                .replace("{version}", "1")
                .replace("{id}", "0123456789abcdef");
            let answer = s
                .client()
                .request(
                    Method::from_bytes(method.as_bytes()).expect("a method"),
                    s.url(&path),
                )
                .bearer_auth(&token)
                .header("cookie", &cookie)
                .header("sec-fetch-site", "same-origin")
                .header("origin", s.url(""))
                .header("x-csrf-token", &started.csrf)
                .send()
                .await
                .expect("an answer");
            assert_eq!(
                refused(answer).await,
                (400, "key-and-session".to_owned(), None),
                "{method} {path}"
            );
            asked += 1;
        }
        assert!(
            asked > 32,
            "every operation that takes a key or a session: {asked}"
        );
        assert_eq!(
            read(&s, &started.id).await.status(),
            200,
            "the session is live"
        );
    }

    #[tokio::test]
    async fn a_key_is_not_a_session() {
        let s = server().await;
        let token = s.key(&[Scope::TenantAdmin]);
        let answer = s
            .client()
            .get(s.url("/session"))
            .bearer_auth(&token)
            .header("sec-fetch-site", "same-origin")
            .send()
            .await
            .expect("an answer");
        assert_eq!(
            refused(answer).await,
            no_session(false),
            "a key has no session"
        );
    }

    #[tokio::test]
    async fn a_session_cookie_alone_reaches_no_route_that_takes_a_key() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let (cookie, origin) = (cookie_of(&started.id), s.url(""));
        for path in ["/vcons", "/key", "/health", "/tenant/settings", "/audit"] {
            let headers = [
                ("cookie", cookie.as_str()),
                ("origin", origin.as_str()),
                ("x-csrf-token", started.csrf.as_str()),
            ];
            let answer = send(&s, Method::GET, path, &headers).await;
            assert_eq!(
                (answer.status().as_u16(), problem(answer).await),
                (401, "unauthorized".to_owned()),
                "{path}: these routes take a key until they are moved to sessions"
            );
        }
    }

    #[tokio::test]
    async fn a_session_past_its_idle_limit_is_refused_and_cleared() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        s.clock.set(START + time::Duration::seconds(901));
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "idle"
        );
        assert_eq!(
            refused(logout(&s, &started).await).await,
            no_session(true),
            "nothing to sign out of"
        );
    }

    #[tokio::test]
    async fn a_session_in_use_is_refused_at_its_absolute_limit() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        for tenth in 1..48 {
            s.clock.set(START + time::Duration::minutes(10 * tenth));
            assert_eq!(
                read(&s, &started.id).await.status(),
                200,
                "in use at {tenth}"
            );
        }
        s.clock
            .set(START + time::Duration::hours(8) + time::Duration::SECOND);
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "8 hours after the start"
        );
    }

    #[tokio::test]
    async fn a_role_change_or_a_deactivation_ends_the_session_at_once() {
        let s = server().await;
        let (user, started) = signed_in(&s);
        s.tenants
            .grant_membership(
                Actor::Cli,
                user,
                Some(s.tenant),
                Roles::of(&[Role::Auditor]),
            )
            .expect("granted");
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "ended by the grant"
        );
        let again = s
            .tenants
            .start_session(user, Some(s.tenant), &["hwk"])
            .expect("signed in again");
        s.tenants
            .deactivate_user(Actor::Cli, user)
            .expect("deactivated");
        assert_eq!(
            refused(logout(&s, &again).await).await,
            no_session(true),
            "ended by the deactivation"
        );
    }

    #[tokio::test]
    async fn a_listener_with_no_public_origin_serves_no_session() {
        let s = server_without_origin().await;
        let (_, started) = signed_in(&s);
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            bad_origin(),
            "the hand-out"
        );
        assert_eq!(
            refused(logout(&s, &started).await).await,
            bad_origin(),
            "sign-out"
        );
    }

    #[tokio::test]
    async fn a_session_of_the_other_plane_is_refused_by_a_listener_and_counted() {
        let tenant = server_on(Plane::Tenant).await;
        let (user, started) = operator_signed_in(&tenant);
        assert_eq!(
            refused(read(&tenant, &started.id).await).await,
            (403, "wrong-listener".to_owned(), None),
            "an operator's session at the tenant listener"
        );
        assert_eq!(
            counted(&admin_log(&tenant, "wrong_listener")),
            [(json!(format!("user:{user}")), json!(403), json!(1))],
            "counted under the user"
        );
        let operator = server_on(Plane::Operator).await;
        let (_, started) = signed_in(&operator);
        assert_eq!(
            refused(logout(&operator, &started).await).await,
            (403, "wrong-listener".to_owned(), None),
            "a tenant's session at the operator listener"
        );
        assert_eq!(
            operator.tenant_log("session_end"),
            Vec::<Value>::new(),
            "and it is not ended there"
        );
    }

    #[tokio::test]
    async fn refusals_are_counted_as_failed_authentication() {
        let s = server().await;
        let (user, started) = signed_in(&s);
        let unknown = crate::session::new_secret();
        for _ in 0..2 {
            assert_eq!(read(&s, &unknown).await.status(), 401, "no session");
        }
        let cookie = cookie_of(&started.id);
        let answer = send(&s, Method::GET, "/session", &[("cookie", &cookie)]).await;
        assert_eq!(answer.status(), 403, "bad origin");
        let origin = s.url("");
        let headers = [("cookie", cookie.as_str()), ("origin", origin.as_str())];
        let answer = send(&s, Method::POST, "/session/logout", &headers).await;
        assert_eq!(answer.status(), 403, "bad csrf");
        assert_eq!(
            counted(&admin_log(&s, "auth_failed")),
            [
                (json!(""), json!(401), json!(2)),
                (json!(""), json!(403), json!(1))
            ],
            "with no actor in the admin log while no session is known"
        );
        assert_eq!(
            counted(&s.tenant_log("auth_failed")),
            [(json!(format!("user:{user}")), json!(403), json!(1))],
            "under the user in the tenant's log once the session is known"
        );
    }

    #[tokio::test]
    async fn a_key_beside_a_cookie_no_session_has_is_refused_all_the_same() {
        let s = server().await;
        let token = s.key(&[Scope::Read]);
        for cookie in [
            cookie_of(&crate::session::new_secret()),
            cookie_of("short"),
            format!("{COOKIE}="),
        ] {
            let answer = s
                .client()
                .get(s.url("/key"))
                .bearer_auth(&token)
                .header("cookie", &cookie)
                .send()
                .await
                .expect("an answer");
            assert_eq!(
                refused(answer).await,
                (400, "key-and-session".to_owned(), None),
                "{cookie}"
            );
        }
        assert_eq!(
            counted(&admin_log(&s, "auth_failed")),
            [(json!(""), json!(400), json!(3))],
            "counted with no actor"
        );
    }
}

mod limits {
    use super::*;

    #[tokio::test]
    async fn the_session_routes_take_one_method_each() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let (cookie, origin) = (cookie_of(&started.id), s.url(""));
        let headers = [
            ("cookie", cookie.as_str()),
            ("origin", origin.as_str()),
            ("x-csrf-token", started.csrf.as_str()),
            ("sec-fetch-site", "same-origin"),
        ];
        for (method, path) in [
            (Method::POST, "/session"),
            (Method::DELETE, "/session"),
            (Method::PUT, "/session"),
            (Method::GET, "/session/logout"),
            (Method::DELETE, "/session/logout"),
        ] {
            let answer = send(&s, method.clone(), path, &headers).await;
            assert_eq!(answer.status(), 405, "{method} {path}");
        }
        assert_eq!(
            read(&s, &started.id).await.status(),
            200,
            "the session is live"
        );
    }

    #[tokio::test]
    async fn a_session_is_live_one_second_before_its_idle_limit() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        s.clock.set(START + time::Duration::seconds(899));
        assert_eq!(
            read(&s, &started.id).await.status(),
            200,
            "14 minutes 59 seconds"
        );
    }

    #[tokio::test]
    async fn a_refused_request_does_not_count_as_seen() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        s.clock.set(START + time::Duration::minutes(10));
        let cookie = cookie_of(&started.id);
        let answer = send(&s, Method::GET, "/session", &[("cookie", &cookie)]).await;
        assert_eq!(answer.status(), 403, "refused for its origin");
        let origin = s.url("");
        let headers = [("cookie", cookie.as_str()), ("origin", origin.as_str())];
        let answer = send(&s, Method::POST, "/session/logout", &headers).await;
        assert_eq!(answer.status(), 403, "refused for its token");
        s.clock.set(START + time::Duration::seconds(901));
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "idle since the start"
        );
    }

    #[tokio::test]
    async fn the_origin_is_checked_before_the_cookie_is_looked_up() {
        let s = server().await;
        let unknown = cookie_of(&crate::session::new_secret());
        let answer = send(&s, Method::GET, "/session", &[("cookie", &unknown)]).await;
        assert_eq!(
            refused(answer).await,
            bad_origin(),
            "a request from elsewhere learns nothing of a cookie"
        );
    }

    #[tokio::test]
    async fn two_origin_headers_are_refused() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let (cookie, origin) = (cookie_of(&started.id), s.url(""));
        let headers = [
            ("cookie", cookie.as_str()),
            ("origin", origin.as_str()),
            ("origin", origin.as_str()),
            ("x-csrf-token", started.csrf.as_str()),
        ];
        let answer = send(&s, Method::POST, "/session/logout", &headers).await;
        assert_eq!(
            refused(answer).await,
            bad_origin(),
            "one Origin is expected"
        );
    }

    #[tokio::test]
    async fn the_session_cookie_is_found_among_other_cookies() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let cookie = format!("theme=dark; {}; lang=en", cookie_of(&started.id));
        let headers = [
            ("cookie", cookie.as_str()),
            ("sec-fetch-site", "same-origin"),
        ];
        let answer = send(&s, Method::GET, "/session", &headers).await;
        assert_eq!(answer.status(), 200, "the one session cookie");
    }

    #[tokio::test]
    async fn a_step_up_s_new_cookie_is_served_and_the_old_one_is_not() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let stepped = s
            .tenants
            .step_up_session(&started.id, &["hwk", "mfa"])
            .expect("stepped up");
        let body: Value = read(&s, &stepped.id).await.json().await.expect("JSON");
        assert_eq!(body["amr"], json!(["hwk", "mfa"]), "the new methods");
        assert_eq!(body["csrf_token"], stepped.csrf, "the new token");
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "the id before the step-up"
        );
    }

    #[tokio::test]
    async fn the_four_refusals_are_problem_documents_of_their_types() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let cookie = cookie_of(&started.id);
        let answer = send(&s, Method::GET, "/session", &[("cookie", &cookie)]).await;
        let body: Value = answer.json().await.expect("a problem document");
        assert_eq!(
            body,
            json!({
                "type": "https://vcon-server.com/problems/bad-origin",
                "title": "The request does not come from this listener's own pages",
                "status": 403,
            }),
            "bad-origin"
        );
        let origin = s.url("");
        let headers = [("cookie", cookie.as_str()), ("origin", origin.as_str())];
        let answer = send(&s, Method::POST, "/session/logout", &headers).await;
        let body: Value = answer.json().await.expect("a problem document");
        assert_eq!(
            body,
            json!({
                "type": "https://vcon-server.com/problems/bad-csrf",
                "title": "The session's CSRF token is missing or wrong",
                "status": 403,
            }),
            "bad-csrf"
        );
        let answer = send(
            &s,
            Method::GET,
            "/session",
            &[("sec-fetch-site", "same-origin")],
        )
        .await;
        let body: Value = answer.json().await.expect("a problem document");
        assert_eq!(
            body,
            json!({
                "type": "https://vcon-server.com/problems/no-session",
                "title": "A live session is needed",
                "status": 401,
            }),
            "no-session"
        );
        let token = s.key(&[Scope::Read]);
        let answer = s
            .client()
            .get(s.url("/key"))
            .bearer_auth(&token)
            .header("cookie", &cookie)
            .send()
            .await
            .expect("an answer");
        let body: Value = answer.json().await.expect("a problem document");
        assert_eq!(
            body,
            json!({
                "type": "https://vcon-server.com/problems/key-and-session",
                "title": "The request carries both a key and a session cookie",
                "status": 400,
            }),
            "key-and-session"
        );
    }

    #[tokio::test]
    async fn a_refusal_of_a_key_route_says_bearer_and_of_a_session_route_says_session() {
        let s = server().await;
        let answer = send(&s, Method::GET, "/key", &[]).await;
        assert_eq!(
            answer.headers()["www-authenticate"],
            "Bearer",
            "a key route"
        );
        let origin = s.url("");
        let answer = send(&s, Method::POST, "/session/logout", &[("origin", &origin)]).await;
        assert_eq!(
            answer.headers()["www-authenticate"],
            "Session",
            "a session route"
        );
    }

    #[tokio::test]
    async fn a_session_whose_tenant_was_removed_is_gone() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        s.tenants
            .remove(Actor::Cli, s.tenant, crate::tenants::Removal::Empty)
            .expect("removed");
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "no session outlives its tenant"
        );
    }

    #[tokio::test]
    async fn the_console_and_its_report_route_take_a_session_cookie_as_any_other() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        let cookie = cookie_of(&started.id);
        let answer = send(&s, Method::GET, "/console/", &[("cookie", &cookie)]).await;
        assert_eq!(answer.status(), 200, "a page takes no key and no session");
        assert!(
            answer.headers().get("set-cookie").is_none(),
            "and sets no cookie"
        );
    }

    #[tokio::test]
    async fn a_cap_change_ends_the_tenant_s_sessions_over_http() {
        let s = server().await;
        let (_, started) = signed_in(&s);
        s.tenants
            .set_role_cap(Actor::Cli, s.tenant, Some(Roles::of(&[Role::Reader])))
            .expect("capped");
        assert_eq!(
            refused(read(&s, &started.id).await).await,
            no_session(true),
            "ended by the cap"
        );
    }
}
