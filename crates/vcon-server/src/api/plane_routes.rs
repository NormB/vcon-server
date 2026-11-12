//! The two planes over HTTPS: on a server with an operator listener, the
//! tenant listener serves tenant keys only and the operator listener
//! operator keys only. A valid key of the other kind is refused with
//! `wrong-listener` before any route sees it, and the refusal is counted.

use reqwest::Method;
use serde_json::Value;

use super::description::operations;
use super::tests::pipeline::{Server, server, server_on};
use crate::access::{Plane, Scope, Scopes};
use crate::audit::Actor;

/// Every scope a tenant key may hold.
const TENANT_SCOPES: [Scope; 5] = [
    Scope::Write,
    Scope::Read,
    Scope::Delete,
    Scope::Legal,
    Scope::Audit,
];

const UUID: &str = "0199e5c6-0d7a-7c11-8000-0000000000aa";

fn operator(s: &Server) -> String {
    s.tenants
        .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
        .expect("an operator key")
        .token
}

/// The id a token names, as the audit log writes it.
fn key_id(token: &str) -> String {
    crate::access::parse(token).expect("a token").to_string()
}

async fn call(s: &Server, method: Method, path: &str, token: Option<&str>) -> reqwest::Response {
    let mut request = s.client().request(method, s.url(path));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.expect("an answer")
}

async fn get(s: &Server, path: &str, token: &str) -> reqwest::Response {
    call(s, Method::GET, path, Some(token)).await
}

/// The status of an answer, and the last part of its problem type.
async fn refusal(answer: reqwest::Response) -> (u16, String) {
    let status = answer.status().as_u16();
    let body: Value = answer.json().await.expect("a problem document");
    let name = body["type"]
        .as_str()
        .expect("a type")
        .rsplit('/')
        .next()
        .expect("a name")
        .to_owned();
    (status, name)
}

fn wrong_listener() -> (u16, String) {
    (403, "wrong-listener".to_owned())
}

/// A path the route template `route` matches.
fn path_of(route: &str) -> String {
    route
        .replace("{uuid}", UUID)
        .replace("{version}", "1")
        .replace("{id}", "0123456789abcdef")
}

/// Every operation of the description that needs a key, as a method and
/// a path its route matches.
fn keyed_operations() -> Vec<(Method, String)> {
    let all: Vec<(Method, String)> = operations()
        .into_iter()
        .filter(|(method, path)| super::description::keyed(method, path))
        .map(|(method, path)| {
            (
                Method::from_bytes(method.as_bytes()).expect("a method"),
                path_of(&path),
            )
        })
        .collect();
    assert!(all.len() >= 20, "the description's operations: {all:?}");
    all
}

fn admin_log(s: &Server, action: &str) -> Vec<Value> {
    s.tenants
        .admin_records()
        .into_iter()
        .filter(|r| r["action"] == action)
        .collect()
}

// A listener that serves the key's kind.

#[tokio::test]
async fn the_one_listener_of_a_server_without_an_operator_listener_serves_both_kinds() {
    let s = server_on(Plane::Both).await;
    let reader = s.key(&[Scope::Read]);
    let op = operator(&s);
    assert_eq!(get(&s, "/key", &reader).await.status(), 200, "a tenant key");
    assert_eq!(get(&s, "/key", &op).await.status(), 200, "an operator key");
    assert_eq!(
        get(&s, "/admin/tenants", &op).await.status(),
        200,
        "the operator routes"
    );
    assert_eq!(get(&s, "/vcons", &reader).await.status(), 200, "the list");
}

/// A client file written for `vcon-server mcp` names the server with
/// `/mcp` at the end. `vcon-server tui` with that address asks for
/// `/mcp/key`, and the terminal UI's page says what the server answers.
#[tokio::test]
async fn a_path_under_mcp_is_not_a_route_and_answers_404_with_no_problem_document() {
    let s = server().await;
    let answer = get(&s, "/mcp/key", &s.key(&[Scope::Read])).await;
    assert_eq!(answer.status(), 404, "no route has this path");
    let body = answer.text().await.expect("a body");
    assert!(
        serde_json::from_str::<Value>(&body).is_err(),
        "the answer is not a problem document, so a client has only the status: {body:?}"
    );
}

#[tokio::test]
async fn the_default_test_server_is_the_one_listener() {
    let s = server().await;
    assert_eq!(
        get(&s, "/key", &operator(&s)).await.status(),
        200,
        "an operator key, as before"
    );
    assert_eq!(
        get(&s, "/key", &s.key(&[Scope::Read])).await.status(),
        200,
        "a tenant key, as before"
    );
}

