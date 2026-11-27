//! A tenant catalog this build cannot open is reported with its cause.
//!
//! On 2026-11-25, 0.4.16's `check`, run on a data directory 0.4.17 had
//! upgraded, printed "its catalog cannot be opened: Io": the catalog's
//! CatalogError::UnknownSchema became OpenError::Store(StoreError::Io)
//! (store.rs, `From<CatalogError> for OpenError`), and every caller after
//! it saw only Io. Now the schema version, and any other cause of a
//! catalog that does not open, reach `check`, `serve`'s start-up refusal,
//! the commands and the server's log.

use std::io::Write;
use std::sync::{Arc, Mutex};

use time::macros::datetime;

use super::examine::Unexamined;
use super::tests::fixture;
use super::{Actor, OpenFailure, Removal, TenantError};
use crate::catalog::{CatalogError, SCHEMA_VERSION};
use crate::ids::TenantId;
use crate::retention::Retention;
use crate::store::{OpenError, StoreError};

const NEWER: i64 = SCHEMA_VERSION + 1;

/// The refusal's text, as the catalog gives it.
fn refusal(version: i64) -> String {
    CatalogError::UnknownSchema(version).to_string()
}

/// Sets the tenant's catalog to schema version `version`, as a newer (or
/// a damaged) build leaves it; the registry has not opened it yet.
fn at_version(f: &super::tests::Fixture, id: TenantId, version: i64) {
    let store = f.store(id);
    store
        .conn_for_test()
        .execute_batch(&format!("PRAGMA user_version = {version};"))
        .expect("the version is set");
}

fn made(f: &super::tests::Fixture, name: &str) -> TenantId {
    f.tenants.create(Actor::Cli, name, None).expect("made")
}

// The cause reaches each caller.

#[test]
fn the_store_of_a_newer_catalog_is_refused_with_its_version() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    assert_eq!(
        f.tenants.store(id).err(),
        Some(TenantError::UnknownSchema(NEWER)),
        "the version, not Io"
    );
}

#[test]
fn store_or_why_names_the_version() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    match f.tenants.store_or_why(id).map(|_| ()) {
        Err(OpenFailure::Refused(TenantError::UnknownSchema(n))) => {
            assert_eq!(n, NEWER, "the version")
        }
        other => panic!("a refusal naming the version: {other:?}"),
    }
}

#[test]
fn the_start_up_count_gives_the_refusal_text() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let order = f.tenants.key_order().expect("read");
    assert_eq!(order.catalogs_unopened, 1, "{order:?}");
    assert_eq!(
        order.unopened_error,
        Some(refusal(NEWER)),
        "the text serve prints"
    );
    assert!(!order.is_sound(), "a start is refused");
}

#[test]
fn a_newer_catalog_is_not_a_key_problem() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let order = f.tenants.key_order().expect("read");
    assert_eq!(
        order.data_under_other_keys, 0,
        "the key is not the cause: {order:?}"
    );
    assert_eq!(order.wrapped_elsewhere, 0, "{order:?}");
}

#[test]
fn serves_refusal_names_the_version() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let order = f.tenants.key_order().expect("read");
    assert_eq!(
        crate::serve::ServeError::KeysOutOfOrder(order).to_string(),
        format!(
            "1 tenant catalog(s) cannot be opened: {}; vcon-server check names the tenants",
            refusal(NEWER)
        ),
        "serve's refusal"
    );
}

#[test]
fn examine_keeps_the_version() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let found = f
        .tenants
        .examine(Retention::Years(7), datetime!(2026-10-23 12:00 UTC))
        .expect("examined");
    match &found.tenants[0].store {
        Err(Unexamined::Open(OpenFailure::Refused(TenantError::UnknownSchema(n)))) => {
            assert_eq!(*n, NEWER, "the version");
        }
        other => panic!("a refusal naming the version: {other:?}"),
    }
}

