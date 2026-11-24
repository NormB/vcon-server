//! A test's helper thread that is always joined, in one place: the test
//! crates that read a child process's output on a thread bring it in by
//! `#[path]` (tests/file_modes.rs, formats.rs, mcp_stdio.rs, rotation.rs,
//! serve.rs, tui_terminal.rs); its own tests are in
//! tests/joined_threads.rs. A stub server's threads are joined by
//! tests/support/stub_server.rs.
//!
//! A thread that is never joined can still be running, or dropping what it
//! holds, when the test process exits: on 2026-11-24 such a thread,
//! dropping its clone of a Store (SQLite, SQLCipher, OpenSSL's per-thread
//! state), crashed CI's coverage run with SIGSEGV after its test had
//! passed. A [`Joined`] is joined when it is dropped, so its owner (the
//! struct that holds the child process) joins it after the child has
//! exited and the pipe the thread reads has ended.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// How long a drop waits for the thread to end: far past the end of a
/// pipe whose process has exited, on a loaded host.
pub const JOIN_WAIT: Duration = Duration::from_secs(30);

/// A thread that is joined when this is dropped, or by [`Joined::join`].
pub struct Joined<T> {
    thread: Option<JoinHandle<T>>,
    /// In a mutex so a `Joined` is `Sync`, as the structs that hold one
    /// are shared with a server's tasks; only `&mut self` reads it.
    ended: Mutex<Receiver<()>>,
}

/// Says that its thread has ended when dropped: also when the thread
/// panics.
struct Ends(#[allow(dead_code, reason = "held only to be dropped")] Sender<()>);

impl<T: Send + 'static> Joined<T> {
    /// Runs `f` on a thread of its own.
    pub fn spawn(f: impl FnOnce() -> T + Send + 'static) -> Joined<T> {
        let (ends, ended) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _ends = Ends(ends);
            f()
        });
        Joined {
            thread: Some(thread),
            ended: Mutex::new(ended),
        }
    }
}

impl<T> Joined<T> {
    /// Waits for the thread to end, at most `within`, and joins it: what
    /// it returned. Panics when it has not ended by then (the drop then
    /// waits for it again), when it panicked, or when it was joined
    /// already.
    #[allow(
        dead_code,
        reason = "not every test crate that holds this module joins by hand"
    )]
    pub fn join(&mut self, within: Duration) -> T {
        match self.end(within) {
            Ok(Some(value)) => value,
            Ok(None) => panic!("a test's thread joined twice"),
            Err(why) => panic!("{why}"),
        }
    }

    /// Waits for the thread to end, at most `within`, and joins it.
    fn end(&mut self, within: Duration) -> Result<Option<T>, String> {
        let Some(thread) = self.thread.take() else {
            return Ok(None);
        };
        let ended = self
            .ended
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .recv_timeout(within);
        match ended {
            // Disconnected: the thread's `Ends` is dropped, so it has ended.
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {}
            Err(RecvTimeoutError::Timeout) => {
                // Kept, so the drop that follows the panic waits for it
                // again.
                self.thread = Some(thread);
                return Err(format!("a test's thread did not end within {within:?}"));
            }
        }
        thread
            .join()
            .map(Some)
            .map_err(|_| "a test's thread panicked".to_owned())
    }
}

impl<T> Drop for Joined<T> {
    /// Joins the thread. A thread that did not end, or panicked, fails the
    /// test here, unless the test is already failing.
    fn drop(&mut self) {
        if let Err(why) = self.end(JOIN_WAIT)
            && !std::thread::panicking()
        {
            panic!("{why}");
        }
    }
}
