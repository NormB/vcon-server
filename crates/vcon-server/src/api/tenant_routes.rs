//! The routes of a tenant's administrator over HTTPS: `/tenant/settings`,
//! `/tenant/keys` and `/tenant/memberships`. Each needs a key of a tenant
//! that holds `tenant_admin`, works in that key's tenant alone, and is
//! recorded in the tenant's audit log.

use reqwest::Method;
use serde_json::{Value, json};

use super::tests::pipeline::{Server, problem, server, server_on};
use crate::access::{Plane, Scope, Scopes};
use crate::audit::Actor;
use crate::ids::TenantId;
use crate::roles::{Role, Roles};

fn administrator(s: &Server) -> String {
    s.key(&[Scope::TenantAdmin])
}

/// The id a token names, as the audit log writes it.
fn key_id(token: &str) -> String {
    crate::access::parse(token).expect("a token").to_string()
}

async fn send(
    s: &Server,
    method: Method,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> reqwest::Response {
    let mut request = s.client().request(method, s.url(path)).bearer_auth(token);
    if let Some(body) = body {
        request = request.json(body);
    }
    request.send().await.expect("an answer")
}

async fn get(s: &Server, path: &str, token: &str) -> reqwest::Response {
    send(s, Method::GET, path, token, None).await
}

/// The status of an answer and its JSON body.
async fn answered(answer: reqwest::Response) -> (u16, Value) {
    let status = answer.status().as_u16();
    (status, answer.json().await.expect("a JSON body"))
}

/// The status of a refusal and the last part of its problem type.
async fn refused(answer: reqwest::Response) -> (u16, String) {
    (answer.status().as_u16(), problem(answer).await)
}

/// The records of `action` in the tenant's log: actor, target and status.
fn logged(s: &Server, action: &str) -> Vec<(Value, Value, Value)> {
    s.tenant_log(action)
        .into_iter()
        .map(|r| (r["actor"].clone(), r["target"].clone(), r["status"].clone()))
        .collect()
}

/// A second tenant and a key of its administrator.
fn other_tenant(s: &Server) -> (TenantId, String) {
    let tenant = s
        .tenants
        .create(Actor::Cli, "other", None)
        .expect("a tenant");
    let token = s
        .tenants
        .create_key(
            Actor::Cli,
            Some(tenant),
            Scopes::of(&[Scope::TenantAdmin]),
            "admin",
        )
        .expect("a key")
        .token;
    (tenant, token)
}

// What must work.

#[tokio::test]
async fn an_administrator_reads_every_setting_with_its_default() {
    let s = server().await;
    let (status, body) = answered(get(&s, "/tenant/settings", &administrator(&s)).await).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "masking.reader": true,
            "masking.producer": true,
            "masking.records": true,
            "masking.legal": true,
            "masking.auditor": true,
            "masking.tenant-admin": true,
            "session.idle_seconds": 900,
            "session.absolute_seconds": 28_800,
        }),
        "every name, each with its default"
    );
}

#[tokio::test]
async fn a_change_answers_every_setting_after_it_and_a_read_agrees() {
    let s = server().await;
    let token = administrator(&s);
    let change = json!({ "masking.legal": false, "session.idle_seconds": 300 });
    let (status, body) =
        answered(send(&s, Method::PUT, "/tenant/settings", &token, Some(&change)).await).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["masking.legal"], json!(false), "the change");
    assert_eq!(body["session.idle_seconds"], json!(300), "the change");
    assert_eq!(body["masking.reader"], json!(true), "a name left alone");
    let (_, read) = answered(get(&s, "/tenant/settings", &token).await).await;
    assert_eq!(read, body, "a read gives what the change answered");
}

#[tokio::test]
async fn a_null_puts_a_setting_back_to_its_default() {
    let s = server().await;
    let token = administrator(&s);
    let set = json!({ "session.idle_seconds": 120 });
    send(&s, Method::PUT, "/tenant/settings", &token, Some(&set)).await;
    let unset = json!({ "session.idle_seconds": null });
    let (status, body) =
        answered(send(&s, Method::PUT, "/tenant/settings", &token, Some(&unset)).await).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["session.idle_seconds"], json!(900), "the default");
}

