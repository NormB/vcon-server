//! Every screen the terminal UI draws, as one list: the tests of the views,
//! the screens the website shows (website/data/tui-screens.json) and the
//! terminal UI page's tests all read it, so a screen added here must be
//! drawn for the site and described on the page.

use time::format_description::well_known::Rfc3339;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

use crate::masked;
use crate::reveal::Part;
use crate::tui::client;
use crate::tui::keys::{BINDINGS, Binding};
use crate::tui::state::{
    Action, ApiError, Content, Event, KeyInfo, Listing, Page, Revealed, Row, State, View,
};

/// When the screens were read.
pub const NOW: OffsetDateTime = datetime!(2026-11-10 01:20:00 UTC);

/// The tenant of the screens.
pub const SHOWN_TENANT: &str = "8c41f2d9a07e4b5c9d13e6f0a2b4c7d8";

/// A screen of the terminal UI: a view in one of the states that change
/// what it shows, or a layout a terminal's width selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Screen {
    /// The list, with more vCons on the server.
    List,
    /// The list with every vCon read.
    ListEnd,
    /// The list of a tenant with one vCon.
    ListOne,
    /// The list while the next page is read.
    MoreReading,
    /// The list after the next page arrived.
    MoreRead,
    /// The list after the next page failed.
    NextPageFailed,
    /// A tenant with no vCons.
    Empty,
    /// Before the server has answered for the key.
    Connecting,
    /// The key answered and the first page has not.
    Loading,
    /// The server did not answer.
    NotConnected,
    /// The server refused the key.
    RefusedKey,
    /// The server's certificate was refused.
    RefusedCertificate,
    /// The key is an operator key.
    OperatorKey,
    /// The key lacks the read scope.
    NoReadScope,
    /// The key answered and the list was refused by the server.
    ServerError,
    /// The detail view of one vCon.
    Detail,
    /// The detail view of a tenant's only vCon.
    DetailOne,
    /// The detail view while the list is read again.
    DetailRefreshing,
    /// The detail view of a vCon that left the list.
    DetailGone,
    /// The detail view after a refresh failed.
    DetailFailed,
    /// The detail view while the masked view is read.
    DetailReading,
    /// The detail view of a masked view that was not read.
    DetailMaskedFailed,
    /// A reveal in flight.
    Revealing,
    /// The parties revealed.
    RevealParties,
    /// The subject revealed.
    RevealSubject,
    /// The dialog revealed.
    RevealDialog,
    /// The analysis revealed.
    RevealAnalysis,
    /// The links revealed.
    RevealLinks,
    /// The whole vCon revealed.
    RevealWhole,
    /// A reveal that failed.
    RevealFailed,
    /// The help view.
    Help,
    /// The list in a terminal of 120 columns.
    Wide,
    /// The list in a terminal of 60 columns.
    Narrow,
    /// The list in a terminal of 40 columns.
    Narrowest,
    /// A terminal too small for a view.
    TooSmall,
}

impl Screen {
    /// Every screen, in the order the page describes them.
    pub const ALL: [Screen; 35] = [
        Screen::List,
        Screen::ListEnd,
        Screen::ListOne,
        Screen::MoreReading,
        Screen::MoreRead,
        Screen::NextPageFailed,
        Screen::Empty,
        Screen::Connecting,
        Screen::Loading,
        Screen::NotConnected,
        Screen::RefusedKey,
        Screen::RefusedCertificate,
        Screen::OperatorKey,
        Screen::NoReadScope,
        Screen::ServerError,
        Screen::Detail,
        Screen::DetailOne,
        Screen::DetailRefreshing,
        Screen::DetailGone,
        Screen::DetailFailed,
        Screen::DetailReading,
        Screen::DetailMaskedFailed,
        Screen::Revealing,
        Screen::RevealParties,
        Screen::RevealSubject,
        Screen::RevealDialog,
        Screen::RevealAnalysis,
        Screen::RevealLinks,
        Screen::RevealWhole,
        Screen::RevealFailed,
        Screen::Help,
        Screen::Wide,
        Screen::Narrow,
        Screen::Narrowest,
        Screen::TooSmall,
    ];

