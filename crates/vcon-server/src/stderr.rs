//! The one way the binary writes to stderr: its log lines and its refusals.
//!
//! A reader of stderr can go away while the process runs (a `serve 2>&1 |
//! tee` whose reader ends, a log collector restarted), and every later
//! write then fails with EPIPE. A failed write has nowhere to be reported,
//! so [`Quiet`] drops it and answers as if it had been written. It must not
//! panic: `eprintln!` does, and in a request's task the panic reset the
//! request's stream, so a server whose log pipe closed answered no request;
//! in a command it turned the exit status into 101.

use std::io::{self, Write};

/// A writer that passes each write to `W` and drops any error.
#[derive(Debug)]
pub struct Quiet<W>(pub W);

/// stderr, behind [`Quiet`]: for the log's writer and for [`line()`].
#[must_use]
pub fn stderr() -> Quiet<io::Stderr> {
    Quiet(io::stderr())
}

/// Writes `text` and a newline to stderr, as `eprintln!` would, dropping
/// a failure.
pub fn line(text: &str) {
    let mut out = stderr();
    let _ = writeln!(out, "{text}");
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
}
