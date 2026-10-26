//! Listing a tenant's live vCons, newest stored first, a page at a time.
//! The HTTP route, the MCP tools and the console all list through here,
//! so they take the same parameters and leave the same audit record.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use rusqlite::params;

use super::{Caller, Store, StoreError, commit, immediate, io};
use crate::audit::{Action, Actor, Entry};
use crate::clock::{from_nanos, nanos};
use crate::ids::VersionId;
use crate::query::{self, BadQuery};

/// Where a page ends: the last listed vCon's first stored time and uuid.
/// Clients get it as opaque text and send it back unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    stored_at: i64,
    uuid: Uuid,
}

impl Cursor {
    /// The cursor of a page that ends at `last`.
    pub fn after(last: &Listed) -> Cursor {
        Cursor {
            stored_at: nanos(last.stored_at),
            uuid: last.uuid,
        }
    }

    /// The cursor `Display` wrote, or None for any other text.
    pub fn parse(text: &str) -> Option<Cursor> {
        // The engine refuses padding and spare bits, so one cursor has one
        // text.
        let bytes: [u8; 24] = URL_SAFE_NO_PAD.decode(text).ok()?.try_into().ok()?;
        let (at, uuid) = bytes.split_at(8);
        Some(Cursor {
            stored_at: i64::from_be_bytes(at.try_into().ok()?),
            uuid: Uuid::from_slice(uuid).ok()?,
        })
    }
}

impl std::fmt::Display for Cursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut bytes = [0; 24];
        bytes[..8].copy_from_slice(&self.stored_at.to_be_bytes());
        bytes[8..].copy_from_slice(self.uuid.as_bytes());
        f.write_str(&URL_SAFE_NO_PAD.encode(bytes))
    }
}

/// What a list asks for: at most `limit` vCons, after `cursor`, first
/// stored at or after `since` and before `until`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListQuery {
    pub limit: u32,
    pub cursor: Option<Cursor>,
    pub since: Option<OffsetDateTime>,
    pub until: Option<OffsetDateTime>,
}

impl ListQuery {
    pub const DEFAULT_LIMIT: u32 = 100;
    pub const MAX_LIMIT: u32 = 1000;

    /// A list query from its parameters' values, the one rule for them
    /// wherever they come from: a limit of 1 to `MAX_LIMIT`, a cursor
    /// `Cursor::parse` reads, and RFC 3339 times.
    pub fn new(
        limit: Option<u64>,
        cursor: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<ListQuery, BadQuery> {
        let limit = match limit {
            None => ListQuery::DEFAULT_LIMIT,
            Some(n) => u32::try_from(n)
                .ok()
                .filter(|n| (1..=ListQuery::MAX_LIMIT).contains(n))
                .ok_or(BadQuery)?,
        };
        let cursor = cursor
            .map(|text| Cursor::parse(text).ok_or(BadQuery))
            .transpose()?;
        Ok(ListQuery {
            limit,
            cursor,
            since: since.map(time_bound).transpose()?,
            until: until.map(time_bound).transpose()?,
        })
    }

    /// A list query from an HTTP query string, which may name `limit`,
    /// `cursor`, `since` and `until`, each at most once.
    pub fn from_query(query: Option<&str>) -> Result<ListQuery, BadQuery> {
        let mut read = query::pairs(query, &["limit", "cursor", "since", "until"])?;
        let limit = read
            .remove("limit")
            .map(|n| n.parse::<u64>().map_err(|_| BadQuery))
            .transpose()?;
        ListQuery::new(
            limit,
            read.get("cursor").map(String::as_str),
            read.get("since").map(String::as_str),
            read.get("until").map(String::as_str),
        )
    }
}

/// A live vCon as the list shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    pub uuid: Uuid,
    /// Its current version.
    pub version: VersionId,
    /// When the uuid was first stored.
    pub stored_at: OffsetDateTime,
    /// When its current version was stored.
    pub updated_at: OffsetDateTime,
}

