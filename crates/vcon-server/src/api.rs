//! The HTTP API, served over TLS only.

use std::collections::BTreeSet;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{
    AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, ETAG, LOCATION, WWW_AUTHENTICATE,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use http_body_util::LengthLimitError;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::access::{self, AuthError, Caller, Scope};
use crate::audit::{Action, Actor, Entry};
use crate::ids::TenantId;
use crate::ids::VersionId;
use crate::store::{Caller as StoreCaller, Created, Store, StoreError};
use crate::tenants::Tenants;
use vcon_form::Rejection;

/// The limits from configuration that the API applies to every request.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The most bytes a request body may hold, as received.
    pub body: usize,
    /// The most bytes gzip inflation may produce while a body is inspected.
    pub inflated: u64,
    /// How long a request body may take to arrive.
    pub body_timeout: Duration,
}

/// What the API serves from.
pub struct App {
    pub tenants: Arc<Tenants>,
    pub limits: Limits,
    /// The must_support extensions this server supports.
    pub supported: BTreeSet<String>,
}

/// The routes, served from `app`.
pub fn router(app: Arc<App>) -> Router {
    Router::new().route("/vcons", post(create)).with_state(app)
}

/// `POST /vcons`: stores a new vCon, or answers 200 when the same bytes
/// are already current.
async fn create(State(app): State<Arc<App>>, headers: HeaderMap, body: Body) -> Response {
    let content_type = headers
        .get(CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or("\u{0}").to_owned());
    let admitted = match admit(&app, &headers, body, Scope::Write, Action::Create).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let entry = |refused: Problem| Entry {
        actor: Actor::Key(admitted.caller.key),
        action: Action::Create,
        target: None,
        version: None,
        status: refused.status().as_u16(),
    };
    if !content_type.as_deref().is_none_or(is_vcon_media_type) {
        let refused = Problem::UnsupportedMediaType;
        let tenant = Some(admitted.tenant);
        return app
            .refuse(tenant, entry(refused), refused)
            .await
            .into_response();
    }
    let worker = app.clone();
    let (store, body, key) = (
        admitted.store.clone(),
        admitted.body.clone(),
        admitted.caller.key,
    );
    let outcome = blocking(move || {
        let limits = vcon_form::Limits {
            inflated: worker.limits.inflated,
        };
        let found = vcon_form::inspect(&body, content_type.as_deref(), &limits, &worker.supported)
            .map_err(|r| Err(rejected(&r)))?;
        store
            .create(&StoreCaller { key }, &found, &body)
            .map(|created| (found.uuid, created))
            .map_err(|e| Ok(failed(e)))
    })
    .await;
    match outcome {
        Ok((uuid, Created::New(version))) => (
            StatusCode::CREATED,
            [
                (LOCATION, format!("/vcons/{uuid}")),
                (ETAG, entity_tag(version)),
            ],
        )
            .into_response(),
        Ok((_, Created::Same(version))) => {
            (StatusCode::OK, [(ETAG, entity_tag(version))]).into_response()
        }
        // Store records its own refusals; Format's are recorded here.
        Err(Ok(refused)) => refused.into_response(),
        Err(Err(refused)) => app
            .refuse(Some(admitted.tenant), entry(refused), refused)
            .await
            .into_response(),
    }
}

/// A version id as a strong entity tag.
fn entity_tag(version: VersionId) -> String {
    format!("\"{version}\"")
}

/// Whether a request's media type is one a vCon may be sent as:
/// `application/vcon`, `application/vcon+gzip` or `application/json`, in
/// any case and with any parameters.
fn is_vcon_media_type(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or("").trim();
    [
        "application/vcon",
        "application/vcon+gzip",
        "application/json",
    ]
    .iter()
    .any(|taken| essence.eq_ignore_ascii_case(taken))
}

/// A request the pipeline let through, with its body read.
struct Admitted {
    caller: Caller,
    tenant: TenantId,
    store: Arc<Store>,
    body: Bytes,
}

