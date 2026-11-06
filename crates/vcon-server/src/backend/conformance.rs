//! The store conformance suite: every outcome of every Store call that a
//! caller can see, written once against Store's public interface, and run
//! on every backend combination. A backend passes when this suite does;
//! the races and crash points that need the local backend's hooks stay in
//! Store's own tests.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use base64::Engine;
use time::OffsetDateTime;
use uuid::Uuid;
use vcon::read::{Inspection, Limits, inspect};

use crate::backend::Objects;
use crate::clock::Clock;
use crate::envelope::Key;
use crate::ids::VersionId;
use crate::retention::Retention;
use crate::store::{Caller, Created, Cursor, ListQuery, Store, StoreError};

/// A clock a test moves by hand.
pub(crate) struct TestClock(Mutex<OffsetDateTime>);

impl TestClock {
    pub(crate) fn at(now: OffsetDateTime) -> Arc<TestClock> {
        Arc::new(TestClock(Mutex::new(now)))
    }

    pub(crate) fn set(&self, now: OffsetDateTime) {
        *self.0.lock().expect("clock") = now;
    }
}

impl Clock for TestClock {
    fn now(&self) -> OffsetDateTime {
        *self.0.lock().expect("clock")
    }
}

/// What the suite needs of a backend combination: a new, empty Store for
/// each check, all on `clock`, and the same Store opened again.
pub(crate) trait Harness {
    /// A new tenant's Store, empty, opened with start-up recovery.
    fn fresh(&self, clock: Arc<TestClock>) -> Fresh;
}

/// Writes an object under any name, as a crash or another program would
/// leave it.
pub(crate) type Plant = Box<dyn Fn(&str, &[u8])>;

/// A Store from `Harness::fresh`, a way to open it again, and its objects
/// reached around it, so a check can see what is really kept.
pub(crate) struct Fresh {
    pub(crate) store: Store,
    pub(crate) reopen: Box<dyn Fn() -> Store>,
    /// The objects the Store reads and writes.
    pub(crate) objects: Arc<dyn Objects>,
    /// Plants an object around the Store.
    pub(crate) plant: Plant,
    /// The same Store opened again, without start-up recovery, on objects
    /// it cannot read: every read fails, and not as an absent object
    /// (local: files the process may not read; S3: a service that does not
    /// answer).
    pub(crate) unreadable: Box<dyn Fn() -> Store>,
}

/// The moment every check starts at: 2026-10-23 12:00 UTC.
pub(crate) const START: OffsetDateTime = match OffsetDateTime::from_unix_timestamp(1_792_756_800) {
    Ok(at) => at,
    Err(_) => panic!("a valid time"),
};

/// A check of the suite, by name.
type Check = (&'static str, fn(&dyn Harness));

/// Every check, in order; each panics with its name on failure.
pub(crate) fn run(harness: &dyn Harness) {
    let checks: [Check; 17] = [
        ("create_then_get", create_then_get),
        ("create_same_conflict_gone", create_same_conflict_gone),
        ("forms_read_back", forms_read_back),
        ("replace_rules", replace_rules),
        ("delete_and_get", delete_and_get),
        ("purge_timing_and_hold", purge_timing_and_hold),
        ("legal_reads_and_restore", legal_reads_and_restore),
        ("list_pages", list_pages),
        ("audit_records_and_pages", audit_records_and_pages),
        ("rewrap_moves_every_key", rewrap_moves_every_key),
        ("check_and_counts", check_and_counts),
        ("reopen_keeps_everything", reopen_keeps_everything),
        ("purge_removes_the_object", purge_removes_the_object),
        (
            "recovery_removes_only_garbage",
            recovery_removes_only_garbage,
        ),
        (
            "purge_of_a_lost_object_finishes",
            purge_of_a_lost_object_finishes,
        ),
        (
            "a_lost_or_altered_object_is_damage",
            a_lost_or_altered_object_is_damage,
        ),
        (
            "an_object_the_storage_cannot_read_is_a_failure",
            an_object_the_storage_cannot_read_is_a_failure,
        ),
    ];
    for (name, check) in checks {
        eprintln!("conformance: {name}");
        check(harness);
    }
}

fn caller() -> Caller {
    Caller {
        key: Key::generate().expect("key").id(),
    }
}

/// An unsigned vCon of `uuid`.
pub(crate) fn vcon(uuid: &str, subject: &str) -> (Inspection, Vec<u8>) {
    let body = format!(
        r#"{{"uuid": "{uuid}", "created_at": "2026-10-23T12:00:00Z", "subject": "{subject}", "parties": []}}"#
    )
    .into_bytes();
    (found(&body), body)
}

fn found(body: &[u8]) -> Inspection {
    inspect(body, None, &Limits::new(1 << 20), &BTreeSet::new()).expect("a valid vCon")
}

fn uuid(n: u8) -> String {
    format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}")
}

