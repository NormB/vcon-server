//! The server's HTTP API as the terminal UI uses it: `GET /key`,
//! `GET /vcons`, a vCon's masked view (`?view=masked`) and its reveals
//! (`?reveal=<part>`), with one bearer key over TLS. The requests are behind
//! [`Api`], so the UI is driven in tests without a network; the conversions
//! a request makes (its path, and its answer to a value) are functions of
//! their own.

use std::error::Error;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use zeroize::Zeroizing;

use crate::masked::{self, Summary};
use crate::mcp::stdio::{self, ClientConfig};
use crate::reveal::{self, Part};
use crate::tui::state::{ApiError, Effect, Event, KeyInfo, Page};

/// The vCons asked for in one page.
pub(crate) const PAGE: u16 = 100;
/// How long one request may take, connection included.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(15);
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

    /// `GET /vcons/{uuid}?view=masked`: the vCon's masked view, as the
    /// lines the detail view shows ([`summary_lines`]).
    ///
    /// # Errors
    ///
    /// [`ApiError`] when no answer arrived, it was a refusal, or it was not
    /// the masked view.
    fn masked(&self, uuid: &str) -> Result<Vec<String>, ApiError>;

    /// `GET /vcons/{uuid}?reveal=<part>`: the whole vCon's text, inflated,
    /// read as a reveal of `part`, which the audit log records.
    ///
    /// # Errors
    ///
    /// [`ApiError`] when no answer arrived, it was a refusal, or it could
    /// not be read.
    fn reveal(&self, uuid: &str, part: Part) -> Result<String, ApiError>;

    /// Ends every request under way, and refuses every later one, at once.
    /// The UI calls it at its exit, before it joins its requests' threads.
    fn close(&self) {}
}

/// Makes the request `effect` names and returns its answer as an event. A
/// reveal's answer is reduced at once to the part it reveals.
pub fn perform(api: &dyn Api, effect: &Effect) -> Event {
    match effect {
        Effect::FetchKey => Event::KeyInfo(api.key()),
        Effect::FetchPage(cursor) => Event::Page(api.page(cursor.as_deref())),
        Effect::FetchMasked(uuid) => Event::Masked(uuid.clone(), api.masked(uuid)),
        Effect::FetchReveal(uuid, part) => Event::Revealed(
            uuid.clone(),
            *part,
            api.reveal(uuid, *part)
                .map(|text| reveal_text(&text, *part)),
        ),
    }
}

/// The most bytes read of a reveal's answer, and of the vCon it inflates
/// to: 16 MiB.
pub const REVEAL_BODY: usize = 16 << 20;