#[tokio::test]
async fn the_tenant_listener_tells_a_tenant_key_what_it_may_do() {
    let s = server_on(Plane::Tenant).await;
    let token = s.key(&[Scope::Read, Scope::Audit]);
    let answer = get(&s, "/key", &token).await;
    assert_eq!(answer.status(), 200, "the status");
    let body: Value = answer.json().await.expect("JSON");
    assert_eq!(body["id"], key_id(&token), "the key's id");
    assert_eq!(body["tenant"], s.tenant.to_string(), "its tenant");
    assert_eq!(
        body["scopes"],
        serde_json::json!(["read", "audit"]),
        "its scopes"
    );
}

#[tokio::test]
async fn the_tenant_listener_stores_and_reads_a_vcon_for_tenant_keys() {
    let s = server_on(Plane::Tenant).await;
    let token = s.key(&[Scope::Write, Scope::Read]);
    let body = format!(r#"{{"uuid":"{UUID}","created_at":"2026-10-23T12:00:00Z","parties":[]}}"#);
    let stored = s
        .client()
        .post(s.url("/vcons"))
        .bearer_auth(&token)
        .header("content-type", "application/vcon")
        .body(body.clone())
        .send()
        .await
        .expect("an answer");
    assert_eq!(stored.status(), 201, "stored");
    let read = get(&s, &format!("/vcons/{UUID}"), &token).await;
    assert_eq!(read.status(), 200, "read");
    assert_eq!(read.text().await.expect("a body"), body, "byte for byte");
}

#[tokio::test]
async fn the_tenant_listener_answers_each_tenant_scopes_own_route() {
    let s = server_on(Plane::Tenant).await;
    for (scope, path) in [
        (Scope::Read, "/vcons".to_owned()),
        (Scope::Audit, "/audit".to_owned()),
        (Scope::Legal, format!("/legal/vcons/{UUID}/versions")),
    ] {
        let status = get(&s, &path, &s.key(&[scope])).await.status().as_u16();
        // The legal route finds no such vCon: reached, and answered.
        let expected = if scope == Scope::Legal { 404 } else { 200 };
        assert_eq!(status, expected, "{scope:?} on {path}");
    }
}

#[tokio::test]
async fn the_tenant_listener_answers_health_and_the_description_to_a_tenant_key() {
    let s = server_on(Plane::Tenant).await;
    let token = s.key(&[Scope::Read]);
    assert_eq!(get(&s, "/health", &token).await.status(), 200, "health");
    assert_eq!(
        get(&s, "/openapi.json", &token).await.status(),
        200,
        "the description"
    );
}

#[tokio::test]
async fn the_operator_listener_tells_an_operator_key_what_it_may_do() {
    let s = server_on(Plane::Operator).await;
    let op = operator(&s);
    let answer = get(&s, "/key", &op).await;
    assert_eq!(answer.status(), 200, "the status");
    let body: Value = answer.json().await.expect("JSON");
    assert_eq!(body["id"], key_id(&op), "the key's id");
    assert_eq!(body["tenant"], Value::Null, "no tenant");
    assert_eq!(body["scopes"], serde_json::json!(["admin"]), "its scope");
}

#[tokio::test]
async fn the_operator_listener_makes_and_lists_tenants_and_their_keys() {
    let s = server_on(Plane::Operator).await;
    let op = operator(&s);
    let made = s
        .client()
        .post(s.url("/admin/tenants"))
        .bearer_auth(&op)
        .json(&serde_json::json!({"name": "example"}))
        .send()
        .await
        .expect("an answer");
    assert_eq!(made.status(), 201, "a tenant made");
    let id = made.json::<Value>().await.expect("JSON")["id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let listed: Value = get(&s, "/admin/tenants", &op)
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        listed.as_array().expect("a list").len(),
        2,
        "both tenants listed"
    );
    let key = s
        .client()
        .post(s.url(&format!("/admin/tenants/{id}/keys")))
        .bearer_auth(&op)
        .json(&serde_json::json!({"scopes": ["read"], "label": "reader"}))
        .send()
        .await
        .expect("an answer");
    assert_eq!(key.status(), 201, "a key made");
    assert_eq!(
        get(&s, &format!("/admin/tenants/{id}/keys"), &op)
            .await
            .status(),
        200,
        "its keys listed"
    );
}

#[tokio::test]
async fn the_operator_listener_reads_the_admin_log() {
    let s = server_on(Plane::Operator).await;
    let op = operator(&s);
    let answer = get(&s, "/admin/audit", &op).await;
    assert_eq!(answer.status(), 200, "the status");
    assert!(
        answer.text().await.expect("a body").contains("\"action\""),
        "records, a line each"
    );
}

#[tokio::test]
async fn the_operator_listener_answers_health_and_the_description_to_an_operator_key() {
    let s = server_on(Plane::Operator).await;
    let op = operator(&s);
    assert_eq!(get(&s, "/health", &op).await.status(), 200, "health");
    assert_eq!(
        get(&s, "/openapi.json", &op).await.status(),
        200,
        "the description"
    );
}

#[tokio::test]
async fn every_listener_serves_the_console_files_without_a_key() {
    for plane in [Plane::Both, Plane::Tenant, Plane::Operator] {
        let s = server_on(plane).await;
        for (name, body, media_type) in crate::console::FILES {
            let path = crate::console::path_of(name);
            let answer = call(&s, Method::GET, &path, None).await;
            assert_eq!(answer.status(), 200, "{plane:?} {path}");
            assert_eq!(
                answer.headers()["content-type"],
                media_type,
                "{plane:?} {path}"
            );
            assert_eq!(
                answer.bytes().await.expect("a body").as_ref(),
                body,
                "{plane:?} {path}"
            );
        }
    }
}

#[tokio::test]
async fn a_key_of_the_listeners_kind_leaves_no_wrong_listener_record() {
    for plane in [Plane::Both, Plane::Tenant, Plane::Operator] {
        let s = server_on(plane).await;
        if plane != Plane::Operator {
            get(&s, "/key", &s.key(&[Scope::Read])).await;
        }
        if plane != Plane::Tenant {
            get(&s, "/key", &operator(&s)).await;
        }
        assert_eq!(
            admin_log(&s, "wrong_listener"),
            Vec::<Value>::new(),
            "{plane:?}: the admin log"
        );
        assert_eq!(
            s.tenant_log("wrong_listener"),
            Vec::<Value>::new(),
            "{plane:?}: the tenant log"
        );
    }
}

// A listener that serves the other kind.

#[tokio::test]
async fn the_tenant_listener_refuses_an_operator_key_on_every_operation() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    for (method, path) in keyed_operations() {
        let answer = call(&s, method.clone(), &path, Some(&op)).await;
        assert_eq!(refusal(answer).await, wrong_listener(), "{method} {path}");
    }
}

