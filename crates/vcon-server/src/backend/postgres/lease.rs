//! The leases of the work one server does for every server of a
//! database ([`Job`]): the purge sweep and the pack run. A lease is a row
//! of `vcon.lease` (schema version 15): the job's name, the holder, when
//! the holder took it and when it ends.
//!
//! - Every time is the database's: the one statement that takes or renews
//!   a lease ([`TAKE`]) reads `now()`, and nothing here reads the clock of
//!   the host. Two servers whose clocks disagree still agree on who holds
//!   a lease, and until when.
//! - A holder is named once per connection to the database
//!   ([`new_holder`]), 128 random bits and then the host and the process
//!   id for an operator who reads the row: a server that starts again is a
//!   new holder, and never takes over the lease its earlier run held. It
//!   waits for that lease to end.
//! - A lease held by another holder is taken only once it has ended; the
//!   holder's own lease is renewed, and keeps the time it was taken while
//!   it has not ended.

use std::time::Duration;

use crate::backend::Job;

/// The one statement that takes the lease of the job named `$1` for the
/// holder `$2`, for `$3` seconds from the database's `now()`, or renews
/// it when `$2` holds it. It answers one row when `$2` holds the lease
/// after it, and none when another holder's lease has not ended.
///
/// `now()` is the time the transaction began, so the statement reads the
/// database's clock once: a lease taken ends exactly `$3` seconds after
/// the time it records as taken. A renewal keeps that time while the
/// lease has not ended; a lease that ended is taken anew, by its old
/// holder too.
pub(crate) const TAKE: &str = "INSERT INTO vcon.lease (name, holder, taken_at, expires_at) \
     VALUES ($1, $2, now(), now() + make_interval(secs => $3)) \
     ON CONFLICT (name) DO UPDATE SET holder = EXCLUDED.holder, \
     taken_at = CASE WHEN vcon.lease.holder = EXCLUDED.holder AND vcon.lease.expires_at \
     > now() THEN vcon.lease.taken_at ELSE now() END, \
     expires_at = EXCLUDED.expires_at \
     WHERE vcon.lease.holder = EXCLUDED.holder OR vcon.lease.expires_at <= now() \
     RETURNING holder";

/// The statement that gives up the lease of the job named `$1` when the
/// holder `$2` holds it; another holder's lease is left.
pub(crate) const GIVE_UP: &str = "DELETE FROM vcon.lease WHERE name = $1 AND holder = $2";

/// The longest host name a holder carries, in bytes.
pub(crate) const HOST_LEN: usize = 64;

/// A holder's name: `random` as 32 lowercase hex digits, then `@`, the
/// host as [`host_text`] gives it, `:` and the process id.
#[must_use]
pub(crate) fn holder(random: [u8; 16], host: &str, pid: u32) -> String {
    let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!("{hex}@{host}:{pid}")
}

/// `host` as a holder carries it: each byte that is not an ASCII letter,
/// digit, `.`, `-` or `_` made `_`, at most [`HOST_LEN`] bytes, and
/// `unknown` for an empty name.
#[must_use]
pub(crate) fn host_text(host: &[u8]) -> String {
    if host.is_empty() {
        return "unknown".to_owned();
    }
    host.iter()
        .take(HOST_LEN)
        .map(|&b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
                char::from(b)
            } else {
                '_'
            }
        })
        .collect()
}

/// A new holder's name for this process: fresh random bits, this host's
/// name and this process's id.
#[must_use]
pub(crate) fn new_holder() -> String {
    let mut random = [0; 16];
    aws_lc_rs::rand::fill(&mut random).expect("AWS-LC aborts instead of failing");
    holder(random, &host_text(&host_name()), std::process::id())
}

/// This host's name as the system gives it, or nothing when it does not.
fn host_name() -> Vec<u8> {
    let mut buf = [0u8; 256];
    // SAFETY: the pointer and the length are those of `buf`, which
    // gethostname writes at most that many bytes into.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return Vec::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf[..end].to_vec()
}

/// `ttl` in seconds, as the statement's `make_interval` takes them.
#[must_use]
pub(crate) fn seconds(ttl: Duration) -> f64 {
    ttl.as_secs_f64()
}

/// The name of `job`'s row.
#[must_use]
pub(crate) fn name(job: Job) -> &'static str {
    match job {
        Job::Sweep => "sweep",
        Job::Pack => "pack",
    }
}
