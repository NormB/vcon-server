//! `vcon-server mcp`, the stdio MCP command, run as its own process against
//! a running `vcon-server serve`: the handshake comes back from the server
//! through the command, and a key the server refuses stops the command.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vcon_server::access::{Scope, Scopes};
use vcon_server::audit::Actor;
use vcon_server::clock::SystemClock;
use vcon_server::envelope::Key;
use vcon_server::fs::OsFs;
use vcon_server::tenants::Tenants;

/// A data directory with one tenant and a key for it, a certificate, and a
/// running server; the server is killed when this is dropped.
struct Setup {
    dir: tempfile::TempDir,
    cert: PathBuf,
    token: String,
    revoked: String,
    addr: SocketAddr,
    server: Running,
}

/// A child process killed when dropped, so a failed assertion leaves none.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// The server listens on port 0, so it takes a port of its own when it
/// binds, and the address comes from its `serving` line. A port chosen
/// here and freed for the server could be taken by another test's
/// listener first (MCP-STDIO-HANDSHAKE-RACE).
fn setup() -> Setup {
    setup_at("127.0.0.1:0")
}

/// As `setup`, with the server's configuration listening at `listen`; the
/// server is ready once its own log says where it listens.
fn setup_at(listen: &str) -> Setup {
    let dir = tempfile::tempdir().expect("temp dir");
    let data = dir.path().join("data");
    std::fs::create_dir(&data).expect("data dir");
    let master_key = dir.path().join("master.key");
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .expect("urandom")
        .read_exact(&mut bytes)
        .expect("32 random bytes");
    std::fs::write(&master_key, bytes).expect("write the key file");
    let tenants = Tenants::open(
        &data,
        Key::load_master(&master_key).expect("the master key"),
        Arc::new(OsFs),
        Arc::new(SystemClock),
    )
    .expect("opens");
    let tenant = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let key = |label| {
        tenants
            .create_key(Actor::Cli, Some(tenant), Scopes::of(&[Scope::Read]), label)
            .expect("a key")
    };
    let token = key("agent").token;
    let revoked = key("gone");
    tenants.revoke_key(Actor::Cli, revoked.id).expect("revoked");
    drop(tenants);

    let made =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate");
    let (cert, tls_key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    std::fs::write(&cert, made.cert.pem()).expect("write");
    std::fs::write(&tls_key, made.signing_key.serialize_pem()).expect("write");
    let config = dir.path().join("vcon-server.toml");
    std::fs::write(
        &config,
        format!(
            "listen = \"{listen}\"\ndata_dir = \"{}\"\nmaster_key = \"{}\"\n\n[tls]\ncert = \"{}\"\nkey = \"{}\"\n",
            data.display(),
            master_key.display(),
            cert.display(),
            tls_key.display(),
        ),
    )
    .expect("write the configuration");
    let mut server = Running(
        Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .arg("serve")
            .arg("--config")
            .arg(&config)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("serve starts"),
    );
    // Its stderr is read to the end on a thread, so the server never
    // blocks on a full pipe; the lines come here until it says where it
    // listens.
    let stderr = server.0.stderr.take().expect("piped");
    let (send, log) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = send.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut early = Vec::new();
    let addr = loop {
        match log.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => match bound(&line) {
                Some(addr) => break addr,
                None => early.push(line),
            },
            Err(_) => panic!(
                "serve never said where it listens ({:?}): {early:?}",
                server.0.try_wait()
            ),
        }
    };
    assert_ne!(
        addr.port(),
        0,
        "the serving line names the port bound, not the port configured"
    );
    Setup {
        dir,
        cert,
        token,
        revoked: revoked.token,
        addr,
        server,
    }
}

/// The address a `serving` line names, from its `listen=` field.
fn bound(line: &str) -> Option<SocketAddr> {
    let mut fields = line.split_whitespace();
    fields.find(|f| *f == "serving")?;
    fields.find_map(|f| f.strip_prefix("listen="))?.parse().ok()
}

impl Setup {
    /// A client file for `vcon-server mcp` holding `token` in its key file.
    fn client_file(&self, name: &str, token: &str) -> PathBuf {
        let key_file = self.dir.path().join(format!("{name}.token"));
        std::fs::write(&key_file, format!("{token}\n")).expect("write the key file");
        let file = self.dir.path().join(format!("{name}.toml"));
        std::fs::write(
            &file,
            format!(
                "url = \"https://localhost:{}/mcp\"\nkey_file = \"{}\"\nca = \"{}\"\n",
                self.addr.port(),
                key_file.display(),
                self.cert.display(),
            ),
        )
        .expect("write the client file");
        file
    }
}