#[test]
fn checks_problem_line_names_the_version() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let found = f
        .tenants
        .examine(Retention::Years(7), datetime!(2026-10-23 12:00 UTC))
        .expect("examined");
    assert_eq!(
        crate::commands::problems(
            &found.tenants[0],
            std::time::Duration::from_secs(3600),
            datetime!(2026-10-23 12:00 UTC),
            true
        ),
        [format!("its catalog cannot be opened: {}", refusal(NEWER))],
        "check's line"
    );
}

#[test]
fn a_removal_of_the_tenant_is_refused_with_the_version_and_keeps_it() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    assert_eq!(
        f.tenants.remove(Actor::Cli, id, Removal::Empty),
        Err(TenantError::UnknownSchema(NEWER)),
        "a removal opens the Store to count what it holds"
    );
    assert_eq!(
        f.tenants.list(Actor::Cli).expect("list").len(),
        1,
        "the tenant is kept"
    );
}

#[test]
fn the_command_lines_message_names_the_version() {
    let text = crate::commands::CommandError::Tenant(TenantError::UnknownSchema(NEWER)).to_string();
    assert!(
        text.contains(&format!("schema version {NEWER}")),
        "the version: {text}"
    );
    assert!(text.contains("newer build"), "what made it: {text}");
}

#[test]
fn a_refused_open_is_logged_with_the_version_only() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .log_internal_errors(false)
        .finish();
    let answer = tracing::subscriber::with_default(subscriber, || f.tenants.store(id));
    assert!(answer.is_err(), "refused");
    let log = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("UTF-8");
    assert!(log.contains("WARN"), "a warning: {log}");
    assert!(
        log.contains(&format!("schema_version={NEWER}")),
        "the version: {log}"
    );
    assert!(
        !log.contains(&id.to_string()),
        "no tenant id, as the other open failures: {log}"
    );
}

#[test]
fn the_start_up_check_logs_nothing_of_its_own() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .log_internal_errors(false)
        .finish();
    let order = tracing::subscriber::with_default(subscriber, || f.tenants.key_order());
    assert_eq!(order.expect("read").catalogs_unopened, 1, "counted");
    let log = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("UTF-8");
    assert!(
        log.is_empty(),
        "serve's refusal says it once, on stderr: {log}"
    );
}

// Only the tenant whose catalog is newer, and any version.

#[test]
fn only_the_newer_tenant_is_refused() {
    let f = fixture();
    let newer = made(&f, "acme");
    let other = made(&f, "globex");
    at_version(&f, newer, NEWER);
    assert!(f.tenants.store(other).is_ok(), "the other tenant opens");
    let order = f.tenants.key_order().expect("read");
    assert_eq!(order.catalogs_unopened, 1, "one: {order:?}");
}

#[test]
fn a_version_far_ahead_is_named_as_it_is() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, SCHEMA_VERSION + 100);
    assert_eq!(
        f.tenants.store(id).err(),
        Some(TenantError::UnknownSchema(SCHEMA_VERSION + 100)),
        "the stored number"
    );
}

#[test]
fn a_negative_version_is_named_as_it_is() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, -1);
    let order = f.tenants.key_order().expect("read");
    assert_eq!(
        order.unopened_error,
        Some(refusal(-1)),
        "a damaged number is shown too"
    );
}

#[test]
fn the_first_of_two_refusals_is_the_one_serve_prints() {
    let f = fixture();
    let a = made(&f, "acme");
    let b = made(&f, "globex");
    at_version(&f, a, NEWER);
    at_version(&f, b, NEWER + 1);
    let order = f.tenants.key_order().expect("read");
    assert_eq!(order.catalogs_unopened, 2, "{order:?}");
    assert!(
        order.unopened_error.is_some(),
        "one cause is printed: {order:?}"
    );
}

#[test]
fn a_refused_open_is_not_kept_as_an_open_store() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    for _ in 0..2 {
        assert_eq!(
            f.tenants.store(id).err(),
            Some(TenantError::UnknownSchema(NEWER)),
            "each try opens the catalog again"
        );
    }
}

// The conversions on the way, for every cause.

#[test]
fn an_unknown_schema_becomes_its_own_open_error() {
    assert!(
        matches!(OpenError::from(CatalogError::UnknownSchema(NEWER)), OpenError::UnknownSchema(n) if n == NEWER),
        "the version is kept"
    );
}

