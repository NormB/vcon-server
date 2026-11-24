//! The one way the binary writes to stderr: its log lines and its refusals.
//!
//! A reader of stderr can go away while the process runs (a `serve 2>&1 |
//! tee` whose reader ends, a log collector restarted), and every later
//! write then fails with EPIPE. A failed write has nowhere to be reported,
//! so [`Quiet`] drops it and answers as if it had been written. It must not
//! panic: `eprintln!` does, and in a request's task the panic reset the
//! request's stream, so a server whose log pipe closed answered no request;
//! in a command it turned the exit status into 101.
//!
//! A reader can also stop reading and keep its end open. The pipe then
//! fills, and a write to it waits until the reader reads. A request's task
//! must never wait for that, so no caller writes to stderr itself: each
//! line is put on a bounded queue ([`CAPACITY`] bytes) that one thread of
//! its own writes out. A line that finds the queue full is dropped and
//! counted, and once the thread can write again it writes, where those
//! lines would have been, a line that says how many were dropped. Nothing
//! but the log depends on this: audit records are written with the change
//! they describe, in the catalog.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// The most bytes of log lines the queue holds while the thread waits for
/// the reader: about seven thousand request lines.
pub const CAPACITY: usize = 1 << 20;

/// How long the binary waits at exit for its last lines to be written
/// ([`drain`]): a reader that reads gets them, one that does not cannot
/// hold the exit longer.
pub const EXIT_DRAIN: Duration = Duration::from_secs(2);

/// A writer that passes each write to `W` and drops any error.
#[derive(Debug)]
pub struct Quiet<W>(pub W);

/// The process's stderr queue, started on the first line.
static STDERR: OnceLock<Sink> = OnceLock::new();

fn process() -> &'static Sink {
    STDERR.get_or_init(|| Sink::start(Quiet(io::stderr()), CAPACITY))
}

/// stderr, behind the queue: for the log's writer and for [`line()`]. A
/// write returns at once, whether or not the reader reads.
#[must_use]
pub fn stderr() -> Queued {
    process().writer()
}

/// Writes `text` and a newline to stderr, as `eprintln!` would, as one
/// line of the queue, dropping a failure.
pub fn line(text: &str) {
    line_to(&mut stderr(), text);
}

/// [`line()`] onto `out`: one write, so a queue takes or drops the line
/// whole.
fn line_to(out: &mut impl Write, text: &str) {
    let _ = out.write(format!("{text}\n").as_bytes());
}

/// Waits until every line queued so far is written, at most `within`:
/// whether they were. Called before the process exits, so its last lines
/// reach a reader that reads, and a reader that does not cannot hold the
/// exit.
pub fn drain(within: Duration) -> bool {
    STDERR.get().is_none_or(|sink| sink.drain(within))
}

impl<W: Write> Write for Quiet<W> {
    /// The length of `buf`, whether or not `W` took it: a part it did not
    /// take is dropped with the rest, never written again.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = self.0.write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.0.flush();
        Ok(())
    }
}

/// The line the thread writes in place of `count` dropped lines.
fn notice(count: u64) -> String {
    let lines = if count == 1 { "line" } else { "lines" };
    format!("vcon-server: {count} log {lines} dropped: standard error was full\n")
}

/// One place in the queue: a line, or the lines dropped there.
enum Entry {
    Line(Vec<u8>),
    Dropped(u64),
}

/// What the queue's users and its thread share.
struct Shared {
    state: Mutex<State>,
    /// Signaled when an entry is queued or the sink closes.
    ready: Condvar,
    /// Signaled when the thread has written everything queued.
    idle: Condvar,
    capacity: usize,
    out: Mutex<Box<dyn Write + Send>>,
}

struct State {
    entries: VecDeque<Entry>,
    /// The bytes of the lines in `entries`.
    bytes: usize,
    /// Whether the thread is writing an entry it took off the queue.
    busy: bool,
    /// The [`Sink::drain`] calls waiting for the thread.
    draining: usize,
    /// Lines dropped since the sink started.
    dropped: u64,
    /// Whether a thread writes the queue; without one, each line is
    /// written by its caller.
    threaded: bool,
    closed: bool,
}

/// How a sink starts its thread: [`std::thread::Builder::spawn`], or a
/// stand-in for a test.
type Spawn = fn(Box<dyn FnOnce() + Send>) -> io::Result<()>;

