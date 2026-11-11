//! `vcon-server tui` on a terminal: a pseudo-terminal this test opens, with
//! the process as its session leader, as a shell starts a program.
//!
//! What only a terminal shows: the keys arrive without Enter and are not
//! echoed, the UI draws on the alternate screen, a request's answer comes
//! back from its thread and is drawn, and the terminal is as it was after
//! every way out: a quit, an error and a panic. The state machine, the
//! view and the loop have their own tests against a test backend; these
//! run the binary.
//!
//! The server in each test is an address nobody listens on: every request
//! fails, which is an answer the UI draws.
#![cfg(all(feature = "tui", target_os = "linux"))]

use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// How long a test waits for the UI to do one thing. The host that runs
/// these also runs builds: the wait is long, and no test sleeps it out.
const PATIENCE: Duration = Duration::from_secs(60);

const ENTER_ALTERNATE: &str = "\x1b[?1049h";
const LEAVE_ALTERNATE: &str = "\x1b[?1049l";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

/// The name of the test below that is the child of the tests of
/// `tui::with_terminal`, and the variable that tells it what to do.
const CHILD_TEST: &str = "the_child_of_the_with_terminal_tests";
const CHILD_DOES: &str = "VCON_TUI_TERMINAL_CHILD";

/// The modes of the terminal that a raw mode changes, as four numbers.
type Modes = (
    libc::tcflag_t,
    libc::tcflag_t,
    libc::tcflag_t,
    libc::tcflag_t,
);

/// A pseudo-terminal and the process on it.
struct Session {
    master: File,
    child: Child,
    output: Receiver<Vec<u8>>,
    seen: String,
    before: Modes,
    size: (u16, u16),
}

/// The screen a terminal of `columns` by `lines` shows after `written`: its
/// rows as text. The UI draws only the cells that changed, with a cursor
/// move between them, so a sentence on the screen is not a run of bytes in
/// what was written. This follows the cursor moves (`CSI row ; column H`),
/// the clear (`CSI 2 J`), carriage return and line feed, and passes over
/// every other sequence: colors, the cursor's visibility, the alternate
/// screen.
fn screen(written: &str, columns: u16, lines: u16) -> Vec<String> {
    let (columns, lines) = (usize::from(columns), usize::from(lines));
    let mut cells = vec![vec![' '; columns]; lines];
    let (mut row, mut column) = (0_usize, 0_usize);
    let mut chars = written.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' if chars.peek() == Some(&'[') => {
                chars.next();
                let mut parameters = String::new();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        last = c;
                        break;
                    }
                    parameters.push(c);
                }
                if last == 'H' {
                    let mut numbers = parameters
                        .split(';')
                        .map(|n| n.parse::<usize>().unwrap_or(1));
                    row = numbers.next().unwrap_or(1).saturating_sub(1);
                    column = numbers.next().unwrap_or(1).saturating_sub(1);
                } else if last == 'J' && parameters == "2" {
                    cells = vec![vec![' '; columns]; lines];
                }
            }
            '\r' => column = 0,
            '\n' => row += 1,
            c if !c.is_control() => {
                if let Some(cell) = cells.get_mut(row).and_then(|r| r.get_mut(column)) {
                    *cell = c;
                }
                column += 1;
            }
            _ => {}
        }
    }
    cells.into_iter().map(String::from_iter).collect()
}

