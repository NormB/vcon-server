//! The one deadline the live tests give a test server: the PostgreSQL
//! servers `scripts/pg-test-server.sh` starts (`VCON_TEST_PG_*`) and the
//! S3-compatible service of `VCON_TEST_S3_*`.
//!
//! The server's own defaults, `catalog.postgres.connect_timeout` (5 s) and
//! `storage.s3.timeout` (30 s), are meant to fail fast in production. On
//! 2026-11-26, with the host's 1-minute load at 38-45 on 14 cores, two live
//! library tests took the 5 s default, had no answer in time, and failed
//! for a reason that was not the code under test, while the integration
//! tests' own helper gave the same servers 10 s. Every live helper, in the
//! library's tests and in the integration tests, reads this file instead,
//! so there is one rule; the server's defaults and their tests are left as
//! they are.
//!
//! The library includes this file as `crate::live_deadline`, and each
//! integration test crate that talks to a live server includes it at its
//! root as `live_deadline` (tests/pg/mod.rs reads it from there).

use std::path::Path;
use std::time::Duration;

/// The longest a live test waits for a test server to answer: one
/// PostgreSQL connection (TCP, TLS and the login) or one S3 request. Long
/// enough to ride out a loaded host, and bounded, so a server that never
/// answers still fails the test by name before the test runner's own
/// deadline ends it.
pub const LIVE_DEADLINE: Duration = Duration::from_secs(30);

/// `deadline` in whole seconds, as a configuration file takes it: rounded
/// up, so the server never waits less than asked, and at least 1, the
/// least the configuration accepts.
pub fn whole_seconds(deadline: Duration) -> u64 {
    let seconds = deadline.as_secs();
    let rounded = if deadline.subsec_nanos() > 0 {
        seconds.saturating_add(1)
    } else {
        seconds
    };
    rounded.max(1)
}

/// What a live helper says when `who` had no answer within `deadline`.
pub fn missed(who: &str, deadline: Duration) -> String {
    format!(
        "{who}: no answer within the live-test deadline ({deadline:?}, \
         tests/support/live_deadline.rs)"
    )
}

/// The `[catalog]` and `[catalog.postgres]` tables of a server
/// configuration: the test primary on localhost at `port`, the database
/// `dbname`, the role `user` with the password in `password_file`, the CA
/// in `ca_file`, and `deadline` for each connection.
pub fn postgres_tables(
    port: &str,
    dbname: &str,
    user: &str,
    ca_file: &Path,
    password_file: &Path,
    deadline: Duration,
) -> String {
    format!(
        "\n[catalog]\nbackend = \"postgres\"\n\n[catalog.postgres]\nhosts = [\"localhost\"]\nport \
         = {port}\ndbname = {dbname:?}\nuser = {user:?}\nca_file = {ca_file:?}\npassword_file = \
         {password_file:?}\nconnect_timeout = {}\n",
        whole_seconds(deadline)
    )
}

/// The `[storage]` and `[storage.s3]` tables of a server configuration:
/// the test service at `endpoint` over plain HTTP, path-style, the bucket
/// `bucket`, and `deadline` for each request. The keys are left to the
/// environment.
pub fn s3_tables(endpoint: &str, bucket: &str, deadline: Duration) -> String {
    format!(
        "\n[storage]\nobjects = \"s3\"\n\n[storage.s3]\nendpoint = {endpoint:?}\nregion = \
         \"us-east-1\"\nbucket = {bucket:?}\npath_style = true\nallow_http = true\ntimeout = {}\n",
        whole_seconds(deadline)
    )
}