/// The steps every vCon request takes before its route's own work, in
/// order: authenticate, check the scope, open the caller's tenant, read
/// the body within the limits. A refusal at any step is recorded here,
/// since Store never sees the request.
async fn admit(
    app: &Arc<App>,
    headers: &HeaderMap,
    body: Body,
    scope: Scope,
    action: Action,
) -> Result<Admitted, Problem> {
    let authorization = headers
        .get(AUTHORIZATION)
        .map(|v| v.to_str().unwrap_or_default().to_owned());
    let tenants = app.tenants.clone();
    let caller = blocking(move || access::authenticate(&tenants, authorization.as_deref()))
        .await
        .map_err(|e| match e {
            AuthError::Io => Problem::Unavailable,
            _ => Problem::Unauthorized,
        })?;
    let entry = |status: Problem| Entry {
        actor: Actor::Key(caller.key),
        action,
        target: None,
        version: None,
        status: status.status().as_u16(),
    };
    let Ok(tenant) = access::tenant_for(&caller, scope) else {
        let refused = Problem::Forbidden;
        return Err(app.refuse(caller.tenant, entry(refused), refused).await);
    };
    let tenants = app.tenants.clone();
    let Ok(store) = blocking(move || tenants.store(tenant)).await else {
        let refused = Problem::Unavailable;
        return Err(app.refuse(None, entry(refused), refused).await);
    };
    match read_body(body, headers, &app.limits).await {
        Ok(body) => Ok(Admitted {
            caller,
            tenant,
            store,
            body,
        }),
        Err(refused) => Err(app.refuse(Some(tenant), entry(refused), refused).await),
    }
}

impl App {
    /// Records a refusal in the tenant's log, or the admin log when there
    /// is no tenant or its log cannot be opened, and answers it; a refusal
    /// that cannot be recorded answers that the audit failed.
    async fn refuse(&self, tenant: Option<TenantId>, entry: Entry, refused: Problem) -> Problem {
        let tenants = self.tenants.clone();
        let recorded = blocking(move || match tenant.and_then(|t| tenants.store(t).ok()) {
            Some(store) => store.log(&entry).is_ok(),
            None => tenants.record_alone(&entry).is_ok(),
        })
        .await;
        if recorded {
            refused
        } else {
            Problem::AuditFailed
        }
    }
}

/// Reads a body within the size limit and the time limit. A declared
/// length over the limit is refused before anything is read, and a body
/// that passes it is refused as soon as it does.
async fn read_body(body: Body, headers: &HeaderMap, limits: &Limits) -> Result<Bytes, Problem> {
    let declared = headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|length| length > limits.body as u64) {
        return Err(Problem::TooLarge);
    }
    match tokio::time::timeout(limits.body_timeout, axum::body::to_bytes(body, limits.body)).await {
        Err(_) => Err(Problem::Timeout),
        Ok(Ok(body)) => Ok(body),
        Ok(Err(e)) if std::error::Error::source(&e).is_some_and(|s| s.is::<LengthLimitError>()) => {
            Err(Problem::TooLarge)
        }
        Ok(Err(_)) => Err(Problem::BadBody),
    }
}

/// Runs blocking work, SQLite and files, off the async workers.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .expect("blocking work does not panic")
}

/// An error answer: an RFC 9457 problem document with one type per error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Problem {
    Unauthorized,
    Forbidden,
    TooLarge,
    Timeout,
    BadBody,
    Unavailable,
    AuditFailed,
    UnsupportedMediaType,
    /// Format refused the body.
    Invalid(Invalid),
    /// Store refused or failed the request.
    Store(StoreError),
}

/// Why Format refused a body, one problem type each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Invalid {
    BadGzip,
    NotJson,
    DuplicateMember,
    AmbiguousForm,
    NoForm,
    MissingUuid,
    BadUuid,
    MissingCreatedAt,
    BadPayload,
    UuidMismatch,
    BadCritical,
    UnsupportedExtension,
}

/// The problem for Format's refusal. Inflating past the limit is a body
/// too large; the others are unprocessable. No problem repeats the body,
/// so unsupported extensions are not named.
fn rejected(rejection: &Rejection) -> Problem {
    Problem::Invalid(match rejection {
        Rejection::TooLarge => return Problem::TooLarge,
        Rejection::BadGzip => Invalid::BadGzip,
        Rejection::NotJson => Invalid::NotJson,
        Rejection::DuplicateKey => Invalid::DuplicateMember,
        Rejection::AmbiguousForm => Invalid::AmbiguousForm,
        Rejection::NoForm => Invalid::NoForm,
        Rejection::MissingUuid => Invalid::MissingUuid,
        Rejection::BadUuid => Invalid::BadUuid,
        Rejection::MissingCreatedAt => Invalid::MissingCreatedAt,
        Rejection::BadPayload => Invalid::BadPayload,
        Rejection::UuidMismatch => Invalid::UuidMismatch,
        Rejection::BadCritical => Invalid::BadCritical,
        Rejection::UnsupportedCritical(_) => Invalid::UnsupportedExtension,
    })
}

