//! The tests of tests/support/joined.rs, the helper thread every test crate
//! that reads a child process's output joins.

#[path = "support/joined.rs"]
mod joined;

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use joined::Joined;

/// Longer than any thread here takes, on a loaded host.
const WAIT: Duration = Duration::from_secs(20);

/// What a caught panic said.
fn said(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

/// A thread that holds `held` until it is let go (or `WAIT` passes), and
/// the sender that lets it go.
fn held_until_let_go(held: &Arc<()>) -> (Joined<()>, mpsc::Sender<()>) {
    let (let_go, wait) = mpsc::channel::<()>();
    let held = Arc::clone(held);
    let thread = Joined::spawn(move || {
        let _held = held;
        let _ = wait.recv_timeout(WAIT);
    });
    (thread, let_go)
}

// Success.

#[test]
fn join_answers_what_the_thread_returned() {
    let mut thread = Joined::spawn(|| 6 * 7);
    assert_eq!(thread.join(WAIT), 42, "the thread's value");
}

#[test]
fn a_drop_joins_the_thread_and_what_it_held_is_dropped() {
    let held = Arc::new(());
    let (thread, let_go) = held_until_let_go(&held);
    let_go.send(()).expect("the thread waits");
    drop(thread);
    assert_eq!(
        Arc::strong_count(&held),
        1,
        "the thread has ended and dropped its clone"
    );
}

#[test]
fn a_drop_waits_for_a_thread_still_running() {
    let held = Arc::new(());
    let (thread, let_go) = held_until_let_go(&held);
    let (dropped, done) = mpsc::channel();
    let mut dropping = Joined::spawn(move || {
        drop(thread);
        let _ = dropped.send(());
    });
    assert!(
        done.recv_timeout(Duration::from_millis(200)).is_err(),
        "the drop has not returned while its thread runs"
    );
    let_go.send(()).expect("the thread waits");
    done.recv_timeout(WAIT)
        .expect("the drop returns once it ends");
    dropping.join(WAIT);
    assert_eq!(Arc::strong_count(&held), 1, "the thread has been joined");
}

#[test]
fn a_reader_of_a_pipe_is_joined_once_its_writer_closes() {
    let (mut reader, mut writer) = std::io::pipe().expect("a pipe");
    let read = Arc::new(Mutex::new(Vec::new()));
    let thread = {
        let read = Arc::clone(&read);
        Joined::spawn(move || {
            let mut all = Vec::new();
            let _ = reader.read_to_end(&mut all);
            *read.lock().expect("the lock") = all;
        })
    };
    writer.write_all(b"serving\n").expect("writes");
    drop(writer);
    drop(thread);
    assert_eq!(
        *read.lock().expect("the lock"),
        b"serving\n",
        "the thread read to the end before it was joined"
    );
    assert_eq!(Arc::strong_count(&read), 1, "and has ended");
}

#[test]
fn join_returns_once_the_thread_ends_not_at_its_limit() {
    let began = Instant::now();
    Joined::spawn(|| ()).join(WAIT);
    assert!(
        began.elapsed() < WAIT / 2,
        "joined when it ended: {:?}",
        began.elapsed()
    );
}

// Failure.

#[test]
fn join_fails_when_the_thread_does_not_end_in_time() {
    let held = Arc::new(());
    let (thread, let_go) = held_until_let_go(&held);
    let failed = catch_unwind(AssertUnwindSafe(move || {
        let mut thread = thread;
        // Dropped first once the join has given up (the last declared is
        // the first dropped), so the thread's drop finds it ending.
        let _let_go = LetGoOnDrop(let_go);
        thread.join(Duration::from_millis(50))
    }))
    .expect_err("the join gives up");
    assert!(
        said(failed.as_ref()).contains("did not end within"),
        "the panic says why: {}",
        said(failed.as_ref())
    );
    assert_eq!(
        Arc::strong_count(&held),
        1,
        "the drop during the panic still joined the thread"
    );
}

#[test]
fn a_second_join_fails() {
    let mut thread = Joined::spawn(|| ());
    thread.join(WAIT);
    let failed = catch_unwind(AssertUnwindSafe(move || thread.join(WAIT))).expect_err("refused");
    assert!(
        said(failed.as_ref()).contains("twice"),
        "the panic says why: {}",
        said(failed.as_ref())
    );
}

#[test]
fn join_fails_when_the_thread_panicked() {
    let mut thread = Joined::<()>::spawn(|| panic!("the helper's own failure"));
    let failed =
        catch_unwind(AssertUnwindSafe(move || thread.join(WAIT))).expect_err("the join fails");
    assert!(
        said(failed.as_ref()).contains("panicked"),
        "the panic says the thread panicked: {}",
        said(failed.as_ref())
    );
}

#[test]
fn a_drop_fails_when_the_thread_panicked() {
    let thread = Joined::<()>::spawn(|| panic!("the helper's own failure"));
    let failed = catch_unwind(AssertUnwindSafe(|| drop(thread))).expect_err("the drop fails");
    assert!(
        said(failed.as_ref()).contains("panicked"),
        "the panic says the thread panicked: {}",
        said(failed.as_ref())
    );
}

// Edge.

#[test]
fn a_joined_thread_may_be_shared_between_threads() {
    // A struct that holds one (a server and its reader) is shared with a
    // server's tasks in tests/serve.rs, behind an Arc.
    fn shared<T: Send + Sync>(_: &T) -> bool {
        true
    }
    let thread = Joined::spawn(|| ());
    assert!(shared(&thread), "Send and Sync");
}

#[test]
fn a_drop_while_the_test_fails_reports_the_test_not_the_thread() {
    let failed = catch_unwind(AssertUnwindSafe(|| {
        let _thread = Joined::<()>::spawn(|| panic!("the helper's own failure"));
        panic!("the test's failure");
    }))
    .expect_err("the test fails");
    assert_eq!(
        said(failed.as_ref()),
        "the test's failure",
        "the test's own panic is the one reported, and the process did not abort"
    );
}

#[test]
fn a_thread_that_returns_a_value_never_read_is_joined_by_the_drop() {
    let held = Arc::new(());
    let thread = {
        let held = Arc::clone(&held);
        Joined::spawn(move || held)
    };
    drop(thread);
    assert_eq!(
        Arc::strong_count(&held),
        1,
        "the value the thread returned was dropped with its handle"
    );
}

#[test]
fn many_threads_are_each_joined() {
    let held = Arc::new(());
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let held = Arc::clone(&held);
            Joined::spawn(move || drop(held))
        })
        .collect();
    drop(threads);
    assert_eq!(Arc::strong_count(&held), 1, "all eight have ended");
}

/// Lets its thread go when dropped: also while a panic unwinds.
struct LetGoOnDrop(mpsc::Sender<()>);

impl Drop for LetGoOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
