//! Times one server writes and another judges, on one database. Each is
//! the database's clock in the transaction that writes or judges it
//! (`PgTx::clock`), so the host clocks of the servers do not matter. The
//! first server's host clock is conformance::START, a month behind the
//! database's; the second's is set hours or days ahead of the first's, or
//! behind it, far past the reader grace (15 minutes), a session's idle
//! limit (15 minutes) and the shortest retention and key life (a day).
//! Time passing on the database's clock is made by the test seam
//! `step_clock` (the next readings of the clock, one each) or by making a
//! row older as the schema's owner.

use time::Duration as Span;

use super::{
    Actor, Arc, Caller, Catalogs, Key, Removal, Retention, TenantId, Tenants, TestClock, World,
    conformance, in_scope, packed_world, pause, stop, wait, world,
};
use crate::access::{self, AuthError, Scope, Scopes};
use crate::roles::{Role, Roles};
use crate::store::ListQuery;
use crate::store::packer::PackPlan;
use crate::tenants::{KeyLife, Proof, SessionError, Started};

/// How far apart the two servers' host clocks are where an hour matters.
const SKEW: Span = Span::hours(2);

/// How far apart they are where a day is the least that matters.
const DAYS: Span = Span::days(3);

/// How far ahead of the first server's a host's clock is to be ahead of
/// the database's too: conformance::START is a month behind it.
const AHEAD_OF_THE_DATABASE: Span = Span::days(400);

/// The reader grace, which keeps an unnamed pack file and an older
/// manifest.
const GRACE: Span = Span::minutes(15);

/// A second, in nanoseconds.
const SECOND: i64 = 1_000_000_000;

/// The database's clock now, in nanoseconds, as SQL.
const DB_NOW: &str = "(extract(epoch FROM clock_timestamp()) * 1000000000)::bigint";

fn who() -> Caller {
    Caller {
        key: Key::generate().expect("a key").id(),
    }
}

/// `n`'s uuid.
fn uuid_of(n: u8) -> String {
    format!("01a07da8-c2bb-83e5-b9a2-279e0d16bd{n:02x}")
}

/// Stores vCon `n` through `tenants` and answers its uuid.
fn store(tenants: &Tenants, id: TenantId, who: &Caller, n: u8) -> uuid::Uuid {
    let (found, body) = conformance::vcon(&uuid_of(n), "on two servers");
    tenants
        .store(id)
        .expect("the store")
        .create(who, &found, &body)
        .expect("stored");
    found.uuid
}

/// How many of `table`'s rows in `scope` with `column` set hold a time
/// within a minute of the database's clock, of how many: `n/n` when the
/// database's clock wrote them all.
fn by_the_database(w: &World, scope: &str, table: &str, column: &str) -> String {
    w.value_in(
        scope,
        &format!(
            "SELECT (count(*) FILTER (WHERE abs({column} - {DB_NOW}) < 60000000000)) || '/' || \
             count(*) FROM vcon.{table} WHERE {column} IS NOT NULL"
        ),
    )
}