/// Opens a pseudo-terminal of `columns` by `lines`: (master, slave).
fn open_terminal(columns: u16, lines: u16) -> (OwnedFd, OwnedFd) {
    let size = libc::winsize {
        ws_row: lines,
        ws_col: columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: the two numbers and the size are valid for the call; the name
    // and the modes are null, which openpty takes as "none".
    let rc = unsafe {
        libc::openpty(
            &raw mut master,
            &raw mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &raw const size,
        )
    };
    assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
    // SAFETY: openpty returned 0, so `master` is an open descriptor that
    // nothing else owns.
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    // SAFETY: as above, for `slave`.
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    (master, slave)
}

/// The terminal's modes, read through its master.
fn modes(master: &File) -> Modes {
    let mut raw = std::mem::MaybeUninit::<libc::termios>::zeroed();
    // SAFETY: the descriptor is open and `raw` is room for one termios.
    let rc = unsafe { libc::tcgetattr(master.as_raw_fd(), raw.as_mut_ptr()) };
    assert_eq!(rc, 0, "tcgetattr: {}", std::io::Error::last_os_error());
    // SAFETY: tcgetattr returned 0, so it filled `raw`.
    let t = unsafe { raw.assume_init() };
    (t.c_iflag, t.c_oflag, t.c_cflag, t.c_lflag)
}

/// Puts this process in a session of its own, which has no controlling
/// terminal. Runs in the child between fork and exec.
fn leave_session() -> std::io::Result<()> {
    // SAFETY: setsid takes no argument and is async-signal-safe.
    if unsafe { libc::setsid() } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Makes the terminal on standard input the controlling terminal of this
/// process, in a session of its own. Runs in the child between fork and
/// exec.
fn take_terminal() -> std::io::Result<()> {
    leave_session()?;
    // SAFETY: descriptor 0 is the child's standard input; the request
    // takes one integer; ioctl is async-signal-safe.
    if unsafe { libc::ioctl(0, libc::TIOCSCTTY, 0) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

impl Session {
    /// Starts `command` on a new terminal of `columns` by `lines`.
    fn start(mut command: Command, columns: u16, lines: u16) -> Self {
        let (master, slave) = open_terminal(columns, lines);
        let master = File::from(master);
        let before = modes(&master);
        command
            .stdin(Stdio::from(slave.try_clone().expect("clone the slave")))
            .stdout(Stdio::from(slave.try_clone().expect("clone the slave")))
            .stderr(Stdio::from(slave))
            .env("TERM", "xterm-256color")
            .env_remove("NO_COLOR");
        // SAFETY: the closure calls only setsid and ioctl, which are
        // async-signal-safe, and allocates only on an error it returns.
        unsafe { command.pre_exec(take_terminal) };
        let child = command.spawn().expect("the process starts");
        // The child has its own copies: with this one closed, a read of
        // the master ends when the child's are.
        drop(command);
        let mut reader = master.try_clone().expect("clone the master");
        let (send, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0_u8; 4096];
            // Ends at 0 or at the error a master gives once no process
            // holds the terminal.
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                if send.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Session {
            master,
            child,
            output,
            seen: String::new(),
            before,
            size: (columns, lines),
        }
    }

    /// Whether a row of the screen holds `text`, or the terminal was sent
    /// `text` as one run of bytes (a control sequence, or a line printed
    /// outside the UI).
    fn shows(&self, text: &str) -> bool {
        self.seen.contains(text)
            || screen(&self.seen, self.size.0, self.size.1)
                .iter()
                .any(|row| row.contains(text))
    }

    /// `vcon-server tui --config <config>` on a terminal.
    fn tui(config: &Path, columns: u16, lines: u16) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_vcon-server"));
        command.arg("tui").arg("--config").arg(config);
        Self::start(command, columns, lines)
    }

    /// This test binary again, running only [`CHILD_TEST`], which does
    /// `does` with `tui::with_terminal`.
    fn child(does: &str) -> Self {
        let mut command = Command::new(std::env::current_exe().expect("this test's own path"));
        command
            .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD_DOES, does);
        Self::start(command, 80, 24)
    }

    /// Takes what the terminal has shown since the last look.
    fn look(&mut self, wait: Duration) {
        if let Ok(bytes) = self.output.recv_timeout(wait) {
            self.seen.push_str(&String::from_utf8_lossy(&bytes));
        }
    }

    /// Waits until the terminal has shown `text`.
    fn wait_for(&mut self, text: &str) {
        let end = Instant::now() + PATIENCE;
        while !self.shows(text) {
            assert!(
                Instant::now() < end,
                "the terminal never showed {text:?}; its screen is {:#?}, from {:?}",
                screen(&self.seen, self.size.0, self.size.1),
                self.seen
            );
            self.look(Duration::from_millis(50));
        }
    }

    /// Waits until `rule` holds for the terminal's modes.
    fn wait_for_modes(&mut self, what: &str, rule: impl Fn(Modes) -> bool) {
        let end = Instant::now() + PATIENCE;
        while !rule(modes(&self.master)) {
            assert!(Instant::now() < end, "the terminal never was {what}");
            self.look(Duration::from_millis(20));
        }
    }

    /// Types `keys`.
    fn press(&mut self, keys: &[u8]) {
        self.master.write_all(keys).expect("write to the terminal");
    }

    /// Waits for the process to end, then for everything it wrote.
    fn end(&mut self) -> ExitStatus {
        let end = Instant::now() + PATIENCE;
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("wait for the process") {
                break status;
            }
            assert!(
                Instant::now() < end,
                "the process did not end; the terminal showed {:?}",
                self.seen
            );
            self.look(Duration::from_millis(20));
        };
        // The reader ends when the terminal has no process: all of the
        // output is here once its channel closes.
        while let Ok(bytes) = self.output.recv_timeout(PATIENCE) {
            self.seen.push_str(&String::from_utf8_lossy(&bytes));
        }
        status
    }

    /// Whether the process is still running.
    fn running(&mut self) -> bool {
        self.child
            .try_wait()
            .expect("wait for the process")
            .is_none()
    }

    /// Gives the terminal a new size, as a window that is resized does.
    fn resize(&mut self, columns: u16, lines: u16) {
        self.size = (columns, lines);
        let size = libc::winsize {
            ws_row: lines,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: the descriptor is open and `size` is a winsize.
        let rc = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &raw const size) };
        assert_eq!(rc, 0, "TIOCSWINSZ: {}", std::io::Error::last_os_error());
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // A test that failed half-way leaves no process behind.
        drop(self.child.kill());
        drop(self.child.wait());
    }
}