#[test]
fn a_sqlite_failure_keeps_its_cause() {
    let e = OpenError::from(CatalogError::Sqlite(rusqlite::Error::InvalidQuery));
    let OpenError::Catalog(cause) = e else {
        panic!("a catalog failure with its cause: {e:?}")
    };
    assert!(
        super::with_causes(cause.as_ref()).starts_with("sqlite failed"),
        "{cause}"
    );
}

#[test]
fn a_file_failure_keeps_its_cause() {
    let io = std::io::Error::other("the disk is full");
    let e = OpenError::from(CatalogError::Io(io));
    let OpenError::Catalog(cause) = e else {
        panic!("a catalog failure with its cause: {e:?}")
    };
    assert!(
        super::with_causes(cause.as_ref()).ends_with("the disk is full"),
        "{cause}"
    );
}

#[test]
fn a_wrong_key_and_an_access_failure_are_as_before() {
    assert!(
        matches!(OpenError::from(CatalogError::WrongKey), OpenError::WrongKey),
        "WrongKey"
    );
    let access = CatalogError::Access {
        path: "/srv/data/tenants/01/catalog.sqlite".into(),
        error: std::io::Error::from_raw_os_error(libc::EACCES),
    };
    assert!(
        matches!(OpenError::from(access), OpenError::Access { .. }),
        "Access"
    );
}

#[test]
fn a_catalog_failure_reaches_check_with_its_cause() {
    let failure = OpenFailure::from(OpenError::from(CatalogError::Io(std::io::Error::other(
        "the disk is full",
    ))));
    assert_eq!(
        failure.cause().as_deref(),
        Some("a file around the catalog could not be read, written or renamed: the disk is full"),
        "the cause and the error under it"
    );
}

#[test]
fn a_catalog_failure_is_io_to_every_other_caller() {
    let failure = OpenFailure::from(OpenError::from(CatalogError::Sqlite(
        rusqlite::Error::InvalidQuery,
    )));
    assert_eq!(
        TenantError::from(failure),
        TenantError::Io,
        "only check and serve read the cause"
    );
}

#[test]
fn the_cause_of_each_failure() {
    assert_eq!(
        OpenFailure::WrongKey.cause(),
        None,
        "a key problem is reported as that"
    );
    assert_eq!(
        OpenFailure::Refused(TenantError::UnknownSchema(NEWER)).cause(),
        Some(refusal(NEWER)),
        "the version"
    );
    assert_eq!(
        OpenFailure::Refused(TenantError::Io).cause(),
        None,
        "no cause known"
    );
    let access = OpenFailure::Access {
        path: "/x".into(),
        error: std::io::Error::from_raw_os_error(libc::EACCES),
    };
    assert_eq!(
        access.cause().as_deref(),
        Some("Permission denied (os error 13)"),
        "the system's error"
    );
}

#[test]
fn a_store_error_is_still_io() {
    let failure = OpenFailure::from(OpenError::Store(StoreError::Io));
    assert!(
        matches!(failure, OpenFailure::Refused(TenantError::Io)),
        "{failure:?}"
    );
}

#[test]
fn checks_line_for_a_catalog_failure_gives_the_cause() {
    let failure = OpenFailure::from(OpenError::from(CatalogError::Sqlite(
        rusqlite::Error::InvalidQuery,
    )));
    let line = crate::commands::open_problem(&failure);
    assert!(
        line.starts_with("its catalog cannot be opened: sqlite failed"),
        "{line}"
    );
}

// What stays as it was: a catalog that opens, and the other kinds of
// failure, through each surface.

fn sqlite_failure() -> OpenFailure {
    OpenFailure::from(OpenError::from(CatalogError::Sqlite(
        rusqlite::Error::InvalidQuery,
    )))
}

fn access_failure() -> OpenFailure {
    OpenFailure::Access {
        path: "/srv/data/tenants/01/catalog.sqlite".into(),
        error: std::io::Error::from_raw_os_error(libc::EACCES),
    }
}

