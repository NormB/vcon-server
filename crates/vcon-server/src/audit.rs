//! The audit log: one record per request, written in the same transaction
//! as the change it describes. A record holds no vCon content, no token
//! and no client address.

use std::io::{self, Write};

use rusqlite::{Connection, Transaction, params};
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::ids::{KeyId, VersionId};
use crate::query::{self, BadQuery};

/// The audit table, part of every catalog that holds a log.
pub const SCHEMA: &str = "
CREATE TABLE audit (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    target TEXT,
    version TEXT,
    status INTEGER NOT NULL,
    count INTEGER NOT NULL
);
CREATE INDEX audit_counted ON audit (actor, action, status, at);
";

/// Who made a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// A caller's key.
    Key(KeyId),
    /// The purge sweep.
    Sweep,
    /// A command run on the host.
    Cli,
    /// A caller with no known key: its token did not parse, or names a key
    /// id the key table does not hold.
    Unparsed,
}

/// What a request asked for. A refused request records what it asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Replace,
    Read,
    Delete,
    Purge,
    LegalRead,
    LegalVersions,
    Restore,
    Hold,
    Release,
    KeyCreate,
    KeyRevoke,
    KeyList,
    KeyInfo,
    Retention,
    Health,
    AuthFailed,
    TenantCreate,
    TenantList,
    TenantRetention,
    TenantRemove,
    RekeyMaster,
    RekeyTenant,
    List,
    AuditRead,
    Openapi,
    Backup,
}

impl Action {
    /// The name written in the log.
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Create => "create",
            Action::Replace => "replace",
            Action::Read => "read",
            Action::Delete => "delete",
            Action::Purge => "purge",
            Action::LegalRead => "legal_read",
            Action::LegalVersions => "legal_versions",
            Action::Restore => "restore",
            Action::Hold => "hold",
            Action::Release => "release",
            Action::KeyCreate => "key_create",
            Action::KeyRevoke => "key_revoke",
            Action::KeyList => "key_list",
            Action::KeyInfo => "key_info",
            Action::Retention => "retention",
            Action::Health => "health",
            Action::AuthFailed => "auth_failed",
            Action::TenantCreate => "tenant_create",
            Action::TenantList => "tenant_list",
            Action::TenantRetention => "tenant_retention",
            Action::TenantRemove => "tenant_remove",
            Action::RekeyMaster => "rekey_master",
            Action::RekeyTenant => "rekey_tenant",
            Action::List => "list",
            Action::AuditRead => "audit_read",
            Action::Openapi => "openapi",
            Action::Backup => "backup",
        }
    }
}

/// One request, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub actor: Actor,
    pub action: Action,
    /// The vCon's uuid, or the tenant or key id for an admin action.
    pub target: Option<String>,
    /// The version written, read or purged, if there is one.
    pub version: Option<VersionId>,
    /// The HTTP status answered, or the command's result.
    pub status: u16,
}

/// Why a record could not be written or exported.
#[derive(Debug)]
pub enum AuditError {
    Sqlite(rusqlite::Error),
    Write(io::Error),
}

impl From<rusqlite::Error> for AuditError {
    fn from(e: rusqlite::Error) -> AuditError {
        AuditError::Sqlite(e)
    }
}

impl Actor {
    /// The name written in the log: the key id, `sweep`, `cli`, or empty.
    fn name(self) -> String {
        match self {
            Actor::Key(id) => id.to_string(),
            Actor::Sweep => "sweep".to_owned(),
            Actor::Cli => "cli".to_owned(),
            Actor::Unparsed => String::new(),
        }
    }
}

/// `now` in RFC 3339, in UTC.
fn rfc3339(now: OffsetDateTime) -> String {
    now.to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .expect("every UTC time from the clock formats")
}

/// Appends one record, at `now`, in `tx`.
pub fn record(tx: &Transaction, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
    tx.execute(
        "INSERT INTO audit (at, actor, action, target, version, status, count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
        params![
            rfc3339(now),
            entry.actor.name(),
            entry.action.as_str(),
            entry.target,
            entry.version.map(|v| v.to_string()),
            entry.status,
        ],
    )?;
    Ok(())
}

