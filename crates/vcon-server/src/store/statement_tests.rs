//! The catalog connection keeps the statements it prepares and reuses
//! them: SQLite's parser was a sixth of the server's time on a read
//! (vcon-scale, 2026-11-21), and a kept statement is parsed once.

use super::testing::vcon;
use super::tests::fixture;
use super::{Created, StoreError};
use crate::catalog::testing::{keeps, prepared, reprepared};
use crate::retention::Retention;

const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
const OTHER: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc47";

/// The statements a read runs, each named by how its SQL starts.
const READ: [&str; 6] = [
    "BEGIN IMMEDIATE",
    "SELECT 1 FROM tombstone WHERE uuid = ?1",
    "SELECT current, deleted_at FROM vcon WHERE uuid = ?1",
    "SELECT v.form, v.gzip, v.wrapped_key,",
    "INSERT INTO audit (at, actor, action, target, version, status, count)",
    "COMMIT",
];

/// The statements a new vCon's write runs beyond a read's.
const WRITE: [&str; 4] = [
    "SELECT wraps FROM key_use WHERE tenant_key_id = ?1",
    "INSERT INTO key_use (tenant_key_id, wraps)",
    "INSERT INTO version",
    "INSERT INTO vcon (uuid, current, stored_at)",
];

/// A read prepares its statements once: a second and a third read find
/// the very statements the first left, and add none.
#[test]
fn a_second_read_reuses_the_statements_of_the_first() {
    let f = fixture();
    let (found, body) = vcon(UUID, "first");
    f.store.create(&f.caller, &found, &body).expect("stored");
    f.store.get(&f.caller, found.uuid).expect("first read");
    let after_one = prepared(&f.store.conn());
    for sql in READ {
        assert!(
            keeps(&f.store.conn(), sql),
            "the read keeps `{sql}` prepared: {after_one:?}"
        );
    }
    f.store.get(&f.caller, found.uuid).expect("second read");
    f.store.get(&f.caller, found.uuid).expect("third read");
    assert_eq!(
        prepared(&f.store.conn()),
        after_one,
        "later reads reuse the first read's statements and add none"
    );
}

/// A write keeps its statements too, and a second write of another vCon
/// reuses them.
#[test]
fn a_second_write_reuses_the_statements_of_the_first() {
    let f = fixture();
    let (found, body) = vcon(UUID, "first");
    f.store.create(&f.caller, &found, &body).expect("stored");
    let after_one = prepared(&f.store.conn());
    for sql in WRITE {
        assert!(
            keeps(&f.store.conn(), sql),
            "the write keeps `{sql}` prepared: {after_one:?}"
        );
    }
    let (other, body) = vcon(OTHER, "second");
    f.store.create(&f.caller, &other, &body).expect("stored");
    assert_eq!(
        prepared(&f.store.conn()),
        after_one,
        "the second write adds no statement"
    );
}

/// A read that is refused (no such vCon) still records its refusal, and
/// the kept statements serve the next read as before.
#[test]
fn a_refused_read_leaves_the_kept_statements_working() {
    let f = fixture();
    let (found, body) = vcon(UUID, "first");
    f.store.create(&f.caller, &found, &body).expect("stored");
    let absent = uuid::Uuid::parse_str(OTHER).expect("uuid");
    assert_eq!(
        f.store.get(&f.caller, absent),
        Err(StoreError::NotFound),
        "an unknown uuid is not found"
    );
    assert_eq!(
        f.store.get(&f.caller, found.uuid).expect("read").body,
        body,
        "the stored vCon still reads back"
    );
    assert_eq!(
        f.store.get(&f.caller, absent),
        Err(StoreError::NotFound),
        "the unknown uuid is still not found"
    );
    let statuses: Vec<_> = f
        .store
        .records()
        .iter()
        .map(|r| (r["action"].clone(), r["status"].clone()))
        .collect();
    assert_eq!(
        statuses,
        [
            (serde_json::json!("create"), serde_json::json!(201)),
            (serde_json::json!("read"), serde_json::json!(404)),
            (serde_json::json!("read"), serde_json::json!(200)),
            (serde_json::json!("read"), serde_json::json!(404)),
        ],
        "every outcome is recorded, in order"
    );
}

/// Reads and writes of many vCons bind new values to the kept statements,
/// and SQLite parses none of them again: a statement it prepares again on
/// each new value costs a parse on each use, kept or not.
#[test]
fn no_kept_statement_is_prepared_again_for_a_new_value() {
    let f = fixture();
    for i in 0..8_u32 {
        let uuid = format!("01a07da8-c2bb-83e5-b9a2-{i:012x}");
        let (found, body) = vcon(&uuid, "subject");
        f.store.create(&f.caller, &found, &body).expect("stored");
        f.store.get(&f.caller, found.uuid).expect("read");
    }
    assert_eq!(
        reprepared(&f.store.conn()),
        Vec::<(String, i32)>::new(),
        "statements SQLite prepared again, with how many times"
    );
}