/// What the log wrote while `f` ran.
fn logged<T>(f: impl FnOnce() -> T) -> (T, String) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .log_internal_errors(false)
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let log = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("UTF-8");
    (out, log)
}

#[test]
fn a_catalog_at_this_builds_version_opens() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, SCHEMA_VERSION);
    assert!(f.tenants.store(id).is_ok(), "this build's own version");
}

#[test]
fn a_sound_registry_has_no_cause_to_print() {
    let f = fixture();
    made(&f, "acme");
    let order = f.tenants.key_order().expect("read");
    assert_eq!(order.unopened_error, None, "{order:?}");
    assert!(order.is_sound(), "nothing to refuse");
}

#[test]
fn a_tenant_beside_a_newer_one_is_examined_whole() {
    let f = fixture();
    let newer = made(&f, "acme");
    let other = made(&f, "globex");
    at_version(&f, newer, NEWER);
    let found = f
        .tenants
        .examine(Retention::Years(7), datetime!(2026-10-23 12:00 UTC))
        .expect("examined");
    let opened = found.tenants.iter().filter(|t| t.store.is_ok()).count();
    assert_eq!(opened, 1, "the other tenant is examined");
    assert!(
        found
            .tenants
            .iter()
            .any(|t| t.id == other && t.store.is_ok()),
        "globex opens"
    );
}

#[test]
fn checks_line_for_a_wrong_key_is_as_before() {
    assert_eq!(
        crate::commands::open_problem(&OpenFailure::WrongKey),
        "its catalog does not open under its tenant key",
        "a key problem"
    );
}

#[test]
fn checks_line_for_an_access_failure_is_as_before() {
    assert_eq!(
        crate::commands::open_problem(&access_failure()),
        "its catalog cannot be opened: /srv/data/tenants/01/catalog.sqlite: Permission denied (os error 13)",
        "the path and the system's error"
    );
}

#[test]
fn checks_line_for_a_refusal_with_no_cause_is_as_before() {
    assert_eq!(
        crate::commands::open_problem(&OpenFailure::Refused(TenantError::Io)),
        "its catalog cannot be opened: Io",
        "no cause is known"
    );
}

#[test]
fn serves_refusal_for_an_access_failure_is_as_before() {
    let order = super::examine::KeyOrder {
        catalogs_unopened: 1,
        unopened_error: access_failure().cause(),
        ..Default::default()
    };
    assert_eq!(
        crate::serve::ServeError::KeysOutOfOrder(order).to_string(),
        "1 tenant catalog(s) cannot be opened: Permission denied (os error 13); vcon-server check names the tenants",
        "the system's error"
    );
}

#[test]
fn the_command_lines_message_for_io_is_as_before() {
    assert_eq!(
        crate::commands::CommandError::Tenant(TenantError::Io).to_string(),
        "the data could not be read or written",
        "Io"
    );
}

#[test]
fn the_api_answers_an_unknown_schema_as_an_internal_failure() {
    let problem = crate::api::Problem::Tenant(TenantError::UnknownSchema(NEWER));
    assert_eq!(problem.status().as_u16(), 500, "the server's failure");
    assert_eq!(problem.name().0, "internal", "the internal problem type");
}

#[test]
fn the_api_document_names_no_schema_version() {
    let document = crate::api::Problem::Tenant(TenantError::UnknownSchema(NEWER))
        .document()
        .to_string();
    assert!(
        !document.contains(&NEWER.to_string()),
        "no schema detail to a client: {document}"
    );
}

#[test]
fn a_tenant_error_carries_its_version_by_value() {
    let error = TenantError::UnknownSchema(NEWER);
    let copy = error;
    assert_eq!(error, copy, "Copy and Eq, as every TenantError");
    assert_ne!(
        error,
        TenantError::UnknownSchema(NEWER + 1),
        "the version is compared"
    );
}

#[test]
fn an_open_store_is_shared_after_the_catalog_is_current() {
    let f = fixture();
    let id = made(&f, "acme");
    let first = f.tenants.store(id).expect("opens");
    let second = f.tenants.store(id).expect("opens");
    assert!(Arc::ptr_eq(&first, &second), "one Store per tenant");
}

