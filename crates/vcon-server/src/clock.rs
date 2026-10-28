//! Time, behind an interface so tests can set it.

use time::OffsetDateTime;

/// Where the server gets the time.
pub trait Clock: Send + Sync {
    /// The time now, in UTC.
    fn now(&self) -> OffsetDateTime;
}

/// A time as the catalogs keep it: nanoseconds since the Unix epoch.
pub(crate) fn nanos(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos()).expect("a time before 2262")
}

/// A time the catalogs kept as `nanos` wrote it, or None if it is out of
/// range.
pub(crate) fn from_nanos(at: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(at)).ok()
}

/// The system's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

#[cfg(test)]
pub(crate) use manual::ManualClock;

#[cfg(test)]
mod manual {
    use std::sync::Mutex;

    use time::OffsetDateTime;

    use super::Clock;

    /// A clock that stands still until a test moves it.
    pub(crate) struct ManualClock(Mutex<OffsetDateTime>);

    impl ManualClock {
        pub(crate) fn at(now: OffsetDateTime) -> ManualClock {
            ManualClock(Mutex::new(now))
        }

        pub(crate) fn set(&self, now: OffsetDateTime) {
            *self.0.lock().expect("clock") = now;
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> OffsetDateTime {
            *self.0.lock().expect("clock")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_is_close_to_now() {
        let before = OffsetDateTime::now_utc();
        let now = SystemClock.now();
        let after = OffsetDateTime::now_utc();
        assert!(
            before <= now && now <= after,
            "the system clock reads the time now: {before} <= {now} <= {after}"
        );
        assert!(
            now.offset().is_utc(),
            "the trait promises UTC, and comparing instants alone cannot tell"
        );
    }
}
