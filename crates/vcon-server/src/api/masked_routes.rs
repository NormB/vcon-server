//! The masked view on the tenant read route: `?view=masked` answers the
//! vCon's summary (src/masked.rs) in place of the vCon, recorded as
//! `read_masked`, with the read's scope, tag and refusals.

use reqwest::Method;
use serde_json::Value;

use super::description::document;
use super::reveal_routes::{UUID, a4, bare, call, get, gzip, recorded, stored, stored_as};
use super::tests::pipeline::{Server, problem, server};
use crate::access::Scope;
use crate::masked::{self, Summary};

/// What a masked read answers: status, media type, tag and body.
async fn answered(s: &Server, path: &str, token: &str) -> (u16, String, String, Vec<u8>) {
    let answer = get(s, path, token).await;
    let header = |name: &str| {
        answer
            .headers()
            .get(name)
            .map(|v| v.to_str().expect("ascii").to_owned())
            .unwrap_or_default()
    };
    let (status, media, tag) = (
        answer.status().as_u16(),
        header("content-type"),
        header("etag"),
    );
    (
        status,
        media,
        tag,
        answer.bytes().await.expect("body").to_vec(),
    )
}

fn masked_path() -> String {
    format!("/vcons/{UUID}?view=masked")
}

/// The last record of the tenant's log.
fn last(s: &Server) -> (String, u64, Value, Value) {
    recorded(s).last().expect("a record").clone()
}

// Success.

#[tokio::test]
async fn the_masked_view_is_the_summary_of_the_stored_vcon_with_its_tag() {
    let s = server().await;
    let tag = stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    let (status, media, got_tag, body) = answered(&s, &masked_path(), &reader).await;
    assert_eq!(status, 200, "the status");
    assert_eq!(media, "application/json", "the media type");
    assert_eq!(got_tag, tag, "the tag of the plain read");
    let got: Summary = serde_json::from_slice(&body).expect("a summary");
    let wanted = masked::summary(&String::from_utf8(a4()).expect("text"));
    assert_eq!(got, wanted, "the rule of src/masked.rs");
    assert_eq!(got.parties.len(), 2, "A.4's two parties");
}

#[tokio::test]
async fn the_masked_view_is_recorded_as_read_masked_with_the_version() {
    let s = server().await;
    let tag = stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    answered(&s, &masked_path(), &reader).await;
    assert_eq!(
        last(&s),
        ("read_masked".to_owned(), 200, UUID.into(), bare(&tag)),
        "the record"
    );
}

#[tokio::test]
async fn the_masked_view_holds_no_name_or_number_of_the_vcon() {
    let s = server().await;
    stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    let (_, _, _, body) = answered(&s, &masked_path(), &reader).await;
    let text = String::from_utf8(body).expect("text");
    for hidden in ["Alice", "Bob", "2345678901", "9876543210"] {
        assert!(!text.contains(hidden), "{hidden} is in {text}");
    }
}

#[tokio::test]
async fn a_vcon_stored_gzipped_is_summarized_from_its_inflated_text() {
    let s = server().await;
    let tag = stored_as(&s, "application/vcon+gzip", gzip(&a4())).await;
    let reader = s.key(&[Scope::Read]);
    let (status, media, got_tag, body) = answered(&s, &masked_path(), &reader).await;
    assert_eq!(
        (status, media.as_str(), got_tag),
        (200, "application/json", tag),
        "the answer"
    );
    let got: Summary = serde_json::from_slice(&body).expect("a summary");
    assert_eq!(got.form, "unsigned", "read after inflating");
    assert_eq!(got.parties.len(), 2, "A.4's two parties");
}

#[tokio::test]
async fn the_plain_read_and_the_reveals_are_unchanged_beside_the_view() {
    let s = server().await;
    stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    let (status, media, _, body) = answered(&s, &format!("/vcons/{UUID}"), &reader).await;
    assert_eq!(
        (status, media.as_str()),
        (200, "application/vcon"),
        "the plain read"
    );
    assert_eq!(body, a4(), "the stored bytes");
    let (status, _, _, body) = answered(&s, &format!("/vcons/{UUID}?reveal=all"), &reader).await;
    assert_eq!((status, body), (200, a4()), "a reveal");
    assert_eq!(last(&s).0, "reveal_all", "recorded as the reveal");
}

// Failure.

