//! The order of a start: the `serving` line (and, with an operator
//! listener, the line that it is bound) is logged before any background
//! job (the purge sweep, the pack run, the rotation warning) exists, so a
//! reader that waits for `serving` and then reads the log finds the start
//! lines first, every time.
//!
//! The order is observed without timing: a subscriber records, at each
//! line the start logs, how many tasks are alive on the runtime. A
//! background job is a spawned task, so a count of 0 at the `serving` line
//! means no job could have logged before it. A test that only compared
//! the order of the lines would pass or fail with the scheduler.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::audit::Actor;
use crate::clock::SystemClock;
use crate::config::Config;
use crate::envelope::Key;
use crate::fs::OsFs;
use crate::ids::KeyId;
use crate::retention::Retention;
use crate::serve::{ServeError, serve_opened};
use crate::store::Caller;
use crate::store::testing::vcon;
use crate::tenants::Tenants;

/// The serving line's message.
const SERVING: &str = "serving";
/// The operator listener's line, logged just before the serving line.
const OPERATOR_BOUND: &str = "the operator listener is bound";

/// One line a start logged, with the tasks alive on the runtime when it
/// was logged (`None` off a runtime, as on a blocking thread).
#[derive(Debug, Clone)]
struct Seen {
    message: String,
    fields: Vec<(String, String)>,
    tasks: Option<usize>,
}

impl Seen {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A subscriber that keeps every line logged on its thread, and wakes
/// whoever waits for the serving line.
#[derive(Clone, Default)]
struct Watch {
    seen: Arc<Mutex<Vec<Seen>>>,
    serving: Arc<Notify>,
}

#[derive(Default)]
struct Fields {
    message: String,
    fields: Vec<(String, String)>,
}

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .push((field.name().to_owned(), format!("{value:?}")));
        }
    }
}

impl tracing::Subscriber for Watch {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let tasks = tokio::runtime::Handle::try_current()
            .ok()
            .map(|h| h.metrics().num_alive_tasks());
        let serving = fields.message == SERVING;
        self.seen.lock().expect("not poisoned").push(Seen {
            message: fields.message,
            fields: fields.fields,
            tasks,
        });
        if serving {
            self.serving.notify_one();
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// A self-signed certificate for localhost and its key, as PEM files in
/// `dir`.
fn certificate(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let made =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).expect("a certificate");
    let (cert, key) = (
        dir.join(format!("{name}-cert.pem")),
        dir.join(format!("{name}-key.pem")),
    );
    std::fs::write(&cert, made.cert.pem()).expect("write the certificate");
    std::fs::write(&key, made.signing_key.serialize_pem()).expect("write the key");
    std::fs::set_permissions(
        &key,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    )
    .expect("chmod 0600");
    (cert, key)
}

/// A data directory with an opened registry, and a configuration for it.
struct World {
    dir: tempfile::TempDir,
    config: Config,
}

/// The configuration, with `extra` after the `[tls]` table.
fn world(extra: &str) -> World {
    let dir = tempfile::tempdir().expect("temp dir");
    let (cert, key) = certificate(dir.path(), "tenant");
    let text = format!(
        "listen = \"127.0.0.1:0\"\ndata_dir = {:?}\nmaster_key = {:?}\n\n[tls]\ncert = {:?}\nkey \
         = {:?}\n{extra}",
        dir.path(),
        dir.path().join("master.key"),
        cert,
        key,
    );
    let config = crate::config::parse(&text).expect("the configuration reads");
    World { dir, config }
}

/// A configuration with an operator listener on its own certificate.
fn world_with_operator() -> World {
    let dir = tempfile::tempdir().expect("temp dir");
    let (cert, key) = certificate(dir.path(), "tenant");
    let (op_cert, op_key) = certificate(dir.path(), "operator");
    let text = format!(
        "listen = \"127.0.0.1:0\"\ndata_dir = {:?}\nmaster_key = {:?}\n\n[tls]\ncert = {:?}\nkey \
         = {:?}\n\n[operator]\nlisten = \"127.0.0.1:0\"\n\n[operator.tls]\ncert = {:?}\nkey = \
         {:?}\n",
        dir.path(),
        dir.path().join("master.key"),
        cert,
        key,
        op_cert,
        op_key,
    );
    let config = crate::config::parse(&text).expect("the configuration reads");
    World { dir, config }
}

/// The registry of `w`'s data directory, under its master key file; a
/// second one opened beside a running start reads what that start did.
fn tenants(w: &World) -> Tenants {
    let master = w.dir.path().join("master.key");
    if !master.exists() {
        let mut bytes = [0u8; 32];
        std::io::Read::read_exact(
            &mut std::fs::File::open("/dev/urandom").expect("urandom"),
            &mut bytes,
        )
        .expect("32 random bytes");
        std::fs::write(&master, bytes).expect("write the master key");
        std::fs::set_permissions(
            &master,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("chmod 0600");
    }
    Tenants::open(
        w.dir.path(),
        Key::load_master(&master).expect("the master key"),
        Arc::new(OsFs),
        Arc::new(SystemClock),
    )
    .expect("the registry opens")
}

/// The runtime a start runs on.
#[derive(Debug, Clone, Copy)]
enum Flavor {
    CurrentThread,
    MultiThread,
}

fn runtime(flavor: Flavor) -> tokio::runtime::Runtime {
    match flavor {
        Flavor::CurrentThread => tokio::runtime::Builder::new_current_thread(),
        Flavor::MultiThread => {
            let mut b = tokio::runtime::Builder::new_multi_thread();
            b.worker_threads(4);
            b
        }
    }
    .enable_all()
    .build()
    .expect("a runtime")
}

/// What a start did: the lines it logged, the tasks alive once the
/// serving line was out and the start went on, the tasks alive after it
/// ended, and how it ended.
struct Ran {
    seen: Vec<Seen>,
    tasks_after_serving: Option<usize>,
    tasks_at_end: usize,
    ended: Result<(), ServeError>,
}

impl Ran {
    fn messages(&self) -> Vec<&str> {
        self.seen.iter().map(|s| s.message.as_str()).collect()
    }
    fn line(&self, message: &str) -> &Seen {
        let found: Vec<&Seen> = self.seen.iter().filter(|s| s.message == message).collect();
        assert_eq!(
            found.len(),
            1,
            "one {message:?} line: {:?}",
            self.messages()
        );
        found[0]
    }
}

/// Runs `serve_opened` until its serving line is out, then until `more`
/// completes, then stops it.
fn run_with(
    config: &Config,
    tenants: Tenants,
    flavor: Flavor,
    more: impl std::future::Future<Output = ()>,
) -> Ran {
    let watch = Watch::default();
    let rt = runtime(flavor);
    let after = Arc::new(Mutex::new(None));
    let ended = {
        let _guard = tracing::subscriber::set_default(watch.clone());
        let (serving, after) = (watch.serving.clone(), after.clone());
        rt.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(30),
                serve_opened(config, tenants, async move {
                    serving.notified().await;
                    *after.lock().expect("not poisoned") = Some(
                        tokio::runtime::Handle::current()
                            .metrics()
                            .num_alive_tasks(),
                    );
                    more.await;
                }),
            )
            .await
            .expect("the start stopped within 30 s")
        })
    };
    let tasks_at_end = rt.metrics().num_alive_tasks();
    let seen = watch.seen.lock().expect("not poisoned").clone();
    let tasks_after_serving = *after.lock().expect("not poisoned");
    Ran {
        seen,
        tasks_after_serving,
        tasks_at_end,
        ended,
    }
}