/// A path segment with every byte but the unreserved ones percent-encoded.
fn segment(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// The path of a vCon's masked view.
#[must_use]
pub fn masked_path(uuid: &str) -> String {
    format!(
        "/vcons/{}?{}={}",
        segment(uuid),
        masked::PARAMETER,
        masked::MASKED
    )
}

/// The path of a reveal of `part` of a vCon.
#[must_use]
pub fn reveal_path(uuid: &str, part: Part) -> String {
    format!(
        "/vcons/{}?{}={}",
        segment(uuid),
        reveal::MARKER,
        part.name()
    )
}

/// The masked view as the lines the detail view shows.
#[must_use]
pub fn summary_lines(made: &Summary) -> Vec<String> {
    let mut lines = vec![
        "Masked view".to_owned(),
        format!("Form          {}", made.form),
        format!(
            "Created       {}",
            if made.created.is_empty() {
                "not given"
            } else {
                &made.created
            }
        ),
        format!("Subject       {}", made.subject),
    ];
    if let Some(signature) = &made.signature {
        let or_none = |text: &str| {
            if text.is_empty() {
                "none".to_owned()
            } else {
                text.to_owned()
            }
        };
        lines.push(format!(
            "Signature     alg {}, kid {}, x5t {}",
            or_none(&signature.alg),
            or_none(&signature.kid),
            or_none(&signature.x5t)
        ));
    }
    lines.push(format!("Parties       {}", made.parties.len()));
    lines.extend(made.parties.iter().map(|line| format!("  {line}")));
    lines.push(format!(
        "Dialogs       {}, {} s in all",
        made.dialogs,
        masked::js_number(made.seconds)
    ));
    lines.extend(made.dialog.iter().map(|line| format!("  {line}")));
    lines.push(format!("Attachments   {}", made.attachments));
    lines.push(format!("Analyses      {}", made.analyses));
    lines.push(format!("Links         {}", made.links));
    lines
}

/// The answer to the masked view, as its lines.
///
/// # Errors
///
/// [`ApiError::Status`] for a refusal; [`ApiError::Malformed`] for a 200
/// that is not the view, such as the whole vCon a server older than the
/// view sends, which is never shown.
pub fn masked_answer(status: u16, media: &str, body: &[u8]) -> Result<Vec<String>, ApiError> {
    if status == 200 && media.starts_with("application/vcon") {
        return Err(ApiError::Malformed(
            "the server sent the whole vCon, not its masked view: it is older than this terminal UI, so upgrade it; nothing of the vCon is shown"
                .to_owned(),
        ));
    }
    answer::<Summary>(status, body).map(|made| summary_lines(&made))
}

/// The answer to a reveal, as the vCon's text: inflated within
/// [`REVEAL_BODY`] when it was stored gzipped, read as a browser reads it.
///
/// # Errors
///
/// [`ApiError::Status`] for a refusal; [`ApiError::Malformed`] for a body
/// that does not inflate or inflates past the limit.
pub fn reveal_answer(status: u16, media: &str, body: &[u8]) -> Result<String, ApiError> {
    use std::io::Read;
    if status != 200 {
        return answer::<serde_json::Value>(status, body).map(|_| String::new());
    }
    if !media.starts_with("application/vcon+gzip") {
        return Ok(masked::text_of(body));
    }
    let mut inflated = Vec::new();
    flate2::read::MultiGzDecoder::new(body)
        .take(
            u64::try_from(REVEAL_BODY)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut inflated)
        .map_err(|e| ApiError::Malformed(format!("the gzipped vCon does not inflate: {e}")))?;
    if inflated.len() > REVEAL_BODY {
        return Err(ApiError::Malformed(format!(
            "the vCon is longer than {REVEAL_BODY} bytes, the most the terminal UI reads"
        )));
    }
    Ok(masked::text_of(&inflated))
}

/// What the console says of a part of a vCon it cannot read in parts.
const NO_PARTS: &str = "This vCon cannot be read in parts. Reveal the whole vCon instead.";

/// `part` of the vCon `text`, as the detail view shows it: as the console
/// shows each reveal (src/masked.rs `section`), links one a line, and the
/// whole vCon as stored, cut at [`masked::DISPLAY_LIMIT`] characters.
#[must_use]
pub fn reveal_text(text: &str, part: Part) -> String {
    match part {
        Part::All | Part::Audio => {
            let (kept, left) = masked::shown(text);
            if left == 0 {
                kept.to_owned()
            } else {
                format!(
                    "{kept}\n\nOnly the first {} characters are shown; {left} more are not.",
                    masked::DISPLAY_LIMIT
                )
            }
        }
        Part::Links => {
            if masked::section(text, Part::Subject).is_none() {
                return NO_PARTS.to_owned();
            }
            let found = masked::links(text);
            if found.is_empty() {
                return "No links in this vCon.".to_owned();
            }
            found
                .iter()
                .map(|link| format!("{}: {} (never loaded)", link.place, link.url))
                .collect::<Vec<_>>()
                .join("\n")
        }
        Part::Parties | Part::Subject | Part::Dialog | Part::Analysis => {
            masked::section(text, part).unwrap_or_else(|| NO_PARTS.to_owned())
        }
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
    /// How long one request may take; [`TIMEOUT`] outside tests.
    timeout: Duration,
    /// True once [`Api::close`] is called: every request under way ends,
    /// and no later one starts.
    closed: tokio::sync::watch::Sender<bool>,
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
            token,
            client,
            runtime,
            timeout: TIMEOUT,
            closed: tokio::sync::watch::Sender::new(false),
        })
    }

    /// One GET of `path`, read as a `T`.
    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let (status, _, body) = self.fetch(path, MAX_BODY)?;
        answer(status, &body)
    }

    /// One GET of `path`: its status, media type and body, at most `limit`
    /// bytes of it.
    fn fetch(&self, path: &str, limit: usize) -> Result<(u16, String, Vec<u8>), ApiError> {
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
            let media = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|e| unreachable(&e))? {
                if body.len().saturating_add(chunk.len()) > limit {
                    return Err(ApiError::Malformed(format!(
                        "it is longer than {limit} bytes"
                    )));
                }
                body.extend_from_slice(&chunk);
            }
            Ok((status, media, body))
        };
        let mut closed = self.closed.subscribe();
        self.runtime.block_on(async {
            tokio::select! {
                // First, so a request made after the close never starts.
                biased;
                _ = closed.wait_for(|closed| *closed) => Err(was_closed()),
                answer = tokio::time::timeout(self.timeout, exchange) => {
                    answer.unwrap_or_else(|_| Err(no_answer(self.timeout)))
                }
            }
        })
    }
}

/// What a request of a closed client gives.
fn was_closed() -> ApiError {
    ApiError::Unreachable("the client was closed".to_owned())
}

/// What a request that took longer than `timeout` gives.
pub(crate) fn no_answer(timeout: Duration) -> ApiError {
    ApiError::Unreachable(format!("no answer within {} seconds", timeout.as_secs()))
}

impl Api for HttpApi {
    fn key(&self) -> Result<KeyInfo, ApiError> {
        self.get("/key")
    }

    fn page(&self, cursor: Option<&str>) -> Result<Page, ApiError> {
        self.get(&page_path(cursor))
    }

    fn masked(&self, uuid: &str) -> Result<Vec<String>, ApiError> {
        let (status, media, body) = self.fetch(&masked_path(uuid), MAX_BODY)?;
        masked_answer(status, &media, &body)
    }