    /// The heading of the screen's section on the terminal UI page.
    pub fn title(self) -> &'static str {
        match self {
            Screen::List => "The list",
            Screen::ListEnd => "The list with every vCon read",
            Screen::ListOne => "The list of one vCon",
            Screen::MoreReading => "The list while the next page is read",
            Screen::MoreRead => "The list after the next page",
            Screen::NextPageFailed => "A next page that was not read",
            Screen::Empty => "A tenant with no vCons",
            Screen::Connecting => "Connecting",
            Screen::Loading => "Loading",
            Screen::NotConnected => "A server that did not answer",
            Screen::RefusedKey => "A key the server refused",
            Screen::RefusedCertificate => "A certificate that was refused",
            Screen::OperatorKey => "An operator key",
            Screen::NoReadScope => "A key without the read scope",
            Screen::ServerError => "A list the server did not give",
            Screen::Detail => "The detail of a vCon",
            Screen::DetailOne => "The detail of the only vCon",
            Screen::DetailRefreshing => "The detail while the list is read again",
            Screen::DetailGone => "The detail of a vCon that left the list",
            Screen::DetailFailed => "The detail after a list that was not read",
            Screen::DetailReading => "The detail while the masked view is read",
            Screen::DetailMaskedFailed => "A masked view that was not read",
            Screen::Revealing => "A reveal being read",
            Screen::RevealParties => "The parties revealed",
            Screen::RevealSubject => "The subject revealed",
            Screen::RevealDialog => "The dialog revealed",
            Screen::RevealAnalysis => "The analysis revealed",
            Screen::RevealLinks => "The links revealed",
            Screen::RevealWhole => "The whole vCon revealed",
            Screen::RevealFailed => "A reveal that failed",
            Screen::Help => "Help",
            Screen::Wide => "A terminal of 120 columns",
            Screen::Narrow => "A terminal of 60 columns",
            Screen::Narrowest => "A terminal of 40 columns",
            Screen::TooSmall => "A terminal too small",
        }
    }

    /// Whether the screen says that something failed: the list could not
    /// be read, or a request left a notice above the keys.
    pub fn failure(self) -> bool {
        let state = self.state();
        matches!(state.listing, Listing::Failed(_))
            || state.notice.is_some()
            || matches!(state.content, Content::Failed(_))
            || matches!(state.revealed, Some(Revealed::Failed(..)))
    }

    /// The screen's name in website/data/tui-screens.json and on the page.
    pub fn name(self) -> &'static str {
        match self {
            Screen::List => "list",
            Screen::ListEnd => "list-end",
            Screen::ListOne => "list-one",
            Screen::MoreReading => "more-reading",
            Screen::MoreRead => "more-read",
            Screen::NextPageFailed => "next-page-failed",
            Screen::Empty => "empty",
            Screen::Connecting => "connecting",
            Screen::Loading => "loading",
            Screen::NotConnected => "not-connected",
            Screen::RefusedKey => "refused-key",
            Screen::RefusedCertificate => "refused-certificate",
            Screen::OperatorKey => "operator-key",
            Screen::NoReadScope => "no-read-scope",
            Screen::ServerError => "server-error",
            Screen::Detail => "detail",
            Screen::DetailOne => "detail-one",
            Screen::DetailRefreshing => "detail-refreshing",
            Screen::DetailGone => "detail-gone",
            Screen::DetailFailed => "detail-failed",
            Screen::DetailReading => "detail-reading",
            Screen::DetailMaskedFailed => "detail-masked-failed",
            Screen::Revealing => "revealing",
            Screen::RevealParties => "reveal-parties",
            Screen::RevealSubject => "reveal-subject",
            Screen::RevealDialog => "reveal-dialog",
            Screen::RevealAnalysis => "reveal-analysis",
            Screen::RevealLinks => "reveal-links",
            Screen::RevealWhole => "reveal-whole",
            Screen::RevealFailed => "reveal-failed",
            Screen::Help => "help",
            Screen::Wide => "wide",
            Screen::Narrow => "narrow",
            Screen::Narrowest => "narrowest",
            Screen::TooSmall => "too-small",
        }
    }

    /// The columns and lines of the terminal the screen is drawn in.
    pub fn size(self) -> (u16, u16) {
        match self {
            Screen::List
            | Screen::Detail
            | Screen::Help
            | Screen::DetailMaskedFailed
            | Screen::Revealing
            | Screen::RevealParties
            | Screen::RevealSubject
            | Screen::RevealDialog
            | Screen::RevealAnalysis
            | Screen::RevealLinks
            | Screen::RevealWhole
            | Screen::RevealFailed => (80, 24),
            Screen::Wide => (120, 10),
            Screen::Narrow => (60, 10),
            Screen::Narrowest => (40, 10),
            Screen::TooSmall => (36, 6),
            _ => (80, 10),
        }
    }

    /// The state the screen is drawn from.
    pub fn state(self) -> State {
        let reader = || shown_key(Some(SHOWN_TENANT), &["read"]);
        let no_answer = || client::no_answer(client::TIMEOUT);
        let unavailable =
            || ApiError::Status(503, Some("The service cannot answer now".to_owned()));
        match self {
            Screen::List | Screen::Wide | Screen::Narrow | Screen::Narrowest | Screen::TooSmall => {
                shown_list()
            }
            Screen::ListEnd => shown_page(reader(), 0..3, false),
            Screen::ListOne => shown_page(reader(), 0..1, false),
            Screen::MoreReading => press(shown_list(), &[Action::More]),
            Screen::MoreRead => shown_page(press(shown_list(), &[Action::More]), 100..200, false),
            Screen::NextPageFailed => failed(press(shown_list(), &[Action::More]), no_answer()),
            Screen::Empty => shown_page(reader(), 0..0, false),
            Screen::Connecting => connecting(),
            Screen::Loading => reader(),
            Screen::NotConnected => shown_refusal(no_answer()),
            Screen::RefusedKey => shown_refusal(ApiError::Status(401, None)),
            Screen::RefusedCertificate => {
                let advice = client::certificate_advice(&rustls::CertificateError::UnknownIssuer)
                    .expect("advice for a certificate no trusted authority signed");
                shown_refusal(ApiError::Untrusted(advice.to_owned()))
            }
            Screen::OperatorKey => shown_key(None, &[]),
            Screen::NoReadScope => shown_key(Some(SHOWN_TENANT), &["write"]),
            Screen::ServerError => failed(reader(), unavailable()),
            Screen::Detail => shown_masked(press(shown_list(), &[Action::Open])),
            Screen::DetailOne => {
                shown_masked(press(shown_page(reader(), 0..1, false), &[Action::Open]))
            }
            Screen::DetailReading => press(shown_list(), &[Action::Open]),
            Screen::DetailMaskedFailed => {
                let mut state = press(shown_list(), &[Action::Open]);
                let uuid = state.detail.clone().expect("a vCon open");
                let older = client::masked_answer(200, "application/vcon", SHOWN_VCON.as_bytes())
                    .expect_err("the whole vCon is not the masked view");
                state.step(Event::Masked(uuid, Err(older)), NOW);
                state
            }
            Screen::Revealing => press(shown_detail(), &[Action::Reveal(Part::Parties)]),
            Screen::RevealParties => shown_reveal(Part::Parties),
            Screen::RevealSubject => shown_reveal(Part::Subject),
            Screen::RevealDialog => shown_reveal(Part::Dialog),
            Screen::RevealAnalysis => shown_reveal(Part::Analysis),
            Screen::RevealLinks => shown_reveal(Part::Links),
            Screen::RevealWhole => shown_reveal(Part::All),
            Screen::RevealFailed => {
                let mut state = press(shown_detail(), &[Action::Reveal(Part::Dialog)]);
                let uuid = state.detail.clone().expect("a vCon open");
                state.step(Event::Revealed(uuid, Part::Dialog, Err(unavailable())), NOW);
                state
            }
            Screen::DetailRefreshing => press(shown_list(), &[Action::Open, Action::Refresh]),
            // The vCon open in the detail view is deleted; r reads a list
            // without it.
            Screen::DetailGone => shown_page(
                press(shown_list(), &[Action::Open, Action::Refresh]),
                3..100,
                true,
            ),
            Screen::DetailFailed => failed(
                press(shown_list(), &[Action::Open, Action::Refresh]),
                unavailable(),
            ),
            Screen::Help => press(shown_list(), &[Action::Help]),
        }
    }

    /// The view the screen is of.
    pub fn view(self) -> View {
        self.state().view
    }

    /// The bindings whose keys do something on the screen, in the order of
    /// the bindings table: what [`State::acts`] says of its state.
    pub fn keys(self) -> Vec<&'static Binding> {
        let state = self.state();
        BINDINGS
            .iter()
            .filter(|binding| state.acts(binding.action))
            .collect()
    }
}