/// The problem for Store's refusal or failure.
fn failed(error: StoreError) -> Problem {
    Problem::Store(error)
}

impl Problem {
    fn status(self) -> StatusCode {
        match self {
            Problem::Unauthorized => StatusCode::UNAUTHORIZED,
            Problem::Forbidden => StatusCode::FORBIDDEN,
            Problem::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Problem::Timeout => StatusCode::REQUEST_TIMEOUT,
            Problem::BadBody => StatusCode::BAD_REQUEST,
            Problem::Unavailable | Problem::AuditFailed => StatusCode::SERVICE_UNAVAILABLE,
            Problem::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Problem::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Problem::Store(error) => match error {
                StoreError::Conflict | StoreError::NotDeleted | StoreError::Immutable => {
                    StatusCode::CONFLICT
                }
                StoreError::Gone => StatusCode::GONE,
                StoreError::NotFound => StatusCode::NOT_FOUND,
                StoreError::PreconditionFailed => StatusCode::PRECONDITION_FAILED,
                StoreError::LimitReached | StoreError::AuditFailed => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
                StoreError::Damaged | StoreError::Io => StatusCode::INTERNAL_SERVER_ERROR,
            },
        }
    }

    /// The last part of its type URI, and its title.
    fn name(self) -> (&'static str, &'static str) {
        match self {
            Problem::Unauthorized => ("unauthorized", "A valid bearer key is needed"),
            Problem::Forbidden => ("forbidden", "The key does not allow this"),
            Problem::TooLarge => ("too-large", "The body is over the size limit"),
            Problem::Timeout => ("timeout", "The body did not arrive in time"),
            Problem::BadBody => ("bad-body", "The body could not be read"),
            Problem::Unavailable => ("unavailable", "The service cannot answer now"),
            Problem::AuditFailed => ("audit-failed", "The audit record could not be written"),
            Problem::UnsupportedMediaType => {
                ("unsupported-media-type", "This media type is not taken")
            }
            Problem::Invalid(invalid) => match invalid {
                Invalid::BadGzip => ("bad-gzip", "The body is marked gzip but is not"),
                Invalid::NotJson => ("not-json", "The body is not one JSON object"),
                Invalid::DuplicateMember => ("duplicate-member", "An object repeats a member"),
                Invalid::AmbiguousForm => {
                    ("ambiguous-form", "The body is both signed and encrypted")
                }
                Invalid::NoForm => ("no-form", "The body is in no vCon form"),
                Invalid::MissingUuid => ("missing-uuid", "The vCon has no uuid"),
                Invalid::BadUuid => ("bad-uuid", "The uuid is not in RFC 9562 text form"),
                Invalid::MissingCreatedAt => ("missing-created-at", "The vCon has no created_at"),
                Invalid::BadPayload => ("bad-payload", "The signed payload cannot be read"),
                Invalid::UuidMismatch => ("uuid-mismatch", "A signature names another uuid"),
                Invalid::BadCritical => (
                    "bad-critical",
                    "critical or must_support is not a list of names",
                ),
                Invalid::UnsupportedExtension => (
                    "unsupported-extension",
                    "An extension the vCon needs is not supported",
                ),
            },
            Problem::Store(error) => match error {
                StoreError::Conflict => ("conflict", "Another vCon holds the uuid"),
                StoreError::Gone => ("gone", "The vCon was deleted or purged"),
                StoreError::NotFound => ("not-found", "No vCon has the uuid"),
                StoreError::NotDeleted => ("not-deleted", "The vCon is not deleted"),
                StoreError::PreconditionFailed => (
                    "precondition-failed",
                    "If-Match does not name the current version",
                ),
                StoreError::Immutable => {
                    ("immutable", "A signed or encrypted vCon is not replaced")
                }
                StoreError::LimitReached => ("limit-reached", "The tenant key must be rotated"),
                StoreError::Damaged => ("damaged", "A stored version is damaged"),
                StoreError::AuditFailed => {
                    ("audit-failed", "The audit record could not be written")
                }
                StoreError::Io => ("internal", "The server failed"),
            },
        }
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let (name, title) = self.name();
        let status = self.status();
        let body = serde_json::json!({
            "type": format!("https://vcon-server.com/problems/{name}"),
            "title": title,
            "status": status.as_u16(),
        });
        let mut answer = (
            status,
            [(CONTENT_TYPE, "application/problem+json")],
            body.to_string(),
        )
            .into_response();
        if self == Problem::Unauthorized {
            answer
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        answer
    }
}

/// Why the TLS configuration could not be made.
#[derive(Debug)]
pub enum TlsError {
    /// The certificate file could not be read, or holds no certificate.
    Certificate,
    /// The key file could not be read, or holds no key.
    Key,
    /// rustls refused the pair, for instance a key that is not the
    /// certificate's.
    Refused(rustls::Error),
}

/// The server's TLS configuration from a PEM certificate chain and a PEM
/// private key.
pub fn tls_config(certificate: &Path, key: &Path) -> Result<Arc<ServerConfig>, TlsError> {
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(certificate)
        .map_err(|_| TlsError::Certificate)?
        .collect::<Result<_, _>>()
        .map_err(|_| TlsError::Certificate)?;
    if chain.is_empty() {
        return Err(TlsError::Certificate);
    }
    let key = PrivateKeyDer::from_pem_file(key).map_err(|_| TlsError::Key)?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(TlsError::Refused)?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(TlsError::Refused)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Serves `app` on `listener` over TLS until `shutdown` completes. A
/// connection whose handshake fails, such as plain HTTP, is dropped; the
/// server goes on.
pub async fn serve(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    app: Router,
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    let acceptor = TlsAcceptor::from(tls);
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        let (tcp, _) = tokio::select! {
            accepted = listener.accept() => accepted?,
            () = &mut shutdown => return Ok(()),
        };
        let acceptor = acceptor.clone();
        let service = TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            let Ok(stream) = acceptor.accept(tcp).await else {
                return;
            };
            // The connection's own errors end only the connection.
            let _ = auto::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::path::PathBuf;

    use axum::routing::get;

    use super::*;

    /// A self-signed certificate for localhost and its key, as PEM files.
    struct Certificate {
        _dir: tempfile::TempDir,
        cert: PathBuf,
        key: PathBuf,
        pem: String,
    }

    fn certificate() -> Certificate {
        let dir = tempfile::tempdir().expect("temp dir");
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate");
        let pem = made.cert.pem();
        let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        std::fs::write(&cert, &pem).expect("write");
        std::fs::write(&key, made.signing_key.serialize_pem()).expect("write");
        Certificate {
            _dir: dir,
            cert,
            key,
            pem,
        }
    }

    fn client(c: &Certificate, addr: SocketAddr) -> reqwest::Client {
        reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(c.pem.as_bytes()).expect("pem")])
            .resolve("localhost", addr)
            .no_proxy()
            .build()
            .expect("a client")
    }

