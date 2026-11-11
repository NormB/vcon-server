//! `vcon-server tui`: a terminal UI for a running server (cargo feature
//! `tui`, on by default).
//!
//! It is a client of the server's HTTP API with one API key, as any other
//! client is: it opens no data directory, catalog or master key, so the
//! server's scopes, tenant bounds and audit log apply to everything it
//! shows. This release reads only: the key's tenant's vCons as the API
//! lists them (uuid, version and times), and one vCon's row of that list.
//! It never asks the server for a vCon's content.
//!
//! The parts: [`state`] is the state machine, [`view`] draws a state,
//! [`keys`] is the one table of bindings, and [`client`] is the API behind
//! a trait. This module joins them to a real terminal.

pub mod client;
pub mod keys;
pub mod state;
pub mod view;

use std::io::IsTerminal;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event as Input, KeyEventKind};
use time::OffsetDateTime;

use crate::mcp::stdio::ClientConfig;
use client::{Api, HttpApi};
use state::{Effect, Event, State};
use view::Theme;

/// How long the loop waits for a key before it looks for an answer.
const TICK: Duration = Duration::from_millis(100);

/// What a terminal event asks of the state: a bound key's action when the
/// key goes down, or the new size. Releases, repeats of a release, the
/// mouse, focus and paste are nothing.
#[must_use]
pub fn translate(input: &Input) -> Option<Event> {
    match input {
        Input::Key(key) if key.kind != KeyEventKind::Release => {
            keys::action(key.code, key.modifiers).map(Event::Action)
        }
        Input::Resize(columns, lines) => Some(Event::Resize(*columns, *lines)),
        _ => None,
    }
}

/// Runs the UI on this process's terminal until the user quits.
///
/// The terminal is put back on every way out: a normal quit, an error, and
/// a panic (through the hook `ratatui::try_init` installs).
///
/// # Errors
///
/// A sentence naming what failed: no terminal ([`require_terminal`]), the
/// key file, the CA file, or the terminal. It never holds the token.
pub fn run(config: &ClientConfig) -> Result<(), String> {
    // First, so that no key is read for a run that cannot start.
    require_terminal(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    )?;
    let api: Arc<dyn Api> = Arc::new(HttpApi::new(config)?);
    let theme = view::theme(
        std::env::var("NO_COLOR").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    );
    with_terminal(|terminal| {
        let (answers, inbox) = mpsc::channel();
        let mut dispatch = |effects: Vec<Effect>| send(&api, effects, &answers);
        drive(
            terminal,
            &client::base(&config.url),
            theme,
            &mut Keyboard,
            &inbox,
            &mut dispatch,
            &OffsetDateTime::now_utc,
        )
        .map(|_| ())
    })?
}

/// Runs `body` with this process's terminal taken for a UI (keys one at a
/// time and not echoed, the alternate screen), and puts the terminal back
/// when `body` returns and when it panics (through the hook
/// `ratatui::try_init` installs, which runs before the panic's message is
/// written). Returns what `body` returned.
///
/// It is the one place the terminal is taken and put back, apart from the
/// UI's loop, so that a test can put a body of its own on a terminal
/// (`tests/tui_terminal.rs`).
///
/// # Errors
///
/// A sentence with the terminal's error when it cannot be taken; `body`
/// has then not run.
pub fn with_terminal<T>(
    body: impl FnOnce(&mut ratatui::DefaultTerminal) -> T,
) -> Result<T, String> {
    let mut terminal = ratatui::try_init().map_err(|e| format!("the terminal: {e}"))?;
    let value = body(&mut terminal);
    ratatui::restore();
    Ok(value)
}

/// Whether the UI can start: it reads keys from standard input and draws
/// on standard output, so each must be a terminal. `input` and `output`
/// say whether they are.
///
/// # Errors
///
/// A sentence that says a terminal is required, which of the two is not
/// one, and what to do.
pub fn require_terminal(input: bool, output: bool) -> Result<(), String> {
    let what = match (input, output) {
        (true, true) => return Ok(()),
        (false, true) => "standard input is not one",
        (true, false) => "standard output is not one",
        (false, false) => "standard input and standard output are not one",
    };
    Err(format!(
        "a terminal is required: {what}; run it in a terminal, not through a pipe, a redirect or a job without one"
    ))
}