#[tokio::test]
async fn the_operator_listener_refuses_a_tenant_key_on_every_operation() {
    let s = server_on(Plane::Operator).await;
    let token = s.key(&TENANT_SCOPES);
    for (method, path) in keyed_operations() {
        let answer = call(&s, method.clone(), &path, Some(&token)).await;
        assert_eq!(refusal(answer).await, wrong_listener(), "{method} {path}");
    }
}

#[tokio::test]
async fn the_operator_listener_refuses_a_tenant_key_of_each_single_scope() {
    let s = server_on(Plane::Operator).await;
    for scope in TENANT_SCOPES {
        let answer = get(&s, "/key", &s.key(&[scope])).await;
        assert_eq!(refusal(answer).await, wrong_listener(), "{scope:?}");
    }
}

#[tokio::test]
async fn the_refusal_is_a_problem_document_with_its_own_type_and_no_challenge() {
    let s = server_on(Plane::Tenant).await;
    let answer = get(&s, "/key", &operator(&s)).await;
    assert_eq!(answer.status(), 403, "the status");
    assert_eq!(
        answer.headers()["content-type"],
        "application/problem+json",
        "the media type"
    );
    assert!(
        answer.headers().get("www-authenticate").is_none(),
        "no challenge: the key is valid"
    );
    let body: Value = answer.json().await.expect("JSON");
    assert_eq!(
        body,
        serde_json::json!({
            "type": "https://vcon-server.com/problems/wrong-listener",
            "title": "This listener does not serve this kind of key",
            "status": 403,
        }),
        "the document"
    );
}