#[tokio::test]
async fn reads_and_changes_of_settings_are_in_the_tenants_log() {
    let s = server().await;
    let token = administrator(&s);
    get(&s, "/tenant/settings", &token).await;
    let change = json!({ "masking.reader": false });
    send(&s, Method::PUT, "/tenant/settings", &token, Some(&change)).await;
    let (key, tenant) = (json!(key_id(&token)), s.tenant.to_string());
    assert_eq!(
        logged(&s, "settings_read"),
        [(key.clone(), json!(tenant), json!(200))],
        "the read"
    );
    assert_eq!(
        logged(&s, "settings_set"),
        [(key, json!(format!("{tenant}/masking.reader")), json!(200))],
        "the change, with the name and no value"
    );
}

#[tokio::test]
async fn an_administrator_makes_a_key_and_is_shown_its_token_once() {
    let s = server().await;
    let token = administrator(&s);
    let new = json!({ "scopes": ["read", "write"], "label": "reports" });
    let answer = send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await;
    assert_eq!(
        answer.headers().get("cache-control").map(|v| v.as_bytes()),
        Some(&b"no-store"[..]),
        "a token is not kept by a cache"
    );
    let (status, body) = answered(answer).await;
    // The body of a 201 holds the token, so a failure shows only the
    // problem type of another answer.
    assert_eq!(status, 201, "the key is made: {}", body["type"]);
    let made = body["token"].as_str().expect("the token");
    assert_eq!(body["id"], json!(key_id(made)), "the id the token names");
    let (status, _) = answered(get(&s, "/key", made).await).await;
    assert_eq!(status, 200, "the new key works");
}

#[tokio::test]
async fn a_key_made_here_holds_the_scopes_asked_in_the_makers_tenant() {
    let s = server().await;
    let token = administrator(&s);
    let new = json!({ "scopes": ["audit"], "label": "auditor" });
    let (_, body) =
        answered(send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await).await;
    let made = body["token"].as_str().expect("the token");
    let (_, info) = answered(get(&s, "/key", made).await).await;
    assert_eq!(info["scopes"], json!(["audit"]), "the scopes asked: {info}");
    assert_eq!(
        info["tenant"],
        json!(s.tenant.to_string()),
        "the tenant of the key that made it"
    );
}

#[tokio::test]
async fn an_administrator_lists_its_tenants_keys_without_a_token() {
    let s = server().await;
    let token = administrator(&s);
    let new = json!({ "scopes": ["read"], "label": "reports" });
    send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await;
    let (status, body) = answered(get(&s, "/tenant/keys", &token).await).await;
    assert_eq!(status, 200, "{body}");
    let keys = body.as_array().expect("a list");
    let labels: Vec<&str> = keys.iter().filter_map(|k| k["label"].as_str()).collect();
    assert_eq!(labels, ["test", "reports"], "oldest first");
    assert_eq!(
        keys[1]["scopes"],
        json!(["read"]),
        "each with its scopes: {body}"
    );
    assert!(
        keys.iter().all(|k| k.get("token").is_none()),
        "the list never holds a token"
    );
}

#[tokio::test]
async fn an_administrator_revokes_a_key_of_its_tenant() {
    let s = server().await;
    let token = administrator(&s);
    let old = s.key(&[Scope::Read]);
    let path = format!("/tenant/keys/{}", key_id(&old));
    let answer = send(&s, Method::DELETE, &path, &token, None).await;
    assert_eq!(answer.status().as_u16(), 204, "revoked");
    assert_eq!(
        get(&s, "/key", &old).await.status().as_u16(),
        401,
        "the revoked key's next request is refused"
    );
    assert_eq!(
        logged(&s, "key_revoke"),
        [(json!(key_id(&token)), json!(key_id(&old)), json!(204))],
        "in the tenant's log"
    );
}