/// Adds one to the record for this actor, action and status in the minute
/// of `now`, making it if needed. For requests that could otherwise grow a
/// log one row each: failed authentications and health checks.
pub fn count(tx: &Transaction, entry: &Entry, now: OffsetDateTime) -> Result<(), AuditError> {
    let minute = now
        .to_offset(time::UtcOffset::UTC)
        .replace_second(0)
        .and_then(|t| t.replace_nanosecond(0))
        .expect("0 is a valid second and nanosecond");
    let fields = params![
        rfc3339(minute),
        entry.actor.name(),
        entry.action.as_str(),
        entry.target,
        entry.version.map(|v| v.to_string()),
        entry.status,
    ];
    let added = tx.execute(
        "UPDATE audit SET count = count + 1
         WHERE at = ?1 AND actor = ?2 AND action = ?3 AND target IS ?4 AND version IS ?5
           AND status = ?6",
        fields,
    )?;
    if added == 0 {
        tx.execute(
            "INSERT INTO audit (at, actor, action, target, version, status, count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
            fields,
        )?;
    }
    Ok(())
}

/// Writes every record after `since` as JSON lines, oldest first.
pub fn export(log: &Connection, since: Option<i64>, out: &mut dyn Write) -> Result<(), AuditError> {
    // SQLite reads a negative limit as none.
    lines(log, since.unwrap_or(0), -1, out)
}

/// Writes at most `limit` records after `after` as JSON lines, oldest
/// first: the one rendering both `export` and `page` write.
fn lines(log: &Connection, after: i64, limit: i64, out: &mut dyn Write) -> Result<(), AuditError> {
    let mut rows = log.prepare(
        "SELECT seq, at, actor, action, target, version, status, count
         FROM audit WHERE seq > ?1 ORDER BY seq LIMIT ?2",
    )?;
    let mut rows = rows.query(params![after, limit])?;
    while let Some(row) = rows.next()? {
        let record = json!({
            "seq": row.get::<_, i64>(0)?,
            "at": row.get::<_, String>(1)?,
            "actor": row.get::<_, String>(2)?,
            "action": row.get::<_, String>(3)?,
            "target": row.get::<_, Option<String>>(4)?,
            "version": row.get::<_, Option<String>>(5)?,
            "status": row.get::<_, u16>(6)?,
            "count": row.get::<_, i64>(7)?,
        });
        writeln!(out, "{record}").map_err(AuditError::Write)?;
    }
    Ok(())
}

/// What an audit read asks for: at most `limit` records after the seq
/// `after`, oldest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditQuery {
    pub after: i64,
    pub limit: u32,
}

impl AuditQuery {
    pub const DEFAULT_LIMIT: u32 = 1000;
    pub const MAX_LIMIT: u32 = 10000;

    /// An audit query from its parameters' values, the one rule for them
    /// wherever they come from: `after` a seq (0, the default, is the
    /// start of the log), `limit` 1 to `MAX_LIMIT`.
    pub fn new(after: Option<u64>, limit: Option<u64>) -> Result<AuditQuery, BadQuery> {
        let after = after
            .map(|n| i64::try_from(n).map_err(|_| BadQuery))
            .transpose()?
            .unwrap_or(0);
        let limit = match limit {
            None => AuditQuery::DEFAULT_LIMIT,
            Some(n) => u32::try_from(n)
                .ok()
                .filter(|n| (1..=AuditQuery::MAX_LIMIT).contains(n))
                .ok_or(BadQuery)?,
        };
        Ok(AuditQuery { after, limit })
    }

    /// An audit query from an HTTP query string, which may name `after` and
    /// `limit`, each at most once.
    pub fn from_query(text: Option<&str>) -> Result<AuditQuery, BadQuery> {
        let read = query::pairs(text, &["after", "limit"])?;
        let number = |name| {
            read.get(name)
                .map(|n: &String| n.parse::<u64>().map_err(|_| BadQuery))
                .transpose()
        };
        AuditQuery::new(number("after")?, number("limit")?)
    }
}

/// Writes at most `query.limit` records after `query.after` as JSON lines,
/// oldest first: the first lines `export` writes from the same place.
pub fn page(log: &Connection, query: &AuditQuery, out: &mut dyn Write) -> Result<(), AuditError> {
    lines(log, query.after, i64::from(query.limit), out)
}

/// Reading a log back, for tests.
#[cfg(test)]
pub(crate) mod testing {
    use rusqlite::Connection;
    use serde_json::Value;

