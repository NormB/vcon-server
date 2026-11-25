//! Writes under way: the rule by which recovery tells a write's object
//! that is not yet named by a row from a leftover, apart from the objects
//! and rows it reads, so its boundaries can be tested directly.
//!
//! A create or a replace records a pending row for its version, with the
//! time it began, in the transaction before it writes the object, and
//! deletes that row in the transaction that adds the version's row. So an
//! object with no version row is either a write under way, which has its
//! pending row, or a leftover of a write that ended without its row.
//!
//! Recovery removes a leftover. With other writers possibly at work on
//! the same catalog, it keeps an object whose pending row is younger than
//! [`WRITE_LEASE`]; when the process is the catalog's only writer, no
//! write can be under way, and every such object is a leftover. A pending
//! row recovery takes for a leftover is deleted before its object is
//! removed, and the write's own last transaction refuses to add the
//! version's row when its pending row is gone, so no row can name an
//! object recovery removed.
//!
//! A pending row's time and the `now` recovery compares it with come from
//! one clock, the catalog's (`CatalogTx::clock_now`, `RegistryTx::clock_now`):
//! on a database, the database's `now()`, so servers on hosts whose clocks
//! differ by more than the lease still agree on which write is under way;
//! on a data directory, which one process holds, that process's clock.
//! Recovery reads that clock once, in the transaction that deletes the
//! leftover rows, and judges by that reading in its later read of the
//! rows too: a row it kept there is never taken for a leftover's after,
//! which would remove the object of a row that stays.

/// How long a write's pending row keeps its object from recovery, in
/// nanoseconds: one hour, far above the time a write takes between its
/// first and its last transaction (the object store's client bounds each
/// request to seconds).
pub(crate) const WRITE_LEASE: i64 = 3_600 * 1_000_000_000;

/// Who else may write the catalog while recovery runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Writers {
    /// No one: the process holds the catalog alone (a data directory's
    /// lock, or the database's locks of a command that runs alone), so no
    /// write is under way and every pending row is a leftover's.
    None,
    /// Other processes may: a pending row younger than the lease is a write
    /// under way.
    Maybe,
}

/// Whether a write whose pending row says it began at `started_at` may
/// still be under way at `now`, both in nanoseconds since the epoch: with
/// other writers, when the two times are less than [`WRITE_LEASE`] apart.
/// Both times are the catalog's clock's. A start after `now` is that
/// clock set back since, and is under way within the same distance; one
/// further ahead is no write that a clock can explain.
pub(crate) fn under_way(writers: Writers, started_at: i64, now: i64) -> bool {
    match writers {
        Writers::None => false,
        Writers::Maybe => {
            (i128::from(now) - i128::from(started_at)).abs() < i128::from(WRITE_LEASE)
        }
    }
}

/// Whether recovery removes an object: one with no version row naming it
/// that is not a write under way, and a temporary name (a write's first
/// step on local files) unless its write is under way.
pub(crate) fn leftover(named: bool, temporary: bool, under_way: bool) -> bool {
    !under_way && (temporary || !named)
}

#[cfg(test)]
mod tests {
    use super::{WRITE_LEASE, Writers, leftover, under_way};

    /// 2026-10-23 12:00 UTC, in nanoseconds.
    const NOW: i64 = 1_792_756_800 * 1_000_000_000;

    // Success: a write that began within the lease is under way.

    #[test]
    fn a_write_that_began_now_is_under_way() {
        assert!(under_way(Writers::Maybe, NOW, NOW), "a write begun now");
    }

    #[test]
    fn a_write_that_began_a_minute_ago_is_under_way() {
        let minute = 60 * 1_000_000_000;
        assert!(
            under_way(Writers::Maybe, NOW - minute, NOW),
            "a write begun a minute ago"
        );
    }

    #[test]
    fn a_write_one_nanosecond_inside_the_lease_is_under_way() {
        assert!(
            under_way(Writers::Maybe, NOW - WRITE_LEASE + 1, NOW),
            "a write begun one nanosecond less than the lease ago"
        );
    }

    #[test]
    fn a_write_begun_on_a_clock_a_minute_ahead_is_under_way() {
        let minute = 60 * 1_000_000_000;
        assert!(
            under_way(Writers::Maybe, NOW + minute, NOW),
            "another process's clock a minute ahead"
        );
    }

    #[test]
    fn an_object_no_row_names_whose_write_is_under_way_is_kept() {
        assert!(
            !leftover(false, false, true),
            "an object of a write under way is kept"
        );
    }

    #[test]
    fn a_temporary_object_whose_write_is_under_way_is_kept() {
        assert!(
            !leftover(false, true, true),
            "a temporary name of a write under way is kept"
        );
    }

    #[test]
    fn an_object_a_row_names_is_kept() {
        assert!(!leftover(true, false, false), "a named object is kept");
    }

    #[test]
    fn an_object_a_row_names_with_a_pending_row_is_kept() {
        assert!(
            !leftover(true, false, true),
            "a named object with a pending row is kept"
        );
    }

    #[test]
    fn a_lease_is_an_hour() {
        assert_eq!(WRITE_LEASE, 3_600_000_000_000, "the lease, in nanoseconds");
    }