/// The vCon the detail screens show, as a server stores it: synthetic, with
/// reserved names (example.com, a 555-01xx number).
pub const SHOWN_VCON: &str = r#"{"vcon":"0.0.1","uuid":"0193f1c2-5b7a-8e4d-9f60-2a1b3c4d5e6f","created_at":"2026-11-09T16:40:12Z","subject":"Renewal of the annual service plan","parties":[{"tel":"+12125550142","name":"Ada Example"},{"mailto":"support@example.com","name":"Example Support"}],"dialog":[{"type":"text","start":"2026-11-09T16:40:12Z","duration":95,"parties":[0,1],"encoding":"none","body":"Ada: I would like to renew the plan.\nSupport: It is renewed until November 2027."}],"analysis":[{"type":"summary","dialog":0,"vendor":"example","encoding":"none","body":"The customer renewed the annual plan."}],"attachments":[{"type":"invoice","url":"https://files.example.com/invoices/2026-11-09.pdf"}]}"#;

/// `state`, a detail view whose masked view was asked for, once the
/// masked view of [`SHOWN_VCON`] arrived.
pub fn shown_masked(mut state: State) -> State {
    let uuid = state.detail.clone().expect("a vCon open");
    let lines = client::summary_lines(&masked::summary(SHOWN_VCON));
    state.step(Event::Masked(uuid, Ok(lines)), NOW);
    state
}

