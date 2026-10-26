//! The MCP server over streamable HTTP, served in process on a free port
//! with a test certificate and driven by a real HTTPS client.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::access::{Scope, Scopes};
use crate::api::{self, App};
use crate::audit::Actor;
use crate::clock::ManualClock;
use crate::envelope::Key;
use crate::fs::OsFs;
use crate::ids::TenantId;
use crate::retention::Retention;
use crate::tenants::Tenants;

/// The largest body the test server takes.
const LIMIT: usize = 64 * 1024;

/// A server with one tenant, on a free port; stopped when dropped.
pub(super) struct Server {
    _dir: tempfile::TempDir,
    pem: String,
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
    serving(api::router, crate::mcp::Limits::DEFAULT).await
}

/// A server whose routes `routes` makes from its App, with MCP `limits`.
/// Its clock stands still, so every counted request falls in one minute.
pub(super) async fn serving(
    routes: fn(Arc<App>) -> axum::Router,
    limits: crate::mcp::Limits,
) -> Server {
    let dir = tempfile::tempdir().expect("temp dir");
    let tenants = Arc::new(
        Tenants::open(
            dir.path(),
            Key::generate().expect("master key"),
            Arc::new(OsFs),
            Arc::new(ManualClock::at(
                time::macros::datetime!(2026-10-25 12:00 UTC),
            )),
        )
        .expect("opens"),
    );
    let tenant = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let app = Arc::new(App {
        tenants: tenants.clone(),
        limits: api::Limits {
            body: LIMIT,
            inflated: 1 << 20,
            body_timeout: Duration::from_millis(300),
        },
        supported: Default::default(),
        retention_default: Retention::Years(7),
        mcp: limits,
    });
    let made =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate");
    let pem = made.cert.pem();
    let (cert, key): (PathBuf, PathBuf) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    std::fs::write(&cert, &pem).expect("write");
    std::fs::write(&key, made.signing_key.serialize_pem()).expect("write");
    let tls = api::tls_config(&cert, &key).expect("tls");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(api::serve(listener, tls, routes(app), async {
        stopped.await.ok();
    }));
    Server {
        _dir: dir,
        pem,
        addr,
        tenants,
        tenant,
        stop: Some(stop),
    }
}

/// What the server answered to one POST.
pub(super) struct Answer {
    pub(super) status: u16,
    pub(super) headers: reqwest::header::HeaderMap,
    pub(super) body: Value,
}

impl Server {
    pub(super) fn url(&self) -> String {
        format!("https://localhost:{}/mcp", self.addr.port())
    }

    pub(super) fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(self.pem.as_bytes()).expect("pem")])
            .resolve("localhost", self.addr)
            .no_proxy()
            .build()
            .expect("a client")
    }

    /// A tenant key holding `scopes`.
    pub(super) fn key(&self, scopes: &[Scope]) -> String {
        self.tenants
            .create_key(Actor::Cli, Some(self.tenant), Scopes::of(scopes), "test")
            .expect("a key")
            .token
    }

    /// An operator key.
    pub(super) fn operator(&self) -> String {
        self.tenants
            .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "operator")
            .expect("a key")
            .token
    }

    /// Sends one JSON-RPC message with `token`, if any, and the headers
    /// given.
    pub(super) async fn post(
        &self,
        token: Option<&str>,
        message: &Value,
        headers: &[(&str, &str)],
    ) -> Answer {
        let mut request = self
            .client()
            .post(self.url())
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(message.to_string());
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let answer = request.send().await.expect("an answer");
        let (status, headers) = (answer.status().as_u16(), answer.headers().clone());
        let text = answer.text().await.expect("a body");
        Answer {
            status,
            headers,
            body: serde_json::from_str(&text).unwrap_or(Value::String(text)),
        }
    }

    /// The tenant log's records of `action`.
    pub(super) fn tenant_log(&self, action: &str) -> Vec<Value> {
        let store = self.tenants.store(self.tenant).expect("its store");
        store
            .records()
            .into_iter()
            .filter(|r| r["action"] == action)
            .collect()
    }

    /// The admin log's records of `action`.
    pub(super) fn admin_log(&self, action: &str) -> Vec<Value> {
        self.tenants
            .admin_records()
            .into_iter()
            .filter(|r| r["action"] == action)
            .collect()
    }
}

