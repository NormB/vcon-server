//! The terminal UI's page, website/content/docs/tui.md, against the code:
//! its keys table, its client file table, its sizes and limits, each
//! message it quotes and each screen it shows. A page that says what the
//! code does not do, or leaves out a message the code can show, fails here.

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use time::OffsetDateTime;
use time::macros::datetime;

use super::client;
use super::keys::BINDINGS;
use super::screens::Screen;
use super::state::{Action, ApiError, Event, KeyInfo, Page, Row, State, View};
use super::view::{self, Theme};
use crate::api::Problem;
use crate::mcp::stdio;
use crate::secret_file::{self, Refusal};

const NOW: OffsetDateTime = datetime!(2026-11-10 01:20:00 UTC);

/// A file of the repository, by its path from the root.
fn read(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn page() -> String {
    read("website/content/docs/tui.md")
}

/// The number signs a heading line starts with; 0 for any other line.
fn level(line: &str) -> usize {
    let signs = line.chars().take_while(|c| *c == '#').count();
    if signs > 0 && line.chars().nth(signs) == Some(' ') {
        signs
    } else {
        0
    }
}

/// The text under the heading `heading` (its line, with its number signs),
/// up to the next heading of its level or a higher one. Lines inside a code
/// block are never headings.
fn section(text: &str, heading: &str) -> String {
    let wanted = level(heading);
    let mut out = Vec::new();
    let mut inside = false;
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let here = if fenced { 0 } else { level(line) };
        if inside && here > 0 && here <= wanted {
            break;
        }
        if inside {
            out.push(line);
        }
        if !fenced && line == heading {
            inside = true;
        }
    }
    assert!(inside, "the page has the heading {heading:?}");
    out.join("\n")
}

/// The cells of a table row.
fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

/// The rows of every table in `text` whose header starts with the cell
/// `first`: each row's cells, without the header and the rule under it.
fn table(text: &str, first: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            inside = false;
            continue;
        }
        let row = cells(line);
        if row.first().map(String::as_str) == Some(first) {
            inside = true;
        } else if inside && !row.iter().all(|cell| cell.chars().all(|c| c == '-')) {
            rows.push(row);
        }
    }
    rows
}

/// The text of the first code span in `cell`.
fn code_span(cell: &str) -> Option<&str> {
    let rest = cell.split_once('`')?.1;
    Some(rest.split_once('`')?.0)
}

/// The parts of a quoted message between its placeholders: `<reason>`,
/// `<path>` and the like, a lower-case word or words between angle
/// brackets, stand for any text.
fn fragments(quoted: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = quoted;
    let mut part = String::new();
    while let Some((before, after)) = rest.split_once('<') {
        part.push_str(before);
        match after.split_once('>') {
            Some((name, tail))
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == ' ') =>
            {
                out.push(std::mem::take(&mut part));
                rest = tail;
            }
            _ => {
                part.push('<');
                rest = after;
            }
        }
    }
    part.push_str(rest);
    out.push(part);
    out
}

/// Whether `said` holds the quoted message: each of its fragments, in
/// order.
fn holds(said: &str, quoted: &str) -> bool {
    let mut rest = said;
    for fragment in fragments(quoted) {
        match rest.split_once(fragment.as_str()) {
            Some((_, after)) => rest = after,
            None => return false,
        }
    }
    true
}

// The states the UI can be in, made by the state machine.

fn row(n: u16) -> Row {
    Row {
        uuid: format!("01a07da8-c2bb-83e5-b9a2-279e0d16{n:04}"),
        stored_at: "2026-10-25T12:00:00Z".to_owned(),
        updated_at: "2026-10-25T12:30:00Z".to_owned(),
        version: format!("4f0c4b0a1d2e3f405162738495a6{n:04}"),
    }
}

fn connecting() -> State {
    State::new("https://vcon.example.com:8443", (80, 24)).0
}

fn keyed(tenant: Option<&str>, scopes: &[&str]) -> State {
    let mut state = connecting();
    state.step(
        Event::KeyInfo(Ok(KeyInfo {
            id: "0123456789abcdef".to_owned(),
            tenant: tenant.map(str::to_owned),
            scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        })),
        NOW,
    );
    state
}

fn reader() -> State {
    keyed(Some("8c41f2d9a07e4b5c9d13e6f0a2b4c7d8"), &["read"])
}

fn listed(rows: u16, more: bool) -> State {
    let mut state = reader();
    state.step(
        Event::Page(Ok(Page {
            vcons: (0..rows).map(row).collect(),
            next: more.then(|| "cursor-2".to_owned()),
        })),
        NOW,
    );
    state
}

fn press(mut state: State, actions: &[Action]) -> State {
    for action in actions {
        state.step(Event::Action(*action), NOW);
    }
    state
}

/// The state at start when the server's answer for the key is `error`.
fn refused(error: ApiError) -> State {
    let mut state = connecting();
    state.step(Event::KeyInfo(Err(error)), NOW);
    state
}

/// The lines of the screen `state` is drawn as, without color.
fn frame(state: &State, columns: u16, lines: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(columns, lines)).expect("a terminal");
    terminal
        .draw(|frame| view::draw(frame, state, Theme { color: false }))
        .expect("a frame");
    let buffer = terminal.backend().buffer();
    (0..lines)
        .map(|y| (0..columns).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

/// What the screen says, as a reader reads it: its lines joined, so a
/// sentence wrapped over two lines is one sentence.
fn said(state: &State) -> String {
    frame(state, 80, 24)
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A certificate refusal rustls reports for a server certificate that is
/// marked as a certificate authority's.
#[derive(Debug)]
struct CaUsedAsEndEntity;

impl std::fmt::Display for CaUsedAsEndEntity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a certificate authority's certificate")
    }
}

impl std::error::Error for CaUsedAsEndEntity {}

/// Every refusal of a server certificate the UI has words for.
fn certificate_refusals() -> Vec<String> {
    use rustls::CertificateError as Refused;
    [
        Refused::Other(rustls::OtherError(Arc::new(CaUsedAsEndEntity))),
        Refused::UnknownIssuer,
        Refused::NotValidForName,
        Refused::Expired,
        Refused::NotValidYet,
    ]
    .iter()
    .map(|error| {
        let advice = client::certificate_advice(error).expect("advice for this refusal");
        said(&refused(ApiError::Untrusted(advice.to_owned())))
    })
    .collect()
}

/// What the command writes to standard error, after `vcon-server: `, when
/// it cannot start: one entry for each way.
fn start_failures() -> Vec<String> {
    let dir = tempfile::tempdir().expect("temp dir");
    let at = |name: &str| dir.path().join(name);
    let file = |name: &str, bytes: &[u8], mode: u32| {
        let path = at(name);
        std::fs::write(&path, bytes).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod");
        path
    };
    let client_file = |text: &str| {
        let refused = stdio::parse_client(text).expect_err("a client file that is refused");
        format!("client file: {refused}").replace('\n', " ")
    };
    let token = |path: &Path| {
        stdio::token(path)
            .expect_err("a key file that is refused")
            .to_string()
    };
    let ca = |path: &Path| {
        stdio::https_client(Some(path))
            .expect_err("a CA file that is refused")
            .to_string()
    };
    let source = |text: &'static str, holding: &str| {
        text.lines()
            .find(|line| line.contains(holding))
            .unwrap_or_else(|| panic!("the source holds {holding:?}"))
            .trim()
            .to_owned()
    };
    let main = include_str!("../main.rs");
    vec![
        format!(
            "client file: {}",
            stdio::client_config(&at("missing.toml")).expect_err("no such file")
        ),
        client_file("url = \"http://vcon.example.com:8443\"\nkey_file = \"/k\"\n"),
        client_file("url = \"https://vcon.example.com:8443\"\nkey_file = \"/k\"\ntoken = \"t\"\n"),
        client_file("key_file = \"/k\"\n"),
        token(&at("missing.token")),
        token(&file("open.token", b"vcs_0011223344556677_secret\n", 0o644)),
        token(&file("empty.token", b"\n", 0o600)),
        token(&file("two.token", b"vcs_a vcs_b\n", 0o600)),
        token(&file("binary.token", &[0xff, 0xfe], 0o600)),
        token(dir.path()),
        secret_file::describe(
            "key_file",
            &at("other.token"),
            Refusal::Owner {
                uid: 1001,
                process: 1000,
            },
        ),
        ca(&at("missing.pem")),
        ca(&file("empty.pem", b"", 0o644)),
        super::require_terminal(false, true).expect_err("no input"),
        super::require_terminal(true, false).expect_err("no output"),
        super::require_terminal(false, false).expect_err("neither"),
        source(main, "memory cannot be locked: "),
        source(main, "core dumps cannot be turned off: "),
        source(main, "this build has no terminal UI: "),
        source(
            include_str!("../tui.rs"),
            "map_err(|e| format!(\"the terminal: ",
        ),
    ]
}

/// What the UI makes of the server's answer when the server refuses a
/// request with `problem`: the problem document the API sends, read by the
/// UI's own conversion.
fn answered(problem: Problem) -> ApiError {
    let body = serde_json::to_vec(&problem.document()).expect("JSON");
    client::answer::<KeyInfo>(problem.status().as_u16(), &body).expect_err("a refusal is not a key")
}

#[test]
fn a_refusal_is_shown_with_the_title_of_the_servers_problem_document() {
    assert_eq!(
        answered(Problem::Unavailable).to_string(),
        "the server answered 503 (The service cannot answer now)",
        "the status and the problem's title, not the name of its type"
    );
    assert_eq!(
        answered(Problem::Unauthorized).to_string(),
        "the server refused the key (401)",
        "a 401 has words of its own"
    );
}

#[test]
fn an_answer_with_no_problem_document_is_shown_by_its_status_alone() {
    let refused = client::answer::<KeyInfo>(404, b"").expect_err("not a key");
    assert_eq!(
        refused.to_string(),
        "the server answered 404",
        "what a url with a path gives: the server has no such route"
    );
    assert!(
        quoted_messages(&page()).contains(&refused.to_string()),
        "and the page has a row for it"
    );
}