fn new_version(store: &Store, who: &Caller, uuid: &str, subject: &str) -> (VersionId, Vec<u8>) {
    let (found, body) = vcon(uuid, subject);
    match store.create(who, &found, &body) {
        Ok(Created::New(version)) => (version, body),
        other => panic!("a new vCon, got {other:?}"),
    }
}

fn create_then_get(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    let (version, body) = new_version(&f.store, &who, &uuid(1), "first");
    let stored = f
        .store
        .get(&who, Uuid::parse_str(&uuid(1)).expect("uuid"))
        .expect("reads");
    assert_eq!(
        (stored.version, stored.body),
        (version, body),
        "create_then_get: (stored.version, stored.body)"
    );
    let absent = Uuid::parse_str(&uuid(2)).expect("uuid");
    assert_eq!(
        f.store.get(&who, absent),
        Err(StoreError::NotFound),
        "create_then_get: f.store.get(&who, absent)"
    );
}

fn create_same_conflict_gone(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    let (version, body) = new_version(&f.store, &who, &uuid(1), "first");
    let (found, _) = vcon(&uuid(1), "first");
    assert_eq!(
        f.store.create(&who, &found, &body),
        Ok(Created::Same(version)),
        "create_same_conflict_gone: f.store.create(&who, &found, &body)"
    );
    let (other, other_body) = vcon(&uuid(1), "other");
    assert_eq!(
        f.store.create(&who, &other, &other_body),
        Err(StoreError::Conflict),
        "create_same_conflict_gone: f.store.create(&who, &other, &other_body)"
    );
    f.store
        .delete(&who, found.uuid, Retention::Years(7))
        .expect("deletes");
    assert_eq!(
        f.store.create(&who, &found, &body),
        Err(StoreError::Gone),
        "create_same_conflict_gone: f.store.create(&who, &found, &body)"
    );
}

fn forms_read_back(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    let (_, inner) = vcon(&uuid(2), "signed");
    let signed = format!(
        r#"{{"payload": "{}", "signatures": [{{"header": {{}}}}]}}"#,
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(inner)
    )
    .into_bytes();
    let encrypted = format!(
        r#"{{"unprotected": {{"uuid": "{}"}}, "ciphertext": "x", "recipients": []}}"#,
        uuid(3)
    )
    .into_bytes();
    for (body, form) in [
        (signed, vcon::Form::Signed),
        (encrypted, vcon::Form::Encrypted),
    ] {
        let found = found(&body);
        f.store.create(&who, &found, &body).expect("stored");
        let stored = f.store.get(&who, found.uuid).expect("reads");
        assert_eq!(
            (stored.form, stored.body),
            (form, body.clone()),
            "forms_read_back: (stored.form, stored.body)"
        );
        assert_eq!(
            f.store.replace(&who, stored.version, &found, &body),
            Err(StoreError::Immutable),
            "forms_read_back: f.store.replace(&who, stored.version, &found, &body)"
        );
    }
}