/// Where the terminal's events come from: the keyboard in [`run`], a
/// script in tests.
pub trait Inputs {
    /// The next event, or `None` when none arrives within `wait`.
    ///
    /// # Errors
    ///
    /// The terminal's error when it cannot be read.
    fn next(&mut self, wait: Duration) -> std::io::Result<Option<Input>>;
}

/// This process's terminal.
struct Keyboard;

impl Inputs for Keyboard {
    fn next(&mut self, wait: Duration) -> std::io::Result<Option<Input>> {
        if event::poll(wait)? {
            event::read().map(Some)
        } else {
            Ok(None)
        }
    }
}

/// The loop: applies every answer in `inbox`, draws, waits one tick for an
/// event and applies it, until the state says quit. Each request the state
/// asks for goes to `dispatch`, whose answer comes back through `inbox`;
/// `now` is the clock. Returns the state at the end.
///
/// # Errors
///
/// A sentence with the terminal's error when it cannot be drawn or read.
pub fn drive<B: Backend>(
    terminal: &mut Terminal<B>,
    server: &str,
    theme: Theme,
    inputs: &mut dyn Inputs,
    inbox: &Receiver<Event>,
    dispatch: &mut dyn FnMut(Vec<Effect>),
    now: &dyn Fn() -> OffsetDateTime,
) -> Result<State, String> {
    let size = terminal.size().map_err(|e| format!("the terminal: {e}"))?;
    let (mut state, first) = State::new(server, (size.width, size.height));
    dispatch(first);
    while !state.quit {
        while let Ok(event) = inbox.try_recv() {
            dispatch(state.step(event, now()));
        }
        terminal
            .draw(|frame| view::draw(frame, &state, theme))
            .map_err(|e| format!("the terminal: {e}"))?;
        let input = inputs
            .next(TICK)
            .map_err(|e| format!("the terminal: {e}"))?;
        if let Some(event) = input.as_ref().and_then(translate) {
            dispatch(state.step(event, now()));
        }
    }
    Ok(state)
}