#[tokio::test]
async fn an_administrator_lists_its_tenants_memberships() {
    let s = server().await;
    let user = s
        .tenants
        .add_user(Actor::Cli, "Reader", None)
        .expect("a user");
    s.tenants
        .grant_membership(
            Actor::Cli,
            user,
            Some(s.tenant),
            Roles::of(&[Role::Reader, Role::Legal]),
        )
        .expect("granted");
    let token = administrator(&s);
    let (status, body) = answered(get(&s, "/tenant/memberships", &token).await).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body.as_array().map(Vec::len), Some(1), "one member: {body}");
    assert_eq!(body[0]["user"], json!(user.to_string()), "the user's id");
    assert_eq!(body[0]["label"], json!("Reader"), "the label");
    assert_eq!(body[0]["roles"], json!(["reader", "legal"]), "the roles");
    assert_eq!(body[0]["scopes"], json!(["read", "legal"]), "the scopes");
    assert_eq!(
        logged(&s, "membership_list"),
        [(
            json!(key_id(&token)),
            json!(s.tenant.to_string()),
            json!(200)
        )],
        "in the tenant's log"
    );
}

#[tokio::test]
async fn making_and_listing_keys_are_in_the_tenants_log() {
    let s = server().await;
    let token = administrator(&s);
    let new = json!({ "scopes": ["read"], "label": "reports" });
    let (_, body) =
        answered(send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await).await;
    get(&s, "/tenant/keys", &token).await;
    let key = json!(key_id(&token));
    assert_eq!(
        logged(&s, "key_create").last(),
        Some(&(key.clone(), body["id"].clone(), json!(201))),
        "the new key's id"
    );
    assert_eq!(
        logged(&s, "key_list"),
        [(key, json!(s.tenant.to_string()), json!(200))],
        "the list"
    );
}

#[tokio::test]
async fn the_routes_work_on_the_tenant_listener_of_a_split_server() {
    let s = server_on(Plane::Tenant).await;
    let token = administrator(&s);
    for path in ["/tenant/settings", "/tenant/keys", "/tenant/memberships"] {
        assert_eq!(
            get(&s, path, &token).await.status().as_u16(),
            200,
            "GET {path} at the tenant listener"
        );
    }
}

#[tokio::test]
async fn a_key_with_other_scopes_too_administers_its_tenant() {
    let s = server().await;
    let token = s.key(&[Scope::Read, Scope::TenantAdmin]);
    assert_eq!(
        get(&s, "/tenant/settings", &token).await.status().as_u16(),
        200,
        "tenant_admin among a key's scopes is enough"
    );
}

// What must be refused.

#[tokio::test]
async fn a_key_without_tenant_admin_is_forbidden_on_every_route() {
    let s = server().await;
    let token = s.key(&[
        Scope::Write,
        Scope::Read,
        Scope::Delete,
        Scope::Legal,
        Scope::Audit,
    ]);
    for (method, path) in [
        (Method::GET, "/tenant/settings"),
        (Method::PUT, "/tenant/settings"),
        (Method::GET, "/tenant/keys"),
        (Method::POST, "/tenant/keys"),
        (Method::DELETE, "/tenant/keys/0123456789abcdef"),
        (Method::GET, "/tenant/memberships"),
    ] {
        let body = json!({});
        let answer = send(&s, method.clone(), path, &token, Some(&body)).await;
        assert_eq!(
            refused(answer).await,
            (403, "forbidden".to_owned()),
            "{method} {path} with every scope but tenant_admin"
        );
    }
    assert_eq!(
        logged(&s, "settings_set"),
        [(json!(key_id(&token)), Value::Null, json!(403))],
        "a refusal is in the tenant's log"
    );
}