/// An `initialize` request asking for `version`.
pub(super) fn initialize(version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"},
        },
    })
}

/// A `tools/list` request.
pub(super) fn list_tools() -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
}

#[tokio::test]
async fn the_handshake_names_vcon_server_and_offers_tools_only() {
    let s = server().await;
    let token = s.key(&[Scope::Read]);
    for version in ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"] {
        let a = s.post(Some(&token), &initialize(version), &[]).await;
        assert_eq!(a.status, 200, "{version}: {:?}", a.body);
        let result = &a.body["result"];
        assert_eq!(result["protocolVersion"], version);
        assert_eq!(result["serverInfo"]["name"], "vcon-server");
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
        let capabilities = result["capabilities"].as_object().expect("capabilities");
        assert!(capabilities.contains_key("tools"), "{capabilities:?}");
        for none in ["resources", "prompts", "completions", "logging"] {
            assert!(!capabilities.contains_key(none), "{none}: {capabilities:?}");
        }
        let instructions = result["instructions"].as_str().expect("instructions");
        assert!(instructions.contains("tenant"), "{instructions}");
    }
}

#[tokio::test]
async fn server_discover_names_vcon_server() {
    let s = server().await;
    let token = s.operator();
    let discover = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/discover",
        "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "1.0"},
            "io.modelcontextprotocol/clientCapabilities": {},
        }},
    });
    let a = s
        .post(
            Some(&token),
            &discover,
            &[
                ("mcp-protocol-version", "2026-07-28"),
                ("mcp-method", "server/discover"),
            ],
        )
        .await;
    assert_eq!(a.status, 200, "{:?}", a.body);
    // From 2026-07-28 the server's identity travels in the result's _meta.
    let info = a.body["result"]["_meta"]["io.modelcontextprotocol/serverInfo"].clone();
    assert_eq!(info["name"], "vcon-server", "{:?}", a.body);
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(a.body["result"]["capabilities"], json!({"tools": {}}));
}

#[tokio::test]
async fn no_key_or_a_bad_key_gets_401_with_a_bearer_challenge() {
    let s = server().await;
    let revoked = s.key(&[Scope::Read]);
    let id = crate::access::parse(&revoked).expect("a token");
    s.tenants.revoke_key(Actor::Cli, id).expect("revoked");
    let malformed = "vcs_nothing";
    for token in [None, Some(malformed), Some(revoked.as_str())] {
        let a = s.post(token, &initialize("2025-11-25"), &[]).await;
        assert_eq!(a.status, 401, "{token:?}: {:?}", a.body);
        assert_eq!(
            a.headers.get("www-authenticate").map(|v| v.to_str().ok()),
            Some(Some("Bearer")),
            "{token:?}"
        );
        assert_eq!(
            a.body["type"], "https://vcon-server.com/problems/unauthorized",
            "{token:?}"
        );
    }
    // Counted, as every failed authentication is, never recorded one row
    // each.
    let failed: u64 = s
        .tenant_log("auth_failed")
        .iter()
        .chain(s.admin_log("auth_failed").iter())
        .map(|r| r["count"].as_u64().expect("count"))
        .sum();
    assert_eq!(failed, 3);
}

#[tokio::test]
async fn there_are_no_tools_yet() {
    let s = server().await;
    let token = s.key(&[Scope::Read, Scope::Write]);
    let a = s.post(Some(&token), &list_tools(), &[]).await;
    assert_eq!(a.status, 200, "{:?}", a.body);
    assert_eq!(a.body["result"]["tools"], json!([]));
}

#[tokio::test]
async fn each_request_is_counted_in_the_keys_log() {
    let s = server().await;
    let token = s.key(&[Scope::Read]);
    for _ in 0..3 {
        assert_eq!(s.post(Some(&token), &list_tools(), &[]).await.status, 200);
    }
    let counted = s.tenant_log("mcp");
    assert_eq!(counted.len(), 1, "one record a minute: {counted:?}");
    assert_eq!(counted[0]["count"], 3);
    assert_eq!(counted[0]["status"], 200);
    assert!(s.admin_log("mcp").is_empty());

    let operator = s.operator();
    assert_eq!(
        s.post(Some(&operator), &list_tools(), &[]).await.status,
        200
    );
    let counted = s.admin_log("mcp");
    assert_eq!(counted.len(), 1, "{counted:?}");
    assert_eq!(counted[0]["count"], 1);
}

