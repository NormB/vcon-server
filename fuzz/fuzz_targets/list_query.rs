//! Any text as the query string of `GET /vcons`: the names, percent
//! decoding, the limit, the cursor and the time bounds. A query that is
//! taken holds a limit in range, a cursor that is written back as it was
//! read, and times the catalog can hold.
#![no_main]

use libfuzzer_sys::fuzz_target;
use vcon_server::store::{Cursor, ListQuery};

fuzz_target!(|text: &str| {
    let Ok(query) = ListQuery::from_query(Some(text)) else {
        return;
    };
    assert!((1..=ListQuery::MAX_LIMIT).contains(&query.limit));
    if let Some(cursor) = query.cursor {
        assert_eq!(Cursor::parse(&cursor.to_string()), Some(cursor));
    }
    for at in [query.since, query.until].into_iter().flatten() {
        assert!(i64::try_from(at.unix_timestamp_nanos()).is_ok(), "{at}");
    }
});