#[tokio::test]
async fn an_operator_key_is_forbidden_on_every_route() {
    let s = server().await;
    let operator = s
        .tenants
        .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
        .expect("an operator key")
        .token;
    for path in ["/tenant/settings", "/tenant/keys", "/tenant/memberships"] {
        assert_eq!(
            refused(get(&s, path, &operator).await).await,
            (403, "forbidden".to_owned()),
            "GET {path}: an operator key has no tenant"
        );
    }
}

#[tokio::test]
async fn no_key_is_unauthorized_on_every_route() {
    let s = server().await;
    for path in ["/tenant/settings", "/tenant/keys", "/tenant/memberships"] {
        let answer = s.client().get(s.url(path)).send().await.expect("an answer");
        assert_eq!(
            refused(answer).await,
            (401, "unauthorized".to_owned()),
            "GET {path} with no key"
        );
    }
}

#[tokio::test]
async fn a_setting_outside_the_list_is_refused_as_a_bad_setting() {
    let s = server().await;
    let token = administrator(&s);
    for change in [
        json!({ "bulk_export": false }),
        json!({ "masking.reader": "off" }),
        json!({ "session.idle_seconds": 30 }),
        json!({ "session.idle_seconds": 900, "session.absolute_seconds": 600 }),
        json!(["masking.reader"]),
    ] {
        let answer = send(&s, Method::PUT, "/tenant/settings", &token, Some(&change)).await;
        assert_eq!(
            refused(answer).await,
            (422, "bad-setting".to_owned()),
            "{change}"
        );
    }
    let (_, read) = answered(get(&s, "/tenant/settings", &token).await).await;
    assert_eq!(read["masking.reader"], json!(true), "nothing was stored");
}

#[tokio::test]
async fn a_change_that_is_not_json_is_an_invalid_body() {
    let s = server().await;
    let token = administrator(&s);
    let answer = s
        .client()
        .put(s.url("/tenant/settings"))
        .bearer_auth(&token)
        .header("content-type", "application/json")
        .body("{ masking")
        .send()
        .await
        .expect("an answer");
    assert_eq!(
        refused(answer).await,
        (422, "invalid-body".to_owned()),
        "not JSON"
    );
    assert_eq!(
        logged(&s, "settings_set"),
        [(json!(key_id(&token)), Value::Null, json!(422))],
        "recorded"
    );
}

#[tokio::test]
async fn an_administrator_cannot_make_a_key_that_administers() {
    let s = server().await;
    let token = administrator(&s);
    for scopes in [
        json!(["tenant_admin"]),
        json!(["read", "tenant_admin"]),
        json!(["admin"]),
        json!([]),
        json!(["read", "read"]),
        json!(["everything"]),
    ] {
        let new = json!({ "scopes": scopes, "label": "more" });
        let answer = send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await;
        assert_eq!(
            refused(answer).await,
            (422, "bad-scopes".to_owned()),
            "{scopes}"
        );
    }
    let (_, keys) = answered(get(&s, "/tenant/keys", &token).await).await;
    assert_eq!(
        keys.as_array().map(Vec::len),
        Some(1),
        "no key was made: {keys}"
    );
}

#[tokio::test]
async fn a_scope_outside_the_role_cap_is_refused() {
    let s = server().await;
    s.tenants
        .set_role_cap(
            Actor::Cli,
            s.tenant,
            Some(Roles::of(&[Role::Reader, Role::TenantAdmin])),
        )
        .expect("a cap");
    let token = administrator(&s);
    let new = json!({ "scopes": ["write"], "label": "producer" });
    let answer = send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await;
    assert_eq!(
        refused(answer).await,
        (422, "bad-scopes".to_owned()),
        "write is outside a cap of reader"
    );
    let new = json!({ "scopes": ["read"], "label": "reader" });
    let answer = send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await;
    assert_eq!(answer.status().as_u16(), 201, "read is within it");
}

