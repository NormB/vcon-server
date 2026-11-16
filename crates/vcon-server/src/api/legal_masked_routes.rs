//! The masked view on the legal read routes: `?view=masked` on
//! `GET /legal/vcons/{uuid}` and `/versions/{version}` answers the read
//! version's summary (src/masked.rs) in place of the vCon, with its tag and
//! `Vcon-State`, recorded as `legal_read_masked`, with the legal scope and
//! the legal read's refusals.

use reqwest::Method;
use serde_json::Value;

use super::description::document;
use super::reveal_routes::{UUID, a4, bare, call, get, gzip, recorded, stored, stored_as};
use super::tests::pipeline::{Server, problem, server};
use crate::access::Scope;
use crate::masked::{self, Summary};
use crate::retention::Retention;

/// What a legal masked read answers: status, media type, tag, state and
/// body.
async fn answered(s: &Server, path: &str, token: &str) -> (u16, String, String, String, Vec<u8>) {
    let answer = get(s, path, token).await;
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

/// The legal read of the current or last kept version, masked.
fn current_path() -> String {
    format!("/legal/vcons/{UUID}?view=masked")
}

/// The legal read of version `tag`, masked.
fn version_path(tag: &str) -> String {
    format!(
        "/legal/vcons/{UUID}/versions/{}?view=masked",
        tag.trim_matches('"')
    )
}

/// The last record of the tenant's log.
fn last(s: &Server) -> (String, u64, Value, Value) {
    recorded(s).last().expect("a record").clone()
}

/// The summary the rule makes of a body.
fn rule(body: &[u8]) -> Summary {
    masked::summary(&masked::text_of(body))
}

/// A4 with its subject replaced, stored over the version `old`; the new
/// version's tag.
async fn replaced(s: &Server, old: &str, subject: &str) -> String {
    let mut vcon: Value = serde_json::from_slice(&a4()).expect("JSON");
    vcon["subject"] = subject.into();
    let answer = s
        .client()
        .put(s.url(&format!("/vcons/{UUID}")))
        .bearer_auth(s.key(&[Scope::Write]))
        .header("content-type", "application/vcon")
        .header("if-match", old)
        .body(vcon.to_string())
        .send()
        .await
        .expect("an answer");
    assert_eq!(answer.status(), 200, "replaced");
    answer.headers()["etag"].to_str().expect("tag").to_owned()
}

async fn deleted(s: &Server) {
    let deleter = s.key(&[Scope::Delete]);
    let answer = call(s, Method::DELETE, &format!("/vcons/{UUID}"), &deleter).await;
    assert_eq!(answer.status(), 204, "deleted");
}

// Success.

#[tokio::test]
async fn the_legal_masked_view_is_the_summary_of_the_current_version_with_its_tag() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let (status, media, got_tag, state, body) = answered(&s, &current_path(), &legal).await;
    assert_eq!(status, 200, "the status");
    assert_eq!(media, "application/json", "the media type");
    assert_eq!(got_tag, tag, "the current version's tag");
    assert_eq!(state, "live", "the uuid's state");
    let got: Summary = serde_json::from_slice(&body).expect("a summary");
    assert_eq!(got, rule(&a4()), "the rule of src/masked.rs");
}

#[tokio::test]
async fn the_legal_masked_view_is_recorded_as_legal_read_masked_with_the_version() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    answered(&s, &current_path(), &legal).await;
    assert_eq!(
        last(&s),
        ("legal_read_masked".to_owned(), 200, UUID.into(), bare(&tag)),
        "the record"
    );
}