#[tokio::test]
async fn a_view_other_than_masked_is_a_bad_query_recorded_as_a_read() {
    let s = server().await;
    stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    for bad in ["view=whole", "view=", "view", "view=masked&view=masked"] {
        let answer = get(&s, &format!("/vcons/{UUID}?{bad}"), &reader).await;
        assert_eq!(answer.status(), 400, "{bad}");
        assert_eq!(problem(answer).await, "bad-query", "{bad}");
        assert_eq!(
            last(&s),
            ("read".to_owned(), 400, Value::Null, Value::Null),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn the_view_and_a_reveal_in_one_read_are_a_bad_query() {
    let s = server().await;
    stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    for bad in ["view=masked&reveal=all", "reveal=parties&view=masked"] {
        let answer = get(&s, &format!("/vcons/{UUID}?{bad}"), &reader).await;
        assert_eq!(answer.status(), 400, "{bad}");
        assert_eq!(problem(answer).await, "bad-query", "{bad}");
        assert_eq!(last(&s).0, "read", "{bad}");
    }
}

/// The view asks for nothing a key does not hold, and needs the read
/// scope as the plain read does.
#[tokio::test]
async fn a_key_without_read_is_forbidden_and_recorded_as_read_masked() {
    let s = server().await;
    stored(&s).await;
    let writer = s.key(&[Scope::Write]);
    let answer = get(&s, &masked_path(), &writer).await;
    assert_eq!(answer.status(), 403, "the status");
    assert_eq!(
        last(&s),
        ("read_masked".to_owned(), 403, Value::Null, Value::Null),
        "the record"
    );
}

#[tokio::test]
async fn a_uuid_the_tenant_does_not_hold_is_not_found() {
    let s = server().await;
    let reader = s.key(&[Scope::Read]);
    let answer = get(&s, &masked_path(), &reader).await;
    assert_eq!(answer.status(), 404, "the status");
    assert_eq!(problem(answer).await, "not-found", "the problem");
    assert_eq!(
        last(&s),
        ("read_masked".to_owned(), 404, UUID.into(), Value::Null),
        "the record"
    );
}

#[tokio::test]
async fn a_deleted_vcon_is_gone() {
    let s = server().await;
    stored(&s).await;
    let deleter = s.key(&[Scope::Delete]);
    let deleted = call(&s, Method::DELETE, &format!("/vcons/{UUID}"), &deleter).await;
    assert_eq!(deleted.status(), 204, "deleted");
    let reader = s.key(&[Scope::Read]);
    let answer = get(&s, &masked_path(), &reader).await;
    assert_eq!(answer.status(), 410, "the status");
    assert_eq!(last(&s).0, "read_masked", "the record");
}

#[tokio::test]
async fn a_path_uuid_not_in_text_form_is_refused_and_recorded_as_read_masked() {
    let s = server().await;
    let reader = s.key(&[Scope::Read]);
    let answer = get(&s, "/vcons/not-a-uuid?view=masked", &reader).await;
    assert_eq!(answer.status(), 400, "the status");
    assert_eq!(problem(answer).await, "bad-path-uuid", "the problem");
    assert_eq!(last(&s).0, "read_masked", "the record");
}

// Edge.

/// `Vcon-State` belongs to the legal reads: the tenant read, plain or
/// masked, answers only a live vCon and names no state.
#[tokio::test]
async fn the_tenant_masked_view_names_no_state() {
    let s = server().await;
    stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    for path in [masked_path(), format!("/vcons/{UUID}")] {
        let answer = get(&s, &path, &reader).await;
        assert_eq!(answer.status(), 200, "{path}");
        assert!(
            answer.headers().get("vcon-state").is_none(),
            "{path}: {:?}",
            answer.headers().get("vcon-state")
        );
    }
}

#[tokio::test]
async fn an_encoded_view_is_the_view() {
    let s = server().await;
    stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    let (status, media, _, _) =
        answered(&s, &format!("/vcons/{UUID}?%76iew=mask%65d&"), &reader).await;
    assert_eq!(
        (status, media.as_str()),
        (200, "application/json"),
        "the answer"
    );
    assert_eq!(last(&s).0, "read_masked", "the record");
}

#[tokio::test]
async fn after_a_replace_the_view_is_of_the_new_version() {
    let s = server().await;
    let old = stored(&s).await;
    let mut vcon: Value = serde_json::from_slice(&a4()).expect("JSON");
    vcon["subject"] = "".into();
    let answer = s
        .client()
        .put(s.url(&format!("/vcons/{UUID}")))
        .bearer_auth(s.key(&[Scope::Write]))
        .header("content-type", "application/vcon")
        .header("if-match", &old)
        .body(vcon.to_string())
        .send()
        .await
        .expect("an answer");
    assert_eq!(answer.status(), 200, "replaced");
    let new = answer.headers()["etag"].to_str().expect("tag").to_owned();
    let reader = s.key(&[Scope::Read]);
    let (_, _, tag, body) = answered(&s, &masked_path(), &reader).await;
    assert_eq!(tag, new, "the new version's tag");
    let got: Summary = serde_json::from_slice(&body).expect("a summary");
    assert_eq!(got.subject, "none", "the new version's subject");
}

#[test]
fn the_description_names_the_view_on_the_tenant_read() {
    let doc = document();
    let view = &doc["components"]["parameters"]["view"];
    assert_eq!(view["name"], "view", "the name");
    assert_eq!(view["in"], "query", "a query parameter");
    assert_eq!(
        view["schema"]["enum"],
        serde_json::json!(["masked"]),
        "its one value"
    );
    let names = |path: &str| {
        doc["paths"][path]["get"]["parameters"]
            .as_array()
            .is_some_and(|all| {
                all.iter()
                    .any(|p| p["$ref"] == "#/components/parameters/view")
            })
    };
    assert!(names("/vcons/{uuid}"), "the tenant read takes it");
    assert!(!names("/vcons"), "the list does not");
}

/// The schema of the answer names each field the code writes, all required.
#[test]
fn the_described_answer_has_the_fields_of_the_summary() {
    let doc = document();
    let schema = &doc["components"]["schemas"]["MaskedVcon"];
    let made = serde_json::to_value(masked::summary("{}")).expect("JSON");
    let mut fields: Vec<&str> = made
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    let mut required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required fields")
        .iter()
        .map(|f| f.as_str().expect("a name"))
        .collect();
    let mut described: Vec<&str> = schema["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    required.sort_unstable();
    described.sort_unstable();
    assert_eq!(required, fields, "every field is required");
    assert_eq!(described, fields, "every field is described");
    let json =
        &doc["paths"]["/vcons/{uuid}"]["get"]["responses"]["200"]["content"]["application/json"];
    assert_eq!(
        json["schema"]["$ref"], "#/components/schemas/MaskedVcon",
        "the answer's schema"
    );
}