#[tokio::test]
async fn a_request_that_cannot_be_counted_answers_audit_failed() {
    let s = server().await;
    let token = s.key(&[Scope::Read]);
    s.tenants.store(s.tenant).expect("store").break_audit();
    let a = s.post(Some(&token), &list_tools(), &[]).await;
    assert_eq!(a.status, 503, "{:?}", a.body);
    assert_eq!(
        a.body["type"],
        "https://vcon-server.com/problems/audit-failed"
    );
}

#[tokio::test]
async fn a_tenant_being_removed_answers_503_counted_in_the_admin_log() {
    let s = server().await;
    let token = s.key(&[Scope::Read]);
    s.tenants.close(s.tenant);
    let a = s.post(Some(&token), &list_tools(), &[]).await;
    assert_eq!(a.status, 503, "{:?}", a.body);
    assert_eq!(
        a.body["type"],
        "https://vcon-server.com/problems/unavailable"
    );
    let counted = s.admin_log("mcp");
    assert_eq!(counted.len(), 1, "{counted:?}");
    assert_eq!(counted[0]["status"], 503);
}

#[tokio::test]
async fn a_request_from_a_web_page_is_refused_and_any_host_is_taken() {
    let s = server().await;
    let token = s.key(&[Scope::Read]);
    let a = s
        .post(
            Some(&token),
            &list_tools(),
            &[("origin", "https://page.example.com")],
        )
        .await;
    assert_eq!(a.status, 403, "{:?}", a.body);
    let a = s
        .post(Some(&token), &list_tools(), &[("host", "vcon.example.com")])
        .await;
    assert_eq!(a.status, 200, "a deployment's own host name: {:?}", a.body);
}

#[tokio::test]
async fn there_is_no_session_or_event_stream() {
    let s = server().await;
    let token = s.key(&[Scope::Read]);
    let a = s.post(Some(&token), &initialize("2025-11-25"), &[]).await;
    assert!(a.headers.get("mcp-session-id").is_none(), "{:?}", a.headers);
    assert_eq!(
        a.headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.starts_with("application/json")),
        Some(true)
    );
    for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
        let answer = s
            .client()
            .request(method.clone(), s.url())
            .header("authorization", format!("Bearer {token}"))
            .header("accept", "text/event-stream")
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 405, "{method}");
    }
}

#[tokio::test]
async fn a_body_over_the_limit_gets_413() {
    let s = server().await;
    let token = s.key(&[Scope::Read]);
    let mut message = list_tools();
    message["params"]["padding"] = Value::String("x".repeat(LIMIT));
    let a = s.post(Some(&token), &message, &[]).await;
    assert_eq!(a.status, 413, "{:?}", a.body);
    assert_eq!(a.body["type"], "https://vcon-server.com/problems/too-large");
    let counted = s.tenant_log("mcp");
    assert_eq!(counted.len(), 1, "{counted:?}");
    assert_eq!(counted[0]["status"], 413, "counted with its own status");
}

/// The dispatch gate, driven through test tools registered as real ones
/// are.
mod gate {
    use std::sync::OnceLock;

    use rmcp::model::JsonObject;
    use tokio::sync::Notify;

    use super::*;
    use crate::mcp::{self, Hints, Spec};

    #[derive(serde::Deserialize, rmcp::schemars::JsonSchema)]
    #[schemars(crate = "rmcp::schemars")]
    #[serde(deny_unknown_fields)]
    struct NoArgs {}

