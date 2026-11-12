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
    let shown = shown(&page());
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