// The failures, through the other surfaces.

#[test]
fn a_catalog_failure_reaches_serves_refusal_with_its_cause() {
    let order = super::examine::KeyOrder {
        catalogs_unopened: 1,
        unopened_error: sqlite_failure().cause(),
        ..Default::default()
    };
    let text = crate::serve::ServeError::KeysOutOfOrder(order).to_string();
    assert!(
        text.starts_with("1 tenant catalog(s) cannot be opened: sqlite failed"),
        "{text}"
    );
}

#[test]
fn a_command_on_a_newer_tenant_names_the_version() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let error = f.tenants.store(id).err().expect("refused");
    let text = crate::commands::CommandError::Tenant(error).to_string();
    assert!(text.contains(&format!("schema version {NEWER}")), "{text}");
}

#[test]
fn a_request_to_a_newer_tenant_is_logged_once() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let (_, log) = logged(|| f.tenants.store(id));
    assert_eq!(
        log.matches("schema_version=").count(),
        1,
        "one line per refused open: {log}"
    );
}

#[test]
fn a_catalog_failure_through_store_is_io() {
    let failure = sqlite_failure();
    assert!(
        matches!(failure, OpenFailure::Catalog(_)),
        "kept apart for check and serve"
    );
    assert_eq!(
        TenantError::from(failure),
        TenantError::Io,
        "a request sees Io"
    );
}

// The edges: any stored number, several tenants, and what is not logged.

#[test]
fn two_versions_ahead_is_named_as_it_is() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, SCHEMA_VERSION + 2);
    assert_eq!(
        f.tenants.store(id).err(),
        Some(TenantError::UnknownSchema(SCHEMA_VERSION + 2)),
        "the stored number, not the next one"
    );
}

// SQLite keeps user_version as a 32-bit signed number, so these are the
// largest and smallest versions a catalog can hold.

#[test]
fn the_largest_version_is_named() {
    let f = fixture();
    let id = made(&f, "acme");
    let most = i64::from(i32::MAX);
    at_version(&f, id, most);
    assert_eq!(
        f.tenants.store(id).err(),
        Some(TenantError::UnknownSchema(most)),
        "i32::MAX"
    );
}

#[test]
fn the_smallest_version_is_named() {
    let f = fixture();
    let id = made(&f, "acme");
    let least = i64::from(i32::MIN);
    at_version(&f, id, least);
    let order = f.tenants.key_order().expect("read");
    assert_eq!(order.unopened_error, Some(refusal(least)), "i32::MIN");
}

/// Version 0 is a new catalog's: on a catalog that holds tables, making
/// them fails, and check prints SQLite's error, not "Io". This pins
/// today's text; that such a catalog is damage to be refused is backlog
/// item CATALOG-VERSION-0-WITH-TABLES.
#[test]
fn a_catalog_with_tables_at_version_zero_names_sqlites_error() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, 0);
    let order = f.tenants.key_order().expect("read");
    let cause = order.unopened_error.expect("a cause");
    assert!(
        cause.starts_with("sqlite failed: table vcon already exists"),
        "{cause}"
    );
}

// The cause check, serve and the command line show is bounded: its first
// line, at most CAUSE_LIMIT characters. SQLite's error for the catalog
// above quotes the whole schema, about 2 KB on many lines.

/// A Catalog failure whose SQLite message is `message`.
fn sqlite_saying(message: &str) -> OpenFailure {
    OpenFailure::from(OpenError::from(CatalogError::Sqlite(
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(1), Some(message.to_owned())),
    )))
}

/// What rusqlite adds after the message: its SQLite error, as the source.
const UNDER: &str = ": Error code 1: SQL logic error";

/// The longest cause shown: CAUSE_LIMIT characters and the marker.
fn longest() -> usize {
    super::CAUSE_LIMIT + super::CUT_MARKER.chars().count()
}

#[test]
fn the_version_zero_cause_is_one_line() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, 0);
    let cause = f
        .tenants
        .key_order()
        .expect("read")
        .unopened_error
        .expect("a cause");
    assert!(!cause.contains('\n'), "one line: {cause}");
    assert!(
        cause.chars().count() <= longest(),
        "{} characters: {cause}",
        cause.chars().count()
    );
}

