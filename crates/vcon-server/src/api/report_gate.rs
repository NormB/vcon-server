//! How many reports of a content-security-policy violation a listener
//! takes in a minute (`limits.csp_reports`, and from one address
//! `limits.csp_reports_per_address`).
//!
//! The report route takes no key, and each report it answers costs one
//! write to the registry. The gate bounds that: past either number a
//! report is refused with 429 before its body is read, and no write is
//! made for it. Refusals are added up in memory and written as one record
//! for the minute, so a flood shows in the admin log as one row that says
//! how many were refused.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

use time::OffsetDateTime;

/// How many reports a listener takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportLimits {
    /// The most reports taken in one minute, from every address together.
    pub per_minute: u32,
    /// The most reports taken in one minute from one address.
    pub per_address: u32,
}

impl ReportLimits {
    /// A browser reports a violation once for each thing a page was
    /// refused, so a console page with a misbehaving extension sends a
    /// few reports for each load: 20 a minute is several loads from one
    /// address, and 120 is six such addresses at once.
    pub const DEFAULT: Self = Self {
        per_minute: 120,
        per_address: 20,
    };
}

/// What the gate says of one report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Admission {
    /// Whether the report is taken; one that is not is answered 429.
    pub(crate) taken: bool,
    /// The minute of the report.
    pub(crate) minute: OffsetDateTime,
    /// Whether this is the first report refused in its minute, the one
    /// that starts the wait for the minute's end.
    pub(crate) first_refusal: bool,
    /// An earlier minute and the refusals in it that no record holds yet:
    /// the caller writes them.
    pub(crate) earlier: Option<(OffsetDateTime, u64)>,
}

/// One listener's count of the reports of the current minute.
#[derive(Debug)]
pub struct Gate {
    limits: ReportLimits,
    window: Mutex<Window>,
}

#[derive(Debug, Default)]
struct Window {
    minute: Option<OffsetDateTime>,
    taken: u32,
    by_address: HashMap<IpAddr, u32>,
    refused: u64,
    #[cfg(test)]
    flusher: Option<tokio::task::JoinHandle<()>>,
}

/// The start of the minute `now` is in, in UTC.
pub(crate) fn minute_of(now: OffsetDateTime) -> OffsetDateTime {
    now.to_offset(time::UtcOffset::UTC)
        .replace_second(0)
        .and_then(|t| t.replace_nanosecond(0))
        .expect("0 is a valid second and nanosecond")
}

impl Gate {
    /// A gate that has seen no report.
    #[must_use]
    pub fn new(limits: ReportLimits) -> Self {
        Self {
            limits,
            window: Mutex::new(Window::default()),
        }
    }

    /// Takes or refuses one report from `source` at `now`. A report whose
    /// source is not known counts toward the minute's number only.
    pub(crate) fn admit(&self, source: Option<IpAddr>, now: OffsetDateTime) -> Admission {
        let minute = minute_of(now);
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut earlier = None;
        if window.minute != Some(minute) {
            if let Some(before) = window.minute
                && window.refused > 0
            {
                earlier = Some((before, window.refused));
            }
            window.minute = Some(minute);
            window.taken = 0;
            window.refused = 0;
            window.by_address.clear();
        }
        let from_source = source.and_then(|address| window.by_address.get(&address).copied());
        let taken = window.taken < self.limits.per_minute
            && from_source.unwrap_or(0) < self.limits.per_address;
        if taken {
            window.taken += 1;
            // Only a report that is taken adds an address, so the gate
            // holds no more of them than the minute's number.
            if let Some(address) = source {
                *window.by_address.entry(address).or_insert(0) += 1;
            }
        } else {
            window.refused = window.refused.saturating_add(1);
        }
        Admission {
            taken,
            minute,
            first_refusal: !taken && window.refused == 1,
            earlier,
        }
    }

    /// The refusals of `minute` that no record holds yet, which the
    /// caller writes; they are counted here no longer.
    pub(crate) fn take_refused(&self, minute: OffsetDateTime) -> u64 {
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if window.minute == Some(minute) {
            std::mem::take(&mut window.refused)
        } else {
            0
        }
    }

    /// How many addresses the gate holds a count for.
    #[cfg(test)]
    pub(crate) fn addresses(&self) -> usize {
        self.window.lock().expect("the window").by_address.len()
    }

    /// Keeps the task that writes a minute's refusals at its end, so a
    /// test can wait for it.
    #[cfg(test)]
    pub(crate) fn set_flusher(&self, task: tokio::task::JoinHandle<()>) {
        self.window.lock().expect("the window").flusher = Some(task);
    }

