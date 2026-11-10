//! The reveal marker on the routes that read a vCon: `?reveal=<part>`
//! changes the action a read is recorded under, and nothing a key may
//! read.

use reqwest::Method;
use serde_json::Value;

use super::description::document;
use super::tests::pipeline::{Server, problem, server};
use crate::access::{Scope, Scopes};
use crate::audit::Actor;
use crate::reveal::Part;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

fn a4() -> Vec<u8> {
    std::fs::read(format!(
        "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4")
}

async fn call(s: &Server, method: Method, path: &str, token: &str) -> reqwest::Response {
    s.client()
        .request(method, s.url(path))
        .bearer_auth(token)
        .send()
        .await
        .expect("an answer")
}

async fn get(s: &Server, path: &str, token: &str) -> reqwest::Response {
    call(s, Method::GET, path, token).await
}

/// Stores `body` as `content_type` and returns its entity tag.
async fn stored_as(s: &Server, content_type: &str, body: Vec<u8>) -> String {
    let answer = s
        .client()
        .post(s.url("/vcons"))
        .bearer_auth(s.key(&[Scope::Write]))
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .expect("an answer");
    assert_eq!(answer.status(), 201, "the status");
    answer.headers()["etag"].to_str().expect("tag").to_owned()
}

/// Stores A.4 and returns its entity tag.
async fn stored(s: &Server) -> String {
    stored_as(s, "application/vcon", a4()).await
}

/// What an answer is to its caller: status, media type, tag, state, body.
async fn whole(answer: reqwest::Response) -> (u16, String, String, String, Vec<u8>) {
    let header = |name: &str| {
        answer
            .headers()
            .get(name)
            .map(|v| v.to_str().expect("ascii").to_owned())
            .unwrap_or_default()
    };
    let (status, media, tag, state) = (
        answer.status().as_u16(),
        header("content-type"),
        header("etag"),
        header("vcon-state"),
    );
    (
        status,
        media,
        tag,
        state,
        answer.bytes().await.expect("body").to_vec(),
    )
}

/// The tenant log's records: each one's action, status, target and version.
fn recorded(s: &Server) -> Vec<(String, u64, Value, Value)> {
    s.tenants
        .store(s.tenant)
        .expect("its store")
        .records()
        .into_iter()
        .map(|r| {
            (
                r["action"].as_str().expect("an action").to_owned(),
                r["status"].as_u64().expect("a status"),
                r["target"].clone(),
                r["version"].clone(),
            )
        })
        .collect()
}

/// The version id inside an entity tag, as the log writes it.
fn bare(tag: &str) -> Value {
    tag.trim_matches('"').into()
}

#[tokio::test]
async fn a_marked_read_answers_as_the_read_does_and_is_recorded_as_the_reveal() {
    let s = server().await;
    let tag = stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    let plain = whole(get(&s, &format!("/vcons/{UUID}"), &reader).await).await;
    assert_eq!(plain.0, 200, "the status");
    for part in Part::ALL {
        let path = format!("/vcons/{UUID}?reveal={}", part.name());
        assert_eq!(whole(get(&s, &path, &reader).await).await, plain, "{path}");
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &(
                format!("reveal_{}", part.name()),
                200,
                UUID.into(),
                bare(&tag)
            ),
            "{path}"
        );
    }
}

fn gzip(body: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body).expect("gzip");
    encoder.finish().expect("gzip")
}

/// The marker does not change how the body is sent: a vCon stored gzipped
/// is revealed gzipped, the bytes that were stored.
#[tokio::test]
async fn a_marked_read_of_a_gzipped_vcon_answers_the_stored_bytes() {
    let s = server().await;
    let sent = gzip(&a4());
    let tag = stored_as(&s, "application/vcon+gzip", sent.clone()).await;
    let reader = s.key(&[Scope::Read]);
    let answer = get(&s, &format!("/vcons/{UUID}?reveal=all"), &reader).await;
    assert_eq!(
        whole(answer).await,
        (
            200,
            "application/vcon+gzip".to_owned(),
            tag.clone(),
            String::new(),
            sent
        ),
        "the answer"
    );
    assert_eq!(
        recorded(&s).last().expect("a record"),
        &("reveal_all".to_owned(), 200, UUID.into(), bare(&tag)),
        "the record"
    );
}