fn replace_rules(h: &dyn Harness) {
    let clock = TestClock::at(START);
    let f = h.fresh(clock.clone());
    let who = caller();
    let (absent, absent_body) = vcon(&uuid(9), "absent");
    assert_eq!(
        f.store
            .replace(&who, VersionId::generate(), &absent, &absent_body),
        Err(StoreError::NotFound),
        "replace_rules: f.store .replace(&who, VersionId::generate(), &absent, &a..."
    );
    let (first, _) = new_version(&f.store, &who, &uuid(1), "first");
    let (found, second) = vcon(&uuid(1), "second");
    assert_eq!(
        f.store
            .replace(&who, VersionId::generate(), &found, &second),
        Err(StoreError::PreconditionFailed),
        "replace_rules: f.store .replace(&who, VersionId::generate(), &found, &se..."
    );
    clock.set(START + time::Duration::seconds(1));
    let next = f
        .store
        .replace(&who, first, &found, &second)
        .expect("replaces");
    assert_eq!(
        f.store.get(&who, found.uuid).expect("reads").version,
        next,
        "replace_rules: f.store.get(&who, found.uuid).expect(\"reads\").version"
    );
    let kept = f.store.legal_versions(&who, found.uuid).expect("lists");
    assert_eq!(
        kept.iter()
            .map(|v| (v.version, v.current))
            .collect::<Vec<_>>(),
        [(first, false), (next, true)],
        "replace_rules: kept.iter().map(|v| (v.version, v.current)).collect::<Vec..."
    );
    f.store
        .delete(&who, found.uuid, Retention::Years(7))
        .expect("deletes");
    assert_eq!(
        f.store.replace(&who, next, &found, &second),
        Err(StoreError::Gone),
        "replace_rules: f.store.replace(&who, next, &found, &second)"
    );
}

fn delete_and_get(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    new_version(&f.store, &who, &uuid(1), "first");
    let id = Uuid::parse_str(&uuid(1)).expect("uuid");
    f.store
        .delete(&who, id, Retention::Years(7))
        .expect("deletes");
    assert_eq!(
        f.store.get(&who, id),
        Err(StoreError::Gone),
        "delete_and_get: f.store.get(&who, id)"
    );
    assert_eq!(
        f.store.delete(&who, id, Retention::Years(7)),
        Ok(()),
        "deleting again succeeds"
    );
    let never = Uuid::parse_str(&uuid(2)).expect("uuid");
    assert_eq!(
        f.store.delete(&who, never, Retention::Years(7)),
        Err(StoreError::NotFound),
        "delete_and_get: f.store.delete(&who, never, Retention::Years(7))"
    );
}

fn purge_timing_and_hold(h: &dyn Harness) {
    let clock = TestClock::at(START);
    let f = h.fresh(clock.clone());
    let who = caller();
    let days = Retention::Days(1);
    let (version, _) = new_version(&f.store, &who, &uuid(1), "first");
    let id = Uuid::parse_str(&uuid(1)).expect("uuid");
    f.store.delete(&who, id, days).expect("deletes");
    clock.set(START + time::Duration::days(1) - time::Duration::seconds(1));
    assert_eq!(
        f.store.purge_due(days).expect("sweeps").purged,
        0,
        "purge_timing_and_hold: f.store.purge_due(days).expect(\"sweeps\").purged"
    );
    f.store.set_hold(&who, id, true).expect("holds");
    clock.set(START + time::Duration::days(2));
    assert_eq!(f.store.purge_due(days).expect("sweeps").purged, 0, "held");
    f.store.set_hold(&who, id, false).expect("releases");
    assert_eq!(
        f.store.purge_due(days).expect("sweeps").purged,
        1,
        "purge_timing_and_hold: f.store.purge_due(days).expect(\"sweeps\").purged"
    );
    assert_eq!(
        f.store.legal_get(&who, id, Some(version)),
        Err(StoreError::Gone),
        "purge_timing_and_hold: f.store.legal_get(&who, id, Some(version))"
    );
    assert_eq!(
        f.store.check(),
        Ok(Vec::new()),
        "purge_timing_and_hold: f.store.check()"
    );
    assert_eq!(
        f.store.counts().expect("counts").versions,
        0,
        "purge_timing_and_hold: f.store.counts().expect(\"counts\").versions"
    );
    // Retention zero erases inside the delete.
    new_version(&f.store, &who, &uuid(2), "second");
    let second = Uuid::parse_str(&uuid(2)).expect("uuid");
    f.store
        .delete(&who, second, Retention::Days(0))
        .expect("deletes");
    assert_eq!(
        f.store.counts().expect("counts").versions,
        0,
        "purge_timing_and_hold: f.store.counts().expect(\"counts\").versions"
    );
}