#[tokio::test]
async fn a_key_body_without_its_members_is_an_invalid_body() {
    let s = server().await;
    let token = administrator(&s);
    for new in [
        json!({ "scopes": ["read"] }),
        json!({ "label": "x" }),
        json!({ "scopes": "read", "label": "x" }),
        json!(["read"]),
    ] {
        let answer = send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await;
        assert_eq!(
            refused(answer).await,
            (422, "invalid-body".to_owned()),
            "{new}"
        );
    }
}

#[tokio::test]
async fn a_key_of_another_tenant_is_unknown_and_stays_valid() {
    let s = server().await;
    let token = administrator(&s);
    let (_, theirs) = other_tenant(&s);
    let path = format!("/tenant/keys/{}", key_id(&theirs));
    let answer = send(&s, Method::DELETE, &path, &token, None).await;
    assert_eq!(
        refused(answer).await,
        (404, "unknown-key".to_owned()),
        "as if there were no such key"
    );
    assert_eq!(
        get(&s, "/tenant/settings", &theirs).await.status().as_u16(),
        200,
        "the other tenant's key still works"
    );
}

#[tokio::test]
async fn a_key_id_that_is_not_one_is_unknown() {
    let s = server().await;
    let token = administrator(&s);
    for id in ["nothing", "0123", "0123456789abcdef"] {
        let path = format!("/tenant/keys/{id}");
        let answer = send(&s, Method::DELETE, &path, &token, None).await;
        assert_eq!(
            refused(answer).await,
            (404, "unknown-key".to_owned()),
            "{id}"
        );
    }
}

#[tokio::test]
async fn an_operator_key_cannot_be_revoked_from_a_tenant() {
    let s = server().await;
    let token = administrator(&s);
    let operator = s
        .tenants
        .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
        .expect("an operator key")
        .token;
    let path = format!("/tenant/keys/{}", key_id(&operator));
    let answer = send(&s, Method::DELETE, &path, &token, None).await;
    assert_eq!(
        refused(answer).await,
        (404, "unknown-key".to_owned()),
        "not a key of the tenant"
    );
    assert_eq!(
        get(&s, "/admin/tenants", &operator).await.status().as_u16(),
        200,
        "the operator's key still works"
    );
}

#[tokio::test]
async fn the_routes_are_not_served_on_the_operator_listener() {
    let s = server_on(Plane::Operator).await;
    let token = administrator(&s);
    for path in ["/tenant/settings", "/tenant/keys", "/tenant/memberships"] {
        assert_eq!(
            refused(get(&s, path, &token).await).await,
            (403, "wrong-listener".to_owned()),
            "GET {path}: a tenant key at the operator listener"
        );
    }
}

#[tokio::test]
async fn a_method_the_routes_do_not_take_is_not_allowed() {
    let s = server().await;
    let token = administrator(&s);
    for (method, path) in [
        (Method::POST, "/tenant/settings"),
        (Method::DELETE, "/tenant/settings"),
        (Method::PUT, "/tenant/keys"),
        (Method::GET, "/tenant/keys/0123456789abcdef"),
        (Method::POST, "/tenant/memberships"),
    ] {
        let answer = send(&s, method.clone(), path, &token, None).await;
        assert_eq!(answer.status().as_u16(), 405, "{method} {path}");
    }
}

// Limits and odd inputs.

#[tokio::test]
async fn an_empty_change_answers_the_settings_as_they_are() {
    let s = server().await;
    let token = administrator(&s);
    let (status, body) = answered(
        send(
            &s,
            Method::PUT,
            "/tenant/settings",
            &token,
            Some(&json!({})),
        )
        .await,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["session.idle_seconds"], json!(900), "as it was");
}

#[tokio::test]
async fn one_tenants_change_does_not_reach_another_tenants_settings() {
    let s = server().await;
    let token = administrator(&s);
    let (_, theirs) = other_tenant(&s);
    let change = json!({ "masking.reader": false });
    send(&s, Method::PUT, "/tenant/settings", &token, Some(&change)).await;
    let (_, read) = answered(get(&s, "/tenant/settings", &theirs).await).await;
    assert_eq!(
        read["masking.reader"],
        json!(true),
        "the other tenant's default"
    );
}