#[tokio::test]
async fn a_marked_legal_read_answers_as_the_legal_read_does_and_is_recorded_as_the_reveal() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let path = format!("/legal/vcons/{UUID}");
    let plain = whole(get(&s, &path, &legal).await).await;
    assert_eq!((plain.0, plain.3.as_str()), (200, "live"), "the status");
    for part in Part::ALL {
        let marked = format!("{path}?reveal={}", part.name());
        assert_eq!(
            whole(get(&s, &marked, &legal).await).await,
            plain,
            "{marked}"
        );
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &(
                format!("legal_reveal_{}", part.name()),
                200,
                UUID.into(),
                bare(&tag)
            ),
            "{marked}"
        );
    }
}

#[tokio::test]
async fn a_marked_read_of_a_kept_version_is_recorded_as_the_reveal_of_that_version() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let path = format!("/legal/vcons/{UUID}/versions/{}", tag.trim_matches('"'));
    let plain = whole(get(&s, &path, &legal).await).await;
    assert_eq!(plain.0, 200, "the status");
    let marked = format!("{path}?reveal=dialog");
    assert_eq!(
        whole(get(&s, &marked, &legal).await).await,
        plain,
        "the answer"
    );
    assert_eq!(
        recorded(&s).last().expect("a record"),
        &(
            "legal_reveal_dialog".to_owned(),
            200,
            UUID.into(),
            bare(&tag)
        ),
        "the record"
    );
}

#[tokio::test]
async fn a_marked_legal_read_of_a_deleted_vcon_says_it_is_deleted() {
    let s = server().await;
    let tag = stored(&s).await;
    let deleter = s.key(&[Scope::Delete]);
    let deleted = call(&s, Method::DELETE, &format!("/vcons/{UUID}"), &deleter).await;
    assert_eq!(deleted.status(), 204, "the status");
    let legal = s.key(&[Scope::Legal]);
    let answer = get(&s, &format!("/legal/vcons/{UUID}?reveal=parties"), &legal).await;
    let got = whole(answer).await;
    assert_eq!(
        (got.0, got.2.as_str(), got.3.as_str()),
        (200, tag.as_str(), "deleted"),
        "its state"
    );
    assert_eq!(got.4, a4(), "the body");
    assert_eq!(
        recorded(&s).last().expect("a record"),
        &(
            "legal_reveal_parties".to_owned(),
            200,
            UUID.into(),
            bare(&tag)
        ),
        "the record"
    );
}

/// The marker asks for nothing a key does not hold: a key without `read`
/// is refused as on the unmarked read, and the record says what it asked.
#[tokio::test]
async fn a_marked_read_by_a_key_without_read_is_forbidden_and_recorded_as_the_reveal() {
    let s = server().await;
    stored(&s).await;
    for scope in [Scope::Write, Scope::Delete, Scope::Legal, Scope::Audit] {
        let token = s.key(&[scope]);
        let answer = get(&s, &format!("/vcons/{UUID}?reveal=all"), &token).await;
        assert_eq!(answer.status(), 403, "{scope:?}");
        assert_eq!(problem(answer).await, "forbidden", "{scope:?}");
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &("reveal_all".to_owned(), 403, Value::Null, Value::Null),
            "{scope:?}"
        );
    }
}

#[tokio::test]
async fn a_marked_legal_read_by_a_key_without_legal_is_forbidden_and_recorded_as_the_reveal() {
    let s = server().await;
    let tag = stored(&s).await;
    let reader = s.key(&[Scope::Read, Scope::Write, Scope::Delete, Scope::Audit]);
    for path in [
        format!("/legal/vcons/{UUID}?reveal=subject"),
        format!(
            "/legal/vcons/{UUID}/versions/{}?reveal=subject",
            tag.trim_matches('"')
        ),
    ] {
        let answer = get(&s, &path, &reader).await;
        assert_eq!(answer.status(), 403, "{path}");
        assert_eq!(problem(answer).await, "forbidden", "{path}");
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &(
                "legal_reveal_subject".to_owned(),
                403,
                Value::Null,
                Value::Null
            ),
            "{path}"
        );
    }
}