/// One page of a list, and the cursor of the next one, None on the last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub vcons: Vec<Listed>,
    pub next: Option<Cursor>,
}

impl Listing {
    /// The page as `GET /vcons` answers it, times in RFC 3339 UTC.
    pub fn to_json(&self) -> serde_json::Value {
        let vcons: Vec<serde_json::Value> = self
            .vcons
            .iter()
            .map(|v| {
                serde_json::json!({
                    "uuid": v.uuid.to_string(),
                    "version": v.version.to_string(),
                    "stored_at": rfc3339(v.stored_at),
                    "updated_at": rfc3339(v.updated_at),
                })
            })
            .collect();
        serde_json::json!({
            "vcons": vcons,
            "next": self.next.map(|c| c.to_string()),
        })
    }
}

impl Store {
    /// A page of the tenant's live vCons, newest first by when each uuid
    /// was first stored, then by uuid. The page and its audit record are
    /// one transaction.
    pub fn list(&self, who: &Caller, query: &ListQuery) -> Result<Listing, StoreError> {
        let mut conn = self.conn();
        let tx = immediate(&mut conn)?;
        let listing = page(&tx, query);
        let status = if listing.is_ok() { 200 } else { 500 };
        self.record(
            &tx,
            &Entry {
                actor: Actor::Key(who.key),
                action: Action::List,
                target: None,
                version: None,
                status,
            },
        )?;
        commit(tx)?;
        listing
    }
}

/// A time a query bounds the list by: RFC 3339, within what the catalog's
/// nanoseconds hold.
fn time_bound(text: &str) -> Result<OffsetDateTime, BadQuery> {
    let at = OffsetDateTime::parse(text, &Rfc3339).map_err(|_| BadQuery)?;
    i64::try_from(at.unix_timestamp_nanos()).map_err(|_| BadQuery)?;
    Ok(at.to_offset(time::UtcOffset::UTC))
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .expect("every time a catalog holds formats")
}