    #[tokio::test]
    async fn the_listener_speaks_tls_only() {
        let c = certificate();
        let tls = tls_config(&c.cert, &c.key).expect("loads");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let app = Router::new().route("/ping", get(|| async { "pong" }));
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, tls, app, async {
            stopped.await.ok();
        }));
        let plain = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("a client")
            .get(format!("http://{addr}/ping"))
            .send()
            .await;
        assert!(plain.is_err(), "plain HTTP is not served: {plain:?}");
        let answer = client(&c, addr)
            .get(format!("https://localhost:{}/ping", addr.port()))
            .send()
            .await
            .expect("TLS is served, after a plain-HTTP attempt too");
        assert_eq!(answer.status(), 200);
        assert_eq!(answer.version(), reqwest::Version::HTTP_2, "h2 by ALPN");
        assert_eq!(answer.text().await.expect("body"), "pong");
        stop.send(()).expect("stop");
        server.await.expect("joins").expect("serves");
    }

    #[test]
    fn a_missing_or_unusable_certificate_or_key_is_refused() {
        let c = certificate();
        let other = certificate();
        let dir = tempfile::tempdir().expect("temp dir");
        let (missing, empty) = (dir.path().join("missing"), dir.path().join("empty"));
        std::fs::write(&empty, "").expect("write");
        assert!(matches!(
            tls_config(&missing, &c.key),
            Err(TlsError::Certificate)
        ));
        assert!(matches!(
            tls_config(&empty, &c.key),
            Err(TlsError::Certificate)
        ));
        assert!(matches!(tls_config(&c.cert, &missing), Err(TlsError::Key)));
        assert!(matches!(tls_config(&c.cert, &empty), Err(TlsError::Key)));
        assert!(matches!(
            tls_config(&c.cert, &other.key),
            Err(TlsError::Refused(_))
        ));
        assert!(tls_config(&c.cert, &c.key).is_ok());
    }

    mod pipeline {
        use std::net::SocketAddr;

        use serde_json::Value;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        use super::*;
        use crate::access::{Scope, Scopes};
        use crate::audit::Actor;
        use crate::clock::SystemClock;
        use crate::envelope::Key;
        use crate::fs::OsFs;
        use crate::ids::TenantId;

        const LIMIT: usize = 64 * 1024;

        /// A server with one tenant, on a free port.
        pub(super) struct Server {
            _dir: tempfile::TempDir,
            certificate: Certificate,
            addr: SocketAddr,
            pub(super) tenants: Arc<Tenants>,
            pub(super) tenant: TenantId,
            stop: Option<tokio::sync::oneshot::Sender<()>>,
        }

        impl Drop for Server {
            fn drop(&mut self) {
                if let Some(stop) = self.stop.take() {
                    let _ = stop.send(());
                }
            }
        }

        pub(super) async fn server() -> Server {
            server_with(&[]).await
        }

        /// A server that supports the must_support extensions named.
        pub(super) async fn server_with(supported: &[&str]) -> Server {
            let dir = tempfile::tempdir().expect("temp dir");
            let tenants = Arc::new(
                Tenants::open(
                    dir.path(),
                    Key::generate().expect("master key"),
                    Arc::new(OsFs),
                    Arc::new(SystemClock),
                )
                .expect("opens"),
            );
            let tenant = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
            let app = Arc::new(App {
                tenants: tenants.clone(),
                limits: Limits {
                    body: LIMIT,
                    inflated: 1 << 20,
                    body_timeout: Duration::from_millis(300),
                },
                supported: supported.iter().map(|s| (*s).to_owned()).collect(),
            });
            let certificate = certificate();
            let tls = tls_config(&certificate.cert, &certificate.key).expect("tls");
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let addr = listener.local_addr().expect("addr");
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            tokio::spawn(serve(listener, tls, router(app), async {
                stopped.await.ok();
            }));
            Server {
                _dir: dir,
                certificate,
                addr,
                tenants,
                tenant,
                stop: Some(stop),
            }
        }

        impl Server {
            pub(super) fn url(&self, path: &str) -> String {
                format!("https://localhost:{}{path}", self.addr.port())
            }

            pub(super) fn client(&self) -> reqwest::Client {
                client(&self.certificate, self.addr)
            }

            pub(super) fn key(&self, scopes: &[Scope]) -> String {
                self.tenants
                    .create_key(Actor::Cli, Some(self.tenant), Scopes::of(scopes), "test")
                    .expect("a key")
                    .token
            }

            pub(super) fn tenant_log(&self, action: &str) -> Vec<Value> {
                let store = self.tenants.store(self.tenant).expect("its store");
                store
                    .records()
                    .into_iter()
                    .filter(|r| r["action"] == action)
                    .collect()
            }

            /// Sends a POST /vcons over HTTP/1.1 with a chunked body: the
            /// chunks given, and then, unless `finish`, nothing more while
            /// the connection stays open. Returns the status line.
            async fn chunked(&self, token: &str, chunks: &[usize], finish: bool) -> String {
                let mut body = String::new();
                for &size in chunks {
                    body.push_str(&format!("{size:x}\r\n{}\r\n", "x".repeat(size)));
                }
                if finish {
                    body.push_str("0\r\n\r\n");
                }
                self.raw(token, "transfer-encoding: chunked", body.as_bytes())
                    .await
            }

            /// Sends a POST /vcons over HTTP/1.1 with one header of the
            /// caller's and the bytes given, keeping the connection open.
            /// Returns the status line.
            async fn raw(&self, token: &str, header: &str, body: &[u8]) -> String {
                let mut roots = rustls::RootCertStore::empty();
                for cert in CertificateDer::pem_slice_iter(self.certificate.pem.as_bytes()) {
                    roots.add(cert.expect("pem")).expect("a root");
                }
                let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
                let mut config = rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .expect("versions")
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                config.alpn_protocols = vec![b"http/1.1".to_vec()];
                let tcp = tokio::net::TcpStream::connect(self.addr)
                    .await
                    .expect("connect");
                let mut tls = tokio_rustls::TlsConnector::from(Arc::new(config))
                    .connect("localhost".try_into().expect("name"), tcp)
                    .await
                    .expect("handshake");
                let head = format!(
                    "POST /vcons HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer {token}\r\n\
                     content-type: application/vcon\r\n{header}\r\n\r\n"
                );
                tls.write_all(head.as_bytes()).await.expect("head");
                // The server may answer and stop reading before all is sent.
                let _ = tls.write_all(body).await;
                let mut answer = vec![0; 4096];
                let read = tokio::time::timeout(Duration::from_secs(10), tls.read(&mut answer))
                    .await
                    .expect("an answer while the body is still open")
                    .expect("read");
                let answer = String::from_utf8_lossy(&answer[..read]).into_owned();
                answer.lines().next().unwrap_or_default().to_owned()
            }
        }

        async fn post(s: &Server, token: Option<&str>, body: Vec<u8>) -> reqwest::Response {
            let mut request = s
                .client()
                .post(s.url("/vcons"))
                .header("content-type", "application/vcon")
                .body(body);
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send().await.expect("an answer")
        }

        /// Checks the answer is a problem document for its status, and
        /// returns its type's last part.
        pub(super) async fn problem(answer: reqwest::Response) -> String {
            assert_eq!(
                answer.headers()["content-type"],
                "application/problem+json",
                "{answer:?}"
            );
            let status = answer.status().as_u16();
            let body: Value =
                serde_json::from_str(&answer.text().await.expect("text")).expect("JSON");
            assert_eq!(body["status"], status);
            assert!(body["title"].as_str().is_some_and(|t| !t.is_empty()));
            body["type"]
                .as_str()
                .and_then(|t| t.strip_prefix("https://vcon-server.com/problems/"))
                .expect("a type under the server's domain")
                .to_owned()
        }

        #[tokio::test]
        async fn a_request_without_a_valid_key_is_unauthorized() {
            let s = server().await;
            for token in [None, Some("vcs_nonsense")] {
                let answer = post(&s, token, b"{}".to_vec()).await;
                assert_eq!(answer.status(), 401, "{token:?}");
                assert_eq!(answer.headers()["www-authenticate"], "Bearer");
                assert_eq!(problem(answer).await, "unauthorized");
            }
        }

        #[tokio::test]
        async fn a_key_table_that_cannot_be_read_makes_the_service_unavailable() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            s.tenants
                .conn_for_test()
                .execute_batch("ALTER TABLE api_key RENAME TO gone")
                .expect("lose the table");
            let answer = post(&s, Some(&writer), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }

        #[tokio::test]
        async fn a_refusal_that_cannot_be_recorded_answers_that_the_audit_failed() {
            let s = server().await;
            let reader = s.key(&[Scope::Read]);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let answer = post(&s, Some(&reader), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");

            let operator = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made")
                .token;
            s.tenants.break_audit();
            let answer = post(&s, Some(&operator), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }

        #[tokio::test]
        async fn a_key_without_the_scope_is_forbidden_and_recorded() {
            let s = server().await;
            let reader = s.key(&[Scope::Read]);
            let answer = post(&s, Some(&reader), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 403);
            assert_eq!(problem(answer).await, "forbidden");
            let refused = s.tenant_log("create");
            assert_eq!(refused.len(), 1);
            assert_eq!(refused[0]["status"], 403);

            let operator = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made")
                .token;
            let answer = post(&s, Some(&operator), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 403);
            let admin: Vec<Value> = s
                .tenants
                .admin_records()
                .into_iter()
                .filter(|r| r["action"] == "create")
                .collect();
            assert_eq!(admin.len(), 1, "an operator's refusal is in the admin log");
            assert_eq!(admin[0]["status"], 403);
        }

        #[tokio::test]
        async fn a_closing_tenant_is_unavailable() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            s.tenants.close(s.tenant);
            let answer = post(&s, Some(&writer), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }

        #[tokio::test]
        async fn a_declared_length_over_the_limit_is_refused_at_once() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let answer = post(&s, Some(&writer), vec![b'x'; LIMIT + 1]).await;
            assert_eq!(answer.status(), 413);
            assert_eq!(problem(answer).await, "too-large");
            assert_eq!(s.tenant_log("create")[0]["status"], 413);
            let declared = format!("content-length: {}", LIMIT + 1);
            assert_eq!(
                s.raw(&writer, &declared, b"").await,
                "HTTP/1.1 413 Payload Too Large",
                "refused on the declared length, with no byte of the body sent"
            );
        }

        #[tokio::test]
        async fn a_body_over_the_limit_is_refused_before_it_is_read_in_full() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let status = s.chunked(&writer, &[LIMIT / 2, LIMIT / 2 + 1], false).await;
            assert_eq!(status, "HTTP/1.1 413 Payload Too Large");
        }

        #[tokio::test]
        async fn a_body_that_stops_arriving_times_out() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let status = s.chunked(&writer, &[10], false).await;
            assert_eq!(status, "HTTP/1.1 408 Request Timeout");
            assert_eq!(s.tenant_log("create")[0]["status"], 408);
        }

        #[tokio::test]
        /// The body reaches Format whole: a body of x's at the limit is
        /// refused as not JSON, not as too large.
        async fn a_request_within_the_limits_goes_on_to_its_route() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let status = s.chunked(&writer, &[LIMIT / 2, LIMIT / 2], true).await;
            assert_eq!(status, "HTTP/1.1 422 Unprocessable Entity");
            let answer = post(&s, Some(&writer), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "no-form");
        }
    }

    #[test]
    fn every_refusal_from_format_has_its_own_problem() {
        let cases = [
            (Rejection::TooLarge, 413, "too-large"),
            (Rejection::BadGzip, 422, "bad-gzip"),
            (Rejection::NotJson, 422, "not-json"),
            (Rejection::DuplicateKey, 422, "duplicate-member"),
            (Rejection::AmbiguousForm, 422, "ambiguous-form"),
            (Rejection::NoForm, 422, "no-form"),
            (Rejection::MissingUuid, 422, "missing-uuid"),
            (Rejection::BadUuid, 422, "bad-uuid"),
            (Rejection::MissingCreatedAt, 422, "missing-created-at"),
            (Rejection::BadPayload, 422, "bad-payload"),
            (Rejection::UuidMismatch, 422, "uuid-mismatch"),
            (Rejection::BadCritical, 422, "bad-critical"),
            (
                Rejection::UnsupportedCritical(vec!["secret-name".to_owned()]),
                422,
                "unsupported-extension",
            ),
        ];
        for (rejection, status, name) in &cases {
            let problem = rejected(rejection);
            assert_eq!(
                (problem.status().as_u16(), problem.name().0),
                (*status, *name),
                "{rejection:?}"
            );
            let answer = format!("{:?}", problem.into_response().into_body());
            assert!(!answer.contains("secret-name"), "the body is not repeated");
        }
    }

    #[test]
    fn every_store_outcome_has_its_problem() {
        let cases = [
            (StoreError::Conflict, 409, "conflict"),
            (StoreError::Gone, 410, "gone"),
            (StoreError::NotFound, 404, "not-found"),
            (StoreError::NotDeleted, 409, "not-deleted"),
            (StoreError::PreconditionFailed, 412, "precondition-failed"),
            (StoreError::Immutable, 409, "immutable"),
            (StoreError::LimitReached, 503, "limit-reached"),
            (StoreError::Damaged, 500, "damaged"),
            (StoreError::AuditFailed, 503, "audit-failed"),
            (StoreError::Io, 500, "internal"),
        ];
        for (error, status, name) in cases {
            let problem = failed(error);
            assert_eq!(
                (problem.status().as_u16(), problem.name().0),
                (status, name),
                "{error:?}"
            );
        }
    }

    mod vcons {
        use std::io::Write;

        use super::pipeline::{Server, problem, server};
        use crate::access::Scope;
        use crate::store::Caller as StoreCaller;
        use crate::store::testing::vcon;

        const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

        fn uuid(n: u8) -> String {
            format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}")
        }

        async fn post(
            s: &Server,
            token: &str,
            content_type: Option<&str>,
            body: Vec<u8>,
        ) -> reqwest::Response {
            let mut request = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(token)
                .body(body);
            if let Some(content_type) = content_type {
                request = request.header("content-type", content_type);
            }
            request.send().await.expect("an answer")
        }

        fn gzip(body: &[u8]) -> Vec<u8> {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(body).expect("gzip");
            encoder.finish().expect("gzip")
        }

        fn store_caller() -> StoreCaller {
            StoreCaller {
                key: crate::ids::KeyId([9; 8]),
            }
        }

        #[tokio::test]
        async fn a_new_vcon_is_created_with_its_location_and_tag() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (found, body) = vcon(UUID, "first");
            let answer = post(&s, &writer, Some("application/vcon"), body.clone()).await;
            assert_eq!(answer.status(), 201);
            assert_eq!(answer.headers()["location"], format!("/vcons/{UUID}"));
            let tag = answer.headers()["etag"].to_str().expect("ascii").to_owned();
            let stored = s
                .tenants
                .store(s.tenant)
                .expect("its store")
                .get(&store_caller(), found.uuid)
                .expect("stored");
            assert_eq!(tag, format!("\"{}\"", stored.version), "a strong tag");
            assert_eq!(stored.body, body);

            let again = post(&s, &writer, Some("application/vcon"), body).await;
            assert_eq!(again.status(), 200, "the same bytes are already current");
            assert_eq!(again.headers()["etag"], tag.as_str());
        }

        #[tokio::test]
        async fn another_vcon_with_the_uuid_conflicts() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (_, first) = vcon(UUID, "first");
            let (_, other) = vcon(UUID, "other");
            assert_eq!(post(&s, &writer, None, first).await.status(), 201);
            let answer = post(&s, &writer, None, other).await;
            assert_eq!(answer.status(), 409);
            assert_eq!(problem(answer).await, "conflict");
            let statuses: Vec<_> = s
                .tenant_log("create")
                .into_iter()
                .map(|r| r["status"].clone())
                .collect();
            assert_eq!(statuses, [201, 409], "Store records its own, once");
        }

        #[tokio::test]
        async fn a_vcon_needing_an_unsupported_extension_is_unprocessable() {
            let s = super::pipeline::server_with(&["known"]).await;
            let writer = s.key(&[Scope::Write]);
            let needing = |n: u8, extension: &str| {
                format!(
                    r#"{{"uuid": "{}", "created_at": "2026-10-23T12:00:00Z", "must_support": ["{extension}"], "parties": []}}"#,
                    uuid(n)
                )
                .into_bytes()
            };
            assert_eq!(
                post(&s, &writer, None, needing(1, "known")).await.status(),
                201
            );
            let answer = post(&s, &writer, None, needing(2, "unknown")).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "unsupported-extension");
        }

        #[tokio::test]
        async fn a_media_type_that_is_not_text_is_unsupported() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (_, body) = vcon(UUID, "first");
            let answer = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(&writer)
                .header(
                    "content-type",
                    reqwest::header::HeaderValue::from_bytes(b"application/vcon\xff")
                        .expect("a header value"),
                )
                .body(body)
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 415);
        }

        #[tokio::test]
        async fn a_deleted_vcon_is_gone() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (found, body) = vcon(UUID, "first");
            assert_eq!(post(&s, &writer, None, body.clone()).await.status(), 201);
            s.tenants
                .store(s.tenant)
                .expect("its store")
                .delete(&store_caller(), found.uuid)
                .expect("deleted");
            let answer = post(&s, &writer, None, body).await;
            assert_eq!(answer.status(), 410);
            assert_eq!(problem(answer).await, "gone");
        }

        #[tokio::test]
        async fn the_vcon_media_types_json_and_none_are_taken() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let cases: [(Option<&str>, bool); 6] = [
                (None, false),
                (Some("application/vcon"), false),
                (Some("application/vcon+gzip"), true),
                (Some("application/json"), false),
                (Some("Application/VCON; charset=utf-8"), false),
                (Some("APPLICATION/VCON+GZIP ; q=1"), true),
            ];
            for (n, (content_type, gzipped)) in cases.into_iter().enumerate() {
                let (_, body) = vcon(&uuid(n as u8), "typed");
                let body = if gzipped { gzip(&body) } else { body };
                let answer = post(&s, &writer, content_type, body).await;
                assert_eq!(answer.status(), 201, "{content_type:?}");
            }
        }

        #[tokio::test]
        async fn another_media_type_is_unsupported_and_recorded() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (_, body) = vcon(UUID, "first");
            for content_type in ["text/plain", "application/vcon-json", "application/xml"] {
                let answer = post(&s, &writer, Some(content_type), body.clone()).await;
                assert_eq!(answer.status(), 415, "{content_type}");
                assert_eq!(problem(answer).await, "unsupported-media-type");
            }
            assert_eq!(s.tenant_log("create")[0]["status"], 415);
        }

        #[tokio::test]
        async fn a_body_format_refuses_is_unprocessable_and_recorded() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let answer = post(&s, &writer, None, b"not json".to_vec()).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "not-json");
            let (_, body) = vcon(UUID, "first");
            let answer = post(&s, &writer, Some("application/vcon+gzip"), body).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "bad-gzip");
            let refused = s.tenant_log("create");
            assert_eq!(refused.len(), 2);
            assert_eq!(refused[0]["status"], 422);
        }

        #[tokio::test]
        async fn a_body_that_inflates_past_its_limit_is_too_large() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let bomb = gzip(&vec![b' '; 2 << 20]);
            assert!(bomb.len() < 64 * 1024, "small on the wire");
            let answer = post(&s, &writer, Some("application/vcon+gzip"), bomb).await;
            assert_eq!(answer.status(), 413);
            assert_eq!(problem(answer).await, "too-large");
        }

        #[tokio::test]
        async fn a_create_whose_record_fails_is_unavailable() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let (_, body) = vcon(UUID, "first");
            let answer = post(&s, &writer, None, body).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }
    }
}