/// Markers the route does not take. None is passed over as an unmarked
/// read: each is refused and no body is sent.
const BAD_MARKERS: [&str; 6] = [
    "reveal=everything",
    "reveal=",
    "reveal",
    "reveal=all&reveal=all",
    "reveal=all&limit=1",
    "reveal=Parties",
];

#[tokio::test]
async fn a_read_with_a_marker_it_does_not_take_is_a_bad_query_recorded_as_a_read() {
    let s = server().await;
    stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    for bad in BAD_MARKERS {
        let answer = get(&s, &format!("/vcons/{UUID}?{bad}"), &reader).await;
        assert_eq!(answer.status(), 400, "{bad}");
        assert_eq!(problem(answer).await, "bad-query", "{bad}");
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &("read".to_owned(), 400, Value::Null, Value::Null),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn a_legal_read_with_a_marker_it_does_not_take_is_a_bad_query_recorded_as_a_legal_read() {
    let s = server().await;
    stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    for bad in BAD_MARKERS {
        let answer = get(&s, &format!("/legal/vcons/{UUID}?{bad}"), &legal).await;
        assert_eq!(answer.status(), 400, "{bad}");
        assert_eq!(problem(answer).await, "bad-query", "{bad}");
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &("legal_read".to_owned(), 400, Value::Null, Value::Null),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn a_version_read_with_a_marker_it_does_not_take_is_a_bad_query() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    for bad in BAD_MARKERS {
        let path = format!(
            "/legal/vcons/{UUID}/versions/{}?{bad}",
            tag.trim_matches('"')
        );
        let answer = get(&s, &path, &legal).await;
        assert_eq!(answer.status(), 400, "{bad}");
        assert_eq!(problem(answer).await, "bad-query", "{bad}");
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &("legal_read".to_owned(), 400, Value::Null, Value::Null),
            "{bad}"
        );
    }
}

/// A key without the scope learns nothing from a bad marker: the scope is
/// checked first, as on every route.
#[tokio::test]
async fn a_bad_marker_from_a_key_without_read_is_forbidden_before_it_is_a_bad_query() {
    let s = server().await;
    stored(&s).await;
    let writer = s.key(&[Scope::Write]);
    let answer = get(&s, &format!("/vcons/{UUID}?reveal=everything"), &writer).await;
    assert_eq!(answer.status(), 403, "the status");
    let legal = get(
        &s,
        &format!("/legal/vcons/{UUID}?reveal=everything"),
        &writer,
    )
    .await;
    assert_eq!(legal.status(), 403, "the status");
    let log = recorded(&s);
    assert_eq!(
        log[log.len() - 2..],
        [
            ("read".to_owned(), 403, Value::Null, Value::Null),
            ("legal_read".to_owned(), 403, Value::Null, Value::Null)
        ],
        "the record"
    );
}

#[tokio::test]
async fn a_marked_read_of_a_path_uuid_not_in_text_form_is_recorded_as_the_reveal() {
    let s = server().await;
    let token = s.key(&[Scope::Read, Scope::Legal]);
    for (path, action) in [
        ("/vcons/not-a-uuid?reveal=links", "reveal_links"),
        ("/legal/vcons/not-a-uuid?reveal=links", "legal_reveal_links"),
    ] {
        let answer = get(&s, path, &token).await;
        assert_eq!(answer.status(), 400, "{path}");
        assert_eq!(problem(answer).await, "bad-path-uuid", "{path}");
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &(action.to_owned(), 400, Value::Null, Value::Null),
            "{path}"
        );
    }
}

#[tokio::test]
async fn a_marked_read_of_a_version_not_in_text_form_is_recorded_as_the_reveal() {
    let s = server().await;
    stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let path = format!("/legal/vcons/{UUID}/versions/not-a-version?reveal=analysis");
    let answer = get(&s, &path, &legal).await;
    assert_eq!(answer.status(), 400, "the status");
    assert_eq!(
        problem(answer).await,
        "bad-path-version",
        "the problem type"
    );
    assert_eq!(
        recorded(&s).last().expect("a record"),
        &(
            "legal_reveal_analysis".to_owned(),
            400,
            Value::Null,
            Value::Null
        ),
        "the record"
    );
}

/// A key of another tenant marks a read of this tenant's uuid: it finds
/// nothing, its own tenant's log holds the reveal, and this tenant's log
/// holds no record of it.
#[tokio::test]
async fn a_marked_read_from_another_tenant_finds_nothing_and_leaves_this_log_alone() {
    let s = server().await;
    stored(&s).await;
    let before = recorded(&s);
    let other = s
        .tenants
        .create(Actor::Cli, "other", None)
        .expect("a tenant");
    let theirs = s
        .tenants
        .create_key(Actor::Cli, Some(other), Scopes::of(&[Scope::Read]), "test")
        .expect("a key")
        .token;
    let answer = get(&s, &format!("/vcons/{UUID}?reveal=all"), &theirs).await;
    assert_eq!(answer.status(), 404, "the status");
    assert_eq!(problem(answer).await, "not-found", "the problem type");
    assert_eq!(recorded(&s), before, "this tenant's log");
    let their_log = s.tenants.store(other).expect("its store").records();
    let last = their_log.last().expect("a record");
    assert_eq!(
        (last["action"].as_str(), last["status"].as_u64()),
        (Some("reveal_all"), Some(404)),
        "the record"
    );
}

#[tokio::test]
async fn a_percent_encoded_marker_over_http_is_recorded_as_the_reveal() {
    let s = server().await;
    let tag = stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    let answer = get(&s, &format!("/vcons/{UUID}?%72eveal=%6Cinks"), &reader).await;
    assert_eq!(answer.status(), 200, "the status");
    assert_eq!(
        recorded(&s).last().expect("a record"),
        &("reveal_links".to_owned(), 200, UUID.into(), bare(&tag)),
        "the record"
    );
}

/// The description names the marker on the three routes that take it,
/// with exactly the parts the server knows.
#[test]
fn the_description_names_the_marker_and_its_parts_on_every_route_that_takes_it() {
    let doc = document();
    let known: Vec<Value> = Part::ALL.into_iter().map(|p| p.name().into()).collect();
    assert_eq!(
        doc["components"]["parameters"]["reveal"]["schema"]["enum"],
        Value::Array(known),
        "the description"
    );
    assert_eq!(
        doc["components"]["parameters"]["reveal"]["in"], "query",
        "the description"
    );
    assert_eq!(
        doc["components"]["parameters"]["reveal"]["name"], "reveal",
        "the description"
    );
    for path in [
        "/vcons/{uuid}",
        "/legal/vcons/{uuid}",
        "/legal/vcons/{uuid}/versions/{version}",
    ] {
        let listed = doc["paths"][path]["get"]["parameters"]
            .as_array()
            .is_some_and(|all| {
                all.iter()
                    .any(|p| p["$ref"] == "#/components/parameters/reveal")
            });
        assert!(listed, "{path}");
    }
}

#[tokio::test]
async fn a_read_marked_as_audio_is_recorded_as_a_reveal_of_a_recording() {
    let s = server().await;
    let tag = stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    let plain = whole(get(&s, &format!("/vcons/{UUID}"), &reader).await).await;
    let marked = whole(get(&s, &format!("/vcons/{UUID}?reveal=audio"), &reader).await).await;
    assert_eq!(marked, plain, "the answer");
    assert_eq!(
        recorded(&s).last().expect("a record"),
        &("reveal_audio".to_owned(), 200, UUID.into(), bare(&tag)),
        "the record"
    );
}

#[tokio::test]
async fn a_legal_read_marked_as_audio_is_recorded_as_a_legal_reveal_of_a_recording() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    for path in [
        format!("/legal/vcons/{UUID}?reveal=audio"),
        format!(
            "/legal/vcons/{UUID}/versions/{}?reveal=audio",
            tag.trim_matches('"')
        ),
    ] {
        let answer = get(&s, &path, &legal).await;
        assert_eq!(answer.status(), 200, "{path}");
        assert_eq!(
            recorded(&s).last().expect("a record"),
            &(
                "legal_reveal_audio".to_owned(),
                200,
                UUID.into(),
                bare(&tag)
            ),
            "{path}"
        );
    }
}