    /// Waits for the task that writes the refusals, if one runs.
    #[cfg(test)]
    pub(crate) async fn settled(&self) {
        let task = self.window.lock().expect("the window").flusher.take();
        if let Some(task) = task {
            task.await.expect("the task that writes the refusals");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use time::macros::datetime;

    use super::{Gate, ReportLimits, minute_of};

    const NOON: time::OffsetDateTime = datetime!(2026-10-23 12:00:00 UTC);

    fn gate(per_minute: u32, per_address: u32) -> Gate {
        Gate::new(ReportLimits {
            per_minute,
            per_address,
        })
    }

    /// An RFC 5737 address.
    fn address(last: u8) -> Option<IpAddr> {
        Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, last)))
    }

    /// How many of `reports` reports from `source` at `now` are taken.
    fn taken(gate: &Gate, source: Option<IpAddr>, now: time::OffsetDateTime, reports: u32) -> u32 {
        (0..reports)
            .map(|_| u32::from(gate.admit(source, now).taken))
            .sum()
    }

    #[test]
    fn the_defaults_are_120_a_minute_and_20_from_one_address() {
        assert_eq!(ReportLimits::DEFAULT.per_minute, 120, "a minute");
        assert_eq!(ReportLimits::DEFAULT.per_address, 20, "from one address");
    }

    #[test]
    fn a_minute_starts_at_its_first_second_in_utc() {
        assert_eq!(
            minute_of(datetime!(2026-10-23 12:00:59.999 UTC)),
            NOON,
            "the last instant"
        );
        assert_eq!(minute_of(NOON), NOON, "the first instant");
        assert_eq!(
            minute_of(datetime!(2026-10-23 14:00:30 +02:00)),
            NOON,
            "another offset is the same minute"
        );
        // An offset with seconds in it: the instant is 12:00:15 UTC.
        assert_eq!(
            minute_of(datetime!(2026-10-23 12:00:45 +00:00:30)),
            NOON,
            "the minute is UTC's, not the offset's"
        );
    }

    #[test]
    fn the_first_report_is_taken() {
        let admission = gate(3, 2).admit(address(1), NOON);
        assert!(admission.taken, "{admission:?}");
        assert_eq!(admission.minute, NOON, "its minute");
        assert!(!admission.first_refusal, "nothing was refused");
        assert_eq!(admission.earlier, None, "nothing to write");
    }

    #[test]
    fn one_address_is_taken_up_to_its_number() {
        let gate = gate(100, 5);
        assert_eq!(taken(&gate, address(1), NOON, 5), 5, "the first five");
    }

    #[test]
    fn one_address_is_refused_past_its_number() {
        let gate = gate(100, 5);
        assert_eq!(taken(&gate, address(1), NOON, 9), 5, "five of nine");
    }

    #[test]
    fn another_address_is_taken_when_one_is_refused() {
        let gate = gate(100, 2);
        assert_eq!(taken(&gate, address(1), NOON, 4), 2, "the first address");
        assert_eq!(
            taken(&gate, address(2), NOON, 4),
            2,
            "the second has its own count"
        );
    }

    #[test]
    fn all_addresses_together_are_taken_up_to_the_minutes_number() {
        let gate = gate(6, 4);
        let total: u32 = (1..=3).map(|a| taken(&gate, address(a), NOON, 4)).sum();
        assert_eq!(total, 6, "four, two and none of three addresses' four each");
    }

    #[test]
    fn a_report_refused_for_the_minutes_number_uses_none_of_its_addresss() {
        let gate = gate(2, 2);
        assert_eq!(taken(&gate, address(1), NOON, 2), 2, "the minute's two");
        assert_eq!(
            taken(&gate, address(2), NOON, 3),
            0,
            "refused for the minute"
        );
        assert_eq!(
            gate.addresses(),
            1,
            "no count is kept for a refused address"
        );
    }

    #[test]
    fn the_gate_keeps_a_count_for_no_more_addresses_than_it_takes_reports() {
        let gate = gate(10, 1);
        for last in 0..=255 {
            gate.admit(address(last), NOON);
        }
        assert_eq!(gate.addresses(), 10, "256 addresses, ten reports taken");
    }

    #[test]
    fn a_report_with_no_known_source_counts_toward_the_minute_only() {
        let gate = gate(4, 1);
        assert_eq!(
            taken(&gate, None, NOON, 6),
            4,
            "the minute's four, with no address's one"
        );
    }

    #[test]
    fn an_ipv6_address_has_a_count_of_its_own() {
        let gate = gate(100, 1);
        // An RFC 3849 address.
        let v6 = Some(IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 5)));
        assert_eq!(taken(&gate, v6, NOON, 3), 1, "its one");
        assert_eq!(
            taken(&gate, address(1), NOON, 3),
            1,
            "and another address's"
        );
    }

    #[test]
    fn the_first_refusal_of_a_minute_is_marked_and_the_next_is_not() {
        let gate = gate(1, 1);
        gate.admit(address(1), NOON);
        let first = gate.admit(address(1), NOON);
        let second = gate.admit(address(1), NOON);
        assert!(!first.taken && first.first_refusal, "{first:?}");
        assert!(!second.taken && !second.first_refusal, "{second:?}");
    }

    #[test]
    fn a_new_minute_takes_reports_again() {
        let gate = gate(2, 2);
        assert_eq!(taken(&gate, address(1), NOON, 5), 2, "the first minute");
        let next = NOON + time::Duration::MINUTE;
        assert_eq!(taken(&gate, address(1), next, 5), 2, "the next");
    }

    #[test]
    fn the_last_instant_of_a_minute_is_still_that_minute() {
        let gate = gate(1, 1);
        gate.admit(address(1), NOON);
        let admission = gate.admit(address(1), datetime!(2026-10-23 12:00:59.999999 UTC));
        assert!(!admission.taken, "{admission:?}");
        assert_eq!(admission.minute, NOON, "the same minute");
    }

    #[test]
    fn a_new_minute_hands_over_the_refusals_of_the_one_before() {
        let gate = gate(1, 1);
        taken(&gate, address(1), NOON, 8);
        let next = gate.admit(address(1), NOON + time::Duration::MINUTE);
        assert!(next.taken, "{next:?}");
        assert_eq!(next.earlier, Some((NOON, 7)), "seven were refused at noon");
    }

    #[test]
    fn a_new_minute_counts_its_refusals_from_none() {
        let gate = gate(1, 1);
        taken(&gate, address(1), NOON, 4);
        let next = NOON + time::Duration::MINUTE;
        gate.admit(address(1), next);
        let refused = gate.admit(address(1), next);
        assert!(
            refused.first_refusal,
            "the first of the new minute: {refused:?}"
        );
        assert_eq!(gate.take_refused(next), 1, "one, not noon's three and one");
    }

    #[test]
    fn a_new_minute_after_none_refused_hands_over_nothing() {
        let gate = gate(5, 5);
        taken(&gate, address(1), NOON, 3);
        let next = gate.admit(address(1), NOON + time::Duration::MINUTE);
        assert_eq!(next.earlier, None, "nothing was refused");
    }

    #[test]
    fn refusals_are_handed_over_once() {
        let gate = gate(1, 1);
        taken(&gate, address(1), NOON, 4);
        let next = NOON + time::Duration::MINUTE;
        assert_eq!(
            gate.admit(address(1), next).earlier,
            Some((NOON, 3)),
            "the first report after"
        );
        assert_eq!(gate.admit(address(1), next).earlier, None, "the second");
        assert_eq!(gate.take_refused(NOON), 0, "and none is left to take");
    }

    #[test]
    fn the_refusals_of_a_minute_are_taken_by_naming_it() {
        let gate = gate(1, 1);
        taken(&gate, address(1), NOON, 6);
        assert_eq!(gate.take_refused(NOON), 5, "five refused");
        assert_eq!(gate.take_refused(NOON), 0, "taken once");
    }

    #[test]
    fn taking_the_refusals_of_another_minute_takes_none() {
        let gate = gate(1, 1);
        taken(&gate, address(1), NOON, 6);
        assert_eq!(
            gate.take_refused(NOON + time::Duration::MINUTE),
            0,
            "the next minute"
        );
        assert_eq!(
            gate.take_refused(NOON - time::Duration::MINUTE),
            0,
            "the one before"
        );
        assert_eq!(gate.take_refused(NOON), 5, "they are still there");
    }

    #[test]
    fn taking_from_a_gate_that_saw_no_report_takes_none() {
        assert_eq!(gate(1, 1).take_refused(NOON), 0, "nothing came");
    }

    #[test]
    fn taking_the_refusals_frees_no_place() {
        let gate = gate(1, 1);
        taken(&gate, address(1), NOON, 3);
        gate.take_refused(NOON);
        let again = gate.admit(address(1), NOON);
        assert!(!again.taken, "the minute's number is still used");
        assert!(
            again.first_refusal,
            "and it is the first refusal no record holds"
        );
        assert_eq!(gate.take_refused(NOON), 1, "counted from there");
    }

    #[test]
    fn a_handed_over_minute_is_not_taken_again_by_name() {
        let gate = gate(1, 1);
        taken(&gate, address(1), NOON, 4);
        gate.admit(address(1), NOON + time::Duration::MINUTE);
        assert_eq!(
            gate.take_refused(NOON),
            0,
            "the new minute's report took them"
        );
    }

    #[test]
    fn a_clock_that_goes_back_starts_a_minute_and_loses_no_refusal() {
        let gate = gate(1, 1);
        taken(&gate, address(1), NOON, 3);
        let before = NOON - time::Duration::MINUTE;
        let admission = gate.admit(address(1), before);
        assert!(admission.taken, "{admission:?}");
        assert_eq!(
            admission.earlier,
            Some((NOON, 2)),
            "noon's two are handed over"
        );
    }

    #[test]
    fn a_limit_of_the_largest_number_takes_every_report() {
        let gate = gate(u32::MAX, u32::MAX);
        assert_eq!(taken(&gate, address(1), NOON, 1000), 1000, "all of them");
    }

    #[test]
    fn refusals_past_four_thousand_million_are_counted() {
        let gate = gate(1, 1);
        gate.admit(address(1), NOON);
        gate.window.lock().expect("the window").refused = u64::from(u32::MAX);
        gate.admit(address(1), NOON);
        assert_eq!(
            gate.take_refused(NOON),
            u64::from(u32::MAX) + 1,
            "more than a u32 holds"
        );
    }
}