    fn reveal(&self, uuid: &str, part: Part) -> Result<String, ApiError> {
        let (status, media, body) = self.fetch(&reveal_path(uuid, part), REVEAL_BODY)?;
        reveal_answer(status, &media, &body)
    }

    fn close(&self) {
        // Kept even with no request under way, for the requests after it.
        self.closed.send_replace(true);
    }
}

/// What is wrong with a certificate the client refused, and what to do,
/// for the refusals an operator can act on; `None` for any other, whose
/// own text is shown.
pub(crate) fn certificate_advice(error: &rustls::CertificateError) -> Option<&'static str> {
    use rustls::CertificateError as Refused;
    match error {
        // rustls carries webpki's `CaUsedAsEndEntity` only as an opaque
        // error, so its name in the debug form is all there is to match.
        // The test against a server with such a certificate fails if a
        // rustls release changes it.
        Refused::Other(other) if format!("{other:?}").contains("CaUsedAsEndEntity") => Some(
            "it is marked as a certificate authority (CA:TRUE), and a server's own certificate may not be one; make the server's certificate with basicConstraints=CA:FALSE, as getting started shows",
        ),
        // A trusted certificate of the issuer's name that did not sign it
        // is a bad signature: the server's certificate was replaced and
        // the client file's `ca` still holds the old one.
        Refused::UnknownIssuer | Refused::BadSignature => Some(
            "no certificate this client trusts signed it; set ca in the client file to the server's current certificate authority",
        ),
        Refused::NotValidForName | Refused::NotValidForNameContext { .. } => Some(
            "it is not for the name in url; use a name the certificate holds, or give the server a certificate for this one",
        ),
        Refused::Expired | Refused::ExpiredContext { .. } => {
            Some("it has expired; renew the server's certificate, or correct this machine's clock")
        }
        Refused::NotValidYet | Refused::NotValidYetContext { .. } => Some(
            "it is not valid yet; correct this machine's clock, or wait for the certificate's start",
        ),
        _ => None,
    }
}