/// What the screen says in each state in which something is wrong.
fn screen_failures() -> Vec<String> {
    let mut next_page_failed = press(listed(3, true), &[Action::More]);
    next_page_failed.step(Event::Page(Err(client::no_answer(client::TIMEOUT))), NOW);
    let mut gone = press(listed(3, false), &[Action::Open, Action::Refresh]);
    gone.step(
        Event::Page(Ok(Page {
            vcons: vec![row(7)],
            next: None,
        })),
        NOW,
    );
    let mut out = vec![
        said(&refused(client::no_answer(client::TIMEOUT))),
        said(&refused(ApiError::Unreachable(
            "error sending request for url (https://vcon.example.com:8443/key): client error (Connect): tcp connect error: Connection refused (os error 111)".to_owned(),
        ))),
        said(&refused(ApiError::Status(401, None))),
        said(&refused(ApiError::Status(404, None))),
        said(&refused(answered(Problem::Unavailable))),
        said(&refused(answered(Problem::AuditFailed))),
        said(&refused(ApiError::Malformed("expected value at line 1 column 1".to_owned()))),
        said(&keyed(None, &[])),
        said(&keyed(Some("8c41f2d9a07e4b5c9d13e6f0a2b4c7d8"), &["write"])),
        said(&next_page_failed),
        said(&gone),
        frame(&listed(3, false), 39, 8)
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    ];
    out.extend(certificate_refusals());
    out
}

/// What the screen says in each state in which nothing is wrong.
fn screen_states() -> Vec<String> {
    vec![
        said(&connecting()),
        said(&reader()),
        said(&listed(0, false)),
        said(&listed(3, false)),
        said(&listed(3, true)),
        said(&press(listed(3, true), &[Action::More])),
        said(&press(listed(3, false), &[Action::Open])),
        said(&press(listed(3, false), &[Action::Open, Action::Refresh])),
        said(&press(listed(3, false), &[Action::Help])),
    ]
}

/// The message each row of the page's troubleshooting tables quotes.
fn quoted_messages(page: &str) -> Vec<String> {
    table(&section(page, "## When it does not work"), "Message")
        .iter()
        .map(|row| {
            code_span(row.first().expect("a first cell"))
                .unwrap_or_else(|| panic!("the row {row:?} quotes its message as code"))
                .to_owned()
        })
        .collect()
}

// Messages.

#[test]
fn every_message_the_page_quotes_is_one_the_code_writes() {
    let written: Vec<String> = start_failures()
        .into_iter()
        .chain(screen_failures())
        .chain(screen_states())
        .collect();
    let quoted = quoted_messages(&page());
    assert!(
        quoted.len() >= 20,
        "the tables quote the messages: {}",
        quoted.len()
    );
    for message in quoted {
        assert!(
            written.iter().any(|said| holds(said, &message)),
            "the page quotes {message:?}, which the code does not write"
        );
    }
}

#[test]
fn every_start_failure_has_a_row_on_the_page() {
    let quoted = quoted_messages(&page());
    for failure in start_failures() {
        assert!(
            quoted.iter().any(|message| holds(&failure, message)),
            "no row of the page is for the start failure {failure:?}"
        );
    }
}

#[test]
fn every_failure_the_screen_shows_has_a_row_on_the_page() {
    let quoted = quoted_messages(&page());
    for failure in screen_failures() {
        assert!(
            quoted.iter().any(|message| holds(&failure, message)),
            "no row of the page is for the screen that says {failure:?}"
        );
    }
}

#[test]
fn each_certificate_refusal_has_a_row_of_its_own() {
    let quoted = quoted_messages(&page());
    let mut rows = BTreeSet::new();
    for refusal in certificate_refusals() {
        let row = quoted
            .iter()
            .find(|message| {
                message.starts_with("the server's certificate was refused: ")
                    && holds(&refusal, message)
            })
            .unwrap_or_else(|| panic!("no certificate row for {refusal:?}"));
        rows.insert(row.clone());
    }
    assert_eq!(rows.len(), 5, "five refusals, five rows: {rows:?}");
}

#[test]
fn every_row_says_the_cause_and_what_to_do() {
    for row in table(&section(&page(), "## When it does not work"), "Message") {
        assert_eq!(row.len(), 3, "message, cause, what to do: {row:?}");
        assert!(
            row.iter().all(|cell| !cell.is_empty()),
            "no empty cell: {row:?}"
        );
    }
}

#[test]
fn the_troubleshooting_is_grouped_by_where_the_message_appears() {
    let section = section(&page(), "## When it does not work");
    let groups: Vec<&str> = section.lines().filter(|line| level(line) == 3).collect();
    assert!(
        groups.len() >= 4,
        "the messages are in groups, each under a heading: {groups:?}"
    );
    let mut grouped = 0_usize;
    for group in groups {
        let rows = table(&self::section(&page(), group), "Message");
        assert!(
            !rows.is_empty() && rows.len() <= 8,
            "{group} holds a short table: {} rows",
            rows.len()
        );
        grouped = grouped.saturating_add(rows.len());
    }
    assert_eq!(
        grouped,
        table(&section, "Message").len(),
        "every row is in a group: no table stands before the first heading"
    );
}

#[test]
fn the_page_shows_the_two_lines_that_end_every_failure_on_the_screen() {
    let lines = frame(&refused(client::no_answer(client::TIMEOUT)), 80, 24);
    // The status line is first and the key hints are last; what the
    // failure says is between them.
    let said: Vec<&str> = lines
        .iter()
        .skip(1)
        .take(lines.len().saturating_sub(2))
        .map(|line| line.trim_end())
        .filter(|line| !line.is_empty())
        .collect();
    let last_two = said
        .rchunks(2)
        .next()
        .expect("a failure has at least two lines")
        .join("\n");
    assert!(
        last_two.starts_with("Press r to try again"),
        "what r and q do, then what r cannot do: {last_two}"
    );
    assert!(
        section(&page(), "## When it does not work").contains(&last_two),
        "the page shows them as the screen has them: {last_two}"
    );
}

#[test]
fn the_page_quotes_what_the_screen_says_in_each_state_that_is_not_a_failure() {
    let page = page();
    for words in [
        "Connecting to the server...",
        "Loading vCons...",
        "Reading the list again...",
        "Reading the next page...",
        "This tenant has no vCons.",
        "Content is not shown: this view lists metadata only.",
        "more on the server",
    ] {
        assert!(
            screen_states().iter().any(|said| said.contains(words)),
            "the code writes {words:?}"
        );
        assert!(page.contains(words), "the page quotes {words:?}");
    }
}

// Keys.

/// A key as the page writes it.
fn key_name(code: KeyCode, modifiers: KeyModifiers) -> String {
    let name = match code {
        KeyCode::Char(c) if modifiers == KeyModifiers::CONTROL => {
            return format!("Ctrl-{}", c.to_ascii_uppercase());
        }
        KeyCode::Char(c) => return format!("`{c}`"),
        KeyCode::Down => "Down",
        KeyCode::Up => "Up",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PgUp",
        KeyCode::PageDown => "PgDn",
        KeyCode::Enter => "Enter",
        KeyCode::Esc => "Esc",
        KeyCode::Backspace => "Backspace",
        other => panic!("the page has no name for {other:?}"),
    };
    name.to_owned()
}

/// What a binding does, as the help view says it, without the other keys
/// the help view names in parentheses: the page's first column has them.
fn does(text: &str) -> &str {
    text.split_once(" (also ")
        .map_or(text, |(before, _)| before)
}

/// Whether `action` does anything in `view`: the state changes, or a
/// request is made. The list holds 30 vCons with more on the server, and
/// the eleventh is selected, so every movement has somewhere to go.
fn works(view: View, action: Action) -> bool {
    let list = press(listed(30, true), &[Action::Down; 10]);
    let state = match view {
        View::List => list,
        View::Detail => press(list, &[Action::Open]),
        View::Help => press(list, &[Action::Help]),
    };
    let mut after = state.clone();
    let asked = after.step(Event::Action(action), NOW);
    !asked.is_empty() || after != state
}

fn keys_table() -> Vec<Vec<String>> {
    table(&section(&page(), "### Keys"), "Key")
}

#[test]
fn the_keys_table_has_a_row_for_every_binding_with_every_key() {
    let rows = keys_table();
    assert_eq!(rows.len(), BINDINGS.len(), "one row for each binding");
    for (binding, row) in BINDINGS.iter().zip(&rows) {
        let keys: Vec<String> = binding
            .keys
            .iter()
            .map(|(code, modifiers)| key_name(*code, *modifiers))
            .collect();
        assert_eq!(
            row.first().map(String::as_str),
            Some(keys.join(", ").as_str()),
            "the keys of {:?}, in the order the help view lists the bindings",
            binding.action
        );
    }
}

#[test]
fn the_keys_table_says_what_the_help_view_says_each_key_does() {
    for (binding, row) in BINDINGS.iter().zip(&keys_table()) {
        assert_eq!(
            row.get(1).map(String::as_str),
            Some(does(binding.does)),
            "what {:?} does",
            binding.action
        );
    }
}

#[test]
fn the_keys_table_says_in_which_views_each_key_does_something() {
    for (binding, row) in BINDINGS.iter().zip(&keys_table()) {
        for (column, view) in [(2, View::List), (3, View::Detail), (4, View::Help)] {
            let wanted = if works(view, binding.action) {
                "yes"
            } else {
                "no"
            };
            assert_eq!(
                row.get(column).map(String::as_str),
                Some(wanted),
                "{:?} in the {view:?} view",
                binding.action
            );
        }
    }
}

#[test]
fn the_views_reader_knows_a_key_that_does_nothing_from_one_that_does() {
    assert!(works(View::List, Action::Open), "Enter opens a vCon");
    assert!(
        !works(View::Detail, Action::Open),
        "Enter does nothing in the detail view"
    );
    assert!(
        !works(View::List, Action::Back),
        "Esc does nothing in the list"
    );
    assert!(
        !works(View::Help, Action::Refresh),
        "help takes only the keys that close it and quit"
    );
    assert!(
        works(View::Detail, Action::More),
        "n reads on from the detail view"
    );
}

#[test]
fn the_page_says_that_a_key_not_in_the_table_does_nothing() {
    let page = page().replace('\n', " ");
    assert!(
        super::translate(&ratatui::crossterm::event::Event::Key(
            ratatui::crossterm::event::KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)
        ))
        .is_none(),
        "x is not bound"
    );
    assert!(
        page.contains("Any other key does nothing"),
        "and the page says so"
    );
}

#[test]
fn the_page_says_that_the_mouse_does_nothing() {
    use ratatui::crossterm::event::{Event as Input, MouseButton, MouseEvent, MouseEventKind};
    let click = Input::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 3,
        row: 3,
        modifiers: KeyModifiers::NONE,
    });
    assert!(super::translate(&click).is_none(), "a click is no action");
    assert!(
        page().replace('\n', " ").contains("The mouse does nothing"),
        "and the page says so"
    );
}