fn legal_reads_and_restore(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    let (version, body) = new_version(&f.store, &who, &uuid(1), "first");
    let id = Uuid::parse_str(&uuid(1)).expect("uuid");
    f.store
        .delete(&who, id, Retention::Years(7))
        .expect("deletes");
    let read = f.store.legal_get(&who, id, None).expect("legal read");
    assert!(read.deleted, "legal_reads_and_restore: read.deleted");
    assert_eq!(
        read.stored.body, body,
        "legal_reads_and_restore: read.stored.body"
    );
    assert_eq!(
        f.store.restore(&who, id),
        Ok(version),
        "legal_reads_and_restore: f.store.restore(&who, id)"
    );
    assert_eq!(
        f.store.restore(&who, id),
        Err(StoreError::NotDeleted),
        "legal_reads_and_restore: f.store.restore(&who, id)"
    );
    assert_eq!(
        f.store.get(&who, id).expect("live again").body,
        body,
        "legal_reads_and_restore: f.store.get(&who, id).expect(\"live again\").body"
    );
    let never = Uuid::parse_str(&uuid(2)).expect("uuid");
    assert_eq!(
        f.store.legal_get(&who, never, None).map(|_| ()),
        Err(StoreError::NotFound),
        "legal_reads_and_restore: f.store.legal_get(&who, never, None).map(|_| ())"
    );
    assert_eq!(
        f.store.set_hold(&who, never, true),
        Err(StoreError::NotFound),
        "legal_reads_and_restore: f.store.set_hold(&who, never, true)"
    );
}

fn list_pages(h: &dyn Harness) {
    let clock = TestClock::at(START);
    let f = h.fresh(clock.clone());
    let who = caller();
    for n in 1..=5 {
        clock.set(START + time::Duration::seconds(i64::from(n)));
        new_version(&f.store, &who, &uuid(n), "listed");
    }
    let mut seen = Vec::new();
    let mut cursor: Option<Cursor> = None;
    loop {
        let text = cursor.map(|c| c.to_string());
        let query = ListQuery::new(Some(2), text.as_deref(), None, None).expect("a query");
        let page = f.store.list(&who, &query).expect("lists");
        seen.extend(page.vcons.iter().map(|v| v.uuid.to_string()));
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    let newest_first: Vec<String> = (1..=5).rev().map(uuid).collect();
    assert_eq!(seen, newest_first, "list_pages: seen");
}

fn audit_records_and_pages(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    new_version(&f.store, &who, &uuid(1), "first");
    let id = Uuid::parse_str(&uuid(1)).expect("uuid");
    f.store.get(&who, id).expect("reads");
    let mut out = Vec::new();
    f.store.export(None, &mut out).expect("exports");
    let actions: Vec<String> = String::from_utf8(out)
        .expect("UTF-8")
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("JSON");
            format!("{} {}", v["action"].as_str().expect("action"), v["status"])
        })
        .collect();
    assert_eq!(
        actions,
        ["create 201", "read 200"],
        "audit_records_and_pages: actions"
    );
    let query = crate::audit::AuditQuery::new(Some(1), Some(1)).expect("a query");
    let page = f.store.audit_page(&who, &query).expect("a page");
    assert_eq!(
        String::from_utf8(page).expect("UTF-8").lines().count(),
        1,
        "audit_records_and_pages: String::from_utf8(page).expect(\"UTF-8\").lines().count()"
    );
}