    /// Released by the capacity test to let `vcon_wait` return.
    fn release() -> &'static Notify {
        static RELEASE: OnceLock<Notify> = OnceLock::new();
        RELEASE.get_or_init(Notify::new)
    }

    fn probe<'a>(
        _: &'a mcp::Server,
        _: crate::access::Caller,
        args: JsonObject,
    ) -> mcp::Running<'a> {
        Box::pin(async move {
            let _: NoArgs = mcp::arguments(args)?;
            Ok(json!({"probed": true}))
        })
    }

    fn wait<'a>(_: &'a mcp::Server, _: crate::access::Caller, _: JsonObject) -> mcp::Running<'a> {
        Box::pin(async move {
            release().notified().await;
            Ok(json!({"waited": true}))
        })
    }

    fn spec(name: &'static str, hints: Option<Hints>, action: Action, run: mcp::Run) -> Spec {
        let mut tool = mcp::tool::<NoArgs>(name, "A test tool.", hints.unwrap_or(Hints::NONE));
        if hints.is_none() {
            tool.annotations = None;
        }
        Spec { tool, action, run }
    }

    use crate::audit::Action;

    fn specs() -> Vec<Spec> {
        let read = Hints {
            read_only: true,
            ..Hints::NONE
        };
        vec![
            spec("vcon_probe", Some(read), Action::Read, probe),
            spec("vcon_wait", Some(read), Action::Read, wait),
            spec("vcon_change", Some(Hints::NONE), Action::Replace, probe),
            spec(
                "vcon_erase",
                Some(Hints {
                    destructive: true,
                    ..Hints::NONE
                }),
                Action::Delete,
                probe,
            ),
            spec("legal_probe", Some(read), Action::LegalRead, probe),
            spec("admin_probe", Some(read), Action::TenantList, probe),
            spec("vcon_unhinted", None, Action::Read, probe),
        ]
    }

    /// A server on a free port serving only `/mcp`, with the test tools and
    /// `limits`.
    async fn gate_server(limits: mcp::Limits) -> Server {
        serving(|app| mcp::router_with(app, specs()), limits).await
    }

    fn call(name: &str, arguments: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        })
    }

    /// The JSON-RPC error's code and the problem document in its data.
    fn refusal(a: &Answer) -> (i64, Value) {
        let error = &a.body["error"];
        assert!(error.is_object(), "a refusal: {:?}", a.body);
        (
            error["code"].as_i64().expect("a code"),
            error["data"].clone(),
        )
    }

    fn problem(name: &str) -> String {
        format!("https://vcon-server.com/problems/{name}")
    }

    /// The tenant log's records of `action` with `status`.
    fn logged(s: &Server, action: &str, status: u16) -> Vec<Value> {
        s.tenant_log(action)
            .into_iter()
            .filter(|r| r["status"] == status)
            .collect()
    }

    #[tokio::test]
    async fn a_call_within_every_bound_runs_and_the_gate_records_nothing() {
        let s = gate_server(mcp::Limits::DEFAULT).await;
        let token = s.key(&[Scope::Read]);
        let a = s
            .post(Some(&token), &call("vcon_probe", json!({})), &[])
            .await;
        assert_eq!(a.status, 200, "{:?}", a.body);
        assert_eq!(
            a.body["result"]["structuredContent"],
            json!({"probed": true})
        );
        assert!(a.body["result"]["isError"].is_null() || a.body["result"]["isError"] == false);
        assert!(
            s.tenant_log("read").is_empty(),
            "the tool's own layer records"
        );
    }

    #[tokio::test]
    async fn past_the_rate_a_call_is_refused_and_counted() {
        let s = gate_server(mcp::Limits {
            calls_per_second: 1,
            ..mcp::Limits::DEFAULT
        })
        .await;
        let token = s.key(&[Scope::Read]);
        let first = s
            .post(Some(&token), &call("vcon_probe", json!({})), &[])
            .await;
        assert!(first.body["result"].is_object(), "{:?}", first.body);
        for _ in 0..2 {
            let a = s
                .post(Some(&token), &call("vcon_probe", json!({})), &[])
                .await;
            let (code, data) = refusal(&a);
            assert_eq!(code, -32000);
            assert_eq!(data["type"], problem("rate-limited"));
            assert_eq!(data["status"], 429);
            let retry = data["retry_after_ms"].as_u64().expect("retry_after_ms");
            assert!((1..=1000).contains(&retry), "{retry}");
        }
        let counted = logged(&s, "read", 429);
        assert_eq!(counted.len(), 1, "counted, not one row each: {counted:?}");
        assert_eq!(counted[0]["count"], 2);
        // Another key has its own allowance.
        let other = s.key(&[Scope::Read]);
        let a = s
            .post(Some(&other), &call("vcon_probe", json!({})), &[])
            .await;
        assert!(a.body["result"].is_object(), "{:?}", a.body);
    }

    #[tokio::test]
    async fn past_the_concurrency_cap_a_call_is_refused_at_once_and_counted() {
        let s = Arc::new(
            gate_server(mcp::Limits {
                concurrent: 1,
                ..mcp::Limits::DEFAULT
            })
            .await,
        );
        let token = s.key(&[Scope::Read]);
        let held = {
            let (s, token) = (s.clone(), token.clone());
            tokio::spawn(async move {
                s.post(Some(&token), &call("vcon_wait", json!({})), &[])
                    .await
            })
        };
        // The waiting call holds the only slot once its count of mcp
        // requests is not yet written: wait until it is running.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let refused = loop {
            let a = s
                .post(Some(&token), &call("vcon_probe", json!({})), &[])
                .await;
            if a.body["error"].is_object() {
                break a;
            }
            assert!(std::time::Instant::now() < deadline, "never at capacity");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let (code, data) = refusal(&refused);
        assert_eq!(code, -32000);
        assert_eq!(data["type"], problem("at-capacity"));
        assert_eq!(data["status"], 503);
        assert!(!logged(&s, "read", 503).is_empty());
        release().notify_one();
        let waited = held.await.expect("joins");
        assert_eq!(
            waited.body["result"]["structuredContent"],
            json!({"waited": true})
        );
        let a = s
            .post(Some(&token), &call("vcon_probe", json!({})), &[])
            .await;
        assert!(
            a.body["result"].is_object(),
            "the slot is free again: {:?}",
            a.body
        );
    }

    #[tokio::test]
    async fn an_unknown_tool_is_refused_and_recorded() {
        let s = gate_server(mcp::Limits::DEFAULT).await;
        let token = s.key(&[Scope::Read]);
        let a = s
            .post(Some(&token), &call("vcon_nothing", json!({})), &[])
            .await;
        let (code, data) = refusal(&a);
        assert_eq!(code, -32602);
        assert_eq!(data["type"], problem("unknown-tool"));
        assert_eq!(data["status"], 404);
        let recorded = logged(&s, "mcp", 404);
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(recorded[0]["count"], 1);
    }

    #[tokio::test]
    async fn a_scope_the_key_lacks_is_refused_and_recorded_under_the_tools_action() {
        let s = gate_server(mcp::Limits::DEFAULT).await;
        let read = s.key(&[Scope::Read]);
        let operator = s.operator();
        let everything = s.key(&[Scope::Read, Scope::Write, Scope::Delete, Scope::Legal]);
        for (token, tool, action) in [
            (&read, "vcon_change", "replace"),
            (&read, "vcon_erase", "delete"),
            (&read, "legal_probe", "legal_read"),
            (&read, "admin_probe", "tenant_list"),
            (&everything, "admin_probe", "tenant_list"),
            (&everything, "vcon_unhinted", "read"),
        ] {
            let before = logged(&s, action, 403).len();
            let a = s.post(Some(token), &call(tool, json!({})), &[]).await;
            let (code, data) = refusal(&a);
            assert_eq!(code, -32001, "{tool}");
            assert_eq!(data["type"], problem("forbidden"), "{tool}");
            assert_eq!(data["status"], 403, "{tool}");
            assert_eq!(logged(&s, action, 403).len(), before + 1, "{tool}");
        }
        // An operator key holds no vCon scope, and is recorded in the admin
        // log.
        let a = s
            .post(Some(&operator), &call("vcon_probe", json!({})), &[])
            .await;
        assert_eq!(refusal(&a).1["type"], problem("forbidden"));
        let admin: Vec<Value> = s
            .admin_log("read")
            .into_iter()
            .filter(|r| r["status"] == 403)
            .collect();
        assert_eq!(admin.len(), 1, "{admin:?}");
        // And the scopes each tool needs let it through.
        for (token, tool) in [
            (&everything, "vcon_change"),
            (&everything, "vcon_erase"),
            (&everything, "legal_probe"),
            (&operator, "admin_probe"),
        ] {
            let a = s.post(Some(token), &call(tool, json!({})), &[]).await;
            assert!(a.body["result"].is_object(), "{tool}: {:?}", a.body);
        }
    }

    #[tokio::test]
    async fn arguments_that_do_not_fit_are_refused_and_recorded() {
        let s = gate_server(mcp::Limits::DEFAULT).await;
        let token = s.key(&[Scope::Read]);
        for arguments in [json!({"token": "vcs_x"}), json!({"extra": 1})] {
            let a = s
                .post(Some(&token), &call("vcon_probe", arguments.clone()), &[])
                .await;
            let (code, data) = refusal(&a);
            assert_eq!(code, -32602, "{arguments}");
            assert_eq!(data["type"], problem("invalid-arguments"), "{arguments}");
            assert_eq!(data["status"], 422);
            let text = a.body.to_string();
            assert!(!text.contains("vcs_x"), "a value is never repeated: {text}");
        }
        assert_eq!(logged(&s, "read", 422).len(), 2);
    }

    /// The audit trigger refuses inserts only, so a request counted earlier
    /// in the minute keeps being counted while a new record fails: what
    /// fails is the gate's own record.
    #[tokio::test]
    async fn a_refusal_that_cannot_be_recorded_answers_audit_failed() {
        let s = gate_server(mcp::Limits::DEFAULT).await;
        let token = s.key(&[Scope::Read]);
        assert_eq!(s.post(Some(&token), &list_tools(), &[]).await.status, 200);
        s.tenants.store(s.tenant).expect("store").break_audit();
        for (tool, arguments) in [
            ("vcon_change", json!({})),
            ("vcon_nothing", json!({})),
            ("vcon_probe", json!({"extra": 1})),
        ] {
            let a = s.post(Some(&token), &call(tool, arguments), &[]).await;
            assert_eq!(a.status, 200, "{tool}: the request itself was counted");
            let (code, data) = refusal(&a);
            assert_eq!(code, -32000, "{tool}");
            assert_eq!(data["type"], problem("audit-failed"), "{tool}");
            assert_eq!(data["status"], 503, "{tool}");
        }
    }

    #[tokio::test]
    async fn a_count_that_cannot_be_written_answers_audit_failed() {
        let s = gate_server(mcp::Limits {
            calls_per_second: 1,
            ..mcp::Limits::DEFAULT
        })
        .await;
        let token = s.key(&[Scope::Read]);
        let first = s
            .post(Some(&token), &call("vcon_probe", json!({})), &[])
            .await;
        assert!(first.body["result"].is_object(), "{:?}", first.body);
        s.tenants.store(s.tenant).expect("store").break_audit();
        let a = s
            .post(Some(&token), &call("vcon_probe", json!({})), &[])
            .await;
        assert_eq!(a.status, 200, "the request itself was counted");
        let (code, data) = refusal(&a);
        assert_eq!(code, -32000);
        assert_eq!(data["type"], problem("audit-failed"));
    }

    #[tokio::test]
    async fn tools_list_shows_only_what_the_key_may_call() {
        let s = gate_server(mcp::Limits::DEFAULT).await;
        let names = |a: Answer| -> Vec<String> {
            a.body["result"]["tools"]
                .as_array()
                .expect("tools")
                .iter()
                .map(|t| t["name"].as_str().expect("name").to_owned())
                .collect()
        };
        let read = s.key(&[Scope::Read]);
        let everything = s.key(&[Scope::Read, Scope::Write, Scope::Delete, Scope::Legal]);
        let operator = s.operator();
        assert_eq!(
            names(s.post(Some(&read), &list_tools(), &[]).await),
            ["vcon_probe", "vcon_wait"]
        );
        assert_eq!(
            names(s.post(Some(&everything), &list_tools(), &[]).await),
            [
                "legal_probe",
                "vcon_change",
                "vcon_erase",
                "vcon_probe",
                "vcon_wait"
            ]
        );
        assert_eq!(
            names(s.post(Some(&operator), &list_tools(), &[]).await),
            ["admin_probe"]
        );
    }
}

