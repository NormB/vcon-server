//! Every file vcon-server writes into a data directory or a backup is 0600
//! and every directory it makes is 0700, whatever the umask of the process
//! that runs it: the modes are the server's choice, not the caller's.

use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Setup {
    dir: tempfile::TempDir,
    config: PathBuf,
    data: PathBuf,
    pem: String,
    umask: &'static str,
}

/// Where every server in these tests listens: port 0, so each takes a port
/// of its own when it binds, and `start` reads that port from its
/// `serving` line. A port picked beforehand could be taken by another
/// process before the server binds it.
const ANY_PORT: &str = "127.0.0.1:0";

fn key_file(path: &Path, byte: u8) {
    std::fs::write(path, [byte; 32]).expect("a key file");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
}

fn setup(umask: &'static str) -> Setup {
    let dir = tempfile::tempdir().expect("temp dir");
    let data = dir.path().join("data");
    std::fs::create_dir(&data).expect("data dir");
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    key_file(&dir.path().join("master.key"), 0x42);
    key_file(&dir.path().join("new.key"), 0x43);
    let made =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate");
    let pem = made.cert.pem();
    let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
    std::fs::write(&cert, &pem).expect("write");
    std::fs::write(&key, made.signing_key.serialize_pem()).expect("write");
    let config = dir.path().join("vcon-server.toml");
    write_config(
        &config,
        ANY_PORT.parse().expect("an address"),
        &data,
        &dir.path().join("master.key"),
        &cert,
        &key,
    );
    Setup {
        dir,
        config,
        data,
        pem,
        umask,
    }
}

fn write_config(
    config: &Path,
    addr: SocketAddr,
    data: &Path,
    master: &Path,
    cert: &Path,
    key: &Path,
) {
    std::fs::write(
        config,
        format!(
            "listen = \"{addr}\"\ndata_dir = {data:?}\nmaster_key = {master:?}\n\
             [tls]\ncert = {cert:?}\nkey = {key:?}\n"
        ),
    )
    .expect("configuration");
}