fn mcp(client_file: &Path) -> Running {
    Running(
        Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .arg("mcp")
            .arg("--config")
            .arg(client_file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("mcp starts"),
    )
}

/// Lines from a child's stdout, read on a thread so a wait can time out.
fn lines(stdout: ChildStdout) -> mpsc::Receiver<String> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            if send.send(line).is_err() {
                return;
            }
        }
    });
    receive
}

/// The next answer on the command's stdout. When none comes, the
/// command's exit status and stderr say why.
fn next(lines: &mpsc::Receiver<String>, running: &mut Running) -> Value {
    let line = match lines.recv_timeout(Duration::from_secs(20)) {
        Ok(line) => line,
        Err(e) => {
            let _ = running.0.kill();
            let status = running.0.wait();
            let mut stderr = String::new();
            if let Some(mut pipe) = running.0.stderr.take() {
                let _ = pipe.read_to_string(&mut stderr);
            }
            panic!("an answer on stdout: {e}; the command: {status:?}, stderr: {stderr}");
        }
    };
    serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
}

/// A server that cannot bind its port is never taken for a ready one,
/// even when another process listens there. That other listener is what
/// a test found when a port it had freed was taken before its server
/// bound it: the command then reached the other server, or none, and
/// stopped before it answered (MCP-STDIO-HANDSHAKE-RACE).
#[test]
fn a_server_that_cannot_bind_is_never_taken_for_ready() {
    let decoy = TcpListener::bind("127.0.0.1:0").expect("a port");
    let held = decoy.local_addr().expect("addr").to_string();
    let started = std::panic::catch_unwind(|| setup_at(&held));
    let refused = started
        .err()
        .expect("the setup refuses a server that never bound its port");
    let why = refused
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| refused.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default();
    assert!(
        why.contains("serve never said where it listens"),
        "the refusal names the missing serving line: {why}"
    );
    drop(decoy);
}

#[test]
fn the_handshake_over_stdio_comes_from_the_server() {
    let s = setup();
    let mut running = mcp(&s.client_file("agent", &s.token));
    let mut stdin = running.0.stdin.take().expect("stdin");
    let out = lines(running.0.stdout.take().expect("stdout"));
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"},
        },
    });
    writeln!(stdin, "{initialize}").expect("write");
    let answer = next(&out, &mut running);
    assert_eq!(answer["id"], 1, "{answer}");
    let result = &answer["result"];
    assert_eq!(result["serverInfo"]["name"], "vcon-server", "{answer}");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(result["capabilities"]["tools"].is_object(), "{answer}");
    assert!(result["capabilities"]["resources"].is_null(), "{answer}");

    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .expect("write");
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
    )
    .expect("write");
    let answer = next(&out, &mut running);
    assert_eq!(answer["id"], 2, "{answer}");
    let names: Vec<&str> = answer["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().expect("a name"))
        .collect();
    assert_eq!(
        names,
        ["vcon_get", "vcon_list"],
        "the key's tools, from the server: {answer}"
    );

    // A tool call goes to the server: its refusal comes back as it sent it.
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "vcon_nothing", "arguments": {}}})
    )
    .expect("write");
    let answer = next(&out, &mut running);
    assert_eq!(answer["id"], 3, "{answer}");
    assert_eq!(answer["error"]["code"], -32602, "{answer}");
    assert_eq!(
        answer["error"]["data"]["type"], "https://vcon-server.com/problems/unknown-tool",
        "{answer}"
    );

    // The end of stdin ends the command, cleanly.
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = running.0.try_wait().expect("wait") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "mcp did not stop at end of input"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{status}");
    drop(s.server);
}

