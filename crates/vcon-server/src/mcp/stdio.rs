//! `vcon-server mcp`: MCP on stdin and stdout, for an agent that starts the
//! server as a command. Each request is forwarded to a running server's
//! `/mcp` with the one key the client file names, so stdio passes the same
//! authentication, gate and audit as HTTP. The command opens no data
//! directory and no master key.

use std::fmt;
use std::path::{Path, PathBuf};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ListToolsResult, PaginatedRequestParams, ServerConfig,
};
use rmcp::service::{
    ClientInitializeError, Peer, RequestContext, RoleClient, RoleServer, RunningService,
    ServiceError,
};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::{ErrorData, ServerHandler, ServiceExt};
use serde::Deserialize;

use crate::config::ConfigError;

/// The client file `vcon-server mcp` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// The server's MCP address, `https://host:port/mcp`.
    pub url: String,
    /// A file holding one API token and nothing else.
    pub key_file: PathBuf,
    /// PEM certificates to trust for the server; the system's roots when
    /// absent.
    pub ca: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    url: String,
    key_file: PathBuf,
    ca: Option<PathBuf>,
}

/// Reads the client file at `path`.
///
/// # Errors
///
/// [`ConfigError`] when the file cannot be read or does not parse as a
/// client file.
pub fn client_config(path: &Path) -> Result<ClientConfig, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
    parse_client(&text)
}

/// Reads a client file from its TOML text. The URL must be `https`: the
/// server speaks TLS only.
///
/// # Errors
///
/// [`ConfigError`] when the text is not TOML, has a key a client file
/// does not take, or names a URL that is not `https`.
pub fn parse_client(text: &str) -> Result<ClientConfig, ConfigError> {
    let raw: Raw = toml::from_str(text).map_err(|e| ConfigError(e.to_string()))?;
    if !raw.url.starts_with("https://") {
        return Err(ConfigError(format!(
            "url: {:?} is not an https:// URL",
            raw.url
        )));
    }
    Ok(ClientConfig {
        url: raw.url,
        key_file: raw.key_file,
        ca: raw.ca,
    })
}

/// Why the command stopped. None of these holds the token.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub enum StdioError {
    /// The key file could not be read, or holds no single token.
    KeyFile(String),
    /// The CA file could not be read, or the TLS client not made.
    Tls(String),
    /// The server refused the key with this status.
    Refused(u16),
    /// The server could not be reached, or answered something other than
    /// MCP.
    Server(String),
    /// stdin or stdout failed.
    Io(String),
}

impl std::error::Error for StdioError {}

impl fmt::Display for StdioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StdioError::KeyFile(why) => f.write_str(why),
            StdioError::Tls(why) => write!(f, "ca: {why}"),
            StdioError::Refused(status) => {
                write!(f, "the server refused the key ({status})")
            }
            StdioError::Server(why) => write!(f, "the server: {why}"),
            StdioError::Io(why) => write!(f, "stdio: {why}"),
        }
    }
}

/// The one token a key file holds, with white space at either end removed.
/// The file is held to the one secret-file rule (`crate::secret_file`).
pub(crate) fn token(key_file: &Path) -> Result<String, StdioError> {
    let refused =
        |why: &str| StdioError::KeyFile(format!("key_file {}: {why}", key_file.display()));
    let bytes = crate::secret_file::read(key_file, "key_file").map_err(StdioError::KeyFile)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| refused("the file is not UTF-8"))?;
    let token = text.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(refused("holds no single token"));
    }
    Ok(token.to_owned())
}

/// An HTTPS client trusting `ca`, or the system's roots without it. It
/// connects directly, through no proxy.
pub(crate) fn https_client(ca: Option<&Path>) -> Result<reqwest::Client, StdioError> {
    let mut builder = reqwest::Client::builder().no_proxy();
    if let Some(ca) = ca {
        let pem =
            std::fs::read(ca).map_err(|e| StdioError::Tls(format!("{}: {e}", ca.display())))?;
        let roots = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|e| StdioError::Tls(format!("{}: {e}", ca.display())))?;
        if roots.is_empty() {
            return Err(StdioError::Tls(format!(
                "{}: holds no certificate",
                ca.display()
            )));
        }
        builder = builder.tls_certs_only(roots);
    }
    builder.build().map_err(|e| StdioError::Tls(e.to_string()))
}