#[test]
fn the_version_zero_line_of_check_is_bounded() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, 0);
    let found = f
        .tenants
        .examine(Retention::Years(7), datetime!(2026-10-23 12:00 UTC))
        .expect("examined");
    let lines = crate::commands::problems(
        &found.tenants[0],
        std::time::Duration::from_secs(3600),
        datetime!(2026-10-23 12:00 UTC),
        true,
    );
    let line = &lines[0];
    assert!(!line.contains('\n'), "one line: {line}");
    assert!(
        line.chars().count() <= "its catalog cannot be opened: ".len() + longest(),
        "bounded: {line}"
    );
}

#[test]
fn serves_refusal_for_a_long_cause_is_bounded() {
    let failure = sqlite_saying(&"x".repeat(5000));
    let order = super::examine::KeyOrder {
        catalogs_unopened: 1,
        unopened_error: failure.cause(),
        ..Default::default()
    };
    let text = crate::serve::ServeError::KeysOutOfOrder(order).to_string();
    assert!(text.len() < 400, "{} bytes", text.len());
    assert!(
        text.ends_with("vcon-server check names the tenants"),
        "the pointer stays: {text}"
    );
}

#[test]
fn a_multi_line_cause_shows_its_first_line() {
    let cause = sqlite_saying("first line\nsecond line\nthird")
        .cause()
        .expect("a cause");
    assert!(cause.starts_with("sqlite failed: first line"), "{cause}");
    assert!(!cause.contains("second line"), "{cause}");
}

#[test]
fn a_cut_cause_says_where_the_rest_is() {
    let cause = sqlite_saying("first line\nsecond line")
        .cause()
        .expect("a cause");
    assert!(cause.ends_with(super::CUT_MARKER), "{cause}");
}

#[test]
fn a_long_single_line_is_cut_at_the_limit() {
    let cause = sqlite_saying(&"y".repeat(1000)).cause().expect("a cause");
    assert_eq!(
        cause.chars().count(),
        longest(),
        "the limit and the marker: {cause}"
    );
}

#[test]
fn a_short_cause_is_shown_whole() {
    assert_eq!(
        sqlite_saying("database is locked").cause().as_deref(),
        Some("sqlite failed: database is locked: Error code 1: SQL logic error"),
        "no marker on a cause that fits"
    );
}

#[test]
fn a_cause_of_exactly_the_limit_is_shown_whole() {
    let text = "z".repeat(super::CAUSE_LIMIT - "sqlite failed: ".len() - UNDER.len());
    let cause = sqlite_saying(&text).cause().expect("a cause");
    assert_eq!(
        cause.chars().count(),
        super::CAUSE_LIMIT,
        "exactly the limit"
    );
    assert!(!cause.ends_with(super::CUT_MARKER), "not cut: {cause}");
}

#[test]
fn a_cause_one_past_the_limit_is_cut() {
    let text = "z".repeat(super::CAUSE_LIMIT - "sqlite failed: ".len() - UNDER.len() + 1);
    let cause = sqlite_saying(&text).cause().expect("a cause");
    assert!(cause.ends_with(super::CUT_MARKER), "cut: {cause}");
}

#[test]
fn a_cause_is_cut_between_characters() {
    let cause = sqlite_saying(&"é".repeat(1000))
        .cause()
        .expect("a cut on a character boundary");
    assert_eq!(cause.chars().count(), longest(), "characters, not bytes");
}

#[test]
fn the_whole_cause_is_in_the_debug_log_only() {
    let failure = sqlite_saying("first line\nsecond line");
    let (cause, debug) = logged_at(tracing::Level::DEBUG, || failure.cause());
    assert!(
        debug.contains("second line"),
        "the whole error at debug: {debug}"
    );
    let (_, info) = logged_at(tracing::Level::INFO, || failure.cause());
    assert!(!info.contains("second line"), "nothing at info: {info}");
    assert!(
        !cause.expect("a cause").contains("second line"),
        "the shown cause is cut"
    );
}