/// The same for replaces, deletes, refused reads and reads of deleted
/// vCons, across several vCons.
#[test]
fn no_statement_of_replace_or_delete_is_prepared_again_for_a_new_value() {
    let f = fixture();
    for i in 0..8_u32 {
        let uuid = format!("01a07da8-c2bb-83e5-b9a2-{i:012x}");
        let (found, body) = vcon(&uuid, "subject");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("vCon {i} is not stored");
        };
        let (_, newer) = vcon(&uuid, "newer");
        f.store
            .replace(&f.caller, version, &found, &newer)
            .expect("replaced");
        f.store.get(&f.caller, found.uuid).expect("read");
        f.store
            .delete(&f.caller, found.uuid, Retention::Years(7))
            .expect("deleted");
        assert_eq!(
            f.store.get(&f.caller, found.uuid),
            Err(StoreError::Gone),
            "a deleted vCon is gone"
        );
    }
    assert_eq!(
        reprepared(&f.store.conn()),
        Vec::<(String, i32)>::new(),
        "statements SQLite prepared again, with how many times"
    );
}

/// A kept statement is reset after each use, so it holds no read of the
/// WAL: a truncating checkpoint right after reads finds no reader.
#[test]
fn kept_statements_hold_no_reader_of_the_wal() {
    let f = fixture();
    let (found, body) = vcon(UUID, "first");
    f.store.create(&f.caller, &found, &body).expect("stored");
    for _ in 0..3 {
        f.store.get(&f.caller, found.uuid).expect("read");
    }
    let busy: i64 = f
        .store
        .conn()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .expect("checkpoint");
    assert_eq!(busy, 0, "no kept statement holds the WAL");
    let wal = f.dir.path().join("catalog.sqlite-wal");
    let len = std::fs::metadata(&wal).map_or(0, |m| m.len());
    assert_eq!(len, 0, "the WAL is truncated: {}", wal.display());
}

/// Every kind of work a Store does keeps a bounded set of statements:
/// many writes, reads, replaces and deletes leave no more than the
/// cache holds.
#[test]
fn the_kept_statements_stay_bounded_by_the_cache() {
    let f = fixture();
    for i in 0..40_u32 {
        let uuid = format!("01a07da8-c2bb-83e5-b9a2-{i:012x}");
        let (found, body) = vcon(&uuid, "subject");
        let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
            panic!("vCon {i} is not stored");
        };
        f.store.get(&f.caller, found.uuid).expect("read");
        let (_, newer) = vcon(&uuid, "newer");
        f.store
            .replace(&f.caller, version, &found, &newer)
            .expect("replaced");
        f.store
            .delete(&f.caller, found.uuid, Retention::Years(7))
            .expect("deleted");
    }
    let kept = prepared(&f.store.conn()).len();
    assert!(
        (1..=crate::catalog::STATEMENT_CACHE).contains(&kept),
        "{kept} statements kept, at most {}",
        crate::catalog::STATEMENT_CACHE
    );
}

/// The lists the packer and the purge read (live and hidden versions,
/// pack files, files waiting for their purge, unnamed files) keep their
/// statements, and a second pass adds none.
#[test]
fn the_lists_of_versions_and_files_keep_their_statements() {
    let f = fixture();
    let (found, body) = vcon(UUID, "first");
    let Ok(Created::New(version)) = f.store.create(&f.caller, &found, &body) else {
        panic!("the vCon is not stored");
    };
    let (_, newer) = vcon(UUID, "newer");
    f.store
        .replace(&f.caller, version, &found, &newer)
        .expect("replaced");
    let pass = || {
        let mut tx = f.store.catalog.immediate().expect("a transaction");
        assert_eq!(
            tx.live_versions().expect("live").len(),
            1,
            "one live version"
        );
        assert_eq!(
            tx.hidden_versions(None).expect("hidden").len(),
            1,
            "the replaced version"
        );
        assert_eq!(
            tx.pack_files().expect("pack files"),
            vec![],
            "nothing is packed"
        );
        assert_eq!(
            tx.purge_pending().expect("pending"),
            vec![],
            "no purge is waiting"
        );
        assert_eq!(
            tx.unnamed_files().expect("unnamed"),
            vec![],
            "no file is unnamed"
        );
    };
    pass();
    let after_one = prepared(&f.store.conn());
    for sql in [
        "SELECT current FROM vcon WHERE current IS NOT NULL AND deleted_at IS NULL",
        "SELECT version FROM version WHERE hidden_at IS NOT NULL",
        "SELECT file_id FROM parquet_file ORDER BY file_id",
        "SELECT file_id FROM purge_pending ORDER BY file_id",
        "SELECT file_id FROM parquet_file WHERE unnamed_at IS NOT NULL",
    ] {
        assert!(
            keeps(&f.store.conn(), sql),
            "the list keeps `{sql}` prepared: {after_one:?}"
        );
    }
    pass();
    assert_eq!(
        prepared(&f.store.conn()),
        after_one,
        "the second pass reuses the first pass's statements and adds none"
    );
}

/// An export of the audit log reads it page by page with one kept
/// statement, which a second export reuses.
#[test]
fn an_audit_export_keeps_its_page_statement() {
    let f = fixture();
    let (found, body) = vcon(UUID, "first");
    f.store.create(&f.caller, &found, &body).expect("stored");
    f.store.get(&f.caller, found.uuid).expect("read");
    let mut first = Vec::new();
    f.store.export(None, &mut first).expect("exported");
    assert!(!first.is_empty(), "the log has records to export");
    let page = "SELECT seq, at, actor, action, target, version, status, count FROM audit";
    let after_one = prepared(&f.store.conn());
    assert!(
        keeps(&f.store.conn(), page),
        "the export keeps its page statement prepared: {after_one:?}"
    );
    let mut second = Vec::new();
    f.store.export(None, &mut second).expect("exported again");
    assert_eq!(second, first, "nothing was recorded between the exports");
    assert_eq!(
        prepared(&f.store.conn()),
        after_one,
        "the second export adds no statement"
    );
}