#[tokio::test]
async fn a_deleted_vcon_shows_its_last_kept_version_masked_and_says_it_is_deleted() {
    let s = server().await;
    let tag = stored(&s).await;
    deleted(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let (status, media, got_tag, state, body) = answered(&s, &current_path(), &legal).await;
    assert_eq!(
        (status, media.as_str(), got_tag, state.as_str()),
        (200, "application/json", tag.clone(), "deleted"),
        "the answer"
    );
    let got: Summary = serde_json::from_slice(&body).expect("a summary");
    assert_eq!(got, rule(&a4()), "the last kept version's summary");
    assert_eq!(
        last(&s),
        ("legal_read_masked".to_owned(), 200, UUID.into(), bare(&tag)),
        "the record"
    );
}

#[tokio::test]
async fn a_kept_version_is_summarized_as_it_was_stored() {
    let s = server().await;
    let old = stored(&s).await;
    // A.4 has no subject; the new version has one.
    replaced(&s, &old, "A new subject").await;
    let legal = s.key(&[Scope::Legal]);
    let (status, media, got_tag, state, body) = answered(&s, &version_path(&old), &legal).await;
    assert_eq!(
        (status, media.as_str(), got_tag, state.as_str()),
        (200, "application/json", old.clone(), "live"),
        "the answer"
    );
    let got: Summary = serde_json::from_slice(&body).expect("a summary");
    assert_eq!(got.subject, "none", "the old version's subject");
    assert_eq!(got, rule(&a4()), "the old version's summary");
}

#[tokio::test]
async fn a_kept_version_read_masked_is_recorded_with_that_version() {
    let s = server().await;
    let old = stored(&s).await;
    replaced(&s, &old, "A new subject").await;
    let legal = s.key(&[Scope::Legal]);
    answered(&s, &version_path(&old), &legal).await;
    assert_eq!(
        last(&s),
        ("legal_read_masked".to_owned(), 200, UUID.into(), bare(&old)),
        "the record"
    );
}

#[tokio::test]
async fn a_gzipped_vcon_is_summarized_from_its_inflated_text_on_the_legal_route() {
    let s = server().await;
    let tag = stored_as(&s, "application/vcon+gzip", gzip(&a4())).await;
    let legal = s.key(&[Scope::Legal]);
    for path in [current_path(), version_path(&tag)] {
        let (status, media, _, _, body) = answered(&s, &path, &legal).await;
        assert_eq!(
            (status, media.as_str()),
            (200, "application/json"),
            "{path}"
        );
        let got: Summary = serde_json::from_slice(&body).expect("a summary");
        assert_eq!(got, rule(&a4()), "{path}");
    }
}

#[tokio::test]
async fn the_legal_masked_view_holds_no_name_or_number_of_the_vcon() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    for path in [current_path(), version_path(&tag)] {
        let (_, _, _, _, body) = answered(&s, &path, &legal).await;
        let text = String::from_utf8(body).expect("text");
        for hidden in ["Alice", "Bob", "2345678901", "9876543210"] {
            assert!(!text.contains(hidden), "{hidden} is in {text} of {path}");
        }
    }
}

#[tokio::test]
async fn the_plain_legal_read_and_the_legal_reveals_are_unchanged_beside_the_view() {
    let s = server().await;
    stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let path = format!("/legal/vcons/{UUID}");
    let (status, media, _, _, body) = answered(&s, &path, &legal).await;
    assert_eq!(
        (status, media.as_str(), body),
        (200, "application/vcon", a4()),
        "the plain legal read"
    );
    assert_eq!(last(&s).0, "legal_read", "recorded as the legal read");
    let (status, _, _, _, body) = answered(&s, &format!("{path}?reveal=all"), &legal).await;
    assert_eq!((status, body), (200, a4()), "a legal reveal");
    assert_eq!(
        last(&s).0,
        "legal_reveal_all",
        "recorded as the legal reveal"
    );
}

#[test]
fn the_description_names_the_view_on_both_legal_reads() {
    let doc = document();
    let names = |path: &str| {
        doc["paths"][path]["get"]["parameters"]
            .as_array()
            .is_some_and(|all| {
                all.iter()
                    .any(|p| p["$ref"] == "#/components/parameters/view")
            })
    };
    assert!(names("/legal/vcons/{uuid}"), "the legal read takes it");
    assert!(
        names("/legal/vcons/{uuid}/versions/{version}"),
        "the legal read of a version takes it"
    );
    assert!(
        !names("/legal/vcons/{uuid}/versions"),
        "the list of versions does not"
    );
}

#[test]
fn the_description_answers_both_legal_reads_with_the_masked_vcon() {
    let doc = document();
    for path in [
        "/legal/vcons/{uuid}",
        "/legal/vcons/{uuid}/versions/{version}",
    ] {
        let json = &doc["paths"][path]["get"]["responses"]["200"]["content"]["application/json"];
        assert_eq!(
            json["schema"]["$ref"], "#/components/schemas/MaskedVcon",
            "{path}"
        );
    }
}

#[test]
fn the_action_is_named_legal_read_masked_in_the_log() {
    assert_eq!(
        crate::audit::Action::LegalReadMasked.as_str(),
        "legal_read_masked",
        "the name"
    );
}

// Failure.