/// Makes `column` of every row of `table` in `scope` `by` older, as the
/// schema's owner: what that much time passing on the database's clock
/// does to the rows.
fn older(w: &World, scope: &str, table: &str, column: &str, by: Span) {
    let sql = format!(
        "UPDATE vcon.{table} SET {column} = {column} - {} WHERE {column} IS NOT NULL",
        by.whole_nanoseconds()
    );
    wait(async {
        let owner = w.db.admin().await;
        owner
            .batch_execute("SET ROLE vcon_owner")
            .await
            .expect("the owner role");
        in_scope(&owner, scope, &sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    });
}

/// A number `sql` reads in `scope`.
fn number(w: &World, scope: &str, sql: &str) -> i64 {
    w.value_in(scope, sql).parse().expect("a number")
}

// Pack files a compaction stopped naming, and the reader grace.

/// Packing without merging, then merging, each with the reader grace.
fn plans() -> (PackPlan, PackPlan) {
    let plain = PackPlan {
        merge_below: 0,
        ..PackPlan::with_limit(1 << 20)
    };
    (plain, PackPlan::with_limit(1 << 20))
}

/// A tenant whose first server packed two files and merged them into a
/// third: the two are unnamed, by the first server, and wait out the
/// reader grace. Answers the first server and the tenant.
fn two_files_unnamed(w: &World) -> ((Catalogs, Tenants), TenantId) {
    let first = w.serve();
    let id = first.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let (plain, merging) = plans();
    let packed = first.1.store(id).expect("the first's store");
    for n in 1..=2 {
        store(&first.1, id, &caller, n);
        packed.pack(&plain).expect("packed");
    }
    assert_eq!(packed.compact(&merging), Ok(1), "the two files are merged");
    assert_eq!(
        w.rows(id, "parquet_file"),
        3,
        "two unnamed files and the one that holds their rows"
    );
    (first, id)
}

/// The pack-file removal of a server whose host clock is at `at`.
fn remove_expired_at(w: &World, id: TenantId, at: time::OffsetDateTime, steps: Vec<i64>) {
    let other = w.serve_at(at);
    let store = other.1.store(id).expect("the other's store");
    store.step_clock(steps);
    store.remove_expired(&plans().1).expect("the removal runs");
    drop(store);
    stop(other);
}

#[test]
fn a_file_unnamed_by_one_server_is_unnamed_at_the_database_s_time() {
    let w = packed_world();
    let (_first, id) = two_files_unnamed(&w);
    assert_eq!(
        by_the_database(&w, &id.to_string(), "parquet_file", "unnamed_at"),
        "2/2",
        "each unnamed time is the database's, not the first server's clock"
    );
}

#[test]
fn a_file_unnamed_just_now_is_kept_by_a_server_whose_clock_is_hours_ahead() {
    let w = packed_world();
    let (_first, id) = two_files_unnamed(&w);
    remove_expired_at(&w, id, conformance::START + SKEW, Vec::new());
    assert_eq!(
        w.rows(id, "parquet_file"),
        3,
        "a reader on the older manifest still has its grace"
    );
}

#[test]
fn a_file_unnamed_past_the_grace_is_removed_by_a_server_whose_clock_is_hours_behind() {
    let w = packed_world();
    let (_first, id) = two_files_unnamed(&w);
    older(
        &w,
        &id.to_string(),
        "parquet_file",
        "unnamed_at",
        GRACE + Span::MINUTE,
    );
    remove_expired_at(&w, id, conformance::START - SKEW, Vec::new());
    assert_eq!(
        w.rows(id, "parquet_file"),
        1,
        "both unnamed files are gone; the merged one stays"
    );
}

#[test]
fn a_file_is_removed_once_the_grace_passes_on_the_database_s_clock() {
    let w = packed_world();
    let (_first, id) = two_files_unnamed(&w);
    let grace = i64::try_from(GRACE.whole_nanoseconds()).expect("fits");
    remove_expired_at(&w, id, conformance::START - SKEW, vec![grace + SECOND]);
    assert_eq!(
        w.rows(id, "parquet_file"),
        1,
        "the grace has passed by the clock the files were unnamed by"
    );
}

#[test]
fn a_file_a_minute_inside_the_grace_on_the_database_s_clock_is_kept() {
    let w = packed_world();
    let (_first, id) = two_files_unnamed(&w);
    older(
        &w,
        &id.to_string(),
        "parquet_file",
        "unnamed_at",
        GRACE - Span::MINUTE,
    );
    remove_expired_at(&w, id, conformance::START + DAYS, Vec::new());
    assert_eq!(
        w.rows(id, "parquet_file"),
        3,
        "a minute of the grace is left by the database's clock"
    );
}

// Manifests, and the reader grace after the next one is written.

/// A tenant whose server wrote two manifests, its host clock at
/// conformance::START. Answers the server, the tenant and the server's
/// clock. The manifests are judged on this server, its clock moved: the
/// start-up recovery of another server's store removes every manifest but
/// the newest of its own accord (`Store::finish_purges`).
fn two_manifests(w: &World) -> (Catalogs, Tenants, Arc<TestClock>, TenantId) {
    let (catalogs, tenants, clock) = w.serve_clocked();
    let id = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let (plain, _) = plans();
    let packed = tenants.store(id).expect("the store");
    for n in 1..=2 {
        store(&tenants, id, &caller, n);
        packed.pack(&plain).expect("packed");
    }
    assert_eq!(
        number(
            w,
            &id.to_string(),
            "SELECT count(*) FROM vcon.manifest WHERE published_at IS NOT NULL"
        ),
        2,
        "two manifests are written"
    );
    (catalogs, tenants, clock, id)
}

/// The manifest pruning of the server of `written`, its host clock moved
/// to `at`: a host whose clock is not the writer's.
fn prune_at(
    written: &(Catalogs, Tenants, Arc<TestClock>, TenantId),
    at: time::OffsetDateTime,
    steps: Vec<i64>,
) {
    let (_, tenants, clock, id) = written;
    clock.set(at);
    let store = tenants.store(*id).expect("the store");
    store.step_clock(steps);
    store
        .prune_manifests(Some(std::time::Duration::from_secs(900)))
        .expect("the pruning runs");
}

#[test]
fn a_manifest_is_marked_written_at_the_database_s_time() {
    let w = packed_world();
    let (_, _, _, id) = two_manifests(&w);
    assert_eq!(
        by_the_database(&w, &id.to_string(), "manifest", "published_at"),
        "2/2",
        "each manifest's time is the database's, not the first server's clock"
    );
}

#[test]
fn an_older_manifest_is_kept_by_a_server_whose_clock_is_hours_ahead() {
    let w = packed_world();
    let written = two_manifests(&w);
    let id = written.3;
    prune_at(&written, conformance::START + SKEW, Vec::new());
    assert_eq!(
        w.rows(id, "manifest"),
        2,
        "the newer manifest was written just now: a reader of the older has its grace"
    );
}

#[test]
fn an_older_manifest_past_the_grace_is_removed_by_a_server_whose_clock_is_hours_behind() {
    let w = packed_world();
    let written = two_manifests(&w);
    let id = written.3;
    older(
        &w,
        &id.to_string(),
        "manifest",
        "published_at",
        GRACE + Span::MINUTE,
    );
    prune_at(&written, conformance::START - SKEW, Vec::new());
    assert_eq!(w.rows(id, "manifest"), 1, "only the newest manifest stays");
}

#[test]
fn an_older_manifest_goes_once_the_grace_passes_on_the_database_s_clock() {
    let w = packed_world();
    let written = two_manifests(&w);
    let id = written.3;
    let grace = i64::try_from(GRACE.whole_nanoseconds()).expect("fits");
    prune_at(&written, conformance::START - SKEW, vec![grace + SECOND]);
    assert_eq!(w.rows(id, "manifest"), 1, "only the newest manifest stays");
}

// Store times, retention and the order of versions.

/// A tenant whose first server stored vCon 1 and deleted it, under a
/// retention of a day.
fn deleted_on_the_first(w: &World) -> ((Catalogs, Tenants), TenantId) {
    let first = w.serve();
    let id = first.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let uuid = store(&first.1, id, &caller, 1);
    first
        .1
        .store(id)
        .expect("the store")
        .delete(&caller, uuid, Retention::Days(1))
        .expect("deleted");
    (first, id)
}

/// The purge sweep of a server whose host clock is at `at`, under a
/// retention of a day: how many versions it erased.
fn purged_at(w: &World, id: TenantId, at: time::OffsetDateTime, steps: Vec<i64>) -> u64 {
    let other = w.serve_at(at);
    let store = other.1.store(id).expect("the other's store");
    store.step_clock(steps);
    let purged = store
        .purge_due(Retention::Days(1))
        .expect("the sweep runs")
        .purged;
    drop(store);
    stop(other);
    purged
}

#[test]
fn a_version_is_stored_at_the_database_s_time() {
    let w = world();
    let (_first, id) = deleted_on_the_first(&w);
    assert_eq!(
        by_the_database(&w, &id.to_string(), "version", "stored_at"),
        "1/1",
        "the store time is the database's, not the first server's clock"
    );
    assert_eq!(
        by_the_database(&w, &id.to_string(), "vcon", "stored_at"),
        "1/1",
        "and the uuid's"
    );
}

#[test]
fn a_version_inside_its_retention_is_kept_by_a_server_whose_clock_is_days_ahead() {
    let w = world();
    let (_first, id) = deleted_on_the_first(&w);
    assert_eq!(
        purged_at(&w, id, conformance::START + DAYS, Vec::new()),
        0,
        "the version was stored just now: a day of retention is left"
    );
    assert_eq!(w.rows(id, "version"), 1, "its row is kept");
}

#[test]
fn a_version_past_its_retention_is_purged_by_a_server_whose_clock_is_days_behind() {
    let w = world();
    let (_first, id) = deleted_on_the_first(&w);
    let scope = id.to_string();
    older(&w, &scope, "version", "stored_at", Span::days(2));
    older(&w, &scope, "vcon", "stored_at", Span::days(2));
    assert_eq!(
        purged_at(&w, id, conformance::START - DAYS, Vec::new()),
        1,
        "two days old by the database's clock, under a day of retention"
    );
    assert_eq!(w.rows(id, "version"), 0, "its row is gone");
}

#[test]
fn a_version_is_purged_once_its_retention_passes_on_the_database_s_clock() {
    let w = world();
    let (_first, id) = deleted_on_the_first(&w);
    assert_eq!(
        purged_at(
            &w,
            id,
            conformance::START - DAYS,
            vec![86_400 * SECOND + SECOND]
        ),
        1,
        "a day and a second later by the database's clock"
    );
}

#[test]
fn a_version_a_second_inside_its_retention_on_the_database_s_clock_is_kept() {
    let w = world();
    let (_first, id) = deleted_on_the_first(&w);
    assert_eq!(
        purged_at(
            &w,
            id,
            conformance::START + DAYS,
            vec![86_400 * SECOND - 60 * SECOND]
        ),
        0,
        "a minute of retention is left by the database's clock"
    );
}

#[test]
fn a_vcon_stored_later_on_a_server_whose_clock_is_behind_is_listed_first() {
    let w = world();
    let ahead = w.serve_at(conformance::START + SKEW);
    let id = ahead.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let earlier = store(&ahead.1, id, &caller, 1);
    let behind = w.serve_at(conformance::START - SKEW);
    let later = store(&behind.1, id, &caller, 2);
    let query = ListQuery::new(Some(10), None, None, None).expect("a query");
    let listed: Vec<uuid::Uuid> = behind
        .1
        .store(id)
        .expect("the store")
        .list(&caller, &query)
        .expect("lists")
        .vcons
        .iter()
        .map(|v| v.uuid)
        .collect();
    assert_eq!(
        listed,
        [later, earlier],
        "newest first, by the database's clock"
    );
}

#[test]
fn a_version_stored_later_on_a_server_whose_clock_is_behind_is_the_later_one() {
    let w = world();
    let ahead = w.serve_at(conformance::START + SKEW);
    let id = ahead.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let (found, body) = conformance::vcon(&uuid_of(1), "first");
    let first = ahead
        .1
        .store(id)
        .expect("the store")
        .create(&caller, &found, &body)
        .expect("stored");
    let crate::store::Created::New(first) = first else {
        panic!("a new vCon: {first:?}")
    };
    let behind = w.serve_at(conformance::START - SKEW);
    let store = behind.1.store(id).expect("the store");
    let (_, second) = conformance::vcon(&uuid_of(1), "second");
    let next = store
        .replace(&caller, first, &found, &second)
        .expect("replaced");
    let kept: Vec<_> = store
        .legal_versions(&caller, found.uuid)
        .expect("lists")
        .iter()
        .map(|v| (v.version, v.current))
        .collect();
    assert_eq!(
        kept,
        [(first, false), (next, true)],
        "oldest first, by the database's clock"
    );
}

#[test]
fn two_writes_of_one_tenant_read_the_database_s_clock_in_their_order() {
    let w = world();
    let first = w.serve_at(conformance::START + SKEW);
    let id = first.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let second = w.serve_at(conformance::START - SKEW);
    let caller = who();
    for n in 1..=4 {
        let on = if n % 2 == 0 { &second.1 } else { &first.1 };
        store(on, id, &caller, n);
    }
    assert_eq!(
        w.value_in(
            &id.to_string(),
            "SELECT bool_and(ordered)::text FROM (SELECT stored_at > lag(stored_at) OVER \
             (ORDER BY seq) AS ordered FROM vcon.version) AS t WHERE ordered IS NOT NULL"
        ),
        "true",
        "each version's store time is later than the one written before it"
    );
}

// Sessions.

/// A tenant made through `tenants`, and a user who reads in it.
fn member(tenants: &Tenants) -> (TenantId, crate::ids::UserId) {
    let id = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
    let user = tenants
        .add_user(Actor::Cli, "Member", None)
        .expect("a user");
    tenants
        .grant_membership(Actor::Cli, user, Some(id), Roles::of(&[Role::Reader]))
        .expect("granted");
    (id, user)
}

/// A session started on a first server whose host clock is at `at`.
fn started_at(w: &World, at: time::OffsetDateTime) -> ((Catalogs, Tenants), Started) {
    let first = w.serve_at(at);
    let (id, user) = member(&first.1);
    let started = first
        .1
        .start_session(user, Some(id), &["hwk"])
        .expect("a session");
    (first, started)
}

/// The session's idle limit, in nanoseconds: the default, 15 minutes.
const IDLE: i64 = 900 * SECOND;

/// A request with the session's cookie and token, to `tenants`.
fn asked(tenants: &Tenants, s: &Started) -> Result<(), SessionError> {
    tenants.session(&s.id, Proof::Csrf(Some(&s.csrf))).map(drop)
}

#[test]
fn a_session_is_started_at_the_database_s_time() {
    let w = world();
    let (_first, _) = started_at(&w, conformance::START);
    assert_eq!(
        by_the_database(&w, "registry", "session", "created_at"),
        "1/1",
        "the start is the database's, not the first server's clock"
    );
    assert_eq!(
        w.registry_value("SELECT (idle_expires_at - created_at)::text FROM vcon.session"),
        IDLE.to_string(),
        "the idle limit is counted from it"
    );
}

#[test]
fn a_session_started_on_one_server_is_live_on_a_server_whose_clock_is_hours_ahead() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    let ahead = w.serve_at(conformance::START + SKEW);
    assert_eq!(
        asked(&ahead.1, &s),
        Ok(()),
        "the session was started just now"
    );
}