fn spawn_thread(run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    // The process's sink is a static that lives as long as the process,
    // and its thread with it; the exit waits for its queue with a bound
    // (drain), never a join, because a reader that does not read must not
    // hold the exit, and a join would wait for it. A test's sink thread
    // ends once its sink is dropped and its queue written; it holds only
    // that sink's writer.
    // thread-detached: the writer of the process's stderr, see above.
    std::thread::Builder::new()
        .name("stderr".to_owned())
        .spawn(run)
        .map(drop)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A bounded queue of lines for `out`, and the thread that writes them.
/// Dropping it ends the thread once the queue is written.
struct Sink {
    shared: Arc<Shared>,
}

impl Sink {
    fn start(out: impl Write + Send + 'static, capacity: usize) -> Sink {
        Sink::start_with(out, capacity, spawn_thread)
    }

    /// [`Sink::start`] with `spawn` to start the thread. When it cannot
    /// start one, each line is written by its caller.
    fn start_with(out: impl Write + Send + 'static, capacity: usize, spawn: Spawn) -> Sink {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                entries: VecDeque::new(),
                bytes: 0,
                busy: false,
                draining: 0,
                dropped: 0,
                threaded: false,
                closed: false,
            }),
            ready: Condvar::new(),
            idle: Condvar::new(),
            capacity,
            out: Mutex::new(Box::new(out)),
        });
        let threaded = {
            let shared = shared.clone();
            spawn(Box::new(move || shared.write_out())).is_ok()
        };
        lock(&shared.state).threaded = threaded;
        Sink { shared }
    }

    fn writer(&self) -> Queued {
        Queued(self.shared.clone())
    }

    /// Waits until the thread has written every entry queued, at most
    /// `within`: whether it has.
    fn drain(&self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        let mut state = lock(&self.shared.state);
        while state.busy || !state.entries.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state.draining += 1;
            state = self
                .shared
                .idle
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
            state.draining -= 1;
        }
        true
    }

    #[cfg(test)]
    fn dropped(&self) -> u64 {
        lock(&self.shared.state).dropped
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        lock(&self.shared.state).closed = true;
        self.shared.ready.notify_all();
    }
}

impl Shared {
    /// Queues `buf` as one line, or counts it dropped when the queue has
    /// no room for it.
    fn offer(&self, buf: &[u8]) {
        if buf.is_empty() {
            return;
        }
        let mut state = lock(&self.state);
        if !state.threaded {
            drop(state);
            let _ = lock(&self.out).write_all(buf);
            return;
        }
        // An empty queue takes a line of any length, so a line longer than
        // the capacity is not always dropped.
        if state.closed || (state.bytes > 0 && state.bytes + buf.len() > self.capacity) {
            state.dropped += 1;
            if let Some(Entry::Dropped(count)) = state.entries.back_mut() {
                *count += 1;
            } else if !state.closed {
                state.entries.push_back(Entry::Dropped(1));
            }
            return;
        }
        state.bytes += buf.len();
        state.entries.push_back(Entry::Line(buf.to_vec()));
        drop(state);
        self.ready.notify_one();
    }

    /// The thread: writes each entry as it is queued, until the sink is
    /// dropped and the queue is written.
    fn write_out(&self) {
        loop {
            let entry = {
                let mut state = lock(&self.state);
                loop {
                    if let Some(entry) = state.entries.pop_front() {
                        if let Entry::Line(line) = &entry {
                            state.bytes -= line.len();
                        }
                        state.busy = true;
                        break entry;
                    }
                    state.busy = false;
                    self.idle.notify_all();
                    if state.closed {
                        return;
                    }
                    state = self
                        .ready
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            };
            let mut out = lock(&self.out);
            let _ = match entry {
                Entry::Line(line) => out.write_all(&line),
                Entry::Dropped(count) => out.write_all(notice(count).as_bytes()),
            };
        }
    }
}

/// A writer onto a sink's queue: each write is one line, and returns at
/// once.
pub struct Queued(Arc<Shared>);

impl std::fmt::Debug for Queued {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Queued")
    }
}