/// The TLS library's error at `cause`: the cause itself, or what the I/O
/// errors around it carry. A failed handshake arrives as an I/O error that
/// holds an I/O error that holds it, and `source` does not look inside one.
fn tls_error<'a>(cause: &'a (dyn Error + 'static)) -> Option<&'a rustls::Error> {
    let mut at = cause;
    loop {
        if let Some(found) = at.downcast_ref::<rustls::Error>() {
            return Some(found);
        }
        at = at.downcast_ref::<std::io::Error>()?.get_ref()?;
    }
}

/// A request that got no answer: a refused certificate in words an
/// operator can act on, else the causes reqwest keeps below its own
/// sentence (the refused connection, the handshake that failed).
fn unreachable(error: &reqwest::Error) -> ApiError {
    let mut why = error.to_string();
    let mut cause = error.source();
    while let Some(inner) = cause {
        if let Some(rustls::Error::InvalidCertificate(refused)) = tls_error(inner)
            && let Some(advice) = certificate_advice(refused)
        {
            return ApiError::Untrusted(advice.to_owned());
        }
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

        fn masked(&self, uuid: &str) -> Result<Vec<String>, ApiError> {
            self.asked
                .lock()
                .expect("the lock")
                .push(format!("masked {uuid}"));
            Err(ApiError::Status(404, None))
        }

        fn reveal(&self, uuid: &str, part: Part) -> Result<String, ApiError> {
            self.asked
                .lock()
                .expect("the lock")
                .push(format!("reveal {uuid} {}", part.name()));
            Err(ApiError::Status(404, None))
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

    use super::{
        REVEAL_BODY, masked_answer, masked_path, reveal_answer, reveal_path, reveal_text,
        summary_lines,
    };
    use crate::masked::{Signature, summary};
    use crate::reveal::Part;

    const VCON_UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

    /// The masked view, and the reveals, as `Canned` answers them.
    struct Revealing;

    impl Api for Revealing {
        fn key(&self) -> Result<KeyInfo, ApiError> {
            Err(ApiError::Status(401, None))
        }

        fn page(&self, _: Option<&str>) -> Result<Page, ApiError> {
            Err(ApiError::Status(401, None))
        }

        fn masked(&self, uuid: &str) -> Result<Vec<String>, ApiError> {
            Ok(vec![format!("masked {uuid}")])
        }

        fn reveal(&self, _: &str, _: Part) -> Result<String, ApiError> {
            Ok(r#"{"subject":"Renewal call"}"#.to_owned())
        }
    }

    #[test]
    fn the_masked_and_reveal_effects_are_events_for_their_uuid_and_part() {
        assert_eq!(
            perform(&Revealing, &Effect::FetchMasked(VCON_UUID.to_owned())),
            Event::Masked(
                VCON_UUID.to_owned(),
                Ok(vec![format!("masked {VCON_UUID}")])
            ),
            "the masked view"
        );
        assert_eq!(
            perform(
                &Revealing,
                &Effect::FetchReveal(VCON_UUID.to_owned(), Part::Subject)
            ),
            Event::Revealed(
                VCON_UUID.to_owned(),
                Part::Subject,
                Ok("Renewal call".to_owned())
            ),
            "the part, as the view shows it"
        );
    }

    #[test]
    fn the_paths_ask_for_the_view_and_name_the_part() {
        assert_eq!(
            masked_path(VCON_UUID),
            format!("/vcons/{VCON_UUID}?view=masked"),
            "view"
        );
        assert_eq!(
            reveal_path(VCON_UUID, Part::All),
            format!("/vcons/{VCON_UUID}?reveal=all"),
            "reveal"
        );
    }

    #[test]
    fn the_masked_view_is_shown_as_labeled_lines_with_one_line_a_party_and_a_dialog() {
        let text = r#"{"created_at":"2026-11-10T09:15:00Z","subject":"x","parties":[{"tel":"+12125550142"}],
            "dialog":[{"type":"text","duration":12.5}],"analysis":[{}]}"#;
        assert_eq!(
            summary_lines(&summary(text)),
            [
                "Masked view",
                "Form          unsigned",
                "Created       2026-11-10 09:15:00 UTC+00:00",
                "Subject       present, hidden",
                "Parties       1",
                "  Party 1: number ending 0142",
                "Dialogs       1, 12.5 s in all",
                "  Dialog 1: text, no start, 12.5 s, no parties; no content",
                "Attachments   0",
                "Analyses      1",
                "Links         0",
            ],
            "the lines"
        );
    }

    #[test]
    fn a_signed_vcons_header_is_a_line_and_no_created_time_is_said() {
        let mut made = summary("{}");
        made.signature = Some(Signature {
            alg: "ES256".to_owned(),
            kid: "key-1".to_owned(),
            x5t: String::new(),
        });
        let lines = summary_lines(&made);
        assert!(
            lines.contains(&"Signature     alg ES256, kid key-1, x5t none".to_owned()),
            "{lines:?}"
        );
        assert!(
            lines.contains(&"Created       not given".to_owned()),
            "{lines:?}"
        );
    }

    #[test]
    fn a_masked_answer_is_read_as_its_lines() {
        let body = serde_json::to_vec(&summary("{}")).expect("JSON");
        let lines = masked_answer(200, "application/json", &body).expect("read");
        assert_eq!(
            lines.first().map(String::as_str),
            Some("Masked view"),
            "{lines:?}"
        );
    }

    #[test]
    fn each_part_is_shown_as_the_console_shows_it() {
        let text = r#"{"subject":"Renewal call","parties":[{"name":"Ada Example"}],
            "dialog":[{"url":"https://media.example.com/a.wav"}],"analysis":[]}"#;
        assert_eq!(reveal_text(text, Part::Subject), "Renewal call", "subject");
        assert!(
            reveal_text(text, Part::Parties).contains("\"name\": \"Ada Example\""),
            "parties"
        );
        assert!(
            reveal_text(text, Part::Dialog).contains("media.example.com"),
            "dialog"
        );
        assert_eq!(reveal_text(text, Part::Analysis), "none", "analysis");
        assert_eq!(
            reveal_text(text, Part::Links),
            "dialog 1: https://media.example.com/a.wav (never loaded)",
            "links"
        );
        assert_eq!(
            reveal_text(text, Part::All),
            text,
            "the whole vCon as stored"
        );
    }

    #[test]
    fn a_reveal_answer_is_the_vcons_text() {
        assert_eq!(
            reveal_answer(200, "application/vcon", br#"{"a":1}"#),
            Ok(r#"{"a":1}"#.to_owned()),
            "as stored"
        );
    }

    // Failure.

    /// A server older than the view passes the query over and sends the
    /// whole vCon: it is not shown, and the screen says why.
    #[test]
    fn the_whole_vcon_in_answer_to_the_masked_view_is_refused_and_not_shown() {
        for media in ["application/vcon", "application/vcon+gzip"] {
            let refused = masked_answer(200, media, br#"{"subject":"Renewal call"}"#);
            let Err(ApiError::Malformed(why)) = refused else {
                panic!("{media}: refused, not {refused:?}");
            };
            assert!(why.contains("older"), "{why}");
            assert!(!why.contains("Renewal"), "{why}");
        }
    }

    #[test]
    fn a_refused_masked_view_or_reveal_carries_its_status_and_title() {
        let body = br#"{"title":"forbidden","status":403}"#;
        assert_eq!(
            masked_answer(403, "application/problem+json", body),
            Err(ApiError::Status(403, Some("forbidden".to_owned()))),
            "masked"
        );
        assert_eq!(
            reveal_answer(
                410,
                "application/problem+json",
                br#"{"title":"gone","status":410}"#
            ),
            Err(ApiError::Status(410, Some("gone".to_owned()))),
            "reveal"
        );
    }

    #[test]
    fn a_gzipped_reveal_that_does_not_inflate_is_refused() {
        let refused = reveal_answer(200, "application/vcon+gzip", b"not gzip");
        assert!(
            matches!(refused, Err(ApiError::Malformed(_))),
            "{refused:?}"
        );
    }

    #[test]
    fn a_gzipped_reveal_that_inflates_past_the_limit_is_refused() {
        use std::io::Write as _;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&vec![b' '; REVEAL_BODY + 1]).expect("gzip");
        let body = gz.finish().expect("gzip");
        let refused = reveal_answer(200, "application/vcon+gzip", &body);
        let Err(ApiError::Malformed(why)) = refused else {
            panic!("refused, not {refused:?}");
        };
        assert!(why.contains(&REVEAL_BODY.to_string()), "{why}");
    }

    #[test]
    fn a_masked_answer_that_is_not_the_view_is_malformed() {
        let refused = masked_answer(200, "application/json", br#"{"form":"unsigned"}"#);
        assert!(
            matches!(refused, Err(ApiError::Malformed(_))),
            "{refused:?}"
        );
    }

    #[test]
    fn a_part_of_a_vcon_that_cannot_be_read_in_parts_says_so() {
        for part in [Part::Parties, Part::Subject, Part::Links] {
            assert_eq!(
                reveal_text(r#"{"ciphertext":"x"}"#, part),
                "This vCon cannot be read in parts. Reveal the whole vCon instead.",
                "{part:?}"
            );
        }
    }

    // Edge.

    #[test]
    fn a_vcon_without_links_says_so() {
        assert_eq!(
            reveal_text("{}", Part::Links),
            "No links in this vCon.",
            "none"
        );
    }

    #[test]
    fn a_whole_vcon_longer_than_the_limit_is_cut_and_says_how_much_was_left_out() {
        let text = format!("\"{}\"", "x".repeat(crate::masked::DISPLAY_LIMIT));
        let shown = reveal_text(&text, Part::All);
        assert!(
            shown.ends_with("Only the first 2097152 characters are shown; 2 more are not."),
            "{}",
            shown
                .get(shown.len().saturating_sub(80)..)
                .unwrap_or_default()
        );
    }

    #[test]
    fn a_uuid_in_a_path_is_percent_encoded() {
        assert_eq!(
            masked_path("a b/c"),
            "/vcons/a%20b%2Fc?view=masked",
            "encoded"
        );
    }

    #[test]
    fn a_reveal_answer_with_a_byte_order_mark_loses_it() {
        assert_eq!(
            reveal_answer(200, "application/vcon", b"\xef\xbb\xbf{}"),
            Ok("{}".to_owned()),
            "dropped"
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

    // The HTTPS client, against a TLS server on loopback.

    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::HttpApi;
    use crate::mcp::stdio::ClientConfig;
    use crate::stub_server::StubServer;

    const TOKEN: &str = "vcs_0011223344556677_secret";

    type Certificate = rcgen::CertifiedKey<rcgen::KeyPair>;

    fn certificate() -> Certificate {
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate")
    }

    /// A TLS server for one test: it answers every request with `reply`, a
    /// whole HTTP/1.1 response, and keeps each request's head. The server
    /// ends its threads when dropped; then its port.
    fn server(cert: &Certificate, reply: Vec<u8>) -> (StubServer, u16, Arc<Mutex<Vec<String>>>) {
        let chain = vec![cert.cert.der().clone()];
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.signing_key.serialize_der().into());
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .expect("a server config");
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let config = Arc::new(config);
        let heads = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&heads);
        let stub = StubServer::start(move |tcp| {
            let conn = rustls::ServerConnection::new(Arc::clone(&config)).expect("a connection");
            let mut tls = rustls::StreamOwned::new(conn, tcp);
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match tls.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            if !head.is_empty() {
                seen.lock()
                    .expect("the lock")
                    .push(String::from_utf8_lossy(&head).to_ascii_lowercase());
            }
            // A client that stops reading early closes; that is its
            // test's point, not a failure here.
            drop(tls.write_all(&reply));
            tls.conn.send_close_notify();
            drop(tls.flush());
        });
        let port = stub.address().port();
        (stub, port, heads)
    }

    /// An HTTP/1.1 response with a JSON body.
    fn response(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// A client file's values for the server on `port`, trusting `ca`, with
    /// the token in a file of `mode`.
    fn client(dir: &Path, port: u16, ca: &Certificate, mode: u32) -> ClientConfig {
        use std::os::unix::fs::PermissionsExt;
        let ca_file = dir.join("ca.pem");
        std::fs::write(&ca_file, ca.cert.pem()).expect("write the CA");
        let key_file = dir.join("tui.token");
        std::fs::write(&key_file, format!("{TOKEN}\n")).expect("write the key file");
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(mode)).expect("chmod");
        ClientConfig {
            url: format!("https://localhost:{port}/"),
            key_file,
            ca: Some(ca_file),
        }
    }

    #[test]
    fn the_key_request_carries_the_token_as_a_bearer_and_its_answer_is_read() {
        let cert = certificate();
        let (_stub, port, heads) = server(
            &cert,
            response(
                "200 OK",
                r#"{"id":"0011223344556677","tenant":"t_example","scopes":["read"]}"#,
            ),
        );
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        let info = api.key().expect("the key's answer");
        assert_eq!(
            info.tenant.as_deref(),
            Some("t_example"),
            "the answer is read"
        );
        let heads = heads.lock().expect("the lock");
        assert!(
            heads[0].starts_with("get /key http/1.1"),
            "the route: {}",
            heads[0]
        );
        assert!(
            heads[0].contains(&format!("authorization: bearer {TOKEN}")),
            "the token goes in the Authorization header"
        );
    }

    /// An HTTP/1.1 response with `body` as `content_type`.
    fn typed(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
        let mut made = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        made.extend_from_slice(body);
        made
    }

    #[test]
    fn the_masked_request_asks_for_the_view_and_its_answer_is_read_as_lines() {
        let cert = certificate();
        let body = serde_json::to_vec(&crate::masked::summary("{}")).expect("JSON");
        let (_stub, port, heads) = server(&cert, typed("200 OK", "application/json", &body));
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        let lines = api.masked(VCON_UUID).expect("the masked view");
        assert!(
            lines.contains(&"Form          unsigned".to_owned()),
            "{lines:?}"
        );
        let heads = heads.lock().expect("the lock");
        assert!(
            heads[0].starts_with(&format!("get /vcons/{VCON_UUID}?view=masked http/1.1")),
            "the route: {}",
            heads[0]
        );
        assert!(
            heads[0].contains(&format!("authorization: bearer {TOKEN}")),
            "the token goes in the Authorization header"
        );
    }

    #[test]
    fn the_reveal_request_names_its_part_and_a_gzipped_answer_is_inflated() {
        let cert = certificate();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(br#"{"subject":"Renewal call"}"#)
            .expect("gzip");
        let body = gz.finish().expect("gzip");
        let (_stub, port, heads) = server(&cert, typed("200 OK", "application/vcon+gzip", &body));
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        let text = api.reveal(VCON_UUID, Part::Subject).expect("the vCon");
        assert_eq!(text, r#"{"subject":"Renewal call"}"#, "inflated");
        let heads = heads.lock().expect("the lock");
        assert!(
            heads[0].starts_with(&format!("get /vcons/{VCON_UUID}?reveal=subject http/1.1")),
            "the route: {}",
            heads[0]
        );
    }

    #[test]
    fn the_page_request_carries_the_limit_and_the_cursor() {
        let cert = certificate();
        let (_stub, port, heads) = server(&cert, response("200 OK", r#"{"next":null,"vcons":[]}"#));
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        api.page(Some("abc")).expect("a page");
        let heads = heads.lock().expect("the lock");
        assert!(
            heads[0].starts_with("get /vcons?limit=100&cursor=abc http/1.1"),
            "one slash, the limit and the cursor: {}",
            heads[0]
        );
    }

    #[test]
    fn a_refusal_from_the_server_is_its_status_and_title() {
        let cert = certificate();
        let (_stub, port, _) = server(
            &cert,
            response(
                "401 Unauthorized",
                r#"{"title":"unauthorized","status":401}"#,
            ),
        );
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        assert_eq!(
            api.key(),
            Err(ApiError::Status(401, Some("unauthorized".to_owned()))),
            "the refusal is carried, not hidden"
        );
    }

    #[test]
    fn a_certificate_the_ca_does_not_cover_gets_no_request_and_no_token() {
        let served = certificate();
        let trusted = certificate();
        let (_stub, port, heads) = server(&served, response("200 OK", "{}"));
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &trusted, 0o600)).expect("a client");
        let outcome = api.key();
        assert!(
            matches!(outcome, Err(ApiError::Untrusted(_))),
            "the certificate is verified: {outcome:?}"
        );
        assert_eq!(
            *heads.lock().expect("the lock"),
            Vec::<String>::new(),
            "nothing, the token least of all, is sent to a server that is not trusted"
        );
    }

    // A certificate the client refuses: what the message says.

    /// A certificate for `name` that is its own issuer, changed by `change`
    /// before it is signed.
    fn certificate_with(
        name: &str,
        change: impl FnOnce(&mut rcgen::CertificateParams),
    ) -> Certificate {
        let signing_key = rcgen::KeyPair::generate().expect("a key");
        let mut params = rcgen::CertificateParams::new(vec![name.to_owned()]).expect("params");
        change(&mut params);
        let cert = params.self_signed(&signing_key).expect("a certificate");
        rcgen::CertifiedKey { cert, signing_key }
    }

    /// A self-signed certificate marked as a certificate authority, as
    /// `openssl req -x509` makes one by default.
    fn ca_certificate() -> Certificate {
        certificate_with("localhost", |params| {
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        })
    }

    /// What the UI is told when a server presents `served` and the client
    /// file trusts `trusted`, and the heads of the requests the server got.
    fn refusal(served: &Certificate, trusted: &Certificate) -> (String, Vec<String>) {
        let (_stub, port, heads) = server(served, response("200 OK", "{}"));
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, trusted, 0o600)).expect("a client");
        let error = api.key().expect_err("the certificate is refused");
        let heads = heads.lock().expect("the lock").clone();
        (error.to_string(), heads)
    }

    #[test]
    fn a_ca_certificate_as_the_servers_own_is_said_in_words() {
        let cert = ca_certificate();
        let (said, _) = refusal(&cert, &cert);
        assert!(
            said.starts_with("the server's certificate was refused: ") && said.contains("CA:TRUE"),
            "what is wrong: {said}"
        );
        assert!(
            !said.contains("OtherError") && !said.contains("CaUsedAsEndEntity"),
            "not the library's own text: {said}"
        );
    }

    #[test]
    fn a_ca_certificate_as_the_servers_own_says_what_to_do() {
        let cert = ca_certificate();
        let (said, _) = refusal(&cert, &cert);
        assert!(
            said.contains("basicConstraints=CA:FALSE"),
            "the option that makes a certificate a server's: {said}"
        );
    }

    #[test]
    fn a_ca_certificate_as_the_servers_own_is_still_refused_before_any_request() {
        let cert = ca_certificate();
        let (_, heads) = refusal(&cert, &cert);
        assert_eq!(
            heads,
            Vec::<String>::new(),
            "no request, and so no token, goes to that server"
        );
    }

    #[test]
    fn a_certificate_from_an_issuer_the_client_does_not_know_says_to_set_ca() {
        let served = certificate_with("localhost", |params| {
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, "another issuer");
        });
        let (said, _) = refusal(&served, &certificate());
        assert!(
            said.starts_with("the server's certificate was refused: ")
                && said.contains("set ca in the client file"),
            "what is wrong and what to do: {said}"
        );
    }

    #[test]
    fn a_replaced_certificate_of_the_same_name_says_to_set_ca() {
        // Two self-signed certificates with one subject: the trusted one
        // has the issuer's name and did not sign the served one.
        let (said, _) = refusal(&certificate(), &certificate());
        assert!(
            said.starts_with("the server's certificate was refused: ")
                && said.contains("set ca in the client file"),
            "what is wrong and what to do: {said}"
        );
        assert!(
            !said.contains("BadSignature"),
            "not the library's own text: {said}"
        );
    }

    #[test]
    fn a_certificate_for_another_name_says_so() {
        let cert = certificate_with("other.example.com", |_| {});
        let (said, _) = refusal(&cert, &cert);
        assert!(
            said.starts_with("the server's certificate was refused: ")
                && said.contains("not for the name in url"),
            "what is wrong: {said}"
        );
    }

    #[test]
    fn an_expired_certificate_says_so() {
        let cert = certificate_with("localhost", |params| {
            params.not_before = rcgen::date_time_ymd(2020, 1, 1);
            params.not_after = rcgen::date_time_ymd(2020, 2, 1);
        });
        let (said, _) = refusal(&cert, &cert);
        assert!(
            said.starts_with("the server's certificate was refused: ")
                && said.contains("it has expired"),
            "what is wrong: {said}"
        );
    }

    #[test]
    fn a_certificate_not_valid_yet_says_so() {
        let cert = certificate_with("localhost", |params| {
            params.not_before = rcgen::date_time_ymd(2999, 1, 1);
            params.not_after = rcgen::date_time_ymd(2999, 2, 1);
        });
        let (said, _) = refusal(&cert, &cert);
        assert!(
            said.starts_with("the server's certificate was refused: ")
                && said.contains("it is not valid yet"),
            "what is wrong: {said}"
        );
    }

    #[test]
    fn a_refused_certificate_message_never_holds_the_token() {
        let cert = ca_certificate();
        let (said, _) = refusal(&cert, &cert);
        assert!(!said.contains("secret"), "the token is not in the message");
    }

    #[test]
    fn a_closed_port_is_not_called_a_certificate_refusal() {
        let cert = certificate();
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
            listener.local_addr().expect("an address").port()
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        let said = api.key().expect_err("nothing listens").to_string();
        assert!(
            said.starts_with("the server did not answer: ") && !said.contains("certificate"),
            "no certificate was seen: {said}"
        );
    }

    #[test]
    fn a_refused_certificate_shows_the_connection_as_not_connected() {
        use crate::tui::state::{Connection, State};
        let cert = ca_certificate();
        let (_stub, port, _) = server(&cert, response("200 OK", "{}"));
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        let (mut state, _) = State::new("https://localhost", (80, 24));
        // The key answers by hand; then the list is asked of the server.
        state.step(
            Event::KeyInfo(Ok(KeyInfo {
                id: "0011223344556677".to_owned(),
                tenant: Some("t_example".to_owned()),
                scopes: vec!["read".to_owned()],
            })),
            time::OffsetDateTime::UNIX_EPOCH,
        );
        state.step(
            Event::Page(api.page(None)),
            time::OffsetDateTime::UNIX_EPOCH,
        );
        assert!(
            matches!(&state.connection, Connection::Failed(why) if why.contains("certificate was refused")),
            "a server whose certificate is refused is not connected: {:?}",
            state.connection
        );
    }

    #[test]
    fn a_port_nothing_listens_on_is_unreachable() {
        let cert = certificate();
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
            listener.local_addr().expect("an address").port()
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        let outcome = api.page(None);
        assert!(
            matches!(outcome, Err(ApiError::Unreachable(_))),
            "no server, no answer: {outcome:?}"
        );
    }

    #[test]
    fn an_answer_longer_than_the_limit_is_given_up() {
        let cert = certificate();
        let (_stub, port, _) = server(&cert, response("200 OK", &" ".repeat(2 * super::MAX_BODY)));
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        let outcome = api.page(None);
        assert!(
            matches!(&outcome, Err(ApiError::Malformed(why)) if why.contains("longer than")),
            "the body is not buffered whole: {outcome:?}"
        );
    }

    #[test]
    fn a_server_that_never_answers_is_given_up_after_the_timeout() {
        let cert = certificate();
        // Connections are taken by the kernel and never answered.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        let port = listener.local_addr().expect("an address").port();
        let dir = tempfile::tempdir().expect("temp dir");
        let mut api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        api.timeout = Duration::from_millis(300);
        let outcome = api.key();
        assert!(
            matches!(&outcome, Err(ApiError::Unreachable(why)) if why.contains("no answer within")),
            "the request ends: {outcome:?}"
        );
        drop(listener);
    }

    /// A client of a server whose kernel takes connections that are never
    /// answered, with a timeout far past the test's bounds: a request ends
    /// only by [`Api::close`]. Returns the listener too, which the test
    /// holds open.
    fn api_of_a_server_that_never_answers(
        dir: &std::path::Path,
    ) -> (HttpApi, std::net::TcpListener) {
        let cert = certificate();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        let port = listener.local_addr().expect("an address").port();
        let mut api = HttpApi::new(&client(dir, port, &cert, 0o600)).expect("a client");
        api.timeout = Duration::from_secs(60);
        (api, listener)
    }

    #[test]
    fn close_ends_a_request_under_way_at_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (api, listener) = api_of_a_server_that_never_answers(dir.path());
        let api = std::sync::Arc::new(api);
        let request = {
            let api = std::sync::Arc::clone(&api);
            std::thread::spawn(move || api.key())
        };
        std::thread::sleep(Duration::from_millis(300));
        let closed_at = std::time::Instant::now();
        api.close();
        let outcome = request.join().expect("the request's thread ends");
        assert!(
            closed_at.elapsed() < Duration::from_secs(10),
            "the request ended when the client closed, not at its 60 s timeout: {:?}",
            closed_at.elapsed()
        );
        assert!(
            matches!(&outcome, Err(ApiError::Unreachable(why)) if why.contains("closed")),
            "the request says the client closed: {outcome:?}"
        );
        drop(listener);
    }

    #[test]
    fn a_request_after_close_ends_without_waiting() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (api, listener) = api_of_a_server_that_never_answers(dir.path());
        api.close();
        let began = std::time::Instant::now();
        let outcome = api.page(None);
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "no request starts once the client is closed: {:?}",
            began.elapsed()
        );
        assert!(
            matches!(&outcome, Err(ApiError::Unreachable(why)) if why.contains("closed")),
            "the request says the client closed: {outcome:?}"
        );
        drop(listener);
    }

    #[test]
    fn close_twice_is_the_same_as_once() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (api, listener) = api_of_a_server_that_never_answers(dir.path());
        api.close();
        api.close();
        let outcome = api.masked(UUID_FOR_CLOSE);
        assert!(
            matches!(&outcome, Err(ApiError::Unreachable(why)) if why.contains("closed")),
            "still closed: {outcome:?}"
        );
        drop(listener);
    }

    #[test]
    fn a_client_never_closed_answers_as_before() {
        let cert = certificate();
        let (_stub, port, _) = server(&cert, response("200 OK", r#"{"vcons":[],"next":null}"#));
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), port, &cert, 0o600)).expect("a client");
        let page = api.page(None).expect("a page");
        assert!(page.vcons.is_empty(), "the page was read: {page:?}");
    }

    /// A vCon id for the requests a closed client refuses.
    const UUID_FOR_CLOSE: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc00";

    #[test]
    fn a_key_file_other_users_may_read_is_refused_before_any_request() {
        let cert = certificate();
        let dir = tempfile::tempdir().expect("temp dir");
        let refused = HttpApi::new(&client(dir.path(), 1, &cert, 0o644))
            .expect_err("a world-readable key file");
        assert!(refused.contains("key_file"), "the file is named: {refused}");
        assert!(!refused.contains(TOKEN), "the token is not in the message");
    }

    #[test]
    fn a_ca_file_with_no_certificate_is_refused() {
        let cert = certificate();
        let dir = tempfile::tempdir().expect("temp dir");
        let config = client(dir.path(), 1, &cert, 0o600);
        std::fs::write(
            config.ca.as_ref().expect("a CA file"),
            "not a certificate\n",
        )
        .expect("overwrite the CA");
        let refused = HttpApi::new(&config).expect_err("no certificate to trust");
        assert!(
            refused.starts_with("ca: "),
            "the setting is named: {refused}"
        );
    }

    #[test]
    fn the_debug_form_never_holds_the_token() {
        let cert = certificate();
        let dir = tempfile::tempdir().expect("temp dir");
        let api = HttpApi::new(&client(dir.path(), 1, &cert, 0o600)).expect("a client");
        let shown = format!("{api:?}");
        assert!(
            shown.contains("https://localhost:1"),
            "the address is shown: {shown}"
        );
        assert!(!shown.contains("secret"), "the token is not: {shown}");
    }
}
