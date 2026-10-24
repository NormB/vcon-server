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