/// Whether keys arrive one at a time and are not echoed.
fn raw(m: Modes) -> bool {
    m.3 & (libc::ICANON | libc::ECHO) == 0
}

/// A client file in `dir` for a server at an address nobody listens on,
/// with its key file (mode 0600) if `with_key` says so. Returns the file
/// and the address.
fn client_file(dir: &Path, with_key: bool) -> (PathBuf, String) {
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("the address")
        .port();
    let address = format!("127.0.0.1:{port}");
    let key_file = dir.join("tui.token");
    if with_key {
        std::fs::write(&key_file, "vcs_0011223344556677_secret\n").expect("write the key file");
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    let config = dir.join("tui.toml");
    std::fs::write(
        &config,
        format!(
            "url = \"https://{address}\"\nkey_file = \"{}\"\n",
            key_file.display()
        ),
    )
    .expect("write the client file");
    (config, address)
}

/// The UI started on an 80 by 24 terminal and drawn once: the session, and
/// the server's address.
fn started(dir: &Path) -> (Session, String) {
    let (config, address) = client_file(dir, true);
    let mut session = Session::tui(&config, 80, 24);
    session.wait_for(&address);
    (session, address)
}

// The binary: a quit.

#[test]
fn it_draws_on_the_alternate_screen_and_leaves_it_at_q() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, _) = started(dir.path());
    assert!(
        session.seen.contains(ENTER_ALTERNATE),
        "no alternate screen before the first draw: {:?}",
        session.seen
    );
    assert!(
        !session.seen.contains(LEAVE_ALTERNATE),
        "the alternate screen was left while the UI runs: {:?}",
        session.seen
    );
    session.press(b"q");
    let status = session.end();
    assert!(status.success(), "q ends with {status}: {:?}", session.seen);
    let entered = session.seen.find(ENTER_ALTERNATE).expect("entered");
    let left = session.seen.rfind(LEAVE_ALTERNATE).expect("left");
    assert!(entered < left, "left before it entered: {:?}", session.seen);
}

#[test]
fn keys_arrive_without_enter_and_are_not_echoed_while_it_runs() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, _) = started(dir.path());
    assert!(
        !raw(session.before),
        "a new terminal already had no line mode: the test would show nothing"
    );
    session.wait_for_modes("without line mode and echo", raw);
    session.press(b"q");
    assert!(session.end().success(), "q ends it: {:?}", session.seen);
}

#[test]
fn the_terminal_s_modes_are_as_they_were_after_q() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, _) = started(dir.path());
    session.wait_for_modes("without line mode and echo", raw);
    session.press(b"q");
    assert!(session.end().success(), "q ends it: {:?}", session.seen);
    assert_eq!(
        modes(&session.master),
        session.before,
        "the terminal's modes after the UI"
    );
}

#[test]
fn the_cursor_is_hidden_while_it_runs_and_shown_again_after() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, _) = started(dir.path());
    session.wait_for(HIDE_CURSOR);
    session.press(b"q");
    assert!(session.end().success(), "q ends it: {:?}", session.seen);
    let hidden = session.seen.rfind(HIDE_CURSOR).expect("hidden");
    let shown = session.seen.rfind(SHOW_CURSOR).expect("shown again");
    assert!(
        hidden < shown,
        "the cursor stays hidden: {:?}",
        session.seen
    );
}

#[test]
fn ctrl_c_is_a_key_that_quits_and_not_a_signal() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, _) = started(dir.path());
    session.wait_for_modes("without line mode and echo", raw);
    session.press(b"\x03");
    let status = session.end();
    assert_eq!(status.signal(), None, "ended by a signal");
    assert!(status.success(), "Ctrl-C ends with {status}");
    assert_eq!(
        modes(&session.master),
        session.before,
        "the terminal's modes after Ctrl-C"
    );
    assert!(
        session.seen.contains(LEAVE_ALTERNATE),
        "the alternate screen after Ctrl-C: {:?}",
        session.seen
    );
}