/// Why the handshake with the server failed: the 401 when the server
/// refused the key, else the error's text.
fn refusal(error: &ClientInitializeError) -> StdioError {
    if let ClientInitializeError::TransportError { error, .. } = error
        && let Some(StreamableHttpError::AuthRequired(_)) = error
            .error
            .downcast_ref::<StreamableHttpError<reqwest::Error>>()
    {
        return StdioError::Refused(401);
    }
    StdioError::Server(error.to_string())
}

/// The handshake the stdio side answers: the server's own name,
/// capabilities and instructions.
fn identity(peer: &rmcp::model::ServerPeerInfo) -> Result<ServerConfig, StdioError> {
    let Some(name) = peer.server_info.clone() else {
        return Err(StdioError::Server("no server name".to_owned()));
    };
    let mut info = ServerConfig::new(peer.capabilities.clone());
    info.server_info = name;
    info.instructions = peer.instructions.clone();
    Ok(info)
}

/// Runs the command until stdin ends.
///
/// # Errors
///
/// [`StdioError`] when the key file or the CA cannot be read, when the
/// server cannot be reached or refuses the key, or when stdin or stdout
/// fails.
pub async fn stdio(config: &ClientConfig) -> Result<(), StdioError> {
    let token = token(&config.key_file)?;
    let client = https_client(config.ca.as_deref())?;
    let transport = StreamableHttpClientTransport::with_client(
        client,
        StreamableHttpClientTransportConfig::with_uri(config.url.as_str()).auth_header(token),
    );
    let upstream: RunningService<RoleClient, rmcp::model::ClientConfig> =
        rmcp::model::ClientConfig::default()
            .serve(transport)
            .await
            .map_err(|e| refusal(&e))?;
    // The server's own identity and capabilities, as it gave them.
    let peer = upstream
        .peer_info()
        .ok_or_else(|| StdioError::Server("no server information".to_owned()))?;
    let info = identity(&peer)?;
    let forward = Forward {
        upstream: upstream.peer().clone(),
        info,
    };
    let served = forward
        .serve(rmcp::transport::io::stdio())
        .await
        .map_err(|e| StdioError::Io(e.to_string()))?;
    served
        .waiting()
        .await
        .map_err(|e| StdioError::Io(e.to_string()))?;
    upstream
        .cancel()
        .await
        .map_err(|e| StdioError::Server(e.to_string()))?;
    Ok(())
}

/// The stdio side: answers the handshake as the server answered it, and
/// sends each tool listing and tool call to the server.
#[derive(Clone)]
struct Forward {
    upstream: Peer<RoleClient>,
    info: ServerConfig,
}

/// The server's own refusal as it sent it; any other failure as an
/// internal error that says what failed.
fn forwarded(error: ServiceError) -> ErrorData {
    match error {
        ServiceError::McpError(error) => error,
        other => ErrorData::internal_error(format!("the server: {other}"), None),
    }
}