#[test]
fn a_key_the_server_refuses_stops_the_command_with_the_401() {
    let s = setup();
    for (name, token) in [
        ("revoked", s.revoked.clone()),
        ("unknown", "vcs_0123456789abcdef_AAAA".to_owned()),
    ] {
        let mut running = mcp(&s.client_file(name, &token));
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = running.0.try_wait().expect("wait") {
                break status;
            }
            assert!(Instant::now() < deadline, "{name}: mcp did not stop");
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut stderr = String::new();
        running
            .0
            .stderr
            .take()
            .expect("stderr")
            .read_to_string(&mut stderr)
            .expect("read");
        assert_eq!(status.code(), Some(1), "{name}: {stderr}");
        assert!(stderr.contains("401"), "{name}: {stderr}");
        assert!(!stderr.contains(&token), "{name}: the token is never shown");
    }
}

#[test]
fn a_ca_file_without_a_certificate_or_a_server_not_there_stops_the_command() {
    let s = setup();
    let empty_ca = s.dir.path().join("empty-ca.pem");
    std::fs::write(&empty_ca, "").expect("write");
    // A PEM block that holds no certificate DER: it reads as PEM, and the
    // TLS client then refuses it.
    let bad_ca = s.dir.path().join("bad-ca.pem");
    std::fs::write(
        &bad_ca,
        "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
    )
    .expect("write");
    let key_file = s.dir.path().join("agent.token");
    std::fs::write(&key_file, &s.token).expect("write");
    // A port held by the client end of a connection, where nothing
    // listens: a connection to it is refused, and no server can bind it
    // while it is held. A port freed here could be taken by another
    // test's server first (MCP-STDIO-HANDSHAKE-RACE).
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let held =
        std::net::TcpStream::connect(listener.local_addr().expect("addr")).expect("a connection");
    let nothing = held.local_addr().expect("its client end");
    for (name, url, ca, named) in [
        (
            "empty ca",
            format!("https://localhost:{}/mcp", s.addr.port()),
            Some(empty_ca.clone()),
            "ca: ",
        ),
        (
            "bad ca",
            format!("https://localhost:{}/mcp", s.addr.port()),
            Some(bad_ca.clone()),
            "ca: ",
        ),
        (
            "no server",
            format!("https://localhost:{}/mcp", nothing.port()),
            Some(s.cert.clone()),
            "the server: ",
        ),
        (
            // Without a CA the system's roots are trusted, and they do not
            // hold the test server's own certificate.
            "system roots",
            format!("https://localhost:{}/mcp", s.addr.port()),
            None,
            "the server: ",
        ),
    ] {
        let file = s
            .dir
            .path()
            .join(format!("{}.toml", name.replace(' ', "-")));
        let ca = ca.map_or(String::new(), |ca| format!("ca = \"{}\"\n", ca.display()));
        std::fs::write(
            &file,
            format!(
                "url = \"{url}\"\nkey_file = \"{}\"\n{ca}",
                key_file.display()
            ),
        )
        .expect("write");
        let output = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .arg("mcp")
            .arg("--config")
            .arg(&file)
            .stdin(Stdio::null())
            .output()
            .expect("runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{name}: {stderr}");
        assert!(stderr.contains(named), "{name}: {stderr}");
        assert!(
            !stderr.contains(&s.token),
            "{name}: the token is never shown"
        );
    }
    drop(s.server);
}

#[test]
fn a_bad_client_file_is_refused_naming_its_key() {
    let s = setup();
    let key_file = s.dir.path().join("agent.token");
    std::fs::write(&key_file, &s.token).expect("write");
    let cases = [
        (
            "unknown",
            format!(
                "url = \"https://localhost/mcp\"\nkey_file = \"{}\"\ntoken = \"x\"\n",
                key_file.display()
            ),
            "token",
        ),
        (
            "no url",
            format!("key_file = \"{}\"\n", key_file.display()),
            "url",
        ),
        (
            "plain",
            format!(
                "url = \"http://localhost/mcp\"\nkey_file = \"{}\"\n",
                key_file.display()
            ),
            "url",
        ),
        (
            "no key file",
            "url = \"https://localhost/mcp\"\nkey_file = \"/nonexistent/vcon.token\"\n".to_owned(),
            "key_file",
        ),
    ];
    for (name, text, named) in cases {
        let file = s
            .dir
            .path()
            .join(format!("{}.toml", name.replace(' ', "-")));
        std::fs::write(&file, text).expect("write");
        let output = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .arg("mcp")
            .arg("--config")
            .arg(&file)
            .stdin(Stdio::null())
            .output()
            .expect("runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{name}: {stderr}");
        assert!(stderr.contains(named), "{name}: {stderr}");
    }
    drop(s.server);
}
