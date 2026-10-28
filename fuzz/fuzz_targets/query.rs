//! Any text as a query string, read as `GET /vcons` reads it (names,
//! percent decoding, limit, cursor, time bounds) and as the audit routes
//! read it (after, limit). A query that is taken holds a limit in range,
//! a cursor written back as it was read, and times the catalog can hold.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vcon_server::audit::AuditQuery;
use vcon_server::store::{Cursor, ListQuery};

fuzz_target!(|text: &str| {
    if let Ok(query) = ListQuery::from_query(Some(text)) {
        assert!(
            (1..=ListQuery::MAX_LIMIT).contains(&query.limit),
            "an accepted list query's limit is within its bounds: {}",
            query.limit
        );
        if let Some(cursor) = query.cursor {
            assert_eq!(
                Cursor::parse(&cursor.to_string()),
                Some(cursor),
                "a cursor reads back from the text it writes"
            );
        }
        for at in [query.since, query.until].into_iter().flatten() {
            assert!(i64::try_from(at.unix_timestamp_nanos()).is_ok(), "{at}");
        }
    }
    if let Ok(query) = AuditQuery::from_query(Some(text)) {
        assert!(
            (1..=AuditQuery::MAX_LIMIT).contains(&query.limit),
            "an accepted audit query's limit is within its bounds: {}",
            query.limit
        );
        assert!(
            query.after >= 0,
            "an accepted audit query starts at no negative sequence number: {}",
            query.after
        );
    }
});
