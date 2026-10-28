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

/// A Store from `Harness::fresh`, and a way to open it again.
pub(crate) struct Fresh {
    pub(crate) store: Store,
    pub(crate) reopen: Box<dyn Fn() -> Store>,
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
    let checks: [Check; 12] = [
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

/// The local backend on a temporary directory.
#[cfg(test)]
mod local {
    use super::*;
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
            let open = move || {
                let key = envelope::unwrap(&master, &ctx, &wrapped).expect("unwraps");
                Store::open(
                    &dir,
                    TenantId([0x11; 16]),
                    key,
                    Arc::new(OsFs),
                    clock.clone(),
                )
                .expect("opens")
            };
            Fresh {
                store: open(),
                reopen: Box::new(open),
            }
        }
    }

    #[test]
    fn the_local_backend_conforms() {
        run(&Local(tempfile::tempdir().expect("temp dir")));
    }
}