impl ServerHandler for Forward {
    fn get_info(&self) -> ServerConfig {
        self.info.clone()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.upstream.list_tools(request).await.map_err(forwarded)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.upstream
            .call_tool_once(request)
            .await
            .map_err(forwarded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_file_is_read_and_its_url_must_be_https() {
        assert_eq!(
            parse_client(
                "url = \"https://vcon.example.com:8443/mcp\"\nkey_file = \"/etc/agent.token\"\nca = \"/etc/ca.pem\"\n"
            ),
            Ok(ClientConfig {
                url: "https://vcon.example.com:8443/mcp".to_owned(),
                key_file: PathBuf::from("/etc/agent.token"),
                ca: Some(PathBuf::from("/etc/ca.pem")),
            })
        );
        let without_ca =
            parse_client("url = \"https://vcon.example.com/mcp\"\nkey_file = \"k\"\n").expect("ok");
        assert_eq!(without_ca.ca, None);
        for (text, named) in [
            (
                "url = \"http://vcon.example.com/mcp\"\nkey_file = \"k\"\n",
                "url",
            ),
            ("key_file = \"k\"\n", "url"),
            ("url = \"https://vcon.example.com/mcp\"\n", "key_file"),
            (
                "url = \"https://vcon.example.com/mcp\"\nkey_file = \"k\"\ntoken = \"t\"\n",
                "token",
            ),
        ] {
            let error = parse_client(text).expect_err(text);
            assert!(error.0.contains(named), "{text}: {}", error.0);
        }
    }

    /// The client file the website's command-line page shows reads.
    #[test]
    fn the_client_file_on_the_command_line_page_reads() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../website/content/docs/command-line.md");
        let text = std::fs::read_to_string(&path).expect("the page");
        let section = text
            .split("## MCP for an AI agent\n")
            .nth(1)
            .expect("an MCP section")
            .split("\n## ")
            .next()
            .expect("its text");
        let examples: Vec<&str> = section
            .split("```toml\n")
            .skip(1)
            .map(|block| block.split("```").next().expect("a block"))
            .collect();
        assert_eq!(examples.len(), 1);
        let read = parse_client(examples[0]).expect("reads");
        assert_eq!(read.url, "https://vcon.example.com:8443/mcp");
        assert!(read.ca.is_some());
    }

    #[test]
    fn each_error_names_what_failed() {
        for (error, text) in [
            (
                StdioError::KeyFile("key_file k: gone".to_owned()),
                "key_file k: gone",
            ),
            (StdioError::Tls("c: bad".to_owned()), "ca: c: bad"),
            (StdioError::Refused(401), "the server refused the key (401)"),
            (StdioError::Server("down".to_owned()), "the server: down"),
            (StdioError::Io("closed".to_owned()), "stdio: closed"),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }

    #[test]
    fn the_handshake_answers_with_the_servers_name_or_not_at_all() {
        use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerPeerInfo};
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        let peer = ServerPeerInfo::new(ProtocolVersion::LATEST, capabilities.clone())
            .with_server_info(Implementation::new("vcon-server", "9.9.9"))
            .with_instructions("held");
        let info = identity(&peer).expect("named");
        assert_eq!(info.server_info.name, "vcon-server");
        assert_eq!(info.server_info.version, "9.9.9");
        assert_eq!(info.capabilities, capabilities);
        assert_eq!(info.instructions.as_deref(), Some("held"));
        let nameless = ServerPeerInfo::new(ProtocolVersion::LATEST, capabilities);
        assert!(matches!(identity(&nameless), Err(StdioError::Server(_))));
    }

    #[test]
    fn a_key_file_holds_exactly_one_token() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("agent.token");
        std::fs::write(&file, "  vcs_0011223344556677_secret \n").expect("write");
        key_mode(&file, 0o600);
        assert_eq!(token(&file).expect("read"), "vcs_0011223344556677_secret");
        for text in ["", " \n", "vcs_a b"] {
            std::fs::write(&file, text).expect("write");
            assert!(
                matches!(token(&file), Err(StdioError::KeyFile(_))),
                "{text:?}"
            );
        }
        assert!(matches!(
            token(&dir.path().join("missing")),
            Err(StdioError::KeyFile(_))
        ));
    }