#[test]
fn a_session_seen_on_a_server_whose_clock_is_hours_ahead_moves_its_limit_by_the_database() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    let ahead = w.serve_at(conformance::START + SKEW);
    asked(&ahead.1, &s).expect("live");
    assert_eq!(
        w.registry_value(&format!(
            "SELECT (abs(idle_expires_at - {IDLE} - {DB_NOW}) < 60000000000)::text FROM vcon.session"
        )),
        "true",
        "the idle limit is the idle time from the database's clock"
    );
}

#[test]
fn a_session_ends_on_a_server_whose_clock_is_hours_ahead() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    let ahead = w.serve_at(conformance::START + SKEW);
    assert!(ahead.1.end_session(&s.id).is_ok(), "the live session ends");
    assert_eq!(w.registry_rows("session"), 0, "and its row is gone");
}

#[test]
fn a_session_steps_up_on_a_server_whose_clock_is_hours_ahead() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    let ahead = w.serve_at(conformance::START + SKEW);
    let stepped = ahead
        .1
        .step_up_session(&s.id, &["hwk", "otp"])
        .expect("the live session steps up");
    assert_eq!(asked(&ahead.1, &stepped), Ok(()), "under its new id");
}

#[test]
fn a_session_past_its_idle_limit_on_the_database_s_clock_ends_on_a_server_behind() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    let behind = w.serve_at(conformance::START - SKEW);
    behind.1.step_clock(vec![IDLE + SECOND]);
    assert_eq!(
        asked(&behind.1, &s),
        Err(SessionError::NoSession),
        "the idle limit has passed by the clock it was counted on"
    );
    assert_eq!(w.registry_rows("session"), 0, "and its row is removed");
}