#[test]
fn a_key_with_no_binding_does_not_end_it() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, _) = started(dir.path());
    session.wait_for_modes("without line mode and echo", raw);
    session.press(b"z\r");
    // Long enough for several turns of the loop, which reads a key within
    // a tenth of a second.
    session.look(Duration::from_millis(600));
    assert!(session.running(), "z ended the UI: {:?}", session.seen);
    assert!(
        !session.seen.contains(LEAVE_ALTERNATE),
        "z left the alternate screen: {:?}",
        session.seen
    );
    session.press(b"q");
    assert!(session.end().success(), "q ends it: {:?}", session.seen);
}

// The binary: requests and sizes.

#[test]
fn the_first_screen_names_the_server() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, address) = started(dir.path());
    assert!(
        session.seen.contains(&format!("https://{address}")),
        "the screen: {:?}",
        session.seen
    );
    session.press(b"q");
    assert!(session.end().success(), "q ends it: {:?}", session.seen);
}

#[test]
fn a_request_s_answer_comes_back_from_its_thread_and_is_drawn() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, _) = started(dir.path());
    // Nobody listens at the address: the answer is that the server did not
    // answer, and only the request's thread can bring it.
    session.wait_for("not connected");
    session.wait_for("the server did not answer");
    assert!(
        session.running(),
        "a failed request ended the UI: {:?}",
        session.seen
    );
    session.press(b"q");
    assert!(session.end().success(), "q ends it: {:?}", session.seen);
}

#[test]
fn the_key_is_never_drawn() {
    let dir = tempfile::tempdir().expect("a directory");
    let (mut session, _) = started(dir.path());
    session.wait_for("not connected");
    session.press(b"q");
    assert!(session.end().success(), "q ends it: {:?}", session.seen);
    assert!(
        !session.seen.contains("vcs_0011223344556677") && !session.seen.contains("secret"),
        "the key is on the screen: {:?}",
        session.seen
    );
}

#[test]
fn a_terminal_of_one_cell_starts_and_quits() {
    let dir = tempfile::tempdir().expect("a directory");
    let (config, _) = client_file(dir.path(), true);
    let mut session = Session::tui(&config, 1, 1);
    session.wait_for(ENTER_ALTERNATE);
    session.wait_for_modes("without line mode and echo", raw);
    session.press(b"q");
    let status = session.end();
    assert!(status.success(), "q ends with {status}: {:?}", session.seen);
    assert_eq!(
        modes(&session.master),
        session.before,
        "the terminal's modes after the UI"
    );
}

#[test]
fn a_new_size_is_drawn() {
    let dir = tempfile::tempdir().expect("a directory");
    let (config, address) = client_file(dir.path(), true);
    // Too narrow for the address on one line.
    let mut session = Session::tui(&config, 12, 6);
    session.wait_for(ENTER_ALTERNATE);
    session.wait_for_modes("without line mode and echo", raw);
    session.look(Duration::from_millis(600));
    assert!(
        !session.shows(&address),
        "12 columns showed an address of {} characters: {:?}",
        address.len(),
        session.seen
    );
    session.resize(100, 30);
    session.wait_for(&address);
    session.press(b"q");
    assert!(session.end().success(), "q ends it: {:?}", session.seen);
}

// The binary: an error.

#[test]
fn a_key_file_that_is_missing_is_reported_on_a_terminal_left_alone() {
    let dir = tempfile::tempdir().expect("a directory");
    let (config, _) = client_file(dir.path(), false);
    let mut session = Session::tui(&config, 80, 24);
    let status = session.end();
    assert!(!status.success(), "it started with no key file");
    assert!(
        session.seen.contains("tui.token"),
        "the message does not name the key file: {:?}",
        session.seen
    );
    assert!(
        !session.seen.contains(ENTER_ALTERNATE),
        "the alternate screen was entered: {:?}",
        session.seen
    );
    assert_eq!(
        modes(&session.master),
        session.before,
        "the terminal's modes after the error"
    );
}

#[test]
fn a_client_file_that_is_missing_is_reported_on_a_terminal_left_alone() {
    let dir = tempfile::tempdir().expect("a directory");
    let mut session = Session::tui(&dir.path().join("none.toml"), 80, 24);
    let status = session.end();
    assert!(!status.success(), "it started with no client file");
    assert!(
        session.seen.contains("client file"),
        "the message: {:?}",
        session.seen
    );
    assert!(
        !session.seen.contains(ENTER_ALTERNATE),
        "the alternate screen was entered: {:?}",
        session.seen
    );
    assert_eq!(
        modes(&session.master),
        session.before,
        "the terminal's modes after the error"
    );
}

