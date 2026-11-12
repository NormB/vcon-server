//! Drawing: a state and a theme give a frame. It reads nothing but its
//! arguments, so every screen is tested on ratatui's `TestBackend` at the
//! size the test chooses.
//!
//! Nothing is said by color alone: the connection is a word, the selected
//! row has a marker, and without color the frame uses no color at all.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

use crate::tui::keys::{BINDINGS, binding};
use crate::tui::state::{Action, Connection, Listing, Row, State, View};

/// The smallest terminal a view is drawn in.
const MIN_COLUMNS: u16 = 40;
const MIN_LINES: u16 = 8;
/// The offset every time on screen is in, as it is written beside them.
const ZONE: &str = "UTC+00:00";
/// The columns of a list row: the marker and the uuid, then each time, then
/// the version.
const UUID_COLUMNS: u16 = 38;
const TIME_COLUMNS: u16 = 21;
const VERSION_COLUMNS: u16 = 34;

/// Whether color may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// False for a monochrome terminal or a user who asked for none.
    pub color: bool,
}

/// The theme for an environment: no color when `NO_COLOR` is set to
/// anything but the empty string (<https://no-color.org>), when `TERM` is
/// `dumb`, or when there is no `TERM`.
#[must_use]
pub fn theme(no_color: Option<&str>, term: Option<&str>) -> Theme {
    let refused = no_color.is_some_and(|value| !value.is_empty());
    let capable = term.is_some_and(|term| term != "dumb");
    Theme {
        color: capable && !refused,
    }
}

/// An RFC 3339 time as `YYYY-MM-DD HH:MM:SS` in UTC; text that is not one
/// is returned as it is.
#[must_use]
pub fn utc(text: &str) -> String {
    OffsetDateTime::parse(text, &Rfc3339).map_or_else(|_| text.to_owned(), stamp)
}

