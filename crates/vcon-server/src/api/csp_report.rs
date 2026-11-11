//! Where a browser sends its reports that a console page broke the
//! page's content-security policy: `POST /console/csp-report`.
//!
//! A browser sends a report with no credentials, so the route takes no
//! key. What it does with a report is bounded for that reason: the body
//! is read up to [`BODY_LIMIT`] bytes, checked to be JSON of the shape its
//! media type names, and dropped. Nothing of it is stored or logged. Each
//! request is counted in the admin log under the action `csp_report` and
//! the status that answered it, one record a minute for each status, so a
//! sender without a key can add a few rows a minute and no more.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Extension, State};
use axum::http::header::{CONTENT_TYPE, RETRY_AFTER};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use super::listen::Peer;
use super::{App, Limits, Problem, read_body};

/// The most bytes a report may hold. Chromium's reports of the console's
/// own pages are under 2 KiB.
pub(crate) const BODY_LIMIT: usize = 16 * 1024;

/// The two ways a browser reports a violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The `report-uri` directive's: `application/csp-report`, one JSON
    /// object.
    Violation,
    /// The Reporting API's, named by `report-to`:
    /// `application/reports+json`, a JSON array of reports.
    Reports,
}

/// The kind of report a `Content-Type` value names, if it names one.
/// Parameters after the type are not read, and a media type has no case.
pub(crate) fn kind(content_type: Option<&str>) -> Option<Kind> {
    let named = content_type?.split(';').next()?.trim();
    if named.eq_ignore_ascii_case("application/csp-report") {
        Some(Kind::Violation)
    } else if named.eq_ignore_ascii_case("application/reports+json") {
        Some(Kind::Reports)
    } else {
        None
    }
}

/// Whether `body` is JSON of the shape `kind` has: one object for a
/// violation, one array for reports. Nothing is kept of what it holds.
pub(crate) fn well_formed(kind: Kind, body: &[u8]) -> bool {
    let opens = match kind {
        Kind::Violation => b'{',
        Kind::Reports => b'[',
    };
    let first = body.iter().find(|byte| !byte.is_ascii_whitespace());
    first == Some(&opens) && serde_json::from_slice::<serde::de::IgnoredAny>(body).is_ok()
}

/// Takes one report. The answer is 204, or the problem that refused it:
/// 415 for a media type that is no report's, 413 for a body over
/// [`BODY_LIMIT`], 408 for one that did not arrive in the server's time
/// for a body, 400 for one that is not JSON of its kind's shape. Either
/// way the request is counted, under the answer's status, before the
/// answer is sent; a failure to count changes no answer.
///
/// Before any of that, a report past the listener's number for the minute
/// or for its source address (`api::report_gate`) is answered 429 with no
/// body, a `Retry-After` of what is left of the minute, and no write: the
/// minute's refusals are written as one record when the minute ends, or
/// by the first report of a later minute.
pub(crate) async fn receive(
    State(app): State<Arc<App>>,
    peer: Option<Extension<Peer>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let now = app.tenants.now();
    let admission = app
        .reports
        .admit(peer.map(|Extension(Peer(address))| address), now);
    if let Some((minute, refused)) = admission.earlier {
        write_refused(&app, minute, refused).await;
    }
    if !admission.taken {
        let minute = admission.minute;
        let left = minute + time::Duration::MINUTE - now;
        if admission.first_refusal {
            let flushing = Arc::clone(&app);
            let wait = Duration::try_from(left).unwrap_or(Duration::ZERO);
            let task = tokio::spawn(async move {
                tokio::time::sleep(wait).await;
                let refused = flushing.reports.take_refused(minute);
                write_refused(&flushing, minute, refused).await;
            });
            #[cfg(test)]
            app.reports.set_flusher(task);
            #[cfg(not(test))]
            drop(task);
        }
        // Whole seconds, never fewer than are left.
        let seconds = left.whole_seconds() + i64::from(left.subsec_nanoseconds() > 0);
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(RETRY_AFTER, seconds.to_string())],
        )
            .into_response();
    }
    let answer = match check(&app.limits, &headers, body).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(problem) => problem.into_response(),
    };
    let status = answer.status().as_u16();
    let tenants = Arc::clone(&app.tenants);
    // The registry's connection blocks; the count is awaited so that the
    // record is there when the sender has its answer.
    let _ = tokio::task::spawn_blocking(move || tenants.count_csp_report(status)).await;
    answer
}

