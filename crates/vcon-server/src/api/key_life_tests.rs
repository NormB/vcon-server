//! A key's expiry and last-used time over HTTPS: set where a key is made
//! (`POST /tenant/keys` and `POST /admin/tenants/{id}/keys`), shown where
//! keys are listed, and an expired key refused with 401 `key-expired`.
//!
//! No assertion message here prints a token: a key is named by its label
//! or its place.

use reqwest::Method;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use super::tests::pipeline::{Server, problem, server};
use crate::access::{Scope, Scopes};
use crate::audit::Actor;
use crate::tenants::KeyLife;

fn administrator(s: &Server) -> String {
    s.key(&[Scope::TenantAdmin])
}

fn operator(s: &Server) -> String {
    s.tenants
        .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
        .expect("an operator key")
        .token
}

/// A key of the server's tenant made to live `life`.
fn key_for(s: &Server, scopes: &[Scope], life: KeyLife) -> String {
    s.tenants
        .create_key_for(
            Actor::Cli,
            Some(s.tenant),
            Scopes::of(scopes),
            "dated",
            life,
        )
        .expect("a key")
        .token
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

/// Makes a key by the tenant route with `fields` added to a body that
/// asks for the read scope, and gives the answer.
async fn made(s: &Server, token: &str, fields: &Value) -> reqwest::Response {
    let mut body = json!({"scopes": ["read"], "label": "reports"});
    for (name, value) in fields.as_object().expect("an object") {
        body[name.as_str()] = value.clone();
    }
    send(s, Method::POST, "/tenant/keys", token, Some(&body)).await
}

/// The keys the tenant route lists that have this label.
async fn listed(s: &Server, token: &str, label: &str) -> Vec<Value> {
    let (status, body) = answered(get(s, "/tenant/keys", token).await).await;
    assert_eq!(status, 200, "the list");
    body.as_array()
        .expect("an array of keys")
        .iter()
        .filter(|k| k["label"] == label)
        .cloned()
        .collect()
}

fn time(value: &Value) -> OffsetDateTime {
    OffsetDateTime::parse(value.as_str().expect("a time as text"), &Rfc3339).expect("RFC 3339")
}

/// The second a key was made in. A key's expiry is a whole second: its
/// life is counted from here.
fn made_in(value: &Value) -> OffsetDateTime {
    time(value).replace_nanosecond(0).expect("zero is in range")
}

/// How many keys the server's tenant has.
fn keys(s: &Server) -> usize {
    s.tenants
        .list_keys(Actor::Cli, Some(s.tenant))
        .expect("listed")
        .len()
}

/// The statuses of the records of `action` in the tenant's log.
fn statuses(s: &Server, action: &str) -> Vec<Value> {
    s.tenant_log(action)
        .into_iter()
        .map(|r| r["status"].clone())
        .collect()
}

// What must work.

#[tokio::test]
async fn a_key_made_with_no_expiry_field_expires_90_days_after_it_is_made() {
    let s = server().await;
    let token = administrator(&s);
    let (status, body) = answered(made(&s, &token, &json!({})).await).await;
    assert_eq!(status, 201, "made");
    let shown = listed(&s, &token, "reports").await;
    assert_eq!(shown.len(), 1, "one key with the label");
    assert_eq!(
        time(&shown[0]["expires_at"]) - made_in(&shown[0]["created_at"]),
        Duration::days(90),
        "the default life"
    );
    assert_eq!(
        body["expires_at"], shown[0]["expires_at"],
        "the answer that makes the key tells its expiry"
    );
}

#[tokio::test]
async fn a_key_made_for_a_number_of_days_expires_then() {
    let s = server().await;
    let token = administrator(&s);
    for days in [1, 30, 365] {
        let fields = json!({"expires_in_days": days});
        let (status, body) = answered(made(&s, &token, &fields).await).await;
        assert_eq!(status, 201, "{days} days");
        let shown = listed(&s, &token, "reports").await;
        let last = shown.last().expect("the key just made");
        assert_eq!(
            time(&last["expires_at"]) - made_in(&last["created_at"]),
            Duration::days(days),
            "{days} days"
        );
        assert_eq!(body["expires_at"], last["expires_at"], "{days} days");
    }
}

#[tokio::test]
async fn the_answer_that_makes_a_key_has_one_length_whenever_the_key_is_made() {
    let s = server().await;
    let token = administrator(&s);
    let start = s
        .tenants
        .now()
        .replace_nanosecond(0)
        .expect("zero is in range");
    let mut lengths = Vec::new();
    for nanos in [0_i64, 1, 120_000_000, 123_456_789, 999_999_999] {
        s.clock.set(start + Duration::nanoseconds(nanos));
        let answer = made(&s, &token, &json!({})).await;
        assert_eq!(answer.status().as_u16(), 201, "made at {nanos} ns");
        let bytes = answer.bytes().await.expect("a body");
        let body: Value = serde_json::from_slice(&bytes).expect("JSON");
        let expiry = body["expires_at"].as_str().expect("a time as text");
        assert_eq!(
            expiry.len(),
            "2027-02-10T12:00:00Z".len(),
            "a whole second, with no fraction, at {nanos} ns: {expiry}"
        );
        lengths.push(bytes.len());
    }
    assert_eq!(
        lengths, [136; 5],
        "the answer is 136 bytes whatever part of a second the key is made in"
    );
}

#[tokio::test]
async fn a_key_made_with_no_expiry_shows_null() {
    let s = server().await;
    let token = administrator(&s);
    let (status, body) = answered(made(&s, &token, &json!({"no_expiry": true})).await).await;
    assert_eq!(status, 201, "made");
    assert!(body["expires_at"].is_null(), "the answer says no expiry");
    let shown = listed(&s, &token, "reports").await;
    assert!(shown[0]["expires_at"].is_null(), "and so does the list");
}

#[tokio::test]
async fn the_answer_that_makes_a_key_is_still_not_stored() {
    let s = server().await;
    let token = administrator(&s);
    let answer = made(&s, &token, &json!({"expires_in_days": 7})).await;
    assert_eq!(answer.status().as_u16(), 201, "made");
    assert_eq!(
        answer.headers()["cache-control"],
        "no-store",
        "the answer holds a token"
    );
}

#[tokio::test]
async fn a_list_shows_when_each_key_was_last_used() {
    let s = server().await;
    let token = administrator(&s);
    let (status, _) = answered(made(&s, &token, &json!({})).await).await;
    assert_eq!(status, 201, "made");
    let unused = listed(&s, &token, "reports").await;
    assert!(
        unused[0]["last_used_at"].is_null(),
        "a key no request has used"
    );
    let own = listed(&s, &token, "test").await;
    let used = time(&own[0]["last_used_at"]);
    assert_eq!(
        (used.second(), used.nanosecond()),
        (0, 0),
        "the administrator's own key, to the minute"
    );
    assert!(
        used >= time(&own[0]["created_at"]) - Duration::minutes(1),
        "in the minute it was used"
    );
}

#[tokio::test]
async fn the_operator_makes_a_tenants_key_with_an_expiry_and_lists_it() {
    let s = server().await;
    let op = operator(&s);
    let path = format!("/admin/tenants/{}/keys", s.tenant);
    let body = json!({"scopes": ["read"], "label": "year", "expires_in_days": 365});
    let (status, made) = answered(send(&s, Method::POST, &path, &op, Some(&body)).await).await;
    assert_eq!(status, 201, "made");
    let (status, list) = answered(get(&s, &path, &op).await).await;
    assert_eq!(status, 200, "listed");
    let key = &list[0];
    assert_eq!(
        time(&key["expires_at"]) - made_in(&key["created_at"]),
        Duration::days(365),
        "one year"
    );
    assert_eq!(made["expires_at"], key["expires_at"], "as the answer said");
    assert!(key["last_used_at"].is_null(), "never used");
}

#[tokio::test]
async fn the_operator_makes_a_tenants_key_with_no_expiry() {
    let s = server().await;
    let op = operator(&s);
    let path = format!("/admin/tenants/{}/keys", s.tenant);
    let body = json!({"scopes": ["read"], "label": "forever", "no_expiry": true});
    let (status, made) = answered(send(&s, Method::POST, &path, &op, Some(&body)).await).await;
    assert_eq!(status, 201, "made");
    assert!(made["expires_at"].is_null(), "no expiry");
}

#[tokio::test]
async fn a_key_works_until_just_before_its_expiry() {
    let s = server().await;
    let token = key_for(&s, &[Scope::TenantAdmin], KeyLife::Days(1));
    let own = listed(&s, &token, "dated").await;
    s.clock
        .set(time(&own[0]["expires_at"]) - Duration::seconds(1));
    assert_eq!(
        get(&s, "/tenant/keys", &token).await.status().as_u16(),
        200,
        "one second before the expiry"
    );
}

// What must be refused.

#[tokio::test]
async fn an_expired_key_is_refused_with_401_key_expired_and_a_bearer_challenge() {
    let s = server().await;
    let token = key_for(&s, &[Scope::TenantAdmin], KeyLife::Days(1));
    let own = listed(&s, &token, "dated").await;
    s.clock.set(time(&own[0]["expires_at"]));
    let answer = get(&s, "/tenant/keys", &token).await;
    assert_eq!(
        answer.headers()["www-authenticate"],
        "Bearer",
        "the route takes a key"
    );
    assert_eq!(
        refused(answer).await,
        (401, "key-expired".to_owned()),
        "at the instant of the expiry"
    );
}

#[tokio::test]
async fn an_expired_key_makes_no_key() {
    let s = server().await;
    let token = key_for(&s, &[Scope::TenantAdmin], KeyLife::Days(1));
    let before = keys(&s);
    s.clock.set(s.tenants.now() + Duration::days(2));
    assert_eq!(
        refused(made(&s, &token, &json!({})).await).await,
        (401, "key-expired".to_owned()),
        "refused"
    );
    assert_eq!(keys(&s), before, "no key was made");
}

#[tokio::test]
async fn an_expired_operator_key_is_refused_on_the_admin_routes() {
    let s = server().await;
    let op = operator(&s);
    s.clock.set(s.tenants.now() + Duration::days(90));
    let path = format!("/admin/tenants/{}/keys", s.tenant);
    assert_eq!(
        refused(get(&s, &path, &op).await).await,
        (401, "key-expired".to_owned()),
        "90 days after the operator key was made"
    );
}

#[tokio::test]
async fn an_expired_keys_refusal_is_recorded_as_a_failed_authentication() {
    let s = server().await;
    let token = key_for(&s, &[Scope::Read], KeyLife::Days(1));
    s.clock.set(s.tenants.now() + Duration::days(2));
    let before = statuses(&s, "auth_failed").len();
    assert_eq!(
        refused(get(&s, "/key", &token).await).await.0,
        401,
        "refused"
    );
    let after = statuses(&s, "auth_failed");
    assert_eq!(after.len(), before + 1, "one record in the tenant's log");
    assert_eq!(after.last(), Some(&json!(401)), "with the status");
}

#[tokio::test]
async fn a_wrong_secret_for_an_expired_key_is_plain_unauthorized() {
    let s = server().await;
    let mut token = key_for(&s, &[Scope::Read], KeyLife::Days(1));
    // The first character of the secret, after `vcs_`, the id and `_`.
    let first = token.remove(21);
    token.insert(21, if first == 'A' { 'B' } else { 'A' });
    s.clock.set(s.tenants.now() + Duration::days(2));
    assert_eq!(
        refused(get(&s, "/key", &token).await).await,
        (401, "unauthorized".to_owned()),
        "the expiry is told only to the holder of the secret"
    );
}

#[tokio::test]
async fn a_key_revoked_and_expired_is_plain_unauthorized() {
    let s = server().await;
    let token = key_for(&s, &[Scope::Read], KeyLife::Days(1));
    let id = crate::access::parse(&token).expect("a token");
    s.tenants.revoke_key(Actor::Cli, id).expect("revoked");
    s.clock.set(s.tenants.now() + Duration::days(2));
    assert_eq!(
        refused(get(&s, "/key", &token).await).await,
        (401, "unauthorized".to_owned()),
        "as any revoked key"
    );
}

#[tokio::test]
async fn days_outside_1_to_365_are_422_bad_expiry_and_recorded() {
    let s = server().await;
    let token = administrator(&s);
    let before = keys(&s);
    for days in [0, -1, 366, 100_000] {
        assert_eq!(
            refused(made(&s, &token, &json!({"expires_in_days": days})).await).await,
            (422, "bad-expiry".to_owned()),
            "{days} days"
        );
    }
    assert_eq!(keys(&s), before, "no key was made");
    assert_eq!(
        statuses(&s, "key_create")
            .iter()
            .filter(|status| **status == 422)
            .count(),
        4,
        "each refusal is in the tenant's log"
    );
}

#[tokio::test]
async fn days_and_no_expiry_together_are_422_bad_expiry() {
    let s = server().await;
    let token = administrator(&s);
    let fields = json!({"expires_in_days": 30, "no_expiry": true});
    assert_eq!(
        refused(made(&s, &token, &fields).await).await,
        (422, "bad-expiry".to_owned()),
        "the two contradict each other"
    );
}

#[tokio::test]
async fn no_expiry_is_422_bad_expiry_where_the_configuration_refuses_it() {
    let s = server().await;
    let token = administrator(&s);
    let op = operator(&s);
    s.tenants.allow_no_expiry(false);
    let before = keys(&s);
    assert_eq!(
        refused(made(&s, &token, &json!({"no_expiry": true})).await).await,
        (422, "bad-expiry".to_owned()),
        "by the tenant's administrator"
    );
    let path = format!("/admin/tenants/{}/keys", s.tenant);
    let body = json!({"scopes": ["read"], "label": "forever", "no_expiry": true});
    assert_eq!(
        refused(send(&s, Method::POST, &path, &op, Some(&body)).await).await,
        (422, "bad-expiry".to_owned()),
        "by the operator"
    );
    assert_eq!(keys(&s), before, "no key was made");
    let (status, _) = answered(made(&s, &token, &json!({})).await).await;
    assert_eq!(status, 201, "a key with an expiry is still made");
}

#[tokio::test]
async fn the_operators_route_refuses_days_outside_the_range() {
    let s = server().await;
    let op = operator(&s);
    let path = format!("/admin/tenants/{}/keys", s.tenant);
    for days in [0, 366] {
        let body = json!({"scopes": ["read"], "label": "x", "expires_in_days": days});
        assert_eq!(
            refused(send(&s, Method::POST, &path, &op, Some(&body)).await).await,
            (422, "bad-expiry".to_owned()),
            "{days} days"
        );
    }
}

#[tokio::test]
async fn bad_scopes_are_told_before_a_bad_expiry() {
    let s = server().await;
    let token = administrator(&s);
    let body = json!({"scopes": ["tenant_admin"], "label": "x", "expires_in_days": 0});
    assert_eq!(
        refused(send(&s, Method::POST, "/tenant/keys", &token, Some(&body)).await).await,
        (422, "bad-scopes".to_owned()),
        "the scopes are checked first"
    );
}

// The edges.

#[tokio::test]
async fn an_expiry_field_of_the_wrong_type_or_an_unknown_one_makes_no_key() {
    let s = server().await;
    let token = administrator(&s);
    let before = keys(&s);
    for fields in [
        json!({"expires_in_days": "30"}),
        json!({"expires_in_days": 1.5}),
        json!({"no_expiry": "yes"}),
        json!({"expires_at": "2030-01-01T00:00:00Z"}),
    ] {
        let status = made(&s, &token, &fields).await.status().as_u16();
        assert!(
            (400..500).contains(&status),
            "{fields}: refused as a bad body, with {status}"
        );
    }
    assert_eq!(keys(&s), before, "no key was made");
}

#[tokio::test]
async fn null_expiry_fields_are_as_absent_ones() {
    let s = server().await;
    let token = administrator(&s);
    let fields = json!({"expires_in_days": null, "no_expiry": null});
    let (status, body) = answered(made(&s, &token, &fields).await).await;
    assert_eq!(status, 201, "made");
    assert!(body["expires_at"].is_string(), "with the default expiry");
}

#[tokio::test]
async fn no_expiry_false_with_days_is_days() {
    let s = server().await;
    let token = administrator(&s);
    let fields = json!({"expires_in_days": 2, "no_expiry": false});
    let (status, _) = answered(made(&s, &token, &fields).await).await;
    assert_eq!(status, 201, "made");
    let shown = listed(&s, &token, "reports").await;
    assert_eq!(
        time(&shown[0]["expires_at"]) - made_in(&shown[0]["created_at"]),
        Duration::days(2),
        "two days"
    );
}

#[tokio::test]
async fn a_key_made_by_an_expiring_key_outlives_it() {
    let s = server().await;
    let token = key_for(&s, &[Scope::TenantAdmin], KeyLife::Days(1));
    let fields = json!({"scopes": ["read"], "label": "child", "expires_in_days": 30});
    let (status, child) =
        answered(send(&s, Method::POST, "/tenant/keys", &token, Some(&fields)).await).await;
    assert_eq!(status, 201, "made");
    let child = child["token"].as_str().expect("a token").to_owned();
    s.clock.set(s.tenants.now() + Duration::days(2));
    assert_eq!(
        get(&s, "/key", &token).await.status().as_u16(),
        401,
        "the maker has expired"
    );
    assert_eq!(
        get(&s, "/key", &child).await.status().as_u16(),
        200,
        "the key it made has its own expiry"
    );
}

#[tokio::test]
async fn many_requests_in_a_minute_leave_one_last_used_time() {
    let s = server().await;
    let token = administrator(&s);
    for _ in 0..5 {
        assert_eq!(
            get(&s, "/tenant/keys", &token).await.status().as_u16(),
            200,
            "accepted"
        );
    }
    let own = listed(&s, &token, "test").await;
    let first = time(&own[0]["last_used_at"]);
    s.clock.set(first + Duration::seconds(59));
    let own = listed(&s, &token, "test").await;
    assert_eq!(
        time(&own[0]["last_used_at"]),
        first,
        "59 s into the minute the time is the same"
    );
    s.clock.set(first + Duration::seconds(61));
    assert_eq!(
        get(&s, "/key", &token).await.status().as_u16(),
        200,
        "a request in the next minute"
    );
    let own = listed(&s, &token, "test").await;
    assert_eq!(
        time(&own[0]["last_used_at"]),
        first + Duration::minutes(1),
        "moves the time by one minute"
    );
}