impl Write for Queued {
    /// The length of `buf`: it is queued, or dropped and counted.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.offer(buf);
        Ok(buf.len())
    }

    /// Nothing to do: the thread writes each line as it comes.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The writing end of a pipe whose reading end is closed.
    fn closed() -> io::PipeWriter {
        let (reader, writer) = io::pipe().expect("a pipe");
        drop(reader);
        writer
    }

    /// A writer that fails each write with `kind`.
    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(self.0))
        }
    }

    /// A writer that takes at most `each` bytes per write, fails with
    /// `then` once it holds `cap` bytes, and fails once with Interrupted
    /// first when `interrupt`.
    struct Short {
        got: Vec<u8>,
        each: usize,
        cap: usize,
        then: io::ErrorKind,
        interrupt: bool,
    }

    impl Short {
        fn new(each: usize, cap: usize) -> Short {
            Short {
                got: Vec::new(),
                each,
                cap,
                then: io::ErrorKind::BrokenPipe,
                interrupt: false,
            }
        }
    }

    impl Write for Short {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if std::mem::take(&mut self.interrupt) {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            if self.got.len() >= self.cap {
                return Err(io::Error::from(self.then));
            }
            let n = buf.len().min(self.each).min(self.cap - self.got.len());
            self.got.extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    // Success.

    #[test]
    fn short_writes_deliver_every_byte() {
        let mut out = Quiet(Short::new(3, usize::MAX));
        assert_eq!(
            out.write(b"a log line\n").expect("no error"),
            11,
            "all of it"
        );
        assert_eq!(out.0.got, b"a log line\n", "taken three bytes at a time");
    }

    #[test]
    fn a_write_reaches_the_writer() {
        let mut out = Quiet(Vec::new());
        out.write_all(b"a log line\n").expect("written");
        assert_eq!(out.0, b"a log line\n", "the bytes as given");
    }

    #[test]
    fn a_write_to_an_open_pipe_is_read_back() {
        use std::io::Read;
        let (mut reader, writer) = io::pipe().expect("a pipe");
        let mut out = Quiet(writer);
        out.write_all(b"serving\n").expect("written");
        drop(out);
        let mut read = String::new();
        reader.read_to_string(&mut read).expect("read");
        assert_eq!(read, "serving\n", "the reader gets the line");
    }

    #[test]
    fn a_line_longer_than_a_pipe_reaches_the_writer_whole() {
        let long = vec![b'x'; 70_000];
        let mut out = Quiet(Vec::new());
        out.write_all(&long).expect("written");
        assert_eq!(out.0.len(), 70_000, "every byte");
    }

    #[test]
    fn two_writes_reach_the_writer_in_order() {
        let mut out = Quiet(Vec::new());
        writeln!(out, "serving").expect("written");
        writeln!(out, "stopped").expect("written");
        assert_eq!(out.0, b"serving\nstopped\n", "in the order written");
    }

    #[test]
    fn a_flush_reaches_the_writer() {
        let mut out = Quiet(Vec::new());
        out.flush().expect("flushed");
        assert!(out.0.is_empty(), "nothing written by a flush");
    }

    // Failure.

    #[test]
    fn a_write_to_a_closed_pipe_is_dropped() {
        let mut out = Quiet(closed());
        assert_eq!(
            out.write(b"a log line\n").expect("no error"),
            11,
            "answered as written"
        );
    }

    #[test]
    fn write_all_to_a_closed_pipe_is_not_an_error() {
        let mut out = Quiet(closed());
        assert!(
            out.write_all(b"vcon-server: refused\n").is_ok(),
            "no error to report"
        );
    }

    #[test]
    fn a_flush_of_a_failing_writer_is_not_an_error() {
        let mut out = Quiet(Failing(io::ErrorKind::BrokenPipe));
        assert!(out.flush().is_ok(), "no error to report");
    }

    #[test]
    fn any_kind_of_write_error_is_dropped() {
        for kind in [
            io::ErrorKind::BrokenPipe,
            io::ErrorKind::WriteZero,
            io::ErrorKind::Other,
        ] {
            let mut out = Quiet(Failing(kind));
            assert!(out.write_all(b"x").is_ok(), "{kind:?} dropped");
        }
    }

    // Edge.

    #[test]
    fn a_write_cut_off_part_way_answers_its_whole_length() {
        let mut out = Quiet(Short::new(4, 6));
        assert_eq!(
            out.write(b"serving now\n").expect("no error"),
            12,
            "the whole line"
        );
        assert_eq!(out.0.got, b"servin", "the part taken before the failure");
        assert_eq!(
            out.write(b"x").expect("no error"),
            1,
            "and the next write too"
        );
        assert_eq!(out.0.got, b"servin", "nothing more taken");
    }

    #[test]
    fn an_interrupted_write_is_tried_again() {
        let mut short = Short::new(usize::MAX, usize::MAX);
        short.interrupt = true;
        let mut out = Quiet(short);
        assert_eq!(out.write(b"stopped\n").expect("no error"), 8, "written");
        assert_eq!(out.0.got, b"stopped\n", "after the interruption");
    }

    #[test]
    fn an_empty_write_is_zero_bytes() {
        let mut out = Quiet(closed());
        assert_eq!(out.write(b"").expect("no error"), 0, "nothing to write");
    }

    #[test]
    fn writeln_to_a_closed_pipe_does_not_panic() {
        let mut out = Quiet(closed());
        assert!(
            writeln!(out, "{}", "x".repeat(70_000)).is_ok(),
            "a line past a pipe's buffer"
        );
    }

    #[test]
    fn line_writes_to_the_process_stderr_without_an_error() {
        // The test harness holds stderr open; the call returns.
        line("stderr::line test");
    }

    // ---- The queue and its thread. ----

    /// How long a test waits for the thread before it fails.
    const WAIT: Duration = Duration::from_secs(10);

    /// A writer whose bytes the test reads back.
    #[derive(Clone, Default)]
    struct Collect(Arc<Mutex<Vec<u8>>>);

    impl Collect {
        fn text(&self) -> String {
            String::from_utf8_lossy(&lock(&self.0)).into_owned()
        }
    }

    impl Write for Collect {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A writer that holds each write until the test opens it, and says
    /// when a write has begun: the thread, stopped inside a write, as on a
    /// full pipe.
    #[derive(Clone)]
    struct Gate {
        got: Collect,
        open: Arc<(Mutex<bool>, Condvar)>,
        began: Arc<Mutex<std::sync::mpsc::Sender<()>>>,
    }

    impl Gate {
        /// A closed gate, and the receiver that hears each write begin.
        fn new() -> (Gate, std::sync::mpsc::Receiver<()>) {
            let (tx, rx) = std::sync::mpsc::channel();
            let gate = Gate {
                got: Collect::default(),
                open: Arc::new((Mutex::new(false), Condvar::new())),
                began: Arc::new(Mutex::new(tx)),
            };
            (gate, rx)
        }

        fn set(&self, open: bool) {
            *lock(&self.open.0) = open;
            self.open.1.notify_all();
        }
    }

    impl Write for Gate {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let _ = lock(&self.began).send(());
            let (open, changed) = &*self.open;
            let mut is_open = lock(open);
            while !*is_open {
                is_open = changed
                    .wait(is_open)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            drop(is_open);
            self.got.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A sink on a closed gate whose thread is inside the write of
    /// `first`: what is offered from here on waits in the queue.
    fn stopped_sink(capacity: usize, first: &[u8]) -> (Sink, Gate) {
        let (gate, began) = Gate::new();
        let sink = Sink::start(gate.clone(), capacity);
        sink.writer().write_all(first).expect("queued");
        began
            .recv_timeout(WAIT)
            .expect("the thread begins to write the first line");
        (sink, gate)
    }

    fn offer(sink: &Sink, line: &str) {
        let mut out = sink.writer();
        assert_eq!(
            out.write(line.as_bytes()).expect("never an error"),
            line.len(),
            "a write answers its whole length"
        );
    }

    /// A spawn that always fails, as when the process may start no thread.
    fn no_thread(_: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::WouldBlock))
    }

    /// Runs `f` on a thread of its own and fails if it takes past `WAIT`.
    fn within_wait(f: impl FnOnce() + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        let runner = std::thread::spawn(move || {
            f();
            let _ = tx.send(());
        });
        rx.recv_timeout(WAIT)
            .expect("returns without waiting for the reader");
        // The thread has sent, so it is at its end: wait for it, so no
        // test leaves a thread running past its own end.
        runner.join().expect("the thread ends");
    }

    // Success.

    #[test]
    fn a_line_offered_reaches_the_writer() {
        let got = Collect::default();
        let sink = Sink::start(got.clone(), CAPACITY);
        offer(&sink, "serving\n");
        assert!(sink.drain(WAIT), "written within the wait");
        assert_eq!(got.text(), "serving\n", "the line as offered");
    }

    #[test]
    fn lines_reach_the_writer_in_the_order_offered() {
        let got = Collect::default();
        let sink = Sink::start(got.clone(), CAPACITY);
        let want: String = (0..100).map(|i| format!("line {i}\n")).collect();
        for i in 0..100 {
            offer(&sink, &format!("line {i}\n"));
        }
        assert!(sink.drain(WAIT), "written within the wait");
        assert_eq!(got.text(), want, "in order, none lost");
    }

    #[test]
    fn drain_returns_once_every_line_is_written() {
        let got = Collect::default();
        let sink = Sink::start(got.clone(), CAPACITY);
        for _ in 0..1000 {
            offer(&sink, "x\n");
        }
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(
            got.text().len(),
            2000,
            "every line written when drain returns"
        );
    }

    #[test]
    fn drain_returns_when_the_writer_finishes_not_at_its_time() {
        let (sink, gate) = stopped_sink(CAPACITY, b"first\n");
        let sink = Arc::new(sink);
        let (done_tx, done) = std::sync::mpsc::channel();
        let draining = std::thread::spawn({
            let sink = sink.clone();
            move || {
                let _ = done_tx.send(sink.drain(WAIT * 30));
            }
        });
        // Opened only once the drain waits, so it must be woken.
        let deadline = Instant::now() + WAIT;
        while lock(&sink.shared.state).draining == 0 {
            assert!(Instant::now() < deadline, "the drain begins to wait");
            std::thread::yield_now();
        }
        gate.set(true);
        assert!(
            done.recv_timeout(WAIT)
                .expect("drain returns once the line is written"),
            "drained"
        );
        draining.join().expect("the draining thread ends");
        assert_eq!(
            Arc::strong_count(&sink),
            1,
            "the draining thread has ended and dropped its clone of the sink"
        );
    }

    #[test]
    fn drain_with_nothing_offered_returns_true_at_once() {
        let sink = Sink::start(Collect::default(), CAPACITY);
        let began = Instant::now();
        assert!(sink.drain(WAIT), "nothing to wait for");
        assert!(began.elapsed() < WAIT, "at once: {:?}", began.elapsed());
    }

    #[test]
    fn lines_of_many_threads_all_arrive_whole() {
        let got = Collect::default();
        let sink = Arc::new(Sink::start(got.clone(), CAPACITY));
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let sink = sink.clone();
                std::thread::spawn(move || {
                    for i in 0..100 {
                        offer(&sink, &format!("thread {t} line {i}\n"));
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("each thread offers its lines");
        }
        assert!(sink.drain(WAIT), "drained");
        let text = got.text();
        assert_eq!(text.lines().count(), 800, "every line");
        for t in 0..8 {
            let mine: Vec<&str> = text
                .lines()
                .filter(|l| l.starts_with(&format!("thread {t} ")))
                .collect();
            let want: Vec<String> = (0..100).map(|i| format!("thread {t} line {i}")).collect();
            assert_eq!(mine, want, "thread {t}'s lines whole and in its order");
        }
    }

    #[test]
    fn the_count_of_dropped_lines_is_written_where_they_were_lost() {
        let (sink, gate) = stopped_sink(8, b"first\n");
        offer(&sink, "second\n");
        offer(&sink, "lost 1\n");
        offer(&sink, "lost 2\n");
        gate.set(true);
        assert!(sink.drain(WAIT), "drained");
        offer(&sink, "after\n");
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(
            gate.got.text(),
            format!("first\nsecond\n{}after\n", notice(2)),
            "the count sits between the last line kept and the next"
        );
    }

    #[test]
    fn a_reader_that_reads_again_gets_the_lines_after_the_drops() {
        use std::io::Read;
        let (mut reader, writer) = io::pipe().expect("a pipe");
        let sink = Sink::start(Quiet(writer), 4096);
        let line = format!("{}\n", "x".repeat(99));
        within_wait({
            let mut out = sink.writer();
            move || {
                for _ in 0..5000 {
                    let _ = out.write(line.as_bytes());
                }
            }
        });
        assert!(sink.dropped() > 0, "a pipe never read drops lines");
        // The reader reads again, on a thread of its own, to the end.
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let reading = std::thread::spawn(move || {
            let mut read = Vec::new();
            let _ = reader.read_to_end(&mut read);
            let _ = read_tx.send(read);
        });
        assert!(sink.drain(WAIT), "the queue drains once the reader reads");
        offer(&sink, "after\n");
        drop(sink);
        let read = read_rx
            .recv_timeout(WAIT)
            .expect("the pipe ends once the sink is dropped");
        reading.join().expect("the reading thread ends");
        let text = String::from_utf8_lossy(&read);
        assert!(
            text.contains("log lines dropped"),
            "the count is read: {}",
            text.len()
        );
        assert!(text.ends_with("after\n"), "and the line after the drops");
    }

    #[test]
    fn line_queues_the_text_and_a_newline_as_one_line() {
        // 7 bytes of room: "refused" and then "\n" as two writes would
        // drop the newline; "refused\n" as one is taken by the empty queue.
        let (sink, gate) = stopped_sink(7, b"first\n");
        line_to(&mut sink.writer(), "refused");
        assert_eq!(sink.dropped(), 0, "the line is taken whole");
        line_to(&mut sink.writer(), "again");
        assert_eq!(sink.dropped(), 1, "and the next finds no room");
        gate.set(true);
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(
            gate.got.text(),
            format!("first\nrefused\n{}", notice(1)),
            "the line and the count"
        );
    }

    #[test]
    fn the_queued_writer_answers_each_write_with_its_whole_length() {
        let (sink, gate) = stopped_sink(4, b"first\n");
        let mut out = sink.writer();
        for line in ["fits", "dropped", ""] {
            assert_eq!(
                out.write(line.as_bytes()).expect("never an error"),
                line.len(),
                "{line:?} answered whole"
            );
        }
        gate.set(true);
    }

    #[test]
    fn a_flush_of_the_queued_writer_returns_at_once() {
        let (sink, gate) = stopped_sink(CAPACITY, b"first\n");
        offer(&sink, "queued\n");
        within_wait({
            let mut out = sink.writer();
            move || out.flush().expect("never an error")
        });
        gate.set(true);
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(gate.got.text(), "first\nqueued\n", "a flush adds nothing");
    }

    #[test]
    fn tracing_events_reach_the_writer_as_lines() {
        let got = Collect::default();
        let sink = Arc::new(Sink::start(got.clone(), CAPACITY));
        let make = {
            let sink = sink.clone();
            move || sink.writer()
        };
        let subscriber = tracing_subscriber::fmt()
            .with_writer(make)
            .without_time()
            .with_target(false)
            .log_internal_errors(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("serving");
            tracing::warn!(count = 2, "two");
        });
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(
            got.text(),
            " INFO serving\n WARN two count=2\n",
            "one line per event"
        );
    }

    #[test]
    fn the_process_queue_drains_what_line_wrote() {
        line("stderr::drain test");
        assert!(drain(WAIT), "the harness reads stderr, so it drains");
    }

    // Failure.

    #[test]
    fn a_write_to_a_pipe_never_read_returns_at_once() {
        let (reader, writer) = io::pipe().expect("a pipe");
        let sink = Sink::start(Quiet(writer), 4096);
        let mut out = sink.writer();
        within_wait(move || {
            for _ in 0..10_000 {
                let _ = out.write(b"a line of the log that nobody reads\n");
            }
        });
        drop(reader);
    }

    #[test]
    fn lines_that_find_the_queue_full_are_counted() {
        let (sink, gate) = stopped_sink(10, b"first\n");
        for _ in 0..12 {
            offer(&sink, "12345\n");
        }
        assert_eq!(sink.dropped(), 11, "one fits in 10 bytes, 11 do not");
        gate.set(true);
    }

    #[test]
    fn the_queue_holds_no_more_than_its_capacity() {
        let (sink, gate) = stopped_sink(1000, b"first\n");
        let line = format!("{}\n", "x".repeat(99));
        for _ in 0..50 {
            offer(&sink, &line);
        }
        assert_eq!(sink.dropped(), 40, "ten 100-byte lines fit in 1000");
        gate.set(true);
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(
            gate.got.text(),
            format!("first\n{}{}", line.repeat(10), notice(40)),
            "the ten kept and the count"
        );
    }

    #[test]
    fn one_dropped_line_is_named_in_the_singular() {
        assert_eq!(
            notice(1),
            "vcon-server: 1 log line dropped: standard error was full\n",
            "one line"
        );
    }

    #[test]
    fn more_dropped_lines_are_named_in_the_plural() {
        assert_eq!(
            notice(2),
            "vcon-server: 2 log lines dropped: standard error was full\n",
            "two lines"
        );
    }

    #[test]
    fn a_writer_that_fails_every_write_does_not_stop_the_queue() {
        let sink = Sink::start(Quiet(Failing(io::ErrorKind::BrokenPipe)), 64);
        for _ in 0..100 {
            offer(&sink, "x\n");
        }
        assert!(sink.drain(WAIT), "drained though each write failed");
    }

    #[test]
    fn a_closed_pipe_drops_each_line_and_drains() {
        let sink = Sink::start(Quiet(closed()), CAPACITY);
        for _ in 0..1000 {
            offer(&sink, "a log line\n");
        }
        assert!(sink.drain(WAIT), "drained onto a closed pipe");
    }

    #[test]
    fn drain_gives_up_at_its_time_while_the_writer_waits() {
        let (sink, gate) = stopped_sink(CAPACITY, b"first\n");
        let began = Instant::now();
        assert!(
            !sink.drain(Duration::from_millis(200)),
            "not drained while the writer waits"
        );
        let took = began.elapsed();
        assert!(
            took >= Duration::from_millis(200) && took < WAIT,
            "it waited its time and no longer: {took:?}"
        );
        gate.set(true);
    }

    #[test]
    fn a_sink_whose_thread_cannot_start_writes_each_line_itself() {
        let got = Collect::default();
        let sink = Sink::start_with(got.clone(), CAPACITY, no_thread);
        offer(&sink, "serving\n");
        assert_eq!(got.text(), "serving\n", "written before the write returns");
        assert!(sink.drain(Duration::ZERO), "nothing is left queued");
    }

    #[test]
    fn a_sink_whose_thread_cannot_start_drops_nothing() {
        let got = Collect::default();
        let sink = Sink::start_with(got.clone(), 4, no_thread);
        for _ in 0..10 {
            offer(&sink, "longer than four\n");
        }
        assert_eq!(sink.dropped(), 0, "no queue to fill");
        assert_eq!(got.text().lines().count(), 10, "each line written");
    }

    #[test]
    fn lines_offered_after_the_sink_closed_are_dropped_and_counted() {
        let got = Collect::default();
        let sink = Sink::start(got.clone(), CAPACITY);
        let mut out = sink.writer();
        assert!(sink.drain(WAIT), "drained");
        drop(sink);
        assert_eq!(out.write(b"late\n").expect("never an error"), 5, "answered");
        assert_eq!(lock(&out.0.state).dropped, 1, "counted as dropped");
        assert!(lock(&out.0.state).entries.is_empty(), "never queued");
    }

    #[test]
    fn a_failing_flush_does_not_stop_the_queue() {
        struct BadFlush(Collect);
        impl Write for BadFlush {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
        }
        let got = Collect::default();
        let sink = Sink::start(Quiet(BadFlush(got.clone())), CAPACITY);
        offer(&sink, "one\n");
        offer(&sink, "two\n");
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(got.text(), "one\ntwo\n", "both written");
    }

    #[test]
    fn a_short_writer_gets_each_line_whole_from_the_thread() {
        #[derive(Clone)]
        struct Threes(Collect);
        impl Write for Threes {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                let n = buf.len().min(3);
                self.0.write(&buf[..n])
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let got = Collect::default();
        let sink = Sink::start(Quiet(Threes(got.clone())), CAPACITY);
        offer(&sink, "a log line\n");
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(got.text(), "a log line\n", "three bytes at a time, whole");
    }

    #[test]
    fn a_line_cut_off_by_a_failing_writer_is_not_written_again() {
        #[derive(Clone)]
        struct CutAt(Collect, usize);
        impl Write for CutAt {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                let have = lock(&(self.0).0).len();
                if have >= self.1 {
                    return Err(io::Error::from(io::ErrorKind::BrokenPipe));
                }
                let n = buf.len().min(self.1 - have);
                self.0.write(&buf[..n])
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let got = Collect::default();
        let sink = Sink::start(Quiet(CutAt(got.clone(), 6)), CAPACITY);
        offer(&sink, "serving now\n");
        offer(&sink, "stopped\n");
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(got.text(), "servin", "the part taken, once");
    }

    // Edge.

    #[test]
    fn an_empty_write_is_neither_queued_nor_counted() {
        let (sink, gate) = stopped_sink(0, b"first\n");
        offer(&sink, "");
        assert_eq!(sink.dropped(), 0, "nothing to drop");
        assert!(
            lock(&sink.shared.state).entries.is_empty(),
            "nothing queued"
        );
        gate.set(true);
    }

    #[test]
    fn a_line_of_exactly_the_free_room_is_queued() {
        let (sink, gate) = stopped_sink(12, b"first\n");
        offer(&sink, "12345\n");
        offer(&sink, "abcde\n");
        assert_eq!(sink.dropped(), 0, "6 and 6 bytes fill 12");
        gate.set(true);
    }

    #[test]
    fn a_line_one_byte_past_the_free_room_is_dropped() {
        let (sink, gate) = stopped_sink(12, b"first\n");
        offer(&sink, "12345\n");
        offer(&sink, "abcdef\n");
        assert_eq!(sink.dropped(), 1, "6 and 7 bytes do not fit in 12");
        gate.set(true);
    }

    #[test]
    fn a_line_longer_than_the_capacity_is_queued_when_the_queue_is_empty() {
        let (sink, gate) = stopped_sink(4, b"first\n");
        offer(&sink, "longer than four\n");
        assert_eq!(
            sink.dropped(),
            0,
            "an empty queue takes one line of any length"
        );
        offer(&sink, "x\n");
        assert_eq!(sink.dropped(), 1, "and then has no room");
        gate.set(true);
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(
            gate.got.text(),
            format!("first\nlonger than four\n{}", notice(1)),
            "the long line whole"
        );
    }

    #[test]
    fn a_zero_capacity_queues_one_line_at_a_time() {
        let (sink, gate) = stopped_sink(0, b"first\n");
        offer(&sink, "one\n");
        offer(&sink, "two\n");
        assert_eq!(sink.dropped(), 1, "only one waits");
        gate.set(true);
    }

    #[test]
    fn the_capacity_is_counted_in_bytes_not_lines() {
        let (sink, gate) = stopped_sink(100, b"first\n");
        for _ in 0..50 {
            offer(&sink, "x\n");
        }
        assert_eq!(sink.dropped(), 0, "fifty 2-byte lines fill 100 bytes");
        offer(&sink, "x\n");
        assert_eq!(sink.dropped(), 1, "the fifty-first does not fit");
        gate.set(true);
    }

    #[test]
    fn drops_in_two_runs_are_counted_twice_where_each_happened() {
        let (gate, began) = Gate::new();
        let sink = Sink::start(gate.clone(), 6);
        offer(&sink, "first\n");
        began.recv_timeout(WAIT).expect("the first write begins");
        offer(&sink, "kept1\n");
        offer(&sink, "lost\n");
        gate.set(true);
        assert!(sink.drain(WAIT), "drained");
        gate.set(false);
        while began.try_recv().is_ok() {}
        offer(&sink, "next\n");
        began.recv_timeout(WAIT).expect("the write of next begins");
        offer(&sink, "kept2\n");
        offer(&sink, "lost\n");
        offer(&sink, "lost\n");
        gate.set(true);
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(
            gate.got.text(),
            format!("first\nkept1\n{}next\nkept2\n{}", notice(1), notice(2)),
            "each run's count where its lines were lost"
        );
        assert_eq!(sink.dropped(), 3, "every drop in the total");
    }

    #[test]
    fn a_drain_after_drops_waits_for_the_count_to_be_written() {
        let (sink, gate) = stopped_sink(1, b"first\n");
        offer(&sink, "a\n");
        offer(&sink, "b\n");
        gate.set(true);
        assert!(sink.drain(WAIT), "drained");
        assert!(
            gate.got.text().ends_with(&notice(1)),
            "the count is written before drain returns: {:?}",
            gate.got.text()
        );
    }

    #[test]
    fn a_line_without_a_newline_is_written_as_given() {
        let got = Collect::default();
        let sink = Sink::start(got.clone(), CAPACITY);
        offer(&sink, "no newline");
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(got.text(), "no newline", "the bytes as given");
    }

    #[test]
    fn bytes_that_are_not_utf8_are_written_as_given() {
        let got = Collect::default();
        let sink = Sink::start(got.clone(), CAPACITY);
        sink.writer()
            .write_all(&[0xff, 0xfe, b'\n'])
            .expect("queued");
        assert!(sink.drain(WAIT), "drained");
        assert_eq!(*lock(&got.0), vec![0xff, 0xfe, b'\n'], "byte for byte");
    }

    #[test]
    fn the_count_is_one_whole_line() {
        for count in [1, 2, 7, 1_000_000, u64::MAX] {
            let text = notice(count);
            assert!(text.starts_with("vcon-server: "), "{count}: {text:?}");
            assert!(text.ends_with('\n'), "{count}: {text:?}");
            assert_eq!(text.matches('\n').count(), 1, "{count}: one line");
            assert!(
                text.contains(&count.to_string()),
                "{count}: names the count"
            );
        }
    }

    #[test]
    fn the_thread_ends_when_the_sink_is_dropped() {
        struct Dropped(std::sync::mpsc::Sender<()>);
        impl Write for Dropped {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl Drop for Dropped {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let sink = Sink::start(Dropped(tx), CAPACITY);
        offer(&sink, "one\n");
        drop(sink);
        rx.recv_timeout(WAIT)
            .expect("the thread ends and the writer with it");
    }

    #[test]
    fn drain_of_a_process_that_never_wrote_starts_no_thread() {
        // The process queue is shared by these tests, so this asserts only
        // that the call returns at once and true when nothing waits.
        let began = Instant::now();
        assert!(drain(WAIT), "nothing waits");
        assert!(began.elapsed() < WAIT, "at once");
    }
}
