//! A measurement, run by hand: how long a stop waits inside one tenant's
//! purge or pack step, on a real filesystem. It is ignored by default,
//! because it stores many vCons and reports times rather than checking
//! them. Run it in release mode:
//!
//! ```sh
//! VCON_STOP_MEASURE_N=20000 VCON_STOP_MEASURE_DIR=/some/disk \
//!   cargo test --release -p vcon-server --lib stop_latency_is_measured \
//!   -- --ignored --nocapture
//! ```
//!
//! For each step it prints the time of the whole step, the longest and the
//! mean time between two asks of the stop test (one item's work, the
//! longest a stop waits for the item under way), and the time from the
//! ask that answered true to the step's return (what the step does after
//! the stop). The pack step writes its files to a directory (`DirPacks`),
//! not to S3: an S3 request takes the network's time on top.

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use time::macros::datetime;

use super::packed::tests::DirPacks;
use super::packer::PackPlan;
use super::testing::vcon;
use super::{Caller, Created, Store};
use crate::clock::ManualClock;
use crate::envelope::Key;
use crate::ids::TenantId;
use crate::retention::Retention;

/// What one step's asks of the stop test saw.
struct Asks {
    begun: Instant,
    last: Cell<Instant>,
    longest: Cell<Duration>,
    count: Cell<u64>,
    stopped_at: Cell<Option<Instant>>,
    /// Asks answered false before the stop.
    go: u64,
}

impl Asks {
    fn new(go: u64) -> Asks {
        let now = Instant::now();
        Asks {
            begun: now,
            last: Cell::new(now),
            longest: Cell::new(Duration::ZERO),
            count: Cell::new(0),
            stopped_at: Cell::new(None),
            go,
        }
    }

    fn ask(&self) -> bool {
        let now = Instant::now();
        let gap = now - self.last.get();
        if gap > self.longest.get() {
            self.longest.set(gap);
        }
        self.last.set(now);
        let n = self.count.get() + 1;
        self.count.set(n);
        if n > self.go {
            if self.stopped_at.get().is_none() {
                self.stopped_at.set(Some(now));
            }
            return true;
        }
        false
    }

    fn print(&self, what: &str, ended: Instant) {
        let asked = self.count.get();
        let until = self.stopped_at.get().unwrap_or(ended);
        let mean = if asked > 1 {
            (until - self.begun) / u32::try_from(asked).unwrap_or(u32::MAX)
        } else {
            Duration::ZERO
        };
        let after = self.stopped_at.get().map(|at| ended - at);
        println!(
            "{what}: asks {asked}, longest item {:?}, mean item {mean:?}, after the stop {after:?}, step {:?}",
            self.longest.get(),
            ended - self.begun,
        );
    }
}

fn count() -> usize {
    std::env::var("VCON_STOP_MEASURE_N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(10_000)
}

fn parent() -> std::path::PathBuf {
    std::env::var_os("VCON_STOP_MEASURE_DIR")
        .map(Into::into)
        .unwrap_or_else(std::env::temp_dir)
}

/// A Store on the real filesystem with `n` vCons, each deleted, and the
/// clock moved on so each is due under a day's retention.
fn store_of(dir: &std::path::Path, n: usize, clock: &Arc<ManualClock>) -> Store {
    let key = Key::generate().expect("tenant key");
    let store = Store::open(
        dir,
        TenantId([0x5a; 16]),
        key,
        Arc::new(crate::fs::OsFs),
        clock.clone(),
    )
    .expect("opens");
    let caller = Caller {
        key: Key::generate().expect("caller key").id(),
    };
    for i in 0..n {
        let uuid = format!("01a07da8-c2bb-83e5-b9a2-{i:012x}");
        let (found, body) = vcon(&uuid, "measured");
        let Ok(Created::New(_)) = store.create(&caller, &found, &body) else {
            panic!("a new vCon")
        };
        store
            .delete(&caller, found.uuid, Retention::Years(7))
            .expect("deletes");
    }
    store
}

#[test]
#[ignore = "a measurement run by hand in release mode; see the module's comment"]
fn stop_latency_is_measured() {
    let n = count();
    let go = u64::try_from(n / 2).expect("a count");
    println!("vCons: {n}; the stop answers true after {go} asks");

    // The purge step: one tenant's whole step, then one stopped half way.
    for stopped in [false, true] {
        let dir = tempfile::tempdir_in(parent()).expect("temp dir");
        let clock = Arc::new(ManualClock::at(datetime!(2026-10-23 12:00 UTC)));
        let store = store_of(dir.path(), n, &clock);
        clock.set(datetime!(2026-10-26 12:00 UTC));
        let asks = Asks::new(if stopped { go } else { u64::MAX });
        let report = store
            .purge_due_until(Retention::Days(1), &|| asks.ask())
            .expect("purges");
        asks.print(
            &format!(
                "purge, {}, purged {}",
                if stopped { "stopped" } else { "whole" },
                report.purged
            ),
            Instant::now(),
        );
    }

    // The pack step, into a directory.
    for stopped in [false, true] {
        let dir = tempfile::tempdir_in(parent()).expect("temp dir");
        let clock = Arc::new(ManualClock::at(datetime!(2026-10-23 12:00 UTC)));
        let mut store = store_of(dir.path(), 0, &clock);
        let caller = Caller {
            key: Key::generate().expect("caller key").id(),
        };
        for i in 0..n {
            let uuid = format!("01a07da8-c2bb-83e5-b9a2-{i:012x}");
            let (found, body) = vcon(&uuid, "measured");
            store.create(&caller, &found, &body).expect("stores");
        }
        store.packs = Some(DirPacks::new());
        let asks = Asks::new(if stopped { go } else { u64::MAX });
        let report = store
            .pack_until(&PackPlan::with_limit(1 << 20), &|| asks.ask())
            .expect("packs");
        asks.print(
            &format!(
                "pack, {}, files {}, versions {}",
                if stopped { "stopped" } else { "whole" },
                report.files,
                report.versions
            ),
            Instant::now(),
        );
    }
}
