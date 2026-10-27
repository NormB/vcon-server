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
use crate::clock::SystemClock;
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
        limits: api::Limits {
            body: LIMIT,
            inflated: 1 << 20,
            body_timeout: Duration::from_millis(300),
        },
        supported: Default::default(),
        retention_default: Retention::Years(7),
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
    tokio::spawn(api::serve(listener, tls, api::router(app), async {
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
}