// `tui::with_terminal`, which `tui::run` draws inside: a panic, a value
// and no terminal. The binary has no way to panic on request, so these
// run this test binary again as the process on the terminal.

/// Not a test of its own: the process the tests below start. Without the
/// variable it does nothing.
#[test]
fn the_child_of_the_with_terminal_tests() {
    let Ok(does) = std::env::var(CHILD_DOES) else {
        return;
    };
    let result = vcon_server::tui::with_terminal(|terminal| {
        terminal
            .draw(|frame| frame.render_widget("drawn by the child", frame.area()))
            .expect("draw");
        // The parent reads the terminal's modes while this waits.
        let mut line = [0_u8; 1];
        let n = std::io::stdin().read(&mut line).expect("read a key");
        assert_eq!(n, 1, "one key");
        assert!(does != "panic", "the child panics as it was told to");
        7
    });
    match result {
        Ok(value) => println!("the child's value is {value}"),
        Err(why) => println!("the child's error is: {why}"),
    }
}

#[test]
fn a_panic_puts_the_terminal_back_before_its_message() {
    let mut session = Session::child("panic");
    session.wait_for("drawn by the child");
    session.wait_for_modes("without line mode and echo", raw);
    assert!(
        session.seen.contains(ENTER_ALTERNATE),
        "no alternate screen: {:?}",
        session.seen
    );
    session.press(b"x");
    let status = session.end();
    assert!(!status.success(), "a panic ended with {status}");
    assert_eq!(
        modes(&session.master),
        session.before,
        "the terminal's modes after a panic"
    );
    let left = session
        .seen
        .find(LEAVE_ALTERNATE)
        .expect("left the alternate screen");
    let said = session
        .seen
        .find("the child panics as it was told to")
        .expect("the panic's message");
    assert!(
        left < said,
        "the message was written to the alternate screen, which is then thrown away: {:?}",
        session.seen
    );
}

#[test]
fn a_panic_shows_the_cursor_again() {
    let mut session = Session::child("panic");
    session.wait_for("drawn by the child");
    session.press(b"x");
    let status = session.end();
    assert!(!status.success(), "a panic ended with {status}");
    let hidden = session.seen.rfind(HIDE_CURSOR).expect("hidden");
    let shown = session.seen.rfind(SHOW_CURSOR).expect("shown again");
    assert!(
        hidden < shown,
        "the cursor stays hidden: {:?}",
        session.seen
    );
}

#[test]
fn the_body_s_value_is_returned_with_the_terminal_put_back() {
    let mut session = Session::child("return");
    session.wait_for("drawn by the child");
    session.wait_for_modes("without line mode and echo", raw);
    session.press(b"x");
    let status = session.end();
    assert!(status.success(), "ended with {status}: {:?}", session.seen);
    assert_eq!(
        modes(&session.master),
        session.before,
        "the terminal's modes after the body"
    );
    let left = session
        .seen
        .find(LEAVE_ALTERNATE)
        .expect("left the alternate screen");
    let value = session
        .seen
        .find("the child's value is 7")
        .expect("the value");
    assert!(
        left < value,
        "the value was printed first: {:?}",
        session.seen
    );
}

#[test]
fn the_body_draws_on_the_alternate_screen_in_raw_mode() {
    let mut session = Session::child("return");
    session.wait_for("drawn by the child");
    let entered = session.seen.find(ENTER_ALTERNATE).expect("entered");
    let drawn = session.seen.find("drawn").expect("drawn");
    assert!(
        entered < drawn,
        "drawn on the main screen: {:?}",
        session.seen
    );
    session.wait_for_modes("without line mode and echo", raw);
    session.press(b"x");
    assert!(session.end().success(), "the child: {:?}", session.seen);
}

#[test]
fn without_a_terminal_the_body_does_not_run_and_the_error_says_the_terminal() {
    // In a session of its own, so that it has no controlling terminal
    // whoever runs the tests.
    let mut command = Command::new(std::env::current_exe().expect("this test's own path"));
    command
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_DOES, "return")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: the function calls only setsid, which is async-signal-safe,
    // and allocates only on an error it returns.
    unsafe { command.pre_exec(leave_session) };
    let out = command.output().expect("the child runs");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("the child's error is: the terminal: "),
        "stdout: {said:?}"
    );
    assert!(
        !said.contains("drawn") && !said.contains(ENTER_ALTERNATE),
        "the body ran: {said:?}"
    );
}
