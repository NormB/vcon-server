//! The listener: connections accepted, taken through TLS within a time
//! limit, and served.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::middleware::{self, Next};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio_rustls::TlsAcceptor;

/// Where connections come from: a bound socket, or in a test one that
/// fails first.
pub trait Accept {
    /// The next connection, or why none could be taken.
    fn accept(&self) -> impl Future<Output = io::Result<TcpStream>> + Send;
}

impl Accept for TcpListener {
    async fn accept(&self) -> io::Result<TcpStream> {
        TcpListener::accept(self).await.map(|(tcp, _)| tcp)
    }
}

/// How long a connection may take over each step before it is closed, and
/// how long the listener waits after it could take no connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionLimits {
    /// The TLS handshake, from the connection's start to its end.
    pub handshake: Duration,
    /// The head of the connection's first request, counted from the end
    /// of the handshake, on HTTP/1.1 and HTTP/2.
    pub first_head: Duration,
    /// The head of each HTTP/1.1 request, which for a request after the
    /// first includes the wait for it on a connection kept open.
    pub head: Duration,
    /// The wait after the system refused to hand over a connection, such
    /// as when the process is out of file descriptors.
    pub accept_pause: Duration,
}

impl Default for ConnectionLimits {
    fn default() -> Self {
        Self {
            handshake: Duration::from_secs(10),
            first_head: Duration::from_secs(30),
            head: Duration::from_secs(30),
            accept_pause: Duration::from_secs(1),
        }
    }
}

/// What the listener does after it could take no connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterFailure {
    /// Take the next one at once: the failure was that one connection's.
    Next,
    /// Wait first: the failure is the system's and would repeat.
    Pause,
}

/// What follows a failed accept. A connection that ended before it was
/// taken is its own failure; anything else is the system's.
#[must_use]
pub fn after_failure(error: &io::Error) -> AfterFailure {
    match error.kind() {
        io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionRefused => AfterFailure::Next,
        _ => AfterFailure::Pause,
    }
}

/// Serves `app` on `listener` over TLS until `shutdown` completes, within
/// the default [`ConnectionLimits`]. A connection whose handshake fails,
/// such as plain HTTP, is dropped; the server goes on.
pub async fn serve(
    listener: impl Accept,
    tls: Arc<ServerConfig>,
    app: Router,
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    serve_within(listener, tls, app, shutdown, ConnectionLimits::default()).await
}

/// [`serve`], within `limits`. A failed accept never ends serving: the
/// listener goes on, after `limits.accept_pause` when the failure is the
/// system's ([`after_failure`]). A connection is closed when its
/// handshake is not done within `limits.handshake`, when no request head
/// has arrived within `limits.first_head` of the handshake, and when an
/// HTTP/1.1 request head takes longer than `limits.head`.
pub async fn serve_within(
    listener: impl Accept,
    tls: Arc<ServerConfig>,
    app: Router,
    shutdown: impl Future<Output = ()>,
    limits: ConnectionLimits,
) -> io::Result<()> {
    let acceptor = TlsAcceptor::from(tls);
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            () = &mut shutdown => return Ok(()),
        };
        let tcp = match accepted {
            Ok(tcp) => tcp,
            Err(error) => {
                if after_failure(&error) == AfterFailure::Pause {
                    // The error's text names no caller: none was taken.
                    tracing::warn!(%error, "no connection could be taken");
                    tokio::select! {
                        () = tokio::time::sleep(limits.accept_pause) => {}
                        () = &mut shutdown => return Ok(()),
                    }
                }
                continue;
            }
        };
        tokio::spawn(connection(tcp, acceptor.clone(), app.clone(), limits));
    }
}

