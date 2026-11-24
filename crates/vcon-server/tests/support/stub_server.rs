//! A TCP server on the loopback address for a test, whose threads are all
//! joined when it is dropped.
//!
//! A stub server's threads hold what their handler holds (a TLS
//! configuration, aws-lc-rs and OpenSSL state, a counter the test reads).
//! One left running when its test returns can still be ending when the
//! test process exits: on 2026-11-24 a test thread that was never joined,
//! dropping its clone of a Store (SQLite, SQLCipher, OpenSSL's per-thread
//! state), crashed CI's coverage run with SIGSEGV after the test had
//! passed. So a stub keeps every thread it starts, and its drop shuts down
//! each connection it accepted, wakes its accept and joins them all.
//!
//! Each connection runs `handler` on a thread of its own. A handler must
//! end once its stream is shut down (a read answers 0 or an error, a
//! write fails); one that waits on anything else holds the drop.
//!
//! In one place for the library's tests (src/lib.rs brings it in by
//! `#[path]` under `cfg(test)`) and the test crates' (tests/migrate_command.rs);
//! its own tests, below, run in each crate that holds it.

use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// The connections a stub server has accepted, and whether it has stopped
/// accepting: one lock, so no connection is accepted after its drop shut
/// down the ones it knew of.
#[derive(Default)]
struct Open {
    stopped: bool,
    streams: Vec<TcpStream>,
}

/// A stub server; see the module.
pub(crate) struct StubServer {
    address: SocketAddr,
    open: Arc<Mutex<Open>>,
    /// The accepting thread: it joins the connections' threads and answers
    /// what each handler that panicked said.
    acceptor: Option<JoinHandle<Vec<String>>>,
}

/// Shuts down its connection when dropped: the clone kept for the server's
/// drop holds the socket open, so without this a connection would not end
/// when its handler returns or panics, as it does with no clone.
struct EndsOnDrop(TcpStream);

impl Drop for EndsOnDrop {
    fn drop(&mut self) {
        // One the client has closed already fails; it has ended.
        drop(self.0.shutdown(Shutdown::Both));
    }
}

fn lock(open: &Mutex<Open>) -> MutexGuard<'_, Open> {
    open.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a panic said, from its payload.
fn said(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic with no message".to_owned())
}