#[tokio::test]
async fn an_operator_keys_refusal_is_counted_in_the_admin_log() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    get(&s, "/admin/tenants", &op).await;
    let counted = admin_log(&s, "wrong_listener");
    assert_eq!(counted.len(), 1, "one record: {counted:?}");
    assert_eq!(counted[0]["actor"], key_id(&op), "the key");
    assert_eq!(counted[0]["status"], 403, "the status");
    assert_eq!(counted[0]["count"], 1, "the count");
    assert_eq!(counted[0]["target"], Value::Null, "no target");
    assert_eq!(
        s.tenant_log("wrong_listener"),
        Vec::<Value>::new(),
        "nothing in a tenant's log"
    );
}

#[tokio::test]
async fn a_tenant_keys_refusal_is_counted_in_its_tenants_log() {
    let s = server_on(Plane::Operator).await;
    let token = s.key(&[Scope::Read]);
    get(&s, "/vcons", &token).await;
    let counted = s.tenant_log("wrong_listener");
    assert_eq!(counted.len(), 1, "one record: {counted:?}");
    assert_eq!(counted[0]["actor"], key_id(&token), "the key");
    assert_eq!(counted[0]["status"], 403, "the status");
    assert_eq!(counted[0]["count"], 1, "the count");
    assert_eq!(
        admin_log(&s, "wrong_listener"),
        Vec::<Value>::new(),
        "nothing in the admin log"
    );
}

#[tokio::test]
async fn a_refused_operator_key_makes_no_tenant() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    let answer = s
        .client()
        .post(s.url("/admin/tenants"))
        .bearer_auth(&op)
        .json(&serde_json::json!({"name": "example"}))
        .send()
        .await
        .expect("an answer");
    assert_eq!(refusal(answer).await, wrong_listener(), "refused");
    assert_eq!(
        s.tenants.ids().expect("the tenants").len(),
        1,
        "only the tenant the server started with"
    );
    assert_eq!(
        admin_log(&s, "tenant_create")
            .iter()
            .filter(|r| r["actor"] == key_id(&op))
            .count(),
        0,
        "the route never saw the request"
    );
}

#[tokio::test]
async fn a_refused_tenant_key_reads_no_vcon() {
    let s = server_on(Plane::Operator).await;
    let token = s.key(&[Scope::Read]);
    let store = s.tenants.store(s.tenant).expect("its store");
    let (found, body) = crate::store::testing::vcon(UUID, "kept");
    store
        .create(
            &crate::store::Caller {
                key: crate::access::parse(&token).expect("a token"),
            },
            &found,
            &body,
        )
        .expect("stored");
    let answer = get(&s, &format!("/vcons/{UUID}"), &token).await;
    let text = answer.text().await.expect("a body");
    assert!(
        text.contains("wrong-listener"),
        "the refusal, not the vCon: {text}"
    );
    assert!(!text.contains("kept"), "none of the vCon: {text}");
    assert_eq!(
        s.tenant_log("read"),
        Vec::<Value>::new(),
        "no read recorded"
    );
}

#[tokio::test]
async fn a_refused_tenant_key_stores_no_vcon() {
    let s = server_on(Plane::Operator).await;
    let token = s.key(&[Scope::Write]);
    let body = format!(r#"{{"uuid":"{UUID}","created_at":"2026-10-23T12:00:00Z","parties":[]}}"#);
    let answer = s
        .client()
        .post(s.url("/vcons"))
        .bearer_auth(&token)
        .header("content-type", "application/vcon")
        .body(body)
        .send()
        .await
        .expect("an answer");
    assert_eq!(refusal(answer).await, wrong_listener(), "refused");
    let counts = s
        .tenants
        .store(s.tenant)
        .expect("its store")
        .counts()
        .expect("counts");
    assert_eq!(counts.versions, 0, "nothing stored");
}

#[tokio::test]
async fn no_key_and_a_made_up_key_are_unauthorized_on_every_plane() {
    for plane in [Plane::Both, Plane::Tenant, Plane::Operator] {
        let s = server_on(plane).await;
        for path in ["/key", "/vcons", "/admin/tenants", "/health"] {
            let none = call(&s, Method::GET, path, None).await;
            assert_eq!(
                refusal(none).await,
                (401, "unauthorized".to_owned()),
                "{plane:?} {path} with no key"
            );
            let made_up = get(&s, path, "vcs_0123456789abcdef_AAAA").await;
            assert_eq!(
                refusal(made_up).await,
                (401, "unauthorized".to_owned()),
                "{plane:?} {path} with a made-up key"
            );
        }
        assert_eq!(
            admin_log(&s, "wrong_listener"),
            Vec::<Value>::new(),
            "{plane:?}: a key that is not valid is never a wrong listener"
        );
    }
}

#[tokio::test]
async fn a_revoked_key_of_the_other_kind_is_unauthorized_not_told_of_the_listener() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    s.tenants
        .revoke_key(Actor::Cli, crate::access::parse(&op).expect("a token"))
        .expect("revoked");
    let answer = get(&s, "/key", &op).await;
    assert_eq!(
        refusal(answer).await,
        (401, "unauthorized".to_owned()),
        "revoked"
    );
    let t = server_on(Plane::Operator).await;
    let token = t.key(&[Scope::Read]);
    t.tenants
        .revoke_key(Actor::Cli, crate::access::parse(&token).expect("a token"))
        .expect("revoked");
    let answer = get(&t, "/key", &token).await;
    assert_eq!(
        refusal(answer).await,
        (401, "unauthorized".to_owned()),
        "revoked"
    );
}