    /// The records `export` writes after `since`, parsed.
    pub(crate) fn exported(log: &Connection, since: Option<i64>) -> Vec<Value> {
        let mut out = Vec::new();
        super::export(log, since, &mut out).expect("export");
        String::from_utf8(out)
            .expect("UTF-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use time::macros::datetime;

    use super::testing::exported;
    use super::*;
    use crate::catalog;
    use crate::envelope::Key;

    struct Log {
        _dir: tempfile::TempDir,
        conn: Connection,
    }

    fn log() -> Log {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = Key::generate().expect("key");
        let conn = catalog::open(&dir.path().join("catalog.sqlite"), &key, SCHEMA).expect("opens");
        Log { _dir: dir, conn }
    }

    fn read_by(key: KeyId, status: u16) -> Entry {
        Entry {
            actor: Actor::Key(key),
            action: Action::Read,
            target: Some("01a07da8-c2bb-83e5-b9a2-279e0d16bc46".into()),
            version: Some(VersionId([0x22; 16])),
            status,
        }
    }

    #[test]
    fn records_are_written_and_exported_back() {
        let mut log = log();
        let key = Key::generate().expect("key").id();
        let tx = log.conn.transaction().expect("tx");
        record(
            &tx,
            &read_by(key, 200),
            datetime!(2026-10-23 12:34:56.789 UTC),
        )
        .expect("record");
        let purge = Entry {
            actor: Actor::Sweep,
            action: Action::Purge,
            target: Some("01a07da8-c2bb-83e5-b9a2-279e0d16bc46".into()),
            version: None,
            status: 200,
        };
        record(&tx, &purge, datetime!(2026-10-23 12:35:00 UTC)).expect("record");
        tx.commit().expect("commit");
        assert_eq!(
            exported(&log.conn, None),
            [
                json!({"seq": 1, "at": "2026-10-23T12:34:56.789Z", "actor": key.to_string(),
                       "action": "read", "target": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
                       "version": "22".repeat(16), "status": 200, "count": 1}),
                json!({"seq": 2, "at": "2026-10-23T12:35:00Z", "actor": "sweep",
                       "action": "purge", "target": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
                       "version": null, "status": 200, "count": 1}),
            ]
        );
        assert_eq!(
            exported(&log.conn, Some(1)).len(),
            1,
            "only records after seq 1"
        );
        assert_eq!(exported(&log.conn, Some(2)).len(), 0);
    }

    #[test]
    fn times_are_written_in_utc() {
        let mut log = log();
        let key = Key::generate().expect("key").id();
        let tx = log.conn.transaction().expect("tx");
        record(
            &tx,
            &read_by(key, 200),
            datetime!(2026-10-23 14:34:56 +02:00),
        )
        .expect("record");
        tx.commit().expect("commit");
        assert_eq!(exported(&log.conn, None)[0]["at"], "2026-10-23T12:34:56Z");
    }

    #[test]
    fn counted_records_hold_one_row_per_actor_action_status_and_minute() {
        let mut log = log();
        let health = |status| Entry {
            actor: Actor::Unparsed,
            action: Action::Health,
            target: None,
            version: None,
            status,
        };
        let tx = log.conn.transaction().expect("tx");
        for second in [0, 30, 59] {
            let now = datetime!(2026-10-23 12:34:00 UTC) + time::Duration::seconds(second);
            count(&tx, &health(200), now).expect("count");
        }
        count(&tx, &health(200), datetime!(2026-10-23 12:35:00 UTC)).expect("next minute");
        count(&tx, &health(503), datetime!(2026-10-23 12:34:10 UTC)).expect("other status");
        // Same actor, status and minute as the 503 health record above,
        // so only the action keeps the two apart.
        let failed = Entry {
            action: Action::AuthFailed,
            ..health(503)
        };
        count(&tx, &failed, datetime!(2026-10-23 12:34:20 UTC)).expect("other action");
        let cli = Entry {
            actor: Actor::Cli,
            ..health(200)
        };
        count(&tx, &cli, datetime!(2026-10-23 12:34:40 UTC)).expect("other actor");
        tx.commit().expect("commit");
        let rows: Vec<(String, String, String, u64, u64)> = exported(&log.conn, None)
            .iter()
            .map(|r| {
                (
                    r["at"].as_str().expect("at").to_owned(),
                    r["actor"].as_str().expect("actor").to_owned(),
                    r["action"].as_str().expect("action").to_owned(),
                    r["status"].as_u64().expect("status"),
                    r["count"].as_u64().expect("count"),
                )
            })
            .collect();
        let row = |at: &str, actor: &str, action: &str, status, count| {
            (
                at.to_owned(),
                actor.to_owned(),
                action.to_owned(),
                status,
                count,
            )
        };
        assert_eq!(
            rows,
            [
                row("2026-10-23T12:34:00Z", "", "health", 200, 3),
                row("2026-10-23T12:35:00Z", "", "health", 200, 1),
                row("2026-10-23T12:34:00Z", "", "health", 503, 1),
                row("2026-10-23T12:34:00Z", "", "auth_failed", 503, 1),
                row("2026-10-23T12:34:00Z", "cli", "health", 200, 1),
            ]
        );
    }

    #[test]
    fn a_record_in_a_transaction_that_does_not_commit_is_not_kept() {
        let mut log = log();
        let tx = log.conn.transaction().expect("tx");
        let key = Key::generate().expect("key").id();
        record(&tx, &read_by(key, 200), datetime!(2026-10-23 12:00:00 UTC)).expect("record");
        drop(tx);
        assert_eq!(exported(&log.conn, None), Vec::<Value>::new());
    }

    #[test]
    fn every_action_has_its_name_in_the_log() {
        let names = [
            (Action::Create, "create"),
            (Action::Replace, "replace"),
            (Action::Read, "read"),
            (Action::Delete, "delete"),
            (Action::Purge, "purge"),
            (Action::LegalRead, "legal_read"),
            (Action::LegalVersions, "legal_versions"),
            (Action::Restore, "restore"),
            (Action::Hold, "hold"),
            (Action::Release, "release"),
            (Action::KeyCreate, "key_create"),
            (Action::KeyRevoke, "key_revoke"),
            (Action::KeyList, "key_list"),
            (Action::Retention, "retention"),
            (Action::Health, "health"),
            (Action::AuthFailed, "auth_failed"),
            (Action::TenantCreate, "tenant_create"),
            (Action::TenantList, "tenant_list"),
            (Action::TenantRetention, "tenant_retention"),
            (Action::TenantRemove, "tenant_remove"),
            (Action::RekeyMaster, "rekey_master"),
            (Action::RekeyTenant, "rekey_tenant"),
            (Action::List, "list"),
            (Action::AuditRead, "audit_read"),
            (Action::Openapi, "openapi"),
            (Action::Backup, "backup"),
        ];
        for (action, name) in names {
            assert_eq!(action.as_str(), name);
        }
    }

    /// Five read records, at whole seconds.
    fn five(log: &mut Log) {
        let key = Key::generate().expect("key").id();
        let tx = log.conn.transaction().expect("tx");
        for second in 0..5 {
            let now = datetime!(2026-10-23 12:00:00 UTC) + time::Duration::seconds(second);
            record(&tx, &read_by(key, 200), now).expect("record");
        }
        tx.commit().expect("commit");
    }

    fn exported_text(log: &Connection, since: Option<i64>) -> String {
        let mut out = Vec::new();
        export(log, since, &mut out).expect("export");
        String::from_utf8(out).expect("UTF-8")
    }

    fn paged(log: &Connection, after: i64, limit: u32) -> String {
        let mut out = Vec::new();
        page(log, &AuditQuery { after, limit }, &mut out).expect("page");
        String::from_utf8(out).expect("UTF-8")
    }

    #[test]
    fn a_page_is_the_first_lines_export_writes_from_the_same_place() {
        let mut log = log();
        five(&mut log);
        for after in 0..=6 {
            let lines: Vec<String> = exported_text(&log.conn, Some(after))
                .lines()
                .map(|l| format!("{l}\n"))
                .collect();
            for limit in 1..=6 {
                let take = lines.len().min(usize::try_from(limit).expect("small"));
                assert_eq!(
                    paged(&log.conn, after, limit),
                    lines[..take].concat(),
                    "after {after}, limit {limit}"
                );
            }
        }
        assert_eq!(paged(&log.conn, 0, 10), exported_text(&log.conn, None));
    }

    #[test]
    fn an_audit_query_takes_a_seq_and_a_limit_of_1_to_10000() {
        let q = |after, limit| AuditQuery { after, limit };
        assert_eq!(AuditQuery::new(None, None), Ok(q(0, 1000)));
        assert_eq!(AuditQuery::new(Some(7), Some(1)), Ok(q(7, 1)));
        assert_eq!(AuditQuery::new(None, Some(10000)), Ok(q(0, 10000)));
        assert_eq!(
            AuditQuery::new(Some(i64::MAX as u64), None),
            Ok(q(i64::MAX, 1000))
        );
        for (after, limit) in [
            (None, Some(0)),
            (None, Some(10001)),
            (None, Some(u64::MAX)),
            (Some(i64::MAX as u64 + 1), None),
        ] {
            assert_eq!(
                AuditQuery::new(after, limit),
                Err(BadQuery),
                "{after:?} {limit:?}"
            );
        }
        assert_eq!(AuditQuery::from_query(None), Ok(q(0, 1000)));
        assert_eq!(
            AuditQuery::from_query(Some("after=12&limit=3")),
            Ok(q(12, 3))
        );
        for bad in [
            "after=-1",
            "after=x",
            "after=",
            "limit=0",
            "limit=1&limit=1",
            "since=3",
            "after=1.0",
        ] {
            assert_eq!(AuditQuery::from_query(Some(bad)), Err(BadQuery), "{bad}");
        }
    }
}