#[test]
fn a_session_at_its_idle_limit_exactly_on_the_database_s_clock_has_ended() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    older(
        &w,
        "registry",
        "session",
        "idle_expires_at",
        Span::seconds(900),
    );
    let behind = w.serve_at(conformance::START - SKEW);
    assert_eq!(
        asked(&behind.1, &s),
        Err(SessionError::NoSession),
        "a limit at or before the database's clock has passed"
    );
}

#[test]
fn a_session_past_its_idle_limit_on_the_database_s_clock_cannot_end_again() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    let behind = w.serve_at(conformance::START - SKEW);
    behind.1.step_clock(vec![IDLE + SECOND]);
    assert_eq!(
        behind.1.end_session(&s.id).map(drop),
        Err(SessionError::NoSession),
        "no live session has the id"
    );
}

#[test]
fn a_session_past_its_idle_limit_on_the_database_s_clock_cannot_step_up() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    let behind = w.serve_at(conformance::START - SKEW);
    behind.1.step_clock(vec![IDLE + SECOND]);
    assert_eq!(
        behind.1.step_up_session(&s.id, &["hwk", "otp"]).map(drop),
        Err(SessionError::NoSession),
        "no live session has the id"
    );
}

#[test]
fn a_start_on_a_server_behind_removes_the_sessions_ended_on_the_database_s_clock() {
    let w = world();
    let (_first, _ended) = started_at(&w, conformance::START);
    let behind = w.serve_at(conformance::START - SKEW);
    let user = behind
        .1
        .add_user(Actor::Cli, "Operator", None)
        .expect("a user");
    behind
        .1
        .grant_membership(Actor::Cli, user, None, Roles::of(&[Role::Operator]))
        .expect("granted");
    behind.1.step_clock(vec![IDLE + SECOND]);
    behind
        .1
        .start_session(user, None, &["hwk"])
        .expect("a session");
    assert_eq!(
        w.registry_rows("session"),
        1,
        "the first session ended by the database's clock and was removed"
    );
}