    #[test]
    fn a_write_begun_at_the_epoch_is_under_way_at_the_epoch() {
        assert!(
            under_way(Writers::Maybe, 0, 0),
            "the same time, at the epoch"
        );
    }

    #[test]
    fn a_write_before_the_epoch_is_under_way_just_after_it() {
        assert!(
            under_way(Writers::Maybe, -1, 1),
            "two nanoseconds apart across the epoch"
        );
    }

    #[test]
    fn a_write_one_nanosecond_inside_the_lease_ahead_is_under_way() {
        assert!(
            under_way(Writers::Maybe, NOW + WRITE_LEASE - 1, NOW),
            "a clock ahead by one nanosecond less than the lease"
        );
    }

    // Failure: an object no write is under way for is a leftover.

    #[test]
    fn an_object_no_row_names_and_no_write_is_under_way_for_is_a_leftover() {
        assert!(
            leftover(false, false, false),
            "an object with no row and no write under way"
        );
    }

    #[test]
    fn a_temporary_object_no_write_is_under_way_for_is_a_leftover() {
        assert!(
            leftover(false, true, false),
            "a temporary name with no write under way"
        );
    }

    #[test]
    fn a_temporary_object_a_row_names_is_a_leftover() {
        assert!(
            leftover(true, true, false),
            "a temporary name is never a version's object"
        );
    }

    #[test]
    fn a_write_that_began_two_hours_ago_is_not_under_way() {
        assert!(
            !under_way(Writers::Maybe, NOW - 2 * WRITE_LEASE, NOW),
            "a write begun twice the lease ago"
        );
    }

    #[test]
    fn a_write_at_exactly_the_lease_is_not_under_way() {
        assert!(
            !under_way(Writers::Maybe, NOW - WRITE_LEASE, NOW),
            "a write begun exactly the lease ago"
        );
    }

    #[test]
    fn a_write_exactly_the_lease_ahead_is_not_under_way() {
        assert!(
            !under_way(Writers::Maybe, NOW + WRITE_LEASE, NOW),
            "a clock ahead by exactly the lease"
        );
    }

    #[test]
    fn a_write_a_day_ahead_is_not_under_way() {
        assert!(
            !under_way(Writers::Maybe, NOW + 24 * WRITE_LEASE, NOW),
            "a clock a day ahead"
        );
    }

    #[test]
    fn with_no_other_writer_a_write_begun_now_is_not_under_way() {
        assert!(
            !under_way(Writers::None, NOW, NOW),
            "the process holds the catalog alone"
        );
    }

    #[test]
    fn with_no_other_writer_a_write_a_minute_old_is_not_under_way() {
        let minute = 60 * 1_000_000_000;
        assert!(
            !under_way(Writers::None, NOW - minute, NOW),
            "the process holds the catalog alone"
        );
    }

    #[test]
    fn with_no_other_writer_a_write_ahead_is_not_under_way() {
        assert!(
            !under_way(Writers::None, NOW + 1, NOW),
            "the process holds the catalog alone"
        );
    }

    #[test]
    fn a_write_at_the_lease_with_no_other_writer_is_not_under_way() {
        assert!(
            !under_way(Writers::None, NOW - WRITE_LEASE, NOW),
            "the process holds the catalog alone"
        );
    }

    #[test]
    fn a_write_one_nanosecond_past_the_lease_is_not_under_way() {
        assert!(
            !under_way(Writers::Maybe, NOW - WRITE_LEASE - 1, NOW),
            "a write begun one nanosecond more than the lease ago"
        );
    }

    // Edge: the extremes of the time range do not overflow.

    #[test]
    fn the_earliest_start_at_the_latest_time_is_not_under_way() {
        assert!(
            !under_way(Writers::Maybe, i64::MIN, i64::MAX),
            "the widest distance there is"
        );
    }

    #[test]
    fn the_latest_start_at_the_earliest_time_is_not_under_way() {
        assert!(
            !under_way(Writers::Maybe, i64::MAX, i64::MIN),
            "the widest distance there is, ahead"
        );
    }

    #[test]
    fn the_latest_start_at_the_latest_time_is_under_way() {
        assert!(
            under_way(Writers::Maybe, i64::MAX, i64::MAX),
            "the same time, at the end of the range"
        );
    }

    #[test]
    fn the_earliest_start_at_the_earliest_time_is_under_way() {
        assert!(
            under_way(Writers::Maybe, i64::MIN, i64::MIN),
            "the same time, at the start of the range"
        );
    }

    #[test]
    fn a_start_inside_the_lease_of_the_latest_time_is_under_way() {
        assert!(
            under_way(Writers::Maybe, i64::MAX - WRITE_LEASE + 1, i64::MAX),
            "one nanosecond inside the lease, at the end of the range"
        );
    }

    #[test]
    fn a_start_inside_the_lease_of_the_earliest_time_is_under_way() {
        assert!(
            under_way(Writers::Maybe, i64::MIN + WRITE_LEASE - 1, i64::MIN),
            "one nanosecond inside the lease ahead, at the start of the range"
        );
    }
}
