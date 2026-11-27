//! A stop while a background job's run is under way: `run_opened` waits
//! for the run to end before it gives up the data directory's lock and
//! returns, so no purge, pack or manifest write of the old server goes on
//! after a next server or a command could start.
//!
//! In the tests of `run_opened`, the run is held inside its step for a
//! tenant by a test hook, the point where the purge sweep and the pack run
//! write. The held step waits for one of two events, never a time: the
//! stop's line that it waits for the run, or `run_opened` returning. Then
//! it reads whether the data directory is still locked. A stop that waits
//! wakes it with its line, with the lock held; a stop that does not wait
//! returns first, with the lock given up.
//!
//! The tests of [`Background`] hold a run of a work of their own, and the
//! tests of `sweep` and `pack_all` drive the stop test those are given.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use time::macros::datetime;
use tokio::sync::Notify;

use super::start_tests::{World, beside, tenants, world};
use crate::audit::Actor;
use crate::backend::Catalogs;
use crate::config::{Config, ParquetSettings, READER_GRACE};
use crate::datadir::{self, Sharing};
use crate::ids::{KeyId, TenantId};
use crate::retention::Retention;
use crate::serve::{
    Background, PackRunReport, STOP_WARN, ServeError, SweepReport, pack_all, run_opened, sweep,
};
use crate::store::Caller;
use crate::store::testing::vcon;
use crate::tenants::{Tenants, fixture};

/// The line a stop logs when it waits for a background job's run.
const STOP_WAITS: &str = "stopping: waiting for a background job's run to end";
/// The line a stop logs each time STOP_WARN passes while it still waits.
const STILL_WAITS: &str =
    "stopping: a background job's run has not ended; the stop goes on waiting";
/// The line a run that panicked is logged with.
const FAILED: &str = "a background job's run failed";

/// Patience for an event that is due at once; a test that reaches it has
/// failed.
const PATIENCE: Duration = Duration::from_secs(60);

/// One line logged: its message and its other fields.
#[derive(Debug, Clone, Default)]
struct Line {
    message: String,
    fields: Vec<(String, String)>,
}

impl Line {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A subscriber that keeps each line logged on its thread, and sends a
/// wake at the stop's waiting line for `job` (any job's, without one).
#[derive(Clone)]
struct Lines {
    seen: Arc<Mutex<Vec<Line>>>,
    wake: mpsc::Sender<&'static str>,
    job: Option<String>,
}

impl Lines {
    fn new() -> (Lines, mpsc::Receiver<&'static str>) {
        Lines::waking_for(None)
    }

    /// Lines that wake at the stop's waiting line for `job` only: a stop
    /// may wait for another job's run first.
    fn waking_for(job: Option<&str>) -> (Lines, mpsc::Receiver<&'static str>) {
        let (wake, woken) = mpsc::channel();
        let lines = Lines {
            seen: Arc::new(Mutex::new(Vec::new())),
            wake,
            job: job.map(|j| format!("{j:?}")),
        };
        (lines, woken)
    }

    /// The lines whose message is `message`.
    fn with(&self, message: &str) -> Vec<Line> {
        self.seen
            .lock()
            .expect("not poisoned")
            .iter()
            .filter(|l| l.message == message)
            .cloned()
            .collect()
    }
}

#[derive(Default)]
struct Fields(Line);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.message = format!("{value:?}");
        } else {
            self.0
                .fields
                .push((field.name().to_owned(), format!("{value:?}")));
        }
    }
}