/// Writes the reports refused in `minute`, if any, as one record.
async fn write_refused(app: &Arc<App>, minute: time::OffsetDateTime, refused: u64) {
    if refused == 0 {
        return;
    }
    let tenants = Arc::clone(&app.tenants);
    let _ = tokio::task::spawn_blocking(move || tenants.count_csp_refused(minute, refused)).await;
}

/// Reads a report and drops it, or says why it is refused.
async fn check(limits: &Limits, headers: &HeaderMap, body: Body) -> Result<(), Problem> {
    let content_type = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok());
    let kind = kind(content_type).ok_or(Problem::UnsupportedMediaType)?;
    let limits = Limits {
        body: BODY_LIMIT.min(limits.body),
        ..*limits
    };
    let body = read_body(body, headers, &limits).await?;
    if well_formed(kind, &body) {
        Ok(())
    } else {
        Err(Problem::BadBody)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::super::tests::pipeline::{LIMIT, Server, server, server_limiting, server_on};
    use super::{BODY_LIMIT, Kind, kind, well_formed};
    use crate::access::Plane;
    use crate::api::ReportLimits;
    use crate::console;

    const PATH: &str = "/console/csp-report";
    const VIOLATION: &str = "application/csp-report";
    const REPORTS: &str = "application/reports+json";
    /// What a browser sends for the `report-uri` directive.
    const A_VIOLATION: &str = r#"{"csp-report":{"document-uri":"https://localhost/console/","violated-directive":"script-src-elem","blocked-uri":"https://example.com/x.js","disposition":"enforce"}}"#;
    /// What a browser sends to the endpoint `report-to` names.
    const SOME_REPORTS: &str = r#"[{"type":"csp-violation","age":1,"url":"https://localhost/console/","body":{"blockedURL":"https://example.com/x.js","effectiveDirective":"script-src-elem"}}]"#;

    /// The admin log's records of reports.
    fn counted(s: &Server) -> Vec<Value> {
        s.tenants
            .admin_records()
            .into_iter()
            .filter(|r| r["action"] == "csp_report")
            .collect()
    }

    /// The count under each status, in status order.
    fn by_status(s: &Server) -> Vec<(u64, u64)> {
        let mut all: Vec<(u64, u64)> = counted(s)
            .iter()
            .map(|r| {
                (
                    r["status"].as_u64().expect("a status"),
                    r["count"].as_u64().expect("a count"),
                )
            })
            .collect();
        all.sort_unstable();
        all
    }

    async fn report(
        s: &Server,
        media_type: Option<&str>,
        body: impl Into<reqwest::Body>,
    ) -> reqwest::Response {
        let mut request = s.client().post(s.url(PATH)).body(body);
        if let Some(media_type) = media_type {
            request = request.header("content-type", media_type);
        }
        request.send().await.expect("an answer")
    }

    /// A JSON object of exactly `size` bytes.
    fn object_of(size: usize) -> String {
        let frame = r#"{"csp-report":{"x":""}}"#;
        let body = format!(
            r#"{{"csp-report":{{"x":"{}"}}}}"#,
            "a".repeat(size - frame.len())
        );
        assert_eq!(body.len(), size, "the body's size");
        body
    }

    // The media type.

    #[test]
    fn the_report_uri_media_type_is_a_violation() {
        assert_eq!(kind(Some(VIOLATION)), Some(Kind::Violation), "{VIOLATION}");
    }

    #[test]
    fn the_reporting_api_media_type_is_reports() {
        assert_eq!(kind(Some(REPORTS)), Some(Kind::Reports), "{REPORTS}");
    }

    #[test]
    fn a_media_type_with_a_parameter_is_read_by_its_type() {
        assert_eq!(
            kind(Some("application/csp-report; charset=utf-8")),
            Some(Kind::Violation),
            "a parameter after a space"
        );
        assert_eq!(
            kind(Some("application/reports+json;charset=utf-8")),
            Some(Kind::Reports),
            "a parameter with no space"
        );
    }

    #[test]
    fn a_media_type_is_read_whatever_its_case() {
        assert_eq!(
            kind(Some("Application/CSP-Report")),
            Some(Kind::Violation),
            "media types have no case"
        );
    }

    #[test]
    fn a_media_type_with_space_around_it_is_read() {
        assert_eq!(
            kind(Some(" application/reports+json ")),
            Some(Kind::Reports),
            "spaces"
        );
    }

    #[test]
    fn no_media_type_is_no_report() {
        assert_eq!(kind(None), None, "no header");
    }

    #[test]
    fn an_empty_media_type_is_no_report() {
        assert_eq!(kind(Some("")), None, "an empty header");
    }

    #[test]
    fn another_media_type_is_no_report() {
        for other in [
            "application/json",
            "text/plain",
            "application/vcon",
            "application/x-www-form-urlencoded",
            "multipart/form-data; boundary=x",
        ] {
            assert_eq!(kind(Some(other)), None, "{other}");
        }
    }

    #[test]
    fn a_media_type_that_only_starts_like_a_reports_is_no_report() {
        for near in [
            "application/csp-reports",
            "application/csp-report+json",
            "application/reports",
            "application/reports+jsonx",
            "xapplication/csp-report",
            "application/csp-report, application/json",
        ] {
            assert_eq!(kind(Some(near)), None, "{near}");
        }
    }

    // The body's shape.

    #[test]
    fn a_violation_is_one_object() {
        assert!(
            well_formed(Kind::Violation, A_VIOLATION.as_bytes()),
            "a browser's violation"
        );
    }

    #[test]
    fn reports_are_an_array() {
        assert!(
            well_formed(Kind::Reports, SOME_REPORTS.as_bytes()),
            "a browser's reports"
        );
    }

    #[test]
    fn an_empty_object_and_an_empty_array_have_the_shape() {
        assert!(well_formed(Kind::Violation, b"{}"), "an empty object");
        assert!(well_formed(Kind::Reports, b"[]"), "an empty array");
    }

    #[test]
    fn space_around_the_json_is_allowed() {
        assert!(
            well_formed(Kind::Violation, b" \n{} \n"),
            "JSON may have space around it"
        );
    }

    #[test]
    fn a_violation_that_is_an_array_has_the_wrong_shape() {
        assert!(
            !well_formed(Kind::Violation, SOME_REPORTS.as_bytes()),
            "an array as a violation"
        );
    }

    #[test]
    fn reports_that_are_an_object_have_the_wrong_shape() {
        assert!(
            !well_formed(Kind::Reports, A_VIOLATION.as_bytes()),
            "an object as reports"
        );
    }

    #[test]
    fn a_body_that_is_not_json_has_no_shape() {
        for kind in [Kind::Violation, Kind::Reports] {
            assert!(!well_formed(kind, b"csp-report=1"), "{kind:?}: a form");
            assert!(!well_formed(kind, b"{"), "{kind:?}: half an object");
            assert!(!well_formed(kind, &[0xff, 0xfe]), "{kind:?}: not UTF-8");
        }
    }

    #[test]
    fn an_empty_body_has_no_shape() {
        for kind in [Kind::Violation, Kind::Reports] {
            assert!(!well_formed(kind, b""), "{kind:?}");
        }
    }

    #[test]
    fn a_json_value_that_is_neither_object_nor_array_has_no_shape() {
        for kind in [Kind::Violation, Kind::Reports] {
            for value in ["1", "\"x\"", "null", "true"] {
                assert!(!well_formed(kind, value.as_bytes()), "{kind:?}: {value}");
            }
        }
    }

    #[test]
    fn bytes_after_the_json_value_break_the_shape() {
        assert!(!well_formed(Kind::Violation, b"{} {}"), "two objects");
        assert!(!well_formed(Kind::Reports, b"[] x"), "an array and a word");
    }

    #[test]
    fn json_nested_five_thousand_deep_is_read_without_a_panic() {
        let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
        // The body is skipped, not built, so depth costs no stack.
        assert!(
            well_formed(Kind::Reports, deep.as_bytes()),
            "5000 arrays deep"
        );
        assert!(
            !well_formed(Kind::Reports, &deep.as_bytes()[..9999]),
            "and one bracket short"
        );
    }

    #[test]
    fn the_limit_is_16_kib_and_under_the_servers_own() {
        assert_eq!(BODY_LIMIT, 16 * 1024, "the limit");
        assert_eq!(
            BODY_LIMIT.min(LIMIT - 1),
            BODY_LIMIT,
            "under the test server's body limit, so this limit is the one met"
        );
    }

    // The route.

    #[tokio::test]
    async fn a_violation_is_answered_204_with_no_body() {
        let s = server().await;
        let answer = report(&s, Some(VIOLATION), A_VIOLATION).await;
        assert_eq!(answer.status(), 204, "{answer:?}");
        assert_eq!(answer.bytes().await.expect("a body").len(), 0, "no body");
    }

    #[tokio::test]
    async fn reports_are_answered_204() {
        let s = server().await;
        let answer = report(&s, Some(REPORTS), SOME_REPORTS).await;
        assert_eq!(answer.status(), 204, "{answer:?}");
    }

    #[tokio::test]
    async fn a_report_is_counted_once_with_no_actor_and_no_target() {
        let s = server().await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        let records = counted(&s);
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0]["status"], 204, "the status that answered it");
        assert_eq!(records[0]["count"], 1, "one report");
        assert_eq!(records[0]["actor"], "", "no key came with it");
        assert_eq!(records[0]["target"], Value::Null, "no target");
        assert_eq!(records[0]["version"], Value::Null, "no version");
    }

    #[tokio::test]
    async fn reports_in_one_minute_share_a_record() {
        let s = server().await;
        for _ in 0..3 {
            report(&s, Some(VIOLATION), A_VIOLATION).await;
        }
        report(&s, Some(REPORTS), SOME_REPORTS).await;
        assert_eq!(
            by_status(&s),
            [(204, 4)],
            "one record for four reports of either kind"
        );
    }

    #[tokio::test]
    async fn a_report_is_counted_before_it_is_answered() {
        // No wait between the answer and the read of the log.
        let s = server().await;
        for sent in 1..=5 {
            report(&s, Some(VIOLATION), A_VIOLATION).await;
            assert_eq!(by_status(&s), [(204, sent)], "after report {sent}");
        }
    }

    #[tokio::test]
    async fn a_token_sent_with_a_report_is_not_read() {
        let s = server().await;
        let answer = s
            .client()
            .post(s.url(PATH))
            .header("content-type", VIOLATION)
            .bearer_auth("not-a-key")
            .body(A_VIOLATION)
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 204, "{answer:?}");
        let failed: Vec<Value> = s
            .tenants
            .admin_records()
            .into_iter()
            .filter(|r| r["action"] == "auth_failed")
            .collect();
        assert_eq!(
            failed,
            Vec::<Value>::new(),
            "no failed authentication is counted"
        );
    }

    #[tokio::test]
    async fn the_answer_to_a_report_carries_the_consoles_headers() {
        let s = server().await;
        let answer = report(&s, Some(VIOLATION), A_VIOLATION).await;
        for (name, value) in console::HEADERS {
            assert_eq!(answer.headers()[name], value, "{name}");
        }
    }

    #[tokio::test]
    async fn every_listener_takes_reports() {
        for plane in [Plane::Both, Plane::Tenant, Plane::Operator] {
            let s = server_on(plane).await;
            let answer = report(&s, Some(VIOLATION), A_VIOLATION).await;
            assert_eq!(answer.status(), 204, "{plane:?}");
            assert_eq!(by_status(&s), [(204, 1)], "{plane:?}");
        }
    }

    #[tokio::test]
    async fn a_report_of_exactly_the_limit_is_taken() {
        let s = server().await;
        let answer = report(&s, Some(VIOLATION), object_of(BODY_LIMIT)).await;
        assert_eq!(answer.status(), 204, "{answer:?}");
    }

    #[tokio::test]
    async fn nothing_a_report_says_reaches_the_admin_log() {
        let s = server().await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        report(&s, Some(REPORTS), SOME_REPORTS).await;
        let log = serde_json::to_string(&s.tenants.admin_records()).expect("JSON");
        for said in [
            "example.com",
            "script-src-elem",
            "document-uri",
            "blockedURL",
            "localhost",
        ] {
            assert!(!log.contains(said), "{said} in {log}");
        }
    }

    #[tokio::test]
    async fn a_report_is_not_counted_in_a_tenants_log() {
        let s = server().await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        assert_eq!(
            s.tenant_log("csp_report"),
            Vec::<Value>::new(),
            "the tenant's log"
        );
    }

    #[tokio::test]
    async fn a_query_on_the_path_changes_nothing() {
        let s = server().await;
        let answer = s
            .client()
            .post(s.url("/console/csp-report?x=1"))
            .header("content-type", VIOLATION)
            .body(A_VIOLATION)
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 204, "{answer:?}");
        assert_eq!(by_status(&s), [(204, 1)], "counted");
    }

    #[tokio::test]
    async fn a_report_with_no_media_type_is_refused_415_and_counted_so() {
        let s = server().await;
        let answer = report(&s, None, A_VIOLATION).await;
        assert_eq!(answer.status(), 415, "{answer:?}");
        assert_eq!(by_status(&s), [(415, 1)], "counted under its status");
    }

    #[tokio::test]
    async fn a_report_of_another_media_type_is_refused_415() {
        let s = server().await;
        for other in ["application/json", "text/plain", "application/vcon"] {
            let answer = report(&s, Some(other), A_VIOLATION).await;
            assert_eq!(answer.status(), 415, "{other}");
        }
        assert_eq!(by_status(&s), [(415, 3)], "each counted");
    }

    #[tokio::test]
    async fn the_media_type_is_checked_before_the_size() {
        let s = server().await;
        let answer = report(&s, Some("text/plain"), "x".repeat(BODY_LIMIT + 1)).await;
        assert_eq!(answer.status(), 415, "{answer:?}");
    }

    #[tokio::test]
    async fn a_report_that_is_not_json_is_refused_400_and_counted_so() {
        let s = server().await;
        let answer = report(&s, Some(VIOLATION), "csp-report=1").await;
        assert_eq!(answer.status(), 400, "{answer:?}");
        assert_eq!(by_status(&s), [(400, 1)], "counted under its status");
    }

    #[tokio::test]
    async fn a_report_of_the_other_kinds_shape_is_refused_400() {
        let s = server().await;
        let answer = report(&s, Some(VIOLATION), SOME_REPORTS).await;
        assert_eq!(answer.status(), 400, "an array as a violation");
        let answer = report(&s, Some(REPORTS), A_VIOLATION).await;
        assert_eq!(answer.status(), 400, "an object as reports");
    }

    #[tokio::test]
    async fn an_empty_report_is_refused_400() {
        let s = server().await;
        let answer = report(&s, Some(VIOLATION), "").await;
        assert_eq!(answer.status(), 400, "{answer:?}");
    }

    #[tokio::test]
    async fn a_report_one_byte_over_the_limit_is_refused_413_and_counted_so() {
        let s = server().await;
        let answer = report(&s, Some(VIOLATION), object_of(BODY_LIMIT + 1)).await;
        assert_eq!(answer.status(), 413, "{answer:?}");
        assert_eq!(by_status(&s), [(413, 1)], "counted under its status");
    }

    #[tokio::test]
    async fn a_report_far_over_the_limit_is_refused_413() {
        let s = server().await;
        let answer = report(&s, Some(REPORTS), object_of(LIMIT)).await;
        assert_eq!(answer.status(), 413, "{answer:?}");
    }

    #[tokio::test]
    async fn a_report_with_no_declared_length_is_refused_once_it_passes_the_limit() {
        let s = server().await;
        let mut body = String::new();
        for _ in 0..5 {
            body.push_str(&format!("{:x}\r\n{}\r\n", 4096, "x".repeat(4096)));
        }
        let head = format!(
            "POST {PATH} HTTP/1.1\r\nhost: localhost\r\ncontent-type: {VIOLATION}\r\n\
             transfer-encoding: chunked\r\n\r\n"
        );
        // The body is never finished: the refusal comes while it is open.
        let status = s.raw_request(&head, body.as_bytes()).await;
        assert_eq!(status, "HTTP/1.1 413 Payload Too Large", "the status line");
        assert_eq!(by_status(&s), [(413, 1)], "counted under its status");
    }

    #[tokio::test]
    async fn a_report_whose_body_does_not_arrive_is_refused_408_and_counted_so() {
        let s = server().await;
        let head = format!(
            "POST {PATH} HTTP/1.1\r\nhost: localhost\r\ncontent-type: {VIOLATION}\r\n\
             content-length: 100\r\n\r\n"
        );
        // The server's own limit on a body's arrival ends the wait.
        let status = s.raw_request(&head, b"{").await;
        assert_eq!(status, "HTTP/1.1 408 Request Timeout", "the status line");
        assert_eq!(by_status(&s), [(408, 1)], "counted under its status");
    }

    #[tokio::test]
    async fn a_refusal_is_a_problem_and_carries_the_consoles_headers() {
        let s = server().await;
        let answer = report(&s, Some("text/plain"), A_VIOLATION).await;
        assert_eq!(
            answer.headers()["content-type"],
            "application/problem+json",
            "the media type"
        );
        for (name, value) in console::HEADERS {
            assert_eq!(answer.headers()[name], value, "{name}");
        }
        let problem: Value = answer.json().await.expect("JSON");
        assert_eq!(problem["status"], 415, "{problem}");
    }

    #[tokio::test]
    async fn another_method_on_the_path_is_refused_405_and_not_counted() {
        let s = server().await;
        for request in [
            s.client().get(s.url(PATH)),
            s.client().put(s.url(PATH)),
            s.client().delete(s.url(PATH)),
            s.client().patch(s.url(PATH)),
            s.client().head(s.url(PATH)),
        ] {
            let answer = request.send().await.expect("an answer");
            assert_eq!(answer.status(), 405, "{answer:?}");
        }
        assert_eq!(counted(&s), Vec::<Value>::new(), "only a report is counted");
    }

    #[tokio::test]
    async fn a_path_that_is_not_exactly_the_reports_is_not_found_and_not_counted() {
        let s = server().await;
        for near in [
            "/console/csp-report/",
            "/console/csp-report/x",
            "/console/CSP-REPORT",
            "/console/csp-reports",
            "/csp-report",
        ] {
            let answer = s
                .client()
                .post(s.url(near))
                .header("content-type", VIOLATION)
                .body(A_VIOLATION)
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 404, "{near}");
        }
        assert_eq!(counted(&s), Vec::<Value>::new(), "nothing counted");
    }

    #[tokio::test]
    async fn each_status_has_a_record_of_its_own() {
        let s = server().await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        report(&s, None, A_VIOLATION).await;
        report(&s, Some(VIOLATION), "x").await;
        report(&s, Some(VIOLATION), object_of(BODY_LIMIT + 1)).await;
        assert_eq!(
            by_status(&s),
            [(204, 2), (400, 1), (413, 1), (415, 1)],
            "one record for each status"
        );
    }

    #[tokio::test]
    async fn reports_in_another_minute_have_another_record() {
        let s = server().await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:01:30 UTC));
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        let mut minutes: Vec<(String, u64)> = counted(&s)
            .iter()
            .map(|r| {
                (
                    r["at"].as_str().expect("a time").to_owned(),
                    r["count"].as_u64().expect("a count"),
                )
            })
            .collect();
        minutes.sort();
        assert_eq!(
            minutes,
            [
                ("2026-10-23T12:00:00Z".to_owned(), 1),
                ("2026-10-23T12:01:00Z".to_owned(), 2)
            ],
            "one record for each minute"
        );
    }

    #[tokio::test]
    async fn many_reports_add_no_rows_within_a_minute() {
        let s = server().await;
        for _ in 0..20 {
            report(&s, Some(VIOLATION), A_VIOLATION).await;
        }
        assert_eq!(by_status(&s), [(204, 20)], "one row for twenty reports");
    }

    #[tokio::test]
    async fn a_report_with_a_parameter_on_its_media_type_is_taken() {
        let s = server().await;
        let answer = report(
            &s,
            Some("application/csp-report; charset=utf-8"),
            A_VIOLATION,
        )
        .await;
        assert_eq!(answer.status(), 204, "{answer:?}");
    }

    // The number of reports taken in a minute.

    fn limits(per_minute: u32, per_address: u32) -> ReportLimits {
        ReportLimits {
            per_minute,
            per_address,
        }
    }

    /// The statuses that answer `reports` valid reports, in order.
    async fn statuses(s: &Server, reports: usize) -> Vec<u16> {
        let mut all = Vec::new();
        for _ in 0..reports {
            all.push(
                report(s, Some(VIOLATION), A_VIOLATION)
                    .await
                    .status()
                    .as_u16(),
            );
        }
        all
    }

    /// The records of reports as (minute, status, count), in order.
    fn records(s: &Server) -> Vec<(String, u64, u64)> {
        let mut all: Vec<(String, u64, u64)> = counted(s)
            .iter()
            .map(|r| {
                (
                    r["at"].as_str().expect("a time").to_owned(),
                    r["status"].as_u64().expect("a status"),
                    r["count"].as_u64().expect("a count"),
                )
            })
            .collect();
        all.sort();
        all
    }

    #[tokio::test]
    async fn reports_past_the_minutes_number_are_refused_429() {
        let s = server_limiting(limits(3, 3)).await;
        assert_eq!(
            statuses(&s, 6).await,
            [204, 204, 204, 429, 429, 429],
            "three taken"
        );
    }

    #[tokio::test]
    async fn reports_from_one_address_past_its_number_are_refused_429() {
        // Every request of a test comes from one address, so a refusal
        // under the minute's number is the address's.
        let s = server_limiting(limits(10, 2)).await;
        assert_eq!(
            statuses(&s, 4).await,
            [204, 204, 429, 429],
            "two taken of ten allowed"
        );
    }

    #[tokio::test]
    async fn a_server_with_the_default_limits_takes_20_reports_from_one_address() {
        let s = server().await;
        let all = statuses(&s, 22).await;
        assert_eq!(all[..20], [204; 20], "the first twenty");
        assert_eq!(all[20..], [429, 429], "and no more");
    }

    #[tokio::test]
    async fn a_refusal_for_the_number_has_no_body_and_says_when_to_come_back() {
        let s = server_limiting(limits(1, 1)).await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        let answer = report(&s, Some(VIOLATION), A_VIOLATION).await;
        assert_eq!(answer.status(), 429, "{answer:?}");
        // The test clock stands at the first second of a minute.
        assert_eq!(
            answer.headers()["retry-after"],
            "60",
            "seconds to the next minute"
        );
        for (name, value) in console::HEADERS {
            assert_eq!(answer.headers()[name], value, "{name}");
        }
        assert_eq!(answer.bytes().await.expect("a body").len(), 0, "no body");
    }

    #[tokio::test]
    async fn the_time_to_come_back_is_what_is_left_of_the_minute() {
        let s = server_limiting(limits(1, 1)).await;
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:00:42.5 UTC));
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        let answer = report(&s, Some(VIOLATION), A_VIOLATION).await;
        assert_eq!(
            answer.headers()["retry-after"],
            "18",
            "17.5 seconds, as whole seconds"
        );
    }

    #[tokio::test]
    async fn a_refused_report_makes_no_write_of_its_own() {
        let s = server_limiting(limits(2, 2)).await;
        statuses(&s, 9).await;
        assert_eq!(
            by_status(&s),
            [(204, 2)],
            "seven refused, and no record of them yet"
        );
    }

    #[tokio::test]
    async fn the_number_is_checked_before_the_media_type_and_the_size() {
        let s = server_limiting(limits(1, 1)).await;
        report(&s, Some(VIOLATION), A_VIOLATION).await;
        let wrong_type = report(&s, Some("text/plain"), A_VIOLATION).await;
        assert_eq!(wrong_type.status(), 429, "not 415");
        let too_large = report(&s, Some(VIOLATION), object_of(BODY_LIMIT + 1)).await;
        assert_eq!(too_large.status(), 429, "not 413");
    }

    #[tokio::test]
    async fn a_request_the_route_refuses_uses_the_number_too() {
        // Each answer but 429 costs a write, whatever its status.
        let s = server_limiting(limits(2, 2)).await;
        assert_eq!(
            report(&s, Some("text/plain"), "x").await.status(),
            415,
            "the first"
        );
        assert_eq!(
            report(&s, Some(VIOLATION), "x").await.status(),
            400,
            "the second"
        );
        assert_eq!(
            report(&s, Some(VIOLATION), A_VIOLATION).await.status(),
            429,
            "the third"
        );
    }

    #[tokio::test]
    async fn the_refusals_of_a_minute_are_written_as_one_record_when_it_ends() {
        let s = server_limiting(limits(2, 2)).await;
        // Half a millisecond before the minute ends.
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:00:59.9995 UTC));
        assert_eq!(statuses(&s, 7).await[2..], [429; 5], "five refused");
        s.app.reports.settled().await;
        assert_eq!(
            records(&s),
            [
                ("2026-10-23T12:00:00Z".to_owned(), 204, 2),
                ("2026-10-23T12:00:00Z".to_owned(), 429, 5)
            ],
            "one record says five were refused"
        );
    }

    #[tokio::test]
    async fn the_first_report_of_a_later_minute_writes_the_refusals_before_it() {
        let s = server_limiting(limits(1, 1)).await;
        statuses(&s, 4).await;
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:07:10 UTC));
        assert_eq!(statuses(&s, 1).await, [204], "a new minute");
        assert_eq!(
            records(&s),
            [
                ("2026-10-23T12:00:00Z".to_owned(), 204, 1),
                ("2026-10-23T12:00:00Z".to_owned(), 429, 3),
                ("2026-10-23T12:07:00Z".to_owned(), 204, 1)
            ],
            "the three refused at noon are one record of noon"
        );
    }

    #[tokio::test]
    async fn refusals_written_at_the_minutes_end_are_not_written_again() {
        let s = server_limiting(limits(1, 1)).await;
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:00:59.9995 UTC));
        statuses(&s, 3).await;
        s.app.reports.settled().await;
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:01:05 UTC));
        statuses(&s, 1).await;
        let refused: Vec<(String, u64, u64)> = records(&s)
            .into_iter()
            .filter(|(_, status, _)| *status == 429)
            .collect();
        assert_eq!(
            refused,
            [("2026-10-23T12:00:00Z".to_owned(), 429, 2)],
            "once"
        );
    }

    #[tokio::test]
    async fn a_new_minute_takes_reports_again() {
        let s = server_limiting(limits(1, 1)).await;
        assert_eq!(statuses(&s, 2).await, [204, 429], "the first minute");
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:01:00 UTC));
        assert_eq!(statuses(&s, 2).await, [204, 429], "the next");
    }

    #[tokio::test]
    async fn a_flood_adds_one_row_for_what_it_was_refused() {
        let s = server_limiting(limits(1, 1)).await;
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:00:59.9995 UTC));
        statuses(&s, 60).await;
        s.app.reports.settled().await;
        assert_eq!(
            by_status(&s),
            [(204, 1), (429, 59)],
            "two rows for sixty requests"
        );
    }

    #[tokio::test]
    async fn a_minute_with_no_refusal_has_no_record_of_refusals() {
        let s = server_limiting(limits(5, 5)).await;
        statuses(&s, 5).await;
        s.clock
            .set(time::macros::datetime!(2026-10-23 12:01:00 UTC));
        statuses(&s, 1).await;
        s.app.reports.settled().await;
        assert_eq!(by_status(&s), [(204, 1), (204, 5)], "only reports taken");
    }
}