fn rewrap_moves_every_key(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    for n in 1..=3 {
        new_version(&f.store, &who, &uuid(n), "kept");
    }
    let new = Key::generate().expect("key");
    assert_eq!(
        f.store.rewrap(&new, 2),
        Ok(2),
        "rewrap_moves_every_key: f.store.rewrap(&new, 2)"
    );
    assert_eq!(
        f.store.rows_under_other_keys(),
        Ok(2),
        "rewrap_moves_every_key: f.store.rows_under_other_keys()"
    );
    assert_eq!(
        f.store.rewrap(&new, 2),
        Ok(1),
        "rewrap_moves_every_key: f.store.rewrap(&new, 2)"
    );
    assert_eq!(
        f.store.rewrap(&new, 2),
        Ok(0),
        "rewrap_moves_every_key: f.store.rewrap(&new, 2)"
    );
    assert_eq!(
        f.store.rows_under_other_keys(),
        Ok(3),
        "rewrap_moves_every_key: f.store.rows_under_other_keys()"
    );
}

fn check_and_counts(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    new_version(&f.store, &who, &uuid(1), "first");
    let id = Uuid::parse_str(&uuid(1)).expect("uuid");
    f.store.set_hold(&who, id, true).expect("holds");
    let counts = f.store.counts().expect("counts");
    assert_eq!(
        (counts.versions, counts.held),
        (1, 1),
        "check_and_counts: (counts.versions, counts.held)"
    );
    assert_eq!(
        f.store.check(),
        Ok(Vec::new()),
        "check_and_counts: f.store.check()"
    );
}

fn reopen_keeps_everything(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    let (version, body) = new_version(&f.store, &who, &uuid(1), "first");
    drop(f.store);
    let store = (f.reopen)();
    let stored = store
        .get(&who, Uuid::parse_str(&uuid(1)).expect("uuid"))
        .expect("reads after reopening");
    assert_eq!(
        (stored.version, stored.body),
        (version, body),
        "reopen_keeps_everything: (stored.version, stored.body)"
    );
}

/// The names the objects hold, as the storage lists them.
fn kept(objects: &dyn Objects) -> Vec<String> {
    objects.list().expect("lists the objects")
}

/// A purged version's object is gone from the storage, not only from the
/// catalog: after a sweep, and when retention zero erases in the delete.
fn purge_removes_the_object(h: &dyn Harness) {
    let clock = TestClock::at(START);
    let f = h.fresh(clock.clone());
    let who = caller();
    let days = Retention::Days(1);
    let (swept, _) = new_version(&f.store, &who, &uuid(1), "first");
    let (erased, _) = new_version(&f.store, &who, &uuid(2), "second");
    let (live, _) = new_version(&f.store, &who, &uuid(3), "third");
    let mut all = vec![swept.to_string(), erased.to_string(), live.to_string()];
    all.sort();
    assert_eq!(
        kept(&*f.objects),
        all,
        "purge_removes_the_object: one object each"
    );
    let first = Uuid::parse_str(&uuid(1)).expect("uuid");
    f.store.delete(&who, first, days).expect("deletes");
    clock.set(START + time::Duration::days(2));
    assert_eq!(
        f.store.purge_due(days).expect("sweeps").purged,
        1,
        "purge_removes_the_object: the sweep purges the deleted version"
    );
    let second = Uuid::parse_str(&uuid(2)).expect("uuid");
    f.store
        .delete(&who, second, Retention::Days(0))
        .expect("deletes");
    assert_eq!(
        kept(&*f.objects),
        [live.to_string()],
        "purge_removes_the_object: only the live version's object is kept"
    );
}

