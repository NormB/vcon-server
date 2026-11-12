//! `vcon-server serve`, run as its own process: it starts from its
//! configuration, stores and reads, holds its memory locked and out of core
//! dumps, and stops on SIGTERM. Each way it refuses to start is tried too.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use time::OffsetDateTime;

use vcon_server::access::{Scope, Scopes};
use vcon_server::audit::Actor;
use vcon_server::clock::{Clock, SystemClock};
use vcon_server::envelope::Key;
use vcon_server::fs::OsFs;
use vcon_server::ids::{KeyId, TenantId, VersionId};
use vcon_server::retention::Retention;
use vcon_server::store::Caller;
use vcon_server::tenants::Tenants;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

/// A data directory with one tenant and a key for it, a certificate and a
/// configuration file, ready for `serve`. The configuration listens on
/// port 0, so each server takes a port of its own when it binds, and
/// `start` reads that port from the `serving` line.
struct Setup {
    dir: tempfile::TempDir,
    config: PathBuf,
    master_key: PathBuf,
    tls_key: PathBuf,
    pem: String,
    token: String,
    data: PathBuf,
    tenant: TenantId,
}

/// A clock a fixed time behind the system's.
struct Behind(Duration);

impl Clock for Behind {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc() - self.0
    }
}

fn random_key_file(path: &std::path::Path) {
    let mut bytes = [0u8; 32];
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .expect("urandom")
        .read_exact(&mut bytes)
        .expect("32 random bytes");
    std::fs::write(path, bytes).expect("write the key file");
    chmod(path, 0o600);
}

fn chmod(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

fn setup() -> Setup {
    setup_on(Arc::new(SystemClock))
}

/// `setup`, with the tenant and its key made at `clock`'s time.
fn setup_on(clock: Arc<dyn Clock>) -> Setup {
    let dir = tempfile::tempdir().expect("temp dir");
    let data = dir.path().join("data");
    std::fs::create_dir(&data).expect("data dir");
    let master_key = dir.path().join("master.key");
    random_key_file(&master_key);
    let tenants = Tenants::open(
        &data,
        Key::load_master(&master_key).expect("the master key"),
        Arc::new(OsFs),
        clock,
    )
    .expect("opens");
    let tenant = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let token = tenants
        .create_key(
            Actor::Cli,
            Some(tenant),
            Scopes::of(&[Scope::Write, Scope::Read]),
            "test",
        )
        .expect("a key")
        .token;
    drop(tenants);

    let made =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate");
    let pem = made.cert.pem();
    let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    std::fs::write(&cert, &pem).expect("write");
    std::fs::write(&key, made.signing_key.serialize_pem()).expect("write");
    chmod(&key, 0o600);
    let config = dir.path().join("vcon-server.toml");
    std::fs::write(
        &config,
        format!(
            r#"listen = "127.0.0.1:0"
data_dir = "{data}"
master_key = "{master}"
purge_interval = 3600
rotation_warn = 2592000

[tls]
cert = "{cert}"
key = "{key}"

[limits]
body = 10485760
inflated = 104857600

[limits.timeouts]
read = 30
"#,
            data = data.display(),
            master = master_key.display(),
            cert = cert.display(),
            key = key.display(),
        ),
    )
    .expect("write the configuration");
    Setup {
        dir,
        config,
        master_key,
        tls_key: key,
        pem,
        token,
        data,
        tenant,
    }
}

fn serve(s: &Setup) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vcon-server"));
    command.arg("serve").arg("--config").arg(&s.config);
    command
}

/// Runs a `serve` that is expected to refuse to start.
fn refused(mut command: Command) -> Output {
    let mut running = Running(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("starts"),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while running.0.try_wait().expect("wait").is_none() {
        assert!(Instant::now() < deadline, "serve did not refuse to start");
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut stderr = Vec::new();
    use std::io::Read;
    running
        .0
        .stderr
        .take()
        .expect("piped")
        .read_to_end(&mut stderr)
        .expect("read stderr");
    let output = Output {
        status: running.0.wait().expect("wait"),
        stdout: Vec::new(),
        stderr,
    };
    assert!(!output.status.success(), "it refused: {output:?}");
    output
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A running server, killed when dropped, so a failed assertion leaves no
/// server behind.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// A server that has said where it listens: the address its `serving`
/// line names, and its stderr, line by line, including the lines it wrote
/// before that one.
struct Started {
    running: Running,
    addr: SocketAddr,
    early: VecDeque<String>,
    lines: Receiver<String>,
}

/// Starts `command` with its stderr piped, and waits for its `serving`
/// line.
fn start(mut command: Command) -> Started {
    let mut running = Running(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("starts"),
    );
    let stderr = running.0.stderr.take().expect("piped");
    let (line_tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut early = VecDeque::new();
    let addr = loop {
        match lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => match bound(&line) {
                Some(addr) => break addr,
                None => early.push_back(line),
            },
            Err(_) => panic!(
                "serve never said where it listens ({:?}): {early:?}",
                running.0.try_wait()
            ),
        }
    };
    assert_ne!(
        addr.port(),
        0,
        "the serving line names the port bound, not the port configured"
    );
    Started {
        running,
        addr,
        early,
        lines,
    }
}

impl Started {
    /// The next stderr line that contains `pattern`, if one comes within
    /// `within`.
    fn line_with(&mut self, pattern: &str, within: Duration) -> Option<String> {
        while let Some(line) = self.early.pop_front() {
            if line.contains(pattern) {
                return Some(line);
            }
        }
        let deadline = Instant::now() + within;
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .ok()?;
            if line.contains(pattern) {
                return Some(line);
            }
        }
    }

    /// A client that trusts `s`'s certificate and reaches this server as
    /// localhost.
    fn client(&self, s: &Setup) -> reqwest::Client {
        reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(s.pem.as_bytes()).expect("pem")])
            .resolve("localhost", self.addr)
            .no_proxy()
            .build()
            .expect("a client")
    }

    /// The URL of `path` on this server.
    fn url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.addr.port())
    }
}

/// The address a `serving` line names, from its `listen=` field.
fn bound(line: &str) -> Option<SocketAddr> {
    let mut fields = line.split_whitespace();
    fields.find(|f| *f == "serving")?;
    fields.find_map(|f| f.strip_prefix("listen="))?.parse().ok()
}

/// A field of a process's status, in kB.
fn status_kb(pid: u32, field: &str) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .expect("read status")
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix(field)?.strip_prefix(':')?;
            value.trim().strip_suffix(" kB")?.parse().ok()
        })
        .unwrap_or_else(|| panic!("{field}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn mlock_serve_stores_reads_keeps_its_memory_locked_and_stops_on_sigterm() {
    let s = setup();
    let mut started = start(serve(&s));
    let pid = started.running.0.id();

    let client = started.client(&s);
    let url = |path: &str| started.url(path);
    let body = std::fs::read(format!(
        "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4");
    let stored = client
        .post(url("/vcons"))
        .bearer_auth(&s.token)
        .header("content-type", "application/vcon")
        .body(body.clone())
        .send()
        .await
        .expect("an answer");
    assert_eq!(stored.status(), 201);
    let read = client
        .get(url(&format!("/vcons/{UUID}")))
        .bearer_auth(&s.token)
        .send()
        .await
        .expect("an answer");
    assert_eq!(read.status(), 200);
    assert_eq!(read.bytes().await.expect("body").as_ref(), body.as_slice());

    // A process that is not dumpable has its /proc files owned by root
    // (proc_pid(5)); this test does not run as root, so the owner tells.
    // SAFETY: getuid has no preconditions.
    assert_ne!(unsafe { libc::getuid() }, 0, "the test runs as a user");
    let owner = std::fs::metadata(format!("/proc/{pid}/status"))
        .expect("status")
        .uid();
    assert_eq!(owner, 0, "the server is not dumpable");
    let (locked, resident) = (status_kb(pid, "VmLck"), status_kb(pid, "VmRSS"));
    assert!(
        locked >= resident,
        "{locked} kB locked of {resident} kB resident"
    );

    let second = refused(serve(&s));
    assert!(
        stderr(&second).contains("data directory"),
        "a second server on the same data: {}",
        stderr(&second)
    );

    // SAFETY: the pid is our running child's.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) }, 0);
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = started.running.0.try_wait().expect("wait") {
            break status;
        }
        assert!(Instant::now() < deadline, "serve did not stop on SIGTERM");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "it stopped cleanly: {status:?}");
}

/// Starts a server and sends it `signal` as soon as its serving line is
/// read, `rounds` times over; each must stop as it does on a signal that
/// comes later: cleanly, with exit status 0.
fn stops_cleanly_on_a_signal_at_the_serving_line(signal: libc::c_int, rounds: usize) {
    for round in 0..rounds {
        let s = setup();
        let mut started = start(serve(&s));
        let pid = started.running.0.id();
        // SAFETY: the pid is our running child's.
        let sent = unsafe { libc::kill(pid as libc::pid_t, signal) };
        assert_eq!(sent, 0, "the signal was sent");
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = started.running.0.try_wait().expect("wait") {
                break status;
            }
            assert!(Instant::now() < deadline, "serve did not stop");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(
            status.success(),
            "round {round}: a signal at the serving line is an orderly stop, not {status:?}"
        );
    }
}

/// The serving line says the server can be stopped as well as reached: a
/// supervisor that stops it the moment it reads the line gets an orderly
/// stop.
#[test]
fn a_sigterm_at_the_serving_line_stops_cleanly() {
    stops_cleanly_on_a_signal_at_the_serving_line(libc::SIGTERM, 8);
}

#[test]
fn a_sigint_at_the_serving_line_stops_cleanly() {
    stops_cleanly_on_a_signal_at_the_serving_line(libc::SIGINT, 8);
}

/// The connection limits come from the configuration: with a handshake
/// limit of one second, a connection that sends nothing is closed after
/// about a second, where the default keeps it ten.
#[test]
fn serve_closes_a_silent_connection_at_the_configured_handshake_limit() {
    use std::io::Read;
    let s = setup();
    let text = std::fs::read_to_string(&s.config)
        .expect("read")
        .replace("read = 30\n", "read = 30\nhandshake = 1\n");
    assert!(text.contains("handshake = 1"), "the key was written");
    std::fs::write(&s.config, text).expect("write");
    let started = start(serve(&s));
    let mut silent = std::net::TcpStream::connect(started.addr).expect("connects");
    silent
        .set_read_timeout(Some(Duration::from_secs(8)))
        .expect("a read limit");
    let begun = Instant::now();
    let mut sink = [0; 16];
    // Closed is a read of nothing, or a reset; still open is a timeout.
    let closed = match silent.read(&mut sink) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => !matches!(
            e.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ),
    };
    assert!(
        closed,
        "closed within 8 seconds, before the default's 10: {:?}",
        begun.elapsed()
    );
    assert!(
        begun.elapsed() >= Duration::from_millis(900),
        "not before the limit: {:?}",
        begun.elapsed()
    );
}

