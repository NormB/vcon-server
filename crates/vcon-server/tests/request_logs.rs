//! Each request is logged by its route template, with no id in the log.
//! tracing decides per call site whether anyone listens, process-wide, so
//! this test is alone in its binary and installs a global subscriber
//! before anything logs.

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vcon_server::access::{Scope, Scopes};
use vcon_server::api::{App, Limits, router, serve, tls_config};
use vcon_server::audit::Actor;
use vcon_server::clock::SystemClock;
use vcon_server::envelope::Key;
use vcon_server::fs::OsFs;
use vcon_server::tenants::Tenants;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

/// Collects what the log writes.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn each_request_is_logged_by_its_route_template_and_no_id() {
    let capture = Capture::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .finish(),
    )
    .expect("the only subscriber");

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
    let key = tenants
        .create_key(
            Actor::Cli,
            Some(tenant),
            Scopes::of(&[Scope::Write, Scope::Read, Scope::Delete]),
            "test",
        )
        .expect("a key");
    let app = Arc::new(App {
        tenants: tenants.clone(),
        limits: Limits {
            body: 1 << 20,
            inflated: 1 << 20,
            body_timeout: Duration::from_secs(5),
        },
        supported: BTreeSet::new(),
    });

    let made =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate");
    let (cert, pem) = (dir.path().join("cert.pem"), made.cert.pem());
    let key_file = dir.path().join("key.pem");
    std::fs::write(&cert, &pem).expect("write");
    std::fs::write(&key_file, made.signing_key.serialize_pem()).expect("write");
    let tls = tls_config(&cert, &key_file).expect("tls");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, tls, router(app), async {
        stopped.await.ok();
    }));

    let client = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(pem.as_bytes()).expect("pem")])
        .resolve("localhost", addr)
        .no_proxy()
        .build()
        .expect("a client");
    let url = |path: &str| format!("https://localhost:{}{path}", addr.port());
    let token = key.token.clone();
    let body = std::fs::read(format!(
        "{}/../vcon-form/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4");
    let requests = [
        client
            .post(url("/vcons"))
            .bearer_auth(&token)
            .body(body.clone()),
        client
            .get(url(&format!("/vcons/{UUID}")))
            .bearer_auth(&token),
        client
            .put(url(&format!("/vcons/{UUID}")))
            .bearer_auth(&token)
            .header("if-match", "garbage")
            .body(body),
        client
            .delete(url(&format!("/vcons/{UUID}")))
            .bearer_auth(&token),
        client.get(url("/health")).bearer_auth(&token),
        client
            .get(url(&format!("/nowhere/{UUID}")))
            .bearer_auth(&token),
    ];
    let mut statuses = Vec::new();
    for request in requests {
        statuses.push(request.send().await.expect("an answer").status().as_u16());
    }
    assert_eq!(statuses, [201, 200, 412, 204, 200, 404]);
    stop.send(()).expect("stop");
    server.await.expect("joins").expect("serves");

    let logged = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("UTF-8");
    let lines: Vec<&str> = logged
        .lines()
        .filter(|l| l.contains("request_id="))
        .collect();
    let expected = [
        ("POST", "/vcons", 201),
        ("GET", "/vcons/{uuid}", 200),
        ("PUT", "/vcons/{uuid}", 412),
        ("DELETE", "/vcons/{uuid}", 204),
        ("GET", "/health", 200),
        ("GET", "unmatched", 404),
    ];
    assert_eq!(lines.len(), expected.len(), "{logged}");
    for (line, (method, route, status)) in lines.iter().zip(expected) {
        assert!(line.contains(&format!("method={method}")), "{line}");
        assert!(line.contains(&format!("route={route} ")), "{line}");
        assert!(line.contains(&format!("status={status}")), "{line}");
        assert!(line.contains("duration_ms="), "{line}");
    }
    for secret in [
        token.as_str(),
        &token[21..],
        UUID,
        &tenant.to_string(),
        &key.id.to_string(),
    ] {
        assert!(!logged.contains(secret), "{secret} is in the log");
    }
}
