//! When a kept version is erased: the purge rule, apart from the files and
//! rows it touches, so its boundaries can be tested directly.

use time::OffsetDateTime;

use crate::retention::Retention;

/// What the purge rule reads of one version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kept {
    /// When the version was stored; retention counts from here.
    pub stored_at: OffsetDateTime,
    /// Whether it was replaced or its uuid deleted. A current version is
    /// never purged.
    pub hidden: bool,
    /// Whether its uuid is under a legal hold.
    pub held: bool,
}

/// Whether `version` is due for purge at `now` under `retention`: it is
/// hidden, its uuid has no hold, and `retention` has passed since it was
/// stored, counted in calendar time in UTC. A period that ends past the
/// last time this build can hold never passes. A version hidden after it
/// reached that age is due at once.
pub fn due(version: &Kept, retention: Retention, now: OffsetDateTime) -> bool {
    version.hidden
        && !version.held
        && retention
            .after(version.stored_at)
            .is_some_and(|end| now >= end)
}

#[cfg(test)]
mod tests {
    use time::Duration;
    use time::macros::datetime;

    use super::*;

    const SECOND: Duration = Duration::seconds(1);

    fn hidden(stored_at: OffsetDateTime) -> Kept {
        Kept {
            stored_at,
            hidden: true,
            held: false,
        }
    }

    #[test]
    fn a_hidden_version_is_due_from_the_second_its_period_ends() {
        let seven = Retention::Years(7);
        let cases = [
            // Stored on an ordinary day: due seven calendar years later.
            (
                datetime!(2026-03-15 10:00 UTC),
                datetime!(2033-03-15 10:00 UTC),
            ),
            // Stored on 29 February: due on 28 February seven years later,
            // not on 1 March.
            (
                datetime!(2028-02-29 12:00 UTC),
                datetime!(2035-02-28 12:00 UTC),
            ),
        ];
        for (stored_at, end) in cases {
            let version = hidden(stored_at);
            assert!(
                !due(&version, seven, end - SECOND),
                "{stored_at}: a second short"
            );
            assert!(due(&version, seven, end), "{stored_at}: at the end");
            assert!(
                due(&version, seven, end + SECOND),
                "{stored_at}: a second past"
            );
        }
    }

    #[test]
    fn a_hold_keeps_a_version_past_its_end() {
        let stored_at = datetime!(2028-02-29 12:00 UTC);
        let end = datetime!(2035-02-28 12:00 UTC);
        let version = Kept {
            held: true,
            ..hidden(stored_at)
        };
        for now in [end - SECOND, end, end + SECOND, end + Duration::days(3650)] {
            assert!(!due(&version, Retention::Years(7), now), "{now}");
        }
    }

    #[test]
    fn a_current_version_is_never_due() {
        let version = Kept {
            hidden: false,
            ..hidden(datetime!(2026-03-15 10:00 UTC))
        };
        let end = datetime!(2033-03-15 10:00 UTC);
        for now in [end - SECOND, end, end + SECOND] {
            assert!(!due(&version, Retention::Years(7), now), "{now}");
        }
    }

    #[test]
    fn a_zero_period_makes_a_hidden_version_due_at_once() {
        let stored_at = datetime!(2026-03-15 10:00 UTC);
        let version = hidden(stored_at);
        assert!(due(&version, Retention::Days(0), stored_at));
        assert!(!due(&version, Retention::Days(0), stored_at - SECOND));
    }

    #[test]
    fn a_period_ending_past_the_last_time_held_is_never_due() {
        let version = hidden(datetime!(9998-06-01 00:00 UTC));
        let last = datetime!(9999-12-31 23:59:59 UTC);
        assert!(!due(&version, Retention::Years(7), last));
    }
}
