//! `vcon-server mcp`: MCP on stdin and stdout, for an agent that starts the
//! server as a command. Each request is forwarded to a running server's
//! `/mcp` with the one key the client file names, so stdio passes the same
//! authentication, gate and audit as HTTP. The command opens no data
//! directory and no master key.

use std::fmt;
use std::path::{Path, PathBuf};

use rmcp::model::ServerConfig;
use rmcp::service::{ClientInitializeError, RoleClient, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::{ServerHandler, ServiceExt};
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    url: String,
    key_file: PathBuf,
    ca: Option<PathBuf>,
}

/// Reads the client file at `path`.
pub fn client_config(path: &Path) -> Result<ClientConfig, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("{}: {e}", path.display())))?;
    parse_client(&text)
}

/// Reads a client file from its TOML text. The URL must be `https`: the
/// server speaks TLS only.
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
#[derive(Debug)]
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

impl fmt::Display for StdioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StdioError::KeyFile(why) => write!(f, "key_file: {why}"),
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
fn token(key_file: &Path) -> Result<String, StdioError> {
    let text = std::fs::read_to_string(key_file)
        .map_err(|e| StdioError::KeyFile(format!("{}: {e}", key_file.display())))?;
    let token = text.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err(StdioError::KeyFile(format!(
            "{}: holds no single token",
            key_file.display()
        )));
    }
    Ok(token.to_owned())
}

/// An HTTPS client trusting `ca`, or the system's roots without it. It
/// connects directly, through no proxy.
fn https_client(ca: Option<&Path>) -> Result<reqwest::Client, StdioError> {
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
    let forward = Forward { info };
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

/// The stdio side: answers the handshake as the server answered it.
#[derive(Clone)]
struct Forward {
    info: ServerConfig,
}

impl ServerHandler for Forward {
    fn get_info(&self) -> ServerConfig {
        self.info.clone()
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
                StdioError::KeyFile("k: gone".to_owned()),
                "key_file: k: gone",
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
}