// Access keys.

/// A key of a day's life made on a first server whose host clock is
/// START, and its header.
fn key_made(w: &World) -> ((Catalogs, Tenants), String) {
    key_made_at(w, conformance::START)
}

/// `key_made`, the first server's host clock at `at`.
fn key_made_at(w: &World, at: time::OffsetDateTime) -> ((Catalogs, Tenants), String) {
    let first = w.serve_at(at);
    let id = first.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let made = first
        .1
        .create_key_for(
            Actor::Cli,
            Some(id),
            Scopes::of(&[Scope::Read]),
            "on two servers",
            KeyLife::Days(1),
        )
        .expect("a key");
    (first, format!("Bearer {}", made.token))
}

/// The answer to a request with `header` on a server whose host clock is
/// at `at`.
fn authenticated_at(
    w: &World,
    header: &str,
    at: time::OffsetDateTime,
    steps: Vec<i64>,
) -> Result<(), AuthError> {
    let other = w.serve_at(at);
    other.1.step_clock(steps);
    let answer = access::authenticate(&other.1, Some(header)).map(drop);
    stop(other);
    answer
}

#[test]
fn a_key_s_expiry_is_counted_from_the_database_s_clock() {
    let w = world();
    let (_first, _) = key_made(&w);
    assert_eq!(
        w.registry_value(&format!(
            "SELECT (abs(expires_at - 86400000000000 - {DB_NOW}) < 60000000000)::text \
             FROM vcon.api_key"
        )),
        "true",
        "a day from the database's clock, not the first server's"
    );
}

#[test]
fn a_key_inside_its_life_is_accepted_by_a_server_whose_clock_is_days_ahead() {
    let w = world();
    let (_first, header) = key_made(&w);
    assert_eq!(
        authenticated_at(&w, &header, conformance::START + DAYS, Vec::new()),
        Ok(()),
        "the key was made just now"
    );
}

#[test]
fn a_key_past_its_life_on_the_database_s_clock_is_refused_by_a_server_days_behind() {
    let w = world();
    let (_first, header) = key_made(&w);
    assert_eq!(
        authenticated_at(
            &w,
            &header,
            conformance::START - DAYS,
            vec![86_400 * SECOND + SECOND]
        ),
        Err(AuthError::Expired),
        "a day and a second later by the clock its life was counted on"
    );
}

#[test]
fn a_key_at_its_expiry_exactly_on_the_database_s_clock_is_refused() {
    let w = world();
    let (_first, header) = key_made(&w);
    older(&w, "registry", "api_key", "expires_at", Span::days(1));
    assert_eq!(
        authenticated_at(&w, &header, conformance::START - DAYS, Vec::new()),
        Err(AuthError::Expired),
        "an expiry at or before the database's clock has passed"
    );
}

#[test]
fn a_key_s_use_is_noted_at_the_database_s_minute() {
    let w = world();
    let (_first, header) = key_made(&w);
    authenticated_at(&w, &header, conformance::START + SKEW, Vec::new()).expect("accepted");
    assert_eq!(
        w.registry_value(&format!(
            "SELECT (abs(last_used_at - {DB_NOW}) < 120000000000)::text FROM vcon.api_key"
        )),
        "true",
        "the last-used time is the database's minute, not the server's"
    );
}

#[test]
fn a_later_use_on_a_server_behind_moves_the_last_used_time_on() {
    let w = world();
    let (_first, header) = key_made(&w);
    authenticated_at(&w, &header, conformance::START + SKEW, Vec::new()).expect("accepted");
    let noted = number(&w, "registry", "SELECT last_used_at FROM vcon.api_key");
    authenticated_at(&w, &header, conformance::START - SKEW, vec![120 * SECOND]).expect("accepted");
    let later = number(&w, "registry", "SELECT last_used_at FROM vcon.api_key");
    assert!(
        later > noted,
        "two minutes later by the database's clock is noted: {noted} then {later}"
    );
}