/// The most connections at once comes from the configuration: with one
/// place, a silent connection holds it, and a request is answered only
/// once the handshake limit of two seconds has closed that connection.
#[tokio::test(flavor = "multi_thread")]
async fn serve_holds_as_many_connections_at_once_as_the_configuration_says() {
    let s = setup();
    let text = std::fs::read_to_string(&s.config)
        .expect("read")
        .replace("[limits]\n", "[limits]\nconnections = 1\n")
        .replace("read = 30\n", "read = 30\nhandshake = 2\n");
    assert!(text.contains("connections = 1"), "the key was written");
    assert!(text.contains("handshake = 2"), "the key was written");
    std::fs::write(&s.config, text).expect("write");
    let started = start(serve(&s));
    let _silent = std::net::TcpStream::connect(started.addr).expect("connects");
    // Time for the listener to take the silent connection.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let begun = Instant::now();
    let answer = started
        .client(&s)
        .get(started.url("/key"))
        .bearer_auth(&s.token)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .expect("an answer once the place is free");
    assert_eq!(answer.status(), 200, "served");
    assert!(
        begun.elapsed() >= Duration::from_millis(1200),
        "it waited for the one place: {:?}",
        begun.elapsed()
    );
}

/// `setup`, with `limits` written at the head of the `[limits]` table and
/// `timeouts` at the head of `[limits.timeouts]`.
fn setup_with_limits(limits: &str, timeouts: &str) -> Setup {
    let s = setup();
    let text = std::fs::read_to_string(&s.config)
        .expect("read")
        .replace("[limits]\n", &format!("[limits]\n{limits}"))
        .replace("read = 30\n", &format!("read = 30\n{timeouts}"));
    assert!(text.contains(limits), "the [limits] keys were written");
    assert!(text.contains(timeouts), "the timeouts were written");
    std::fs::write(&s.config, text).expect("write");
    s
}