fn run(config: &Config, tenants: Tenants, flavor: Flavor) -> Ran {
    run_with(config, tenants, flavor, async {})
}

/// Runs a start that is refused: no serving line comes, so it is not
/// waited for.
fn refused(config: &Config, tenants: Tenants) -> Ran {
    let watch = Watch::default();
    let rt = runtime(Flavor::CurrentThread);
    let ended = {
        let _guard = tracing::subscriber::set_default(watch.clone());
        rt.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(30),
                serve_opened(config, tenants, std::future::pending()),
            )
            .await
            .expect("the refusal came within 30 s")
        })
    };
    let tasks_at_end = rt.metrics().num_alive_tasks();
    let seen = watch.seen.lock().expect("not poisoned").clone();
    Ran {
        seen,
        tasks_after_serving: None,
        tasks_at_end,
        ended,
    }
}

/// A listener holding a port, so a start on it is refused.
fn taken() -> (std::net::TcpListener, SocketAddr) {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a port");
    let addr = held.local_addr().expect("its address");
    (held, addr)
}

mod order {
    use super::*;

    #[test]
    fn no_background_job_exists_at_the_serving_line_on_one_thread() {
        let w = world("");
        let ran = run(&w.config, tenants(&w), Flavor::CurrentThread);
        assert_eq!(
            ran.line(SERVING).tasks,
            Some(0),
            "no task spawned before the serving line: {:?}",
            ran.seen
        );
    }

    #[test]
    fn no_background_job_exists_at_the_serving_line_on_worker_threads() {
        let w = world("");
        let ran = run(&w.config, tenants(&w), Flavor::MultiThread);
        assert_eq!(
            ran.line(SERVING).tasks,
            Some(0),
            "no task spawned before the serving line: {:?}",
            ran.seen
        );
    }