    /// A TLS server for one test: to each request, after its headers, it
    /// answers 200 with a chunked `application/json` body of spaces that
    /// never ends, until the client closes or `cap` bytes are sent. It
    /// returns how many body bytes it sent, over every connection.
    fn endless_json_server(
        cert: &rcgen::CertifiedKey<rcgen::KeyPair>,
        cap: usize,
    ) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
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
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = sent.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(tcp) = stream else { return };
                let conn = rustls::ServerConnection::new(config.clone()).expect("a connection");
                let mut tls = rustls::StreamOwned::new(conn, tcp);
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match tls.read(&mut byte) {
                        Ok(1) => head.push(byte[0]),
                        _ => break,
                    }
                }
                let reply = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n";
                if tls.write_all(reply).is_err() {
                    continue;
                }
                let chunk = vec![b' '; 64 * 1024];
                let framed = [
                    format!("{:x}\r\n", chunk.len()).into_bytes(),
                    chunk,
                    b"\r\n".to_vec(),
                ]
                .concat();
                while counter.load(Ordering::SeqCst) < cap {
                    if tls.write_all(&framed).is_err() {
                        break;
                    }
                    counter.fetch_add(64 * 1024, Ordering::SeqCst);
                }
            }
        });
        (port, sent)
    }

    /// RUSTSEC-2026-0462: before rmcp 3.9.0 its Streamable HTTP client read
    /// a JSON answer whole, with no limit, so an endless one from a
    /// malicious or broken server held the bridge, and its host's memory,
    /// until the process ran out. The bridge must give up on it instead.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_endless_json_answer_to_the_handshake_is_given_up_not_buffered() {
        use std::sync::atomic::Ordering;
        const CAP: usize = 128 * 1024 * 1024;
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate");
        let (port, sent) = endless_json_server(&cert, CAP);
        let dir = tempfile::tempdir().expect("temp dir");
        let ca = dir.path().join("ca.pem");
        std::fs::write(&ca, cert.cert.pem()).expect("write the CA");
        let key_file = dir.path().join("agent.token");
        std::fs::write(&key_file, "vcs_0011223344556677_secret\n").expect("write the key file");
        std::fs::set_permissions(
            &key_file,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("chmod 0600");
        let config = parse_client(&format!(
            "url = \"https://localhost:{port}/mcp\"\nkey_file = {key_file:?}\nca = {ca:?}\n"
        ))
        .expect("a client configuration");
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(120), stdio(&config))
            .await
            .expect("the bridge gives up within two minutes");
        assert!(
            matches!(outcome, Err(StdioError::Server(_))),
            "the handshake fails: {outcome:?}"
        );
        let bytes = sent.load(Ordering::SeqCst);
        assert!(
            bytes < 32 * 1024 * 1024,
            "the bridge stopped reading near rmcp's 16 MiB limit, not after {bytes} bytes"
        );
    }

    fn key_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn key_file_of_mode(dir: &tempfile::TempDir, mode: u32) -> std::path::PathBuf {
        let file = dir.path().join("agent.token");
        std::fs::write(&file, "vcs_0011223344556677_secret\n").expect("write");
        key_mode(&file, mode);
        file
    }

    #[test]
    fn a_key_file_other_users_may_read_is_refused_with_the_fix() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = key_file_of_mode(&dir, 0o644);
        let error = token(&file).expect_err("refused");
        let path = file.display();
        assert_eq!(
            error.to_string(),
            format!(
                "key_file {path}: other users may read or change the file (mode 0644); run \
                 `chmod 600 {path}`, or `chmod 640 {path}` for a group that only reads it"
            ),
            "the refusal names key_file, the file, its mode and the fix"
        );
        assert!(!error.to_string().contains("vcs_0011"), "{error}");
    }

    #[test]
    fn a_key_file_its_group_may_read_is_used() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = key_file_of_mode(&dir, 0o640);
        assert_eq!(
            token(&file).expect("read"),
            "vcs_0011223344556677_secret",
            "a group may read a 0640 key file"
        );
    }

    #[test]
    fn a_missing_key_file_names_the_key_and_the_file() {
        let error = token(Path::new("/nonexistent/agent.token")).expect_err("refused");
        assert_eq!(
            error.to_string(),
            "key_file /nonexistent/agent.token: No such file or directory (os error 2)",
            "a missing key file is named with its key and path"
        );
    }
}
