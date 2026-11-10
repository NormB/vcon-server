//! `vcon-server tui`: a terminal UI for a running server.
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

use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use ratatui::DefaultTerminal;
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
/// A sentence naming what failed: the key file, the CA file, or the
/// terminal. It never holds the token.
pub fn run(config: &ClientConfig) -> Result<(), String> {
    let api: Arc<dyn Api> = Arc::new(HttpApi::new(config)?);
    let theme = view::theme(
        std::env::var("NO_COLOR").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    );
    let mut terminal = ratatui::try_init().map_err(|e| format!("the terminal: {e}"))?;
    let result = turns(&mut terminal, &api, &client::base(&config.url), theme);
    ratatui::restore();
    result.map_err(|e| format!("the terminal: {e}"))
}

/// Draws, waits for an event and applies it, until the state says quit.
/// Each request runs on a thread of its own and its answer comes back on a
/// channel, so the keys stay live while the server is slow.
fn turns(
    terminal: &mut DefaultTerminal,
    api: &Arc<dyn Api>,
    server: &str,
    theme: Theme,
) -> std::io::Result<()> {
    let size = terminal.size()?;
    let (mut state, first) = State::new(server, (size.width, size.height));
    let (answers, inbox) = mpsc::channel();
    send(api, first, &answers);
    while !state.quit {
        terminal.draw(|frame| view::draw(frame, &state, theme))?;
        let mut events: Vec<Event> = inbox.try_iter().collect();
        if event::poll(TICK)? {
            events.extend(translate(&event::read()?));
        }
        for event in events {
            let effects = state.step(event, OffsetDateTime::now_utc());
            send(api, effects, &answers);
        }
    }
    Ok(())
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