// The client file.

fn client_table() -> Vec<Vec<String>> {
    table(&section(&page(), "### The client file"), "Key")
}

/// A client file with the example of each row of the page's table but
/// the row for `without`.
fn client_text(without: Option<&str>) -> String {
    client_table()
        .iter()
        .filter_map(|row| {
            let key = code_span(row.first()?)?;
            let example = code_span(row.get(4)?)?;
            (Some(key) != without).then(|| format!("{key} = {example}\n"))
        })
        .collect()
}

#[test]
fn the_client_file_table_names_the_keys_the_file_takes() {
    let keys: Vec<String> = client_table()
        .iter()
        .map(|row| {
            code_span(row.first().expect("a key"))
                .expect("as code")
                .to_owned()
        })
        .collect();
    assert_eq!(keys, ["url", "key_file", "ca"], "the three keys, in order");
}

#[test]
fn the_examples_in_the_client_file_table_make_a_file_the_command_takes() {
    let config = stdio::parse_client(&client_text(None)).expect("the examples parse");
    assert_eq!(
        config.url, "https://vcon.example.com:8443",
        "the address of the pages' server"
    );
    assert!(config.key_file.is_absolute(), "a full path: {config:?}");
    assert!(config.ca.is_some(), "the CA is in the table: {config:?}");
}

#[test]
fn the_client_file_table_says_which_keys_are_required() {
    for row in client_table() {
        let key = code_span(row.first().expect("a key")).expect("as code");
        let outcome = stdio::parse_client(&client_text(Some(key)));
        let required = match row.get(1).map(String::as_str) {
            Some("yes") => true,
            Some("no") => false,
            other => panic!("required is yes or no, not {other:?}"),
        };
        assert_eq!(
            outcome.is_err(),
            required,
            "a file without {key}: {outcome:?}"
        );
    }
}

#[test]
fn a_key_the_table_does_not_name_is_refused() {
    let text = format!("{}proxy = \"http://192.0.2.1:3128\"\n", client_text(None));
    let refused = stdio::parse_client(&text).expect_err("an unknown key");
    assert!(
        refused.to_string().contains("unknown field `proxy`"),
        "the file takes the table's keys only: {refused}"
    );
}

#[test]
fn the_page_says_the_url_has_no_path_and_what_one_gives() {
    assert_eq!(
        format!("{}/key", client::base("https://vcon.example.com:8443/mcp")),
        "https://vcon.example.com:8443/mcp/key",
        "a path in url goes before every request's path"
    );
    let row = client_table()
        .into_iter()
        .find(|row| row.first().map(String::as_str) == Some("`url`"))
        .expect("the url row");
    assert!(
        row.get(3).is_some_and(|cell| cell.contains("no path")),
        "the meaning of url says so: {row:?}"
    );
}

#[test]
fn a_trailing_slash_in_url_is_not_a_path() {
    assert_eq!(
        client::base("https://vcon.example.com:8443/"),
        "https://vcon.example.com:8443",
        "so the page's example may be written with or without it"
    );
}

#[test]
fn key_file_is_read_as_written_so_the_page_asks_for_a_full_path() {
    let config = stdio::parse_client(
        "url = \"https://vcon.example.com:8443\"\nkey_file = \"~/tui.token\"\n",
    )
    .expect("it parses");
    assert_eq!(
        config.key_file,
        Path::new("~/tui.token"),
        "a tilde is not the home directory"
    );
    let row = client_table()
        .into_iter()
        .find(|row| row.first().map(String::as_str) == Some("`key_file`"))
        .expect("the key_file row");
    assert!(
        row.get(3)
            .is_some_and(|cell| cell.contains("full path") && cell.contains("`~`")),
        "the meaning of key_file says so: {row:?}"
    );
}

// Sizes, limits and the environment.

/// The value of the row of the sizes table whose first cell is `name`.
fn limit(name: &str) -> String {
    table(&section(&page(), "### Sizes and limits"), "What")
        .into_iter()
        .find(|row| row.first().map(String::as_str) == Some(name))
        .and_then(|row| row.get(1).cloned())
        .unwrap_or_else(|| panic!("the sizes table has the row {name:?}"))
}

/// The fewest columns at which the list's header holds `column`.
fn first_width_with(column: &str) -> u16 {
    let state = listed(3, false);
    (40..=200)
        .find(|columns| {
            frame(&state, *columns, 24)
                .get(1)
                .is_some_and(|header| header.contains(column))
        })
        .unwrap_or_else(|| panic!("no width shows {column}"))
}

#[test]
fn the_page_states_the_vcons_read_at_a_time() {
    assert_eq!(
        limit("vCons read at a time"),
        client::PAGE.to_string(),
        "the page size the UI asks for"
    );
    assert!(
        client::page_path(None).ends_with(&format!("limit={}", client::PAGE)),
        "and it is what the request carries"
    );
}

#[test]
fn the_page_states_the_time_allowed_for_one_request() {
    assert_eq!(
        limit("Time allowed for one request"),
        format!("{} seconds", client::TIMEOUT.as_secs()),
        "the limit after which the screen says no answer"
    );
}

#[test]
fn the_page_states_the_smallest_terminal() {
    let state = listed(3, false);
    let too_small = |columns, lines| {
        frame(&state, columns, lines)
            .join(" ")
            .contains("Terminal too small")
    };
    assert!(!too_small(40, 8), "40 by 8 is drawn");
    assert!(too_small(39, 8), "one column fewer is not");
    assert!(too_small(40, 7), "one line fewer is not");
    assert_eq!(
        limit("Smallest terminal"),
        "40 columns by 8 lines",
        "as the page says"
    );
}

#[test]
fn the_page_states_the_width_from_which_each_column_is_shown() {
    for (row, column) in [
        ("Stored time shown from", "STORED"),
        ("Updated time shown from", "UPDATED"),
        ("Version shown from", "VERSION"),
    ] {
        assert_eq!(
            limit(row),
            format!("{} columns", first_width_with(column)),
            "the first width whose header holds {column}"
        );
    }
}

#[test]
fn the_uuid_is_shown_whole_at_the_smallest_width() {
    let lines = frame(&listed(3, false), 40, 8);
    assert!(
        lines.iter().any(|line| line.contains(&row(0).uuid)),
        "a uuid is never cut: {lines:?}"
    );
}

#[test]
fn the_environment_table_says_when_color_is_used() {
    let rows = table(&section(&page(), "### Environment"), "Variable");
    let names: Vec<&str> = rows
        .iter()
        .filter_map(|row| code_span(row.first()?))
        .collect();
    assert_eq!(
        names,
        ["NO_COLOR", "TERM"],
        "the two variables the UI reads"
    );
    assert!(
        view::theme(None, Some("xterm-256color")).color,
        "color by default"
    );
    assert!(
        !view::theme(Some("1"), Some("xterm-256color")).color,
        "NO_COLOR turns it off"
    );
    assert!(
        !view::theme(None, Some("dumb")).color,
        "TERM=dumb turns it off"
    );
    assert!(!view::theme(None, None).color, "no TERM turns it off");
}

#[test]
fn the_page_says_that_an_empty_no_color_does_not_count() {
    assert!(
        view::theme(Some(""), Some("xterm-256color")).color,
        "an empty NO_COLOR is not a request"
    );
    let rows = table(&section(&page(), "### Environment"), "Variable");
    let row = rows.first().expect("the NO_COLOR row");
    assert!(
        row.get(1).is_some_and(|cell| cell.contains("empty")),
        "and the row says so: {row:?}"
    );
}

#[test]
fn the_page_says_that_no_proxy_setting_is_used() {
    assert!(
        include_str!("../mcp/stdio.rs").contains("reqwest::Client::builder().no_proxy()"),
        "the client connects directly"
    );
    assert!(
        section(&page(), "### Environment").contains("`HTTPS_PROXY`"),
        "and the page names the variable a reader would try"
    );
}

#[test]
fn the_exit_codes_on_the_page_are_the_two_the_command_has() {
    let rows = table(&section(&page(), "### The command"), "Exit code");
    let codes: Vec<&str> = rows
        .iter()
        .filter_map(|row| row.first().map(String::as_str))
        .collect();
    assert_eq!(codes, ["0", "1"], "ExitCode::SUCCESS and ExitCode::FAILURE");
}

// Screens.

/// The screens in website/data/tui-screens.json: each name, with its
/// columns and its lines as text.
fn screens() -> Vec<(String, u64, Vec<String>)> {
    let data: serde_json::Value =
        serde_json::from_str(&read("website/data/tui-screens.json")).expect("JSON");
    data.as_object()
        .expect("an object")
        .iter()
        .map(|(name, screen)| {
            let lines = screen["lines"]
                .as_array()
                .expect("lines")
                .iter()
                .map(|line| {
                    line.as_array()
                        .expect("runs")
                        .iter()
                        .map(|run| run["text"].as_str().expect("text"))
                        .collect()
                })
                .collect();
            (
                name.clone(),
                screen["columns"].as_u64().expect("columns"),
                lines,
            )
        })
        .collect()
}

