//! The server's HTTP API as the terminal UI uses it: `GET /key` and
//! `GET /vcons`, with one bearer key over TLS. The requests are behind
//! [`Api`], so the UI is driven in tests without a network; the conversions
//! a request makes (its path, and its answer to a value) are functions of
//! their own.

use std::error::Error;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crate::mcp::stdio::{self, ClientConfig};
use crate::tui::state::{ApiError, Effect, Event, KeyInfo, Page};

/// The vCons asked for in one page.
const PAGE: u16 = 100;
/// How long one request may take, connection included.
const TIMEOUT: Duration = Duration::from_secs(15);
/// The most bytes read of one answer; a page of [`PAGE`] rows is far
/// smaller.
const MAX_BODY: usize = 1 << 20;

/// The requests the UI makes.
pub trait Api: Send + Sync {
    /// `GET /key`: the key in use.
    ///
    /// # Errors
    ///
    /// [`ApiError`] when no answer arrived, or it was a refusal.
    fn key(&self) -> Result<KeyInfo, ApiError>;

    /// `GET /vcons`: one page, from `cursor` or from the start.
    ///
    /// # Errors
    ///
    /// [`ApiError`] when no answer arrived, or it was a refusal.
    fn page(&self, cursor: Option<&str>) -> Result<Page, ApiError>;
}

/// Makes the request `effect` names and returns its answer as an event.
pub fn perform(api: &dyn Api, effect: &Effect) -> Event {
    match effect {
        Effect::FetchKey => Event::KeyInfo(api.key()),
        Effect::FetchPage(cursor) => Event::Page(api.page(cursor.as_deref())),
    }
}

/// The server's address without trailing slashes, so a path can follow it.
#[must_use]
pub fn base(url: &str) -> String {
    url.trim_end_matches('/').to_owned()
}

/// The path and query of one page of the list.
#[must_use]
pub fn page_path(cursor: Option<&str>) -> String {
    let mut path = format!("/vcons?limit={PAGE}");
    if let Some(cursor) = cursor {
        path.push_str("&cursor=");
        path.extend(form_urlencoded::byte_serialize(cursor.as_bytes()));
    }
    path
}

/// The value a `status` and `body` answer holds.
///
/// # Errors
///
/// [`ApiError::Status`] for any status but 200, with the title of the
/// problem document if the body is one; [`ApiError::Malformed`] for a 200
/// whose body is not a `T`.
pub fn answer<T: DeserializeOwned>(status: u16, body: &[u8]) -> Result<T, ApiError> {
    #[derive(Deserialize)]
    struct Problem {
        title: String,
    }
    if status == 200 {
        return serde_json::from_slice(body).map_err(|e| ApiError::Malformed(e.to_string()));
    }
    let title = serde_json::from_slice::<Problem>(body)
        .ok()
        .map(|problem| problem.title);
    Err(ApiError::Status(status, title))
}

/// A running server, reached over HTTPS with one key.
pub struct HttpApi {
    base: String,
    token: Zeroizing<String>,
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

/// The address only: the token is never written.
impl std::fmt::Debug for HttpApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpApi")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl HttpApi {
    /// The server a client file names. The key file is held to the one
    /// secret-file rule, and the server's certificate is always verified:
    /// against the file's `ca`, or the system's roots without it.
    ///
    /// # Errors
    ///
    /// A sentence naming the key file, the CA file or the runtime that
    /// could not be used. It never holds the token.
    pub fn new(config: &ClientConfig) -> Result<Self, String> {
        let token = stdio::token(&config.key_file).map_err(|e| e.to_string())?;
        let client = stdio::https_client(config.ca.as_deref()).map_err(|e| e.to_string())?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("the runtime: {e}"))?;
        Ok(Self {
            base: base(&config.url),
            token: Zeroizing::new(token),
            client,
            runtime,
        })
    }

    /// One GET of `path`, read as a `T`.
    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = format!("{}{path}", self.base);
        let exchange = async {
            let mut response = self
                .client
                .get(&url)
                .bearer_auth(self.token.as_str())
                .send()
                .await
                .map_err(|e| unreachable(&e))?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| unreachable(&e))? {
                if body.len().saturating_add(chunk.len()) > MAX_BODY {
                    return Err(ApiError::Malformed(format!(
                        "it is longer than {MAX_BODY} bytes"
                    )));
                }
                body.extend_from_slice(&chunk);
            }
            answer(status, &body)
        };
        self.runtime.block_on(async {
            tokio::time::timeout(TIMEOUT, exchange)
                .await
                .unwrap_or_else(|_| {
                    Err(ApiError::Unreachable(format!(
                        "no answer within {} seconds",
                        TIMEOUT.as_secs()
                    )))
                })
        })
    }
}

impl Api for HttpApi {
    fn key(&self) -> Result<KeyInfo, ApiError> {
        self.get("/key")
    }

    fn page(&self, cursor: Option<&str>) -> Result<Page, ApiError> {
        self.get(&page_path(cursor))
    }
}

/// A request that got no answer, with the causes reqwest keeps below its
/// own sentence (the refused connection, the certificate that failed).
fn unreachable(error: &reqwest::Error) -> ApiError {
    let mut why = error.to_string();
    let mut cause = error.source();
    while let Some(inner) = cause {
        why.push_str(": ");
        why.push_str(&inner.to_string());
        cause = inner.source();
    }
    ApiError::Unreachable(why)
}

