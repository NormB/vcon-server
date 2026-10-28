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
use vcon_server::ids::{KeyId, TenantId};
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
async fn serve_stores_reads_keeps_its_memory_locked_and_stops_on_sigterm() {
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