// The rotation backlog: purges and removals since a key's rotation.

#[test]
fn a_purge_after_a_rotation_on_a_server_ahead_counts_toward_the_next() {
    let w = world();
    let ahead = w.serve_at(conformance::START + SKEW);
    let id = ahead.1.create(Actor::Cli, "acme", None).expect("a tenant");
    ahead.1.rekey_tenant(Actor::Cli, id).expect("rotated");
    let behind = w.serve_at(conformance::START - SKEW);
    let caller = who();
    let uuid = store(&behind.1, id, &caller, 1);
    behind
        .1
        .store(id)
        .expect("the store")
        .delete(&caller, uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(
        ahead.1.rotation_backlog().expect("the backlog").purges,
        1,
        "the purge came after the rotation by the database's clock"
    );
}

#[test]
fn a_removal_after_the_master_key_s_first_use_on_a_server_ahead_counts() {
    let w = world();
    let ahead = w.serve_at(conformance::START + AHEAD_OF_THE_DATABASE);
    ahead.1.create(Actor::Cli, "first", None).expect("a tenant");
    let behind = w.serve_at(conformance::START - SKEW);
    let id = behind
        .1
        .create(Actor::Cli, "second", None)
        .expect("a tenant");
    behind
        .1
        .remove(Actor::Cli, id, Removal::Empty)
        .expect("removed");
    assert_eq!(
        ahead.1.rotation_backlog().expect("the backlog").removals,
        1,
        "the removal came after the master key's use began, by the database's clock"
    );
}

// Audit records written by a host's clock, as every record of an earlier
// release was (and a counted record still is), beside records written by
// the database's.

/// The tenant log's records as `audit_page` gives them: seq and time.
fn logged(store: &crate::store::Store) -> Vec<(i64, String)> {
    let query = crate::audit::AuditQuery {
        after: 0,
        limit: 1000,
    };
    let page = store.audit_page(&who(), &query).expect("a page");
    String::from_utf8(page)
        .expect("UTF-8")
        .lines()
        .map(|line| {
            let record: serde_json::Value = serde_json::from_str(line).expect("JSON");
            (
                record["seq"].as_i64().expect("a seq"),
                record["at"].as_str().expect("a time").to_owned(),
            )
        })
        .collect()
}

#[test]
fn records_of_a_host_s_clock_and_of_the_database_s_are_listed_in_their_order() {
    let w = world();
    let first = w.serve();
    let id = first.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let store_of = first.1.store(id).expect("the store");
    let caller = who();
    let counted = crate::audit::Entry {
        actor: Actor::Key(caller.key),
        action: crate::audit::Action::Read,
        target: None,
        version: None,
        status: 401,
    };
    store_of
        .count(&counted)
        .expect("counted at the host's minute");
    store(&first.1, id, &caller, 1);
    let log = logged(&store_of);
    let host = crate::audit::rfc3339(conformance::START);
    let at_host = log
        .iter()
        .position(|(_, at)| *at == host)
        .unwrap_or_else(|| panic!("the counted record keeps the host's time: {log:?}"));
    let after = &log[at_host + 1..];
    assert!(
        !after.is_empty(),
        "the create's records come after it: {log:?}"
    );
    let db = number(&w, "registry", &format!("SELECT {DB_NOW}"));
    for (seq, at) in after {
        let at = time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339)
            .expect("a time");
        assert!(
            (crate::clock::nanos(at) - db).abs() < 60 * SECOND,
            "record {seq} is at the database's time, a month after the one before it"
        );
    }
    assert!(
        log.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "the page is in seq order whatever the times say: {log:?}"
    );
}

#[test]
fn a_purge_after_the_change_counts_against_a_rotation_an_earlier_release_recorded() {
    let w = world();
    let first = w.serve();
    let id = first.1.create(Actor::Cli, "acme", None).expect("a tenant");
    // An earlier release rotated the key at its host's clock, a month
    // behind the database's.
    w.as_owner(&format!(
        "ALTER TABLE vcon.tenant DISABLE ROW LEVEL SECURITY; UPDATE vcon.tenant SET key_since \
         = {}; ALTER TABLE vcon.tenant ENABLE ROW LEVEL SECURITY",
        crate::clock::nanos(conformance::START)
    ));
    let caller = who();
    let uuid = store(&first.1, id, &caller, 1);
    first
        .1
        .store(id)
        .expect("the store")
        .delete(&caller, uuid, Retention::Days(0))
        .expect("deleted and purged");
    let backlog = first.1.rotation_backlog().expect("the backlog");
    assert_eq!(backlog.purges, 1, "the purge is after the rotation");
    let oldest = crate::clock::nanos(backlog.oldest_purge.expect("a purge"));
    let db = number(&w, "registry", &format!("SELECT {DB_NOW}"));
    assert!(
        (oldest - db).abs() < 60 * SECOND,
        "and is at the database's time: {oldest} and {db}"
    );
}

// More of each: the other direction of skew, and the exact limits.