    #[test]
    fn no_background_job_exists_at_either_line_of_an_operator_listener_start() {
        let w = world_with_operator();
        let ran = run(&w.config, tenants(&w), Flavor::MultiThread);
        for message in [OPERATOR_BOUND, SERVING] {
            assert_eq!(
                ran.line(message).tasks,
                Some(0),
                "no task spawned before {message:?}: {:?}",
                ran.seen
            );
        }
    }

    #[test]
    fn the_operator_line_comes_just_before_the_serving_line() {
        let w = world_with_operator();
        let ran = run(&w.config, tenants(&w), Flavor::CurrentThread);
        let messages = ran.messages();
        let at = messages
            .iter()
            .position(|m| *m == SERVING)
            .expect("a serving line");
        assert!(at > 0, "a line before serving: {messages:?}");
        assert_eq!(
            messages[at - 1],
            OPERATOR_BOUND,
            "the operator line just before serving: {messages:?}"
        );
    }

    #[test]
    fn the_background_jobs_are_running_once_the_start_goes_on() {
        let w = world("");
        let ran = run(&w.config, tenants(&w), Flavor::CurrentThread);
        let after = ran.tasks_after_serving.expect("the start went on");
        assert!(
            after >= 2,
            "the sweeper and the rotation warner run after the serving line: {after} task(s)"
        );
    }

    #[test]
    fn the_background_jobs_are_running_beside_an_operator_listener() {
        let w = world_with_operator();
        let ran = run(&w.config, tenants(&w), Flavor::MultiThread);
        let after = ran.tasks_after_serving.expect("the start went on");
        assert!(
            after >= 2,
            "the sweeper and the rotation warner run after the serving line: {after} task(s)"
        );
    }