/// The shown list's third vCon open, its masked view read.
pub fn shown_detail() -> State {
    shown_masked(press(shown_list(), &[Action::Open]))
}

/// The detail view with `part` of [`SHOWN_VCON`] revealed.
pub fn shown_reveal(part: Part) -> State {
    let mut state = press(shown_detail(), &[Action::Reveal(part)]);
    let uuid = state.detail.clone().expect("a vCon open");
    let text = client::reveal_text(SHOWN_VCON, part);
    state.step(Event::Revealed(uuid, part, Ok(text)), NOW);
    state
}

/// The state before the server has answered.
pub fn connecting() -> State {
    State::new("https://vcon.example.com:8443", (80, 24)).0
}

/// `state` after the key presses `actions`.
pub fn press(mut state: State, actions: &[Action]) -> State {
    for action in actions {
        state.step(Event::Action(*action), NOW);
    }
    state
}

/// `state` after a request for a page failed with `error`.
pub fn failed(mut state: State, error: ApiError) -> State {
    state.step(Event::Page(Err(error)), NOW);
    state
}

/// A tenant's first page of 100 vCons, newest first, with more on the
/// server, and the third one selected: the list the site shows.
pub fn shown_list() -> State {
    let state = shown_page(shown_key(Some(SHOWN_TENANT), &["read"]), 0..100, true);
    press(state, &[Action::Down, Action::Down])
}

/// The state once the server has answered for a key of `tenant` (an
/// operator key has none) that holds `scopes`.
pub fn shown_key(tenant: Option<&str>, scopes: &[&str]) -> State {
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
pub fn shown_page(mut state: State, rows: std::ops::Range<u64>, more: bool) -> State {
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
pub fn shown_refusal(error: ApiError) -> State {
    let mut state = connecting();
    state.step(Event::KeyInfo(Err(error)), NOW);
    state
}

/// The `n`th vCon of the shown list: a version 8 uuid made from `n`,
/// stored 37 minutes before the one above it, some updated since.
pub fn shown_row(n: u64) -> Row {
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