/// Start-up recovery applies the catalog rule: an object with no row, and
/// a temporary one, is garbage and removed; a version with a row keeps its
/// object; a name that is not a version id is not the server's and stays.
fn recovery_removes_only_garbage(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    let (version, body) = new_version(&f.store, &who, &uuid(1), "first");
    let garbage = VersionId::generate().to_string();
    let temporary = format!("{}.tmp", VersionId::generate());
    let foreign = "not-a-version";
    for name in [garbage.as_str(), temporary.as_str(), foreign] {
        (f.plant)(name, b"no row names this");
    }
    drop(f.store);
    let store = (f.reopen)();
    let mut expected = vec![version.to_string(), foreign.to_owned()];
    expected.sort();
    assert_eq!(
        kept(&*f.objects),
        expected,
        "recovery_removes_only_garbage: the objects left after recovery"
    );
    assert_eq!(
        store
            .get(&who, Uuid::parse_str(&uuid(1)).expect("uuid"))
            .expect("reads")
            .body,
        body,
        "recovery_removes_only_garbage: the stored vCon reads back"
    );
}

/// A purge whose object was already lost (removed by hand, or by a purge
/// cut off after its commit) still purges: the row goes, the sweep goes
/// on, and nothing is left for `check` to report.
fn purge_of_a_lost_object_finishes(h: &dyn Harness) {
    let clock = TestClock::at(START);
    let f = h.fresh(clock.clone());
    let who = caller();
    let days = Retention::Days(1);
    let (lost, _) = new_version(&f.store, &who, &uuid(1), "first");
    new_version(&f.store, &who, &uuid(2), "second");
    for n in [1, 2] {
        let id = Uuid::parse_str(&uuid(n)).expect("uuid");
        f.store.delete(&who, id, days).expect("deletes");
    }
    f.objects
        .remove_name(&lost.to_string())
        .expect("removes the object around the Store");
    clock.set(START + time::Duration::days(2));
    assert_eq!(
        f.store.purge_due(days).map(|r| r.purged),
        Ok(2),
        "purge_of_a_lost_object_finishes: both versions are purged"
    );
    assert_eq!(
        f.store.check(),
        Ok(Vec::new()),
        "purge_of_a_lost_object_finishes: f.store.check()"
    );
    assert!(
        kept(&*f.objects).is_empty(),
        "purge_of_a_lost_object_finishes: no object is left"
    );
}

/// Every read that opens a version answers `expected` for `uuid(n)`,
/// whose current version is `version`: get, a create of the same body,
/// and the legal reads of its current version and of `version` by id.
fn reads_answer(
    store: &Store,
    who: &Caller,
    n: u8,
    version: VersionId,
    expected: StoreError,
    check: &str,
) {
    let (found, body) = vcon(&uuid(n), "first");
    let id = found.uuid;
    assert_eq!(store.get(who, id).err(), Some(expected), "{check}: get");
    assert_eq!(
        store.create(who, &found, &body).err(),
        Some(expected),
        "{check}: create of the same body"
    );
    for asked in [None, Some(version)] {
        assert_eq!(
            store.legal_get(who, id, asked).err(),
            Some(expected),
            "{check}: legal_get {asked:?}"
        );
    }
}

/// Damage is the stored data itself gone bad: an object a row names that
/// the storage no longer holds, or one that does not open. Each read
/// answers Damaged, and `examine` lists exactly those versions.
fn a_lost_or_altered_object_is_damage(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    let (lost, _) = new_version(&f.store, &who, &uuid(1), "first");
    let (altered, _) = new_version(&f.store, &who, &uuid(2), "first");
    new_version(&f.store, &who, &uuid(3), "first");
    f.objects
        .remove_name(&lost.to_string())
        .expect("removes the object around the Store");
    (f.plant)(&altered.to_string(), b"not a sealed body");
    let check = "a_lost_or_altered_object_is_damage";
    reads_answer(&f.store, &who, 1, lost, StoreError::Damaged, check);
    reads_answer(&f.store, &who, 2, altered, StoreError::Damaged, check);
    let mut expected = vec![lost, altered];
    expected.sort_by_key(|v| v.0);
    assert_eq!(
        f.store
            .examine(Retention::Years(7), START, START)
            .map(|e| e.damaged),
        Ok(expected),
        "{check}: examine lists the lost and the altered version"
    );
}