#[cfg(test)]
mod tests {
    use super::{Api, answer, base, page_path, perform};
    use crate::tui::state::{ApiError, Effect, Event, KeyInfo, Page};

    /// An API that answers from memory and records what it was asked.
    struct Canned {
        asked: std::sync::Mutex<Vec<String>>,
    }

    impl Api for Canned {
        fn key(&self) -> Result<KeyInfo, ApiError> {
            self.asked.lock().expect("the lock").push("key".to_owned());
            Err(ApiError::Status(401, None))
        }

        fn page(&self, cursor: Option<&str>) -> Result<Page, ApiError> {
            self.asked
                .lock()
                .expect("the lock")
                .push(format!("page {cursor:?}"));
            Ok(Page {
                vcons: Vec::new(),
                next: None,
            })
        }
    }

    // Success.

    #[test]
    fn a_key_answer_is_read() {
        let info: KeyInfo = answer(
            200,
            br#"{"id":"0123456789abcdef","tenant":"t_example","scopes":["read","audit"]}"#,
        )
        .expect("a key answer");
        assert_eq!(info.tenant.as_deref(), Some("t_example"), "the tenant");
        assert_eq!(info.scopes, ["read", "audit"], "the scopes");
    }

    #[test]
    fn a_page_answer_is_read_with_its_cursor() {
        let page: Page = answer(
            200,
            br#"{"next":"GOHD","vcons":[{"stored_at":"2026-10-25T12:00:00Z","updated_at":"2026-10-25T12:30:00Z","uuid":"01a07da8-c2bb-83e5-b9a2-279e0d16bc46","version":"4f0c4b0a1d2e3f405162738495a6b7c8"}]}"#,
        )
        .expect("a page");
        assert_eq!(page.next.as_deref(), Some("GOHD"), "the cursor");
        assert_eq!(
            page.vcons[0].uuid, "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
            "the row"
        );
    }

    #[test]
    fn the_first_page_is_asked_for_without_a_cursor() {
        assert_eq!(page_path(None), "/vcons?limit=100", "the first page");
    }

    #[test]
    fn each_effect_is_one_request_and_its_answer_one_event() {
        let api = Canned {
            asked: std::sync::Mutex::default(),
        };
        let page = perform(&api, &Effect::FetchPage(Some("c".to_owned())));
        assert!(
            matches!(page, Event::Page(Ok(_))),
            "a page's answer is a page event"
        );
        let key = perform(&api, &Effect::FetchKey);
        assert_eq!(
            key,
            Event::KeyInfo(Err(ApiError::Status(401, None))),
            "a refusal is carried"
        );
        assert_eq!(
            *api.asked.lock().expect("the lock"),
            ["page Some(\"c\")", "key"],
            "one request each, with the cursor"
        );
    }

    // Failure.

    #[test]
    fn a_refusal_carries_its_status_and_the_problems_title() {
        let refused: Result<Page, _> = answer(
            403,
            br#"{"type":"https://vcon-server.com/problems/forbidden/","title":"forbidden","status":403}"#,
        );
        assert_eq!(
            refused,
            Err(ApiError::Status(403, Some("forbidden".to_owned()))),
            "the title of the problem document"
        );
    }

    #[test]
    fn a_refusal_without_a_problem_document_carries_only_its_status() {
        let refused: Result<Page, _> = answer(502, b"<html>bad gateway</html>");
        assert_eq!(
            refused,
            Err(ApiError::Status(502, None)),
            "a proxy's page is not shown"
        );
    }

    #[test]
    fn a_200_that_is_not_the_documented_json_is_malformed() {
        let odd: Result<Page, _> = answer(200, b"{\"vcons\": 7}");
        assert!(
            matches!(odd, Err(ApiError::Malformed(_))),
            "not a page: {odd:?}"
        );
    }

    // Edge.

    #[test]
    fn a_cursor_is_percent_encoded() {
        assert_eq!(
            page_path(Some("a+b/c=&d")),
            "/vcons?limit=100&cursor=a%2Bb%2Fc%3D%26d",
            "a cursor cannot add a parameter"
        );
    }

    #[test]
    fn the_address_loses_its_trailing_slashes() {
        assert_eq!(
            base("https://vcon.example.com:8443/"),
            "https://vcon.example.com:8443",
            "one"
        );
        assert_eq!(
            base("https://vcon.example.com"),
            "https://vcon.example.com",
            "none"
        );
    }

    #[test]
    fn an_operator_keys_null_tenant_is_none() {
        let info: KeyInfo = answer(
            200,
            br#"{"id":"0123456789abcdef","tenant":null,"scopes":["admin"]}"#,
        )
        .expect("a key answer");
        assert_eq!(info.tenant, None, "null is no tenant");
    }

    #[test]
    fn a_last_page_has_no_cursor() {
        let page: Page = answer(200, br#"{"next":null,"vcons":[]}"#).expect("a page");
        assert_eq!(
            page,
            Page {
                vcons: Vec::new(),
                next: None
            },
            "empty and last"
        );
    }
}
