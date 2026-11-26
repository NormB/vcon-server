//! A start, and `check`, unwraps the tenant keys with at most
//! `key_wrap.start_parallel` unwraps in flight: a backend that holds each
//! unwrap until as many as a test asks for are in flight shows how many
//! ran at once.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use time::macros::datetime;

use super::Tenants;
use crate::audit::Actor;
use crate::backend::{Catalogs, ObjectPlace};
use crate::clock::ManualClock;
use crate::envelope::Key;
use crate::fs::OsFs;
use crate::ids::{KeyId, TenantId};
use crate::key_wrap::{FileWrap, KeyWrap, WrapError, Wrapped, WrappedKek};

/// How long an unwrap waits for the others a test expects at once. A
/// build that unwraps one at a time waits this long at each.
const HOLD: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
struct Flight {
    now: usize,
    most: usize,
}

/// The file backend, holding each unwrap until `want` are in flight (or
/// `HOLD` passes), and counting the most in flight at once.
#[derive(Debug)]
struct Gated {
    inner: FileWrap,
    want: Mutex<usize>,
    flight: Mutex<Flight>,
    turn: Condvar,
    fail: bool,
}

#[derive(Debug)]
struct Shared(Arc<Gated>);

impl KeyWrap for Shared {
    fn key_id(&self) -> KeyId {
        self.0.inner.key_id()
    }

    fn root(&self) -> &Key {
        self.0.inner.root()
    }

    fn wrap(&self, tenant: TenantId, key: &Key) -> Result<Wrapped, WrapError> {
        self.0.inner.wrap(tenant, key)
    }

    fn unwrap(
        &self,
        tenant: TenantId,
        wrapped: &[u8],
        kek: Option<&WrappedKek>,
    ) -> Result<Key, WrapError> {
        let want = *self.0.want.lock().expect("want");
        {
            let mut flight = self.0.flight.lock().expect("flight");
            flight.now += 1;
            flight.most = flight.most.max(flight.now);
            self.0.turn.notify_all();
            let (mut flight, _) = self
                .0
                .turn
                .wait_timeout_while(flight, HOLD, |f| f.most < want)
                .expect("flight");
            flight.now -= 1;
        }
        if self.0.fail {
            return Err(WrapError::Damaged);
        }
        self.0.inner.unwrap(tenant, wrapped, kek)
    }
}

struct World {
    _data: tempfile::TempDir,
    gated: Arc<Gated>,
    tenants: Tenants,
}

/// A registry over the gated backend with `n` tenants, whose key order is
/// read with `parallel` unwraps at once.
fn world(n: usize, parallel: usize, fail: bool) -> World {
    let data = tempfile::tempdir().expect("temp dir");
    let gated = Arc::new(Gated {
        inner: FileWrap::new(Key::generate().expect("a key")),
        want: Mutex::new(0),
        flight: Mutex::new(Flight::default()),
        turn: Condvar::new(),
        fail,
    });
    let tenants = Tenants::opened(
        data.path(),
        Box::new(Shared(gated.clone())),
        Arc::new(OsFs),
        Arc::new(ManualClock::at(datetime!(2026-11-26 12:00 UTC))),
        false,
        ObjectPlace::Local,
        &Catalogs::Local,
    )
    .expect("opens");
    for i in 0..n {
        tenants
            .create(Actor::Cli, &format!("tenant-{i}"), None)
            .expect("made");
    }
    tenants.set_start_parallel(parallel);
    World {
        _data: data,
        gated,
        tenants,
    }
}

impl World {
    /// Reads the key order with unwraps held until `want` are in flight,
    /// and gives the order and the most that were in flight at once.
    fn order(&self, want: usize) -> (super::KeyOrder, usize) {
        *self.gated.want.lock().expect("want") = want;
        self.gated.flight.lock().expect("flight").most = 0;
        let order = self.tenants.key_order().expect("reads");
        (order, self.gated.flight.lock().expect("flight").most)
    }
}

// Success.

#[test]
fn two_at_once_unwrap_two_at_once() {
    let w = world(4, 2, false);
    let (order, most) = w.order(2);
    assert_eq!(most, 2, "two in flight at once");
    assert!(order.is_sound(), "every key opens: {order:?}");
}

#[test]
fn one_at_once_is_one_after_another() {
    let w = world(3, 1, false);
    let (order, most) = w.order(1);
    assert_eq!(most, 1, "never two");
    assert!(order.is_sound(), "{order:?}");
}

#[test]
fn never_more_than_start_parallel_are_in_flight() {
    let w = world(6, 3, false);
    // Six tenants, three at once: the most in flight is three, not six.
    let (order, most) = w.order(3);
    assert_eq!(most, 3, "at most three");
    assert!(order.is_sound(), "{order:?}");
}

#[test]
fn the_setting_reads_back() {
    let w = world(0, 5, false);
    assert_eq!(w.tenants.start_parallel(), 5, "as set");
}

// Failure.

#[test]
fn a_key_that_does_not_open_is_counted_with_unwraps_in_parallel() {
    let w = world(4, 2, true);
    let (order, most) = w.order(2);
    assert_eq!(order.wrapped_elsewhere, 4, "every key: {order:?}");
    assert_eq!(most, 2, "still two at once");
}

#[test]
fn a_failing_backend_one_at_a_time_counts_each_key() {
    let w = world(3, 1, true);
    let (order, _) = w.order(1);
    assert_eq!(order.wrapped_elsewhere, 3, "{order:?}");
    assert!(!order.is_sound(), "out of order");
}

// Edge.

#[test]
fn more_at_once_than_tenants_unwraps_them_all_at_once() {
    let w = world(3, 8, false);
    let (order, most) = w.order(3);
    assert_eq!(most, 3, "all three at once");
    assert!(order.is_sound(), "{order:?}");
}

#[test]
fn no_tenants_unwrap_nothing() {
    let w = world(0, 4, false);
    let (order, most) = w.order(1);
    assert_eq!(most, 0, "no unwrap");
    assert!(order.is_sound(), "{order:?}");
}

#[test]
fn zero_at_once_is_taken_as_one() {
    let w = world(2, 0, false);
    assert_eq!(w.tenants.start_parallel(), 1, "at least one");
    let (order, most) = w.order(1);
    assert_eq!(most, 1, "one at a time");
    assert!(order.is_sound(), "{order:?}");
}

#[test]
fn one_tenant_with_many_at_once_is_one_unwrap() {
    let w = world(1, 64, false);
    let (order, most) = w.order(1);
    assert_eq!(most, 1, "one");
    assert!(order.is_sound(), "{order:?}");
}