/// One connection, from its handshake to its end. Its own errors end only
/// the connection.
async fn connection(tcp: TcpStream, acceptor: TlsAcceptor, app: Router, limits: ConnectionLimits) {
    let Ok(Ok(stream)) = tokio::time::timeout(limits.handshake, acceptor.accept(tcp)).await else {
        return;
    };
    // Told when the connection's first request head has arrived.
    let first = Arc::new(Notify::new());
    let seen = first.clone();
    let app = Router::new()
        .fallback_service(app)
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            seen.notify_one();
            next.run(request)
        }));
    let mut builder = auto::Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.head);
    let served = builder.serve_connection(TokioIo::new(stream), TowerToHyperService::new(app));
    let mut served = std::pin::pin!(served);
    tokio::select! {
        _ = &mut served => return,
        arrived = tokio::time::timeout(limits.first_head, first.notified()) => {
            if arrived.is_err() {
                return;
            }
        }
    }
    let _ = served.await;
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::net::SocketAddr;
    use std::sync::Mutex;
    use std::time::Instant;

    use axum::routing::get;
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::client::TlsStream;

    use super::*;

    /// A listener that fails with each of `failures` in turn before it
    /// takes connections from its socket.
    struct Failing {
        socket: TcpListener,
        failures: Mutex<VecDeque<io::Error>>,
    }

    impl Accept for Failing {
        async fn accept(&self) -> io::Result<TcpStream> {
            let failure = self.failures.lock().expect("the failures").pop_front();
            match failure {
                Some(failure) => Err(failure),
                None => Accept::accept(&self.socket).await,
            }
        }
    }

    /// Limits no test waits out, so only the one a test sets is felt.
    const NEVER: ConnectionLimits = ConnectionLimits {
        handshake: Duration::from_secs(3600),
        first_head: Duration::from_secs(3600),
        head: Duration::from_secs(3600),
        accept_pause: Duration::from_secs(3600),
    };

    /// The longest a test waits for the server to do what it should.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// A short limit: long enough to be told from no wait at all.
    const SHORT: Duration = Duration::from_millis(300);

    struct Running {
        addr: SocketAddr,
        pem: String,
        stop: Option<tokio::sync::oneshot::Sender<()>>,
        server: tokio::task::JoinHandle<io::Result<()>>,
    }

    impl Running {
        /// Stops the server and gives what `serve_within` returned.
        async fn stopped(mut self) -> io::Result<()> {
            self.stop.take().expect("not yet stopped").send(()).ok();
            tokio::time::timeout(PATIENCE, self.server)
                .await
                .expect("the server stops when asked")
                .expect("the server's task joins")
        }

        fn client(&self) -> reqwest::Client {
            reqwest::Client::builder()
                .tls_certs_only([reqwest::Certificate::from_pem(self.pem.as_bytes()).expect("pem")])
                .resolve("localhost", self.addr)
                .no_proxy()
                .build()
                .expect("a client")
        }

        /// The status and body of `GET /ping` from a new client.
        async fn ping(&self) -> (u16, String) {
            let answer = tokio::time::timeout(
                PATIENCE,
                self.client()
                    .get(format!("https://localhost:{}/ping", self.addr.port()))
                    .send(),
            )
            .await
            .expect("an answer in time")
            .expect("an answer");
            (
                answer.status().as_u16(),
                answer.text().await.expect("a body"),
            )
        }

        async fn tcp(&self) -> TcpStream {
            TcpStream::connect(self.addr).await.expect("connects")
        }

        /// A connection taken through the TLS handshake, offering `alpn`,
        /// or no protocol when it is empty.
        async fn tls(&self, alpn: &[u8]) -> TlsStream<TcpStream> {
            let mut roots = rustls::RootCertStore::empty();
            for cert in CertificateDer::pem_slice_iter(self.pem.as_bytes()) {
                roots.add(cert.expect("pem")).expect("a root");
            }
            let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
            let mut config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            if !alpn.is_empty() {
                config.alpn_protocols = vec![alpn.to_vec()];
            }
            tokio_rustls::TlsConnector::from(Arc::new(config))
                .connect("localhost".try_into().expect("name"), self.tcp().await)
                .await
                .expect("handshake")
        }
    }

    /// A server answering `GET /ping` within `limits`, whose listener
    /// fails with `failures` first.
    async fn running(limits: ConnectionLimits, failures: Vec<io::Error>) -> Running {
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate");
        let dir = tempfile::tempdir().expect("temp dir");
        let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        let pem = made.cert.pem();
        std::fs::write(&cert, &pem).expect("write");
        std::fs::write(&key, made.signing_key.serialize_pem()).expect("write");
        std::fs::set_permissions(
            &key,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("chmod 0600");
        let tls = crate::api::tls_config(&cert, &key).expect("loads");
        let socket = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = socket.local_addr().expect("addr");
        let listener = Failing {
            socket,
            failures: Mutex::new(failures.into()),
        };
        let app = Router::new()
            .route("/ping", get(|| async { "pong" }))
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(SHORT * 3).await;
                    "done"
                }),
            );
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_within(
            listener,
            tls,
            app,
            async {
                stopped.await.ok();
            },
            limits,
        ));
        Running {
            addr,
            pem,
            stop: Some(stop),
            server,
        }
    }

    /// The error the system gives a process that is out of file
    /// descriptors (EMFILE).
    fn out_of_descriptors() -> io::Error {
        io::Error::from_raw_os_error(24)
    }

    /// How long until the server closes `stream`, reading what it sends
    /// until then. Panics when it is still open after [`PATIENCE`].
    async fn closed_after(stream: &mut (impl AsyncReadExt + Unpin)) -> Duration {
        let started = Instant::now();
        let mut sink = [0; 1024];
        tokio::time::timeout(PATIENCE, async {
            // An error is the connection reset: closed as well.
            while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
        })
        .await
        .expect("the server closes the connection");
        started.elapsed()
    }

    /// Whether the server still holds `stream` open after `wait`.
    async fn open_after(stream: &mut (impl AsyncReadExt + Unpin), wait: Duration) -> bool {
        let mut sink = [0; 1024];
        tokio::time::timeout(wait, async {
            while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
        })
        .await
        .is_err()
    }

    // The listener after a failed accept.

    #[tokio::test]
    async fn the_server_goes_on_after_it_is_out_of_file_descriptors() {
        let limits = ConnectionLimits {
            accept_pause: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![out_of_descriptors()]).await;
        assert_eq!(
            s.ping().await,
            (200, "pong".to_owned()),
            "served after the failure"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn the_server_goes_on_after_several_failures_in_a_row() {
        let limits = ConnectionLimits {
            accept_pause: Duration::from_millis(50),
            ..NEVER
        };
        let failures = vec![
            out_of_descriptors(),
            io::Error::from_raw_os_error(23),
            io::Error::other("the system's"),
            out_of_descriptors(),
        ];
        let s = running(limits, failures).await;
        assert_eq!(s.ping().await.0, 200, "served after four failures in a row");
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_connection_gone_before_it_was_taken_costs_no_wait() {
        // The pause is an hour: an answer at all shows it was not taken.
        let failures = vec![
            io::Error::from(io::ErrorKind::ConnectionAborted),
            io::Error::from(io::ErrorKind::ConnectionReset),
            io::Error::from(io::ErrorKind::ConnectionRefused),
        ];
        let s = running(NEVER, failures).await;
        assert_eq!(s.ping().await.0, 200, "served without the pause");
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn the_listener_waits_after_the_system_refused_a_connection() {
        let limits = ConnectionLimits {
            accept_pause: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![out_of_descriptors()]).await;
        let started = Instant::now();
        assert_eq!(s.ping().await.0, 200, "served after the pause");
        assert!(
            started.elapsed() >= SHORT - Duration::from_millis(50),
            "the pause was taken: {:?}",
            started.elapsed()
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_stop_during_the_pause_ends_serving_at_once() {
        let s = running(NEVER, vec![out_of_descriptors()]).await;
        // Time for the failure to be met and the hour's pause to start.
        tokio::time::sleep(SHORT).await;
        let started = Instant::now();
        s.stopped().await.expect("serving ended without an error");
        assert!(
            started.elapsed() < PATIENCE,
            "the stop did not wait for the pause"
        );
    }

    #[tokio::test]
    async fn a_failure_is_not_returned_as_the_servers_end() {
        let limits = ConnectionLimits {
            accept_pause: Duration::from_millis(10),
            ..NEVER
        };
        let s = running(limits, vec![out_of_descriptors()]).await;
        tokio::time::sleep(SHORT).await;
        assert!(
            !s.server.is_finished(),
            "the server is still serving after the failure"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[test]
    fn only_a_connections_own_end_is_passed_over_without_a_wait() {
        for kind in [
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionRefused,
        ] {
            assert_eq!(
                after_failure(&io::Error::from(kind)),
                AfterFailure::Next,
                "{kind:?} is the connection's own"
            );
        }
        for error in [
            out_of_descriptors(),
            io::Error::from_raw_os_error(23),
            io::Error::from_raw_os_error(105),
            io::Error::from_raw_os_error(12),
            io::Error::other("unknown"),
            io::Error::from(io::ErrorKind::PermissionDenied),
            io::Error::from(io::ErrorKind::TimedOut),
        ] {
            assert_eq!(
                after_failure(&error),
                AfterFailure::Pause,
                "{error:?} is the system's"
            );
        }
    }

    // A connection that stalls.

    #[tokio::test]
    async fn a_connection_that_sends_nothing_is_closed_at_the_handshake_limit() {
        let limits = ConnectionLimits {
            handshake: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut silent = s.tcp().await;
        let after = closed_after(&mut silent).await;
        assert!(
            after >= SHORT - Duration::from_millis(50),
            "not before the limit: {after:?}"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_handshake_begun_and_left_is_closed_at_the_handshake_limit() {
        let limits = ConnectionLimits {
            handshake: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut begun = s.tcp().await;
        // The first five bytes of a TLS record: a handshake of 512 bytes
        // that never come.
        begun
            .write_all(&[0x16, 0x03, 0x01, 0x02, 0x00])
            .await
            .expect("sent");
        closed_after(&mut begun).await;
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn many_silent_connections_are_all_closed_and_the_next_is_served() {
        let limits = ConnectionLimits {
            handshake: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut silent = Vec::new();
        for _ in 0..20 {
            silent.push(s.tcp().await);
        }
        for stream in &mut silent {
            closed_after(stream).await;
        }
        assert_eq!(s.ping().await.0, 200, "served after them");
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_silent_connection_does_not_delay_another() {
        let s = running(NEVER, vec![]).await;
        let _silent = s.tcp().await;
        assert_eq!(s.ping().await.0, 200, "served beside a silent connection");
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_handshake_within_the_limit_is_served() {
        let limits = ConnectionLimits {
            handshake: Duration::from_secs(10),
            first_head: Duration::from_secs(10),
            head: Duration::from_secs(10),
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut slow = s.tcp().await;
        tokio::time::sleep(SHORT).await;
        // Plain HTTP where TLS is due: refused, which shows the server
        // was still reading after the wait.
        slow.write_all(b"GET /ping HTTP/1.1\r\n\r\n")
            .await
            .expect("sent");
        closed_after(&mut slow).await;
        assert_eq!(s.ping().await.0, 200, "and TLS is served");
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_connection_silent_after_the_handshake_is_closed_at_the_head_limit() {
        for alpn in [&b"http/1.1"[..], &b"h2"[..]] {
            let limits = ConnectionLimits {
                first_head: SHORT,
                ..NEVER
            };
            let s = running(limits, vec![]).await;
            let mut silent = s.tls(alpn).await;
            let after = closed_after(&mut silent).await;
            assert!(
                after >= SHORT - Duration::from_millis(50),
                "not before the limit: {after:?}"
            );
            s.stopped().await.expect("serving ended without an error");
        }
    }

    #[tokio::test]
    async fn a_connection_that_offers_no_protocol_and_sends_nothing_is_closed() {
        let limits = ConnectionLimits {
            first_head: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut silent = s.tls(b"").await;
        closed_after(&mut silent).await;
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_request_head_begun_and_left_is_closed_at_the_head_limit() {
        let limits = ConnectionLimits {
            head: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut begun = s.tls(b"http/1.1").await;
        begun
            .write_all(b"GET /ping HTTP/1.1\r\nhost: localhost\r\n")
            .await
            .expect("sent");
        let mut answer = Vec::new();
        tokio::time::timeout(PATIENCE, begun.read_to_end(&mut answer))
            .await
            .expect("closed")
            .ok();
        assert!(
            !String::from_utf8_lossy(&answer).contains("200 OK"),
            "an unfinished head is not served"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_head_sent_a_byte_at_a_time_past_the_limit_is_closed() {
        let limits = ConnectionLimits {
            head: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut slow = s.tls(b"http/1.1").await;
        let started = Instant::now();
        let mut refused = false;
        // A byte each 50 ms never ends the head; the limit is on the whole
        // head, not on the gap between bytes.
        for byte in b"GET /ping HTTP/1.1\r\nx-slow: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" {
            if slow.write_all(&[*byte]).await.is_err() || slow.flush().await.is_err() {
                refused = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !refused {
            closed_after(&mut slow).await;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "closed while the head was still arriving"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_request_within_the_limits_is_served_over_each_protocol() {
        let limits = ConnectionLimits {
            handshake: Duration::from_secs(10),
            first_head: Duration::from_secs(10),
            head: Duration::from_secs(10),
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut one = s.tls(b"http/1.1").await;
        tokio::time::sleep(SHORT).await;
        one.write_all(b"GET /ping HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n")
            .await
            .expect("sent");
        let mut answer = Vec::new();
        tokio::time::timeout(PATIENCE, one.read_to_end(&mut answer))
            .await
            .expect("answered")
            .ok();
        let answer = String::from_utf8_lossy(&answer);
        assert!(answer.starts_with("HTTP/1.1 200 OK"), "HTTP/1.1: {answer}");
        assert!(answer.ends_with("pong"), "the body: {answer}");
        let two = s
            .client()
            .get(format!("https://localhost:{}/ping", s.addr.port()))
            .send()
            .await
            .expect("an answer");
        assert_eq!(two.version(), reqwest::Version::HTTP_2, "h2 by ALPN");
        assert_eq!(two.text().await.expect("a body"), "pong", "over h2");
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_connection_stays_open_until_its_limit() {
        let limits = ConnectionLimits {
            handshake: Duration::from_secs(3600),
            first_head: Duration::from_secs(3600),
            head: Duration::from_secs(3600),
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut plain = s.tcp().await;
        assert!(
            open_after(&mut plain, SHORT).await,
            "a connection inside the handshake limit is kept"
        );
        let mut secured = s.tls(b"http/1.1").await;
        assert!(
            open_after(&mut secured, SHORT).await,
            "a connection inside the head limit is kept"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn two_requests_on_one_connection_are_both_served() {
        let limits = ConnectionLimits {
            first_head: Duration::from_secs(10),
            head: Duration::from_secs(10),
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let client = s.client();
        for n in 0..2 {
            let answer = client
                .get(format!("https://localhost:{}/ping", s.addr.port()))
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 200, "request {n}");
        }
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_connection_idle_after_its_answer_is_closed_at_the_head_limit() {
        let limits = ConnectionLimits {
            head: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut kept = s.tls(b"http/1.1").await;
        kept.write_all(b"GET /ping HTTP/1.1\r\nhost: localhost\r\n\r\n")
            .await
            .expect("sent");
        let mut answer = Vec::new();
        tokio::time::timeout(PATIENCE, kept.read_to_end(&mut answer))
            .await
            .expect("closed once idle past the limit")
            .ok();
        let answer = String::from_utf8_lossy(&answer);
        assert!(
            answer.starts_with("HTTP/1.1 200 OK"),
            "answered first: {answer}"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn a_head_that_arrives_in_two_parts_within_the_limit_is_served() {
        let limits = ConnectionLimits {
            first_head: Duration::from_secs(10),
            head: Duration::from_secs(10),
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut split = s.tls(b"http/1.1").await;
        split.write_all(b"G").await.expect("sent");
        split.flush().await.expect("flushed");
        tokio::time::sleep(SHORT).await;
        split
            .write_all(b"ET /ping HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n")
            .await
            .expect("sent");
        let mut answer = Vec::new();
        tokio::time::timeout(PATIENCE, split.read_to_end(&mut answer))
            .await
            .expect("answered")
            .ok();
        let answer = String::from_utf8_lossy(&answer);
        assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn an_answer_that_takes_longer_than_the_head_limit_is_still_given() {
        // The limit is on the head's arrival, not on the request's work.
        for version in [reqwest::Version::HTTP_11, reqwest::Version::HTTP_2] {
            let limits = ConnectionLimits {
                first_head: SHORT,
                head: SHORT,
                ..NEVER
            };
            let s = running(limits, vec![]).await;
            let client = if version == reqwest::Version::HTTP_11 {
                reqwest::Client::builder().http1_only()
            } else {
                reqwest::Client::builder()
            }
            .tls_certs_only([reqwest::Certificate::from_pem(s.pem.as_bytes()).expect("pem")])
            .resolve("localhost", s.addr)
            .no_proxy()
            .build()
            .expect("a client");
            let answer = tokio::time::timeout(
                PATIENCE,
                client
                    .get(format!("https://localhost:{}/slow", s.addr.port()))
                    .send(),
            )
            .await
            .expect("an answer in time")
            .expect("the connection was kept while the request ran");
            assert_eq!(answer.version(), version, "the protocol asked for");
            assert_eq!(answer.text().await.expect("a body"), "done", "{version:?}");
            s.stopped().await.expect("serving ended without an error");
        }
    }

    #[tokio::test]
    async fn the_first_head_limit_alone_does_not_close_a_connection_idle_after_its_answer() {
        let limits = ConnectionLimits {
            first_head: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut kept = s.tls(b"http/1.1").await;
        kept.write_all(b"GET /ping HTTP/1.1\r\nhost: localhost\r\n\r\n")
            .await
            .expect("sent");
        let mut answer = [0; 1024];
        let read = tokio::time::timeout(PATIENCE, kept.read(&mut answer))
            .await
            .expect("answered")
            .expect("read");
        assert!(
            String::from_utf8_lossy(&answer[..read]).starts_with("HTTP/1.1 200 OK"),
            "answered first"
        );
        assert!(
            open_after(&mut kept, SHORT * 3).await,
            "kept: the limit on later heads is an hour"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[tokio::test]
    async fn the_later_head_limit_alone_does_not_close_an_http2_connection_that_sends_nothing() {
        let limits = ConnectionLimits {
            head: SHORT,
            ..NEVER
        };
        let s = running(limits, vec![]).await;
        let mut silent = s.tls(b"h2").await;
        assert!(
            open_after(&mut silent, SHORT * 3).await,
            "kept: the limit on the first head is an hour"
        );
        s.stopped().await.expect("serving ended without an error");
    }

    #[test]
    fn the_default_limits_are_ten_seconds_thirty_seconds_and_one_second() {
        assert_eq!(
            ConnectionLimits::default(),
            ConnectionLimits {
                handshake: Duration::from_secs(10),
                first_head: Duration::from_secs(30),
                head: Duration::from_secs(30),
                accept_pause: Duration::from_secs(1),
            },
            "the limits serve applies"
        );
    }
}