#[tokio::test]
async fn a_view_other_than_masked_is_a_bad_query_recorded_as_a_legal_read() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    for base in [
        format!("/legal/vcons/{UUID}"),
        format!("/legal/vcons/{UUID}/versions/{}", tag.trim_matches('"')),
    ] {
        for bad in ["view=whole", "view=", "view", "view=masked&view=masked"] {
            let path = format!("{base}?{bad}");
            let answer = get(&s, &path, &legal).await;
            assert_eq!(answer.status(), 400, "{path}");
            assert_eq!(problem(answer).await, "bad-query", "{path}");
            assert_eq!(
                last(&s),
                ("legal_read".to_owned(), 400, Value::Null, Value::Null),
                "{path}"
            );
        }
    }
}

#[tokio::test]
async fn the_view_and_a_reveal_in_one_legal_read_are_a_bad_query() {
    let s = server().await;
    let tag = stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    for base in [
        format!("/legal/vcons/{UUID}"),
        format!("/legal/vcons/{UUID}/versions/{}", tag.trim_matches('"')),
    ] {
        for bad in ["view=masked&reveal=all", "reveal=parties&view=masked"] {
            let path = format!("{base}?{bad}");
            let answer = get(&s, &path, &legal).await;
            assert_eq!(answer.status(), 400, "{path}");
            assert_eq!(problem(answer).await, "bad-query", "{path}");
            assert_eq!(last(&s).0, "legal_read", "{path}");
        }
    }
}

#[tokio::test]
async fn a_key_without_legal_is_forbidden_and_recorded_as_legal_read_masked() {
    let s = server().await;
    let tag = stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    for path in [current_path(), version_path(&tag)] {
        let answer = get(&s, &path, &reader).await;
        assert_eq!(answer.status(), 403, "{path}");
        assert_eq!(
            last(&s),
            (
                "legal_read_masked".to_owned(),
                403,
                Value::Null,
                Value::Null
            ),
            "{path}"
        );
    }
}

#[tokio::test]
async fn a_uuid_the_tenant_never_held_is_not_found() {
    let s = server().await;
    let legal = s.key(&[Scope::Legal]);
    let answer = get(&s, &current_path(), &legal).await;
    assert_eq!(answer.status(), 404, "the status");
    assert_eq!(problem(answer).await, "not-found", "the problem");
    assert_eq!(
        last(&s),
        (
            "legal_read_masked".to_owned(),
            404,
            UUID.into(),
            Value::Null
        ),
        "the record"
    );
}

#[tokio::test]
async fn a_purged_uuid_is_gone_on_both_legal_reads() {
    let s = server().await;
    let tag = stored(&s).await;
    deleted(&s).await;
    s.tenants
        .store(s.tenant)
        .expect("its store")
        .purge_due(Retention::Days(0))
        .expect("swept");
    let legal = s.key(&[Scope::Legal]);
    for path in [current_path(), version_path(&tag)] {
        let answer = get(&s, &path, &legal).await;
        assert_eq!(answer.status(), 410, "{path}");
        assert_eq!(last(&s).0, "legal_read_masked", "{path}");
    }
}

#[tokio::test]
async fn a_version_that_was_never_kept_is_not_found() {
    let s = server().await;
    stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    // A version id in its text form that no version of the uuid has.
    let never = "0199e5c60d7a7c118000000000000001";
    let answer = get(&s, &version_path(never), &legal).await;
    assert_eq!(answer.status(), 404, "the status");
    assert_eq!(
        last(&s),
        (
            "legal_read_masked".to_owned(),
            404,
            UUID.into(),
            Value::Null
        ),
        "the record"
    );
}

#[tokio::test]
async fn a_path_uuid_not_in_text_form_is_refused_and_recorded_as_legal_read_masked() {
    let s = server().await;
    let legal = s.key(&[Scope::Legal]);
    for path in [
        "/legal/vcons/not-a-uuid?view=masked",
        "/legal/vcons/not-a-uuid/versions/0199e5c6-0d7a-7c11-8000-000000000001?view=masked",
    ] {
        let answer = get(&s, path, &legal).await;
        assert_eq!(answer.status(), 400, "{path}");
        assert_eq!(problem(answer).await, "bad-path-uuid", "{path}");
        assert_eq!(last(&s).0, "legal_read_masked", "{path}");
    }
}