/// The website's MCP page, held to the server it describes.
mod site_page {
    use super::*;

    fn read(relative: &str) -> String {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn page() -> String {
        read("website/content/docs/mcp.md")
    }

    /// The text of the page's section headed `heading`, up to the next one.
    fn section(page: &str, heading: &str) -> String {
        page.split(&format!("\n## {heading}\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("no section {heading}"))
            .split("\n## ")
            .next()
            .expect("its text")
            .to_owned()
    }

    /// Every three-digit status in the section's bullets.
    fn statuses(text: &str) -> std::collections::BTreeSet<u16> {
        text.lines()
            .filter(|l| l.starts_with("- "))
            .flat_map(|l| {
                l.split(|c: char| !c.is_ascii_digit())
                    .filter(|w| w.len() == 3)
                    .filter_map(|w| w.parse::<u16>().ok())
                    .filter(|n| (100..600).contains(n))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[tokio::test]
    async fn the_page_names_each_tool_or_says_there_are_none() {
        let s = server().await;
        let token = s.operator();
        let a = s.post(Some(&token), &list_tools(), &[]).await;
        let tools = a.body["result"]["tools"].as_array().expect("tools").clone();
        let page = page();
        if tools.is_empty() {
            assert!(
                section(&page, "Tools").contains("There are no tools yet."),
                "the server has no tools; the page must say so"
            );
        }
        for tool in &tools {
            let name = tool["name"].as_str().expect("a name");
            assert!(
                page.contains(&format!("`{name}`")),
                "{name} is not on the page"
            );
        }
    }

    /// The versions the handshake tests take, which are rmcp's own list.
    #[test]
    fn the_page_names_each_protocol_version_the_server_takes() {
        let known: Vec<&str> = rmcp::model::ProtocolVersion::KNOWN_VERSIONS
            .iter()
            .map(|v| v.as_str())
            .collect();
        assert_eq!(
            known,
            [
                "2024-11-05",
                "2025-03-26",
                "2025-06-18",
                "2025-11-25",
                "2026-07-28"
            ],
            "rmcp's versions changed: update the handshake tests and the page"
        );
        // Prose wraps anywhere, so compare it with its whitespace collapsed.
        let text = section(&page(), "Connect over HTTPS")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains("2024-11-05 to 2025-11-25") && text.contains("2026-07-28"),
            "{text}"
        );
    }

    /// Each answer the page's HTTPS section names is the one the server
    /// gives: no key, a web page's Origin, GET and DELETE, a body too large.
    #[tokio::test]
    async fn the_answers_the_page_names_are_the_servers() {
        let s = server().await;
        let token = s.key(&[Scope::Read]);
        let mut seen = std::collections::BTreeSet::new();
        let none = s.post(None, &initialize("2025-11-25"), &[]).await;
        assert_eq!(
            none.headers
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer")
        );
        seen.insert(none.status);
        let origin = [("origin", "https://page.example.com")];
        seen.insert(s.post(Some(&token), &list_tools(), &origin).await.status);
        for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
            let answer = s
                .client()
                .request(method, s.url())
                .header("authorization", format!("Bearer {token}"))
                .send()
                .await
                .expect("an answer");
            seen.insert(answer.status().as_u16());
        }
        let mut large = list_tools();
        large["params"]["padding"] = Value::String("x".repeat(LIMIT));
        seen.insert(s.post(Some(&token), &large, &[]).await.status);

        let text = section(&page(), "Connect over HTTPS");
        assert_eq!(statuses(&text), seen, "{text}");
        assert!(text.contains("`WWW-Authenticate: Bearer`"), "{text}");
    }

    /// Each client configuration on the page starts `vcon-server mcp
    /// --config` with a client file, or reaches `/mcp` over HTTPS with a
    /// bearer key.
    #[test]
    fn the_client_configurations_use_the_servers_commands() {
        let page = page();
        let blocks: Vec<Value> = page
            .split("```json\n")
            .skip(1)
            .map(|b| serde_json::from_str(b.split("```").next().expect("a block")).expect("JSON"))
            .collect();
        let (mut stdio, mut http) = (0, 0);
        for block in &blocks {
            let servers = block["mcpServers"].as_object().expect("mcpServers");
            for (name, server) in servers {
                assert_eq!(name, "vcon-server");
                if let Some(command) = server.get("command") {
                    assert_eq!(command, "vcon-server");
                    let args = server["args"].as_array().expect("args");
                    assert_eq!(args.len(), 3, "{args:?}");
                    assert_eq!(args[..2], [json!("mcp"), json!("--config")]);
                    let file = args[2].as_str().expect("a path");
                    assert!(file.starts_with('/') && file.ends_with(".toml"), "{file}");
                    stdio += 1;
                } else {
                    assert_eq!(server["type"], "http");
                    let url = server["url"].as_str().expect("a url");
                    assert!(
                        url.starts_with("https://") && url.ends_with("/mcp"),
                        "{url}"
                    );
                    let auth = server["headers"]["Authorization"].as_str().expect("auth");
                    assert!(auth.starts_with("Bearer "), "{auth}");
                    http += 1;
                }
            }
        }
        assert!(stdio >= 1 && http >= 1, "stdio {stdio}, http {http}");
    }

    #[test]
    fn llms_txt_links_the_page() {
        assert!(read("website/static/llms.txt").contains("(https://vcon-server.com/docs/mcp/)"));
    }

    /// The rows of the page's gate table: check, refusal link, status and
    /// JSON-RPC code, in the order the page lists them.
    fn gate_rows(text: &str) -> Vec<Vec<String>> {
        text.lines()
            .filter(|l| l.starts_with("| ") && !l.starts_with("| Check"))
            .map(|l| {
                l.trim_matches('|')
                    .split(" | ")
                    .map(|c| c.trim().to_owned())
                    .collect()
            })
            .collect()
    }

    /// Each refusal the gate gives, in the order it checks, with its
    /// problem type, status and JSON-RPC code, and the limits' defaults.
    #[test]
    fn the_gate_the_page_names_is_the_servers() {
        use crate::api::Problem;
        let text = section(&page(), "Tool calls");
        let rows = gate_rows(&text);
        let order = [
            Problem::RateLimited,
            Problem::AtCapacity,
            Problem::UnknownTool,
            Problem::Forbidden,
            Problem::InvalidArguments,
        ];
        assert_eq!(rows.len(), order.len(), "{rows:?}");
        for (row, problem) in rows.iter().zip(order) {
            let slug = problem.name().0;
            let code = crate::mcp::gate::refused(problem, None).code.0;
            assert_eq!(
                row[1..],
                [
                    format!("[{slug}](@/problems/{slug}.md)"),
                    problem.status().as_u16().to_string(),
                    code.to_string(),
                ],
                "{row:?}"
            );
        }
        let limits = crate::mcp::Limits::DEFAULT;
        assert!(rows[0][0].contains(&format!(
            "`mcp.calls_per_second` calls by one key in one second (default {})",
            limits.calls_per_second
        )));
        assert!(rows[1][0].contains(&format!(
            "`mcp.concurrent` calls running on the server (default {})",
            limits.concurrent
        )));
    }

    /// The scope rule the page states is the one the gate applies.
    #[test]
    fn the_scope_rule_the_page_states_is_the_gates() {
        use rmcp::model::ToolAnnotations;
        let hints = |read_only, destructive| {
            let mut h = ToolAnnotations::new();
            h.read_only_hint = Some(read_only);
            h.destructive_hint = Some(destructive);
            h
        };
        let scope = |name: &str, r, d| crate::mcp::gate::scope(name, Some(&hints(r, d)));
        assert_eq!(scope("vcon_x", true, false), Some(Scope::Read));
        assert_eq!(scope("vcon_x", false, true), Some(Scope::Delete));
        assert_eq!(scope("vcon_x", false, false), Some(Scope::Write));
        assert_eq!(scope("legal_x", true, false), Some(Scope::Legal));
        assert_eq!(scope("admin_x", false, true), Some(Scope::Admin));
        let text = section(&page(), "Tool calls")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.contains(
                "a `vcon_` tool needs read when it is read-only, delete when it is \
                 destructive, and write otherwise; a `legal_` tool needs legal; an \
                 `admin_` tool needs admin"
            ),
            "{text}"
        );
    }
}

/// The MCP module's types carry the traits the Rust API Guidelines ask for.
mod traits {
    fn error<T: std::error::Error + Send + Sync + Clone + PartialEq + 'static>() {}

    fn debug<T: std::fmt::Debug>() {}

    #[test]
    fn error_types_are_errors_and_every_type_is_debug() {
        error::<crate::mcp::stdio::StdioError>();
        error::<crate::mcp::Outcome>();
        debug::<crate::mcp::Limits>();
        debug::<crate::mcp::Hints>();
        debug::<crate::mcp::Spec>();
        debug::<crate::mcp::Server>();
        debug::<crate::mcp::gate::Gate>();
    }

    #[test]
    fn an_outcome_says_why_the_tool_did_not_answer() {
        assert_eq!(
            crate::mcp::Outcome::Arguments.to_string(),
            "the arguments do not fit the tool",
            "Display of Outcome::Arguments"
        );
    }
}