impl tracing::Subscriber for Lines {
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
        if fields.0.message == STOP_WAITS
            && self
                .job
                .as_deref()
                .is_none_or(|job| fields.0.field("job") == Some(job))
        {
            let _ = self.wake.send("the stop's waiting line");
        }
        self.seen.lock().expect("not poisoned").push(fields.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// The runtime a stop runs on.
#[derive(Debug, Clone, Copy)]
enum Flavor {
    CurrentThread,
    MultiThread,
}

/// What one held stop did, in order, the steps the jobs began, the lines
/// logged on the stop's thread, and how `run_opened` ended.
struct Held {
    events: Vec<String>,
    steps: Vec<&'static str>,
    lines: Lines,
    ended: Result<(), ServeError>,
}

/// Runs `run_opened` over `registry`, holds the first step of `job` for a
/// tenant, stops the server while it is held, and records what came in
/// which order.
fn held_stop(w: &World, config: &Config, registry: Tenants, job: &'static str) -> Held {
    held_stop_on(w, config, registry, job, Flavor::CurrentThread)
}

fn held_stop_on(
    w: &World,
    config: &Config,
    registry: Tenants,
    job: &'static str,
    flavor: Flavor,
) -> Held {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let steps = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let note = |events: &Arc<Mutex<Vec<String>>>, what: String| {
        events.lock().expect("not poisoned").push(what);
    };
    let (lines, woken) = Lines::waking_for(Some(job));
    let wake = lines.wake.clone();
    let woken = Mutex::new(woken);
    let (done, is_done) = mpsc::channel::<()>();
    let done = Mutex::new(done);
    let entered = Arc::new(Notify::new());
    let held_once = AtomicBool::new(false);
    let dir = w.dir.path().to_owned();
    {
        let (events, steps, entered) = (events.clone(), steps.clone(), entered.clone());
        registry.when_a_job_steps_for_test(Arc::new(move |name| {
            steps.lock().expect("not poisoned").push(name);
            if name != job || held_once.swap(true, Ordering::SeqCst) {
                return;
            }
            note(&events, format!("{name} held in its step"));
            entered.notify_one();
            let why = woken
                .lock()
                .expect("not poisoned")
                .recv_timeout(PATIENCE)
                .unwrap_or("nothing within 60 s");
            note(&events, format!("woken by {why}"));
            let free = datadir::lock(&dir, Sharing::Alone).is_ok();
            note(
                &events,
                if free {
                    "the data directory was free".to_owned()
                } else {
                    "the data directory was locked".to_owned()
                },
            );
            note(&events, format!("{name} step released"));
            let _ = done.lock().expect("not poisoned").send(());
        }));
    }
    let lock = datadir::lock(w.dir.path(), Sharing::Serve).expect("the data directory locks");
    let rt = match flavor {
        Flavor::CurrentThread => tokio::runtime::Builder::new_current_thread(),
        Flavor::MultiThread => {
            let mut b = tokio::runtime::Builder::new_multi_thread();
            b.worker_threads(4);
            b
        }
    }
    .enable_all()
    .build()
    .expect("a runtime");
    let ended = {
        let _guard = tracing::subscriber::set_default(lines.clone());
        rt.block_on(async {
            tokio::time::timeout(
                PATIENCE,
                run_opened(config, (lock, Catalogs::Local, registry), async move {
                    entered.notified().await;
                }),
            )
            .await
            .expect("the server stopped within 60 s")
        })
    };
    note(&events, "run_opened returned".to_owned());
    let _ = wake.send("run_opened returning");
    if is_done.recv_timeout(PATIENCE).is_err() {
        note(&events, "the held step did not end within 60 s".to_owned());
    }
    drop(rt);
    let events = events.lock().expect("not poisoned").clone();
    let steps = steps.lock().expect("not poisoned").clone();
    Held {
        events,
        steps,
        lines,
        ended,
    }
}

const CALLER: Caller = Caller { key: KeyId([7; 8]) };

/// A tenant of `registry` with one deleted vCon, due for purge at once.
fn due(registry: &Tenants, name: &str, uuid: &str) -> TenantId {
    let id = registry.create(Actor::Cli, name, None).expect("a tenant");
    let store = registry.store(id).expect("its store");
    let (found, body) = vcon(uuid, "due");
    store.create(&CALLER, &found, &body).expect("stored");
    store
        .delete(&CALLER, found.uuid, Retention::Years(7))
        .expect("deleted");
    registry
        .set_retention(Actor::Cli, id, Some(Retention::Days(0)))
        .expect("due at once");
    id
}

/// The versions `id` keeps, read beside the stopped server.
fn kept(w: &World, id: TenantId) -> u64 {
    beside(w)
        .store(id)
        .expect("its store")
        .counts()
        .expect("counts")
        .versions
}

/// The configuration of `w` with a pack run: Parquet settings on local
/// objects, which a configuration file refuses, so they are set here. The
/// run's step for a tenant is still where it would write.
fn packing(w: &World) -> Config {
    let mut config = w.config.clone();
    config.storage.parquet = Some(SETTINGS);
    config
}

const SETTINGS: ParquetSettings = ParquetSettings {
    pack_interval: Duration::from_secs(300),
    pack_limit: 1 << 20,
    reader_grace: READER_GRACE,
};

/// The order a stop that waits gives, for the held `job`.
fn waited(job: &str) -> Vec<String> {
    vec![
        format!("{job} held in its step"),
        "woken by the stop's waiting line".to_owned(),
        "the data directory was locked".to_owned(),
        format!("{job} step released"),
        "run_opened returned".to_owned(),
    ]
}

/// Asserts that the stop held by `held` waited for `job`'s run.
fn assert_waited(held: &Held, job: &str) {
    assert!(
        held.ended.is_ok(),
        "it stopped cleanly: {:?}",
        held.ended.as_ref().err()
    );
    assert_eq!(
        held.events,
        waited(job),
        "the stop waits for the {job}'s step, with the data directory locked, and returns after it"
    );
}

/// The stop of a server held in the sweep's step.
mod sweep_stop {
    use super::*;

    #[test]
    fn a_stop_during_a_sweep_returns_only_after_the_sweep_has_ended() {
        let w = world("");
        let registry = tenants(&w);
        due(&registry, "acme", "8a3b4c5d-0000-4000-8000-000000000001");
        let held = held_stop(&w, &w.config, registry, "purge sweep");
        assert_waited(&held, "purge sweep");
    }

    #[test]
    fn a_stop_during_a_sweep_on_worker_threads_returns_only_after_it() {
        let w = world("");
        let registry = tenants(&w);
        due(&registry, "acme", "8a3b4c5d-0000-4000-8000-000000000002");
        let held = held_stop_on(&w, &w.config, registry, "purge sweep", Flavor::MultiThread);
        assert_waited(&held, "purge sweep");
    }

    #[test]
    fn the_stop_s_waiting_line_names_the_purge_sweep() {
        let w = world("");
        let registry = tenants(&w);
        due(&registry, "acme", "8a3b4c5d-0000-4000-8000-00000000000a");
        let held = held_stop(&w, &w.config, registry, "purge sweep");
        assert_waited(&held, "purge sweep");
        assert_names(&held, "purge sweep");
    }

    #[test]
    fn the_purge_under_way_at_the_stop_is_finished_before_it_returns() {
        let w = world("");
        let registry = tenants(&w);
        let id = due(&registry, "acme", "8a3b4c5d-0000-4000-8000-000000000003");
        let held = held_stop(&w, &w.config, registry, "purge sweep");
        assert_waited(&held, "purge sweep");
        assert_eq!(
            kept(&w, id),
            0,
            "the tenant's step that had begun purged its version before the stop returned"
        );
    }

    #[test]
    fn a_stop_during_a_tenant_s_step_waits_for_one_version_not_the_tenant() {
        let w = world("");
        let registry = tenants(&w);
        let id = due(&registry, "acme", "8a3b4c5d-0000-4000-8000-000000000010");
        let store = registry.store(id).expect("its store");
        for n in 11..14 {
            let (found, body) = vcon(&format!("8a3b4c5d-0000-4000-8000-0000000000{n}"), "due");
            store.create(&CALLER, &found, &body).expect("stored");
            store
                .delete(&CALLER, found.uuid, Retention::Years(7))
                .expect("deleted");
        }
        drop(store);
        let held = held_stop(&w, &w.config, registry, "purge sweep");
        assert_waited(&held, "purge sweep");
        assert_eq!(
            kept(&w, id),
            3,
            "the step purged the version under way at the stop, and left the other three"
        );
    }

    #[test]
    fn a_sweep_stopped_in_one_tenant_s_step_begins_no_other() {
        let w = world("");
        let registry = tenants(&w);
        let one = due(&registry, "acme", "8a3b4c5d-0000-4000-8000-000000000004");
        let two = due(&registry, "globex", "8a3b4c5d-0000-4000-8000-000000000005");
        let held = held_stop(&w, &w.config, registry, "purge sweep");
        assert_waited(&held, "purge sweep");
        let sweeps = held.steps.iter().filter(|s| **s == "purge sweep").count();
        assert_eq!(
            sweeps, 1,
            "one tenant's step, then the run ends: {:?}",
            held.steps
        );
        assert_eq!(
            kept(&w, one) + kept(&w, two),
            1,
            "the tenant whose step had begun was purged, the other waits for a later sweep"
        );
    }

    #[test]
    fn the_data_directory_is_free_once_the_stop_has_returned() {
        let w = world("");
        let registry = tenants(&w);
        due(&registry, "acme", "8a3b4c5d-0000-4000-8000-000000000006");
        let held = held_stop(&w, &w.config, registry, "purge sweep");
        assert_waited(&held, "purge sweep");
        assert!(
            datadir::lock(w.dir.path(), Sharing::Alone).is_ok(),
            "a command that runs alone takes the data directory after the stop"
        );
    }

    /// A step that fails while the stop waits (its tenant's retention no
    /// longer reads) is still waited for, and the stop ends cleanly.
    #[test]
    fn a_sweep_step_that_fails_during_the_stop_is_still_waited_for() {
        let w = world("");
        let registry = tenants(&w);
        let id = due(&registry, "acme", "8a3b4c5d-0000-4000-8000-000000000007");
        registry
            .conn_for_test()
            .execute("UPDATE tenant SET retention = '7w' WHERE id = ?1", [id.0])
            .expect("damaged");
        let held = held_stop(&w, &w.config, registry, "purge sweep");
        assert_waited(&held, "purge sweep");
        assert_eq!(kept(&w, id), 1, "the failed step purged nothing");
    }

    /// A step whose tenant is being removed fails to open its Store, and
    /// the stop still waits for it.
    #[test]
    fn a_sweep_step_whose_tenant_is_closing_is_still_waited_for() {
        let w = world("");
        let registry = tenants(&w);
        let id = due(&registry, "acme", "8a3b4c5d-0000-4000-8000-000000000008");
        registry.close(id);
        let held = held_stop(&w, &w.config, registry, "purge sweep");
        assert_waited(&held, "purge sweep");
        assert_eq!(kept(&w, id), 1, "the closing tenant was not swept");
    }
}

/// The stop of a server held in the pack run's step.
mod pack_stop {
    use super::*;

    #[test]
    fn a_stop_during_a_pack_run_returns_only_after_the_run_has_ended() {
        let w = world("");
        let registry = tenants(&w);
        registry.create(Actor::Cli, "acme", None).expect("a tenant");
        let held = held_stop(&w, &packing(&w), registry, "pack run");
        assert_waited(&held, "pack run");
    }

    #[test]
    fn a_stop_during_a_pack_run_on_worker_threads_returns_only_after_it() {
        let w = world("");
        let registry = tenants(&w);
        registry.create(Actor::Cli, "acme", None).expect("a tenant");
        let held = held_stop_on(&w, &packing(&w), registry, "pack run", Flavor::MultiThread);
        assert_waited(&held, "pack run");
    }

    #[test]
    fn a_pack_run_stopped_in_one_tenant_s_step_begins_no_other() {
        let w = world("");
        let registry = tenants(&w);
        for name in ["acme", "globex", "initech"] {
            registry.create(Actor::Cli, name, None).expect("a tenant");
        }
        let held = held_stop(&w, &packing(&w), registry, "pack run");
        assert_waited(&held, "pack run");
        let packs = held.steps.iter().filter(|s| **s == "pack run").count();
        assert_eq!(
            packs, 1,
            "one tenant's step, then the run ends: {:?}",
            held.steps
        );
    }

    #[test]
    fn the_data_directory_is_free_once_a_pack_stop_has_returned() {
        let w = world("");
        let registry = tenants(&w);
        registry.create(Actor::Cli, "acme", None).expect("a tenant");
        let held = held_stop(&w, &packing(&w), registry, "pack run");
        assert_waited(&held, "pack run");
        assert!(
            datadir::lock(w.dir.path(), Sharing::Alone).is_ok(),
            "a command that runs alone takes the data directory after the stop"
        );
    }

    /// A pack step whose tenant is being removed fails, and the stop still
    /// waits for it.
    #[test]
    fn a_pack_step_whose_tenant_is_closing_is_still_waited_for() {
        let w = world("");
        let registry = tenants(&w);
        let id = registry.create(Actor::Cli, "acme", None).expect("a tenant");
        registry.close(id);
        let held = held_stop(&w, &packing(&w), registry, "pack run");
        assert_waited(&held, "pack run");
    }

    #[test]
    fn the_stop_s_waiting_line_names_the_pack_run() {
        let w = world("");
        let registry = tenants(&w);
        registry.create(Actor::Cli, "acme", None).expect("a tenant");
        let held = held_stop(&w, &packing(&w), registry, "pack run");
        assert_waited(&held, "pack run");
        assert_names(&held, "pack run");
    }
}

/// Asserts that the stop held by `held` logged its waiting line for `job`.
fn assert_names(held: &Held, job: &str) {
    let named: Vec<Option<String>> = held
        .lines
        .with(STOP_WAITS)
        .iter()
        .map(|l| l.field("job").map(str::to_owned))
        .collect();
    assert!(
        named.contains(&Some(format!("{job:?}"))),
        "a waiting line names the {job}: {named:?}"
    );
}

/// A work for [`Background`] whose first run can be held until released.
struct Gate {
    began: tokio::sync::mpsc::UnboundedReceiver<()>,
    release: mpsc::Sender<()>,
    /// What `stopping` answered at the end of each run.
    answers: Arc<Mutex<Vec<bool>>>,
    runs: Arc<AtomicUsize>,
    ended: Arc<AtomicUsize>,
}

/// How a gated work behaves.
#[derive(Clone, Copy)]
struct Mode {
    /// The first run waits for the release.
    hold: bool,
    /// The first run panics at its end.
    panic: bool,
}

const HOLD: Mode = Mode {
    hold: true,
    panic: false,
};

/// A work that counts its runs and, as `mode` says, holds or panics in
/// its first.
fn gated(mode: Mode) -> (Gate, impl Fn(&dyn Fn() -> bool) + Send + Sync + 'static) {
    let (began, is_begun) = tokio::sync::mpsc::unbounded_channel();
    let (release, released) = mpsc::channel();
    let released = Mutex::new(released);
    let answers = Arc::new(Mutex::new(Vec::new()));
    let runs = Arc::new(AtomicUsize::new(0));
    let ended = Arc::new(AtomicUsize::new(0));
    let (a, r, e) = (answers.clone(), runs.clone(), ended.clone());
    let work = move |stopping: &dyn Fn() -> bool| {
        let first = r.fetch_add(1, Ordering::SeqCst) == 0;
        let _ = began.send(());
        if first && mode.hold {
            released
                .lock()
                .expect("not poisoned")
                .recv_timeout(PATIENCE)
                .expect("released within 60 s");
        }
        a.lock().expect("not poisoned").push(stopping());
        e.fetch_add(1, Ordering::SeqCst);
        if first && mode.panic {
            panic!("the held run failed");
        }
    };
    (
        Gate {
            began: is_begun,
            release,
            answers,
            runs,
            ended,
        },
        work,
    )
}

impl Gate {
    async fn began(&mut self) {
        tokio::time::timeout(PATIENCE, self.began.recv())
            .await
            .expect("a run began within 60 s")
            .expect("the work is still there");
    }
}

/// Lets the current-thread runtime run every task that can run.
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

const HOUR: Duration = Duration::from_secs(3600);

mod background {
    use super::*;

    #[tokio::test]
    async fn a_stop_returns_only_after_the_run_going_has_ended() {
        let (mut gate, work) = gated(HOLD);
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        assert!(!stop.is_finished(), "the stop waits for the held run");
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        assert_eq!(
            gate.ended.load(Ordering::SeqCst),
            1,
            "the run had ended when the stop returned"
        );
    }

    #[tokio::test]
    async fn the_run_going_is_told_the_server_is_stopping() {
        let (mut gate, work) = gated(HOLD);
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        assert_eq!(
            *gate.answers.lock().expect("not poisoned"),
            vec![true],
            "the run, asked after the stop, is told to end"
        );
    }

    #[tokio::test]
    async fn a_run_before_any_stop_is_told_the_server_is_not_stopping() {
        let (mut gate, work) = gated(Mode {
            hold: false,
            panic: false,
        });
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        // The run's answer is recorded before it counts its end: wait for
        // the end, not the beginning, so the stop comes after the answer.
        tokio::time::timeout(PATIENCE, async {
            while gate.ended.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the run ends");
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        stop.await.expect("the stop ends");
        assert_eq!(
            *gate.answers.lock().expect("not poisoned"),
            vec![false],
            "the run began and ended before the stop"
        );
    }

    #[tokio::test]
    async fn a_stop_logs_that_it_waits_and_names_the_job() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (mut gate, work) = gated(HOLD);
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        let waits = lines.with(STOP_WAITS);
        assert_eq!(waits.len(), 1, "one waiting line");
        assert_eq!(
            waits[0].field("job"),
            Some("\"test job\""),
            "it names the job"
        );
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
    }

    #[tokio::test]
    async fn a_stop_asks_every_job_before_it_waits_for_any() {
        let (mut first, work) = gated(HOLD);
        let one = Background::start("first job", HOUR, work);
        let (mut second, work) = gated(HOLD);
        let two = Background::start("second job", HOUR, work);
        first.began().await;
        second.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![one, two]));
        settle().await;
        // The second job's run ends while the stop still waits for the
        // first's: it was asked to stop all the same.
        second.release.send(()).expect("the run waits");
        tokio::time::timeout(PATIENCE, async {
            while second.ended.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the second run ends");
        assert_eq!(
            *second.answers.lock().expect("not poisoned"),
            vec![true],
            "the second job was asked before the stop waited for the first"
        );
        first.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        assert_eq!(
            *first.answers.lock().expect("not poisoned"),
            vec![true],
            "the first job was asked as well"
        );
    }

    #[tokio::test]
    async fn a_stop_waits_for_every_job_s_run() {
        let (mut first, work) = gated(HOLD);
        let one = Background::start("first job", HOUR, work);
        let (mut second, work) = gated(HOLD);
        let two = Background::start("second job", HOUR, work);
        first.began().await;
        second.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![one, two]));
        settle().await;
        first.release.send(()).expect("the run waits");
        settle().await;
        assert!(
            !stop.is_finished(),
            "the stop still waits for the second run"
        );
        second.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        assert_eq!(first.ended.load(Ordering::SeqCst), 1, "the first run ended");
        assert_eq!(
            second.ended.load(Ordering::SeqCst),
            1,
            "the second run ended"
        );
    }

    #[tokio::test]
    async fn a_run_going_on_worker_threads_is_waited_for() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime");
        let (mut gate, work) = gated(HOLD);
        let ended = gate.ended.clone();
        let stop = rt.spawn(async move {
            let job = Background::start("test job", HOUR, work);
            gate.began().await;
            let release = gate.release.clone();
            let stopping = tokio::spawn(Background::stop_all(vec![job]));
            settle().await;
            let waiting = !stopping.is_finished();
            release.send(()).expect("the run waits");
            stopping.await.expect("the stop ends");
            waiting
        });
        let waiting = tokio::time::timeout(PATIENCE, stop)
            .await
            .expect("within 60 s")
            .expect("the task ends");
        assert!(waiting, "the stop waited for the held run");
        assert_eq!(ended.load(Ordering::SeqCst), 1, "the run had ended");
        rt.shutdown_background();
    }