/// Starts each request on a thread that sends its answer to `answers`.
fn send(api: &Arc<dyn Api>, effects: Vec<Effect>, answers: &Sender<Event>) {
    for effect in effects {
        let api = Arc::clone(api);
        let answers = answers.clone();
        std::thread::spawn(move || {
            // The receiver is gone only when the UI has quit; the answer
            // then has no reader.
            drop(answers.send(client::perform(api.as_ref(), &effect)));
        });
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{
        Event as Input, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    };

    use std::collections::VecDeque;
    use std::sync::mpsc;
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use time::OffsetDateTime;
    use time::macros::datetime;

    use super::client::{self, Api};
    use super::state::{Action, ApiError, Effect, Event, KeyInfo, Page, Row, State, View};
    use super::view::Theme;
    use super::{Inputs, drive, translate};

    const NOW: OffsetDateTime = datetime!(2026-11-10 01:20:00 UTC);

    fn row(n: u8) -> Row {
        Row {
            uuid: format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02}"),
            stored_at: "2026-10-25T12:00:00Z".to_owned(),
            updated_at: "2026-10-25T12:30:00Z".to_owned(),
            version: format!("4f0c4b0a1d2e3f405162738495a6b7{n:02}"),
        }
    }

    /// A server with a reader key and two vCons.
    struct TwoVcons;

    impl Api for TwoVcons {
        fn key(&self) -> Result<KeyInfo, ApiError> {
            Ok(KeyInfo {
                id: "0123456789abcdef".to_owned(),
                tenant: Some("t_example".to_owned()),
                scopes: vec!["read".to_owned()],
            })
        }

        fn page(&self, _cursor: Option<&str>) -> Result<Page, ApiError> {
            Ok(Page {
                vcons: vec![row(0), row(1)],
                next: None,
            })
        }
    }

    /// Keys from a list. When the list ends the input fails, so a loop that
    /// does not end on quit fails its test instead of hanging.
    struct Script(VecDeque<KeyCode>);

    impl Inputs for Script {
        fn next(&mut self, _wait: Duration) -> std::io::Result<Option<Input>> {
            let code = self
                .0
                .pop_front()
                .ok_or_else(|| std::io::Error::other("the script ended"))?;
            Ok(Some(key(code, KeyEventKind::Press)))
        }
    }

    /// Runs the loop on a test terminal with `keys`, each request answered
    /// at once by [`TwoVcons`], and returns the result and the last frame.
    fn driven(keys: &[KeyCode]) -> (Result<State, String>, String) {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("a terminal");
        let (answers, inbox) = mpsc::channel();
        let mut dispatch = |effects: Vec<Effect>| {
            for effect in effects {
                answers
                    .send(client::perform(&TwoVcons, &effect))
                    .expect("the loop holds the receiver");
            }
        };
        let result = drive(
            &mut terminal,
            "https://vcon.example.com:8443",
            Theme { color: false },
            &mut Script(keys.iter().copied().collect()),
            &inbox,
            &mut dispatch,
            &|| NOW,
        );
        let buffer = terminal.backend().buffer();
        let frame = buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect();
        (result, frame)
    }

    #[test]
    fn the_loop_applies_answers_and_keys_in_order_and_ends_on_quit() {
        let (result, frame) = driven(&[KeyCode::Down, KeyCode::Enter, KeyCode::Char('q')]);
        let state = result.expect("the loop ends on q");
        assert_eq!(state.view, View::Detail, "Enter opened the detail view");
        assert_eq!(
            state.detail,
            Some(row(1).uuid),
            "both answers were applied before Down, so Down moved to the second vCon"
        );
        assert!(
            frame.contains("Content is not shown"),
            "the last frame drawn is the detail view: {frame}"
        );
    }

    fn key(code: KeyCode, kind: KeyEventKind) -> Input {
        Input::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind))
    }

    #[test]
    fn the_loop_stops_with_the_terminals_error_when_input_fails() {
        let (result, _) = driven(&[KeyCode::Down]);
        assert_eq!(
            result,
            Err("the terminal: the script ended".to_owned()),
            "the error is returned, with what it is about"
        );
    }

    #[test]
    fn the_loop_stamps_answers_with_the_clock_it_is_given() {
        let (result, _) = driven(&[KeyCode::Char('q')]);
        assert_eq!(
            result.expect("the loop ends on q").as_of,
            Some(NOW),
            "the list was read at the time the clock said"
        );
    }

    #[test]
    fn a_bound_key_going_down_is_its_action() {
        assert_eq!(
            translate(&key(KeyCode::Char('q'), KeyEventKind::Press)),
            Some(Event::Action(Action::Quit)),
            "q quits"
        );
    }

    #[test]
    fn a_held_key_repeats_its_action() {
        assert_eq!(
            translate(&key(KeyCode::Down, KeyEventKind::Repeat)),
            Some(Event::Action(Action::Down)),
            "holding Down keeps moving"
        );
    }

    #[test]
    fn a_key_coming_up_is_nothing() {
        assert_eq!(
            translate(&key(KeyCode::Char('q'), KeyEventKind::Release)),
            None,
            "a terminal that reports releases must not act twice"
        );
    }

    #[test]
    fn an_unbound_key_is_nothing() {
        assert_eq!(
            translate(&key(KeyCode::Char('x'), KeyEventKind::Press)),
            None,
            "x is not bound"
        );
    }

    #[test]
    fn a_resize_carries_the_new_size() {
        assert_eq!(
            translate(&Input::Resize(132, 43)),
            Some(Event::Resize(132, 43)),
            "columns, then lines"
        );
    }

    #[test]
    fn focus_and_paste_are_nothing() {
        assert_eq!(
            translate(&Input::FocusGained),
            None,
            "focus changes nothing"
        );
        assert_eq!(
            translate(&Input::Paste("q".to_owned())),
            None,
            "pasted text is not typed keys"
        );
    }
}
