//! The HTTP API, served over TLS only.

use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

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
}
