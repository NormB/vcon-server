//! A retention period: a calendar period of whole years, months or days,
//! added to a time in UTC.

use std::fmt;

use time::{Date, Duration, Month, OffsetDateTime, UtcOffset};

/// How long a version is kept after it is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    Years(u32),
    Months(u32),
    Days(u32),
}

impl Retention {
    /// Reads a period written by `Display`: decimal digits with no sign and
    /// no leading zero, then `y`, `m` or `d`.
    pub fn parse(text: &str) -> Option<Retention> {
        let (digits, unit) = text.split_at_checked(text.len().checked_sub(1)?)?;
        let leading_zero = digits.len() > 1 && digits.starts_with('0');
        if leading_zero || !digits.bytes().all(|d| d.is_ascii_digit()) {
            return None;
        }
        let n = digits.parse().ok()?;
        match unit {
            "y" => Some(Retention::Years(n)),
            "m" => Some(Retention::Months(n)),
            "d" => Some(Retention::Days(n)),
            _ => None,
        }
    }

    /// The time this period after `at`, in UTC, or None if that is past
    /// the last time this build can hold.
    pub fn after(self, at: OffsetDateTime) -> Option<OffsetDateTime> {
        let at = at.to_offset(UtcOffset::UTC);
        let months = match self {
            Retention::Days(n) => return at.checked_add(Duration::days(i64::from(n))),
            Retention::Years(n) => i64::from(n) * 12,
            Retention::Months(n) => i64::from(n),
        };
        let total = i64::from(at.year()) * 12 + i64::from(u8::from(at.month())) - 1 + months;
        let year = i32::try_from(total.div_euclid(12)).ok()?;
        let month = Month::try_from(u8::try_from(total.rem_euclid(12) + 1).ok()?).ok()?;
        let day = at.day().min(month.length(year));
        Some(at.replace_date(Date::from_calendar_date(year, month, day).ok()?))
    }
}

impl fmt::Display for Retention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Retention::Years(n) => write!(f, "{n}y"),
            Retention::Months(n) => write!(f, "{n}m"),
            Retention::Days(n) => write!(f, "{n}d"),
        }
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn a_period_is_added_by_the_calendar_and_clamps_to_the_months_end() {
        let cases = [
            (
                datetime!(2026-10-23 12:00 UTC),
                Retention::Years(7),
                datetime!(2033-10-23 12:00 UTC),
            ),
            (
                datetime!(2028-02-29 12:00 UTC),
                Retention::Years(1),
                datetime!(2029-02-28 12:00 UTC),
            ),
            (
                datetime!(2028-02-29 12:00 UTC),
                Retention::Years(4),
                datetime!(2032-02-29 12:00 UTC),
            ),
            (
                datetime!(2026-01-31 12:00 UTC),
                Retention::Months(1),
                datetime!(2026-02-28 12:00 UTC),
            ),
            (
                datetime!(2028-01-31 12:00 UTC),
                Retention::Months(1),
                datetime!(2028-02-29 12:00 UTC),
            ),
            (
                datetime!(2026-12-31 12:00 UTC),
                Retention::Months(1),
                datetime!(2027-01-31 12:00 UTC),
            ),
            (
                datetime!(2026-11-15 12:00 UTC),
                Retention::Months(18),
                datetime!(2028-05-15 12:00 UTC),
            ),
            (
                datetime!(2026-10-23 12:00 UTC),
                Retention::Days(90),
                datetime!(2027-01-21 12:00 UTC),
            ),
            (
                datetime!(2028-02-28 12:00 UTC),
                Retention::Days(1),
                datetime!(2028-02-29 12:00 UTC),
            ),
            (
                datetime!(2026-10-23 12:34:56.789 UTC),
                Retention::Days(0),
                datetime!(2026-10-23 12:34:56.789 UTC),
            ),
            (
                datetime!(2026-10-23 12:34:56.789 UTC),
                Retention::Months(0),
                datetime!(2026-10-23 12:34:56.789 UTC),
            ),
            (
                datetime!(2026-10-23 12:34:56.789 UTC),
                Retention::Years(0),
                datetime!(2026-10-23 12:34:56.789 UTC),
            ),
            (
                datetime!(2026-05-31 23:59:59.999999999 UTC),
                Retention::Months(1),
                datetime!(2026-06-30 23:59:59.999999999 UTC),
            ),
        ];
        for (at, period, due) in cases {
            assert_eq!(period.after(at), Some(due), "{at} + {period:?}");
        }
    }

    /// 2026-03-31 02:00 at +05:00 is 2026-03-30 21:00 UTC. A month later in
    /// UTC is 2026-04-30 21:00; counted at +05:00 it would be 2026-04-29
    /// 21:00 UTC.
    #[test]
    fn a_period_is_counted_in_utc() {
        let due = Retention::Months(1)
            .after(datetime!(2026-03-31 02:00 +05:00))
            .expect("due");
        assert_eq!(due, datetime!(2026-04-30 21:00 UTC));
        assert!(due.offset().is_utc());
    }

    #[test]
    fn a_period_past_the_last_time_this_build_holds_is_none() {
        let late = datetime!(9999-12-31 23:59 UTC);
        for period in [
            Retention::Days(1),
            Retention::Months(1),
            Retention::Years(1),
        ] {
            assert_eq!(period.after(late), None, "{period:?}");
        }
        let now = datetime!(2026-10-23 12:00 UTC);
        for period in [
            Retention::Days(u32::MAX),
            Retention::Months(u32::MAX),
            Retention::Years(u32::MAX),
        ] {
            assert_eq!(period.after(now), None, "{period:?}");
        }
        assert_eq!(
            Retention::Years(7973).after(now),
            Some(datetime!(9999-10-23 12:00 UTC)),
            "the last year still holds"
        );
    }

    #[test]
    fn a_period_reads_back_what_it_writes() {
        for (text, period) in [
            ("7y", Retention::Years(7)),
            ("18m", Retention::Months(18)),
            ("90d", Retention::Days(90)),
            ("0d", Retention::Days(0)),
            ("0m", Retention::Months(0)),
            ("0y", Retention::Years(0)),
            ("4294967295d", Retention::Days(u32::MAX)),
        ] {
            assert_eq!(Retention::parse(text), Some(period), "{text}");
            assert_eq!(period.to_string(), text);
        }
    }

    #[test]
    fn only_the_written_spelling_is_read() {
        for text in [
            "",
            "y",
            "7",
            "7w",
            "7Y",
            "7 y",
            " 7y",
            "7y ",
            "-1y",
            "+1y",
            "07y",
            "00d",
            "1.5y",
            "7yd",
            "1e3d",
            "\u{0663}y",
            "4294967296d",
            "99999999999999999999y",
        ] {
            assert_eq!(Retention::parse(text), None, "{text:?}");
        }
    }
}