    #[test]
    fn the_first_sweep_still_runs_at_start() {
        let w = world("");
        let registry = tenants(&w);
        let id = registry.create(Actor::Cli, "acme", None).expect("a tenant");
        let store = registry.store(id).expect("its store");
        let caller = Caller { key: KeyId([7; 8]) };
        let (found, body) = vcon("8a3b4c5d-0000-4000-8000-000000000001", "due");
        store.create(&caller, &found, &body).expect("stored");
        store
            .delete(&caller, found.uuid, Retention::Years(7))
            .expect("deleted");
        registry
            .set_retention(Actor::Cli, id, Some(Retention::Days(0)))
            .expect("due at once");
        drop(store);
        let polled = tenants(&w);
        let ran = run_with(&w.config, registry, Flavor::MultiThread, async move {
            let deadline = Instant::now() + Duration::from_secs(20);
            while polled
                .store(id)
                .expect("its store")
                .counts()
                .expect("counts")
                .versions
                > 0
            {
                assert!(Instant::now() < deadline, "no sweep within 20 s");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        assert!(
            ran.ended.is_ok(),
            "it stopped cleanly: {:?}",
            ran.ended.err()
        );
    }
}

/// A start refused before it serves logs no serving line and leaves no
/// background job running.
mod refused {
    use super::*;

    use crate::api::TlsError;

    /// Asserts that `ran` served nothing and started no job.
    fn nothing_started(ran: &Ran) {
        assert!(
            !ran.messages().contains(&SERVING),
            "no serving line: {:?}",
            ran.messages()
        );
        assert!(
            !ran.messages().contains(&OPERATOR_BOUND),
            "no operator line: {:?}",
            ran.messages()
        );
        assert_eq!(ran.tasks_at_end, 0, "no background job left running");
    }

    #[test]
    fn a_missing_certificate_refuses_before_any_job() {
        let mut w = world("");
        w.config.tls_cert = w.dir.path().join("missing.pem");
        let ran = refused(&w.config, tenants(&w));
        assert!(
            matches!(ran.ended, Err(ServeError::Tls(TlsError::Certificate))),
            "refused for its certificate: {:?}",
            ran.ended.err()
        );
        nothing_started(&ran);
    }

    #[test]
    fn an_empty_certificate_refuses_before_any_job() {
        let mut w = world("");
        let empty = w.dir.path().join("empty.pem");
        std::fs::write(&empty, "").expect("write");
        w.config.tls_cert = empty;
        let ran = refused(&w.config, tenants(&w));
        assert!(
            matches!(ran.ended, Err(ServeError::Tls(TlsError::Certificate))),
            "refused for its certificate: {:?}",
            ran.ended.err()
        );
        nothing_started(&ran);
    }

    #[test]
    fn a_key_others_may_read_refuses_before_any_job() {
        let w = world("");
        std::fs::set_permissions(
            &w.config.tls_key,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o644),
        )
        .expect("chmod 0644");
        let ran = refused(&w.config, tenants(&w));
        assert!(
            matches!(ran.ended, Err(ServeError::Tls(TlsError::KeyFile(_)))),
            "refused for its key file: {:?}",
            ran.ended.err()
        );
        nothing_started(&ran);
    }

    #[test]
    fn a_key_that_is_not_pem_refuses_before_any_job() {
        let w = world("");
        std::fs::write(&w.config.tls_key, "not a key").expect("write");
        let ran = refused(&w.config, tenants(&w));
        assert!(
            matches!(ran.ended, Err(ServeError::Tls(TlsError::Key))),
            "refused for its key: {:?}",
            ran.ended.err()
        );
        nothing_started(&ran);
    }

    #[test]
    fn a_taken_address_refuses_before_any_job() {
        let mut w = world("");
        let (_held, addr) = taken();
        w.config.listen = addr;
        let ran = refused(&w.config, tenants(&w));
        assert!(
            matches!(ran.ended, Err(ServeError::Listen(_))),
            "refused for its address: {:?}",
            ran.ended.err()
        );
        nothing_started(&ran);
    }

    #[test]
    fn a_taken_operator_address_refuses_before_any_job() {
        let mut w = world_with_operator();
        let (_held, addr) = taken();
        w.config
            .operator
            .as_mut()
            .expect("an operator listener")
            .listen = addr;
        let ran = refused(&w.config, tenants(&w));
        assert!(
            matches!(ran.ended, Err(ServeError::OperatorListen(_))),
            "refused for the operator address: {:?}",
            ran.ended.err()
        );
        nothing_started(&ran);
    }

    #[test]
    fn a_missing_operator_certificate_refuses_before_any_job() {
        let mut w = world_with_operator();
        w.config
            .operator
            .as_mut()
            .expect("an operator listener")
            .tls_cert = w.dir.path().join("missing.pem");
        let ran = refused(&w.config, tenants(&w));
        assert!(
            matches!(
                ran.ended,
                Err(ServeError::OperatorTls(TlsError::Certificate))
            ),
            "refused for the operator certificate: {:?}",
            ran.ended.err()
        );
        nothing_started(&ran);
    }
}

mod edges {
    use super::*;

    #[test]
    fn a_stop_at_the_serving_line_ends_every_job() {
        let w = world("");
        let ran = run(&w.config, tenants(&w), Flavor::MultiThread);
        assert!(
            ran.ended.is_ok(),
            "it stopped cleanly: {:?}",
            ran.ended.err()
        );
        assert_eq!(ran.tasks_at_end, 0, "no job outlives the start");
    }

    #[test]
    fn a_stop_beside_an_operator_listener_ends_every_job() {
        let w = world_with_operator();
        let ran = run(&w.config, tenants(&w), Flavor::CurrentThread);
        assert!(
            ran.ended.is_ok(),
            "it stopped cleanly: {:?}",
            ran.ended.err()
        );
        assert_eq!(ran.tasks_at_end, 0, "no job outlives the start");
    }

    #[test]
    fn a_one_second_purge_interval_still_logs_serving_first() {
        let mut w = world("");
        w.config.purge_interval = Duration::from_secs(1);
        let ran = run(&w.config, tenants(&w), Flavor::MultiThread);
        assert_eq!(
            ran.line(SERVING).tasks,
            Some(0),
            "no task spawned before the serving line: {:?}",
            ran.seen
        );
    }

    #[test]
    fn the_serving_line_names_the_port_bound_not_port_0() {
        let w = world("");
        let ran = run(&w.config, tenants(&w), Flavor::CurrentThread);
        let listen: SocketAddr = ran
            .line(SERVING)
            .field("listen")
            .expect("a listen field")
            .parse()
            .expect("an address");
        assert_ne!(listen.port(), 0, "the port the system chose");
    }

    #[test]
    fn the_operator_line_names_its_own_port_bound() {
        let w = world_with_operator();
        let ran = run(&w.config, tenants(&w), Flavor::CurrentThread);
        let operator: SocketAddr = ran
            .line(OPERATOR_BOUND)
            .field("operator_listen")
            .expect("an operator_listen field")
            .parse()
            .expect("an address");
        let listen: SocketAddr = ran
            .line(SERVING)
            .field("listen")
            .expect("a listen field")
            .parse()
            .expect("an address");
        assert_ne!(operator.port(), 0, "the port the system chose");
        assert_ne!(operator, listen, "two listeners, two addresses");
    }

    #[test]
    fn only_the_start_lines_come_before_the_serving_line() {
        let w = world("");
        let ran = run(&w.config, tenants(&w), Flavor::MultiThread);
        let messages = ran.messages();
        let at = messages
            .iter()
            .position(|m| *m == SERVING)
            .expect("a serving line");
        assert_eq!(at, 0, "the serving line is the first: {messages:?}");
    }
}
