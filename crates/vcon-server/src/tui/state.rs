//! The terminal UI's state machine: a state and an event give the next
//! state and the requests to make. It draws nothing, reads no clock and
//! calls no server, so every transition can be tested as a value.

use std::fmt;

use serde::Deserialize;
use time::OffsetDateTime;

/// What `GET /key` answers: the key in use.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct KeyInfo {
    /// The key's id. Not the token: an id opens nothing.
    pub id: String,
    /// The key's tenant; `None` for an operator key.
    pub tenant: Option<String>,
    /// The scopes the key holds.
    pub scopes: Vec<String>,
}

/// One vCon of a `GET /vcons` page: the metadata the API lists.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Row {
    /// The vCon's uuid.
    pub uuid: String,
    /// When the uuid was first stored, RFC 3339 in UTC.
    pub stored_at: String,
    /// When the current version was stored, RFC 3339 in UTC.
    pub updated_at: String,
    /// The current version: the `ETag` without quotes.
    pub version: String,
}

/// One page of `GET /vcons`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Page {
    /// The page's vCons, newest first.
    pub vcons: Vec<Row>,
    /// The cursor of the next page; `None` on the last.
    pub next: Option<String>,
}

/// Why a request gave no answer to show. None of these holds the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// No answer arrived: the address, TLS or the network.
    Unreachable(String),
    /// The client refused the server's certificate, so no request was
    /// sent: what is wrong with it and what to do.
    Untrusted(String),
    /// The server answered this status, with the problem's title if it
    /// sent one.
    Status(u16, Option<String>),
    /// The answer was not the JSON the API describes.
    Malformed(String),
}

impl std::error::Error for ApiError {}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Unreachable(why) => write!(f, "the server did not answer: {why}"),
            ApiError::Untrusted(why) => {
                write!(f, "the server's certificate was refused: {why}")
            }
            ApiError::Status(401, _) => f.write_str("the server refused the key (401)"),
            ApiError::Status(status, Some(title)) => {
                write!(f, "the server answered {status} ({title})")
            }
            ApiError::Status(status, None) => write!(f, "the server answered {status}"),
            ApiError::Malformed(why) => {
                write!(f, "the server's answer could not be read: {why}")
            }
        }
    }
}

/// Whether the server has answered for the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    /// `GET /key` is in flight.
    Connecting,
    /// The server answered for this key.
    Connected(KeyInfo),
    /// The server did not answer, or refused the key; why.
    Failed(String),
}

/// The state of the list of vCons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listing {
    /// The first page is in flight.
    Loading,
    /// The rows are what the server last answered.
    Ready,
    /// There is no list to show; why.
    Failed(String),
}

/// Which view is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The tenant's vCons.
    List,
    /// One vCon's metadata.
    Detail,
    /// The key bindings.
    Help,
}

/// What a key press asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Select the row above.
    Up,
    /// Select the row below.
    Down,
    /// Select the first row.
    Top,
    /// Select the last row.
    Bottom,
    /// Move up by the rows on screen.
    PageUp,
    /// Move down by the rows on screen.
    PageDown,
    /// Open the selected vCon.
    Open,
    /// Leave the detail or help view.
    Back,
    /// Read the list again from the server.
    Refresh,
    /// Read the next page.
    More,
    /// Open or close help.
    Help,
    /// End the program.
    Quit,
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A key press, as its action.
    Action(Action),
    /// The terminal is now this many columns and lines.
    Resize(u16, u16),
    /// `GET /key` finished.
    KeyInfo(Result<KeyInfo, ApiError>),
    /// `GET /vcons` finished.
    Page(Result<Page, ApiError>),
}

/// A request for the caller to make; its answer comes back as an [`Event`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// `GET /key`.
    FetchKey,
    /// `GET /vcons`, from this cursor or from the start.
    FetchPage(Option<String>),
}

/// The lines a frame takes around the list's rows: the status line, the
/// column header, a blank line, the notice line and the key hints.
const FRAME_LINES: u16 = 5;

/// Everything the UI shows. It never holds the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// The server's address, as the client file gives it.
    pub server: String,
    /// Whether the server has answered for the key.
    pub connection: Connection,
    /// The state of the list.
    pub listing: Listing,
    /// The vCons read so far, newest first.
    pub rows: Vec<Row>,
    /// The cursor of the next page, if there is one.
    pub next: Option<String>,
    /// The selected row's index in `rows`.
    pub selected: usize,
    /// The view drawn.
    pub view: View,
    /// The view help was opened from.
    pub behind_help: View,
    /// The uuid the detail view shows.
    pub detail: Option<String>,
    /// Whether a next page is in flight.
    pub more: bool,
    /// When the rows were last read from the server.
    pub as_of: Option<OffsetDateTime>,
    /// What went wrong with the last request, when the list is still shown.
    pub notice: Option<String>,
    /// The terminal's columns and lines.
    pub size: (u16, u16),
    /// Whether the program should end.
    pub quit: bool,
}

impl State {
    /// The state at start for the server at `server` in a terminal of
    /// `size` columns and lines, and the first request.
    #[must_use]
    pub fn new(server: &str, size: (u16, u16)) -> (Self, Vec<Effect>) {
        let state = Self {
            server: server.to_owned(),
            connection: Connection::Connecting,
            listing: Listing::Loading,
            rows: Vec::new(),
            next: None,
            selected: 0,
            view: View::List,
            behind_help: View::List,
            detail: None,
            more: false,
            as_of: None,
            notice: None,
            size,
            quit: false,
        };
        (state, vec![Effect::FetchKey])
    }

    /// The rows of the list a terminal of `lines` lines shows: at least 1.
    #[must_use]
    pub fn list_rows(lines: u16) -> u16 {
        lines.saturating_sub(FRAME_LINES).max(1)
    }