#[test]
fn a_file_unnamed_exactly_the_grace_ago_on_the_database_s_clock_is_removed() {
    let w = packed_world();
    let (_first, id) = two_files_unnamed(&w);
    older(&w, &id.to_string(), "parquet_file", "unnamed_at", GRACE);
    remove_expired_at(&w, id, conformance::START - SKEW, Vec::new());
    assert_eq!(
        w.rows(id, "parquet_file"),
        1,
        "the grace has passed at its end, by the database's clock"
    );
}

#[test]
fn an_older_manifest_exactly_the_grace_past_on_the_database_s_clock_is_removed() {
    let w = packed_world();
    let written = two_manifests(&w);
    let id = written.3;
    older(&w, &id.to_string(), "manifest", "published_at", GRACE);
    prune_at(&written, conformance::START - SKEW, Vec::new());
    assert_eq!(w.rows(id, "manifest"), 1, "only the newest manifest stays");
}

#[test]
fn a_version_exactly_its_retention_old_on_the_database_s_clock_is_purged() {
    let w = world();
    let (_first, id) = deleted_on_the_first(&w);
    let scope = id.to_string();
    older(&w, &scope, "version", "stored_at", Span::days(1));
    older(&w, &scope, "vcon", "stored_at", Span::days(1));
    assert_eq!(
        purged_at(&w, id, conformance::START - DAYS, Vec::new()),
        1,
        "a day old by the database's clock, under a day of retention"
    );
}

#[test]
fn a_vcon_s_store_time_and_its_first_version_s_are_one_reading() {
    let w = world();
    let (_first, id) = deleted_on_the_first(&w);
    assert_eq!(
        w.value_in(
            &id.to_string(),
            "SELECT (v.stored_at = c.stored_at)::text FROM vcon.version v, vcon.vcon c"
        ),
        "true",
        "one transaction, one reading of the database's clock"
    );
    assert_eq!(
        by_the_database(&w, &id.to_string(), "vcon", "stored_at"),
        "1/1",
        "and that reading is the database's, not the server's clock"
    );
}

#[test]
fn a_replaced_version_is_hidden_at_its_successor_s_store_time() {
    let w = world();
    let ahead = w.serve_at(conformance::START + SKEW);
    let id = ahead.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let (found, body) = conformance::vcon(&uuid_of(1), "first");
    let crate::store::Created::New(first) = ahead
        .1
        .store(id)
        .expect("the store")
        .create(&caller, &found, &body)
        .expect("stored")
    else {
        panic!("a new vCon")
    };
    let (_, second) = conformance::vcon(&uuid_of(1), "second");
    ahead
        .1
        .store(id)
        .expect("the store")
        .replace(&caller, first, &found, &second)
        .expect("replaced");
    assert_eq!(
        w.value_in(
            &id.to_string(),
            "SELECT (h.hidden_at = n.stored_at)::text FROM vcon.version h, vcon.version n \
             WHERE h.hidden_at IS NOT NULL AND n.hidden_at IS NULL"
        ),
        "true",
        "the replace's one reading of the database's clock"
    );
    assert_eq!(
        by_the_database(&w, &id.to_string(), "version", "hidden_at"),
        "1/1",
        "and it is the database's"
    );
}

#[test]
fn a_session_started_on_a_server_behind_is_live_on_a_server_ahead() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START - SKEW);
    let ahead = w.serve_at(conformance::START + SKEW);
    assert_eq!(
        asked(&ahead.1, &s),
        Ok(()),
        "four hours apart, started just now"
    );
}

#[test]
fn a_session_past_its_absolute_limit_on_the_database_s_clock_ends_on_a_server_behind() {
    let w = world();
    let (_first, s) = started_at(&w, conformance::START);
    older(&w, "registry", "session", "expires_at", Span::hours(8));
    let behind = w.serve_at(conformance::START - SKEW);
    assert_eq!(
        asked(&behind.1, &s),
        Err(SessionError::NoSession),
        "seen inside its idle limit, but its whole life has passed"
    );
}

#[test]
fn a_key_made_on_a_server_days_behind_is_accepted_on_one_days_ahead() {
    let w = world();
    let (_first, header) = key_made_at(&w, conformance::START - DAYS);
    assert_eq!(
        authenticated_at(&w, &header, conformance::START + DAYS, Vec::new()),
        Ok(()),
        "six days apart, made just now with a day's life"
    );
}

#[test]
fn a_key_made_on_a_server_ahead_of_the_database_expires_a_day_from_the_database_s_clock() {
    let w = world();
    let (_first, _) = key_made_at(&w, conformance::START + AHEAD_OF_THE_DATABASE);
    assert_eq!(
        w.registry_value(&format!(
            "SELECT (abs(expires_at - 86400000000000 - {DB_NOW}) < 60000000000)::text \
             FROM vcon.api_key"
        )),
        "true",
        "a day from the database's clock, not from the host's, which is ahead of it"
    );
}

#[test]
fn a_key_made_on_a_server_ahead_is_refused_behind_once_its_life_passes() {
    let w = world();
    let (_first, header) = key_made_at(&w, conformance::START + DAYS);
    assert_eq!(
        authenticated_at(
            &w,
            &header,
            conformance::START - DAYS,
            vec![86_400 * SECOND + SECOND]
        ),
        Err(AuthError::Expired),
        "a day and a second later by the database's clock"
    );
}