/// `at` as `YYYY-MM-DD HH:MM:SS` in UTC.
fn stamp(at: OffsetDateTime) -> String {
    let at = at.to_offset(UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

/// Draws `state` on the whole frame.
pub fn draw(frame: &mut Frame<'_>, state: &State, theme: Theme) {
    let area = frame.area();
    if area.width < MIN_COLUMNS || area.height < MIN_LINES {
        let message = format!(
            "Terminal too small: {MIN_COLUMNS}x{MIN_LINES} is needed. Enlarge it, or press q to quit."
        );
        frame.render_widget(Paragraph::new(message).wrap(Wrap { trim: true }), area);
        return;
    }
    let line = |y: u16| Rect {
        y: area.y.saturating_add(y),
        height: 1,
        ..area
    };
    // The notice takes the lines it needs, up to half of what is left
    // after the status line and the key hints, and one line when empty.
    let most = usize::from(area.height.saturating_sub(2) / 2).max(1);
    let said = fitted(&notice(state), usize::from(area.width), most);
    let notice_lines = u16::try_from(said.len()).unwrap_or(1).max(1);
    let body = Rect {
        y: area.y.saturating_add(1),
        height: area.height.saturating_sub(notice_lines.saturating_add(2)),
        ..area
    };
    frame.render_widget(Paragraph::new(status(state, area.width, theme)), line(0));
    match state.view {
        View::List => list(frame, state, body),
        View::Detail => detail(frame, state, body),
        View::Help => help(frame, body),
    }
    let said: Vec<Line<'_>> = said.into_iter().map(Line::from).collect();
    frame.render_widget(
        Paragraph::new(said),
        Rect {
            y: area.y.saturating_add(body.height).saturating_add(1),
            height: notice_lines,
            ..area
        },
    );
    frame.render_widget(
        Paragraph::new(hints(state.view, usize::from(area.width))),
        line(area.height.saturating_sub(1)),
    );
}

/// `text` as lines of at most `columns` characters, broken between words;
/// a word longer than a line is broken where the line ends.
fn wrapped(text: &str, columns: usize) -> Vec<String> {
    let columns = columns.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut used = 0_usize;
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        let mut rest: Vec<char> = word.chars().collect();
        if used > 0 && used.saturating_add(1).saturating_add(rest.len()) > columns {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        while rest.len() > columns {
            lines.push(rest.drain(..columns).collect());
        }
        if used > 0 {
            line.push(' ');
            used = used.saturating_add(1);
        }
        used = used.saturating_add(rest.len());
        line.extend(rest);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The parts of the notice as the lines drawn: on one line, joined by
/// [`BETWEEN`], when that fits `columns`; otherwise each part on lines of
/// its own, wrapped. At most `most` lines: when there are more, the last
/// one drawn ends with [`CUT`].
fn fitted(parts: &[String], columns: usize, most: usize) -> Vec<String> {
    let whole = parts.join(BETWEEN);
    if whole.chars().count() <= columns {
        return vec![whole];
    }
    let mut lines: Vec<String> = parts
        .iter()
        .flat_map(|part| wrapped(part, columns))
        .collect();
    if lines.len() > most {
        lines.truncate(most);
        if let Some(last) = lines.last_mut() {
            let kept = columns.saturating_sub(CUT.chars().count());
            *last = last.chars().take(kept).collect();
            last.push_str(CUT);
        }
    }
    lines
}

/// Whose key the server answered for, as the status line names it.
enum Owner<'a> {
    /// The server has not answered for the key.
    Unknown,
    /// An operator key, which has no tenant.
    Operator,
    /// A tenant key, with the tenant's id.
    Tenant(&'a str),
}

/// The scheme every server address has, and so the first thing left out.
const SCHEME: &str = "https://";
/// Between the parts of the status line.
const BETWEEN: &str = " | ";
/// Ends a part that was shortened.
const CUT: &str = "...";
/// The fewest characters of a tenant id worth showing.
const ID_MIN: usize = 4;

/// The status line before the connection word, which the caller appends:
/// the server's address and whose key it is, each followed by [`BETWEEN`].
///
/// THE RULE for a line longer than `columns`. An operator needs the
/// connection first, then the server, then the tenant, so the parts give
/// way in the other order, and nothing that carries information goes
/// before something that carries none:
///
/// 1. The connection word is never shortened or left out.
/// 2. The address loses `https://`, which every address has.
/// 3. A tenant id is shortened to what fits, with `...`, down to
///    [`ID_MIN`] characters. A phrase (`tenant unknown`, `no tenant
///    (operator key)`) is never cut.
/// 4. The tenant part is left out.
/// 5. Only then is the address shortened, at its end, with `...`.
fn status_lead(server: &str, owner: &Owner<'_>, word: &str, columns: usize) -> String {
    let length = |text: &str| text.chars().count();
    let owner_whole = match owner {
        Owner::Unknown => "tenant unknown".to_owned(),
        Owner::Operator => "no tenant (operator key)".to_owned(),
        Owner::Tenant(id) => format!("tenant {id}"),
    };
    // What the address and the tenant part may take together.
    let room = columns.saturating_sub(length(word));
    let with_owner = |address: &str, owner: &str| format!("{address}{BETWEEN}{owner}{BETWEEN}");
    let whole = with_owner(server, &owner_whole);
    if length(&whole) <= room {
        return whole;
    }
    let bare = server.strip_prefix(SCHEME).unwrap_or(server);
    let unschemed = with_owner(bare, &owner_whole);
    if length(&unschemed) <= room {
        return unschemed;
    }
    if let Owner::Tenant(id) = owner {
        let around = length(&with_owner(bare, "tenant ")).saturating_add(length(CUT));
        let kept = room.saturating_sub(around);
        if kept >= ID_MIN {
            let id: String = id.chars().take(kept).collect();
            return with_owner(bare, &format!("tenant {id}{CUT}"));
        }
    }
    let alone = format!("{bare}{BETWEEN}");
    if length(&alone) <= room {
        return alone;
    }
    let kept = room.saturating_sub(length(BETWEEN).saturating_add(length(CUT)));
    let address: String = bare.chars().take(kept).collect();
    format!("{address}{CUT}{BETWEEN}")
}

/// The status line: the server, the tenant and the connection, shortened
/// by [`status_lead`]'s rule when the terminal is narrower than the line.
fn status(state: &State, columns: u16, theme: Theme) -> Line<'static> {
    let (owner, word, color) = match &state.connection {
        Connection::Connecting => (Owner::Unknown, "connecting", Color::Yellow),
        Connection::Failed(_) => (Owner::Unknown, "not connected", Color::Red),
        Connection::Connected(info) => (
            info.tenant
                .as_deref()
                .map_or(Owner::Operator, Owner::Tenant),
            "connected",
            Color::Green,
        ),
    };
    let lead = status_lead(&state.server, &owner, word, usize::from(columns));
    let mut style = Style::new().add_modifier(Modifier::BOLD);
    if theme.color {
        style = style.fg(color);
    }
    Line::from(vec![Span::raw(lead), Span::styled(word, style)])
}

/// A message in place of a view's content, wrapped to the body.
fn message(frame: &mut Frame<'_>, body: Rect, lines: Vec<String>) {
    let lines: Vec<Line<'_>> = lines.into_iter().map(Line::from).collect();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}

/// What a view shows instead of its content while there is no list: the
/// connecting, loading and error states. `None` when the list is ready.
fn waiting(state: &State, loading: &str) -> Option<Vec<String>> {
    if state.connection == Connection::Connecting {
        return Some(vec![
            "Connecting to the server...".to_owned(),
            String::new(),
            "Press q to quit.".to_owned(),
        ]);
    }
    match &state.listing {
        Listing::Ready => None,
        Listing::Loading => Some(vec![
            loading.to_owned(),
            String::new(),
            "Press q to quit.".to_owned(),
        ]),
        Listing::Failed(why) => {
            let mut lines = vec![format!("Could not list vCons: {why}."), String::new()];
            if why.contains("(401)") {
                lines
                    .push("Check that key_file holds a key the server has not revoked.".to_owned());
            }
            lines.push("Press r to try again, or q to quit.".to_owned());
            // The client file, the token and the CA are read once, before
            // the first screen, so r cannot pick up a corrected file.
            lines.push(
                "A change to the client file or the key file takes effect at the next start."
                    .to_owned(),
            );
            Some(lines)
        }
    }
}

/// The list view.
fn list(frame: &mut Frame<'_>, state: &State, body: Rect) {
    if let Some(lines) = waiting(state, "Loading vCons...") {
        return message(frame, body, lines);
    }
    if state.rows.is_empty() {
        let lines = vec![
            "This tenant has no vCons.".to_owned(),
            String::new(),
            "Press r to read again, or q to quit.".to_owned(),
        ];
        return message(frame, body, lines);
    }
    let times = u16::from(body.width >= UUID_COLUMNS.saturating_add(TIME_COLUMNS)).saturating_add(
        u16::from(body.width >= UUID_COLUMNS.saturating_add(TIME_COLUMNS.saturating_mul(2))),
    );
    let version = body.width
        >= UUID_COLUMNS
            .saturating_add(TIME_COLUMNS.saturating_mul(2))
            .saturating_add(VERSION_COLUMNS);
    let columns = |marker: &str, uuid: &str, stored: &str, updated: &str, tag: &str| {
        let mut text = format!("{marker}{uuid:<36}");
        if times >= 1 {
            text.push_str(&format!("  {stored:<19}"));
        }
        if times >= 2 {
            text.push_str(&format!("  {updated:<19}"));
        }
        if version {
            text.push_str(&format!("  {tag}"));
        }
        text
    };
    let visible = usize::from(body.height.saturating_sub(1));
    let first = state.selected.saturating_add(1).saturating_sub(visible);
    let mut lines = vec![Line::styled(
        columns(
            "  ",
            "UUID",
            &format!("STORED ({ZONE})"),
            &format!("UPDATED ({ZONE})"),
            "VERSION",
        ),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    for (index, row) in state.rows.iter().enumerate().skip(first).take(visible) {
        let selected = index == state.selected;
        let text = columns(
            if selected { "> " } else { "  " },
            &row.uuid,
            &utc(&row.stored_at),
            &utc(&row.updated_at),
            &row.version,
        );
        let style = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        lines.push(Line::styled(text, style));
    }
    frame.render_widget(Paragraph::new(lines), body);
}

/// The detail view: the metadata of one vCon, from its row of the list.
fn detail(frame: &mut Frame<'_>, state: &State, body: Rect) {
    if let Some(lines) = waiting(state, "Reading the list again...") {
        return message(frame, body, lines);
    }
    let lines = match state.detail_row() {
        Some(row) => metadata(row),
        None => vec![
            "This vCon is no longer in the list: it was deleted, or it is past the pages read."
                .to_owned(),
            String::new(),
            "Press Esc to go back.".to_owned(),
        ],
    };
    message(frame, body, lines);
}

/// The lines of one vCon's metadata.
fn metadata(row: &Row) -> Vec<String> {
    vec![
        format!("vCon {}", row.uuid),
        String::new(),
        format!("Version       {}", row.version),
        format!("First stored  {} {ZONE}", utc(&row.stored_at)),
        format!("Last updated  {} {ZONE}", utc(&row.updated_at)),
        String::new(),
        "Content is not shown: this view lists metadata only.".to_owned(),
    ]
}

/// The help view: every binding, from the table the key reader uses.
fn help(frame: &mut Frame<'_>, body: Rect) {
    let mut lines = vec!["Keys".to_owned()];
    for binding in &BINDINGS {
        lines.push(format!("  {:<7}{}", binding.label, binding.does));
    }
    lines.push(String::new());
    lines.push(format!("Every time shown is in UTC ({ZONE})."));
    message(frame, body, lines);
}

/// The notice above the key hints, as its parts: what went wrong with the
/// last request, or where the selection is and when the list was read.
/// [`fitted`] makes lines of them.
fn notice(state: &State) -> Vec<String> {
    if let Some(notice) = &state.notice {
        return vec![notice.clone()];
    }
    if state.more {
        return vec!["Reading the next page...".to_owned()];
    }
    let (Listing::Ready, Some(as_of)) = (&state.listing, state.as_of) else {
        return Vec::new();
    };
    let as_of = format!("as of {} {ZONE}", stamp(as_of));
    if state.rows.is_empty() {
        return vec![as_of];
    }
    let more = if state.next.is_some() {
        ", more on the server"
    } else {
        ""
    };
    vec![
        format!(
            "{} of {} read{more}",
            state.selected.saturating_add(1),
            state.rows.len()
        ),
        as_of,
    ]
}

/// The key hints of a view, from the bindings, in `columns` columns. Help
/// and quit end every line. When the line is longer than the terminal is
/// wide, the hint before those two is left out, then the one before it,
/// until the line fits: a hint is whole or absent, and `?` lists every key.
fn hints(view: View, columns: usize) -> String {
    /// The hints never left out: the last two of a view.
    const KEPT: usize = 2;
    let mut shown = view_hints(view);
    let length = |shown: &[String]| {
        shown
            .iter()
            .map(|hint| hint.chars().count())
            .sum::<usize>()
            .saturating_add(shown.len().saturating_sub(1).saturating_mul(2))
    };
    while shown.len() > KEPT && length(&shown) > columns {
        shown.remove(shown.len().saturating_sub(KEPT).saturating_sub(1));
    }
    shown.join("  ")
}

/// Every key hint of a view, each a key and a word.
fn view_hints(view: View) -> Vec<String> {
    let actions: &[Action] = match view {
        View::List => &[
            Action::Down,
            Action::Up,
            Action::Open,
            Action::Refresh,
            Action::More,
            Action::Help,
            Action::Quit,
        ],
        View::Detail => &[
            Action::Back,
            Action::Down,
            Action::Up,
            Action::Refresh,
            Action::Help,
            Action::Quit,
        ],
        View::Help => &[Action::Back, Action::Quit],
    };
    actions
        .iter()
        .filter_map(|action| binding(*action))
        .map(|binding| format!("{} {}", binding.label, binding.hint))
        .collect()
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Cell;
    use ratatui::style::{Color, Modifier};
    use time::format_description::well_known::Rfc3339;
    use time::macros::datetime;
    use time::{Duration, OffsetDateTime};

    use super::{Theme, draw, theme, utc};
    use crate::tui::client;
    use crate::tui::keys::BINDINGS;
    use crate::tui::state::{Action, ApiError, Event, KeyInfo, Page, Row, State};

    const NOW: OffsetDateTime = datetime!(2026-11-10 01:20:00 UTC);
    const MONO: Theme = Theme { color: false };
    const COLOR: Theme = Theme { color: true };

    fn row(n: u8) -> Row {
        Row {
            uuid: format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02}"),
            stored_at: "2026-10-25T12:00:00.123456789Z".to_owned(),
            updated_at: "2026-10-25T12:30:00Z".to_owned(),
            version: format!("4f0c4b0a1d2e3f405162738495a6b7{n:02}"),
        }
    }

    fn connecting() -> State {
        State::new("https://vcon.example.com:8443", (80, 24)).0
    }

    fn connected() -> State {
        let mut state = connecting();
        state.step(
            Event::KeyInfo(Ok(KeyInfo {
                id: "0123456789abcdef".to_owned(),
                tenant: Some("t_example".to_owned()),
                scopes: vec!["read".to_owned()],
            })),
            NOW,
        );
        state
    }

    fn listed(rows: u8, next: Option<&str>) -> State {
        let mut state = connected();
        state.step(
            Event::Page(Ok(Page {
                vcons: (0..rows).map(row).collect(),
                next: next.map(str::to_owned),
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

    fn terminal(state: &State, columns: u16, lines: u16, theme: Theme) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(columns, lines)).expect("a terminal");
        terminal
            .draw(|frame| draw(frame, state, theme))
            .expect("a frame");
        terminal
    }

    /// The screen's lines as text.
    fn screen(state: &State, columns: u16, lines: u16) -> Vec<String> {
        let terminal = terminal(state, columns, lines, MONO);
        let buffer = terminal.backend().buffer();
        (0..lines)
            .map(|y| (0..columns).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    fn text(state: &State) -> String {
        screen(state, 80, 24).join("\n")
    }

    // Success.

    #[test]
    fn the_status_line_names_the_server_the_tenant_and_the_connection() {
        let lines = screen(&listed(1, None), 80, 24);
        assert!(
            lines[0].contains("https://vcon.example.com:8443"),
            "the server: {}",
            lines[0]
        );
        assert!(
            lines[0].contains("tenant t_example"),
            "the tenant: {}",
            lines[0]
        );
        assert!(
            lines[0].contains("connected"),
            "the connection, in words: {}",
            lines[0]
        );
    }

    #[test]
    fn the_list_shows_each_vcon_with_its_times_in_utc() {
        let lines = screen(&listed(2, None), 80, 24);
        assert!(
            lines[1].contains("STORED (UTC+00:00)"),
            "the header states the offset: {}",
            lines[1]
        );
        assert!(
            lines[2].contains(&row(0).uuid),
            "the first row's uuid: {}",
            lines[2]
        );
        assert!(
            lines[2].contains("2026-10-25 12:00:00"),
            "its stored time: {}",
            lines[2]
        );
        assert!(
            lines[2].contains("2026-10-25 12:30:00"),
            "its updated time: {}",
            lines[2]
        );
        assert!(
            lines[3].contains(&row(1).uuid),
            "the second row: {}",
            lines[3]
        );
    }

    #[test]
    fn the_selected_row_is_marked_with_a_character() {
        let lines = screen(&press(listed(3, None), &[Action::Down]), 80, 24);
        assert!(
            lines[3].starts_with("> "),
            "the selected row has a marker: {}",
            lines[3]
        );
        assert!(
            lines[2].starts_with("  "),
            "the others do not: {}",
            lines[2]
        );
    }

    #[test]
    fn the_list_footer_hints_are_real_bindings() {
        let lines = screen(&listed(1, None), 80, 24);
        let footer = &lines[23];
        for wanted in [Action::Open, Action::Refresh, Action::Help, Action::Quit] {
            let binding = BINDINGS
                .iter()
                .find(|b| b.action == wanted)
                .expect("a binding");
            assert!(
                footer.contains(&format!("{} {}", binding.label, binding.hint)),
                "the footer shows {wanted:?} as bound: {footer}"
            );
        }
    }

    #[test]
    fn the_notice_line_says_how_many_are_loaded_and_as_of_when() {
        let lines = screen(&listed(2, Some("cursor-1")), 80, 24);
        assert!(lines[22].contains("1 of 2"), "the position: {}", lines[22]);
        assert!(
            lines[22].contains("more on the server"),
            "that more exist: {}",
            lines[22]
        );
        assert!(
            lines[22].contains("as of 2026-11-10 01:20:00 UTC+00:00"),
            "when it was read, with the offset: {}",
            lines[22]
        );
    }

    #[test]
    fn the_detail_view_shows_the_metadata_and_says_content_is_not_shown() {
        let shown = text(&press(listed(2, None), &[Action::Open]));
        assert!(shown.contains(&row(0).uuid), "the uuid: {shown}");
        assert!(shown.contains(&row(0).version), "the version: {shown}");
        assert!(
            shown.contains("2026-10-25 12:00:00 UTC+00:00"),
            "first stored: {shown}"
        );
        assert!(
            shown.contains("2026-10-25 12:30:00 UTC+00:00"),
            "last updated: {shown}"
        );
        assert!(
            shown.contains("Content is not shown"),
            "what is left out: {shown}"
        );
        assert!(shown.contains("Esc back"), "how to leave: {shown}");
    }

    #[test]
    fn the_help_view_lists_every_binding() {
        let shown = text(&press(listed(1, None), &[Action::Help]));
        for binding in &BINDINGS {
            assert!(
                shown.contains(binding.label),
                "{} is listed: {shown}",
                binding.label
            );
            assert!(
                shown.contains(binding.does),
                "{} is described: {shown}",
                binding.does
            );
        }
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selection_on_screen() {
        let state = press(listed(60, None), &[Action::Bottom]);
        let shown = text(&state);
        assert!(
            shown.contains(&format!("> {}", row(59).uuid)),
            "the last row is drawn selected"
        );
        assert!(
            !shown.contains(&row(0).uuid),
            "the first row has scrolled off"
        );
    }

    #[test]
    fn with_color_the_connection_is_colored_and_still_in_words() {
        let state = listed(1, None);
        let terminal = terminal(&state, 80, 24, COLOR);
        let buffer = terminal.backend().buffer();
        let colored = (0..80).any(|x| buffer[(x, 0)].fg != Color::Reset);
        assert!(colored, "a color terminal gets color");
        assert!(
            screen(&state, 80, 24)[0].contains("connected"),
            "and the word is still there"
        );
    }

    #[test]
    fn rfc_3339_times_are_shown_to_the_second_in_utc() {
        assert_eq!(
            utc("2026-10-25T12:00:00.123456789Z"),
            "2026-10-25 12:00:00",
            "fractions dropped"
        );
        assert_eq!(
            utc("2026-10-25T14:30:00+02:00"),
            "2026-10-25 12:30:00",
            "an offset is converted"
        );
    }

    #[test]
    fn a_color_terminal_gets_color() {
        assert_eq!(
            theme(None, Some("xterm-256color")),
            COLOR,
            "nothing asks for none"
        );
    }

    #[test]
    fn a_wider_terminal_shows_the_version_column() {
        let lines = screen(&listed(1, None), 132, 43);
        assert!(lines[1].contains("VERSION"), "the header: {}", lines[1]);
        assert!(
            lines[2].contains(&row(0).version),
            "the version: {}",
            lines[2]
        );
    }

    // Failure.

    #[test]
    fn a_failed_connection_is_said_in_the_status_line_and_the_list() {
        let mut state = connecting();
        state.step(
            Event::KeyInfo(Err(ApiError::Unreachable("connection refused".to_owned()))),
            NOW,
        );
        let lines = screen(&state, 80, 24);
        assert!(
            lines[0].contains("not connected"),
            "the status line: {}",
            lines[0]
        );
        let shown = lines.join("\n");
        assert!(shown.contains("connection refused"), "why: {shown}");
        assert!(
            shown.contains("Press r to try again"),
            "what to press: {shown}"
        );
    }

    #[test]
    fn a_long_error_wraps_instead_of_being_cut() {
        let mut state = connected();
        let long = format!("{} the-end", "word ".repeat(30));
        state.step(Event::Page(Err(ApiError::Malformed(long))), NOW);
        assert!(
            text(&state).contains("the-end"),
            "the whole reason is on screen"
        );
    }

    #[test]
    fn an_operator_key_is_named_in_the_status_line() {
        let mut state = connecting();
        state.step(
            Event::KeyInfo(Ok(KeyInfo {
                id: "0123456789abcdef".to_owned(),
                tenant: None,
                scopes: vec!["admin".to_owned()],
            })),
            NOW,
        );
        let lines = screen(&state, 80, 24);
        assert!(
            lines[0].contains("no tenant (operator key)"),
            "the status line: {}",
            lines[0]
        );
        assert!(
            lines
                .join("\n")
                .contains("this is an operator key, which reaches no vCon"),
            "and the list says why it is empty"
        );
    }

    #[test]
    fn a_failed_next_page_shows_its_notice_over_the_rows() {
        let mut state = press(listed(2, Some("cursor-1")), &[Action::More]);
        state.step(
            Event::Page(Err(ApiError::Unreachable("timed out".to_owned()))),
            NOW,
        );
        let lines = screen(&state, 80, 24);
        assert!(
            lines[22].contains("The next page was not read"),
            "the notice: {}",
            lines[22]
        );
        assert!(
            lines[2].contains(&row(0).uuid),
            "the rows stay: {}",
            lines[2]
        );
    }

    #[test]
    fn the_detail_of_a_vcon_no_longer_listed_says_so() {
        let mut state = press(listed(2, None), &[Action::Open, Action::Refresh]);
        state.step(
            Event::Page(Ok(Page {
                vcons: vec![row(1)],
                next: None,
            })),
            NOW,
        );
        let shown = text(&state);
        assert!(
            shown.contains("no longer in the list"),
            "what happened: {shown}"
        );
        assert!(
            shown.contains("Press Esc to go back"),
            "what to press: {shown}"
        );
    }

    #[test]
    fn the_detail_view_shows_a_failed_refresh() {
        let mut state = press(listed(2, None), &[Action::Open, Action::Refresh]);
        state.step(Event::Page(Err(ApiError::Status(503, None))), NOW);
        let shown = text(&state);
        assert!(shown.contains("the server answered 503"), "why: {shown}");
        assert!(
            shown.contains("Press r to try again"),
            "what to press: {shown}"
        );
    }

    #[test]
    fn a_terminal_too_small_says_so_and_how_to_quit() {
        let lines = screen(&listed(3, None), 30, 5);
        let shown = lines.join("\n");
        assert!(shown.contains("too small"), "what is wrong: {shown}");
        assert!(shown.contains("q"), "the way out: {shown}");
        assert!(!shown.contains("01a07da8"), "no half-drawn list");
    }

    #[test]
    fn a_time_that_does_not_parse_is_shown_as_sent() {
        assert_eq!(utc("yesterday"), "yesterday", "nothing is invented for it");
    }

    #[test]
    fn a_refused_key_says_to_check_the_key_file() {
        let mut state = connecting();
        state.step(Event::KeyInfo(Err(ApiError::Status(401, None))), NOW);
        let shown = text(&state);
        assert!(shown.contains("refused the key (401)"), "why: {shown}");
        assert!(shown.contains("key_file"), "where to look: {shown}");
    }

    // Edge.

    #[test]
    fn the_loading_state_says_so_and_how_to_quit() {
        let shown = text(&connected());
        assert!(
            shown.contains("Loading vCons"),
            "what is happening: {shown}"
        );
        assert!(shown.contains("q quit"), "the way out: {shown}");
    }

    #[test]
    fn the_connecting_state_is_in_the_status_line() {
        let lines = screen(&connecting(), 80, 24);
        assert!(
            lines[0].contains("connecting"),
            "the status line: {}",
            lines[0]
        );
        assert!(
            lines.join("\n").contains("Connecting to the server"),
            "and the body"
        );
    }

    #[test]
    fn an_empty_tenant_says_so_and_what_to_press() {
        let shown = text(&listed(0, None));
        assert!(
            shown.contains("This tenant has no vCons"),
            "what happened: {shown}"
        );
        assert!(
            shown.contains("Press r to read again"),
            "what to press: {shown}"
        );
    }

    #[test]
    fn the_detail_view_says_refreshing_while_a_refresh_is_in_flight() {
        let shown = text(&press(listed(2, None), &[Action::Open, Action::Refresh]));
        assert!(
            shown.contains("Reading the list again"),
            "what is happening: {shown}"
        );
    }

    #[test]
    fn at_80_columns_every_list_column_fits() {
        let lines = screen(&listed(1, None), 80, 24);
        assert!(
            lines[2].trim_end().ends_with("2026-10-25 12:30:00"),
            "the last column is whole: {}",
            lines[2]
        );
    }

    #[test]
    fn below_80_columns_the_updated_column_is_dropped_not_cut() {
        let lines = screen(&listed(1, None), 60, 24);
        assert!(
            lines[2].contains("2026-10-25 12:00:00"),
            "stored stays: {}",
            lines[2]
        );
        assert!(
            !lines[2].contains("2026-10-25 12:3"),
            "no part of updated: {}",
            lines[2]
        );
        assert!(
            !lines[1].contains("UPDATED"),
            "nor its header: {}",
            lines[1]
        );
    }

    #[test]
    fn every_size_from_1x1_up_draws_without_a_panic() {
        let states = [
            connecting(),
            listed(0, None),
            listed(60, Some("cursor-1")),
            press(listed(3, None), &[Action::Open]),
            press(listed(3, None), &[Action::Help]),
        ];
        for state in &states {
            for columns in [1, 2, 10, 39, 40, 79, 80, 200] {
                for lines in [1, 2, 7, 8, 24, 60] {
                    let drawn = screen(state, columns, lines);
                    assert_eq!(
                        drawn.len(),
                        usize::from(lines),
                        "{columns}x{lines} drew a frame"
                    );
                }
            }
        }
    }

    #[test]
    fn no_color_and_a_dumb_terminal_turn_color_off() {
        assert_eq!(
            theme(Some("1"), Some("xterm-256color")),
            MONO,
            "NO_COLOR set"
        );
        assert_eq!(theme(None, Some("dumb")), MONO, "a terminal with no color");
        assert_eq!(theme(None, None), MONO, "no TERM at all");
        assert_eq!(
            theme(Some(""), Some("xterm")),
            COLOR,
            "an empty NO_COLOR is not set"
        );
    }

    #[test]
    fn without_color_no_cell_has_a_color() {
        for state in [
            listed(3, None),
            press(listed(3, None), &[Action::Help]),
            connecting(),
        ] {
            let terminal = terminal(&state, 80, 24, MONO);
            let buffer = terminal.backend().buffer();
            for cell in buffer.content() {
                assert_eq!(cell.fg, Color::Reset, "no foreground color");
                assert_eq!(cell.bg, Color::Reset, "no background color");
            }
        }
    }

    #[test]
    fn the_status_line_keeps_the_connection_when_the_address_is_long() {
        let mut state = State::new(
            &format!("https://{}.example.com:8443", "a".repeat(90)),
            (80, 24),
        )
        .0;
        state.step(Event::KeyInfo(Err(ApiError::Status(401, None))), NOW);
        let lines = screen(&state, 80, 24);
        assert!(
            lines[0].contains("not connected"),
            "the state is never pushed off: {}",
            lines[0]
        );
    }

    // The status line at each width. The rule: the connection is never
    // shortened; then the server's address; then the tenant.

    const SERVER: &str = "https://vcon.example.com:8443";
    const LONG_ID: &str = "658a6d102142fea77f47ee8a332323d8";

    /// A state for the server at `server` whose key answered with `tenant`.
    fn keyed(server: &str, tenant: Option<&str>) -> State {
        let mut state = State::new(server, (80, 24)).0;
        state.step(
            Event::KeyInfo(Ok(KeyInfo {
                id: "0123456789abcdef".to_owned(),
                tenant: tenant.map(str::to_owned),
                scopes: vec!["read".to_owned()],
            })),
            NOW,
        );
        state
    }

    /// A state whose server refused the key.
    fn refused(server: &str) -> State {
        let mut state = State::new(server, (80, 24)).0;
        state.step(Event::KeyInfo(Err(ApiError::Status(401, None))), NOW);
        state
    }

    /// The status line of `state` in a terminal of `columns`.
    fn status_at(state: &State, columns: u16) -> String {
        screen(state, columns, 24)[0].trim_end().to_owned()
    }

    #[test]
    fn at_120_columns_the_status_line_is_whole() {
        assert_eq!(
            status_at(&keyed(SERVER, Some(LONG_ID)), 120),
            format!("{SERVER} | tenant {LONG_ID} | connected"),
            "everything fits"
        );
    }

    #[test]
    fn at_80_columns_the_address_loses_only_its_scheme() {
        assert_eq!(
            status_at(&keyed(SERVER, Some(LONG_ID)), 80),
            format!("vcon.example.com:8443 | tenant {LONG_ID} | connected"),
            "https:// says nothing a reader needs: every address has it"
        );
    }

    #[test]
    fn at_60_columns_the_tenant_id_is_shortened_and_the_address_is_not() {
        assert_eq!(
            status_at(&keyed(SERVER, Some(LONG_ID)), 60),
            "vcon.example.com:8443 | tenant 658a6d102142fe... | connected",
            "the tenant gives way before the server does"
        );
    }

    #[test]
    fn at_40_columns_the_tenant_is_left_out_and_the_address_is_not() {
        assert_eq!(
            status_at(&keyed(SERVER, Some(LONG_ID)), 40),
            "vcon.example.com:8443 | connected",
            "the server and the connection"
        );
    }

    #[test]
    fn a_line_that_fits_exactly_keeps_its_scheme() {
        assert_eq!(
            status_at(&keyed(SERVER, Some(LONG_ID)), 83),
            format!("{SERVER} | tenant {LONG_ID} | connected"),
            "83 characters in 83 columns"
        );
    }

    #[test]
    fn a_short_tenant_id_is_whole_at_60_columns() {
        assert_eq!(
            status_at(&keyed(SERVER, Some("t_example")), 60),
            format!("{SERVER} | tenant t_example | connected"),
            "60 characters in 60 columns"
        );
    }

    #[test]
    fn an_address_with_a_path_is_shown_with_it() {
        let line = status_at(
            &keyed("https://vcon.example.com:8443/base", Some("t_example")),
            120,
        );
        assert!(
            line.starts_with("https://vcon.example.com:8443/base | "),
            "the path is part of the address: {line}"
        );
    }

    #[test]
    fn a_refused_key_at_40_columns_shows_the_address_and_not_connected() {
        assert_eq!(
            status_at(&refused(SERVER), 40),
            "vcon.example.com:8443 | not connected",
            "`tenant unknown` is the part left out"
        );
    }

    #[test]
    fn a_refused_key_at_60_columns_keeps_tenant_unknown() {
        assert_eq!(
            status_at(&refused(SERVER), 60),
            "vcon.example.com:8443 | tenant unknown | not connected",
            "without the scheme the three parts fit"
        );
    }

    #[test]
    fn a_refused_key_at_80_columns_is_whole() {
        assert_eq!(
            status_at(&refused(SERVER), 80),
            format!("{SERVER} | tenant unknown | not connected"),
            "everything fits"
        );
    }

    #[test]
    fn connecting_at_40_columns_shows_the_address_and_connecting() {
        assert_eq!(
            status_at(&State::new(SERVER, (40, 24)).0, 40),
            "vcon.example.com:8443 | connecting",
            "`tenant unknown` is the part left out"
        );
    }

    #[test]
    fn an_operator_key_at_60_columns_is_named_whole() {
        assert_eq!(
            status_at(&keyed(SERVER, None), 60),
            "vcon.example.com:8443 | no tenant (operator key) | connected",
            "60 characters in 60 columns"
        );
    }

    #[test]
    fn an_operator_key_at_40_columns_is_left_out_not_cut() {
        assert_eq!(
            status_at(&keyed(SERVER, None), 40),
            "vcon.example.com:8443 | connected",
            "a phrase is whole or absent"
        );
    }

    #[test]
    fn a_long_address_is_cut_at_its_end_and_the_connection_is_whole() {
        let server = format!("https://{}.example.com:8443", "a".repeat(90));
        assert_eq!(
            status_at(&refused(&server), 40),
            format!("{}... | not connected", "a".repeat(21)),
            "40 columns: 21 of the address, the mark, and the connection"
        );
    }

    #[test]
    fn four_characters_of_a_tenant_id_are_the_fewest_shown() {
        assert_eq!(
            status_at(&keyed(SERVER, Some(LONG_ID)), 50),
            "vcon.example.com:8443 | tenant 658a... | connected",
            "50 columns leave room for four"
        );
    }

    #[test]
    fn a_tenant_id_with_room_for_three_characters_is_left_out() {
        assert_eq!(
            status_at(&keyed(SERVER, Some(LONG_ID)), 49),
            "vcon.example.com:8443 | connected",
            "three characters name no tenant"
        );
    }

    #[test]
    fn one_column_short_of_whole_the_scheme_goes_first() {
        assert_eq!(
            status_at(&keyed(SERVER, Some(LONG_ID)), 82),
            format!("vcon.example.com:8443 | tenant {LONG_ID} | connected"),
            "the tenant id stays whole"
        );
    }

    #[test]
    fn one_column_short_of_a_whole_id_shortens_it_to_fit() {
        let line = status_at(&keyed(SERVER, Some(LONG_ID)), 74);
        assert_eq!(
            line,
            format!(
                "vcon.example.com:8443 | tenant {}... | connected",
                &LONG_ID[..28]
            ),
            "74 columns: 28 characters and the mark"
        );
    }

    #[test]
    fn an_address_that_is_not_https_is_shown_as_it_is() {
        assert_eq!(
            status_at(&refused("vcon.example.com:8443"), 40),
            "vcon.example.com:8443 | not connected",
            "there is no scheme to leave out"
        );
    }

    #[test]
    fn an_address_that_fits_exactly_at_40_columns_is_whole() {
        let server = format!("https://{}", "b".repeat(28));
        assert_eq!(
            status_at(&keyed(&server, Some(LONG_ID)), 40),
            format!("{} | connected", "b".repeat(28)),
            "28 characters, the separator and the connection are 40"
        );
    }

    #[test]
    fn the_status_line_fits_and_ends_with_the_connection_at_every_width() {
        let long = format!("https://{}.example.com:8443", "a".repeat(90));
        for server in [SERVER, long.as_str()] {
            let states = [
                (State::new(server, (80, 24)).0, "connecting"),
                (refused(server), "not connected"),
                (keyed(server, Some(LONG_ID)), "connected"),
                (keyed(server, None), "connected"),
            ];
            for (state, word) in &states {
                for columns in 40..=130 {
                    let line = status_at(state, columns);
                    assert!(
                        line.chars().count() <= usize::from(columns),
                        "{columns} columns: {line}"
                    );
                    assert!(
                        line.ends_with(&format!(" | {word}")),
                        "{columns} columns, the connection is whole and last: {line}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_tenant_phrase_is_whole_or_absent_at_every_width() {
        for (state, phrase) in [
            (refused(SERVER), "tenant unknown"),
            (keyed(SERVER, None), "no tenant (operator key)"),
        ] {
            for columns in 40..=130 {
                let line = status_at(&state, columns);
                assert!(
                    line.contains(phrase) || !line.contains("tenant"),
                    "{columns} columns: {line}"
                );
            }
        }
    }

    #[test]
    fn the_help_view_states_the_time_zone() {
        let shown = text(&press(listed(1, None), &[Action::Help]));
        assert!(
            shown.contains("UTC+00:00"),
            "times are UTC, stated: {shown}"
        );
    }

    // The last two lines in a narrow terminal.

    /// The last line of the screen: the key hints.
    fn footer_at(state: &State, columns: u16) -> String {
        screen(state, columns, 24)[23].trim_end().to_owned()
    }

    #[test]
    fn at_40_columns_the_list_footer_still_shows_help_and_quit() {
        let footer = footer_at(&listed(2, None), 40);
        assert!(
            footer.ends_with("? help  q quit"),
            "how to get help and how to leave are whole: {footer:?}"
        );
        assert!(
            footer.starts_with("j down  k up"),
            "the first keys stay first: {footer:?}"
        );
    }

    #[test]
    fn at_40_columns_the_detail_footer_still_shows_help_and_quit() {
        let footer = footer_at(&press(listed(2, None), &[Action::Open]), 40);
        assert!(
            footer.ends_with("? help  q quit"),
            "how to get help and how to leave are whole: {footer:?}"
        );
        assert!(
            footer.starts_with("Esc back"),
            "how to go back stays first: {footer:?}"
        );
    }

    #[test]
    fn at_80_columns_the_footers_are_whole() {
        assert_eq!(
            footer_at(&listed(2, None), 80),
            "j down  k up  Enter open  r refresh  n more  ? help  q quit",
            "the list's keys"
        );
        assert_eq!(
            footer_at(&press(listed(2, None), &[Action::Open]), 80),
            "Esc back  j down  k up  r refresh  ? help  q quit",
            "the detail's keys"
        );
        assert_eq!(
            footer_at(&press(listed(2, None), &[Action::Help]), 80),
            "Esc back  q quit",
            "help's keys"
        );
    }

    #[test]
    fn no_footer_ends_in_a_cut_hint_at_any_width() {
        for view in [&[][..], &[Action::Open][..], &[Action::Help][..]] {
            let state = press(listed(2, None), view);
            for columns in 40..=100 {
                let footer = footer_at(&state, columns);
                assert!(
                    footer.ends_with("q quit"),
                    "at {columns} columns the last hint is whole: {footer:?}"
                );
                assert!(
                    footer.chars().count() <= usize::from(columns),
                    "at {columns} columns the footer fits: {footer:?}"
                );
            }
        }
    }

    #[test]
    fn one_column_short_of_the_whole_list_footer_drops_one_hint() {
        assert_eq!(
            footer_at(&listed(2, None), 58),
            "j down  k up  Enter open  r refresh  ? help  q quit",
            "the hint before help goes first"
        );
        assert_eq!(
            footer_at(&listed(2, None), 59),
            "j down  k up  Enter open  r refresh  n more  ? help  q quit",
            "at its own width the footer is whole"
        );
    }

    /// The text of the lines from `first` on, without the key hints, joined
    /// as a reader reads a wrapped sentence.
    fn joined(lines: &[String], first: usize) -> String {
        lines[first..lines.len().saturating_sub(1)]
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_notice_longer_than_the_terminal_is_wrapped_not_cut() {
        let mut state = press(listed(2, Some("cursor-1")), &[Action::More]);
        state.step(
            Event::Page(Err(ApiError::Unreachable(
                "error sending request for url (https://vcon.example.com:8443/vcons): client error (Connect): tcp connect error: Connection refused (os error 111)".to_owned(),
            ))),
            NOW,
        );
        let lines = screen(&state, 80, 24);
        let said = joined(&lines, 19);
        assert!(
            said.contains("The next page was not read: the server did not answer: ")
                && said.ends_with("Connection refused (os error 111)."),
            "the whole sentence is on screen, to its end: {said}"
        );
        assert!(
            lines[2].contains(&row(0).uuid),
            "the rows stay: {}",
            lines[2]
        );
    }

    #[test]
    fn a_short_notice_takes_one_line() {
        let mut state = press(listed(2, Some("cursor-1")), &[Action::More]);
        state.step(
            Event::Page(Err(ApiError::Unreachable("timed out".to_owned()))),
            NOW,
        );
        let lines = screen(&state, 80, 24);
        assert!(
            lines[22].starts_with("The next page was not read"),
            "on the line above the key hints: {}",
            lines[22]
        );
        assert_eq!(lines[21].trim(), "", "and the line above it is empty");
    }

    #[test]
    fn at_60_columns_the_position_line_keeps_when_the_list_was_read() {
        let lines = screen(&listed(2, Some("cursor-1")), 60, 24);
        let said = joined(&lines, 20);
        assert!(
            said.contains("1 of 2 read, more on the server")
                && said.ends_with("as of 2026-11-10 01:20:00 UTC+00:00"),
            "the position and the time are whole: {said}"
        );
    }

    // A failure whose remedy is in a file the UI read at start.

    /// What `r` cannot do, as the screen says it.
    const AT_START: &str =
        "A change to the client file or the key file takes effect at the next start.";

    #[test]
    fn a_refused_key_says_that_a_new_key_file_needs_a_new_start() {
        let shown = text(&shown_refusal(ApiError::Status(401, None)));
        assert!(
            shown.contains(AT_START),
            "the token is read once, so r cannot read a corrected key file: {shown}"
        );
    }

    #[test]
    fn a_refused_certificate_says_that_a_new_ca_needs_a_new_start() {
        let advice =
            client::certificate_advice(&rustls::CertificateError::UnknownIssuer).expect("advice");
        let shown = text(&shown_refusal(ApiError::Untrusted(advice.to_owned())));
        assert!(
            shown
                .replace('\n', " ")
                .contains("set ca in the client file")
                && shown.contains(AT_START),
            "the advice names the client file, which is read once: {shown}"
        );
    }

    #[test]
    fn a_key_that_may_not_list_says_that_another_key_needs_a_new_start() {
        for state in [
            shown_key(None, &[]),
            shown_key(Some(SHOWN_TENANT), &["write"]),
        ] {
            let shown = text(&state);
            assert!(
                shown.contains(AT_START),
                "another key is another key file, read at start: {shown}"
            );
        }
    }

    #[test]
    fn a_failure_still_names_r_and_q() {
        let shown = text(&shown_refusal(client::no_answer(client::TIMEOUT)));
        assert!(
            shown.contains("Press r to try again, or q to quit."),
            "a server that did not answer may answer the next request: {shown}"
        );
    }

    #[test]
    fn an_empty_tenant_does_not_mention_the_files() {
        let shown = text(&shown_page(
            shown_key(Some(SHOWN_TENANT), &["read"]),
            0..0,
            false,
        ));
        assert!(
            !shown.contains(AT_START),
            "nothing failed, so there is nothing to correct: {shown}"
        );
    }

    // The screens the site shows.

    /// The file the terminal UI's page and the home page draw their screens
    /// from: `tui_screen` in website/templates/components.html.
    fn screens_file() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../website/data/tui-screens.json")
    }

    /// A tenant's first page of 100 vCons, newest first, with more on the
    /// server, and the third one selected: the list the site shows.
    fn shown_list() -> State {
        let state = shown_page(shown_key(Some(SHOWN_TENANT), &["read"]), 0..100, true);
        press(state, &[Action::Down, Action::Down])
    }

    /// The tenant of the screens the site shows.
    const SHOWN_TENANT: &str = "8c41f2d9a07e4b5c9d13e6f0a2b4c7d8";

    /// The state once the server has answered for a key of `tenant` (an
    /// operator key has none) that holds `scopes`.
    fn shown_key(tenant: Option<&str>, scopes: &[&str]) -> State {
        let mut state = connecting();
        state.step(
            Event::KeyInfo(Ok(KeyInfo {
                id: "5b0c3f9a1d2e4f60".to_owned(),
                tenant: tenant.map(str::to_owned),
                scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
            })),
            NOW,
        );
        state
    }

    /// `state` after a page with the vCons `rows` arrived, with `more` on
    /// the server or not.
    fn shown_page(mut state: State, rows: std::ops::Range<u64>, more: bool) -> State {
        state.step(
            Event::Page(Ok(Page {
                vcons: rows.map(shown_row).collect(),
                next: more.then(|| "cursor-2".to_owned()),
            })),
            NOW,
        );
        state
    }

    /// The state at start when the server's answer for the key is `error`.
    fn shown_refusal(error: ApiError) -> State {
        let mut state = connecting();
        state.step(Event::KeyInfo(Err(error)), NOW);
        state
    }

    /// Every screen the site shows: its name in the file, the state drawn,
    /// and the terminal's columns and lines.
    fn shown_screens() -> Vec<(&'static str, State, u16, u16)> {
        let reader = || shown_key(Some(SHOWN_TENANT), &["read"]);
        let no_answer = || client::no_answer(client::TIMEOUT);
        let mut next_page_failed = press(shown_list(), &[Action::More]);
        next_page_failed.step(Event::Page(Err(no_answer())), NOW);
        // The vCon open in the detail view is deleted; r reads a list
        // without it.
        let detail_gone = shown_page(
            press(shown_list(), &[Action::Open, Action::Refresh]),
            3..100,
            true,
        );
        let unknown_issuer = client::certificate_advice(&rustls::CertificateError::UnknownIssuer)
            .expect("advice for a certificate no trusted authority signed");
        vec![
            ("list", shown_list(), 80, 24),
            ("detail", press(shown_list(), &[Action::Open]), 80, 24),
            ("help", press(shown_list(), &[Action::Help]), 80, 24),
            ("list-end", shown_page(reader(), 0..3, false), 80, 10),
            (
                "more-read",
                shown_page(press(shown_list(), &[Action::More]), 100..200, false),
                80,
                10,
            ),
            ("empty", shown_page(reader(), 0..0, false), 80, 10),
            ("not-connected", shown_refusal(no_answer()), 80, 10),
            (
                "refused-key",
                shown_refusal(ApiError::Status(401, None)),
                80,
                10,
            ),
            (
                "refused-certificate",
                shown_refusal(ApiError::Untrusted(unknown_issuer.to_owned())),
                80,
                10,
            ),
            ("operator-key", shown_key(None, &[]), 80, 10),
            ("next-page-failed", next_page_failed, 80, 10),
            ("detail-gone", detail_gone, 80, 10),
            ("narrow", shown_list(), 60, 10),
            ("narrowest", shown_list(), 40, 10),
            ("wide", shown_list(), 120, 10),
            ("too-small", shown_list(), 36, 6),
        ]
    }

    /// The `n`th vCon of the shown list: a version 8 uuid made from `n`,
    /// stored 37 minutes before the one above it, some updated since.
    fn shown_row(n: u64) -> Row {
        let mix = n
            .wrapping_add(1)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15)
            .rotate_left(17);
        let minutes = i64::try_from(n).expect("a small number").saturating_mul(37);
        let stored = NOW
            - Duration::minutes(minutes.saturating_add(11))
            - Duration::seconds(i64::try_from(mix % 60).expect("under 60"));
        let updated =
            stored + Duration::minutes(i64::try_from(n % 4).expect("under 4").saturating_mul(6));
        let time = |at: OffsetDateTime| at.format(&Rfc3339).expect("an RFC 3339 time");
        Row {
            uuid: format!(
                "{:08x}-{:04x}-8{:03x}-b{:03x}-{:012x}",
                mix >> 32,
                (mix >> 16) & 0xffff,
                mix & 0xfff,
                (mix >> 20) & 0xfff,
                mix.rotate_left(29) & 0xffff_ffff_ffff
            ),
            stored_at: time(stored),
            updated_at: time(updated),
            version: format!("{mix:016x}{:016x}", mix.rotate_left(7)),
        }
    }

    /// The class names website/sass/style.scss styles a cell with.
    fn classes(cell: &Cell) -> String {
        assert_eq!(cell.bg, Color::Reset, "the site draws no background color");
        let mut out = Vec::new();
        if cell.modifier.contains(Modifier::BOLD) {
            out.push("bold");
        }
        if cell.modifier.contains(Modifier::REVERSED) {
            out.push("reversed");
        }
        match cell.fg {
            Color::Reset => {}
            Color::Green => out.push("green"),
            Color::Yellow => out.push("yellow"),
            Color::Red => out.push("red"),
            other => panic!("the site has no class for {other:?}"),
        }
        out.join(" ")
    }

    /// The screen, in color, in a terminal of `columns` by `rows`: each line as runs of text that
    /// share one style, without the plain blanks that end a line.
    fn styled(state: &State, columns: u16, rows: u16) -> serde_json::Value {
        let terminal = terminal(state, columns, rows, COLOR);
        let buffer = terminal.backend().buffer();
        let lines: Vec<serde_json::Value> = (0..rows)
            .map(|y| {
                let mut runs: Vec<(String, String)> = Vec::new();
                for x in 0..columns {
                    let cell = &buffer[(x, y)];
                    let class = classes(cell);
                    match runs.last_mut() {
                        Some((last, text)) if *last == class => text.push_str(cell.symbol()),
                        _ => runs.push((class, cell.symbol().to_owned())),
                    }
                }
                if let Some((class, text)) = runs.last_mut()
                    && class.is_empty()
                {
                    text.truncate(text.trim_end().len());
                }
                runs.retain(|(_, text)| !text.is_empty());
                runs.into_iter()
                    .map(|(class, text)| serde_json::json!({"class": class, "text": text}))
                    .collect()
            })
            .collect();
        serde_json::json!({"columns": columns, "lines": lines})
    }

    #[test]
    fn the_site_shows_the_screens_draw_makes() {
        let mut made = serde_json::Map::new();
        for (name, state, columns, rows) in shown_screens() {
            let before = made.insert(name.to_owned(), styled(&state, columns, rows));
            assert!(before.is_none(), "the screen {name} is made once");
        }
        let made = format!(
            "{}\n",
            serde_json::to_string_pretty(&serde_json::Value::Object(made)).expect("JSON")
        );
        let path = screens_file();
        if std::env::var_os("VCON_SERVER_WRITE_SCREENS").is_some() {
            std::fs::write(&path, &made)
                .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        }
        let shown = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            shown == made,
            "{} is not the screens draw makes now: run this test with VCON_SERVER_WRITE_SCREENS=1 to write them again",
            path.display()
        );
    }
}