    /// The row the detail view shows, if the list still holds it.
    #[must_use]
    pub fn detail_row(&self) -> Option<&Row> {
        let uuid = self.detail.as_deref()?;
        self.rows.iter().find(|row| row.uuid == uuid)
    }

    /// Whether a request is in flight, so another would race it.
    fn busy(&self) -> bool {
        self.more
            || self.connection == Connection::Connecting
            || (self.listing == Listing::Loading
                && matches!(self.connection, Connection::Connected(_)))
    }

    /// Whether a press of the key for `action` can do anything in this
    /// state. The footer names only keys that act.
    #[must_use]
    pub fn acts(&self, action: Action) -> bool {
        match (self.view, action) {
            (_, Action::Quit | Action::Help) | (View::Help | View::Detail, Action::Back) => true,
            (View::Help, _) | (View::List, Action::Back) | (View::Detail, Action::Open) => false,
            (_, Action::Refresh) => !self.busy() && self.key_refusal().is_none(),
            (_, Action::More) => {
                !self.busy() && self.listing == Listing::Ready && self.next.is_some()
            }
            (View::List, Action::Open) => self.rows_shown() && !self.rows.is_empty(),
            (
                view,
                Action::Up
                | Action::Down
                | Action::Top
                | Action::Bottom
                | Action::PageUp
                | Action::PageDown,
            ) => {
                // In a detail view whose vCon left the list, a key that
                // moves shows the selected row, even when it is the only one.
                let reselects =
                    view == View::Detail && !self.rows.is_empty() && self.detail_row().is_none();
                self.rows_shown() && (self.rows.len() > 1 || reselects)
            }
        }
    }

    /// Whether the screen shows the rows, or the detail of one: the list
    /// is read. Otherwise a message stands in their place, and there is no
    /// selection to move or open. While a connection is being made the
    /// list is being read too, so the listing alone decides.
    #[must_use]
    pub fn rows_shown(&self) -> bool {
        self.listing == Listing::Ready
    }

    /// Why the key the server answered for may not list vCons, if it may
    /// not. The key is read once, at start, so no request changes this.
    #[must_use]
    pub fn key_refusal(&self) -> Option<&'static str> {
        match &self.connection {
            Connection::Connected(info) => refusal(info),
            Connection::Connecting | Connection::Failed(_) => None,
        }
    }

    /// Applies `event`, which happened at `now`, and returns the requests
    /// to make.
    pub fn step(&mut self, event: Event, now: OffsetDateTime) -> Vec<Effect> {
        match event {
            Event::Action(action) => {
                self.notice = None;
                self.act(action)
            }
            Event::Resize(columns, lines) => {
                self.size = (columns, lines);
                Vec::new()
            }
            Event::KeyInfo(Ok(info)) => self.connected(info),
            Event::KeyInfo(Err(error)) => {
                self.connection = Connection::Failed(error.to_string());
                self.listing = Listing::Failed(error.to_string());
                Vec::new()
            }
            Event::Page(Ok(page)) => {
                self.page(page, now);
                Vec::new()
            }
            Event::Page(Err(error)) => {
                self.page_failed(&error);
                Vec::new()
            }
        }
    }

    /// The key answered: asks for the first page if the key may list.
    fn connected(&mut self, info: KeyInfo) -> Vec<Effect> {
        let refusal = refusal(&info);
        self.connection = Connection::Connected(info);
        match refusal {
            Some(why) => {
                self.listing = Listing::Failed(why.to_owned());
                Vec::new()
            }
            None => {
                self.listing = Listing::Loading;
                vec![Effect::FetchPage(None)]
            }
        }
    }

    /// A page arrived: a next page is appended, a first page replaces the
    /// rows and the selection follows the vCon it was on.
    fn page(&mut self, page: Page, now: OffsetDateTime) {
        if self.more {
            self.rows.extend(page.vcons);
        } else {
            let on = self
                .detail
                .clone()
                .or_else(|| self.rows.get(self.selected).map(|row| row.uuid.clone()));
            self.rows = page.vcons;
            if let Some(index) =
                on.and_then(|uuid| self.rows.iter().position(|row| row.uuid == uuid))
            {
                self.selected = index;
            }
        }
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
        self.next = page.next;
        self.more = false;
        self.listing = Listing::Ready;
        self.as_of = Some(now);
    }

    /// A page did not arrive: a next page leaves the rows and a notice, a
    /// first page leaves an error in place of rows that may be out of date.
    /// A server that did not answer, or that no longer takes the key (401),
    /// is not connected, as at start, so the next refresh asks for the key
    /// again.
    fn page_failed(&mut self, error: &ApiError) {
        if matches!(
            error,
            ApiError::Unreachable(_) | ApiError::Untrusted(_) | ApiError::Status(401, _)
        ) {
            self.connection = Connection::Failed(error.to_string());
        }
        if self.more {
            self.more = false;
            self.notice = Some(format!("The next page was not read: {error}."));
        } else {
            self.rows.clear();
            self.next = None;
            self.selected = 0;
            self.as_of = None;
            self.listing = Listing::Failed(error.to_string());
        }
    }

    /// Applies a key press.
    fn act(&mut self, action: Action) -> Vec<Effect> {
        match (self.view, action) {
            (_, Action::Quit) => self.quit = true,
            (View::Help, Action::Help | Action::Back) => self.view = self.behind_help,
            (View::Help, _) | (View::List, Action::Back) | (View::Detail, Action::Open) => {}
            (view, Action::Help) => {
                self.behind_help = view;
                self.view = View::Help;
            }
            (View::Detail, Action::Back) => self.view = View::List,
            (_, Action::Refresh) => return self.refresh(),
            (_, Action::More) => return self.more(),
            // A message stands in place of the rows: nothing to open or
            // to move from.
            (
                _,
                Action::Open
                | Action::Up
                | Action::Down
                | Action::Top
                | Action::Bottom
                | Action::PageUp
                | Action::PageDown,
            ) if !self.rows_shown() => {}
            (View::List, Action::Open) => {
                if let Some(row) = self.rows.get(self.selected) {
                    self.detail = Some(row.uuid.clone());
                    self.view = View::Detail;
                }
            }
            (
                view,
                Action::Up
                | Action::Down
                | Action::Top
                | Action::Bottom
                | Action::PageUp
                | Action::PageDown,
            ) => {
                self.travel(action);
                if view == View::Detail
                    && let Some(row) = self.rows.get(self.selected)
                {
                    self.detail = Some(row.uuid.clone());
                }
            }
        }
        Vec::new()
    }

    /// Moves the selection, staying within the rows.
    fn travel(&mut self, action: Action) {
        let last = self.rows.len().saturating_sub(1);
        let page = usize::from(Self::list_rows(self.size.1));
        self.selected = match action {
            Action::Up => self.selected.saturating_sub(1),
            Action::Down => self.selected.saturating_add(1),
            Action::Top => 0,
            Action::Bottom => last,
            Action::PageUp => self.selected.saturating_sub(page),
            Action::PageDown => self.selected.saturating_add(page),
            _ => self.selected,
        }
        .min(last);
    }

    /// Reads from the start again: the key first if the server had not
    /// answered for it, the first page otherwise.
    fn refresh(&mut self) -> Vec<Effect> {
        if self.busy() {
            return Vec::new();
        }
        if let Connection::Connected(info) = &self.connection {
            return self.connected(info.clone());
        }
        self.connection = Connection::Connecting;
        self.listing = Listing::Loading;
        vec![Effect::FetchKey]
    }

    /// Reads the next page, if there is one and nothing is in flight.
    fn more(&mut self) -> Vec<Effect> {
        if self.busy() || self.listing != Listing::Ready {
            return Vec::new();
        }
        let Some(cursor) = self.next.clone() else {
            return Vec::new();
        };
        self.more = true;
        vec![Effect::FetchPage(Some(cursor))]
    }
}

