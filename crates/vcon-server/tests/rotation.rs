//! The key rotation an operator runs, end to end, with the binary: a vCon
//! purged before the rotation cannot be recovered from a copy of the data
//! directory taken before the purge, once the old master key is gone.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use serde_json::Value;
use uuid::Uuid;
use vcon_server::catalog::{self, CatalogError};
use vcon_server::clock::SystemClock;
use vcon_server::envelope::{self, Context, Key};
use vcon_server::fs::OsFs;
use vcon_server::ids::{KeyId, TenantId, VersionId};
use vcon_server::retention::Retention;
use vcon_server::store::{Caller, Created};
use vcon_server::tenants::Tenants;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

struct Setup {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Setup {
    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn key_file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .output()
            .expect("the binary runs")
    }

    fn ok(&self, args: &[&str]) -> Vec<Value> {
        let out = self.run(args);
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

    /// Points the configuration at the master key file `name`.
    fn use_master(&self, name: &str) {
        std::fs::write(
            &self.config,
            format!(
                "listen = \"127.0.0.1:1\"\ndata_dir = {:?}\nmaster_key = {:?}\n\
                 [tls]\ncert = \"unused.pem\"\nkey = \"unused.pem\"\n",
                self.data(),
                self.key_file(name)
            ),
        )
        .expect("configuration");
    }
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir(dir.path().join("data")).expect("data dir");
    std::fs::write(dir.path().join("old.key"), [0x61; 32]).expect("old master");
    std::fs::set_permissions(
        dir.path().join("old.key"),
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    )
    .expect("chmod 0600");
    let s = Setup {
        config: dir.path().join("vcon-server.toml"),
        dir,
    };
    s.use_master("old.key");
    s
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("list") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

/// Stores a vCon in `tenant` as a producer would, and returns its version.
fn store_vcon(s: &Setup, master: &str, tenant: TenantId) -> VersionId {
    let tenants = Tenants::open(
        &s.data(),
        Key::load_master(&s.key_file(master)).expect("master"),
        Arc::new(OsFs),
        Arc::new(SystemClock),
    )
    .expect("opens");
    let body = format!(
        r#"{{"uuid": "{UUID}", "created_at": "2026-10-25T12:00:00Z", "subject": "to be purged", "parties": []}}"#
    );
    let found = vcon::read::inspect(
        body.as_bytes(),
        None,
        &vcon::read::Limits::new(1 << 20),
        &Default::default(),
    )
    .expect("a vCon");
    match tenants.store(tenant).expect("store").create(
        &Caller { key: KeyId([1; 8]) },
        &found,
        body.as_bytes(),
    ) {
        Ok(Created::New(version)) => version,
        other => panic!("{other:?}"),
    }
}

/// What the copy holds of the purged version, read with the keys of its
/// time: the wrapped data key and the sealed body.
struct Copied {
    wrapped_data_key: Vec<u8>,
    sealed: Vec<u8>,
}

/// Reads the copy with the old master key, as anyone who still had it
/// could, and checks that it does give the vCon back.
fn read_copy_with(copy: &Path, old_master: &Key, tenant: TenantId, version: VersionId) -> Copied {
    let server = catalog::open(
        &copy.join("server.sqlite"),
        &envelope::catalog_key(old_master).expect("key"),
        "",
    )
    .expect("the copy opens under the old master key");
    let wrapped: Vec<u8> = server
        .query_row(
            "SELECT wrapped_tenant_key FROM tenant WHERE id = ?1",
            [tenant.0],
            |r| r.get(0),
        )
        .expect("the tenant's row");
    let tenant_key = envelope::unwrap(old_master, &Context::Tenant { tenant }, &wrapped)
        .expect("the old tenant key");
    let dir = copy.join("tenants").join(tenant.to_string());
    let catalog = catalog::open(
        &dir.join("catalog.sqlite"),
        &envelope::catalog_key(&tenant_key).expect("key"),
        "",
    )
    .expect("the tenant's old catalog");
    let wrapped_data_key: Vec<u8> = catalog
        .query_row(
            "SELECT wrapped_key FROM version WHERE version = ?1",
            [version.0],
            |r| r.get(0),
        )
        .expect("the version's row");
    let ctx = Context::Version {
        tenant,
        uuid: Uuid::parse_str(UUID).expect("uuid"),
        version,
    };
    let data_key = envelope::unwrap(&tenant_key, &ctx, &wrapped_data_key).expect("the data key");
    let sealed = std::fs::read(dir.join("objects").join(version.to_string())).expect("sealed");
    let body = envelope::open(&data_key, &ctx, &sealed).expect("the body opens");
    assert!(
        String::from_utf8_lossy(&body).contains("to be purged"),
        "the copy really holds the vCon"
    );
    Copied {
        wrapped_data_key,
        sealed,
    }
}

#[test]
fn a_vcon_purged_before_a_rotation_cannot_be_recovered_from_an_old_copy() {
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let tenant = TenantId::parse(&tenant).expect("tenant id");
    let version = store_vcon(&s, "old.key", tenant);

    // A backup, taken while the vCon was kept.
    let copy = s.dir.path().join("backup");
    copy_dir(&s.data(), &copy);

    // The vCon is deleted under a zero retention period: purged at once.
    s.ok(&["tenant", "set-retention", &tenant.to_string(), "0d"]);
    {
        let tenants = Tenants::open(
            &s.data(),
            Key::load_master(&s.key_file("old.key")).expect("master"),
            Arc::new(OsFs),
            Arc::new(SystemClock),
        )
        .expect("opens");
        tenants
            .store(tenant)
            .expect("store")
            .delete(
                &Caller { key: KeyId([1; 8]) },
                Uuid::parse_str(UUID).expect("uuid"),
                Retention::Days(0),
            )
            .expect("deleted and purged");
    }

    // Until the rotation, anyone with the old master key reads the copy.
    let copied = read_copy_with(
        &copy,
        &Key::load_master(&s.key_file("old.key")).expect("old"),
        tenant,
        version,
    );

    // The rotation, as the operator guide gives it.
    let checked = s.ok(&["check"]);
    assert_eq!(
        checked[0]["purges_awaiting_rotation"], 1,
        "the purged vCon can still be read from the backup until the tenant key changes"
    );
    s.ok(&["rekey", "tenant", &tenant.to_string()]);
    assert_eq!(
        s.ok(&["check"])[0]["purges_awaiting_rotation"],
        0,
        "rekeying the tenant ends the wait"
    );
    std::fs::write(s.key_file("new.key"), [0x62; 32]).expect("new master");
    std::fs::set_permissions(
        s.key_file("new.key"),
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    )
    .expect("chmod 0600");
    let new_master_path = s.key_file("new.key");
    s.ok(&[
        "rekey",
        "master",
        "--new-master",
        new_master_path.to_str().expect("path"),
    ]);
    s.use_master("new.key");
    std::fs::remove_file(s.key_file("old.key")).expect("the old master key destroyed");
    assert_eq!(
        s.ok(&["check"]).last().expect("summary")["problems"],
        0,
        "after the rotation the data directory checks clean under the new master alone"
    );

    // Every key that still exists, tried on the copy.
    let new_master = Key::load_master(&s.key_file("new.key")).expect("new master");
    assert!(
        matches!(
            catalog::open(
                &copy.join("server.sqlite"),
                &envelope::catalog_key(&new_master).expect("key"),
                ""
            ),
            Err(CatalogError::WrongKey)
        ),
        "the copy's registry was encrypted under the old master, which is gone"
    );
    let live_server = catalog::open(
        &s.data().join("server.sqlite"),
        &envelope::catalog_key(&new_master).expect("key"),
        "",
    )
    .expect("the live registry opens under the new master");
    let live_wrapped: Vec<u8> = live_server
        .query_row(
            "SELECT wrapped_tenant_key FROM tenant WHERE id = ?1",
            [tenant.0],
            |r| r.get(0),
        )
        .expect("the tenant's row");
    drop(live_server);
    let tenant_key = envelope::unwrap(&new_master, &Context::Tenant { tenant }, &live_wrapped)
        .expect("the current tenant key");
    let old_catalog = copy
        .join("tenants")
        .join(tenant.to_string())
        .join("catalog.sqlite");
    for key in [&new_master, &tenant_key] {
        assert!(
            matches!(
                catalog::open(&old_catalog, &envelope::catalog_key(key).expect("key"), ""),
                Err(CatalogError::WrongKey)
            ),
            "no key that still exists opens the copy's tenant catalog"
        );
    }
    let ctx = Context::Version {
        tenant,
        uuid: Uuid::parse_str(UUID).expect("uuid"),
        version,
    };
    for key in [&new_master, &tenant_key] {
        assert!(
            envelope::unwrap(key, &ctx, &copied.wrapped_data_key).is_err(),
            "no key that still exists unwraps the purged vCon's data key"
        );
        assert!(
            envelope::open(key, &ctx, &copied.sealed).is_err(),
            "no key that still exists opens the purged vCon"
        );
    }
    // And the live data no longer holds it at all.
    assert!(
        !s.data()
            .join("tenants")
            .join(tenant.to_string())
            .join("objects")
            .join(version.to_string())
            .exists(),
        "the purge removed the version's object file"
    );
}

#[cfg(all(feature = "backend-tests", feature = "postgres"))]
mod pg;

/// The same rotation with the catalogs in PostgreSQL and the objects in a
/// bucket, run with the binary alone: a server stores and deletes, the
/// commands rotate. The copy an attacker keeps is the database's rows and
/// the bucket's object, taken before the purge. It needs the servers of
/// `scripts/pg-test-server.sh` (`VCON_TEST_PG_*`) and the S3-compatible
/// service of `VCON_TEST_S3_*`.
#[cfg(all(feature = "backend-tests", feature = "postgres"))]
mod postgres {
    use std::io::BufRead;
    use std::net::SocketAddr;
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    use super::pg::{self, var};
    use super::*;

    fn s3_var(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| {
            panic!("{name} is not set: these tests need a real S3-compatible service")
        })
    }

    /// curl, signing its request for S3 with the test service's keys; the
    /// answer's body, and whether the service answered with success.
    fn signed(method: &str, path: &str) -> (bool, Vec<u8>) {
        let out = Command::new("curl")
            .args(["-sS", "--fail", "-X", method])
            .args(["--aws-sigv4", "aws:amz:us-east-1:s3", "--user"])
            .arg(format!(
                "{}:{}",
                s3_var("VCON_TEST_S3_ACCESS_KEY"),
                s3_var("VCON_TEST_S3_SECRET_KEY")
            ))
            .arg(format!("{}/{path}", s3_var("VCON_TEST_S3_ENDPOINT")))
            .output()
            .expect("runs curl");
        (out.status.success(), out.stdout)
    }

    /// `command`, with the test service's keys in the environment only.
    fn with_keys(mut command: Command) -> Command {
        for name in ["AWS_PROFILE", "AWS_SESSION_TOKEN", "AWS_CONFIG_FILE"] {
            command.env_remove(name);
        }
        command
            .env("AWS_ACCESS_KEY_ID", s3_var("VCON_TEST_S3_ACCESS_KEY"))
            .env("AWS_SECRET_ACCESS_KEY", s3_var("VCON_TEST_S3_SECRET_KEY"))
            .env("AWS_SHARED_CREDENTIALS_FILE", "/nonexistent")
            .env("AWS_EC2_METADATA_DISABLED", "true");
        command
    }

    fn chmod_600(path: &Path) {
        std::fs::set_permissions(
            path,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("chmod 0600");
    }

    /// A deployment on a database and a bucket of the test's own, under
    /// the master key file `old.key`.
    struct World {
        dir: tempfile::TempDir,
        db: pg::Db,
        bucket: String,
        pem: String,
    }

    impl World {
        fn new() -> World {
            let dir = tempfile::tempdir().expect("temp dir");
            std::fs::create_dir(dir.path().join("data")).expect("data dir");
            for (name, byte) in [("old.key", 0x61), ("new.key", 0x62)] {
                std::fs::write(dir.path().join(name), [byte; 32]).expect("a master key");
                chmod_600(&dir.path().join(name));
            }
            let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
                .expect("a certificate");
            std::fs::write(dir.path().join("cert.pem"), made.cert.pem()).expect("write");
            std::fs::write(dir.path().join("key.pem"), made.signing_key.serialize_pem())
                .expect("write");
            chmod_600(&dir.path().join("key.pem"));
            std::fs::copy(pg::dir().join("password"), dir.path().join("pg.password"))
                .expect("copies the password");
            chmod_600(&dir.path().join("pg.password"));
            let bucket = format!("vcon-rot-{}", &TenantId::generate().to_string()[..12]);
            assert!(signed("PUT", &bucket).0, "the bucket is made");
            let w = World {
                dir,
                db: pg::Db::fresh(),
                bucket,
                pem: made.cert.pem(),
            };
            w.configure("old.key", "vcon_app", "vcon-server.toml");
            w.configure("old.key", "vcon_migrate", "vcon-migrate.toml");
            w
        }

        /// Writes the configuration `file`: the master key file `master`
        /// and the database login `user`.
        fn configure(&self, master: &str, user: &str, file: &str) {
            let at = |name: &str| self.dir.path().join(name);
            std::fs::write(
                at(file),
                format!(
                    "listen = \"127.0.0.1:0\"\ndata_dir = {:?}\nmaster_key = {:?}\n\n[tls]\ncert \
                     = {:?}\nkey = {:?}\n\n[storage]\nobjects = \"s3\"\n\n[storage.s3]\nendpoint \
                     = {:?}\nregion = \"us-east-1\"\nbucket = {:?}\npath_style = \
                     true\nallow_http = true\n\n[catalog]\nbackend = \
                     \"postgres\"\n\n[catalog.postgres]\nhosts = [\"localhost\"]\nport = \
                     {}\ndbname = {:?}\nuser = {user:?}\nca_file = {:?}\npassword_file = {:?}\n",
                    at("data"),
                    at(master),
                    at("cert.pem"),
                    at("key.pem"),
                    s3_var("VCON_TEST_S3_ENDPOINT"),
                    self.bucket,
                    var("VCON_TEST_PG_PORT"),
                    self.db.name,
                    pg::dir().join("ca.pem"),
                    at("pg.password"),
                ),
            )
            .expect("configuration");
        }

        fn command(&self, args: &[&str], config: &str) -> Command {
            let mut command = Command::new(env!("CARGO_BIN_EXE_vcon-server"));
            command
                .args(args)
                .arg("--config")
                .arg(self.dir.path().join(config));
            with_keys(command)
        }

        fn run(&self, args: &[&str]) -> Output {
            self.command(args, "vcon-server.toml")
                .output()
                .expect("the binary runs")
        }

        /// The lines a command that succeeds prints. What it printed is
        /// not shown when it fails: `key create` prints a token.
        fn ok(&self, args: &[&str]) -> Vec<Value> {
            let out = self.run(args);
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

        /// Starts a server and waits for its `serving` line.
        fn serve(&self) -> Server {
            let mut child = self
                .command(&["serve"], "vcon-server.toml")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("starts");
            let stderr = child.stderr.take().expect("piped");
            let (tx, lines) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(stderr)
                    .lines()
                    .map_while(Result::ok)
                {
                    // Shown when the test fails: what the server logged.
                    eprintln!("server: {line}");
                    let _ = tx.send(line);
                }
            });
            let mut server = Server {
                child,
                addr: None,
                pem: self.pem.clone(),
            };
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut said = Vec::new();
            while server.addr.is_none() {
                match lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(line) => {
                        let mut fields = line.split_whitespace();
                        if fields.any(|f| f == "serving") {
                            server.addr = fields
                                .find_map(|f| f.strip_prefix("listen="))
                                .and_then(|a| a.parse().ok());
                        }
                        said.push(line);
                    }
                    Err(_) => panic!("serve never said where it listens: {said:?}"),
                }
            }
            server
        }

        /// One value the schema's owner reads in a transaction with
        /// `vcon.tenant` set to `scope`.
        fn read(&self, scope: &str, select: &str) -> String {
            self.db
                .as_owner(&format!(
                    "BEGIN; SET LOCAL vcon.tenant = '{scope}'; {select}; COMMIT"
                ))
                .concat()
        }
    }