    // Failures.

    #[tokio::test]
    async fn a_run_that_panics_during_the_stop_is_logged_as_an_error() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (mut gate, work) = gated(Mode {
            hold: true,
            panic: true,
        });
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        let failed = lines.with(FAILED);
        assert_eq!(failed.len(), 1, "the failed run is logged once");
        assert_eq!(
            failed[0].field("job"),
            Some("\"test job\""),
            "with the job's name"
        );
    }

    #[tokio::test]
    async fn the_logged_failure_carries_the_panic_s_message() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (mut gate, work) = gated(Mode {
            hold: true,
            panic: true,
        });
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        let failed = lines.with(FAILED);
        let error = failed
            .first()
            .and_then(|l| l.field("error"))
            .unwrap_or_default()
            .to_owned();
        assert!(
            error.contains("the held run failed"),
            "the panic's message is in the line: {error:?}"
        );
    }

    #[tokio::test]
    async fn a_run_that_panics_before_any_stop_is_logged_and_the_job_goes_on() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (mut gate, work) = gated(Mode {
            hold: false,
            panic: true,
        });
        let job = Background::start("test job", Duration::ZERO, work);
        gate.began().await;
        gate.began().await;
        Background::stop_all(vec![job]).await;
        assert!(
            gate.runs.load(Ordering::SeqCst) >= 2,
            "a run began after the one that failed"
        );
        assert_eq!(lines.with(FAILED).len(), 1, "the failed run is logged");
    }

    #[tokio::test]
    async fn a_failed_run_in_one_job_does_not_end_the_wait_for_another() {
        let (mut failing, work) = gated(Mode {
            hold: true,
            panic: true,
        });
        let one = Background::start("failing job", HOUR, work);
        let (mut held, work) = gated(HOLD);
        let two = Background::start("held job", HOUR, work);
        failing.began().await;
        held.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![one, two]));
        settle().await;
        failing.release.send(()).expect("the run waits");
        settle().await;
        assert!(!stop.is_finished(), "the stop still waits for the held job");
        held.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        assert_eq!(held.ended.load(Ordering::SeqCst), 1, "the held run ended");
    }

    #[tokio::test]
    async fn a_stop_after_a_failed_run_still_logs_that_it_waits() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (mut gate, work) = gated(Mode {
            hold: true,
            panic: true,
        });
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        let messages: Vec<String> = lines
            .seen
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|l| l.message.clone())
            .collect();
        let wait = messages.iter().position(|m| m == STOP_WAITS);
        let fail = messages.iter().position(|m| m == FAILED);
        assert!(
            matches!((wait, fail), (Some(w), Some(f)) if w < f),
            "the waiting line, then the failure: {messages:?}"
        );
    }

    // Edges.

    #[tokio::test]
    async fn a_job_stopped_before_its_first_run_never_runs() {
        let (gate, work) = gated(HOLD);
        let job = Background::start("test job", HOUR, work);
        Background::stop_all(vec![job]).await;
        assert_eq!(gate.runs.load(Ordering::SeqCst), 0, "no run began");
    }

    #[tokio::test]
    async fn no_run_begins_after_the_stop_even_with_no_interval() {
        let (mut gate, work) = gated(HOLD);
        let job = Background::start("test job", Duration::ZERO, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        settle().await;
        assert_eq!(gate.runs.load(Ordering::SeqCst), 1, "only the held run");
    }

    #[tokio::test]
    async fn a_stop_with_no_run_going_logs_no_waiting_line() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (mut gate, work) = gated(Mode {
            hold: false,
            panic: false,
        });
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        tokio::time::timeout(PATIENCE, async {
            while job.running.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the run ends");
        Background::stop_all(vec![job]).await;
        assert!(lines.with(STOP_WAITS).is_empty(), "nothing to wait for");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stop_still_waiting_after_stop_warn_logs_a_warning() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (mut gate, work) = gated(HOLD);
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        assert!(lines.with(STILL_WAITS).is_empty(), "not before STOP_WARN");
        tokio::time::advance(STOP_WARN).await;
        settle().await;
        let still = lines.with(STILL_WAITS);
        assert_eq!(still.len(), 1, "one warning once STOP_WARN has passed");
        assert_eq!(
            still[0].field("job"),
            Some("\"test job\""),
            "naming the job"
        );
        assert_eq!(still[0].field("waited_secs"), Some("30"), "and the wait");
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stop_warns_again_each_stop_warn_and_goes_on_waiting() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (mut gate, work) = gated(HOLD);
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        for _ in 0..3 {
            tokio::time::advance(STOP_WARN).await;
            settle().await;
        }
        let waited: Vec<Option<String>> = lines
            .with(STILL_WAITS)
            .iter()
            .map(|l| l.field("waited_secs").map(str::to_owned))
            .collect();
        assert_eq!(
            waited,
            [
                Some("30".to_owned()),
                Some("60".to_owned()),
                Some("90".to_owned())
            ],
            "a warning each STOP_WARN, with the time waited"
        );
        assert!(!stop.is_finished(), "the stop goes on waiting");
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stop_never_returns_before_the_run_however_long_it_takes() {
        let (mut gate, work) = gated(HOLD);
        let job = Background::start("test job", HOUR, work);
        gate.began().await;
        let stop = tokio::spawn(Background::stop_all(vec![job]));
        settle().await;
        tokio::time::advance(STOP_WARN * 100).await;
        settle().await;
        assert!(!stop.is_finished(), "no bound ends the wait early");
        gate.release.send(()).expect("the run waits");
        stop.await.expect("the stop ends");
        assert_eq!(gate.ended.load(Ordering::SeqCst), 1, "the run ended first");
    }
}

/// The stop test `sweep` and `pack_all` read before each tenant's step.
mod steps {
    use super::*;

    /// A stop test that answers false `n` times, then true, and counts
    /// the times it was asked.
    fn after(n: usize) -> (Arc<AtomicUsize>, impl Fn() -> bool) {
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = asked.clone();
        (asked, move || counted.fetch_add(1, Ordering::SeqCst) >= n)
    }

    /// A fixture with `n` tenants, each with one version due for purge at
    /// the clock's time once it is moved on.
    fn due_tenants(n: usize) -> (crate::tenants::Fixture, Vec<TenantId>) {
        let f = fixture();
        let mut ids = Vec::new();
        for i in 0..n {
            let id = f
                .tenants
                .create(Actor::Cli, &format!("tenant{i}"), Some(Retention::Days(1)))
                .expect("made");
            let store = f.tenants.store(id).expect("its store");
            let (found, body) = vcon(&format!("0199e5c6-0d7a-7c11-8000-00000000001{i}"), "kept");
            store.create(&CALLER, &found, &body).expect("stored");
            store
                .delete(&CALLER, found.uuid, Retention::Years(7))
                .expect("deleted");
            ids.push(id);
        }
        f.clock.set(datetime!(2026-10-25 13:00 UTC));
        (f, ids)
    }

    fn versions(f: &crate::tenants::Fixture, ids: &[TenantId]) -> u64 {
        ids.iter()
            .map(|id| {
                f.tenants
                    .store(*id)
                    .expect("its store")
                    .counts()
                    .expect("counts")
                    .versions
            })
            .sum()
    }

    /// The pack run's steps the hook counts.
    fn counted_steps(tenants: &Tenants) -> Arc<AtomicUsize> {
        let steps = Arc::new(AtomicUsize::new(0));
        let counted = steps.clone();
        tenants.when_a_job_steps_for_test(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        steps
    }

    #[test]
    fn a_sweep_not_stopping_sweeps_every_tenant_and_asks_before_each() {
        let (f, ids) = due_tenants(3);
        let (asked, stopping) = after(usize::MAX);
        let report = sweep(&f.tenants, Retention::Days(1), &stopping);
        assert_eq!(
            report,
            SweepReport {
                purged: 3,
                failed: 0
            },
            "every tenant swept"
        );
        assert_eq!(asked.load(Ordering::SeqCst), 3, "asked before each step");
        assert_eq!(versions(&f, &ids), 0, "nothing left");
    }

    #[test]
    fn a_pack_run_not_stopping_steps_through_every_tenant_and_asks_before_each() {
        let (f, _) = due_tenants(3);
        let steps = counted_steps(&f.tenants);
        let (asked, stopping) = after(usize::MAX);
        let report = pack_all(&f.tenants, &SETTINGS, &stopping);
        assert_eq!(
            report,
            PackRunReport::default(),
            "local storage packs nothing"
        );
        assert_eq!(steps.load(Ordering::SeqCst), 3, "a step for each tenant");
        assert_eq!(asked.load(Ordering::SeqCst), 3, "asked before each step");
    }

    // Failures in the step under way at the stop.

    #[test]
    fn a_sweep_step_that_fails_as_the_stop_comes_is_counted() {
        let (f, ids) = due_tenants(3);
        for id in &ids {
            f.tenants.close(*id);
        }
        let (_, stopping) = after(1);
        let report = sweep(&f.tenants, Retention::Days(1), &stopping);
        assert_eq!(
            report,
            SweepReport {
                purged: 0,
                failed: 1
            },
            "the one step under way failed and is counted"
        );
    }

    #[test]
    fn a_stopped_sweep_still_logs_its_report_with_the_failed_step() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (f, ids) = due_tenants(2);
        for id in &ids {
            f.tenants
                .conn_for_test()
                .execute("UPDATE tenant SET retention = '7w' WHERE id = ?1", [id.0])
                .expect("damaged");
        }
        let (_, stopping) = after(1);
        sweep(&f.tenants, Retention::Days(1), &stopping);
        let logged = lines.with("purge sweep");
        assert_eq!(logged.len(), 1, "the report is logged");
        assert_eq!(logged[0].field("failed"), Some("1"), "with the failed step");
        assert_eq!(logged[0].field("purged"), Some("0"), "and nothing purged");
    }

    #[test]
    fn a_pack_step_that_fails_as_the_stop_comes_is_counted() {
        let (f, ids) = due_tenants(2);
        for id in &ids {
            f.tenants.close(*id);
        }
        let (_, stopping) = after(1);
        let report = pack_all(&f.tenants, &SETTINGS, &stopping);
        assert_eq!(
            report,
            PackRunReport {
                files: 0,
                versions: 0,
                failed: 1
            },
            "the one step under way failed and is counted"
        );
    }

    #[test]
    fn a_stopped_pack_run_still_logs_its_report_with_the_failed_step() {
        let (lines, _woken) = Lines::new();
        let _guard = tracing::subscriber::set_default(lines.clone());
        let (f, ids) = due_tenants(3);
        for id in &ids {
            f.tenants.close(*id);
        }
        let (_, stopping) = after(1);
        pack_all(&f.tenants, &SETTINGS, &stopping);
        let logged = lines.with("pack run");
        assert_eq!(logged.len(), 1, "the report is logged");
        assert_eq!(logged[0].field("failed"), Some("1"), "with the failed step");
    }

    // Edges.

    #[test]
    fn a_sweep_asked_to_stop_before_its_first_step_begins_none() {
        let (f, ids) = due_tenants(2);
        let (asked, stopping) = after(0);
        let report = sweep(&f.tenants, Retention::Days(1), &stopping);
        assert_eq!(report, SweepReport::default(), "no step");
        assert_eq!(asked.load(Ordering::SeqCst), 1, "asked once, then ended");
        assert_eq!(versions(&f, &ids), 2, "nothing purged");
    }

    #[test]
    fn a_sweep_asked_to_stop_after_one_step_purges_one_tenant() {
        let (f, ids) = due_tenants(3);
        let (_, stopping) = after(1);
        let report = sweep(&f.tenants, Retention::Days(1), &stopping);
        assert_eq!(
            report,
            SweepReport {
                purged: 1,
                failed: 0
            },
            "the step under way, no other"
        );
        assert_eq!(versions(&f, &ids), 2, "two tenants wait for the next sweep");
    }

    #[test]
    fn a_pack_run_asked_to_stop_before_its_first_step_begins_none() {
        let (f, _) = due_tenants(2);
        let steps = counted_steps(&f.tenants);
        let (asked, stopping) = after(0);
        pack_all(&f.tenants, &SETTINGS, &stopping);
        assert_eq!(steps.load(Ordering::SeqCst), 0, "no step");
        assert_eq!(asked.load(Ordering::SeqCst), 1, "asked once, then ended");
    }

    #[test]
    fn a_pack_run_asked_to_stop_after_one_step_begins_no_other() {
        let (f, _) = due_tenants(3);
        let steps = counted_steps(&f.tenants);
        let (_, stopping) = after(1);
        pack_all(&f.tenants, &SETTINGS, &stopping);
        assert_eq!(
            steps.load(Ordering::SeqCst),
            1,
            "the step under way, no other"
        );
    }

    // Inside a tenant's step: the stop is asked between its versions.

    /// A fixture with one tenant whose `n` versions are due for purge at
    /// the clock's time once it is moved on.
    fn one_tenant_due(n: u8) -> (crate::tenants::Fixture, TenantId) {
        let f = fixture();
        let id = f
            .tenants
            .create(Actor::Cli, "acme", Some(Retention::Days(1)))
            .expect("made");
        let store = f.tenants.store(id).expect("its store");
        for i in 0..n {
            let (found, body) = vcon(&format!("0199e5c6-0d7a-7c11-8000-0000000002{i:02}"), "kept");
            store.create(&CALLER, &found, &body).expect("stored");
            store
                .delete(&CALLER, found.uuid, Retention::Years(7))
                .expect("deleted");
        }
        f.clock.set(datetime!(2026-10-25 13:00 UTC));
        (f, id)
    }

    #[test]
    fn a_sweep_stopped_inside_a_tenant_s_step_ends_after_the_version_under_way() {
        let (f, id) = one_tenant_due(4);
        // The tenant's step begins; the stop comes before its second
        // version.
        let (asked, stopping) = after(1);
        let report = sweep(&f.tenants, Retention::Days(1), &stopping);
        assert_eq!(
            report,
            SweepReport {
                purged: 1,
                failed: 0
            },
            "the version under way, no other"
        );
        assert_eq!(asked.load(Ordering::SeqCst), 2, "the step, then the stop");
        assert_eq!(versions(&f, &[id]), 3, "three wait for the next sweep");
    }

    #[test]
    fn a_sweep_not_stopping_asks_between_a_tenant_s_versions() {
        let (f, id) = one_tenant_due(3);
        let (asked, stopping) = after(usize::MAX);
        let report = sweep(&f.tenants, Retention::Days(1), &stopping);
        assert_eq!(report.purged, 3, "every version");
        assert_eq!(
            asked.load(Ordering::SeqCst),
            3,
            "before the step and before its second and third versions"
        );
        assert_eq!(versions(&f, &[id]), 0, "nothing left");
    }

    #[test]
    fn a_sweep_renews_its_lease_per_tenant_and_asks_the_stop_per_version() {
        let (f, _) = one_tenant_due(3);
        let mut leased = 0;
        let (stops, stopping) = after(usize::MAX);
        let report = super::super::sweep_while(
            &f.tenants,
            Retention::Days(1),
            &mut || {
                leased += 1;
                true
            },
            &stopping,
        );
        assert_eq!(report.purged, 3, "every version");
        assert_eq!(leased, 1, "the lease is asked for once, before the tenant");
        assert_eq!(
            stops.load(Ordering::SeqCst),
            2,
            "the stop between the versions"
        );
    }

    #[test]
    fn a_led_sweep_is_stopped_inside_a_tenant_s_step() {
        let (f, id) = one_tenant_due(3);
        // Asked before the lease, before the tenant, then before its
        // second version, where it answers true.
        let (asked, stopping) = after(2);
        let report = super::super::lead_sweep(
            &f.tenants,
            Retention::Days(1),
            super::super::lease_for(Duration::from_secs(60)),
            &stopping,
        );
        assert_eq!(
            report,
            Some(SweepReport {
                purged: 1,
                failed: 0
            }),
            "the version under way, no other"
        );
        assert_eq!(asked.load(Ordering::SeqCst), 3, "then no more");
        assert_eq!(versions(&f, &[id]), 2, "two wait");
    }

    #[test]
    fn a_sweep_s_second_tenant_is_not_begun_after_a_stop_inside_the_first() {
        let (f, ids) = due_tenants(2);
        let store = f.tenants.store(ids[0]).expect("its store");
        let (found, body) = vcon("0199e5c6-0d7a-7c11-8000-000000000299", "kept");
        store.create(&CALLER, &found, &body).expect("stored");
        store
            .delete(&CALLER, found.uuid, Retention::Years(7))
            .expect("deleted");
        f.clock.set(datetime!(2026-10-27 13:00 UTC));
        // The first tenant's step begins, the stop comes before its second
        // version, and the second tenant is not begun.
        let (_, stopping) = after(1);
        let report = sweep(&f.tenants, Retention::Days(1), &stopping);
        assert_eq!(report.purged, 1, "one version: {report:?}");
        assert_eq!(versions(&f, &ids), 2, "the rest wait");
    }

    #[test]
    fn a_sweep_of_no_tenant_never_asks() {
        let f = fixture();
        let (asked, stopping) = after(0);
        let report = sweep(&f.tenants, Retention::Days(1), &stopping);
        assert_eq!(report, SweepReport::default(), "nothing to sweep");
        assert_eq!(asked.load(Ordering::SeqCst), 0, "no step to ask before");
    }

    #[test]
    fn a_pack_run_of_no_tenant_never_asks() {
        let f = fixture();
        let (asked, stopping) = after(0);
        let report = pack_all(&f.tenants, &SETTINGS, &stopping);
        assert_eq!(report, PackRunReport::default(), "nothing to pack");
        assert_eq!(asked.load(Ordering::SeqCst), 0, "no step to ask before");
    }
}