/// Why a key may not list vCons, if it may not: an operator key reaches no
/// tenant, and a tenant key needs the read scope.
fn refusal(info: &KeyInfo) -> Option<&'static str> {
    if info.tenant.is_none() {
        Some("this is an operator key, which reaches no vCon: use a tenant key with the read scope")
    } else if info.scopes.iter().all(|scope| scope != "read") {
        Some("this key lacks the read scope: use a tenant key that holds it")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;
    use time::macros::datetime;

    use super::{
        Action, ApiError, Connection, Effect, Event, KeyInfo, Listing, Page, Row, State, View,
    };

    const NOW: OffsetDateTime = datetime!(2026-11-10 01:20:00 UTC);

    fn row(n: u8) -> Row {
        Row {
            uuid: format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02}"),
            stored_at: "2026-10-25T12:00:00.123456789Z".to_owned(),
            updated_at: "2026-10-25T12:30:00Z".to_owned(),
            version: format!("4f0c4b0a1d2e3f405162738495a6b7{n:02}"),
        }
    }

    fn reader() -> KeyInfo {
        KeyInfo {
            id: "0123456789abcdef".to_owned(),
            tenant: Some("t_example".to_owned()),
            scopes: vec!["read".to_owned()],
        }
    }

    fn page(rows: std::ops::Range<u8>, next: Option<&str>) -> Page {
        Page {
            vcons: rows.map(row).collect(),
            next: next.map(str::to_owned),
        }
    }

    /// A state that is connected and holds `rows` rows of one page.
    fn listed(rows: u8, next: Option<&str>) -> State {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        state.step(Event::KeyInfo(Ok(reader())), NOW);
        state.step(Event::Page(Ok(page(0..rows, next))), NOW);
        state
    }

    fn press(state: &mut State, action: Action) -> Vec<Effect> {
        state.step(Event::Action(action), NOW)
    }

    // Success.

    #[test]
    fn a_new_state_is_connecting_and_asks_for_the_key() {
        let (state, effects) = State::new("https://vcon.example.com:8443", (80, 24));
        assert_eq!(
            state.connection,
            Connection::Connecting,
            "nothing has answered yet"
        );
        assert_eq!(
            effects,
            [Effect::FetchKey],
            "the key's tenant and scopes come first"
        );
    }

    #[test]
    fn a_reader_key_connects_and_asks_for_the_first_page() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        let effects = state.step(Event::KeyInfo(Ok(reader())), NOW);
        assert_eq!(
            state.connection,
            Connection::Connected(reader()),
            "the key answered"
        );
        assert_eq!(state.listing, Listing::Loading, "the page is on its way");
        assert_eq!(
            effects,
            [Effect::FetchPage(None)],
            "the first page has no cursor"
        );
    }

    #[test]
    fn a_page_fills_the_list_and_records_when() {
        let state = listed(3, Some("cursor-1"));
        assert_eq!(state.listing, Listing::Ready, "the page arrived");
        assert_eq!(state.rows.len(), 3, "every row of the page is kept");
        assert_eq!(
            state.next.as_deref(),
            Some("cursor-1"),
            "the cursor is kept for more"
        );
        assert_eq!(
            state.as_of,
            Some(NOW),
            "the clock passed in is when it was read"
        );
    }

    #[test]
    fn down_and_up_move_the_selection() {
        let mut state = listed(3, None);
        press(&mut state, Action::Down);
        press(&mut state, Action::Down);
        assert_eq!(state.selected, 2, "two rows down");
        press(&mut state, Action::Up);
        assert_eq!(state.selected, 1, "one row back up");
    }

    #[test]
    fn bottom_and_top_jump_to_the_ends() {
        let mut state = listed(5, None);
        press(&mut state, Action::Bottom);
        assert_eq!(state.selected, 4, "the last row");
        press(&mut state, Action::Top);
        assert_eq!(state.selected, 0, "the first row");
    }

    #[test]
    fn open_shows_the_selected_vcon_and_back_returns() {
        let mut state = listed(3, None);
        press(&mut state, Action::Down);
        let effects = press(&mut state, Action::Open);
        assert_eq!(state.view, View::Detail, "the detail view is open");
        assert_eq!(
            state.detail,
            Some(row(1).uuid),
            "on the row that was selected"
        );
        assert_eq!(
            effects,
            [],
            "the detail is drawn from the list row: no request"
        );
        press(&mut state, Action::Back);
        assert_eq!(state.view, View::List, "back at the list");
    }

    #[test]
    fn more_asks_for_the_next_page_and_appends_it() {
        let mut state = listed(2, Some("cursor-1"));
        let effects = press(&mut state, Action::More);
        assert_eq!(
            effects,
            [Effect::FetchPage(Some("cursor-1".to_owned()))],
            "the cursor goes back unchanged"
        );
        state.step(Event::Page(Ok(page(2..4, None))), NOW);
        assert_eq!(state.rows.len(), 4, "the second page follows the first");
        assert_eq!(state.next, None, "the last page has no cursor");
    }

    #[test]
    fn refresh_asks_for_the_first_page_again_and_replaces_the_rows() {
        let mut state = listed(3, None);
        let effects = press(&mut state, Action::Refresh);
        assert_eq!(effects, [Effect::FetchPage(None)], "from the start");
        assert_eq!(
            state.listing,
            Listing::Loading,
            "loading is shown meanwhile"
        );
        state.step(Event::Page(Ok(page(0..1, None))), NOW);
        assert_eq!(state.rows.len(), 1, "the new page replaces the old rows");
    }

    #[test]
    fn help_opens_over_a_view_and_returns_to_it() {
        let mut state = listed(3, None);
        press(&mut state, Action::Open);
        press(&mut state, Action::Help);
        assert_eq!(state.view, View::Help, "help is open");
        press(&mut state, Action::Back);
        assert_eq!(
            state.view,
            View::Detail,
            "back at the view help was opened from"
        );
    }

    #[test]
    fn quit_ends_from_every_view() {
        for open in [None, Some(Action::Open), Some(Action::Help)] {
            let mut state = listed(1, None);
            if let Some(open) = open {
                press(&mut state, open);
            }
            press(&mut state, Action::Quit);
            assert!(state.quit, "quit ends from {:?}", state.view);
        }
    }

    #[test]
    fn page_down_moves_by_the_rows_the_terminal_shows() {
        let mut state = listed(60, None);
        press(&mut state, Action::PageDown);
        assert_eq!(
            state.selected,
            usize::from(State::list_rows(24)),
            "a 24-line terminal pages by its list rows"
        );
        press(&mut state, Action::PageUp);
        assert_eq!(state.selected, 0, "and back");
    }

    #[test]
    fn down_in_the_detail_view_shows_the_next_vcon() {
        let mut state = listed(3, None);
        press(&mut state, Action::Open);
        press(&mut state, Action::Down);
        assert_eq!(
            state.detail,
            Some(row(1).uuid),
            "the detail follows the selection"
        );
    }

    #[test]
    fn a_refresh_keeps_the_selection_on_the_same_vcon() {
        let mut state = listed(3, None);
        press(&mut state, Action::Bottom);
        press(&mut state, Action::Open);
        press(&mut state, Action::Refresh);
        state.step(Event::Page(Ok(page(1..3, None))), NOW);
        assert_eq!(
            state.selected, 1,
            "the vCon moved up one row and the selection with it"
        );
        assert_eq!(
            state.detail,
            Some(row(2).uuid),
            "the detail is still that vCon"
        );
    }

    // Failure.

    #[test]
    fn an_unreachable_server_fails_the_connection_and_the_list() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        let effects = state.step(
            Event::KeyInfo(Err(ApiError::Unreachable("connection refused".to_owned()))),
            NOW,
        );
        assert_eq!(
            effects,
            [],
            "nothing more is asked of a server that did not answer"
        );
        assert!(
            matches!(&state.connection, Connection::Failed(why) if why.contains("connection refused")),
            "the status line says why: {:?}",
            state.connection
        );
        assert!(
            matches!(state.listing, Listing::Failed(_)),
            "the list says so too"
        );
    }

    #[test]
    fn a_refused_key_says_the_status() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        state.step(Event::KeyInfo(Err(ApiError::Status(401, None))), NOW);
        assert!(
            matches!(&state.connection, Connection::Failed(why) if why.contains("401")),
            "the refusal's status is shown: {:?}",
            state.connection
        );
    }

    #[test]
    fn an_operator_key_connects_but_lists_nothing() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        let operator = KeyInfo {
            id: "0123456789abcdef".to_owned(),
            tenant: None,
            scopes: vec!["admin".to_owned()],
        };
        let effects = state.step(Event::KeyInfo(Ok(operator)), NOW);
        assert_eq!(
            effects,
            [],
            "an operator key reaches no vCon, so none is asked for"
        );
        assert!(
            matches!(&state.listing, Listing::Failed(why) if why.contains("operator key")),
            "the list says which key to use: {:?}",
            state.listing
        );
    }

    #[test]
    fn a_key_without_read_lists_nothing() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        let writer = KeyInfo {
            scopes: vec!["write".to_owned()],
            ..reader()
        };
        let effects = state.step(Event::KeyInfo(Ok(writer)), NOW);
        assert_eq!(effects, [], "the list needs read, so it is not asked for");
        assert!(
            matches!(&state.listing, Listing::Failed(why) if why.contains("read scope")),
            "the list names the scope: {:?}",
            state.listing
        );
    }

    #[test]
    fn a_failed_first_page_is_an_error_state() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        state.step(Event::KeyInfo(Ok(reader())), NOW);
        state.step(
            Event::Page(Err(ApiError::Status(503, Some("unavailable".to_owned())))),
            NOW,
        );
        assert!(
            matches!(&state.listing, Listing::Failed(why) if why.contains("503") && why.contains("unavailable")),
            "the status and the problem's title are shown: {:?}",
            state.listing
        );
        assert_eq!(state.as_of, None, "nothing was read, so no time is claimed");
    }

    #[test]
    fn a_failed_next_page_keeps_the_rows_and_leaves_a_notice() {
        let mut state = listed(2, Some("cursor-1"));
        press(&mut state, Action::More);
        state.step(
            Event::Page(Err(ApiError::Unreachable("timed out".to_owned()))),
            NOW,
        );
        assert_eq!(state.rows.len(), 2, "the rows already read stay");
        assert_eq!(state.listing, Listing::Ready, "the list is still usable");
        assert_eq!(
            state.next.as_deref(),
            Some("cursor-1"),
            "the cursor is kept to try again"
        );
        assert!(
            state
                .notice
                .as_deref()
                .is_some_and(|n| n.contains("timed out")),
            "a notice says why: {:?}",
            state.notice
        );
    }

    #[test]
    fn a_failed_refresh_drops_the_old_rows() {
        let mut state = listed(2, None);
        press(&mut state, Action::Refresh);
        state.step(Event::Page(Err(ApiError::Status(500, None))), NOW);
        assert!(
            matches!(state.listing, Listing::Failed(_)),
            "the error is shown"
        );
        assert_eq!(
            state.rows,
            [],
            "rows that may be out of date are not shown as current"
        );
    }

    #[test]
    fn a_malformed_answer_is_an_error_state() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        state.step(Event::KeyInfo(Ok(reader())), NOW);
        state.step(
            Event::Page(Err(ApiError::Malformed("expected value".to_owned()))),
            NOW,
        );
        assert!(
            matches!(&state.listing, Listing::Failed(why) if why.contains("expected value")),
            "what could not be read is shown: {:?}",
            state.listing
        );
    }

    #[test]
    fn refresh_after_a_failed_connection_asks_for_the_key_again() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        state.step(
            Event::KeyInfo(Err(ApiError::Unreachable("refused".to_owned()))),
            NOW,
        );
        let effects = press(&mut state, Action::Refresh);
        assert_eq!(
            effects,
            [Effect::FetchKey],
            "the connection is made again from the start"
        );
        assert_eq!(
            state.connection,
            Connection::Connecting,
            "and shown as such"
        );
    }

    #[test]
    fn a_server_that_stops_answering_is_shown_as_not_connected() {
        let mut state = listed(2, None);
        press(&mut state, Action::Refresh);
        state.step(
            Event::Page(Err(ApiError::Unreachable("connection reset".to_owned()))),
            NOW,
        );
        assert!(
            matches!(state.connection, Connection::Failed(_)),
            "the status line no longer says connected"
        );
    }

    #[test]
    fn a_vcon_gone_after_a_refresh_leaves_the_detail_without_a_row() {
        let mut state = listed(2, None);
        press(&mut state, Action::Open);
        press(&mut state, Action::Refresh);
        state.step(Event::Page(Ok(page(1..2, None))), NOW);
        assert_eq!(
            state.detail,
            Some(row(0).uuid),
            "the detail still names what was open"
        );
        assert_eq!(state.detail_row(), None, "and has no row to draw");
    }

    #[test]
    fn each_api_error_says_what_failed() {
        for (error, text) in [
            (
                ApiError::Unreachable("refused".to_owned()),
                "the server did not answer: refused",
            ),
            (
                ApiError::Status(401, None),
                "the server refused the key (401)",
            ),
            (
                ApiError::Status(403, Some("forbidden".to_owned())),
                "the server answered 403 (forbidden)",
            ),
            (ApiError::Status(500, None), "the server answered 500"),
            (
                ApiError::Malformed("eof".to_owned()),
                "the server's answer could not be read: eof",
            ),
        ] {
            assert_eq!(error.to_string(), text, "{error:?}");
        }
    }

    // Edge.

    #[test]
    fn up_at_the_top_and_down_at_the_bottom_stay() {
        let mut state = listed(2, None);
        press(&mut state, Action::Up);
        assert_eq!(state.selected, 0, "no row above the first");
        press(&mut state, Action::Bottom);
        press(&mut state, Action::Down);
        assert_eq!(state.selected, 1, "no row below the last");
    }

    #[test]
    fn open_on_an_empty_list_does_nothing() {
        let mut state = listed(0, None);
        press(&mut state, Action::Open);
        assert_eq!(state.view, View::List, "there is nothing to open");
        assert_eq!(state.detail, None, "and no vCon is named");
    }

    #[test]
    fn movement_on_an_empty_list_stays_at_zero() {
        let mut state = listed(0, None);
        for action in [Action::Down, Action::Bottom, Action::PageDown, Action::Up] {
            press(&mut state, action);
            assert_eq!(state.selected, 0, "{action:?} on no rows");
        }
    }

    #[test]
    fn more_on_the_last_page_asks_for_nothing() {
        let mut state = listed(2, None);
        assert_eq!(press(&mut state, Action::More), [], "there is no next page");
    }

    #[test]
    fn a_second_refresh_while_loading_asks_for_nothing() {
        let mut state = listed(2, None);
        press(&mut state, Action::Refresh);
        assert_eq!(
            press(&mut state, Action::Refresh),
            [],
            "one request is in flight; a second would race it"
        );
    }

    #[test]
    fn more_while_more_is_loading_asks_for_nothing() {
        let mut state = listed(2, Some("cursor-1"));
        press(&mut state, Action::More);
        assert_eq!(
            press(&mut state, Action::More),
            [],
            "the page is already on its way"
        );
        assert_eq!(
            press(&mut state, Action::Refresh),
            [],
            "and a refresh would race it"
        );
    }

    #[test]
    fn refresh_while_connecting_asks_for_nothing() {
        let (mut state, _) = State::new("https://vcon.example.com:8443", (80, 24));
        assert_eq!(
            press(&mut state, Action::Refresh),
            [],
            "the key request is in flight"
        );
    }

    #[test]
    fn a_resize_changes_only_the_size() {
        let mut state = listed(3, None);
        press(&mut state, Action::Down);
        let before = state.clone();
        let effects = state.step(Event::Resize(132, 43), NOW);
        assert_eq!(effects, [], "a resize asks the server nothing");
        assert_eq!(state.size, (132, 43), "the new size is kept");
        assert_eq!(
            State {
                size: before.size,
                ..state
            },
            before,
            "nothing else changed"
        );
    }

    #[test]
    fn a_shorter_page_after_refresh_clamps_the_selection() {
        let mut state = listed(5, None);
        press(&mut state, Action::Bottom);
        press(&mut state, Action::Refresh);
        state.step(Event::Page(Ok(page(10..12, None))), NOW);
        assert_eq!(
            state.selected, 1,
            "the selection is the last row that exists"
        );
    }

    #[test]
    fn page_down_past_the_end_stops_at_the_last_row() {
        let mut state = listed(3, None);
        press(&mut state, Action::PageDown);
        assert_eq!(state.selected, 2, "the last row, not past it");
    }

    #[test]
    fn a_terminal_too_short_for_a_list_still_pages_by_one() {
        assert_eq!(State::list_rows(0), 1, "never zero: paging would not move");
        assert_eq!(State::list_rows(5), 1, "the frame takes five lines");
        assert_eq!(State::list_rows(24), 19, "24 lines leave 19 for rows");
    }

    #[test]
    fn help_from_help_closes_it() {
        let mut state = listed(1, None);
        press(&mut state, Action::Help);
        press(&mut state, Action::Help);
        assert_eq!(state.view, View::List, "the same key closes help");
    }

    #[test]
    fn a_key_press_clears_the_notice() {
        let mut state = listed(2, Some("cursor-1"));
        press(&mut state, Action::More);
        state.step(
            Event::Page(Err(ApiError::Unreachable("timed out".to_owned()))),
            NOW,
        );
        press(&mut state, Action::Down);
        assert_eq!(
            state.notice, None,
            "the notice is about the last request only"
        );
    }

    #[test]
    fn back_in_the_list_view_does_not_quit() {
        let mut state = listed(1, None);
        press(&mut state, Action::Back);
        assert!(!state.quit, "only the quit keys end the program");
        assert_eq!(state.view, View::List, "and the view stays");
    }

    // A key the server stops accepting while the UI runs.

    #[test]
    fn a_key_refused_at_a_refresh_is_not_connected_as_at_start() {
        let mut state = listed(3, None);
        press(&mut state, Action::Refresh);
        state.step(Event::Page(Err(ApiError::Status(401, None))), NOW);
        assert_eq!(
            state.connection,
            Connection::Failed("the server refused the key (401)".to_owned()),
            "a revoked key is shown as it is when the UI starts with one"
        );
        assert_eq!(
            state.listing,
            Listing::Failed("the server refused the key (401)".to_owned()),
            "and the list says why"
        );
    }

    #[test]
    fn a_refresh_after_a_refused_key_asks_for_the_key_again() {
        let mut state = listed(3, None);
        press(&mut state, Action::Refresh);
        state.step(Event::Page(Err(ApiError::Status(401, None))), NOW);
        assert_eq!(
            press(&mut state, Action::Refresh),
            vec![Effect::FetchKey],
            "the tenant and scopes are read again, not taken from before the refusal"
        );
    }

    #[test]
    fn a_key_refused_at_a_next_page_is_not_connected_and_the_rows_stay() {
        let mut state = listed(3, Some("cursor-1"));
        press(&mut state, Action::More);
        state.step(Event::Page(Err(ApiError::Status(401, None))), NOW);
        assert_eq!(
            state.connection,
            Connection::Failed("the server refused the key (401)".to_owned()),
            "the status line stops saying connected"
        );
        assert_eq!(state.rows.len(), 3, "the rows read before stay");
        assert_eq!(
            state.notice.as_deref(),
            Some("The next page was not read: the server refused the key (401)."),
            "and the notice says why"
        );
    }

    #[test]
    fn a_503_at_a_refresh_leaves_the_connection_as_it_was() {
        let mut state = listed(3, None);
        press(&mut state, Action::Refresh);
        state.step(
            Event::Page(Err(ApiError::Status(503, Some("unavailable".to_owned())))),
            NOW,
        );
        assert!(
            matches!(state.connection, Connection::Connected(_)),
            "the server answered for the key, and may answer the next request"
        );
        assert_eq!(
            press(&mut state, Action::Refresh),
            vec![Effect::FetchPage(None)],
            "so r reads the first page, not the key"
        );
    }

    // Which keys act in a state: `acts` against what a press does.

    const ACTIONS: [Action; 12] = [
        Action::Up,
        Action::Down,
        Action::Top,
        Action::Bottom,
        Action::PageUp,
        Action::PageDown,
        Action::Open,
        Action::Back,
        Action::Refresh,
        Action::More,
        Action::Help,
        Action::Quit,
    ];

    const TRAVEL: [Action; 6] = [
        Action::Up,
        Action::Down,
        Action::Top,
        Action::Bottom,
        Action::PageUp,
        Action::PageDown,
    ];

    fn after(mut state: State, events: Vec<Event>) -> State {
        for event in events {
            state.step(event, NOW);
        }
        state
    }

    fn new() -> State {
        State::new("https://vcon.example.com:8443", (80, 24)).0
    }

    fn key(tenant: Option<&str>, scope: &str) -> Event {
        Event::KeyInfo(Ok(KeyInfo {
            id: "0123456789abcdef".to_owned(),
            tenant: tenant.map(str::to_owned),
            scopes: vec![scope.to_owned()],
        }))
    }

    fn no_answer() -> ApiError {
        ApiError::Unreachable("no answer within 15 seconds".to_owned())
    }

    /// States of every kind the screen can be in, each with a name, in the
    /// list view, the detail view where a row can be opened, and help.
    fn states() -> Vec<(String, State)> {
        let act = Event::Action;
        let mut base: Vec<(&str, State)> = vec![
            ("connecting", new()),
            ("loading", after(new(), vec![Event::KeyInfo(Ok(reader()))])),
            (
                "no answer",
                after(new(), vec![Event::KeyInfo(Err(no_answer()))]),
            ),
            (
                "refused key",
                after(
                    new(),
                    vec![Event::KeyInfo(Err(ApiError::Status(401, None)))],
                ),
            ),
            ("operator key", after(new(), vec![key(None, "admin")])),
            (
                "write key",
                after(new(), vec![key(Some("t_example"), "write")]),
            ),
            (
                "first page failed",
                after(
                    new(),
                    vec![
                        Event::KeyInfo(Ok(reader())),
                        Event::Page(Err(ApiError::Status(503, None))),
                    ],
                ),
            ),
            ("empty", listed(0, None)),
            ("one row", listed(1, None)),
            ("one row, more", listed(1, Some("c"))),
            ("three rows", listed(3, None)),
            ("three rows, more", listed(3, Some("c"))),
            (
                "three rows, second",
                after(listed(3, None), vec![act(Action::Down)]),
            ),
            (
                "three rows, last",
                after(listed(3, Some("c")), vec![act(Action::Bottom)]),
            ),
            (
                "next page out",
                after(listed(3, Some("c")), vec![act(Action::More)]),
            ),
            (
                "next page failed",
                after(
                    listed(3, Some("c")),
                    vec![act(Action::More), Event::Page(Err(no_answer()))],
                ),
            ),
            (
                "refresh out",
                after(listed(3, Some("c")), vec![act(Action::Refresh)]),
            ),
            (
                "refresh failed",
                after(
                    listed(3, None),
                    vec![
                        act(Action::Refresh),
                        Event::Page(Err(ApiError::Status(503, None))),
                    ],
                ),
            ),
            (
                "reconnecting",
                after(
                    listed(3, None),
                    vec![
                        act(Action::Refresh),
                        Event::Page(Err(no_answer())),
                        act(Action::Refresh),
                    ],
                ),
            ),
        ];
        // The vCon open in the detail view leaves the list: three, one and
        // no rows are left.
        for (name, left) in [
            ("gone, three left", 3),
            ("gone, one left", 1),
            ("gone, none left", 0),
        ] {
            let mut state = after(
                listed(5, None),
                vec![act(Action::Bottom), act(Action::Open), act(Action::Refresh)],
            );
            state.step(Event::Page(Ok(page(0..left, None))), NOW);
            assert_eq!(
                state.view,
                View::Detail,
                "{name}: the detail view stays open"
            );
            assert!(
                state.detail_row().is_none(),
                "{name}: its vCon is not listed"
            );
            base.push((name, state));
        }
        let mut all = Vec::new();
        for (name, state) in base {
            let opened = after(state.clone(), vec![act(Action::Open)]);
            if opened.view == View::Detail && state.view != View::Detail {
                let refreshing = after(opened.clone(), vec![act(Action::Refresh)]);
                all.push((
                    format!("{name}, detail, help"),
                    after(opened.clone(), vec![act(Action::Help)]),
                ));
                all.push((format!("{name}, detail, r pressed"), refreshing));
                all.push((format!("{name}, detail"), opened));
            }
            all.push((
                format!("{name}, help"),
                after(state.clone(), vec![act(Action::Help)]),
            ));
            all.push((name.to_owned(), state));
        }
        all
    }

    /// The state after `action`, and the requests it made.
    fn acted(state: &State, action: Action) -> (State, Vec<Effect>) {
        let mut next = state.clone();
        let effects = next.act(action);
        (next, effects)
    }

    #[test]
    fn the_states_tried_cover_every_view_and_kind() {
        let all = states();
        assert!(all.len() >= 50, "{} states are tried", all.len());
        for view in [View::List, View::Detail, View::Help] {
            assert!(
                all.iter().any(|(_, state)| state.view == view),
                "a state in {view:?} is tried"
            );
        }
        let mut names: Vec<&String> = all.iter().map(|(name, _)| name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len(), "each state has its own name");
    }

    #[test]
    fn a_key_that_does_not_act_changes_nothing() {
        for (name, state) in states() {
            for action in ACTIONS {
                if state.acts(action) {
                    continue;
                }
                let (next, effects) = acted(&state, action);
                assert!(
                    next == state && effects.is_empty(),
                    "{name}: {action:?} is said not to act, and a press changed the state or asked the server"
                );
            }
        }
    }

    #[test]
    fn a_key_that_acts_changes_the_state_or_asks_the_server() {
        for (name, state) in states() {
            for action in ACTIONS {
                if !state.acts(action) || TRAVEL.contains(&action) {
                    continue;
                }
                let (next, effects) = acted(&state, action);
                assert!(
                    next != state || !effects.is_empty(),
                    "{name}: {action:?} is said to act, and a press did nothing"
                );
            }
        }
    }

    #[test]
    fn the_keys_that_move_act_when_one_of_them_can_move() {
        for (name, state) in states() {
            let moves = TRAVEL
                .iter()
                .any(|action| acted(&state, *action).0 != state);
            for action in TRAVEL {
                assert_eq!(
                    state.acts(action),
                    moves,
                    "{name}: {action:?} acts when the selection can move ({moves})"
                );
            }
        }
    }

    #[test]
    fn quit_and_help_act_in_every_state() {
        for (name, state) in states() {
            assert!(state.acts(Action::Quit), "{name}: q quits");
            assert!(state.acts(Action::Help), "{name}: ? opens or closes help");
        }
    }

    #[test]
    fn nothing_but_help_back_and_quit_acts_in_the_help_view() {
        for (name, state) in states() {
            if state.view != View::Help {
                continue;
            }
            for action in ACTIONS {
                assert_eq!(
                    state.acts(action),
                    matches!(action, Action::Help | Action::Back | Action::Quit),
                    "{name}: {action:?}"
                );
            }
        }
    }

    #[test]
    fn refresh_does_not_act_while_a_request_is_out() {
        for state in [
            new(),
            after(new(), vec![Event::KeyInfo(Ok(reader()))]),
            after(listed(3, Some("c")), vec![Event::Action(Action::More)]),
            after(listed(3, None), vec![Event::Action(Action::Refresh)]),
        ] {
            assert!(
                !state.acts(Action::Refresh),
                "a second request would race the first: {:?} {:?}",
                state.connection,
                state.listing
            );
        }
    }

    #[test]
    fn refresh_acts_after_a_failure_the_server_made() {
        for state in [
            after(new(), vec![Event::KeyInfo(Err(no_answer()))]),
            after(
                new(),
                vec![Event::KeyInfo(Err(ApiError::Status(401, None)))],
            ),
            after(
                new(),
                vec![
                    Event::KeyInfo(Ok(reader())),
                    Event::Page(Err(ApiError::Status(503, None))),
                ],
            ),
        ] {
            assert!(
                state.acts(Action::Refresh),
                "the server may answer the next request: {:?}",
                state.listing
            );
        }
    }

    #[test]
    fn refresh_does_not_act_for_a_key_that_may_not_list() {
        for event in [key(None, "admin"), key(Some("t_example"), "write")] {
            let mut state = after(new(), vec![event]);
            assert!(
                !state.acts(Action::Refresh),
                "the key is read once, at start: {:?}",
                state.listing
            );
            let before = state.clone();
            assert_eq!(
                press(&mut state, Action::Refresh),
                Vec::new(),
                "nothing is asked of the server"
            );
            assert_eq!(state, before, "and the refusal stays on the screen");
        }
    }

    #[test]
    fn more_acts_only_when_the_server_holds_more_and_nothing_is_out() {
        assert!(listed(3, Some("c")).acts(Action::More), "a cursor is held");
        assert!(!listed(3, None).acts(Action::More), "every vCon is read");
        assert!(
            !after(listed(3, Some("c")), vec![Event::Action(Action::More)]).acts(Action::More),
            "the next page is on its way"
        );
        assert!(
            !after(listed(3, Some("c")), vec![Event::Action(Action::Refresh)]).acts(Action::More),
            "the list is being read again"
        );
        assert!(!new().acts(Action::More), "nothing is read yet");
    }

    #[test]
    fn open_acts_only_in_a_list_that_shows_a_row() {
        assert!(listed(1, None).acts(Action::Open), "one row can be opened");
        assert!(
            !listed(0, None).acts(Action::Open),
            "an empty tenant has none"
        );
        assert!(!new().acts(Action::Open), "nothing is listed yet");
        let detail = after(listed(2, None), vec![Event::Action(Action::Open)]);
        assert!(
            !detail.acts(Action::Open),
            "the detail view is open already"
        );
    }

    #[test]
    fn a_list_of_one_row_has_nowhere_to_move() {
        let state = listed(1, None);
        for action in TRAVEL {
            assert!(!state.acts(action), "{action:?} has no other row to select");
        }
    }

    #[test]
    fn the_selection_does_not_move_while_the_list_is_not_shown() {
        let mut state = after(listed(3, None), vec![Event::Action(Action::Refresh)]);
        assert_eq!(state.listing, Listing::Loading, "the list is being read");
        press(&mut state, Action::Down);
        assert_eq!(state.selected, 0, "the screen shows no row to move from");
        press(&mut state, Action::Open);
        assert_eq!(state.view, View::List, "and none to open");
    }

    #[test]
    fn back_acts_in_the_detail_and_help_views_only() {
        let list = listed(2, None);
        assert!(!list.acts(Action::Back), "the list is the first view");
        let detail = after(list.clone(), vec![Event::Action(Action::Open)]);
        assert!(detail.acts(Action::Back), "Esc returns to the list");
        let help = after(list, vec![Event::Action(Action::Help)]);
        assert!(help.acts(Action::Back), "Esc closes help");
    }

    #[test]
    fn in_a_detail_view_whose_vcon_is_gone_the_keys_that_move_select_a_listed_one() {
        let mut state = after(
            listed(5, None),
            vec![
                Event::Action(Action::Bottom),
                Event::Action(Action::Open),
                Event::Action(Action::Refresh),
            ],
        );
        state.step(Event::Page(Ok(page(0..1, None))), NOW);
        assert!(
            state.acts(Action::Down),
            "one row is left, and j shows it in place of the message"
        );
        press(&mut state, Action::Down);
        assert_eq!(
            state.detail_row().map(|row| row.uuid.clone()),
            Some(row(0).uuid),
            "the detail view shows the row that is left"
        );
        assert!(!state.acts(Action::Down), "then there is nowhere to move");
    }
}