/// A read the storage fails (it cannot be reached, refuses, or does not
/// answer) says nothing about the stored data: each read answers Io, not
/// Damaged, and `examine` fails rather than list a sound version as
/// damaged.
fn an_object_the_storage_cannot_read_is_a_failure(h: &dyn Harness) {
    let f = h.fresh(TestClock::at(START));
    let who = caller();
    let (version, _) = new_version(&f.store, &who, &uuid(1), "first");
    drop(f.store);
    let store = (f.unreadable)();
    let check = "an_object_the_storage_cannot_read_is_a_failure";
    reads_answer(&store, &who, 1, version, StoreError::Io, check);
    assert_eq!(
        store.examine(Retention::Years(7), START, START).err(),
        Some(StoreError::Io),
        "{check}: examine"
    );
    drop(store);
    let (found, body) = vcon(&uuid(1), "first");
    assert_eq!(
        (f.reopen)().create(&who, &found, &body),
        Ok(Created::Same(version)),
        "{check}: the version reads again once the storage does"
    );
}

/// The local backend on a temporary directory.
#[cfg(test)]
mod local {
    use super::*;
    use crate::backend::local::LocalObjects;
    use crate::envelope;
    use crate::fs::OsFs;
    use crate::ids::TenantId;

    struct Local(tempfile::TempDir);

    impl Harness for Local {
        fn fresh(&self, clock: Arc<TestClock>) -> Fresh {
            let dir = self.0.path().join(TenantId::generate().to_string());
            std::fs::create_dir(&dir).expect("tenant directory");
            let key = Key::generate().expect("key");
            let master = Key::generate().expect("master");
            let ctx = envelope::Context::Tenant {
                tenant: TenantId([0x11; 16]),
            };
            let wrapped = envelope::wrap(&master, &ctx, &key);
            let objects_dir = dir.join("objects");
            let open = std::rc::Rc::new(move || {
                let key = envelope::unwrap(&master, &ctx, &wrapped).expect("unwraps");
                Store::open(
                    &dir,
                    TenantId([0x11; 16]),
                    key,
                    Arc::new(OsFs),
                    clock.clone(),
                )
                .expect("opens")
            });
            let store = open();
            let plant_dir = objects_dir.clone();
            let unreadable_dir = objects_dir.clone();
            let reopen = open.clone();
            let readable_dir = objects_dir.clone();
            Fresh {
                store,
                reopen: Box::new(move || {
                    // Readable again, if `unreadable` made them not.
                    use std::os::unix::fs::PermissionsExt;
                    for entry in std::fs::read_dir(&readable_dir).expect("objects") {
                        let path = entry.expect("an entry").path();
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                            .expect("chmod");
                    }
                    reopen()
                }),
                unreadable: Box::new(move || {
                    // Each object file mode 000 until the next open.
                    use std::os::unix::fs::PermissionsExt;
                    let store = open();
                    for entry in std::fs::read_dir(&unreadable_dir).expect("objects") {
                        let path = entry.expect("an entry").path();
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
                            .expect("chmod");
                    }
                    store
                }),
                objects: Arc::new(LocalObjects::new(objects_dir, Arc::new(OsFs))),
                plant: Box::new(move |name, bytes| {
                    std::fs::write(plant_dir.join(name), bytes).expect("plants an object file");
                }),
            }
        }
    }

    #[test]
    fn the_local_backend_conforms() {
        run(&Local(tempfile::tempdir().expect("temp dir")));
    }
}