#[tokio::test]
async fn a_wrong_secret_under_a_key_of_the_other_kind_is_unauthorized() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    // The same key id, with another secret of the same length.
    let (id, secret) = op.rsplit_once('_').expect("id and secret");
    let other = if secret.starts_with('A') { "B" } else { "A" };
    let forged = format!("{id}_{other}{}", &secret[1..]);
    assert_ne!(forged, op, "another secret");
    let answer = get(&s, "/key", &forged).await;
    assert_eq!(
        refusal(answer).await,
        (401, "unauthorized".to_owned()),
        "the secret is checked before the listener"
    );
    assert_eq!(
        admin_log(&s, "wrong_listener"),
        Vec::<Value>::new(),
        "not counted as a wrong listener"
    );
}

#[tokio::test]
async fn a_key_table_that_cannot_be_read_is_unavailable_on_each_plane() {
    for plane in [Plane::Tenant, Plane::Operator] {
        let s = server_on(plane).await;
        let (tenant, op) = (s.key(&[Scope::Read]), operator(&s));
        s.tenants
            .conn_for_test()
            .execute_batch("ALTER TABLE api_key RENAME TO gone")
            .expect("the key table taken away");
        for token in [&tenant, &op] {
            let answer = get(&s, "/key", token).await;
            assert_eq!(
                refusal(answer).await,
                (503, "unavailable".to_owned()),
                "{plane:?}"
            );
        }
    }
}

// The edges of the rule.

#[tokio::test]
async fn a_tenant_key_on_the_tenant_listener_is_forbidden_the_operator_routes_as_before() {
    let s = server_on(Plane::Tenant).await;
    let token = s.key(&TENANT_SCOPES);
    for path in ["/admin/tenants", "/admin/audit"] {
        let answer = get(&s, path, &token).await;
        assert_eq!(
            refusal(answer).await,
            (403, "forbidden".to_owned()),
            "{path}"
        );
    }
}

#[tokio::test]
async fn an_operator_key_on_the_operator_listener_is_forbidden_the_tenant_routes_as_before() {
    let s = server_on(Plane::Operator).await;
    let op = operator(&s);
    for path in ["/vcons", "/audit", &format!("/legal/vcons/{UUID}")] {
        let answer = get(&s, path, &op).await;
        assert_eq!(
            refusal(answer).await,
            (403, "forbidden".to_owned()),
            "{path}"
        );
    }
}

#[tokio::test]
async fn refusals_in_one_minute_share_one_record() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    s.clock
        .set(time::macros::datetime!(2026-10-25 12:00:00 UTC));
    for path in ["/key", "/health", "/admin/tenants", "/vcons", "/key"] {
        get(&s, path, &op).await;
    }
    let counted = admin_log(&s, "wrong_listener");
    assert_eq!(counted.len(), 1, "one record: {counted:?}");
    assert_eq!(counted[0]["count"], 5, "five refusals");
}

#[tokio::test]
async fn refusals_in_two_minutes_make_two_records() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    s.clock
        .set(time::macros::datetime!(2026-10-25 12:00:00 UTC));
    get(&s, "/key", &op).await;
    get(&s, "/key", &op).await;
    s.clock
        .set(time::macros::datetime!(2026-10-25 12:01:00 UTC));
    get(&s, "/key", &op).await;
    let counts: Vec<Value> = admin_log(&s, "wrong_listener")
        .iter()
        .map(|r| r["count"].clone())
        .collect();
    assert_eq!(
        counts,
        vec![serde_json::json!(2), serde_json::json!(1)],
        "a record a minute"
    );
}