#[test]
fn a_use_on_a_server_ahead_of_the_database_notes_no_minute_still_to_come() {
    let w = world();
    let (_first, header) = key_made_at(&w, conformance::START + AHEAD_OF_THE_DATABASE);
    authenticated_at(
        &w,
        &header,
        conformance::START + AHEAD_OF_THE_DATABASE + SKEW,
        Vec::new(),
    )
    .expect("accepted");
    assert_eq!(
        w.registry_value(&format!(
            "SELECT (last_used_at <= {DB_NOW})::text FROM vcon.api_key"
        )),
        "true",
        "the last-used minute has begun on the database's clock"
    );
}

#[test]
fn a_purge_before_a_rotation_on_a_server_behind_does_not_count() {
    let w = world();
    let ahead = w.serve_at(conformance::START + SKEW);
    let id = ahead.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let uuid = store(&ahead.1, id, &caller, 1);
    ahead
        .1
        .store(id)
        .expect("the store")
        .delete(&caller, uuid, Retention::Days(0))
        .expect("deleted and purged");
    let behind = w.serve_at(conformance::START - SKEW);
    behind.1.rekey_tenant(Actor::Cli, id).expect("rotated");
    assert_eq!(
        behind.1.rotation_backlog().expect("the backlog").purges,
        0,
        "the purge came before the rotation by the database's clock"
    );
}

#[test]
fn a_purge_after_a_create_on_a_server_ahead_counts_against_the_tenant_s_key() {
    let w = world();
    let ahead = w.serve_at(conformance::START + AHEAD_OF_THE_DATABASE);
    let id = ahead.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let behind = w.serve_at(conformance::START - SKEW);
    let caller = who();
    let uuid = store(&behind.1, id, &caller, 1);
    behind
        .1
        .store(id)
        .expect("the store")
        .delete(&caller, uuid, Retention::Days(0))
        .expect("deleted and purged");
    assert_eq!(
        ahead.1.rotation_backlog().expect("the backlog").purges,
        1,
        "the tenant key's use began at the create, before the purge, by the database's clock"
    );
}

#[test]
fn a_create_s_record_is_at_its_version_s_store_time() {
    let w = world();
    let first = w.serve();
    let id = first.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    store(&first.1, id, &caller, 1);
    let stored = number(&w, &id.to_string(), "SELECT stored_at FROM vcon.version");
    let store_of = first.1.store(id).expect("the store");
    let at = crate::audit::rfc3339(crate::clock::from_nanos(stored).expect("a time"));
    assert!(
        logged(&store_of).iter().any(|(_, logged)| *logged == at),
        "one reading of the database's clock for the transaction: {at}"
    );
    assert_eq!(
        by_the_database(&w, &id.to_string(), "version", "stored_at"),
        "1/1",
        "and that reading is the database's, not the server's clock"
    );
}

#[test]
fn a_write_that_waits_for_its_tenant_s_lock_reads_the_clock_after_it() {
    let w = world();
    let first = w.serve();
    let id = first.1.create(Actor::Cli, "acme", None).expect("a tenant");
    let caller = who();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a runtime");
    let holder = runtime.block_on(w.db.app());
    let store_of = first.1.store(id).expect("the store");
    let (hook, paused) = pause();
    store_of.on_between(Box::new(hook));
    let (found, body) = conformance::vcon(&uuid_of(1), "after the lock");
    let released = std::thread::scope(|s| {
        let writing = s.spawn(|| store_of.create(&caller, &found, &body));
        // Between its object and its row: the write's last transaction,
        // which reads the store time, is the one that waits.
        paused.wait();
        runtime
            .block_on(holder.batch_execute(&format!(
                "BEGIN; SET LOCAL vcon.tenant = '{id}'; \
                 SELECT 1 FROM vcon.tenant_lock WHERE tenant = '\\x{id}' FOR UPDATE"
            )))
            .expect("the tenant's lock is held");
        paused.release();
        // Until the write waits for the lock, or a minute: the lock is
        // given up either way, so no failure here leaves the write waiting.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let waited = loop {
            let waiting = runtime
                .block_on(holder.query_one(
                    "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() \
                     AND wait_event_type = 'Lock'",
                    &[],
                ))
                .map(|row| row.get::<_, i64>(0) > 0);
            match waiting {
                Ok(true) => break true,
                Err(_) => break false,
                Ok(false) if std::time::Instant::now() >= deadline => break false,
                // The timer is made inside the runtime: one made outside
                // it finds no timer to run on.
                Ok(false) => runtime.block_on(async {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }),
            }
        };
        let released = runtime
            .block_on(holder.query_one(&format!("SELECT {DB_NOW}"), &[]))
            .map(|row| row.get::<_, i64>(0));
        let given_up = runtime.block_on(holder.batch_execute("COMMIT"));
        let written = writing.join().expect("the write ends");
        assert!(waited, "the write waited for the lock");
        given_up.expect("the lock is given up");
        assert!(written.is_ok(), "the write commits: {written:?}");
        released.expect("the clock")
    });
    let stored = number(
        &w,
        &id.to_string(),
        "SELECT max(stored_at) FROM vcon.version",
    );
    assert!(
        stored >= released,
        "the store time is read after the lock: {stored} is not before {released}"
    );
}
