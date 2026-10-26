//! `vcon-server serve`, run as its own process: it starts from its
//! configuration, stores and reads, holds its memory locked and out of core
//! dumps, and stops on SIGTERM. Each way it refuses to start is tried too.

use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vcon_server::access::{Scope, Scopes};
use vcon_server::audit::Actor;
use vcon_server::clock::SystemClock;
use vcon_server::envelope::Key;
use vcon_server::fs::OsFs;
use vcon_server::ids::{KeyId, TenantId};
use vcon_server::retention::Retention;
use vcon_server::store::Caller;
use vcon_server::tenants::Tenants;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

/// A data directory with one tenant and a key for it, a certificate and a
/// configuration file, ready for `serve`.
struct Setup {
    dir: tempfile::TempDir,
    config: PathBuf,
    master_key: PathBuf,
    pem: String,
    token: String,
    addr: SocketAddr,
    data: PathBuf,
    tenant: TenantId,
}

fn random_key_file(path: &std::path::Path) {
    let mut bytes = [0u8; 32];
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .expect("urandom")
        .read_exact(&mut bytes)
        .expect("32 random bytes");
    std::fs::write(path, bytes).expect("write the key file");
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().expect("temp dir");
    let data = dir.path().join("data");
    std::fs::create_dir(&data).expect("data dir");
    let master_key = dir.path().join("master.key");
    random_key_file(&master_key);
    let tenants = Tenants::open(
        &data,
        Key::load_master(&master_key).expect("the master key"),
        Arc::new(OsFs),
        Arc::new(SystemClock),
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
    let addr = TcpListener::bind("127.0.0.1:0")
        .expect("a free port")
        .local_addr()
        .expect("addr");
    let config = dir.path().join("vcon-server.toml");
    std::fs::write(
        &config,
        format!(
            r#"listen = "{addr}"
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
        pem,
        token,
        addr,
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

/// Waits until the server takes connections.
fn ready(s: &Setup, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::net::TcpStream::connect(s.addr).is_err() {
        assert!(
            child.try_wait().expect("wait").is_none(),
            "serve stopped before it listened"
        );
        assert!(Instant::now() < deadline, "serve never listened");
        std::thread::sleep(Duration::from_millis(20));
    }
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
async fn serve_stores_reads_keeps_its_memory_locked_and_stops_on_sigterm() {
    let s = setup();
    let mut running = Running(
        serve(&s)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("starts"),
    );
    ready(&s, &mut running.0);
    let pid = running.0.id();

    let client = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(s.pem.as_bytes()).expect("pem")])
        .resolve("localhost", s.addr)
        .no_proxy()
        .build()
        .expect("a client");
    let url = |path: &str| format!("https://localhost:{}{path}", s.addr.port());
    let body = std::fs::read(format!(
        "{}/../vcon-form/tests/fixtures/ietf/ab_call_ext_rec.vcon",
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
        if let Some(status) = running.0.try_wait().expect("wait") {
            break status;
        }
        assert!(Instant::now() < deadline, "serve did not stop on SIGTERM");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "it stopped cleanly: {status:?}");
}

#[test]
fn a_wrong_master_key_stops_the_start() {
    let s = setup();
    random_key_file(&s.master_key);
    let output = refused(serve(&s));
    assert!(
        stderr(&output).contains("master key"),
        "{}",
        stderr(&output)
    );
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
fn memory_that_cannot_be_locked_stops_the_start() {
    let s = setup();
    let mut command = serve(&s);
    // SAFETY: setrlimit is async-signal-safe and touches only the child.
    unsafe {
        command.pre_exec(|| {
            let small = libc::rlimit {
                rlim_cur: 64 * 1024,
                rlim_max: 64 * 1024,
            };
            if libc::setrlimit(libc::RLIMIT_MEMLOCK, &small) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let output = refused(command);
    assert!(stderr(&output).contains("lock"), "{}", stderr(&output));
    let _ = &s.dir;
}

/// The "purge sweep" lines `running` writes to its stderr, as they come.
fn sweep_lines(running: &mut Running) -> std::sync::mpsc::Receiver<String> {
    let stderr = running.0.stderr.take().expect("piped");
    let (line_tx, line_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            if line.contains("purge sweep") && line_tx.send(line).is_err() {
                break;
            }
        }
    });
    line_rx
}

/// The lines `running` writes to its stderr that contain `pattern`, as
/// they come.
fn lines_with(running: &mut Running, pattern: &'static str) -> std::sync::mpsc::Receiver<String> {
    let stderr = running.0.stderr.take().expect("piped");
    let (line_tx, line_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            if line.contains(pattern) && line_tx.send(line).is_err() {
                break;
            }
        }
    });
    line_rx
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
    assert!(
        stderr(&output).contains("tenant key"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn serve_warns_when_key_rotation_is_overdue() {
    let s = setup();
    let tenants = registry(&s);
    let store = tenants.store(s.tenant).expect("its store");
    let body = std::fs::read(format!(
        "{}/../vcon-form/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4");
    let found = vcon_form::inspect(
        &body,
        Some("application/vcon"),
        &vcon_form::Limits { inflated: 1 << 20 },
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
    let text = std::fs::read_to_string(&s.config).expect("configuration");
    std::fs::write(
        &s.config,
        text.replace("rotation_warn = 2592000", "rotation_warn = 1"),
    )
    .expect("written");
    std::thread::sleep(Duration::from_millis(1100));
    let mut running = Running(
        serve(&s)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("starts"),
    );
    let warned = lines_with(&mut running, "key rotation overdue")
        .recv_timeout(Duration::from_secs(20))
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
    Tenants::open(
        &s.data,
        Key::load_master(&s.master_key).expect("the master key"),
        Arc::new(OsFs),
        Arc::new(SystemClock),
    )
    .expect("opens")
}

#[test]
fn serve_purges_what_is_due_when_it_starts() {
    let s = setup();
    // A vCon deleted while seven years applied, then the tenant's period set
    // to zero: due, and waiting for a sweep.
    let tenants = registry(&s);
    let store = tenants.store(s.tenant).expect("its store");
    let body = std::fs::read(format!(
        "{}/../vcon-form/tests/fixtures/ietf/ab_call_ext_rec.vcon",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("A.4");
    let found = vcon_form::inspect(
        &body,
        Some("application/vcon"),
        &vcon_form::Limits { inflated: 1 << 20 },
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

    let mut running = Running(
        serve(&s)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("starts"),
    );
    let sweeps = sweep_lines(&mut running);
    let swept = sweeps
        .recv_timeout(Duration::from_secs(20))
        .expect("serve logged no purge sweep");
    assert!(swept.contains("purged=1"), "{swept}");
    assert!(swept.contains("failed=0"), "{swept}");
    assert!(
        !swept.contains(&s.tenant.to_string()),
        "a sweep logs counts, never a tenant: {swept}"
    );
    assert!(
        sweeps.recv_timeout(Duration::from_secs(3)).is_err(),
        "no second sweep long before purge_interval (an hour here)"
    );
    drop(running);

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
    let mut running = Running(
        serve(&s)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("starts"),
    );
    let sweeps = sweep_lines(&mut running);
    sweeps
        .recv_timeout(Duration::from_secs(20))
        .expect("a sweep at start");
    let first = Instant::now();
    sweeps
        .recv_timeout(Duration::from_secs(10))
        .expect("a second sweep after purge_interval");
    assert!(
        first.elapsed() >= Duration::from_millis(800),
        "not before purge_interval: {:?}",
        first.elapsed()
    );
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
    let mut running = Running(
        serve(&s)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("starts"),
    );
    ready(&s, &mut running.0);
    let client = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(s.pem.as_bytes()).expect("pem")])
        .resolve("localhost", s.addr)
        .no_proxy()
        .build()
        .expect("a client");
    let url = |path: &str| format!("https://localhost:{}{path}", s.addr.port());
    let tenant = s.tenant.to_string();
    let body = std::fs::read(format!(
        "{}/../vcon-form/tests/fixtures/ietf/ab_call_ext_rec.vcon",
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