    /// A running server, stopped with SIGTERM, or killed when dropped.
    struct Server {
        child: Child,
        addr: Option<SocketAddr>,
        pem: String,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    impl Server {
        /// One request with the key `token`: the answer's status.
        fn request(&self, method: reqwest::Method, path: &str, token: &str, body: &str) -> u16 {
            let addr = self.addr.expect("the server listens");
            let client = reqwest::Client::builder()
                .tls_certs_only([reqwest::Certificate::from_pem(self.pem.as_bytes()).expect("pem")])
                .resolve("localhost", addr)
                .no_proxy()
                .build()
                .expect("a client");
            let url = format!("https://localhost:{}{path}", addr.port());
            let (token, body) = (token.to_owned(), body.to_owned());
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a runtime")
                    .block_on(async {
                        let mut request = client.request(method, url).bearer_auth(token);
                        if !body.is_empty() {
                            request = request
                                .header("content-type", "application/vcon")
                                .body(body);
                        }
                        request.send().await.expect("an answer").status().as_u16()
                    })
            })
            .join()
            .expect("the request's thread")
        }

        fn stop(mut self) {
            let pid = self.child.id();
            // SAFETY: the pid is our running child's.
            let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            assert_eq!(sent, 0, "SIGTERM is sent to the server");
            let status = self.child.wait().expect("ends");
            assert!(status.success(), "the server stopped cleanly: {status:?}");
        }
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len() / 2)
            .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).expect("hexadecimal"))
            .collect()
    }

    #[test]
    fn a_vcon_purged_before_a_rotation_cannot_be_recovered_from_an_old_copy_of_the_database() {
        let w = World::new();
        let migrated = w
            .command(&["migrate"], "vcon-migrate.toml")
            .output()
            .expect("runs");
        assert!(migrated.status.success(), "migrate: {migrated:?}");
        let tenant_text = w.ok(&["tenant", "create", "acme"])[0]["id"]
            .as_str()
            .expect("id")
            .to_owned();
        let tenant = TenantId::parse(&tenant_text).expect("tenant id");
        let token = w.ok(&[
            "key",
            "create",
            "--tenant",
            &tenant_text,
            "--scope",
            "write",
            "--scope",
            "read",
            "--scope",
            "delete",
            "--label",
            "producer",
        ])[0]["token"]
            .as_str()
            .expect("a token")
            .to_owned();
        let body = format!(
            r#"{{"uuid": "{UUID}", "created_at": "2026-10-25T12:00:00Z", "subject": "to be purged", "parties": []}}"#
        );
        let server = w.serve();
        assert_eq!(
            server.request(reqwest::Method::POST, "/vcons", &token, &body),
            201,
            "the vCon is stored"
        );
        server.stop();

        // The copy, taken while the vCon was kept: the tenant's wrapped
        // key, the version's wrapped data key, and the sealed object.
        let wrapped_tenant_key = unhex(&w.read(
            "registry",
            "SELECT encode(wrapped_tenant_key, 'hex') FROM vcon.tenant",
        ));
        let version_row = w.read(
            &tenant_text,
            "SELECT encode(version, 'hex') || ':' || encode(wrapped_key, 'hex') FROM vcon.version",
        );
        let (version_text, wrapped_data_key) = version_row.split_once(':').expect("one version");
        let version = VersionId::parse(version_text).expect("a version id");
        let wrapped_data_key = unhex(wrapped_data_key);
        let object = format!("{}/tenants/{tenant_text}/objects/{version_text}", w.bucket);
        let (found, sealed) = signed("GET", &object);
        assert!(found, "the copy holds the sealed object");
        let ctx = Context::Version {
            tenant,
            uuid: Uuid::parse_str(UUID).expect("uuid"),
            version,
        };

        // The vCon is deleted under a zero retention period: purged at
        // once.
        w.ok(&["tenant", "set-retention", &tenant_text, "0d"]);
        let server = w.serve();
        assert_eq!(
            server.request(
                reqwest::Method::DELETE,
                &format!("/vcons/{UUID}"),
                &token,
                ""
            ),
            204,
            "deleted"
        );
        server.stop();
        assert!(!signed("GET", &object).0, "the purge removed the object");
        assert_eq!(
            w.read(&tenant_text, "SELECT count(*) FROM vcon.version"),
            "0",
            "and the version's row"
        );

        // Until the rotation, anyone with the old master key reads the
        // copy.
        let old_master = Key::load_master(&w.dir.path().join("old.key")).expect("old");
        let old_tenant_key = envelope::unwrap(
            &old_master,
            &Context::Tenant { tenant },
            &wrapped_tenant_key,
        )
        .expect("the old tenant key");
        let data_key =
            envelope::unwrap(&old_tenant_key, &ctx, &wrapped_data_key).expect("the data key");
        let opened = envelope::open(&data_key, &ctx, &sealed).expect("the body opens");
        assert!(
            String::from_utf8_lossy(&opened).contains("to be purged"),
            "the copy really holds the vCon"
        );
        drop((old_master, old_tenant_key, data_key));

        // The rotation, as the operator guide gives it.
        assert_eq!(
            w.ok(&["check"])[0]["purges_awaiting_rotation"],
            1,
            "the purged vCon can still be read from the copy until the tenant key changes"
        );
        w.ok(&["rekey", "tenant", &tenant_text]);
        assert_eq!(
            w.ok(&["check"])[0]["purges_awaiting_rotation"],
            0,
            "rekeying the tenant ends the wait"
        );
        let new_path = w.dir.path().join("new.key");
        w.ok(&[
            "rekey",
            "master",
            "--new-master",
            new_path.to_str().expect("path"),
        ]);
        w.configure("new.key", "vcon_app", "vcon-server.toml");
        std::fs::remove_file(w.dir.path().join("old.key")).expect("the old master key destroyed");
        assert_eq!(
            w.ok(&["check"]).last().expect("summary")["problems"],
            0,
            "after the rotation the database checks clean under the new master alone"
        );

        // Every key that still exists, tried on the copy.
        let new_master = Key::load_master(&new_path).expect("new master");
        assert!(
            envelope::unwrap(
                &new_master,
                &Context::Tenant { tenant },
                &wrapped_tenant_key
            )
            .is_err(),
            "the copy's tenant key was wrapped under the old master, which is gone"
        );
        let live_wrapped = unhex(&w.read(
            "registry",
            "SELECT encode(wrapped_tenant_key, 'hex') FROM vcon.tenant",
        ));
        assert_ne!(
            live_wrapped, wrapped_tenant_key,
            "the database holds another wrap"
        );
        let tenant_key = envelope::unwrap(&new_master, &Context::Tenant { tenant }, &live_wrapped)
            .expect("the current tenant key");
        for key in [&new_master, &tenant_key] {
            assert!(
                envelope::unwrap(key, &ctx, &wrapped_data_key).is_err(),
                "no key that still exists unwraps the purged vCon's data key"
            );
            assert!(
                envelope::open(key, &ctx, &sealed).is_err(),
                "no key that still exists opens the purged vCon"
            );
        }
        // The server starts under the new key, and the vCon is gone.
        let server = w.serve();
        assert_eq!(
            server.request(reqwest::Method::GET, &format!("/vcons/{UUID}"), &token, ""),
            410,
            "the live server answers that it is gone"
        );
        server.stop();
    }
}