impl Setup {
    /// `vcon-server <args...>` under this setup's umask.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(format!("umask {}; exec \"$0\" \"$@\"", self.umask))
            .arg(env!("CARGO_BIN_EXE_vcon-server"))
            .args(args);
        command
    }

    fn ok(&self, args: &[&str]) -> Vec<serde_json::Value> {
        let out: Output = self.command(args).output().expect("runs");
        assert_eq!(
            out.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .expect("text")
            .lines()
            .map(|l| serde_json::from_str(l).expect("a JSON line"))
            .collect()
    }

    fn config(&self) -> &str {
        self.config.to_str().expect("a UTF-8 path")
    }
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Starts `serve` under the setup's umask with the configuration file
/// `config`, and waits for its `serving` line: the server running, and the
/// address it bound. Only that line says the server is ready; something
/// else accepting on its port does not.
fn start(s: &Setup, config: &Path) -> Result<(Running, SocketAddr), String> {
    let mut running = Running(
        s.command(&["serve", "--config", config.to_str().expect("a UTF-8 path")])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("starts"),
    );
    let stderr = running.0.stderr.take().expect("piped");
    let (line_tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        // Read to the end even once no one wants the lines: a pipe left
        // unread fills, and a closed one fails serve's next log line;
        // either stops the server in the middle of a request.
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            let _ = line_tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut said = Vec::new();
    loop {
        match lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => match bound(&line) {
                Some(addr) => return Ok((running, addr)),
                None => said.push(line),
            },
            // The sender is gone once serve exits, or the deadline passed.
            Err(_) => return Err(format!("serve never said where it listens: {said:?}")),
        }
    }
}

/// The address a `serving` line names, from its `listen=` field.
fn bound(line: &str) -> Option<SocketAddr> {
    let mut fields = line.split_whitespace();
    fields.find(|f| *f == "serving")?;
    fields.find_map(|f| f.strip_prefix("listen="))?.parse().ok()
}

/// Stores one vCon through a server started under the setup's umask.
async fn store_one(s: &Setup, token: &str) {
    let (running, addr) = start(s, &s.config).expect("the server starts");
    let client = reqwest::Client::builder()
        .tls_certs_only([reqwest::Certificate::from_pem(s.pem.as_bytes()).expect("pem")])
        .resolve("localhost", addr)
        .no_proxy()
        .build()
        .expect("a client");
    let answer = client
        .post(format!("https://localhost:{}/vcons", addr.port()))
        .bearer_auth(token)
        .header("content-type", "application/vcon")
        .body(r#"{"uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46", "created_at": "2026-10-23T12:00:00Z", "parties": []}"#)
        .send()
        .await
        .expect("an answer");
    assert_eq!(answer.status(), 201, "the vCon is stored");
    drop(running);
}

/// Every path under `root`, `root` itself included, with its mode bits.
fn modes(root: &Path) -> Vec<(PathBuf, bool, u32)> {
    let mut found = Vec::new();
    let meta = std::fs::symlink_metadata(root).expect("metadata");
    found.push((
        root.to_owned(),
        meta.is_dir(),
        meta.permissions().mode() & 0o7777,
    ));
    if meta.is_dir() {
        for entry in std::fs::read_dir(root).expect("reads") {
            found.extend(modes(&entry.expect("entry").path()));
        }
    }
    found
}

fn wrong_modes(root: &Path) -> Vec<String> {
    modes(root)
        .into_iter()
        .filter(|(_, dir, mode)| *mode != if *dir { 0o700 } else { 0o600 })
        .map(|(path, _, mode)| format!("{} {mode:o}", path.display()))
        .collect()
}

/// Every command that writes into the data directory or a backup, run
/// under `umask`, leaves only 0600 files and 0700 directories.
async fn every_writer_under(umask: &'static str) {
    let s = setup(umask);
    let config = s.config().to_owned();
    let made = s.ok(&["tenant", "create", "--config", &config, "acme"]);
    let tenant = made[0]["id"].as_str().expect("an id").to_owned();
    let key = s.ok(&[
        "key", "create", "--config", &config, "--tenant", &tenant, "--scope", "write", "--scope",
        "read", "--label", "test",
    ]);
    let token = key[0]["token"].as_str().expect("a token").to_owned();
    store_one(&s, &token).await;
    // Before any rotation replaces a catalog with a copy, as made at first.
    assert_eq!(
        wrong_modes(&s.data),
        Vec::<String>::new(),
        "as made, umask {umask}"
    );
    s.ok(&["rekey", "tenant", "--config", &config, &tenant]);
    let new_master = s.dir.path().join("new.key");
    s.ok(&[
        "rekey",
        "master",
        "--config",
        &config,
        "--new-master",
        new_master.to_str().expect("UTF-8"),
    ]);
    write_config(
        &s.config,
        ANY_PORT.parse().expect("an address"),
        &s.data,
        &new_master,
        &s.dir.path().join("cert.pem"),
        &s.dir.path().join("key.pem"),
    );
    let backup = s.dir.path().join("backup");
    s.ok(&[
        "backup",
        backup.to_str().expect("UTF-8"),
        "--config",
        &config,
    ]);
    s.ok(&["check", "--config", &config]);
    assert_eq!(
        wrong_modes(&s.data),
        Vec::<String>::new(),
        "data directory, umask {umask}"
    );
    assert_eq!(
        wrong_modes(&backup),
        Vec::<String>::new(),
        "backup, umask {umask}"
    );
    let objects = modes(&backup)
        .into_iter()
        .filter(|(path, dir, _)| !dir && path.parent().is_some_and(|p| p.ends_with("objects")))
        .count();
    assert_eq!(objects, 1, "the backup holds the stored vCon's object");
}

#[tokio::test(flavor = "multi_thread")]
async fn modes_do_not_follow_a_group_writable_umask() {
    every_writer_under("0002").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn modes_do_not_follow_the_usual_umask() {
    every_writer_under("0022").await;
}

/// A server that cannot bind its address is never taken for ready, even
/// when another process listens on that address: the test must not talk to
/// whatever holds the port.
#[test]
fn a_server_that_cannot_bind_is_never_taken_for_ready() {
    let s = setup("0022");
    let holder = TcpListener::bind("127.0.0.1:0").expect("a port held by the test");
    let held = holder.local_addr().expect("addr");
    let config = s.dir.path().join("held.toml");
    write_config(
        &config,
        held,
        &s.data,
        &s.dir.path().join("master.key"),
        &s.dir.path().join("cert.pem"),
        &s.dir.path().join("key.pem"),
    );
    let started = start(&s, &config);
    assert!(
        started.is_err(),
        "a server that could not bind {held} is not ready; the test would talk to the holder"
    );
    drop(holder);
}