impl StubServer {
    /// Listens on a free loopback port and runs `handler` on each
    /// connection, on a thread of its own.
    pub(crate) fn start(handler: impl Fn(TcpStream) + Send + Sync + 'static) -> StubServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("its address");
        let open = Arc::new(Mutex::new(Open::default()));
        let handler = Arc::new(handler);
        let acceptor = {
            let open = Arc::clone(&open);
            std::thread::spawn(move || {
                let mut connections = Vec::new();
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    let mut open = lock(&open);
                    // The connection that woke the accept for the drop, or
                    // one that came after it: not handled.
                    if open.stopped {
                        break;
                    }
                    let (Ok(kept), Ok(own)) = (stream.try_clone(), stream.try_clone()) else {
                        continue;
                    };
                    open.streams.push(kept);
                    let handler = Arc::clone(&handler);
                    connections.push(std::thread::spawn(move || {
                        let _ends = EndsOnDrop(own);
                        handler(stream);
                    }));
                }
                connections
                    .into_iter()
                    .filter_map(|connection| connection.join().err())
                    .map(|payload| said(payload.as_ref()))
                    .collect()
            })
        };
        StubServer {
            address,
            open,
            acceptor: Some(acceptor),
        }
    }

    /// Where it listens.
    #[must_use]
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for StubServer {
    /// Shuts down every connection, wakes the accept and joins every
    /// thread. A handler's panic fails the test here, unless the test is
    /// already failing.
    fn drop(&mut self) {
        {
            let mut open = lock(&self.open);
            open.stopped = true;
            for stream in open.streams.drain(..) {
                // One the client has closed already fails; it has ended.
                drop(stream.shutdown(Shutdown::Both));
            }
        }
        // Wakes the accept, which then sees that it has stopped. It fails
        // when the accept has already ended.
        drop(TcpStream::connect(self.address));
        let Some(acceptor) = self.acceptor.take() else {
            return;
        };
        let panics = match acceptor.join() {
            Ok(panics) => panics,
            Err(payload) => vec![said(payload.as_ref())],
        };
        if !panics.is_empty() && !std::thread::panicking() {
            panic!("a stub server's thread panicked: {panics:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    /// Longer than any drop here takes, on a loaded host.
    const WAIT: Duration = Duration::from_secs(20);

    /// Drops `server` on a thread of its own, and fails if that takes past
    /// `WAIT`: a drop that waits for a handler that never ends.
    fn dropped_within_wait(server: StubServer) {
        let (done, finished) = mpsc::channel();
        let dropping = std::thread::spawn(move || {
            drop(server);
            let _ = done.send(());
        });
        finished
            .recv_timeout(WAIT)
            .expect("the drop returns once its threads end");
        dropping.join().expect("the drop does not panic");
    }

    /// A handler that answers each line it reads with the line, until the
    /// stream ends; it holds `held` while it runs.
    fn echo(held: Arc<()>) -> impl Fn(TcpStream) + Send + Sync + 'static {
        move |mut stream| {
            let _held = Arc::clone(&held);
            let mut buf = [0_u8; 256];
            while let Ok(n @ 1..) = stream.read(&mut buf) {
                if stream.write_all(&buf[..n]).is_err() {
                    return;
                }
            }
        }
    }

    fn connect(server: &StubServer) -> TcpStream {
        let stream = TcpStream::connect(server.address()).expect("connects");
        stream.set_read_timeout(Some(WAIT)).expect("a read timeout");
        stream
    }

    fn round_trip(stream: &mut TcpStream, text: &str) -> String {
        stream.write_all(text.as_bytes()).expect("writes");
        let mut buf = vec![0_u8; text.len()];
        stream.read_exact(&mut buf).expect("the answer");
        String::from_utf8(buf).expect("text")
    }

    /// Waits until `count` holds `want` clones (one each for the test and
    /// every handler running).
    fn until_count(count: &Arc<()>, want: usize) {
        let deadline = std::time::Instant::now() + WAIT;
        while Arc::strong_count(count) != want {
            assert!(
                std::time::Instant::now() < deadline,
                "{want} clones within {WAIT:?}, not {}",
                Arc::strong_count(count)
            );
            std::thread::yield_now();
        }
    }

    // Success.

    #[test]
    fn a_connection_is_answered_by_the_handler() {
        let server = StubServer::start(echo(Arc::new(())));
        let mut client = connect(&server);
        assert_eq!(
            round_trip(&mut client, "ping"),
            "ping",
            "the handler's answer"
        );
        drop(client);
        dropped_within_wait(server);
    }

    #[test]
    fn connections_are_answered_at_the_same_time() {
        let server = StubServer::start(echo(Arc::new(())));
        let mut first = connect(&server);
        let mut second = connect(&server);
        assert_eq!(
            round_trip(&mut second, "two"),
            "two",
            "answered while the first is open"
        );
        assert_eq!(
            round_trip(&mut first, "one"),
            "one",
            "and the first still is"
        );
        dropped_within_wait(server);
    }

    #[test]
    fn the_drop_with_no_connection_ends_every_thread() {
        let held = Arc::new(());
        let server = StubServer::start(echo(Arc::clone(&held)));
        dropped_within_wait(server);
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "no thread of the server still holds what its handler holds"
        );
    }

    #[test]
    fn the_drop_ends_a_handler_whose_client_is_still_connected() {
        let held = Arc::new(());
        let server = StubServer::start(echo(Arc::clone(&held)));
        let mut client = connect(&server);
        assert_eq!(round_trip(&mut client, "x"), "x", "the handler runs");
        until_count(&held, 3);
        dropped_within_wait(server);
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "the handler of a connection still open has ended and been joined"
        );
        let mut rest = Vec::new();
        assert_eq!(
            client
                .read_to_end(&mut rest)
                .expect("the end of the stream"),
            0,
            "the client sees its connection closed"
        );
    }

    #[test]
    fn the_drop_ends_a_handler_that_writes_to_a_client_that_never_reads() {
        let held = Arc::new(());
        let server = StubServer::start({
            let held = Arc::clone(&held);
            move |mut stream: TcpStream| {
                let _held = Arc::clone(&held);
                let chunk = vec![b' '; 64 * 1024];
                while stream.write_all(&chunk).is_ok() {}
            }
        });
        let client = connect(&server);
        until_count(&held, 3);
        dropped_within_wait(server);
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "a handler blocked on a full socket has ended and been joined"
        );
        drop(client);
    }

    #[test]
    fn the_drop_ends_the_handlers_of_many_connections() {
        let held = Arc::new(());
        let server = StubServer::start(echo(Arc::clone(&held)));
        let mut clients: Vec<TcpStream> = (0..8).map(|_| connect(&server)).collect();
        for (i, client) in clients.iter_mut().enumerate() {
            assert_eq!(
                round_trip(client, &format!("{i}")),
                format!("{i}"),
                "client {i} answered"
            );
        }
        until_count(&held, 10);
        dropped_within_wait(server);
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "all eight handlers have ended and been joined"
        );
    }

    #[test]
    fn the_drop_waits_for_a_handler_still_ending_after_its_stream_ends() {
        let held = Arc::new(());
        let (release, released) = mpsc::channel::<()>();
        let released = Mutex::new(released);
        let (ending, at_end) = mpsc::channel();
        let server = StubServer::start({
            let held = Arc::clone(&held);
            move |mut stream: TcpStream| {
                let _held = Arc::clone(&held);
                let mut rest = Vec::new();
                drop(stream.read_to_end(&mut rest));
                // Ends only when the test lets it: until then the drop
                // must wait.
                let _ = ending.send(());
                let _ = released
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .recv_timeout(WAIT);
            }
        });
        let client = connect(&server);
        until_count(&held, 3);
        let (done, finished) = mpsc::channel();
        let dropping = std::thread::spawn(move || {
            drop(server);
            let _ = done.send(());
        });
        at_end
            .recv_timeout(WAIT)
            .expect("the drop ends the handler's stream");
        assert!(
            finished.recv_timeout(Duration::from_millis(200)).is_err(),
            "the drop has not returned while the handler still runs"
        );
        release.send(()).expect("the handler waits");
        finished
            .recv_timeout(WAIT)
            .expect("the drop returns once the handler ends");
        dropping.join().expect("the drop does not panic");
        assert_eq!(
            Arc::strong_count(&held),
            1,
            "the handler has ended and been joined"
        );
        drop(client);
    }

    // Failure.

    #[test]
    fn a_handler_that_panicked_fails_the_drop() {
        let server = StubServer::start(|_stream: TcpStream| panic!("the stub's own failure"));
        let mut client = connect(&server);
        let mut rest = Vec::new();
        assert_eq!(
            client
                .read_to_end(&mut rest)
                .expect("the end of the stream"),
            0,
            "the connection ends when its handler panics"
        );
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(server)))
            .expect_err("the drop reports the handler's panic");
        assert!(
            said(failed.as_ref()).contains("the stub's own failure"),
            "the panic names what the handler said: {}",
            said(failed.as_ref())
        );
    }

    // Edge.

    #[test]
    fn a_connection_the_client_closed_first_does_not_hold_the_drop() {
        let held = Arc::new(());
        let server = StubServer::start(echo(Arc::clone(&held)));
        let mut client = connect(&server);
        assert_eq!(round_trip(&mut client, "x"), "x", "the handler runs");
        drop(client);
        until_count(&held, 2);
        dropped_within_wait(server);
        assert_eq!(Arc::strong_count(&held), 1, "every thread has ended");
    }

    #[test]
    fn each_server_listens_on_a_port_of_its_own() {
        let first = StubServer::start(echo(Arc::new(())));
        let second = StubServer::start(echo(Arc::new(())));
        assert_ne!(first.address(), second.address(), "two servers, two ports");
        assert!(
            first.address().ip().is_loopback(),
            "on the loopback address"
        );
        dropped_within_wait(first);
        dropped_within_wait(second);
    }

    #[test]
    fn a_handler_that_returns_at_once_is_joined() {
        let held = Arc::new(());
        let server = StubServer::start({
            let held = Arc::clone(&held);
            move |_stream: TcpStream| {
                let _held = Arc::clone(&held);
            }
        });
        let mut client = connect(&server);
        let mut rest = Vec::new();
        assert_eq!(
            client.read_to_end(&mut rest).expect("the end"),
            0,
            "the handler closed the stream"
        );
        dropped_within_wait(server);
        assert_eq!(Arc::strong_count(&held), 1, "its thread has been joined");
    }
}