/// The page itself, read in `tx`. One row past the limit says whether a
/// next page exists. A live uuid whose current version has no row is
/// damage, never left out quietly.
fn page(tx: &rusqlite::Transaction, query: &ListQuery) -> Result<Listing, StoreError> {
    let mut rows = tx
        .prepare(
            "SELECT vcon.uuid, vcon.current, vcon.stored_at, version.stored_at
             FROM vcon LEFT JOIN version ON version.version = vcon.current
             WHERE vcon.current IS NOT NULL
               AND (?1 IS NULL OR vcon.stored_at < ?1 OR (vcon.stored_at = ?1 AND vcon.uuid < ?2))
               AND (?3 IS NULL OR vcon.stored_at >= ?3)
               AND (?4 IS NULL OR vcon.stored_at < ?4)
             ORDER BY vcon.stored_at DESC, vcon.uuid DESC
             LIMIT ?5",
        )
        .map_err(io)?;
    let cursor = query.cursor;
    let found = rows
        .query_map(
            params![
                cursor.map(|c| c.stored_at),
                cursor.map(|c| c.uuid.as_bytes().to_vec()),
                query.since.map(nanos),
                query.until.map(nanos),
                i64::from(query.limit) + 1,
            ],
            |r| {
                Ok((
                    r.get::<_, [u8; 16]>(0)?,
                    r.get::<_, [u8; 16]>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .map_err(io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io)?;
    let mut vcons = Vec::with_capacity(found.len());
    for (uuid, version, stored_at, updated_at) in found {
        let (Some(first), Some(Some(current))) =
            (from_nanos(stored_at), updated_at.map(from_nanos))
        else {
            return Err(StoreError::Damaged);
        };
        vcons.push(Listed {
            uuid: Uuid::from_bytes(uuid),
            version: VersionId(version),
            stored_at: first,
            updated_at: current,
        });
    }
    let limit = usize::try_from(query.limit).map_err(io)?;
    let next = if vcons.len() > limit {
        vcons.truncate(limit);
        vcons.last().map(|last| Cursor {
            stored_at: nanos(last.stored_at),
            uuid: last.uuid,
        })
    } else {
        None
    };
    Ok(Listing { vcons, next })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;
    use time::macros::datetime;

    use super::super::testing::vcon;
    use super::super::tests::{Fixture, fixture};
    use super::*;
    use crate::retention::Retention;
    use crate::store::Created;

    fn uuid(n: u32) -> String {
        format!("01a07da8-c2bb-83e5-b9a2-{n:012x}")
    }

    fn at(minute: u8) -> OffsetDateTime {
        datetime!(2026-10-25 12:00 UTC)
            .replace_minute(minute)
            .expect("a minute")
    }

    /// Stores vCon `n` at `when`, and returns its version.
    fn store(f: &Fixture, n: u32, when: OffsetDateTime) -> VersionId {
        f.clock.set(when);
        let (found, body) = vcon(&uuid(n), "listed");
        match f.store.create(&f.caller, &found, &body) {
            Ok(Created::New(version)) => version,
            other => panic!("vCon {n}: {other:?}"),
        }
    }

    fn query(limit: u32) -> ListQuery {
        ListQuery {
            limit,
            cursor: None,
            since: None,
            until: None,
        }
    }

    /// Every page of `query`, followed through its cursors.
    fn pages(f: &Fixture, first: ListQuery) -> Vec<Listing> {
        let mut pages = Vec::new();
        let mut next = Some(first);
        while let Some(query) = next {
            let page = f.store.list(&f.caller, &query).expect("a page");
            next = page.next.map(|cursor| ListQuery {
                cursor: Some(cursor),
                ..query
            });
            pages.push(page);
            assert!(pages.len() < 100, "the cursors end");
        }
        pages
    }

    fn uuids(listing: &[Listing]) -> Vec<String> {
        listing
            .iter()
            .flat_map(|p| p.vcons.iter().map(|v| v.uuid.to_string()))
            .collect()
    }

    fn records(f: &Fixture, action: &str) -> Vec<serde_json::Value> {
        f.store
            .records()
            .into_iter()
            .filter(|r| r["action"] == action)
            .collect()
    }

    #[test]
    fn a_cursor_reads_back_from_its_text_and_from_nothing_else() {
        let cursor = Cursor {
            stored_at: 1_792_929_600_123_456_789,
            uuid: Uuid::parse_str(&uuid(7)).expect("uuid"),
        };
        let text = cursor.to_string();
        assert_eq!(text.len(), 32, "24 bytes in base64url without padding");
        assert!(
            text.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        assert_eq!(Cursor::parse(&text), Some(cursor));
        let negative = Cursor {
            stored_at: -1,
            uuid: Uuid::nil(),
        };
        assert_eq!(Cursor::parse(&negative.to_string()), Some(negative));
        let mut other_last = text.clone().into_bytes();
        // The last character carries no spare bits at 24 bytes, so any
        // change to it is another cursor; a character outside the
        // alphabet is none.
        other_last[31] = b'!';
        for bad in [
            String::new(),
            text[..31].to_owned(),
            format!("{text}A"),
            format!("{text}="),
            String::from_utf8(other_last).expect("ASCII"),
            text.replace(['-', '_'], "+"),
        ] {
            if bad == text {
                continue;
            }
            assert_eq!(Cursor::parse(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_query_takes_a_limit_of_1_to_1000_a_cursor_and_rfc_3339_times() {
        let cursor = Cursor {
            stored_at: 5,
            uuid: Uuid::nil(),
        };
        assert_eq!(ListQuery::new(None, None, None, None), Ok(query(100)));
        assert_eq!(ListQuery::new(Some(1), None, None, None), Ok(query(1)));
        assert_eq!(
            ListQuery::new(Some(1000), None, None, None),
            Ok(query(1000))
        );
        for limit in [0, 1001, u64::MAX] {
            assert_eq!(
                ListQuery::new(Some(limit), None, None, None),
                Err(BadQuery),
                "{limit}"
            );
        }
        assert_eq!(
            ListQuery::new(
                None,
                Some(&cursor.to_string()),
                Some("2026-10-25T13:00:00+01:00"),
                Some("2026-10-26T00:00:00.5Z"),
            ),
            Ok(ListQuery {
                limit: 100,
                cursor: Some(cursor),
                since: Some(datetime!(2026-10-25 12:00 UTC)),
                until: Some(datetime!(2026-10-26 00:00:00.5 UTC)),
            })
        );
        assert_eq!(
            ListQuery::new(None, Some("not-a-cursor"), None, None),
            Err(BadQuery)
        );
        for time in ["2026-10-25", "yesterday", "", "2026-10-25T12:00:00"] {
            assert_eq!(
                ListQuery::new(None, None, Some(time), None),
                Err(BadQuery),
                "{time}"
            );
            assert_eq!(
                ListQuery::new(None, None, None, Some(time)),
                Err(BadQuery),
                "{time}"
            );
        }
        // Valid RFC 3339, but past what the catalog's nanoseconds hold.
        assert_eq!(
            ListQuery::new(None, None, Some("2263-01-01T00:00:00Z"), None),
            Err(BadQuery)
        );
    }

    #[test]
    fn a_query_string_names_each_parameter_at_most_once() {
        assert_eq!(ListQuery::from_query(None), Ok(query(100)));
        assert_eq!(
            ListQuery::from_query(Some("limit=7&since=2026-10-25T12%3A00%3A00Z")),
            Ok(ListQuery {
                since: Some(datetime!(2026-10-25 12:00 UTC)),
                ..query(7)
            })
        );
        for bad in [
            "limit=abc",
            "limit=-1",
            "limit=1.5",
            "limit=",
            "offset=5",
            "limit=5&limit=6",
            "cursor=",
            "until=2026-10-25",
        ] {
            assert_eq!(ListQuery::from_query(Some(bad)), Err(BadQuery), "{bad}");
        }
    }

    #[test]
    fn an_empty_store_lists_nothing_and_records_the_list() {
        let f = fixture();
        let page = f.store.list(&f.caller, &query(10)).expect("a page");
        assert_eq!(
            page,
            Listing {
                vcons: vec![],
                next: None
            }
        );
        let listed = records(&f, "list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["actor"], f.caller.key.to_string());
        assert_eq!(listed[0]["status"], 200);
        assert_eq!(listed[0]["target"], serde_json::Value::Null);
        assert_eq!(listed[0]["version"], serde_json::Value::Null);
    }

    #[test]
    fn the_list_is_newest_first_by_first_store_and_leaves_out_deleted_vcons() {
        let f = fixture();
        let v1 = store(&f, 1, at(1));
        let v2 = store(&f, 2, at(2));
        store(&f, 3, at(3));
        // Replacing vCon 1 later changes its version, not its place.
        f.clock.set(at(10));
        let (found, body) = vcon(&uuid(1), "replaced");
        let v1b = f
            .store
            .replace(&f.caller, v1, &found, &body)
            .expect("replaced");
        f.store
            .delete(&f.caller, found_uuid(3), Retention::Years(7))
            .expect("deleted");
        let page = f.store.list(&f.caller, &query(10)).expect("a page");
        assert_eq!(
            page.vcons,
            vec![
                Listed {
                    uuid: found_uuid(2),
                    version: v2,
                    stored_at: at(2),
                    updated_at: at(2),
                },
                Listed {
                    uuid: found_uuid(1),
                    version: v1b,
                    stored_at: at(1),
                    updated_at: at(10),
                },
            ]
        );
        assert_eq!(page.next, None);
        // A restored vCon comes back in its first place.
        f.store.restore(&f.caller, found_uuid(3)).expect("restored");
        let page = f.store.list(&f.caller, &query(10)).expect("a page");
        assert_eq!(uuids(&[page]), vec![uuid(3), uuid(2), uuid(1)]);
    }

    fn found_uuid(n: u32) -> Uuid {
        Uuid::parse_str(&uuid(n)).expect("uuid")
    }

    /// Nine vCons, some stored in the same instant, so ties fall to the
    /// uuid: newest first, then the higher uuid first.
    fn nine(f: &Fixture) -> Vec<String> {
        for (n, minute) in [
            (1, 1),
            (2, 2),
            (3, 2),
            (4, 2),
            (5, 3),
            (6, 4),
            (7, 4),
            (8, 5),
            (9, 6),
        ] {
            store(f, n, at(minute));
        }
        [9, 8, 7, 6, 5, 4, 3, 2, 1].map(uuid).to_vec()
    }

    #[test]
    fn paging_at_every_size_returns_each_vcon_exactly_once_in_order() {
        let f = fixture();
        let all = nine(&f);
        for limit in 1..=10 {
            let pages = pages(&f, query(limit));
            assert_eq!(uuids(&pages), all, "limit {limit}");
            let full = usize::try_from(limit).expect("small");
            for page in &pages[..pages.len() - 1] {
                assert_eq!(page.vcons.len(), full, "limit {limit}");
                assert!(page.next.is_some());
            }
            assert_eq!(pages.last().expect("a page").next, None, "limit {limit}");
            assert_eq!(
                pages.len(),
                all.len().div_ceil(full).max(1),
                "limit {limit}"
            );
        }
    }

    #[test]
    fn changes_between_pages_move_no_other_vcon() {
        let f = fixture();
        nine(&f);
        let first = f.store.list(&f.caller, &query(3)).expect("a page");
        assert_eq!(uuids(std::slice::from_ref(&first)), [9, 8, 7].map(uuid));
        // Between pages: a new vCon, vCon 5 replaced, vCon 4 deleted, and
        // vCon 8, already listed, deleted then restored.
        store(&f, 10, at(30));
        f.clock.set(at(31));
        let current = f
            .store
            .list(&f.caller, &query(10))
            .expect("a page")
            .vcons
            .iter()
            .find(|v| v.uuid == found_uuid(5))
            .expect("vCon 5")
            .version;
        let (found, body) = vcon(&uuid(5), "replaced");
        f.store
            .replace(&f.caller, current, &found, &body)
            .expect("replaced");
        f.store
            .delete(&f.caller, found_uuid(4), Retention::Years(7))
            .expect("deleted");
        f.store
            .delete(&f.caller, found_uuid(8), Retention::Years(7))
            .expect("deleted");
        f.store.restore(&f.caller, found_uuid(8)).expect("restored");
        let rest = pages(
            &f,
            ListQuery {
                cursor: first.next,
                ..query(3)
            },
        );
        let mut seen = uuids(std::slice::from_ref(&first));
        seen.extend(uuids(&rest));
        assert_eq!(seen, [9, 8, 7, 6, 5, 3, 2, 1].map(uuid));
        let unique: BTreeSet<&String> = seen.iter().collect();
        assert_eq!(unique.len(), seen.len(), "no vCon twice");
    }

    #[test]
    fn since_is_inclusive_and_until_exclusive() {
        let f = fixture();
        nine(&f);
        let between = |since: Option<u8>, until: Option<u8>| {
            uuids(&pages(
                &f,
                ListQuery {
                    since: since.map(at),
                    until: until.map(at),
                    ..query(2)
                },
            ))
        };
        assert_eq!(between(Some(2), Some(4)), [5, 4, 3, 2].map(uuid));
        assert_eq!(between(Some(4), None), [9, 8, 7, 6].map(uuid));
        assert_eq!(between(None, Some(2)), vec![uuid(1)]);
        assert_eq!(between(Some(4), Some(4)), Vec::<String>::new());
        assert_eq!(between(Some(5), Some(2)), Vec::<String>::new());
    }

    #[test]
    fn a_live_vcon_whose_version_row_is_missing_is_damage_and_recorded() {
        let f = fixture();
        store(&f, 1, at(1));
        f.store
            .conn_for_test()
            .execute("DELETE FROM version", [])
            .expect("damaged");
        assert_eq!(
            f.store.list(&f.caller, &query(10)),
            Err(StoreError::Damaged)
        );
        let listed = records(&f, "list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["status"], 500);
    }

    #[test]
    fn a_list_whose_record_fails_lists_nothing() {
        let f = fixture();
        store(&f, 1, at(1));
        f.store.break_audit();
        assert_eq!(
            f.store.list(&f.caller, &query(10)),
            Err(StoreError::AuditFailed)
        );
    }

    #[test]
    fn a_page_as_json() {
        let cursor = Cursor {
            stored_at: 1,
            uuid: Uuid::nil(),
        };
        let version = VersionId([0xab; 16]);
        let page = Listing {
            vcons: vec![Listed {
                uuid: found_uuid(1),
                version,
                stored_at: datetime!(2026-10-25 12:00 UTC),
                updated_at: datetime!(2026-10-25 12:30:00.25 UTC),
            }],
            next: Some(cursor),
        };
        assert_eq!(
            page.to_json(),
            json!({
                "vcons": [{
                    "uuid": uuid(1),
                    "version": version.to_string(),
                    "stored_at": "2026-10-25T12:00:00Z",
                    "updated_at": "2026-10-25T12:30:00.25Z",
                }],
                "next": cursor.to_string(),
            })
        );
        let last = Listing {
            vcons: vec![],
            next: None,
        };
        assert_eq!(last.to_json(), json!({"vcons": [], "next": null}));
    }

    /// The columns of the vcon table, with their types, defaults and
    /// whether each takes NULL.
    fn vcon_columns(f: &Fixture) -> Vec<(String, String, Option<String>, bool)> {
        let conn = f.store.conn_for_test();
        let mut rows = conn.prepare("PRAGMA table_info(vcon)").expect("prepare");
        rows.query_map([], |r| Ok((r.get(1)?, r.get(2)?, r.get(4)?, r.get(3)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    #[test]
    fn a_version_1_catalog_is_upgraded_with_each_uuids_first_store_time() {
        let mut f = fixture();
        let fresh = vcon_columns(&f);
        let v1 = store(&f, 1, at(1));
        store(&f, 2, at(2));
        f.clock.set(at(20));
        let (found, body) = vcon(&uuid(1), "replaced");
        f.store
            .replace(&f.caller, v1, &found, &body)
            .expect("replaced");
        let before = f.store.list(&f.caller, &query(10)).expect("a page");
        // Back to version 1: no stored_at column and no index.
        f.store
            .conn_for_test()
            .execute_batch(
                "DROP INDEX vcon_listed;
                 ALTER TABLE vcon DROP COLUMN stored_at;
                 PRAGMA user_version = 1;",
            )
            .expect("made version 1");
        f.reopen();
        let version: i64 = f
            .store
            .conn_for_test()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("version");
        assert_eq!(version, 2);
        assert_eq!(vcon_columns(&f), fresh, "the same table as a new catalog");
        let after = f.store.list(&f.caller, &query(10)).expect("a page");
        assert_eq!(after, before, "the first store times come back");
        let indexed: i64 = f
            .store
            .conn_for_test()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'vcon_listed'",
                [],
                |r| r.get(0),
            )
            .expect("index");
        assert_eq!(indexed, 1);
    }
}