/// The screens `text` shows, with each one's label.
fn shown(text: &str) -> Vec<(String, String)> {
    text.split("<tui_screen name=\"")
        .skip(1)
        .map(|rest| {
            let (name, rest) = rest.split_once('"').expect("the name ends");
            let label = rest
                .split_once("label=\"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map_or("", |(label, _)| label);
            (name.to_owned(), label.to_owned())
        })
        .collect()
}

#[test]
fn every_screen_the_page_shows_is_one_the_ui_draws() {
    let drawn: BTreeSet<String> = screens().into_iter().map(|(name, _, _)| name).collect();
    for (name, _) in shown(&page()) {
        assert!(
            drawn.contains(&name),
            "the page shows the screen {name:?}, which tui-screens.json does not hold"
        );
    }
}

#[test]
fn every_screen_the_ui_draws_for_the_site_is_shown_on_the_page() {
    let shown: BTreeSet<String> = shown(&page()).into_iter().map(|(name, _)| name).collect();
    for (name, _, _) in screens() {
        assert!(
            shown.contains(&name),
            "tui-screens.json holds the screen {name:?}, which the page does not show"
        );
    }
}

#[test]
fn the_page_shows_each_view_and_each_state() {
    let shown: BTreeSet<String> = shown(&page()).into_iter().map(|(name, _)| name).collect();
    for wanted in [
        "list",
        "detail",
        "help",
        "empty",
        "not-connected",
        "narrow",
        "wide",
        "next-page-failed",
    ] {
        assert!(shown.contains(wanted), "the page shows {wanted}");
    }
}

#[test]
fn each_screen_is_shown_once_with_a_label_of_its_own() {
    // The page opens with the list, which its own section shows again:
    // no other screen is shown twice.
    let shown = shown(&screens_section());
    assert_eq!(
        self::shown(&page()).len(),
        shown.len().saturating_add(1),
        "the screens of the sections, and the one the page opens with"
    );
    let opening = self::shown(&page()).remove(0);
    assert_eq!(opening.0, "list", "the page opens with the list");
    assert!(
        shown.iter().all(|(_, label)| *label != opening.1),
        "and says it in other words than the list's section"
    );
    let names: BTreeSet<&str> = shown.iter().map(|(name, _)| name.as_str()).collect();
    let labels: BTreeSet<&str> = shown.iter().map(|(_, label)| label.as_str()).collect();
    assert_eq!(names.len(), shown.len(), "no screen twice");
    assert_eq!(labels.len(), shown.len(), "no label twice");
    for (name, label) in &shown {
        assert!(
            label.len() >= 40,
            "the label of {name} says what the screen shows: {label:?}"
        );
    }
}

#[test]
fn no_line_of_a_screen_is_wider_than_its_terminal() {
    for (name, columns, lines) in screens() {
        for line in lines {
            assert!(
                u64::try_from(line.chars().count()).expect("a count") <= columns,
                "{name}: {line:?} fits {columns} columns"
            );
        }
    }
}

#[test]
fn the_screens_have_no_tenant_or_address_that_is_not_an_example() {
    for (name, _, lines) in screens() {
        let first = lines.first().expect("a first line");
        assert!(
            name == "too-small" || first.contains("vcon.example.com:8443"),
            "{name}: the server is the pages' example: {first:?}"
        );
    }
}

#[test]
fn the_home_page_shows_a_screen_the_ui_draws() {
    let home = read("website/templates/index.html");
    let drawn: BTreeSet<String> = screens().into_iter().map(|(name, _, _)| name).collect();
    let names: Vec<String> = shown(&home).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["list"], "the home page shows the list");
    assert!(drawn.contains("list"), "which the UI draws");
}

// The page as a guide.

#[test]
fn the_page_opens_with_what_the_reader_gets() {
    let page = page();
    let body = page.splitn(3, "+++").nth(2).expect("a body").trim_start();
    let first = body.split("\n\n").next().expect("a first paragraph");
    assert!(
        first.starts_with("`vcon-server tui` shows"),
        "what it shows, first: {first:?}"
    );
    for word in ["data directory", "master key", "catalog"] {
        assert!(
            !first.contains(word),
            "what it does not open comes later: {first:?}"
        );
    }
    let screen = body.find("<tui_screen name=\"list\"").expect("the list");
    let heading = body.find("\n## ").expect("a first heading");
    assert!(screen < heading, "a real screen before the first heading");
}

#[test]
fn the_page_does_not_tell_the_reader_about_the_projects_tests() {
    let page = page().replace('\n', " ");
    for words in ["CI runs", "a test fails", "test fails when", "is tested"] {
        assert!(
            !page.contains(words),
            "the reader did not ask how the page is tested: {words:?}"
        );
    }
}

#[test]
fn the_page_says_who_should_use_another_tool_before_the_steps() {
    let page = page();
    let choose = page
        .find("\n## Is this the right tool")
        .expect("the section");
    let start = page.find("\n## Before you start").expect("the section");
    assert!(choose < start, "the choice comes before the set-up");
    let rows = table(&section(&page, "## Is this the right tool"), "To");
    assert!(rows.len() >= 6, "a row for each thing a reader may want");
    for row in rows {
        assert_eq!(row.len(), 4, "the task and three tools: {row:?}");
    }
}

#[test]
fn what_the_ui_does_not_do_is_a_no_in_the_first_table() {
    let rows = table(&section(&page(), "## Is this the right tool"), "To");
    let ui: Vec<&str> = rows
        .iter()
        .filter_map(|row| row.get(1).map(String::as_str))
        .collect();
    assert!(
        ui.iter().filter(|cell| **cell == "no").count() >= 4,
        "content, search, writes and the other views: {ui:?}"
    );
    assert!(
        ui.iter().all(|cell| *cell == "yes" || *cell == "no"),
        "yes or no, nothing between: {ui:?}"
    );
}

#[test]
fn before_you_start_is_a_table_that_says_where_each_thing_is_needed() {
    let rows = table(&section(&page(), "## Before you start"), "Have");
    assert!(rows.len() >= 5, "each thing to have ready: {}", rows.len());
    for row in rows {
        let place = row.get(1).map(String::as_str);
        assert!(
            matches!(
                place,
                Some("the server" | "the terminal's machine" | "both")
            ),
            "where it is needed: {row:?}"
        );
    }
}

