//! Recovery judges every pending row by one reading of the catalog's
//! clock. It deletes the rows no write can still be under way for in one
//! transaction and lists what to keep in a later one; a row judged under
//! way in the first must be judged so in the second, or its object is
//! removed while the row stays, and the write's last transaction then adds
//! a row that names no object. A database's clock moves between the two
//! transactions; the test seam `clock_steps` makes it move here.

use std::sync::mpsc;
use std::time::Duration;

use super::pending::{WRITE_LEASE, Writers};
use super::testing::vcon;
use super::tests::fixture;
use crate::clock::Clock;
use crate::store::Created;

/// A minute, in nanoseconds.
const MINUTE: i64 = 60 * 1_000_000_000;

/// A write a minute inside its lease when recovery reads the clock is
/// kept, though the clock passes the lease before recovery lists what to
/// keep.
#[test]
fn recovery_judges_every_pending_row_by_one_reading_of_the_clock() {
    let f = fixture();
    let start = f.clock.now();
    let (found, body) = vcon("01a07da8-c2bb-83e5-b9a2-279e0d16be01", "under way");
    let (arrive, arrived) = mpsc::channel::<()>();
    let (go, went) = mpsc::channel::<()>();
    f.store.on_between(Box::new(move || {
        arrive.send(()).expect("the test waits for the write");
        went.recv_timeout(Duration::from_secs(60))
            .expect("the test lets the write go on");
    }));
    let created = std::thread::scope(|s| {
        let writing = s.spawn(|| f.store.create(&f.caller, &found, &body));
        arrived
            .recv_timeout(Duration::from_secs(60))
            .expect("the write stops between its object and its row");
        f.clock
            .set(start + time::Duration::nanoseconds(WRITE_LEASE - MINUTE));
        f.store.step_clock(vec![0, 2 * MINUTE]);
        f.store.recover(Writers::Maybe).expect("recovery runs");
        go.send(()).expect("the write waits");
        writing.join().expect("the write ends")
    });
    assert!(
        matches!(created, Ok(Created::New(_))),
        // The outcome is not printed: it travels with the caller's key id,
        // and a test's message is a log like any other.
        "the write commits as a new vCon"
    );
    assert_eq!(
        f.store.get(&f.caller, found.uuid).map(|s| s.body),
        Ok(body),
        "its object was kept with its row"
    );
}