#[tokio::test]
async fn a_path_version_not_in_text_form_is_refused_and_recorded_as_legal_read_masked() {
    let s = server().await;
    stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let answer = get(&s, &version_path("not-a-version"), &legal).await;
    assert_eq!(answer.status(), 400, "the status");
    assert_eq!(problem(answer).await, "bad-path-version", "the problem");
    assert_eq!(last(&s).0, "legal_read_masked", "the record");
}

#[tokio::test]
async fn a_refused_legal_masked_view_sends_no_vcon() {
    let s = server().await;
    let tag = stored(&s).await;
    let reader = s.key(&[Scope::Read]);
    let legal = s.key(&[Scope::Legal]);
    for (path, token) in [
        (current_path(), &reader),
        (format!("/legal/vcons/{UUID}?view=whole"), &legal),
        (format!("{}&reveal=all", version_path(&tag)), &legal),
    ] {
        let (status, media, _, _, body) = answered(&s, &path, token).await;
        assert_ne!(status, 200, "{path}");
        assert_ne!(media, "application/vcon", "{path}");
        let text = String::from_utf8(body).expect("text");
        assert!(!text.contains("Alice"), "{path} sent {text}");
    }
}

// Edge.

#[tokio::test]
async fn an_encoded_view_is_the_view_on_the_legal_route() {
    let s = server().await;
    stored(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let (status, media, _, _, _) =
        answered(&s, &format!("/legal/vcons/{UUID}?%76iew=mask%65d&"), &legal).await;
    assert_eq!(
        (status, media.as_str()),
        (200, "application/json"),
        "the answer"
    );
    assert_eq!(last(&s).0, "legal_read_masked", "the record");
}

#[tokio::test]
async fn one_legal_masked_read_writes_one_record() {
    let s = server().await;
    stored(&s).await;
    // Making the key writes a record of its own, before the count.
    let legal = s.key(&[Scope::Legal]);
    let before = recorded(&s).len();
    answered(&s, &current_path(), &legal).await;
    let after = recorded(&s);
    assert_eq!(after.len(), before + 1, "one record: {after:?}");
}

#[tokio::test]
async fn after_a_replace_the_legal_view_is_of_the_new_current_version() {
    let s = server().await;
    let old = stored(&s).await;
    let new = replaced(&s, &old, "A new subject").await;
    let legal = s.key(&[Scope::Legal]);
    let (_, _, tag, state, body) = answered(&s, &current_path(), &legal).await;
    assert_eq!((tag, state.as_str()), (new, "live"), "the new version");
    let got: Summary = serde_json::from_slice(&body).expect("a summary");
    assert_eq!(got.subject, "present, hidden", "the new version's subject");
}

#[tokio::test]
async fn an_old_version_of_a_deleted_vcon_is_summarized_and_says_deleted() {
    let s = server().await;
    let old = stored(&s).await;
    replaced(&s, &old, "A new subject").await;
    deleted(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let (status, _, tag, state, body) = answered(&s, &version_path(&old), &legal).await;
    assert_eq!(
        (status, tag, state.as_str()),
        (200, old, "deleted"),
        "the answer"
    );
    let got: Summary = serde_json::from_slice(&body).expect("a summary");
    assert_eq!(got.subject, "none", "the old version's subject");
}

#[tokio::test]
async fn after_a_restore_the_legal_view_is_live_again() {
    let s = server().await;
    stored(&s).await;
    deleted(&s).await;
    let legal = s.key(&[Scope::Legal]);
    let restored = call(
        &s,
        Method::POST,
        &format!("/legal/vcons/{UUID}/restore"),
        &legal,
    )
    .await;
    assert_eq!(restored.status(), 200, "restored");
    let (status, _, _, state, _) = answered(&s, &current_path(), &legal).await;
    assert_eq!((status, state.as_str()), (200, "live"), "the answer");
}

#[tokio::test]
async fn the_tenant_masked_view_of_a_deleted_vcon_stays_gone_while_the_legal_one_answers() {
    let s = server().await;
    stored(&s).await;
    deleted(&s).await;
    let reader = s.key(&[Scope::Read]);
    let tenant = get(&s, &format!("/vcons/{UUID}?view=masked"), &reader).await;
    assert_eq!(tenant.status(), 410, "the tenant read");
    let legal = s.key(&[Scope::Legal]);
    let (status, _, _, _, _) = answered(&s, &current_path(), &legal).await;
    assert_eq!(status, 200, "the legal read");
}