#[test]
fn the_schema_refusal_and_an_access_failure_are_not_cut() {
    assert_eq!(
        OpenFailure::Refused(TenantError::UnknownSchema(NEWER)).cause(),
        Some(refusal(NEWER)),
        "short"
    );
    assert_eq!(
        access_failure().cause().as_deref(),
        Some("Permission denied (os error 13)"),
        "short"
    );
}

/// What the log wrote at `level` and above while `f` ran.
fn logged_at<T>(level: tracing::Level, f: impl FnOnce() -> T) -> (T, String) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(level)
        .log_internal_errors(false)
        .finish();
    let out = tracing::subscriber::with_default(subscriber, f);
    let log = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("UTF-8");
    (out, log)
}

#[test]
fn store_or_why_answers_the_same_twice() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    for _ in 0..2 {
        assert!(
            matches!(f.tenants.store_or_why(id).map(|_| ()), Err(OpenFailure::Refused(TenantError::UnknownSchema(n))) if n == NEWER),
            "each open reads the catalog again"
        );
    }
}

#[test]
fn an_io_failure_logs_no_schema_line() {
    let f = fixture();
    let (answer, log) = logged(|| f.tenants.store(TenantId([7; 16])));
    assert_eq!(answer.err(), Some(TenantError::NotFound), "no such tenant");
    assert!(
        !log.contains("schema_version"),
        "only the schema refusal logs it: {log}"
    );
}

#[test]
fn the_schema_log_line_names_no_tenant_name_or_path() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let (_, log) = logged(|| f.tenants.store(id));
    assert!(!log.contains("acme"), "no tenant name: {log}");
    assert!(!log.contains("catalog.sqlite"), "no path: {log}");
}

#[test]
fn examine_logs_nothing_for_a_newer_catalog() {
    let f = fixture();
    let id = made(&f, "acme");
    at_version(&f, id, NEWER);
    let (found, log) = logged(|| {
        f.tenants
            .examine(Retention::Years(7), datetime!(2026-10-23 12:00 UTC))
    });
    assert!(found.is_ok(), "examined");
    assert!(
        log.is_empty(),
        "check prints it; the log does not repeat it: {log}"
    );
}

#[test]
fn every_tenant_of_a_newer_build_is_counted() {
    let f = fixture();
    let ids: Vec<TenantId> = ["a1", "b2", "c3"].iter().map(|n| made(&f, n)).collect();
    for id in &ids {
        at_version(&f, *id, NEWER);
    }
    let order = f.tenants.key_order().expect("read");
    assert_eq!(order.catalogs_unopened, 3, "{order:?}");
    assert_eq!(
        order.unopened_error,
        Some(refusal(NEWER)),
        "one cause, the same for all"
    );
}

#[test]
fn a_newer_and_an_unreadable_catalog_are_both_counted() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let newer = made(&f, "acme");
    let closed = made(&f, "globex");
    at_version(&f, newer, NEWER);
    let dir = f.tenants_dir().join(closed.to_string());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).expect("chmod");
    let order = f.tenants.key_order();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).expect("chmod");
    let order = order.expect("read");
    assert_eq!(order.catalogs_unopened, 2, "{order:?}");
    let cause = order.unopened_error.expect("a cause");
    assert!(
        cause == refusal(NEWER) || cause == "Permission denied (os error 13)",
        "the first tenant's cause: {cause}"
    );
}

#[test]
fn a_nested_cause_is_printed_whole() {
    let inner = std::io::Error::other("the disk is full");
    let failure = OpenFailure::from(OpenError::from(CatalogError::Io(std::io::Error::other(
        inner,
    ))));
    let cause = failure.cause().expect("a cause");
    assert!(
        cause.contains("the disk is full"),
        "the error under it: {cause}"
    );
}

#[test]
fn a_refusal_with_another_tenant_error_has_no_cause() {
    for error in [TenantError::NotFound, TenantError::Closing, TenantError::Io] {
        assert_eq!(OpenFailure::Refused(error).cause(), None, "{error:?}");
    }
}

/// Collects what the log writes.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;

    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}
