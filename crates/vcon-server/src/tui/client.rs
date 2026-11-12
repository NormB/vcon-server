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
    /// How long one request may take; [`TIMEOUT`] outside tests.
    timeout: Duration,
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
            timeout: TIMEOUT,
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
            tokio::time::timeout(self.timeout, exchange)
                .await
                .unwrap_or_else(|_| Err(no_answer(self.timeout)))
        })
    }
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

    // The HTTPS client, against a TLS server on loopback.

    use std::io::{Read, Write};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::HttpApi;
    use crate::mcp::stdio::ClientConfig;

    const TOKEN: &str = "vcs_0011223344556677_secret";

    type Certificate = rcgen::CertifiedKey<rcgen::KeyPair>;

    fn certificate() -> Certificate {
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate")
    }

    /// A TLS server for one test: it answers every request with `reply`, a
    /// whole HTTP/1.1 response, and keeps each request's head.
    fn server(cert: &Certificate, reply: Vec<u8>) -> (u16, Arc<Mutex<Vec<String>>>) {
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
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        let port = listener.local_addr().expect("an address").port();
        let heads = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&heads);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(tcp) = stream else { return };
                let conn =
                    rustls::ServerConnection::new(Arc::clone(&config)).expect("a connection");
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
            }
        });
        (port, heads)
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
        let (port, heads) = server(
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

    #[test]
    fn the_page_request_carries_the_limit_and_the_cursor() {
        let cert = certificate();
        let (port, heads) = server(&cert, response("200 OK", r#"{"next":null,"vcons":[]}"#));
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
        let (port, _) = server(
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
        let (port, heads) = server(&served, response("200 OK", "{}"));
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
        let (port, heads) = server(served, response("200 OK", "{}"));
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
        let (port, _) = server(&cert, response("200 OK", "{}"));
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
        let (port, _) = server(&cert, response("200 OK", &" ".repeat(2 * super::MAX_BODY)));
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
