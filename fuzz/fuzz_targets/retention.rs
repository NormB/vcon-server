//! Any text as a retention period, as the admin API and the configuration
//! carry it, added to any time. The first 8 bytes are the time in seconds
//! since 1970; the rest is the period. A period that parses is written
//! back as it was read, and adding it never goes back in time.
#![no_main]

use libfuzzer_sys::fuzz_target;
use time::OffsetDateTime;
use vcon_server::retention::Retention;

fuzz_target!(|data: &[u8]| {
    let Some((seconds, text)) = data.split_first_chunk::<8>() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(text) else {
        return;
    };
    let Some(period) = Retention::parse(text) else {
        return;
    };
    assert_eq!(period.to_string(), text);
    let Ok(at) = OffsetDateTime::from_unix_timestamp(i64::from_le_bytes(*seconds)) else {
        return;
    };
    if let Some(end) = period.after(at) {
        assert!(end >= at, "{period} after {at} gave {end}");
    }
});