#[tokio::test]
async fn two_keys_refused_in_one_minute_have_a_record_each() {
    let s = server_on(Plane::Tenant).await;
    let (first, second) = (operator(&s), operator(&s));
    s.clock
        .set(time::macros::datetime!(2026-10-25 12:00:00 UTC));
    get(&s, "/key", &first).await;
    get(&s, "/key", &second).await;
    let mut actors: Vec<String> = admin_log(&s, "wrong_listener")
        .iter()
        .map(|r| r["actor"].as_str().expect("an actor").to_owned())
        .collect();
    actors.sort();
    let mut expected = vec![key_id(&first), key_id(&second)];
    expected.sort();
    assert_eq!(actors, expected, "one record for each key");
}

#[tokio::test]
async fn a_tenant_keys_refusal_goes_to_the_admin_log_when_its_tenant_is_closing() {
    let s = server_on(Plane::Operator).await;
    let token = s.key(&[Scope::Read]);
    s.tenants.close(s.tenant);
    let answer = get(&s, "/key", &token).await;
    assert_eq!(refusal(answer).await, wrong_listener(), "refused");
    let counted = admin_log(&s, "wrong_listener");
    assert_eq!(counted.len(), 1, "counted in the admin log: {counted:?}");
    assert_eq!(counted[0]["actor"], key_id(&token), "under the key");
}

#[tokio::test]
async fn a_refusal_that_cannot_be_counted_is_refused_all_the_same() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    s.tenants.break_audit();
    let answer = get(&s, "/admin/tenants", &op).await;
    assert_eq!(
        refusal(answer).await,
        wrong_listener(),
        "the listener still refuses the key"
    );
}

#[tokio::test]
async fn a_path_no_route_matches_is_404_on_every_plane() {
    for plane in [Plane::Both, Plane::Tenant, Plane::Operator] {
        let s = server_on(plane).await;
        for token in [None, Some(operator(&s)), Some(s.key(&[Scope::Read]))] {
            let answer = call(&s, Method::GET, "/no-such-route", token.as_deref()).await;
            assert_eq!(answer.status(), 404, "{plane:?} {token:?}");
        }
        assert_eq!(
            admin_log(&s, "wrong_listener"),
            Vec::<Value>::new(),
            "{plane:?}: no route, so no key was checked"
        );
    }
}

#[tokio::test]
async fn a_method_a_path_does_not_take_is_405_on_every_plane() {
    for plane in [Plane::Both, Plane::Tenant, Plane::Operator] {
        let s = server_on(plane).await;
        let op = operator(&s);
        let answer = call(&s, Method::DELETE, "/key", Some(&op)).await;
        assert_eq!(answer.status(), 405, "{plane:?}");
    }
}

#[tokio::test]
async fn the_console_path_without_its_slash_is_sent_on_by_every_listener() {
    for plane in [Plane::Both, Plane::Tenant, Plane::Operator] {
        let s = server_on(plane).await;
        let answer = s
            .client_not_following()
            .get(s.url("/console"))
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 308, "{plane:?}");
        assert_eq!(answer.headers()["location"], "/console/", "{plane:?}");
    }
}

#[tokio::test]
async fn the_authorization_schemes_case_does_not_change_the_refusal() {
    let s = server_on(Plane::Tenant).await;
    let op = operator(&s);
    for scheme in ["Bearer", "bearer", "BEARER"] {
        let answer = s
            .client()
            .get(s.url("/key"))
            .header("authorization", format!("{scheme}  {op}"))
            .send()
            .await
            .expect("an answer");
        assert_eq!(refusal(answer).await, wrong_listener(), "{scheme}");
    }
}

#[tokio::test]
async fn a_refused_request_with_a_body_over_the_limit_is_still_a_wrong_listener() {
    // The key is checked before the body is read.
    let s = server_on(Plane::Operator).await;
    let token = s.key(&[Scope::Write]);
    let answer = s
        .client()
        .post(s.url("/vcons"))
        .bearer_auth(&token)
        .body(vec![b'x'; super::tests::pipeline::LIMIT + 1])
        .send()
        .await
        .expect("an answer");
    assert_eq!(
        refusal(answer).await,
        wrong_listener(),
        "refused on the key"
    );
}

#[tokio::test]
async fn the_description_lists_403_for_every_operation_that_needs_a_key() {
    for (method, path) in operations() {
        if super::description::keyed(&method, &path) {
            assert!(
                super::description::lists(&method, &path, 403),
                "{method} {path} can answer wrong-listener (403)"
            );
        }
    }
}