/// The idle limit comes from the configuration. With one place and an
/// idle limit of one second, a client that keeps its connection after its
/// answer holds the place, and a second client is answered once the idle
/// limit has closed that connection: well inside the 15 seconds it waits,
/// where the default of 120 seconds would leave it without an answer.
async fn an_idle_connection_gives_its_place_up(http1: bool) {
    let s = setup_with_limits("connections = 1\n", "idle = 1\n");
    let started = start(serve(&s));
    let client = |http1: bool| {
        let builder = reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(s.pem.as_bytes()).expect("pem")])
            .resolve("localhost", started.addr)
            .no_proxy()
            .timeout(Duration::from_secs(15));
        if http1 { builder.http1_only() } else { builder }
            .build()
            .expect("a client")
    };
    let keeps = client(http1);
    let first = keeps
        .get(started.url("/key"))
        .bearer_auth(&s.token)
        .send()
        .await
        .expect("an answer");
    assert_eq!(first.status(), 200, "the first client is served");
    let version = first.version();
    assert_eq!(
        version == reqwest::Version::HTTP_11,
        http1,
        "the protocol asked for: {version:?}"
    );
    drop(first.bytes().await);
    // `keeps` is alive, and its connection with it.
    let begun = Instant::now();
    let second = client(http1)
        .get(started.url("/key"))
        .bearer_auth(&s.token)
        .send()
        .await
        .expect("an answer once the idle connection was closed");
    assert_eq!(second.status(), 200, "the second client is served");
    assert!(
        begun.elapsed() >= Duration::from_millis(500),
        "it waited for the place the idle connection held: {:?}",
        begun.elapsed()
    );
    // The client whose connection was closed opens another and is served.
    let again = keeps
        .get(started.url("/key"))
        .bearer_auth(&s.token)
        .send()
        .await
        .expect("an answer on a new connection");
    assert_eq!(again.status(), 200, "served after the close");
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_closes_an_idle_http2_connection_at_the_configured_idle_limit() {
    an_idle_connection_gives_its_place_up(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn serve_closes_an_idle_http1_connection_at_the_configured_idle_limit() {
    an_idle_connection_gives_its_place_up(true).await;
}

/// The `listener_full` records of the admin log, as `audit export` writes
/// them beside the running server.
fn listener_full_records(s: &Setup) -> Vec<serde_json::Value> {
    String::from_utf8(beside(s, &["audit", "export", "--admin"]))
        .expect("text")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSON"))
        .filter(|record| record["action"] == "listener_full")
        .collect()
}

/// [`listener_full_records`], once they say `listener` was full `times`
/// times or more, or as they are after ten seconds: the server writes a
/// count off the listener's task, a moment after the listener filled.
fn listener_full_records_at(s: &Setup, listener: &str, times: u64) -> Vec<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let records = listener_full_records(s);
        if times_full(&records, listener) >= times || Instant::now() >= deadline {
            return records;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// How many times the records say `listener` was full.
fn times_full(records: &[serde_json::Value], listener: &str) -> u64 {
    records
        .iter()
        .filter(|record| record["target"] == listener)
        .map(|record| record["count"].as_u64().expect("a count"))
        .sum()
}

/// A full listener can be seen: one warning in the log, however often it
/// fills inside a minute, and each time counted in the admin log.
#[tokio::test(flavor = "multi_thread")]
async fn serve_logs_a_full_listener_once_and_counts_each_time() {
    let s = setup_with_limits("connections = 1\n", "handshake = 1\n");
    let mut started = start(serve(&s));
    assert_eq!(
        listener_full_records(&s),
        Vec::<serde_json::Value>::new(),
        "nothing before a connection"
    );
    // One connection fills a listener of one place.
    let silent = std::net::TcpStream::connect(started.addr).expect("connects");
    let line = started
        .line_with("the listener is full", Duration::from_secs(10))
        .expect("a warning that the listener is full");
    assert!(
        line.contains(
            " WARN vcon_server::api::listen: the listener is full: it holds \
             limits.connections connections and takes no more until one ends listener="
        ),
        "the line the configuration page shows: {line}"
    );
    assert!(
        line.ends_with(" connections=1"),
        "the limit comes last: {line}"
    );
    assert!(line.contains("connections=1"), "the limit: {line}");
    assert!(
        line.contains(&format!("listener={}", started.addr)),
        "the listener's address: {line}"
    );
    assert!(
        !line.contains(&silent.local_addr().expect("an address").to_string()),
        "no caller's address: {line}"
    );
    drop(silent);
    // Each request is a connection that fills the listener again.
    for n in 0..3 {
        let answer = started
            .client(&s)
            .get(started.url("/key"))
            .bearer_auth(&s.token)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 200, "request {n}");
    }
    let records = listener_full_records_at(&s, "listen", 4);
    assert!(
        times_full(&records, "listen") >= 4,
        "the silent connection and three requests: {records:?}"
    );
    for record in &records {
        assert_eq!(record["actor"], "", "no caller: {record}");
        assert!(
            record["status"].is_null(),
            "no status, since no request was answered: {record}"
        );
        assert_eq!(record["target"], "listen", "the listener's key: {record}");
    }
    assert_eq!(
        started.line_with("the listener is full", Duration::from_secs(1)),
        None,
        "one line a minute, though it filled four times"
    );
}

/// A server whose listener is never full says nothing of it.
#[tokio::test(flavor = "multi_thread")]
async fn serve_neither_logs_nor_counts_a_listener_that_is_not_full() {
    let s = setup();
    let mut started = start(serve(&s));
    for n in 0..3 {
        let answer = started
            .client(&s)
            .get(started.url("/key"))
            .bearer_auth(&s.token)
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 200, "request {n}");
    }
    assert_eq!(
        listener_full_records(&s),
        Vec::<serde_json::Value>::new(),
        "512 places, three connections"
    );
    assert_eq!(
        started.line_with("the listener is full", Duration::from_secs(1)),
        None,
        "no warning"
    );
}

/// A key of the tenant's administrator, made while no server runs.
fn administrator(s: &Setup) -> String {
    let tenants = Tenants::open(
        &s.data,
        Key::load_master(&s.master_key).expect("the master key"),
        Arc::new(OsFs),
        Arc::new(SystemClock),
    )
    .expect("opens");
    tenants
        .create_key(
            Actor::Cli,
            Some(s.tenant),
            Scopes::of(&[Scope::TenantAdmin]),
            "admin",
        )
        .expect("a key")
        .token
}

/// The status a served `POST /tenant/keys` answers to a body that asks
/// for a key with no expiry.
async fn no_expiry_status(s: &Setup, admin: &str) -> u16 {
    let started = start(serve(s));
    started
        .client(s)
        .post(started.url("/tenant/keys"))
        .bearer_auth(admin)
        .json(&serde_json::json!({"scopes": ["read"], "label": "forever", "no_expiry": true}))
        .send()
        .await
        .expect("an answer")
        .status()
        .as_u16()
}

/// `keys.no_expiry` reaches the served API: without the key a key with no
/// expiry is made, and with `false` the same request is refused.
#[tokio::test(flavor = "multi_thread")]
async fn serve_refuses_a_key_with_no_expiry_only_where_the_configuration_says_so() {
    let s = setup();
    let admin = administrator(&s);
    assert_eq!(no_expiry_status(&s, &admin).await, 201, "by default");
    let mut text = std::fs::read_to_string(&s.config).expect("the configuration");
    text.push_str("\n[keys]\nno_expiry = false\n");
    std::fs::write(&s.config, text).expect("written");
    assert_eq!(
        no_expiry_status(&s, &admin).await,
        422,
        "with keys.no_expiry = false"
    );
}

/// A key made before keys had an expiry has none in server.sqlite, and a
/// served request by it is accepted and leaves its last-used time.
#[tokio::test(flavor = "multi_thread")]
async fn serve_accepts_a_key_with_no_expiry_and_writes_when_it_was_last_used() {
    let s = setup();
    let conn = server_sqlite(&s);
    conn.execute("UPDATE api_key SET expires_at = NULL", [])
        .expect("the key as release 0.4.3 left it");
    drop(conn);
    let started = start(serve(&s));
    let answer = started
        .client(&s)
        .get(started.url("/key"))
        .bearer_auth(&s.token)
        .send()
        .await
        .expect("an answer");
    assert_eq!(answer.status(), 200, "a key with no expiry is accepted");
    drop(started);
    let used: Option<i64> = server_sqlite(&s)
        .query_row("SELECT last_used_at FROM api_key", [], |r| r.get(0))
        .expect("the key's row");
    let used = used.expect("a last-used time");
    assert_eq!(used % 60_000_000_000, 0, "the start of a minute");
}

#[test]
fn a_wrong_master_key_stops_the_start() {
    let s = setup();
    random_key_file(&s.master_key);
    let output = refused(serve(&s));
    // Not "the master key cannot be read", which is a key file that is
    // missing or of the wrong size.
    assert!(
        stderr(&output).contains("the master key is not the one server.sqlite was made under"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_master_key_other_users_may_read_stops_the_start_and_names_the_fix() {
    let s = setup();
    chmod(&s.master_key, 0o644);
    let text = stderr(&refused(serve(&s)));
    let path = s.master_key.display();
    assert!(
        text.contains(&format!("the master key cannot be read: master_key {path}: other users may read or change the file (mode 0644); run `chmod 600 {path}`")),
        "{text}"
    );
}

#[test]
fn a_tls_key_other_users_may_read_stops_the_start_and_names_the_fix() {
    let s = setup();
    chmod(&s.tls_key, 0o644);
    let text = stderr(&refused(serve(&s)));
    let path = s.tls_key.display();
    assert!(
        text.contains(&format!("the TLS certificate or key cannot be used: tls.key {path}: other users may read or change the file (mode 0644); run `chmod 600 {path}`")),
        "{text}"
    );
}

#[test]
fn a_tls_key_its_group_may_read_serves() {
    let s = setup();
    chmod(&s.tls_key, 0o640);
    let _running = start(serve(&s));
}

#[test]
fn a_configuration_error_stops_the_start_and_names_the_key() {
    let s = setup();
    let text = std::fs::read_to_string(&s.config).expect("read");
    let without_listen: String = text
        .lines()
        .filter(|l| !l.starts_with("listen"))
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&s.config, without_listen).expect("write");
    let output = refused(serve(&s));
    assert!(stderr(&output).contains("`listen`"), "{}", stderr(&output));
}

#[test]
fn mlock_memory_that_cannot_be_locked_stops_the_start() {
    let s = setup();
    let mut command = serve(&s);
    let limit_locked_memory = || {
        let small = libc::rlimit {
            rlim_cur: 64 * 1024,
            rlim_max: 64 * 1024,
        };
        // SAFETY: `small` is a valid rlimit that outlives the call, and the
        // limit it sets is the calling process's own.
        if unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &small) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    };
    // SAFETY: the closure runs in the child between fork and exec, where
    // only async-signal-safe calls may be made; its one call, setrlimit, is.
    unsafe {
        command.pre_exec(limit_locked_memory);
    }
    let output = refused(command);
    // Not "the data directory cannot be locked", the other refusal that
    // speaks of a lock. A limit above zero but too small is ENOMEM
    // (mlockall(2)), the line getting-started.md shows.
    assert!(
        stderr(&output).contains(
            "vcon-server: memory cannot be locked: Cannot allocate memory (os error 12); \
             allow it, for example LimitMEMLOCK=infinity"
        ),
        "{}",
        stderr(&output)
    );
    let _ = &s.dir;
}

/// server.sqlite opened directly, to put the registry in a state no
/// command would leave it in.
fn server_sqlite(s: &Setup) -> rusqlite::Connection {
    let master = Key::load_master(&s.master_key).expect("the master key");
    vcon_server::catalog::open(
        &s.data.join("server.sqlite"),
        &vcon_server::envelope::catalog_key(&master).expect("catalog key"),
        "",
    )
    .expect("opens")
}

#[test]
fn serve_refuses_while_a_key_rotation_is_stopped_part_way() {
    let s = setup();
    server_sqlite(&s)
        .execute(
            "UPDATE tenant SET pending_wrapped_tenant_key = wrapped_tenant_key",
            [],
        )
        .expect("a pending key");
    let output = refused(serve(&s));
    assert!(
        stderr(&output).contains("key rotation"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("vcon-server check"),
        "{}",
        stderr(&output)
    );
    assert!(
        !stderr(&output).contains(&s.tenant.to_string()),
        "no tenant id in the log"
    );
}

#[test]
fn serve_refuses_a_tenant_key_wrapped_under_another_key() {
    let s = setup();
    server_sqlite(&s)
        .execute("UPDATE tenant SET wrapped_tenant_key = randomblob(61)", [])
        .expect("a foreign wrap");
    let output = refused(serve(&s));
    assert_eq!(
        stderr(&output),
        "vcon-server: 1 tenant key(s) do not unwrap under the master key; \
         vcon-server check names the tenants\n",
        "the one tenant key wrapped under another key is counted, and no tenant is named"
    );
}

#[test]
fn serve_warns_when_key_rotation_is_overdue() {
    // The tenant and its key made 32 days ago, and a purge 31 days ago:
    // older than rotation_warn (30 days), with no rotation since.
    const DAY: Duration = Duration::from_secs(86_400);
    let s = setup_on(Arc::new(Behind(32 * DAY)));
    assert_eq!(
        std::fs::read_to_string(&s.config)
            .expect("configuration")
            .matches("rotation_warn = 2592000")
            .count(),
        1,
        "rotation_warn is 30 days"
    );
    let tenants = registry_on(&s, Arc::new(Behind(31 * DAY)));
    let store = tenants.store(s.tenant).expect("its store");
    let body = std::fs::read(format!(
        "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4");
    let found = vcon::read::inspect(
        &body,
        Some("application/vcon"),
        &vcon::read::Limits::new(1 << 20),
        &Default::default(),
    )
    .expect("a valid vCon");
    let caller = Caller { key: KeyId([7; 8]) };
    store.create(&caller, &found, &body).expect("stored");
    store
        .delete(&caller, found.uuid, Retention::Days(0))
        .expect("deleted and purged");
    drop(store);
    drop(tenants);
    let warned = start(serve(&s))
        .line_with("key rotation overdue", Duration::from_secs(20))
        .expect("serve warned of no overdue rotation");
    assert!(warned.contains("WARN"), "{warned}");
    let fields: Vec<&str> = warned.split_whitespace().collect();
    for field in ["tenants_with_purges=1", "purges=1", "removals=0"] {
        assert!(fields.contains(&field), "{field}: {warned}");
    }
    assert!(
        !warned.contains(&s.tenant.to_string()),
        "no tenant id: {warned}"
    );
}

/// The registry of `s`'s data directory, opened as `serve` opens it.
fn registry(s: &Setup) -> Tenants {
    registry_on(s, Arc::new(SystemClock))
}

/// `registry`, on `clock`.
fn registry_on(s: &Setup, clock: Arc<dyn Clock>) -> Tenants {
    Tenants::open(
        &s.data,
        Key::load_master(&s.master_key).expect("the master key"),
        Arc::new(OsFs),
        clock,
    )
    .expect("opens")
}

/// The objects a crash leaves (a temporary file, and an object no row
/// names) are removed at start, and the server logs how many, with no
/// tenant and no name, so an operator can see that recovery removed
/// something.
#[test]
fn serve_logs_how_many_objects_start_up_recovery_removes() {
    let s = setup();
    let objects = s
        .data
        .join("tenants")
        .join(s.tenant.to_string())
        .join("objects");
    std::fs::create_dir_all(&objects).expect("the objects directory");
    let orphan = VersionId::generate().to_string();
    let temporary = format!("{}.tmp", VersionId::generate());
    for name in [&orphan, &temporary] {
        std::fs::write(objects.join(name), b"left by a crash").expect("plants a leftover");
    }
    let mut started = start(serve(&s));
    let line = started
        .line_with("start-up recovery removed", Duration::from_secs(20))
        .expect("serve logged no start-up recovery removal");
    assert!(
        line.contains("removed=2"),
        "both leftovers are counted: {line}"
    );
    assert!(
        !line.contains(&s.tenant.to_string()) && !line.contains(&orphan),
        "the line names no tenant and no object: {line}"
    );
    let left: Vec<_> = std::fs::read_dir(&objects)
        .expect("reads")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert!(left.is_empty(), "the leftovers are gone: {left:?}");
}

#[test]
fn serve_purges_what_is_due_when_it_starts() {
    let s = setup();
    // A vCon deleted while seven years applied, then the tenant's period set
    // to zero: due, and waiting for a sweep.
    let tenants = registry(&s);
    let store = tenants.store(s.tenant).expect("its store");
    let body = std::fs::read(format!(
        "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4");
    let found = vcon::read::inspect(
        &body,
        Some("application/vcon"),
        &vcon::read::Limits::new(1 << 20),
        &Default::default(),
    )
    .expect("a valid vCon");
    let caller = Caller { key: KeyId([7; 8]) };
    store.create(&caller, &found, &body).expect("stored");
    store
        .delete(&caller, found.uuid, Retention::Years(7))
        .expect("deleted");
    tenants
        .set_retention(Actor::Cli, s.tenant, Some(Retention::Days(0)))
        .expect("set");
    drop(store);
    drop(tenants);

    let mut started = start(serve(&s));
    let swept = started
        .line_with("purge sweep", Duration::from_secs(20))
        .expect("serve logged no purge sweep");
    assert!(swept.contains("purged=1"), "{swept}");
    assert!(swept.contains("failed=0"), "{swept}");
    assert!(
        !swept.contains(&s.tenant.to_string()),
        "a sweep logs counts, never a tenant: {swept}"
    );
    assert!(
        started
            .line_with("purge sweep", Duration::from_secs(3))
            .is_none(),
        "no second sweep long before purge_interval (an hour here)"
    );
    drop(started);

    let tenants = registry(&s);
    let counts = tenants
        .store(s.tenant)
        .expect("its store")
        .counts()
        .expect("counts");
    assert_eq!(counts.versions, 0, "the due version is gone");
}

#[test]
fn serve_sweeps_again_once_purge_interval_has_passed() {
    let s = setup();
    let text = std::fs::read_to_string(&s.config).expect("read");
    assert_eq!(text.matches("purge_interval = 3600").count(), 1);
    std::fs::write(
        &s.config,
        text.replace("purge_interval = 3600", "purge_interval = 1"),
    )
    .expect("write");
    let mut started = start(serve(&s));
    let first = started
        .line_with("purge sweep", Duration::from_secs(20))
        .expect("a sweep at start");
    let second = started
        .line_with("purge sweep", Duration::from_secs(10))
        .expect("a second sweep after purge_interval");
    // Each line is stamped when the sweep is logged, at its end; the next
    // sweep starts purge_interval after that.
    let gap = logged_at(&second) - logged_at(&first);
    assert!(
        gap >= time::Duration::SECOND,
        "not before purge_interval: {gap}\n{first}\n{second}"
    );
}

/// The time a log line was written, from the timestamp it starts with.
fn logged_at(line: &str) -> OffsetDateTime {
    let stamp = line.split_whitespace().next().expect("a timestamp");
    OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|e| panic!("{e}: {line}"))
}

/// Runs a command beside the server and returns what it printed.
fn beside(s: &Setup, args: &[&str]) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .args(args)
        .arg("--config")
        .arg(&s.config)
        .output()
        .expect("runs");
    assert!(output.status.success(), "{args:?}: {output:?}");
    output.stdout
}

fn token_of(printed: &[u8]) -> String {
    let made: serde_json::Value = serde_json::from_slice(printed).expect("JSON");
    made["token"].as_str().expect("a token").to_owned()
}

/// Each audit route answers exactly the lines `audit export` writes for
/// the same records, read from the running server and the command line.
#[tokio::test(flavor = "multi_thread")]
async fn the_audit_routes_answer_what_audit_export_writes() {
    let s = setup();
    let started = start(serve(&s));
    let client = started.client(&s);
    let url = |path: &str| started.url(path);
    let tenant = s.tenant.to_string();
    let body = std::fs::read(format!(
        "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4");
    let stored = client
        .post(url("/vcons"))
        .bearer_auth(&s.token)
        .header("content-type", "application/vcon")
        .body(body)
        .send()
        .await
        .expect("an answer");
    assert_eq!(stored.status(), 201);
    let auditor = token_of(&beside(
        &s,
        &[
            "key", "create", "--tenant", &tenant, "--scope", "audit", "--label", "a",
        ],
    ));
    let operator = token_of(&beside(
        &s,
        &["key", "create", "--operator", "--label", "o"],
    ));

    for (path, token, export) in [
        (
            "/audit?after=1",
            &auditor,
            vec!["audit", "export", "--tenant", &tenant, "--since", "1"],
        ),
        (
            "/admin/audit",
            &operator,
            vec!["audit", "export", "--admin"],
        ),
    ] {
        let answer = client
            .get(url(path))
            .bearer_auth(token)
            .send()
            .await
            .expect("an answer");
        assert_eq!(answer.status(), 200, "{path}");
        assert_eq!(answer.headers()["content-type"], "application/jsonl");
        let page = answer.bytes().await.expect("body").to_vec();
        assert!(!page.is_empty(), "{path}");
        // The export runs after the read, so it also holds the read's own
        // record; the page is every line before it.
        let exported = beside(&s, &export);
        assert_eq!(page, exported[..page.len()], "{path}");
        let rest = String::from_utf8(exported[page.len()..].to_vec()).expect("UTF-8");
        assert_eq!(rest.lines().count(), 1, "{path}: {rest}");
        assert!(rest.contains("\"action\":\"audit_read\""), "{path}: {rest}");
    }
}

/// A backup taken beside a running server while vCons are being stored is
/// whole: `check` on it finds no problem, and a server started on it serves
/// what was stored before it.
#[tokio::test(flavor = "multi_thread")]
async fn a_backup_taken_under_write_load_checks_whole_and_serves() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    let s = setup();
    let started = start(serve(&s));
    let client = started.client(&s);
    let first = std::fs::read(format!(
        "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4");
    let port = started.addr.port();
    let stored = client
        .post(format!("https://localhost:{port}/vcons"))
        .bearer_auth(&s.token)
        .header("content-type", "application/vcon")
        .body(first.clone())
        .send()
        .await
        .expect("an answer");
    assert_eq!(stored.status(), 201);

    // Writes go on for the whole backup.
    let stop = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicU64::new(0));
    let load = {
        let (client, token, stop, written) = (
            client.clone(),
            s.token.clone(),
            stop.clone(),
            written.clone(),
        );
        tokio::spawn(async move {
            let mut n = 0u64;
            while !stop.load(Ordering::SeqCst) {
                let body = format!(
                    r#"{{"uuid": "01a07da8-c2bb-83e5-b9a2-{n:012x}", "created_at": "2026-10-23T12:00:00Z", "parties": []}}"#
                );
                let answer = client
                    .post(format!("https://localhost:{port}/vcons"))
                    .bearer_auth(&token)
                    .header("content-type", "application/vcon")
                    .body(body)
                    .send()
                    .await
                    .expect("an answer");
                assert_eq!(answer.status(), 201);
                n += 1;
                written.store(n, Ordering::SeqCst);
            }
        })
    };
    while written.load(Ordering::SeqCst) < 5 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let before = written.load(Ordering::SeqCst);
    let to = s.dir.path().join("backup");
    let printed = {
        let (config, to) = (s.config.clone(), to.clone());
        tokio::task::spawn_blocking(move || {
            Command::new(env!("CARGO_BIN_EXE_vcon-server"))
                .arg("backup")
                .arg(&to)
                .arg("--config")
                .arg(&config)
                .output()
                .expect("runs")
        })
        .await
        .expect("joins")
    };
    let after = written.load(Ordering::SeqCst);
    stop.store(true, Ordering::SeqCst);
    load.await.expect("the load");
    assert!(printed.status.success(), "{printed:?}");
    assert!(after > before, "vCons were stored while the backup ran");
    let done: serde_json::Value = serde_json::from_slice(&printed.stdout).expect("a JSON line");
    assert_eq!(done["tenants"], 1);
    assert!(
        done["versions"].as_u64().expect("versions") > before,
        "{done}"
    );

    // The backup as a data directory of its own, on a port of its own.
    let text = std::fs::read_to_string(&s.config).expect("the configuration");
    let config = s.dir.path().join("backup.toml");
    std::fs::write(
        &config,
        text.replace(&s.data.display().to_string(), &to.display().to_string()),
    )
    .expect("write");
    let checked = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .arg("check")
        .arg("--config")
        .arg(&config)
        .output()
        .expect("runs");
    assert!(checked.status.success(), "{checked:?}");
    let lines: Vec<serde_json::Value> = String::from_utf8(checked.stdout)
        .expect("UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert_eq!(lines[0]["damaged"], serde_json::json!([]));
    assert_eq!(lines[0]["leftovers"], serde_json::json!([]));
    assert_eq!(lines[0]["versions"], done["versions"]);
    assert_eq!(lines.last().expect("a summary")["problems"], 0);

    let mut restore = Command::new(env!("CARGO_BIN_EXE_vcon-server"));
    restore.arg("serve").arg("--config").arg(&config);
    let restored = start(restore);
    assert_ne!(
        restored.addr, started.addr,
        "each server binds a port of its own"
    );
    let client = restored.client(&s);
    let read = client
        .get(restored.url(&format!("/vcons/{UUID}")))
        .bearer_auth(&s.token)
        .send()
        .await
        .expect("an answer");
    assert_eq!(read.status(), 200);
    assert_eq!(read.bytes().await.expect("body").to_vec(), first);
}

/// The PostgreSQL schema version `vcon-server migrate` leaves: the number
/// of migration files, `schema_v<n>.sql`, in the source. Read from the
/// files, so a test that states the version does not fall behind a new
/// migration: one that pinned the number failed only where a PostgreSQL
/// server was at hand.
fn postgres_schema_version() -> usize {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/backend/postgres");
    std::fs::read_dir(&dir)
        .expect("the PostgreSQL backend's source")
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.strip_prefix("schema_v")
                .and_then(|rest| rest.strip_suffix(".sql"))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .count()
}

#[test]
fn the_postgresql_schema_version_is_the_number_of_migration_files() {
    let version = postgres_schema_version();
    assert!(
        version >= 4,
        "schema_v1.sql to schema_v4.sql at least: {version}"
    );
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/backend/postgres");
    for n in 1..=version {
        assert!(
            dir.join(format!("schema_v{n}.sql")).is_file(),
            "the migrations are numbered from 1 with none left out: schema_v{n}.sql"
        );
    }
    assert!(
        !dir.join(format!("schema_v{}.sql", version + 1)).exists(),
        "and {version} is the last"
    );
}

/// Against the S3-compatible service the `VCON_TEST_S3_*` variables name
/// (see src/backend/s3.rs), with its keys given only in the AWS
/// environment variables, as the AWS default credential chain reads them.
#[cfg(feature = "backend-tests")]
mod s3 {
    use super::*;

    fn var(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| {
            panic!("{name} is not set: these tests need a real S3-compatible service")
        })
    }

    /// curl, signing its request for S3 with the test service's keys.
    pub(super) fn signed(method: &str, path: &str) -> String {
        let output = Command::new("curl")
            .args(["-sS", "--fail-with-body", "-X", method])
            .args(["--aws-sigv4", "aws:amz:us-east-1:s3", "--user"])
            .arg(format!(
                "{}:{}",
                var("VCON_TEST_S3_ACCESS_KEY"),
                var("VCON_TEST_S3_SECRET_KEY")
            ))
            .arg(format!("{}/{path}", var("VCON_TEST_S3_ENDPOINT")))
            .output()
            .expect("runs curl");
        assert!(output.status.success(), "{method} {path}: {output:?}");
        String::from_utf8(output.stdout).expect("UTF-8")
    }

    /// Every object name in `bucket`.
    fn names(bucket: &str) -> Vec<String> {
        signed("GET", &format!("{bucket}?list-type=2"))
            .split("<Key>")
            .skip(1)
            .filter_map(|rest| rest.split_once("</Key>").map(|(key, _)| key.to_owned()))
            .collect()
    }

    /// `s`'s configuration with its objects in `bucket`, and no keys.
    pub(super) fn in_bucket(s: &Setup, bucket: &str) {
        let mut text = std::fs::read_to_string(&s.config).expect("reads");
        text.push_str(&format!(
            "\n[storage]\nobjects = \"s3\"\n\n[storage.s3]\nendpoint = \"{}\"\nregion = \
             \"us-east-1\"\nbucket = \"{bucket}\"\npath_style = true\nallow_http = true\n",
            var("VCON_TEST_S3_ENDPOINT")
        ));
        std::fs::write(&s.config, text).expect("writes");
    }

    /// `serve`, with the test service's keys in the environment only.
    fn serve_with_keys(s: &Setup) -> Command {
        with_keys(serve(s))
    }

    /// `command`, with the test service's keys in the environment only.
    pub(super) fn with_keys(mut command: Command) -> Command {
        for name in ["AWS_PROFILE", "AWS_SESSION_TOKEN", "AWS_CONFIG_FILE"] {
            command.env_remove(name);
        }
        command
            .env("AWS_ACCESS_KEY_ID", var("VCON_TEST_S3_ACCESS_KEY"))
            .env("AWS_SECRET_ACCESS_KEY", var("VCON_TEST_S3_SECRET_KEY"))
            .env("AWS_SHARED_CREDENTIALS_FILE", "/nonexistent")
            .env("AWS_EC2_METADATA_DISABLED", "true");
        command
    }

    // No mlock_ prefix, though serve locks its memory: the prefix keeps a
    // test out of the AddressSanitizer job, which checks that each mlock_
    // test in the source is in its build, and that build never has the
    // backend-tests feature this test needs.
    #[tokio::test(flavor = "multi_thread")]
    async fn serve_keeps_its_objects_in_the_bucket_with_keys_from_the_environment() {
        let bucket = format!("vcon-serve-{}", &TenantId::generate().to_string()[..12]);
        signed("PUT", &bucket);
        let s = setup();
        in_bucket(&s, &bucket);
        let mut started = start(serve_with_keys(&s));
        let pid = started.running.0.id();
        let client = started.client(&s);
        let body = std::fs::read(format!(
            "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("A.4");
        let stored = client
            .post(started.url("/vcons"))
            .bearer_auth(&s.token)
            .header("content-type", "application/vcon")
            .body(body.clone())
            .send()
            .await
            .expect("an answer");
        assert_eq!(stored.status(), 201, "the vCon is stored");
        let read = client
            .get(started.url(&format!("/vcons/{UUID}")))
            .bearer_auth(&s.token)
            .send()
            .await
            .expect("an answer");
        assert_eq!(read.status(), 200, "the vCon is read");
        assert_eq!(
            read.bytes().await.expect("body").as_ref(),
            body.as_slice(),
            "it reads back byte for byte"
        );

        let mut kept = names(&bucket);
        kept.sort();
        let objects = format!("tenants/{}/objects/", s.tenant);
        assert_eq!(kept.len(), 2, "the claim and one version: {kept:?}");
        assert!(kept.iter().any(|k| k == "vcon-server.claim"), "{kept:?}");
        assert!(
            kept.iter().any(|k| k.starts_with(&objects)),
            "the version is under its tenant's name: {kept:?}"
        );
        let local = s
            .data
            .join("tenants")
            .join(s.tenant.to_string())
            .join("objects");
        let files = std::fs::read_dir(&local).map_or(0, Iterator::count);
        assert_eq!(files, 0, "no object file is written in the data directory");

        let other = setup();
        in_bucket(&other, &bucket);
        let refused = refused(serve_with_keys(&other));
        assert!(
            stderr(&refused).contains("another data directory"),
            "a second data directory on the same bucket and prefix: {}",
            stderr(&refused)
        );
        assert_eq!(
            names(&bucket).len(),
            2,
            "and nothing of the first is removed"
        );

        assert_eq!(
            // SAFETY: the pid is our running child's.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) },
            0,
            "the signal is sent"
        );
        let status = started.running.0.wait().expect("waits");
        assert!(status.success(), "it stopped cleanly: {status:?}");
        let mut logged: Vec<String> = started.early.drain(..).collect();
        logged.extend(started.lines.try_iter());
        let secret = var("VCON_TEST_S3_SECRET_KEY");
        assert!(!logged.is_empty(), "the server logged its start");
        for line in logged.iter().chain(std::iter::once(&stderr(&refused))) {
            assert!(!line.contains(&secret), "the secret is logged: {line}");
        }
    }

    /// With `[storage.parquet]`, the server's pack run moves a stored
    /// version from its object into a Parquet file in the bucket, logs the
    /// run, and the vCon still reads back byte for byte.
    #[tokio::test(flavor = "multi_thread")]
    async fn serve_packs_its_objects_into_parquet_files_in_the_bucket() {
        let bucket = format!("vcon-pack-{}", &TenantId::generate().to_string()[..12]);
        signed("PUT", &bucket);
        let s = setup();
        in_bucket(&s, &bucket);
        let mut text = std::fs::read_to_string(&s.config).expect("reads");
        text.push_str("\n[storage.parquet]\npack_interval = 1\n");
        std::fs::write(&s.config, text).expect("writes");
        let mut started = start(serve_with_keys(&s));
        let client = started.client(&s);
        let body = std::fs::read(format!(
            "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("A.4");
        let stored = client
            .post(started.url("/vcons"))
            .bearer_auth(&s.token)
            .header("content-type", "application/vcon")
            .body(body.clone())
            .send()
            .await
            .expect("an answer");
        assert_eq!(stored.status(), 201, "the vCon is stored");
        wait_until_packed(&s, &bucket);
        let read = client
            .get(started.url(&format!("/vcons/{UUID}")))
            .bearer_auth(&s.token)
            .send()
            .await
            .expect("an answer");
        assert_eq!(read.status(), 200, "the vCon is read");
        assert_eq!(
            read.bytes().await.expect("body").as_ref(),
            body.as_slice(),
            "it reads back byte for byte from its row"
        );
        // The run logs once it has finished, which may be after the bucket
        // shows its work.
        let mut logged: Vec<String> = started.early.drain(..).collect();
        let packed_line = |l: &String| l.contains("pack run") && l.contains("versions=1");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !logged.iter().any(packed_line) && std::time::Instant::now() < deadline {
            if let Ok(line) = started
                .lines
                .recv_timeout(std::time::Duration::from_millis(200))
            {
                logged.push(line);
            }
        }
        assert!(
            logged.iter().any(packed_line),
            "the run that packed it is logged: {logged:?}"
        );
    }

    /// Waits until the tenant's objects are all packed into Parquet files
    /// in `bucket`, for at most 30 seconds.
    fn wait_until_packed(s: &Setup, bucket: &str) {
        let objects = format!("tenants/{}/objects/", s.tenant);
        let files = format!("tenants/{}/parquet/date=", s.tenant);
        let packed = (0..60).any(|_| {
            let kept = names(bucket);
            let done = kept
                .iter()
                .any(|k| k.starts_with(&files) && k.ends_with(".parquet"))
                && !kept.iter().any(|k| k.starts_with(&objects));
            if !done {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            done
        });
        assert!(packed, "packed within 30 seconds: {:?}", names(bucket));
    }

    /// Once a version is packed, a start whose configuration no longer has
    /// `[storage.parquet]` is refused, naming the table and why; `check`
    /// names the problem for the tenant without refusing; with the table
    /// back the server starts and the vCon reads back from its row.
    #[tokio::test(flavor = "multi_thread")]
    async fn serve_refuses_to_start_without_storage_parquet_once_versions_are_packed() {
        let bucket = format!("vcon-unpack-{}", &TenantId::generate().to_string()[..12]);
        signed("PUT", &bucket);
        let s = setup();
        in_bucket(&s, &bucket);
        let without = std::fs::read_to_string(&s.config).expect("reads");
        let with = format!("{without}\n[storage.parquet]\npack_interval = 1\n");
        std::fs::write(&s.config, &with).expect("writes");
        let started = start(serve_with_keys(&s));
        let client = started.client(&s);
        let body = std::fs::read(format!(
            "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("A.4");
        let stored = client
            .post(started.url("/vcons"))
            .bearer_auth(&s.token)
            .header("content-type", "application/vcon")
            .body(body.clone())
            .send()
            .await
            .expect("an answer");
        assert_eq!(stored.status(), 201, "the vCon is stored");
        wait_until_packed(&s, &bucket);
        drop(started);

        std::fs::write(&s.config, &without).expect("the table taken out");
        let refusal = refused(serve_with_keys(&s));
        assert_eq!(refusal.status.code(), Some(1), "{}", stderr(&refusal));
        // The last line: the S3 client may warn first about the
        // credentials file the test points at nothing.
        let said = stderr(&refusal);
        assert_eq!(
            said.lines().last(),
            Some(
                "vcon-server: 1 tenant(s) have versions packed into Parquet files, and the \
             configuration has no [storage.parquet]: without it the server packs no more, and the \
             start no longer refuses a bucket with Object Lock or versioning, either of which keeps \
             the bytes of a pack file that a purge rewrote; put the [storage.parquet] table back, \
             since packed versions cannot be turned back into objects; vcon-server check names \
             the tenants"
            ),
            "{said}"
        );
        let mut check = Command::new(env!("CARGO_BIN_EXE_vcon-server"));
        check.arg("check").arg("--config").arg(&s.config);
        let checked = with_keys(check).output().expect("check runs");
        assert_eq!(checked.status.code(), Some(1), "{}", stderr(&checked));
        let report: Vec<serde_json::Value> = String::from_utf8(checked.stdout)
            .expect("text")
            .lines()
            .map(|l| serde_json::from_str(l).expect("a JSON line"))
            .collect();
        let tenant = report
            .iter()
            .find(|line| line["tenant"] == s.tenant.to_string())
            .expect("the tenant's line");
        assert_eq!(tenant["packed"], 1, "{tenant}");
        assert_eq!(
            tenant["problems"],
            serde_json::json!([
                "1 version(s) are packed into Parquet files, and the configuration has no \
                 [storage.parquet]: serve refuses to start until the table is put back"
            ]),
            "{tenant}"
        );

        std::fs::write(&s.config, &with).expect("the table put back");
        let started = start(serve_with_keys(&s));
        let read = started
            .client(&s)
            .get(started.url(&format!("/vcons/{UUID}")))
            .bearer_auth(&s.token)
            .send()
            .await
            .expect("an answer");
        assert_eq!(read.status(), 200, "the vCon is read");
        assert_eq!(
            read.bytes().await.expect("body").as_ref(),
            body.as_slice(),
            "it reads back byte for byte from its row"
        );
    }

    /// An object under the tenant's name in the bucket that no row names,
    /// as a crash between the upload and the commit leaves, is removed at
    /// the next start, and the server logs how many it removed.
    #[test]
    fn serve_logs_how_many_bucket_objects_start_up_recovery_removes() {
        let bucket = format!("vcon-recover-{}", &TenantId::generate().to_string()[..12]);
        signed("PUT", &bucket);
        let s = setup();
        in_bucket(&s, &bucket);
        // The first start claims the bucket; the leftover is planted after.
        let first = start(serve_with_keys(&s));
        drop(first);
        let orphan = format!("tenants/{}/objects/{}", s.tenant, VersionId::generate());
        signed("PUT", &format!("{bucket}/{orphan}"));
        assert!(names(&bucket).contains(&orphan), "the leftover is planted");
        let mut started = start(serve_with_keys(&s));
        let line = started
            .line_with("start-up recovery removed", Duration::from_secs(20))
            .expect("serve logged no start-up recovery removal");
        assert!(
            line.contains("removed=1"),
            "the leftover is counted: {line}"
        );
        assert!(
            !line.contains(&s.tenant.to_string()),
            "the line names no tenant: {line}"
        );
        assert_eq!(
            names(&bucket),
            ["vcon-server.claim"],
            "only the claim is left in the bucket"
        );
    }
}

/// With the catalogs in PostgreSQL, a preview in this release: against the
/// servers `scripts/pg-test-server.sh` starts (`VCON_TEST_PG_*`) and the
/// S3-compatible service of `VCON_TEST_S3_*`, since a PostgreSQL catalog
/// needs its objects in a bucket.
#[cfg(all(feature = "backend-tests", feature = "postgres"))]
mod postgres {
    use super::*;

    fn var(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| {
            panic!("{name} is not set: these tests need scripts/pg-test-server.sh's servers")
        })
    }

    /// `s`'s configuration with its objects in a new bucket and its
    /// catalogs on the test primary, trusting the CA in `ca`.
    fn on_postgres(s: &Setup, ca: &str) {
        use std::os::unix::fs::PermissionsExt;
        let bucket = format!("vcon-pg-{}", &TenantId::generate().to_string()[..12]);
        s3::signed("PUT", &bucket);
        s3::in_bucket(s, &bucket);
        let dir = PathBuf::from(var("VCON_TEST_PG_DIR"));
        let password = s.dir.path().join("pg.password");
        std::fs::copy(dir.join("password"), &password).expect("copies the password");
        std::fs::set_permissions(&password, std::fs::Permissions::from_mode(0o600)).expect("mode");
        let mut text = std::fs::read_to_string(&s.config).expect("reads");
        text.push_str(&format!(
            "\n[catalog]\nbackend = \"postgres\"\n\n[catalog.postgres]\nhosts = \
             [\"localhost\"]\nport = {}\ndbname = \"vcon\"\nuser = \"vcon_app\"\n\
             ca_file = \"{}\"\npassword_file = \"{}\"\n",
            var("VCON_TEST_PG_PORT"),
            dir.join(ca).display(),
            password.display()
        ));
        std::fs::write(&s.config, text).expect("writes");
    }

    /// Runs `vcon-server migrate` as `vcon_migrate`, on a copy of `s`'s
    /// configuration with that login, and checks what it printed: the
    /// schema is at this binary's version, whichever run made it.
    fn migrate(s: &Setup) {
        let text = std::fs::read_to_string(&s.config).expect("reads");
        let migrate_config = s.dir.path().join("vcon-migrate.toml");
        std::fs::write(
            &migrate_config,
            text.replace("user = \"vcon_app\"", "user = \"vcon_migrate\""),
        )
        .expect("writes");
        let out = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .args(["migrate", "--config"])
            .arg(&migrate_config)
            .output()
            .expect("runs");
        let printed = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success()
                && printed.contains(&format!("\"to\":{}", super::postgres_schema_version())),
            "migrate succeeded and left this binary's version: {printed} {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn serve_reaches_postgresql_over_tls_then_refuses_to_go_on_as_a_preview() {
        let s = setup();
        on_postgres(&s, "ca.pem");
        migrate(&s);
        let out = refused(s3::with_keys(serve(&s)));
        let text = stderr(&out);
        assert_eq!(out.status.code(), Some(1), "exit 1: {text}");
        assert!(
            text.contains(&format!(
                "connected to PostgreSQL {}.",
                var("VCON_TEST_PG_MAJOR")
            )) && text.contains("as vcon_app over TLS")
                && text.contains("a preview"),
            "names what it reached and why it stops: {text}"
        );
        let password =
            std::fs::read_to_string(PathBuf::from(var("VCON_TEST_PG_DIR")).join("password"))
                .expect("reads");
        assert!(
            !text.contains(password.trim_end()),
            "never the password: {text}"
        );
    }

    #[test]
    fn serve_refuses_a_postgresql_server_whose_certificate_another_authority_signed() {
        let s = setup();
        on_postgres(&s, "wrong-ca.pem");
        let text = stderr(&refused(s3::with_keys(serve(&s))));
        assert!(
            text.contains("not signed by an authority in catalog.postgres.ca_file"),
            "{text}"
        );
    }
}

/// `serve` with an `[operator]` table: a second listener that serves
/// operator keys only, while `listen` serves tenant keys only.
mod operator_listener {
    use super::*;

    /// `setup`, with an operator key made, and `table` added to the
    /// configuration.
    fn setup_with(table: &str) -> (Setup, String) {
        let s = setup();
        let operator = registry(&s)
            .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "operator")
            .expect("an operator key")
            .token;
        let mut text = std::fs::read_to_string(&s.config).expect("read");
        text.push_str(table);
        std::fs::write(&s.config, text).expect("write");
        (s, operator)
    }

    const TABLE: &str = "\n[operator]\nlisten = \"127.0.0.1:0\"\n";

    /// The address the operator listener's line names.
    fn operator_addr(started: &mut Started) -> SocketAddr {
        let line = started
            .line_with("operator_listen=", Duration::from_secs(20))
            .expect("a line that names the operator listener");
        line.split_whitespace()
            .find_map(|f| f.strip_prefix("operator_listen="))
            .expect("the field")
            .parse()
            .expect("an address")
    }

    /// A client that trusts `pem` and reaches `addr` as localhost.
    fn client(pem: &str, addr: SocketAddr) -> reqwest::Client {
        reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(pem.as_bytes()).expect("pem")])
            .resolve("localhost", addr)
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("a client")
    }

    /// The status of `GET path` at `addr` with `token`, and the last part
    /// of the problem type when the answer is a problem.
    async fn get(pem: &str, addr: SocketAddr, path: &str, token: &str) -> (u16, String) {
        let answer = client(pem, addr)
            .get(format!("https://localhost:{}{path}", addr.port()))
            .bearer_auth(token)
            .send()
            .await
            .expect("an answer");
        let status = answer.status().as_u16();
        let problem = answer
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|body| body["type"].as_str().map(str::to_owned))
            .and_then(|url| url.rsplit('/').next().map(str::to_owned))
            .unwrap_or_default();
        (status, problem)
    }

    fn stop(started: &mut Started) {
        let pid = started.running.0.id();
        // SAFETY: the pid is our running child's.
        let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        assert_eq!(sent, 0, "the signal was sent");
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = started.running.0.try_wait().expect("wait") {
                break status;
            }
            assert!(Instant::now() < deadline, "serve did not stop on SIGTERM");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(status.success(), "it stopped cleanly: {status:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_listener_serves_its_own_kind_of_key_and_refuses_the_other() {
        let (s, operator) = setup_with(TABLE);
        let mut started = start(serve(&s));
        let (tenant_addr, operator_addr) = (started.addr, operator_addr(&mut started));
        assert_ne!(tenant_addr, operator_addr, "two sockets");

        assert_eq!(
            get(&s.pem, tenant_addr, "/key", &s.token).await.0,
            200,
            "a tenant key on the tenant listener"
        );
        assert_eq!(
            get(&s.pem, operator_addr, "/admin/tenants", &operator)
                .await
                .0,
            200,
            "an operator key on the operator listener"
        );
        assert_eq!(
            get(&s.pem, tenant_addr, "/admin/tenants", &operator).await,
            (403, "wrong-listener".to_owned()),
            "an operator key on the tenant listener"
        );
        assert_eq!(
            get(&s.pem, operator_addr, "/vcons", &s.token).await,
            (403, "wrong-listener".to_owned()),
            "a tenant key on the operator listener"
        );
        stop(&mut started);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn without_the_table_the_one_listener_serves_both_kinds_as_before() {
        let (s, operator) = setup_with("");
        let mut started = start(serve(&s));
        let addr = started.addr;
        assert_eq!(
            get(&s.pem, addr, "/key", &s.token).await.0,
            200,
            "a tenant key"
        );
        assert_eq!(
            get(&s.pem, addr, "/admin/tenants", &operator).await.0,
            200,
            "an operator key"
        );
        assert_eq!(
            started.line_with("operator_listen=", Duration::from_millis(300)),
            None,
            "no operator listener is named"
        );
        stop(&mut started);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn both_listeners_serve_the_console_without_a_key() {
        let (s, _) = setup_with(TABLE);
        let mut started = start(serve(&s));
        for addr in [started.addr, operator_addr(&mut started)] {
            let answer = client(&s.pem, addr)
                .get(format!("https://localhost:{}/console/", addr.port()))
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 200, "{addr}");
        }
        stop(&mut started);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_operator_listener_presents_its_own_certificate_when_it_has_one() {
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate");
        let own_pem = made.cert.pem();
        let (s, operator) = setup_with("");
        let (cert, key) = (s.dir.path().join("ops.pem"), s.dir.path().join("ops.key"));
        std::fs::write(&cert, &own_pem).expect("write");
        std::fs::write(&key, made.signing_key.serialize_pem()).expect("write");
        chmod(&key, 0o600);
        let mut text = std::fs::read_to_string(&s.config).expect("read");
        text.push_str(&format!(
            "{TABLE}\n[operator.tls]\ncert = \"{}\"\nkey = \"{}\"\n",
            cert.display(),
            key.display()
        ));
        std::fs::write(&s.config, text).expect("write");
        let mut started = start(serve(&s));
        let (tenant_addr, operator_addr) = (started.addr, operator_addr(&mut started));

        assert_eq!(
            get(&own_pem, operator_addr, "/key", &operator).await.0,
            200,
            "the operator listener, trusted by its own certificate"
        );
        assert_eq!(
            get(&s.pem, tenant_addr, "/key", &s.token).await.0,
            200,
            "the tenant listener, trusted by the server's"
        );
        let crossed = client(&s.pem, operator_addr)
            .get(format!("https://localhost:{}/key", operator_addr.port()))
            .bearer_auth(&operator)
            .send()
            .await;
        assert!(
            crossed.is_err(),
            "the operator listener does not present the server's certificate: {crossed:?}"
        );
        stop(&mut started);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sigterm_stops_both_listeners() {
        let (s, _) = setup_with(TABLE);
        let mut started = start(serve(&s));
        let (tenant_addr, operator_addr) = (started.addr, operator_addr(&mut started));
        for addr in [tenant_addr, operator_addr] {
            assert!(
                std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)).is_ok(),
                "{addr} listens while the server runs"
            );
        }
        // `stop` waits for the process to end and holds its exit status to
        // success. A process that has ended holds no socket, so both
        // listeners are closed. The two ports are not tried after: they
        // are free from that moment, and another test's server, started
        // on a port the system chooses, may be given one of them and
        // accept the connection.
        stop(&mut started);
        assert!(
            matches!(started.running.0.try_wait(), Ok(Some(status)) if status.success()),
            "the one process that held both listeners has ended"
        );
    }

    #[test]
    fn the_serving_line_still_names_the_tenant_listener() {
        let (s, _) = setup_with(TABLE);
        let mut started = start(serve(&s));
        let tenant_addr = started.addr;
        let operator_addr = operator_addr(&mut started);
        assert_ne!(tenant_addr.port(), 0, "the port bound");
        assert_ne!(operator_addr.port(), 0, "the port bound");
        assert_ne!(tenant_addr.port(), operator_addr.port(), "a port each");
        assert!(tenant_addr.ip().is_loopback(), "{tenant_addr}");
        assert!(operator_addr.ip().is_loopback(), "{operator_addr}");
    }

    #[test]
    fn the_operator_listener_is_named_before_the_serving_line() {
        // A script that waits for the serving line finds both listeners
        // bound when it sees it.
        let (s, _) = setup_with(TABLE);
        let started = start(serve(&s));
        assert!(
            started
                .early
                .iter()
                .any(|line| line.contains("operator_listen=127.0.0.1:")),
            "named before the serving line: {:?}",
            started.early
        );
    }

    #[test]
    fn an_operator_address_that_is_taken_stops_the_start() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = taken.local_addr().expect("addr");
        let (s, _) = setup_with(&format!("\n[operator]\nlisten = \"{addr}\"\n"));
        let text = stderr(&refused(serve(&s)));
        assert!(
            text.contains("the operator listening address cannot be bound: "),
            "{text}"
        );
    }

    #[test]
    fn a_tenant_address_that_is_taken_is_still_named_as_the_listening_address() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = taken.local_addr().expect("addr");
        let (s, _) = setup_with(TABLE);
        let text = std::fs::read_to_string(&s.config).expect("read").replace(
            "listen = \"127.0.0.1:0\"\ndata_dir",
            &format!("listen = \"{addr}\"\ndata_dir"),
        );
        std::fs::write(&s.config, text).expect("write");
        let text = stderr(&refused(serve(&s)));
        assert!(
            text.contains("the listening address cannot be bound: "),
            "{text}"
        );
        assert!(!text.contains("operator"), "{text}");
    }

    #[test]
    fn the_tenant_listeners_own_socket_stops_the_start_and_names_the_key() {
        let (s, _) = setup_with("\n[operator]\nlisten = \"127.0.0.1:8443\"\n");
        let text = std::fs::read_to_string(&s.config).expect("read").replace(
            "listen = \"127.0.0.1:0\"\ndata_dir",
            "listen = \"127.0.0.1:8443\"\ndata_dir",
        );
        std::fs::write(&s.config, text).expect("write");
        let text = stderr(&refused(serve(&s)));
        assert!(text.contains("operator.listen: 127.0.0.1:8443"), "{text}");
    }

    #[test]
    fn an_operator_key_file_other_users_may_read_stops_the_start_and_names_the_fix() {
        let (s, _) = setup_with("");
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate");
        let (cert, key) = (s.dir.path().join("ops.pem"), s.dir.path().join("ops.key"));
        std::fs::write(&cert, made.cert.pem()).expect("write");
        std::fs::write(&key, made.signing_key.serialize_pem()).expect("write");
        chmod(&key, 0o644);
        let mut text = std::fs::read_to_string(&s.config).expect("read");
        text.push_str(&format!(
            "{TABLE}\n[operator.tls]\ncert = \"{}\"\nkey = \"{}\"\n",
            cert.display(),
            key.display()
        ));
        std::fs::write(&s.config, text).expect("write");
        let text = stderr(&refused(serve(&s)));
        let path = key.display();
        assert!(
            text.contains(&format!(
                "the operator listener's TLS certificate or key cannot be used: operator.tls.key {path}: other users may read or change the file (mode 0644); run `chmod 600 {path}`"
            )),
            "{text}"
        );
    }

    #[test]
    fn an_operator_certificate_that_is_missing_stops_the_start() {
        let (s, _) = setup_with("");
        let mut text = std::fs::read_to_string(&s.config).expect("read");
        text.push_str(&format!(
            "{TABLE}\n[operator.tls]\ncert = \"{}\"\nkey = \"{}\"\n",
            s.dir.path().join("missing.pem").display(),
            s.tls_key.display()
        ));
        std::fs::write(&s.config, text).expect("write");
        let text = stderr(&refused(serve(&s)));
        assert!(
            text.contains("the operator listener's TLS certificate or key cannot be used"),
            "{text}"
        );
    }

    #[test]
    fn an_operator_table_without_its_address_stops_the_start_and_names_the_key() {
        let (s, _) = setup_with("\n[operator]\n");
        let text = stderr(&refused(serve(&s)));
        assert!(text.contains("`listen`"), "{text}");
    }

    /// Each listener has its own places: with one connection at once, a
    /// silent connection that holds the tenant listener's place takes
    /// nothing from the operator listener.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_full_tenant_listener_takes_no_place_from_the_operator_listener() {
        let (s, operator) = setup_with(TABLE);
        let text = std::fs::read_to_string(&s.config)
            .expect("read")
            .replace("[limits]\n", "[limits]\nconnections = 1\n");
        assert!(text.contains("connections = 1"), "the key was written");
        std::fs::write(&s.config, text).expect("write");
        let mut started = start(serve(&s));
        let (tenant_addr, operator_addr) = (started.addr, operator_addr(&mut started));
        let _silent = std::net::TcpStream::connect(tenant_addr).expect("connects");
        // Time for the tenant listener to take the silent connection.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let begun = Instant::now();
        assert_eq!(
            get(&s.pem, operator_addr, "/key", &operator).await.0,
            200,
            "the operator listener answers"
        );
        assert!(
            begun.elapsed() < Duration::from_secs(5),
            "at once, not after the tenant listener's handshake limit: {:?}",
            begun.elapsed()
        );
        let waiting = client(&s.pem, tenant_addr)
            .get(format!("https://localhost:{}/key", tenant_addr.port()))
            .bearer_auth(&s.token)
            .timeout(Duration::from_secs(2))
            .send()
            .await;
        assert!(
            waiting.is_err(),
            "the tenant listener's one place is held: {waiting:?}"
        );
        stop(&mut started);
    }

    /// Each listener's times of being full are counted under its own
    /// configuration key, and logged with its own address.
    #[tokio::test(flavor = "multi_thread")]
    async fn each_listener_that_is_full_is_counted_under_its_own_key() {
        for operator_is_full in [true, false] {
            let (s, _operator) = setup_with(TABLE);
            let text = std::fs::read_to_string(&s.config)
                .expect("read")
                .replace("[limits]\n", "[limits]\nconnections = 1\n");
            assert!(text.contains("connections = 1"), "the key was written");
            std::fs::write(&s.config, text).expect("write");
            let mut started = start(serve(&s));
            let (tenant_addr, operator_addr) = (started.addr, operator_addr(&mut started));
            let (full, other, key, other_key) = if operator_is_full {
                (operator_addr, tenant_addr, "operator.listen", "listen")
            } else {
                (tenant_addr, operator_addr, "listen", "operator.listen")
            };
            let _silent = std::net::TcpStream::connect(full).expect("connects");
            let line = started
                .line_with("the listener is full", Duration::from_secs(10))
                .expect("a warning that a listener is full");
            assert!(
                line.contains(&format!("listener={full}")),
                "the full listener's address: {line}"
            );
            assert!(
                !line.contains(&format!("listener={other}")),
                "not the other's: {line}"
            );
            let records = listener_full_records_at(&s, key, 1);
            assert_eq!(times_full(&records, key), 1, "{key}: {records:?}");
            assert_eq!(
                times_full(&records, other_key),
                0,
                "{other_key}: {records:?}"
            );
            stop(&mut started);
        }
    }
}