#[tokio::test]
async fn one_tenants_lists_hold_nothing_of_another_tenant() {
    let s = server().await;
    let token = administrator(&s);
    let (other, theirs) = other_tenant(&s);
    let user = s
        .tenants
        .add_user(Actor::Cli, "Theirs", None)
        .expect("a user");
    s.tenants
        .grant_membership(Actor::Cli, user, Some(other), Roles::of(&[Role::Reader]))
        .expect("granted");
    let (_, keys) = answered(get(&s, "/tenant/keys", &token).await).await;
    assert!(
        keys.as_array()
            .expect("a list")
            .iter()
            .all(|k| k["id"] != json!(key_id(&theirs))),
        "the other tenant's key is not listed: {keys}"
    );
    let (_, members) = answered(get(&s, "/tenant/memberships", &token).await).await;
    assert_eq!(members, json!([]), "nor its member");
}

#[tokio::test]
async fn an_administrator_may_revoke_the_key_it_is_calling_with() {
    let s = server().await;
    let token = administrator(&s);
    let path = format!("/tenant/keys/{}", key_id(&token));
    let answer = send(&s, Method::DELETE, &path, &token, None).await;
    assert_eq!(answer.status().as_u16(), 204, "revoked");
    assert_eq!(
        get(&s, "/tenant/settings", &token).await.status().as_u16(),
        401,
        "and its next request is refused"
    );
}

#[tokio::test]
async fn revoking_a_key_twice_is_204_both_times() {
    let s = server().await;
    let token = administrator(&s);
    let path = format!("/tenant/keys/{}", key_id(&s.key(&[Scope::Read])));
    for time in ["once", "twice"] {
        let answer = send(&s, Method::DELETE, &path, &token, None).await;
        assert_eq!(answer.status().as_u16(), 204, "{time}");
    }
}

#[tokio::test]
async fn a_change_larger_than_the_body_limit_is_too_large() {
    let s = server().await;
    let token = administrator(&s);
    let change = json!({ "masking.reader": "x".repeat(2 * 1024 * 1024) });
    let answer = send(&s, Method::PUT, "/tenant/settings", &token, Some(&change)).await;
    assert_eq!(
        answer.status().as_u16(),
        413,
        "the limit of every body holds here too"
    );
}

#[tokio::test]
async fn a_label_with_any_text_is_kept_as_it_was_sent() {
    let s = server().await;
    let token = administrator(&s);
    let label = "reports \u{2014} \"nightly\" <a&b>";
    let new = json!({ "scopes": ["read"], "label": label });
    send(&s, Method::POST, "/tenant/keys", &token, Some(&new)).await;
    let (_, keys) = answered(get(&s, "/tenant/keys", &token).await).await;
    assert_eq!(keys[1]["label"], json!(label), "the label, unchanged");
}

#[tokio::test]
async fn a_tenant_with_no_membership_lists_an_empty_array() {
    let s = server().await;
    let (status, body) = answered(get(&s, "/tenant/memberships", &administrator(&s)).await).await;
    assert_eq!((status, body), (200, json!([])), "an empty list");
}

#[tokio::test]
async fn a_member_whose_roles_the_cap_leaves_out_is_listed_with_no_scope() {
    let s = server().await;
    let user = s
        .tenants
        .add_user(Actor::Cli, "Legal", None)
        .expect("a user");
    s.tenants
        .grant_membership(Actor::Cli, user, Some(s.tenant), Roles::of(&[Role::Legal]))
        .expect("granted");
    s.tenants
        .set_role_cap(Actor::Cli, s.tenant, Some(Roles::of(&[Role::Reader])))
        .expect("a cap");
    let (_, body) = answered(get(&s, "/tenant/memberships", &administrator(&s)).await).await;
    assert_eq!(body[0]["roles"], json!(["legal"]), "the role as granted");
    assert_eq!(body[0]["scopes"], json!([]), "no scope under the cap");
}