#[test]
fn the_two_machine_set_up_is_a_procedure_of_its_own() {
    let page = page();
    let section = section(&page, "## Set it up for a terminal on another machine");
    let steps = section
        .lines()
        .filter(|line| {
            line.split_once(". ")
                .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
        .count();
    assert!(steps >= 4, "numbered steps: {steps}");
    assert!(
        section.contains("On the server") && section.contains("On the terminal's machine"),
        "each step says where it is done"
    );
}

#[test]
fn there_is_a_section_for_each_thing_a_person_wants_to_do() {
    let section = section(&page(), "## Use it");
    for task in [
        "### Read the screen",
        "### Find a vCon",
        "### See when a vCon was stored",
        "### See whether more vCons exist",
        "### Read the next page",
        "### Read the list again",
        "### Use a vCon's uuid elsewhere",
        "### Get help",
        "### Leave",
    ] {
        assert!(
            section.lines().any(|line| line == task),
            "the page has {task:?}"
        );
    }
}

#[test]
fn no_command_on_the_page_is_skipped_and_none_needs_a_here_document() {
    let page = page();
    assert!(
        !page.contains("docs-test: skip"),
        "every command on the page is run"
    );
    assert!(!page.contains("<<"), "no command reads a here-document");
}

#[test]
fn the_token_is_not_kept_in_a_shell_variable() {
    let page = page();
    for line in page.lines().filter(|line| line.contains("key create")) {
        assert!(
            !line.contains("=$("),
            "the answer that holds the token goes to the file, not to a variable: {line}"
        );
    }
}

#[test]
fn the_page_says_how_to_take_the_access_away() {
    let section = section(&page(), "## Remove access");
    assert!(
        section.contains("key revoke"),
        "the key is revoked on the server"
    );
    assert!(
        section.contains("tui.token"),
        "and the token file is removed"
    );
}

#[test]
fn the_security_section_names_the_four_things_a_reader_asks() {
    let section = section(&page(), "## Security");
    let rows = table(&section, "Question");
    assert!(rows.len() >= 4, "token, certificate, content, records");
    for words in ["token", "certificate", "content", "audit"] {
        assert!(
            rows.iter()
                .any(|row| row.first().is_some_and(|cell| cell.contains(words))),
            "a row about the {words}"
        );
    }
}

// The other pages that name the terminal UI.

#[test]
fn the_command_line_page_names_the_client_file_keys_of_the_table() {
    let section = section(
        &read("website/content/docs/command-line.md"),
        "## The terminal UI",
    );
    for row in client_table() {
        let key = row.first().expect("a key");
        assert!(section.contains(key.as_str()), "it names {key}");
    }
    assert!(
        section.contains("@/docs/tui.md"),
        "and links to the terminal UI's page"
    );
}

#[test]
fn the_command_line_page_and_this_page_give_the_same_exit_codes() {
    let section = section(
        &read("website/content/docs/command-line.md"),
        "## The terminal UI",
    )
    .replace('\n', " ");
    assert!(
        section.contains("code 0 when the user quits") && section.contains("code 1 when"),
        "0 on quit and 1 on a failure to start"
    );
}

#[test]
fn every_page_that_names_the_command_links_to_its_page() {
    for (path, link) in [
        ("README.md", "https://vcon-server.com/docs/tui/"),
        ("website/content/docs/status.md", "@/docs/tui.md"),
        ("website/content/docs/how-to.md", "@/docs/tui.md"),
        ("website/content/docs/command-line.md", "@/docs/tui.md"),
        ("website/content/docs/lets-encrypt.md", "@/docs/tui.md"),
        ("website/content/docs/upgrading.md", "@/docs/tui.md"),
        ("website/templates/index.html", "@/docs/tui.md"),
        (
            "website/static/llms.txt",
            "https://vcon-server.com/docs/tui/",
        ),
    ] {
        let text = read(path);
        assert!(text.contains("tui"), "{path} names the terminal UI");
        assert!(text.contains(link), "{path} links to its page");
    }
}

#[test]
fn no_page_says_the_ui_shows_content() {
    for path in [
        "README.md",
        "website/content/_index.md",
        "website/content/docs/status.md",
        "website/content/docs/command-line.md",
    ] {
        let text = read(path).replace('\n', " ");
        assert!(
            text.contains("never its content")
                || text.contains("never a vCon's content")
                || text.contains("never asks for a vCon's content")
                || text.contains("never their content"),
            "{path} says the terminal UI never shows a vCon's content"
        );
    }
}

#[test]
fn the_indexes_describe_the_page_as_its_front_matter_does() {
    let page = page();
    let description = page
        .lines()
        .find_map(|line| line.strip_prefix("description = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("a description");
    assert!(
        !description.contains("build"),
        "the build is not the reader's first question: {description}"
    );
    assert!(
        read("docs/README.md").contains(description),
        "docs/README.md has it word for word"
    );
    assert!(
        read("website/static/llms.txt").contains(description),
        "llms.txt has it word for word"
    );
}

// The readers above.

#[test]
fn the_section_reader_stops_at_the_next_heading_of_its_level() {
    let text = "# a\n## b\none\n### c\ntwo\n## d\nthree\n";
    assert_eq!(
        section(text, "## b"),
        "one\n### c\ntwo",
        "the subsection stays"
    );
    assert_eq!(section(text, "### c"), "two", "a higher heading ends it");
}

#[test]
fn the_section_reader_does_not_take_a_comment_in_a_code_block_for_a_heading() {
    let text = "## a\n```sh\n# not a heading\nls\n```\nafter\n## b\n";
    assert_eq!(
        section(text, "## a"),
        "```sh\n# not a heading\nls\n```\nafter",
        "the block is part of the section"
    );
}

#[test]
fn the_table_reader_takes_the_rows_under_the_header_it_is_given() {
    let text = "| Key | Does |\n|---|---|\n| `j` | down |\n\n| Other | x |\n|---|---|\n| a | b |\n";
    assert_eq!(
        table(text, "Key"),
        vec![vec!["`j`".to_owned(), "down".to_owned()]],
        "the other table is not read"
    );
}

#[test]
fn the_table_reader_reads_each_table_with_that_header() {
    let text = "| Message | Do |\n|---|---|\n| a | b |\n\ntext\n\n| Message | Do |\n|---|---|\n| c | d |\n";
    assert_eq!(table(text, "Message").len(), 2, "both tables' rows");
}

#[test]
fn a_placeholder_stands_for_any_text() {
    assert!(
        holds(
            "key_file /home/a/t: holds no single token",
            "key_file <path>: holds no single token"
        ),
        "the path differs from reader to reader"
    );
    assert!(
        !holds(
            "key_file /home/a/t: gone",
            "key_file <path>: holds no single token"
        ),
        "the words around it must be there"
    );
}

#[test]
fn the_fragments_of_a_message_are_found_in_order() {
    assert!(holds("a then b", "a <x> b"), "a before b");
    assert!(!holds("b then a", "a <x> b"), "not b before a");
}

#[test]
fn an_angle_bracket_that_is_not_a_placeholder_is_text() {
    assert_eq!(
        fragments("a < b and <Name> and <ok>"),
        ["a < b and <Name> and ", ""],
        "only a lower-case word between brackets is a placeholder"
    );
}

#[test]
fn a_message_without_a_placeholder_is_one_fragment() {
    assert_eq!(
        fragments("this key lacks the read scope"),
        ["this key lacks the read scope"],
        "the whole message"
    );
}

#[test]
fn the_code_span_reader_takes_the_first_span() {
    assert_eq!(code_span("`url` and `ca`"), Some("url"), "the first one");
    assert_eq!(code_span("no code"), None, "none");
}

#[test]
fn the_screen_reader_finds_names_and_labels() {
    let text = "{{ <tui_screen name=\"list\" label=\"The list.\" /> }} and {{ <tui_screen name=\"help\" label={x.l} /> }}";
    assert_eq!(
        shown(text),
        [
            ("list".to_owned(), "The list.".to_owned()),
            ("help".to_owned(), String::new())
        ],
        "a label that is not text is empty"
    );
}

// Every screen in the code's list (`Screen::ALL`) is on the page.

/// The section of the page that holds one section for each screen.
fn screens_section() -> String {
    section(&page(), "## Screens")
}

/// The section of `screen`.
fn screen_section(screen: Screen) -> String {
    section(&screens_section(), &format!("### {}", screen.title()))
}

/// The anchor the site gives a heading: lower case, with one hyphen for
/// each run of other characters.
fn anchor(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// The data file's entry for each screen name.
fn screen_data() -> serde_json::Map<String, serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(&read("website/data/tui-screens.json"))
        .expect("JSON")
        .as_object()
        .expect("an object")
        .clone()
}

/// What a press of `action` does to `state`, apart from clearing the
/// notice, which every key does: whether the state changed or a request
/// was made.
fn does_something(state: &State, action: Action) -> bool {
    let mut after = state.clone();
    let asked = after.step(Event::Action(action), NOW);
    after.notice.clone_from(&state.notice);
    !asked.is_empty() || after != *state
}

const MOVES: [Action; 6] = [
    Action::Up,
    Action::Down,
    Action::Top,
    Action::Bottom,
    Action::PageUp,
    Action::PageDown,
];

#[test]
fn the_code_lists_25_screens_with_a_name_and_a_title_each() {
    let names: BTreeSet<&str> = Screen::ALL.iter().map(|s| s.name()).collect();
    let titles: BTreeSet<&str> = Screen::ALL.iter().map(|s| s.title()).collect();
    let anchors: BTreeSet<String> = Screen::ALL.iter().map(|s| anchor(s.title())).collect();
    assert_eq!(Screen::ALL.len(), 25, "the screens the UI can draw");
    assert_eq!(names.len(), 25, "a name each");
    assert_eq!(titles.len(), 25, "a title each");
    assert_eq!(anchors.len(), 25, "and a link target each");
}

#[test]
fn the_list_of_screens_holds_every_view() {
    for view in [View::List, View::Detail, View::Help] {
        assert!(
            Screen::ALL.iter().any(|screen| screen.view() == view),
            "a screen of the {view:?} view"
        );
    }
}

#[test]
fn every_screen_in_the_code_has_a_section_on_the_page() {
    let headings: Vec<String> = screens_section()
        .lines()
        .filter(|line| level(line) == 3)
        .map(|line| line.trim_start_matches("### ").to_owned())
        .collect();
    let titles: Vec<&str> = Screen::ALL.iter().map(|s| s.title()).collect();
    assert_eq!(
        headings, titles,
        "one section for each screen of Screen::ALL, in its order, and no other"
    );
}

#[test]
fn every_screen_in_the_code_is_drawn_in_the_data_file_and_no_other_is() {
    let drawn: Vec<String> = screen_data().keys().cloned().collect();
    let names: BTreeSet<&str> = Screen::ALL.iter().map(|s| s.name()).collect();
    let names: Vec<&str> = names.into_iter().collect();
    assert_eq!(
        drawn, names,
        "tui-screens.json holds the screens of Screen::ALL"
    );
}

#[test]
fn each_screens_section_draws_that_screen_and_no_other() {
    for screen in Screen::ALL {
        let names: Vec<String> = shown(&screen_section(screen))
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            [screen.name()],
            "the section {:?} shows its own screen",
            screen.title()
        );
    }
}

#[test]
fn a_key_listed_for_a_screen_does_something_there_and_no_other_key_does() {
    for screen in Screen::ALL {
        let state = screen.state();
        let listed: Vec<Action> = screen.keys().iter().map(|b| b.action).collect();
        let can_move = MOVES.iter().any(|action| does_something(&state, *action));
        for binding in &BINDINGS {
            let action = binding.action;
            // A key that moves is listed where the selection can move,
            // also when it is at the end that key moves toward.
            let does = if MOVES.contains(&action) {
                can_move
            } else {
                does_something(&state, action)
            };
            assert_eq!(
                listed.contains(&action),
                does,
                "{}: {action:?} is listed exactly when a press does something",
                screen.name()
            );
        }
    }
}

#[test]
fn quit_and_help_are_listed_for_every_screen() {
    for screen in Screen::ALL {
        let labels: Vec<&str> = screen.keys().iter().map(|b| b.label).collect();
        assert!(labels.contains(&"q"), "{}: q quits", screen.name());
        assert!(labels.contains(&"?"), "{}: ? is help", screen.name());
    }
}

#[test]
fn the_waiting_screens_list_help_and_quit_only() {
    for screen in [Screen::Connecting, Screen::Loading] {
        let labels: Vec<&str> = screen.keys().iter().map(|b| b.label).collect();
        assert_eq!(labels, ["?", "q"], "{}", screen.name());
    }
}

#[test]
fn the_help_screen_lists_the_keys_that_close_it_and_quit() {
    let labels: Vec<&str> = Screen::Help.keys().iter().map(|b| b.label).collect();
    assert_eq!(labels, ["Esc", "?", "q"], "nothing else acts in help");
}

#[test]
fn a_key_that_may_not_list_has_no_refresh_and_other_failures_have() {
    for (screen, wanted) in [
        (Screen::OperatorKey, false),
        (Screen::NoReadScope, false),
        (Screen::NotConnected, true),
        (Screen::RefusedKey, true),
        (Screen::RefusedCertificate, true),
        (Screen::ServerError, true),
        (Screen::DetailFailed, true),
        (Screen::NextPageFailed, true),
    ] {
        let labels: Vec<&str> = screen.keys().iter().map(|b| b.label).collect();
        assert_eq!(labels.contains(&"r"), wanted, "{}: r", screen.name());
    }
}

#[test]
fn the_failure_screens_are_the_ones_that_say_something_failed() {
    let failures: Vec<&str> = Screen::ALL
        .iter()
        .filter(|screen| screen.failure())
        .map(|screen| screen.name())
        .collect();
    assert_eq!(
        failures,
        [
            "next-page-failed",
            "not-connected",
            "refused-key",
            "refused-certificate",
            "operator-key",
            "no-read-scope",
            "server-error",
            "detail-failed"
        ],
        "a list that could not be read, or a notice above the keys"
    );
}

#[test]
fn each_failure_screens_section_says_what_to_do() {
    for screen in Screen::ALL {
        let text = screen_section(screen);
        let needs = screen.failure() || matches!(screen, Screen::DetailGone | Screen::TooSmall);
        assert_eq!(
            text.contains("**What to do.**"),
            needs,
            "{:?}: a failure says what to do, and no other screen needs to",
            screen.title()
        );
    }
}

#[test]
fn each_screens_section_says_what_it_shows_and_how_to_get_there_and_leave() {
    for screen in Screen::ALL {
        let text = screen_section(screen);
        for part in ["**When.**", "**Shows.**", "**Leave.**"] {
            let said = text
                .split(part)
                .nth(1)
                .map_or("", |rest| rest.split("\n\n").next().unwrap_or(""));
            assert!(
                said.trim().len() >= 20,
                "{:?} has {part} with a sentence after it: {said:?}",
                screen.title()
            );
        }
    }
}

#[test]
fn each_failure_screens_section_quotes_what_the_screen_says() {
    for screen in Screen::ALL.into_iter().filter(|screen| screen.failure()) {
        let state = screen.state();
        let said = match (&state.listing, &state.notice) {
            (_, Some(notice)) => notice.clone(),
            (super::state::Listing::Failed(why), None) => why.clone(),
            other => panic!("{} is no failure: {other:?}", screen.name()),
        };
        let quoted = said.split(": ").next().expect("a first part");
        assert!(
            screen_section(screen).replace('\n', " ").contains(quoted),
            "{:?} quotes {quoted:?}",
            screen.title()
        );
    }
}

#[test]
fn the_table_at_the_top_links_every_screen_in_order() {
    let rows = table(&section(&page(), "## Every screen"), "Screen");
    assert_eq!(rows.len(), Screen::ALL.len(), "one row for each screen");
    for (row, screen) in rows.iter().zip(Screen::ALL) {
        assert_eq!(
            row.first().map(String::as_str),
            Some(format!("[{}](#{})", screen.title(), anchor(screen.title())).as_str()),
            "the screen, as a link to its section"
        );
        assert!(
            row.get(1).is_some_and(|when| when.len() >= 20),
            "{:?} says when it appears: {row:?}",
            screen.title()
        );
    }
}

#[test]
fn the_table_of_screens_stands_before_the_first_step() {
    let page = page();
    let table = page
        .find("\n## Every screen\n")
        .expect("the table's heading");
    let steps = page.find("\n## Quick start").expect("the first steps");
    assert!(table < steps, "a reader meets the list of screens first");
}

#[test]
fn the_layout_screens_are_drawn_at_their_widths() {
    let data = screen_data();
    for (screen, columns) in [
        (Screen::Wide, 120),
        (Screen::Narrow, 60),
        (Screen::Narrowest, 40),
        (Screen::TooSmall, 36),
        (Screen::List, 80),
    ] {
        assert_eq!(
            data[screen.name()]["columns"],
            columns,
            "{} is drawn in {columns} columns",
            screen.name()
        );
        assert!(
            screen_section(screen).contains(&columns.to_string()),
            "and its section names the width"
        );
    }
}

#[test]
fn the_anchor_of_a_title_is_as_the_site_makes_it() {
    assert_eq!(anchor("The list"), "the-list", "spaces");
    assert_eq!(
        anchor("A terminal of 120 columns"),
        "a-terminal-of-120-columns",
        "digits stay"
    );
    assert_eq!(anchor("Help"), "help", "one word");
}

#[test]
fn a_screen_added_to_the_code_without_a_section_would_be_found() {
    // The reader of sections fails on a heading the page lacks: this is
    // what refuses a screen that is in Screen::ALL and not on the page.
    let refused =
        std::panic::catch_unwind(|| section(&screens_section(), "### A screen nobody wrote about"));
    assert!(refused.is_err(), "a title with no section is a failure");
}

// The keys are stated once, in the Keys table of the reference. A screen's
// section says only what differs on that screen, as the code has it.

/// Whether `word` stands in `text` as a word of its own.
fn has_word(text: &str, word: &str) -> bool {
    let joins = |c: char| c.is_ascii_alphanumeric() || c == '-';
    text.match_indices(word).any(|(at, found)| {
        let before = text[..at].chars().next_back();
        let after = text[at + found.len()..].chars().next();
        !before.is_some_and(joins) && !after.is_some_and(joins)
    })
}

/// The bindings whose keys `text` names, each once, in the order of the
/// bindings table. A key is named as the Keys table writes it: a letter
/// or sign in a code span of its own, any other key as a word of its own.
fn bindings_named(text: &str) -> Vec<Action> {
    BINDINGS
        .iter()
        .filter(|binding| {
            binding.keys.iter().any(|(code, modifiers)| {
                let name = key_name(*code, *modifiers);
                if name.starts_with('`') {
                    text.contains(&name)
                } else {
                    has_word(text, &name)
                }
            })
        })
        .map(|binding| binding.action)
        .collect()
}

/// The pieces of a page that a reader takes in at once, each with the
/// number of its first line and the heading it stands under: a paragraph,
/// a whole list, a row of a table. A code block, a shortcode, a comment
/// and a heading are no piece: a code block and a screen show what the
/// command draws.
fn pieces(text: &str) -> Vec<(usize, String, String)> {
    let mut out: Vec<(usize, String, String)> = Vec::new();
    let (mut fenced, mut open, mut heading) = (false, false, String::new());
    for (index, line) in text.lines().enumerate() {
        if line.starts_with("```") {
            fenced = !fenced;
            open = false;
            continue;
        }
        if fenced {
            continue;
        }
        if level(line) > 0 {
            line.clone_into(&mut heading);
            open = false;
            continue;
        }
        let no_prose = line.trim().is_empty() || line.starts_with("{{") || line.starts_with("<!--");
        if no_prose {
            open = false;
            continue;
        }
        if line.starts_with('|') {
            out.push((index + 1, heading.clone(), line.trim().to_owned()));
            open = false;
        } else if open {
            let last = out.last_mut().expect("an open piece");
            last.2.push(' ');
            last.2.push_str(line.trim());
        } else {
            out.push((index + 1, heading.clone(), line.trim().to_owned()));
            open = true;
        }
    }
    out
}

/// How many bindings a piece may name outside the Keys table: none where
/// it says how a screen is left, and one anywhere else, for the key a step
/// presses.
fn keys_allowed(piece: &str) -> usize {
    usize::from(!piece.starts_with("**Leave.**"))
}

/// What on a page states the keys a second time: the shortcode that listed
/// a screen's keys, a table of keys outside `### Keys`, a piece outside it
/// that names more bindings than [`keys_allowed`] gives it, and a
/// `**Keys.**` paragraph: the screens where fewer keys work are one table
/// under `### Keys`.
fn key_list_findings(text: &str) -> Vec<String> {
    let mut findings = Vec::new();
    if text.contains("tui_keys") {
        findings.push("the page uses the tui_keys shortcode".to_owned());
    }
    for (line, heading, piece) in pieces(text) {
        if heading == "### Keys" {
            continue;
        }
        if piece.starts_with("| Key | Does") {
            findings.push(format!("line {line}: a second table of keys"));
        }
        let named = bindings_named(&piece);
        if named.len() > keys_allowed(&piece) {
            findings.push(format!(
                "line {line} names the keys of {} bindings ({named:?}); [Keys](#keys) states them",
                named.len()
            ));
        }
        if piece.contains("**Keys.**") {
            findings.push(format!(
                "line {line}: a **Keys.** paragraph under {heading:?}"
            ));
        }
    }
    findings
}

/// `names` as a sentence lists them: `a`, `a and b`, `a, b and c`.
fn listed_in_words(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

/// A binding's first key, as the Keys table writes it.
fn first_key(action: Action) -> String {
    let binding = BINDINGS
        .iter()
        .find(|binding| binding.action == action)
        .expect("a binding");
    let (code, modifiers) = binding.keys[0];
    key_name(code, modifiers)
}

/// The sentence a screen's section has for what differs on that screen,
/// from the code alone. `usual` are the actions the Keys table says do
/// something in the screen's view and `acting` those that do something on
/// the screen. No sentence when the two agree; the one or two silent keys
/// when no more are silent; else the keys that still act.
fn keys_sentence(usual: &[Action], acting: &[Action]) -> Option<String> {
    let silent: Vec<String> = usual
        .iter()
        .filter(|action| !acting.contains(action))
        .map(|action| first_key(*action))
        .collect();
    let said = match silent.len() {
        0 => return None,
        1 | 2 => format!("All but {}", listed_in_words(&silent)),
        _ => {
            let acting: Vec<String> = acting.iter().map(|action| first_key(*action)).collect();
            format!("Only {}", listed_in_words(&acting))
        }
    };
    Some(said)
}

/// The actions the Keys table says do something in `view`.
fn usual_in(view: View) -> Vec<Action> {
    BINDINGS
        .iter()
        .map(|binding| binding.action)
        .filter(|action| works(view, *action))
        .collect()
}

fn acting_on(screen: Screen) -> Vec<Action> {
    screen.keys().iter().map(|binding| binding.action).collect()
}

#[test]
fn the_page_states_the_keys_once() {
    let findings = key_list_findings(&page());
    assert!(findings.is_empty(), "tui.md: {findings:#?}");
}

#[test]
fn the_page_has_one_table_of_keys_and_it_is_the_reference() {
    let page = page();
    assert_eq!(
        page.matches("\n| Key | Does |").count(),
        1,
        "one table on the page says what each key does"
    );
    assert!(
        section(&page, "### Keys").contains("\n| Key | Does |"),
        "and it stands under Keys"
    );
    assert_eq!(
        keys_table().len(),
        BINDINGS.len(),
        "with a row for each binding"
    );
}

#[test]
fn no_template_and_no_data_of_the_site_lists_a_screens_keys() {
    assert!(
        !read("website/templates/components.html").contains("tui_keys"),
        "components.html has no tui_keys"
    );
    for (name, screen) in &screen_data() {
        assert!(
            screen.get("keys").is_none(),
            "tui-screens.json: {name} holds no list of keys"
        );
    }
}

#[test]
fn the_table_of_screens_where_fewer_keys_work_is_what_the_code_does() {
    // One row for each view and set of keys, in the order the screens
    // come; each row's screens in that order too, as links.
    let mut wanted: Vec<(String, String, Vec<String>)> = Vec::new();
    for screen in Screen::ALL {
        let acting = acting_on(screen);
        let usual = usual_in(screen.view());
        for action in &acting {
            assert!(
                usual.contains(action),
                "{}: {action:?} acts, so the Keys table says yes for its view",
                screen.name()
            );
        }
        let Some(keys) = keys_sentence(&usual, &acting) else {
            continue;
        };
        let view = format!("{:?}", screen.view());
        let link = format!("[{}](#{})", screen.title(), anchor(screen.title()));
        match wanted.iter_mut().find(|row| row.0 == view && row.1 == keys) {
            Some(row) => row.2.push(link),
            None => wanted.push((view, keys, vec![link])),
        }
    }
    let wanted: Vec<Vec<String>> = wanted
        .into_iter()
        .map(|(view, keys, screens)| vec![view, keys, screens.join(", ")])
        .collect();
    let shown = table(&section(&page(), "### Keys"), "View");
    assert_eq!(shown, wanted, "the table under Keys that starts with View");
}

#[test]
fn no_screens_section_has_a_keys_paragraph_or_a_list_of_keys() {
    for screen in Screen::ALL {
        let text = screen_section(screen);
        assert!(
            !text.contains("**Keys.**"),
            "{:?} has a Keys paragraph",
            screen.title()
        );
        for (_, _, piece) in pieces(&text) {
            let named = bindings_named(&piece);
            assert!(
                named.len() <= 1,
                "{:?} names the keys of {named:?} in one piece: {piece}",
                screen.title()
            );
        }
    }
}

#[test]
fn the_last_line_names_a_key_only_where_it_does_something() {
    assert!(
        page()
            .replace('\n', " ")
            .contains("The last line names a key only where it does something."),
        "the page says so"
    );
    for screen in Screen::ALL {
        let (columns, rows) = screen.size();
        let lines = frame(&screen.state(), columns, rows);
        let footer = lines.last().expect("a last line");
        let acting = acting_on(screen);
        for binding in &BINDINGS {
            let hint = format!("{} {}", binding.label, binding.hint);
            if footer.contains(&hint) {
                assert!(
                    acting.contains(&binding.action),
                    "{}: the last line names {hint:?}, which does nothing there",
                    screen.name()
                );
            }
        }
    }
}

#[test]
fn no_screens_section_says_how_it_is_left_with_a_key() {
    for screen in Screen::ALL {
        for (_, _, piece) in pieces(&screen_section(screen)) {
            if piece.starts_with("**Leave.**") {
                assert_eq!(
                    bindings_named(&piece),
                    [],
                    "{:?}: Leave links the screens that follow and names no key",
                    screen.title()
                );
            }
        }
    }
}

#[test]
fn every_screen_a_leave_sentence_links_is_a_screen_of_the_page() {
    let titles: Vec<String> = Screen::ALL
        .into_iter()
        .map(|screen| anchor(screen.title()))
        .collect();
    let mut links = 0;
    for screen in Screen::ALL {
        for (_, _, piece) in pieces(&screen_section(screen)) {
            if !piece.starts_with("**Leave.**") {
                continue;
            }
            for target in piece
                .split("](#")
                .skip(1)
                .filter_map(|rest| rest.split(')').next())
            {
                links += 1;
                assert!(
                    titles.iter().any(|title| title == target),
                    "{:?}: Leave links #{target}, which is no screen",
                    screen.title()
                );
            }
        }
    }
    assert!(
        links >= Screen::ALL.len(),
        "the Leave sentences link screens: {links}"
    );
}

/// The screens a Leave sentence says a key press leads to: the screen,
/// the action, and the screen it shows then.
const LEADS_TO: [(Screen, Action, Screen); 13] = [
    (Screen::List, Action::Open, Screen::Detail),
    (Screen::List, Action::Help, Screen::Help),
    (Screen::List, Action::More, Screen::MoreReading),
    (Screen::ListOne, Action::Open, Screen::DetailOne),
    (Screen::Empty, Action::Refresh, Screen::Loading),
    (Screen::NotConnected, Action::Refresh, Screen::Connecting),
    (Screen::RefusedKey, Action::Refresh, Screen::Connecting),
    (
        Screen::RefusedCertificate,
        Action::Refresh,
        Screen::Connecting,
    ),
    (Screen::ServerError, Action::Refresh, Screen::Loading),
    (Screen::Detail, Action::Back, Screen::List),
    (Screen::DetailOne, Action::Back, Screen::ListOne),
    (
        Screen::DetailFailed,
        Action::Refresh,
        Screen::DetailRefreshing,
    ),
    (Screen::DetailRefreshing, Action::Back, Screen::Loading),
];

#[test]
fn a_leave_sentence_links_the_screen_the_code_shows_after_the_key() {
    for (from, action, to) in LEADS_TO {
        let after = press(from.state(), &[action]);
        assert_eq!(
            said(&after),
            said(&to.state()),
            "{} after {action:?} is {}",
            from.name(),
            to.name()
        );
        let leave: Vec<String> = pieces(&screen_section(from))
            .into_iter()
            .map(|(_, _, piece)| piece)
            .filter(|piece| piece.starts_with("**Leave.**"))
            .collect();
        assert!(
            leave
                .iter()
                .any(|piece| piece.contains(&format!("](#{})", anchor(to.title())))),
            "{:?}: Leave links {:?}",
            from.title(),
            to.title()
        );
    }
}

// The readers of the rule, on made-up text, both ways.

#[test]
fn a_letter_key_is_named_by_a_code_span_of_its_own() {
    assert_eq!(bindings_named("press `q` now"), [Action::Quit], "`q`");
    assert_eq!(
        bindings_named("the queue is quiet"),
        [],
        "the letter q in a word"
    );
    assert_eq!(bindings_named("`q quit`"), [], "a span that holds more");
}

#[test]
fn a_named_key_is_a_word_of_its_own() {
    assert_eq!(bindings_named("press Enter"), [Action::Open], "Enter");
    assert_eq!(bindings_named("Entered text"), [], "inside a word");
    assert_eq!(
        bindings_named("the PgUp key"),
        [Action::PageUp],
        "PgUp is not Up"
    );
}

#[test]
fn two_keys_of_one_binding_are_one_binding() {
    assert_eq!(bindings_named("`j` or Down"), [Action::Down], "j and Down");
    assert_eq!(
        bindings_named("`q` or Ctrl-C"),
        [Action::Quit],
        "q and Ctrl-C"
    );
    assert_eq!(
        bindings_named("Esc or Backspace"),
        [Action::Back],
        "Esc and Backspace"
    );
}

#[test]
fn the_bindings_named_come_in_the_order_of_the_table() {
    assert_eq!(
        bindings_named("`q`, `?`, then `j`"),
        [Action::Down, Action::Help, Action::Quit],
        "the table's order, not the text's"
    );
}

#[test]
fn a_key_named_twice_is_one_binding() {
    assert_eq!(
        bindings_named("`n`, and `n` again"),
        [Action::More],
        "n twice"
    );
}

#[test]
fn the_case_of_a_letter_key_counts() {
    assert_eq!(bindings_named("`g`"), [Action::Top], "g is the first vCon");
    assert_eq!(bindings_named("`G`"), [Action::Bottom], "G is the last");
    assert_eq!(bindings_named("`Q`"), [], "Q is not bound");
}

#[test]
fn a_key_name_in_lower_case_is_no_key() {
    assert_eq!(
        bindings_named("at the end, go home and enter"),
        [],
        "words of a sentence"
    );
}

#[test]
fn text_with_no_key_names_no_binding() {
    assert_eq!(bindings_named(""), [], "empty");
    assert_eq!(
        bindings_named("The list is read when the command starts."),
        [],
        "prose"
    );
}

#[test]
fn a_word_reader_takes_the_ends_of_the_text_and_punctuation() {
    assert!(has_word("Esc", "Esc"), "the whole text");
    assert!(has_word("(Esc)", "Esc"), "in parentheses");
    assert!(has_word("press Esc.", "Esc"), "before a full stop");
    assert!(!has_word("Escape", "Esc"), "a longer word");
    assert!(!has_word("Ctrl-Up", "Up"), "after a hyphen");
}

#[test]
fn a_paragraph_is_one_piece_with_its_lines_joined() {
    let found = pieces("# T\n\none\ntwo\n\nthree\n");
    assert_eq!(
        found,
        [
            (3, "# T".to_owned(), "one two".to_owned()),
            (6, "# T".to_owned(), "three".to_owned())
        ],
        "two paragraphs"
    );
}

#[test]
fn a_whole_list_is_one_piece_and_each_row_of_a_table_is_one() {
    let found = pieces("- a\n- b\n\n| x |\n|---|\n| y |\nafter\n");
    let texts: Vec<&str> = found.iter().map(|(_, _, piece)| piece.as_str()).collect();
    assert_eq!(
        texts,
        ["- a - b", "| x |", "|---|", "| y |", "after"],
        "a list, three rows, and the line after the table"
    );
}

#[test]
fn a_table_of_screens_may_name_one_key_in_each_row() {
    let text = "| Screen | When |\n|---|---|\n| Help | After `?`. |\n| Next | After `n`. |\n";
    assert_eq!(
        key_list_findings(text),
        Vec::<String>::new(),
        "one key a row"
    );
    let two = "| Screen | When |\n|---|---|\n| Help | After `?` or `n`. |\n";
    assert_eq!(key_list_findings(two).len(), 1, "two keys in one row");
}

#[test]
fn a_code_block_a_shortcode_and_a_comment_are_no_piece() {
    let text = "```text\n`j` `k` `q`\n```\n{{ <tui_screen name=\"list\" /> }}\n<!-- docs-test: keys Enter q -->\n";
    assert_eq!(pieces(text), [], "nothing a reader takes as prose");
}

#[test]
fn a_piece_stands_under_the_last_heading_before_it() {
    let found = pieces("## A\n\none\n\n### B\n\ntwo\n");
    let under: Vec<&str> = found
        .iter()
        .map(|(_, heading, _)| heading.as_str())
        .collect();
    assert_eq!(under, ["## A", "### B"], "each under its heading");
}

#[test]
fn a_heading_ends_a_piece() {
    let found = pieces("one\n### B\ntwo\n");
    assert_eq!(found.len(), 2, "the heading stands between two pieces");
    assert_eq!(found[1].1, "### B", "the second is under it");
}

#[test]
fn a_page_with_no_text_has_no_piece_and_no_finding() {
    assert_eq!(pieces(""), [], "no piece");
    assert_eq!(key_list_findings(""), Vec::<String>::new(), "no finding");
}

#[test]
fn a_step_that_names_the_key_it_presses_is_allowed() {
    let text =
        "### Read the next page\n\nPress `n`. `n` does nothing when the server has no more.\n";
    assert_eq!(key_list_findings(text), Vec::<String>::new(), "one binding");
}

#[test]
fn a_paragraph_that_names_two_bindings_is_refused() {
    let found = key_list_findings("### Find\n\nPress `n` again, or `r`.\n");
    assert_eq!(found.len(), 1, "one finding: {found:?}");
    assert!(
        found[0].starts_with("line 3 names the keys of 2 bindings"),
        "{found:?}"
    );
}

#[test]
fn a_list_with_one_key_in_each_item_is_refused() {
    let found = key_list_findings("- `j` selects the vCon below.\n- `k` selects the one above.\n");
    assert_eq!(found.len(), 1, "the list is one piece: {found:?}");
}

#[test]
fn a_second_table_of_keys_is_refused() {
    let text = "### A screen\n\n| Key | Does |\n|---|---|\n| `r` | Reads |\n| `q` | Quits |\n";
    assert_eq!(
        key_list_findings(text),
        ["line 3: a second table of keys"],
        "a table outside the reference"
    );
}

#[test]
fn the_reference_table_is_not_a_finding() {
    let text = "### Keys\n\n| Key | Does |\n|---|---|\n| `r` | Reads |\n| `q` | Quits |\n\nEsc, `?` and `q` close help.\n";
    assert_eq!(
        key_list_findings(text),
        Vec::<String>::new(),
        "under ### Keys"
    );
}

#[test]
fn the_reference_ends_at_the_next_heading() {
    let text = "### Keys\n\n`r` and `q`.\n\n### Environment\n\n`r` and `q`.\n";
    let found = key_list_findings(text);
    assert_eq!(
        found.len(),
        1,
        "the paragraph after the reference: {found:?}"
    );
    assert!(found[0].starts_with("line 7 "), "{found:?}");
}

#[test]
fn the_shortcode_that_listed_a_screens_keys_is_refused() {
    let found = key_list_findings("### The list\n\n{{ <tui_keys name=\"list\" /> }}\n");
    assert_eq!(
        found,
        ["the page uses the tui_keys shortcode"],
        "the shortcode"
    );
}

#[test]
fn a_leave_sentence_that_names_a_key_is_refused() {
    let found = key_list_findings("### The list\n\n**Leave.** `q` ends the command.\n");
    assert_eq!(found.len(), 1, "Leave names no key: {found:?}");
}

#[test]
fn a_leave_sentence_of_links_is_allowed() {
    let text = "### The list\n\n**Leave.** To [help](#help) or to [loading](#loading).\n";
    assert_eq!(key_list_findings(text), Vec::<String>::new(), "links only");
}

#[test]
fn a_keys_paragraph_that_lists_keys_is_refused_twice_over() {
    let text = "### Loading\n\n**Keys.** Only `?` and `q` do something on this screen.\n";
    let found = key_list_findings(text);
    assert_eq!(found.len(), 2, "the keys, and the paragraph: {found:?}");
    assert_eq!(
        found[1], "line 3: a **Keys.** paragraph under \"### Loading\"",
        "the paragraph"
    );
}

#[test]
fn a_keys_paragraph_that_names_one_key_is_refused() {
    let text = "### Loading\n\n**Keys.** `n` does nothing.\n";
    assert_eq!(
        key_list_findings(text),
        ["line 3: a **Keys.** paragraph under \"### Loading\""],
        "one key is still a Keys paragraph"
    );
}

#[test]
fn the_table_of_screens_where_fewer_keys_work_is_allowed_under_keys() {
    let table = "| View | Keys that work | Screens |\n|---|---|---|\n| List | Only `?` and `q` | [Loading](#loading) |\n";
    assert_eq!(
        key_list_findings(&format!("### Keys\n\n{table}")),
        Vec::<String>::new(),
        "under the reference"
    );
}

#[test]
fn keys_in_a_code_block_are_what_the_command_draws_and_no_finding() {
    let text = "### Help\n\n```text\nEsc back  q quit\n  j  Select\n  k  Select\n```\n";
    assert_eq!(key_list_findings(text), Vec::<String>::new(), "a drawing");
}

#[test]
fn a_what_to_do_paragraph_may_name_the_one_key_to_press() {
    let text = "### A failure\n\n**What to do.** Check the server, then press `r`.\n";
    assert_eq!(key_list_findings(text), Vec::<String>::new(), "one key");
    let two = "### A failure\n\n**What to do.** Press `r`, or `q` to end.\n";
    assert_eq!(key_list_findings(two).len(), 1, "two keys");
}

#[test]
fn a_finding_says_the_line_and_the_bindings_it_names() {
    let found = key_list_findings("### Find\n\ntext\n\nPress `r`,\nthen `n`.\n");
    assert_eq!(
        found,
        ["line 5 names the keys of 2 bindings ([Refresh, More]); [Keys](#keys) states them"],
        "the first line of the paragraph, and both bindings"
    );
}

#[test]
fn a_leave_sentence_that_names_a_key_by_its_word_is_refused() {
    let found = key_list_findings("### The detail\n\n**Leave.** Esc goes back to the list.\n");
    assert_eq!(found.len(), 1, "Esc in Leave: {found:?}");
    assert!(found[0].contains("[Back]"), "{found:?}");
}

#[test]
fn each_paragraph_that_lists_keys_is_a_finding_of_its_own() {
    let found = key_list_findings("`j` and `k` select.\n\nprose\n\n`g` and `G` jump.\n");
    assert_eq!(found.len(), 2, "two paragraphs: {found:?}");
    assert!(found[0].starts_with("line 1 "), "{found:?}");
    assert!(found[1].starts_with("line 5 "), "{found:?}");
}

#[test]
fn a_what_to_do_paragraph_that_names_two_keys_is_refused() {
    let text = "### A failure\n\n**What to do.** Press Esc, then `n` in the list.\n";
    let found = key_list_findings(text);
    assert_eq!(found.len(), 1, "two keys: {found:?}");
    assert!(found[0].contains("[Back, More]"), "{found:?}");
}

#[test]
fn a_table_of_keys_under_a_heading_that_is_not_the_reference_is_refused() {
    let text = "#### Keys\n\n| Key | Does |\n|---|---|\n| `q` | Quits |\n";
    assert_eq!(
        key_list_findings(text),
        ["line 3: a second table of keys"],
        "only ### Keys is the reference"
    );
}

#[test]
fn the_shortcode_is_refused_wherever_the_page_names_it() {
    let found = key_list_findings("See the tui_keys component.\n");
    assert_eq!(
        found,
        ["the page uses the tui_keys shortcode"],
        "in prose too"
    );
}

#[test]
fn a_paragraph_that_names_every_key_is_refused_with_all_twelve() {
    let text = "`j` `k` `g` `G` PgUp PgDn Enter Esc `r` `n` `?` `q`\n";
    let found = key_list_findings(text);
    assert_eq!(found.len(), 1, "one paragraph: {found:?}");
    assert!(found[0].contains("the keys of 12 bindings"), "{found:?}");
}

#[test]
fn names_are_listed_in_words() {
    let names = |n: usize| -> Vec<String> {
        ["a", "b", "c"][..n]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    };
    assert_eq!(listed_in_words(&names(0)), "", "none");
    assert_eq!(listed_in_words(&names(1)), "a", "one");
    assert_eq!(listed_in_words(&names(2)), "a and b", "two");
    assert_eq!(listed_in_words(&names(3)), "a, b and c", "three");
}

#[test]
fn the_first_key_of_a_binding_is_written_as_the_table_writes_it() {
    assert_eq!(first_key(Action::Down), "`j`", "a letter in a code span");
    assert_eq!(first_key(Action::Open), "Enter", "a named key");
    assert_eq!(first_key(Action::Back), "Esc", "the first of two");
    assert_eq!(first_key(Action::Help), "`?`", "a sign");
}

#[test]
fn a_screen_where_every_usual_key_acts_has_no_sentence() {
    let usual = [Action::Back, Action::Help, Action::Quit];
    assert_eq!(keys_sentence(&usual, &usual), None, "nothing differs");
    assert_eq!(keys_sentence(&[], &[]), None, "no key at all");
}

#[test]
fn one_silent_key_is_named_in_the_singular() {
    let usual = [Action::Refresh, Action::More, Action::Quit];
    assert_eq!(
        keys_sentence(&usual, &[Action::Refresh, Action::Quit]).as_deref(),
        Some("All but `n`"),
        "n alone"
    );
}

#[test]
fn two_silent_keys_are_named_in_the_plural() {
    let usual = [Action::Refresh, Action::More, Action::Quit];
    assert_eq!(
        keys_sentence(&usual, &[Action::Quit]).as_deref(),
        Some("All but `r` and `n`"),
        "r and n"
    );
}

#[test]
fn with_three_silent_keys_the_sentence_names_the_keys_that_act() {
    let usual = [
        Action::Open,
        Action::Refresh,
        Action::More,
        Action::Help,
        Action::Quit,
    ];
    assert_eq!(
        keys_sentence(&usual, &[Action::Help, Action::Quit]).as_deref(),
        Some("Only `?` and `q`"),
        "the two that act"
    );
}

#[test]
fn the_keys_that_act_are_named_in_the_order_given() {
    let usual = usual_in(View::List);
    let said = keys_sentence(
        &usual,
        &[Action::Open, Action::Refresh, Action::Help, Action::Quit],
    );
    assert_eq!(
        said.as_deref(),
        Some("Only Enter, `r`, `?` and `q`"),
        "four that act"
    );
}

#[test]
fn the_usual_keys_of_a_view_are_the_yes_cells_of_the_keys_table() {
    for (column, view) in [(2, View::List), (3, View::Detail), (4, View::Help)] {
        let usual = usual_in(view);
        for (binding, row) in BINDINGS.iter().zip(&keys_table()) {
            assert_eq!(
                usual.contains(&binding.action),
                row[column] == "yes",
                "{:?} in the {view:?} view",
                binding.action
            );
        }
    }
}

#[test]
fn help_has_only_the_keys_that_close_it_and_quit_as_usual() {
    assert_eq!(
        usual_in(View::Help),
        [Action::Back, Action::Help, Action::Quit],
        "the Help column"
    );
}

#[test]
fn opening_is_usual_in_the_list_and_going_back_in_the_detail() {
    assert!(
        usual_in(View::List).contains(&Action::Open),
        "Enter in the list"
    );
    assert!(
        !usual_in(View::List).contains(&Action::Back),
        "Esc in the list"
    );
    assert!(
        usual_in(View::Detail).contains(&Action::Back),
        "Esc in the detail"
    );
    assert!(
        !usual_in(View::Detail).contains(&Action::Open),
        "Enter in the detail"
    );
}

#[test]
fn the_waiting_and_refused_screens_differ_and_the_full_list_does_not() {
    let differs =
        |screen: Screen| keys_sentence(&usual_in(screen.view()), &acting_on(screen)).is_some();
    assert!(differs(Screen::Connecting), "connecting");
    assert!(differs(Screen::Loading), "loading");
    assert!(differs(Screen::OperatorKey), "an operator key");
    assert!(!differs(Screen::Help), "help");
    assert!(!differs(Screen::ALL[0]), "the first screen, the list");
}

#[test]
fn every_leave_and_keys_piece_is_one_the_rule_reads() {
    // The rule reads a piece by its first words: the page writes them the
    // same way in each section.
    let text = screens_section();
    let leaves = pieces(&text)
        .iter()
        .filter(|(_, _, piece)| piece.starts_with("**Leave.**"))
        .count();
    assert_eq!(leaves, Screen::ALL.len(), "one Leave piece for each screen");
    assert_eq!(
        text.matches("**Leave.**").count(),
        leaves,
        "no Leave stands in the middle of a paragraph"
    );
}
