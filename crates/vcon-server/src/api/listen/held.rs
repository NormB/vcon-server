//! What a held connection is doing, and from that the limit it is under
//! and what the listener does when the limit is reached.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

use super::ConnectionLimits;

/// How a connection that reached its limit is ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum End {
    /// Closed at once: it stalled before or inside a request head.
    Close,
    /// Told to close, in order: it was idle. An HTTP/2 connection is sent
    /// GOAWAY, and has `ConnectionLimits::goodbye` to end.
    Goodbye,
}

/// What a connection is doing, as far as its limits ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Doing {
    /// Whether it speaks HTTP/2, where bytes that arrive between requests
    /// need not be a request head: they may be a ping or a setting.
    h2: bool,
    /// Whether a request head has arrived on it.
    requests: bool,
    /// The requests being answered now.
    in_flight: usize,
    /// When the first bytes of a request head arrived, on HTTP/1.1, while
    /// no request was being answered.
    head_begun: Option<Instant>,
    /// Since when no request is being answered: the handshake's end, or
    /// the end of the last answer.
    free_since: Instant,
    /// When it was told to close.
    told: Option<Instant>,
}

impl Doing {
    /// A connection whose handshake ended at `now`.
    pub(super) fn new(now: Instant, h2: bool) -> Self {
        Self {
            h2,
            requests: false,
            in_flight: 0,
            head_begun: None,
            free_since: now,
            told: None,
        }
    }

    /// Bytes arrived at `now`. Whether that began a request head.
    pub(super) fn bytes(&mut self, now: Instant) -> bool {
        if self.h2 || self.in_flight > 0 || self.head_begun.is_some() {
            return false;
        }
        self.head_begun = Some(now);
        true
    }

    /// A request head arrived whole, over HTTP/2 when `h2`.
    pub(super) fn begin(&mut self, h2: bool) {
        self.h2 |= h2;
        self.requests = true;
        self.in_flight += 1;
        self.head_begun = None;
    }

    /// The answer to a request ended at `now`.
    pub(super) fn end(&mut self, now: Instant) {
        self.in_flight = self.in_flight.saturating_sub(1);
        if self.in_flight == 0 {
            self.free_since = now;
        }
    }

    /// The connection was told to close at `now`.
    pub(super) fn tell(&mut self, now: Instant) {
        self.told.get_or_insert(now);
    }

    /// When the connection reaches a limit, and how it is then ended.
    /// None while a request is being answered: an answer may take as long
    /// as it needs.
    pub(super) fn due(&self, limits: &ConnectionLimits) -> Option<(Instant, End)> {
        if self.in_flight > 0 {
            return None;
        }
        if let Some(told) = self.told {
            return Some((told.max(self.free_since) + limits.goodbye, End::Close));
        }
        let first = (!self.requests).then(|| self.free_since + limits.first_head);
        let head = self.head_begun.map(|begun| begun + limits.head);
        match (first, head) {
            (Some(first), Some(head)) => Some((first.min(head), End::Close)),
            (Some(at), None) | (None, Some(at)) => Some((at, End::Close)),
            (None, None) => Some((self.free_since + limits.idle, End::Goodbye)),
        }
    }

    /// How the connection is ended at `now`: None while it is under no
    /// limit, or has not reached the one it is under.
    pub(super) fn reached(&self, limits: &ConnectionLimits, now: Instant) -> Option<End> {
        self.due(limits)
            .filter(|(at, _)| *at <= now)
            .map(|(_, end)| end)
    }
}

/// A connection's [`Doing`], shared by the parts that see it change: the
/// bytes read, each request and the end of each answer.
#[derive(Debug)]
pub(super) struct Held {
    doing: Mutex<Doing>,
    changed: Notify,
}

impl Held {
    pub(super) fn new(h2: bool) -> Arc<Self> {
        Arc::new(Self {
            doing: Mutex::new(Doing::new(Instant::now(), h2)),
            changed: Notify::new(),
        })
    }

    fn change<T>(&self, change: impl FnOnce(&mut Doing) -> T) -> T {
        // A panic while the lock is held leaves a `Doing` that is whole.
        let mut doing = self
            .doing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        change(&mut doing)
    }

    /// Bytes arrived from the other side.
    pub(super) fn bytes(&self) {
        if self.change(|doing| doing.bytes(Instant::now())) {
            self.changed.notify_one();
        }
    }

    /// A request head arrived whole. The request is counted as being
    /// answered until what is returned is dropped.
    pub(super) fn begin(self: &Arc<Self>, h2: bool) -> InFlight {
        self.change(|doing| doing.begin(h2));
        self.changed.notify_one();
        InFlight(self.clone())
    }

    /// The connection was told to close.
    pub(super) fn tell(&self) {
        self.change(|doing| doing.tell(Instant::now()));
    }

    /// [`Doing::due`], now.
    pub(super) fn due(&self, limits: &ConnectionLimits) -> Option<(Instant, End)> {
        self.change(|doing| doing.due(limits))
    }

    /// [`Doing::reached`], now.
    pub(super) fn reached(&self, limits: &ConnectionLimits) -> Option<End> {
        self.change(|doing| doing.reached(limits, Instant::now()))
    }

    /// Completes when what the connection is doing has changed.
    pub(super) async fn changed(&self) {
        self.changed.notified().await;
    }
}

/// A request that is being answered. Dropped when its answer ends.
#[derive(Debug)]
pub(super) struct InFlight(Arc<Held>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.change(|doing| doing.end(Instant::now()));
        self.0.changed.notify_one();
    }
}

/// Lets something through once in each `every`, however often it is asked:
/// the log line of a listener that is full.
#[derive(Debug, Default)]
pub(super) struct Sparse {
    last: Option<Instant>,
}

impl Sparse {
    /// Whether to let this one through, at `now`.
    pub(super) fn due(&mut self, now: Instant, every: Duration) -> bool {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < every)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: ConnectionLimits = ConnectionLimits {
        handshake: Duration::from_secs(1),
        first_head: Duration::from_secs(30),
        head: Duration::from_secs(10),
        idle: Duration::from_secs(120),
        goodbye: Duration::from_secs(5),
        accept_pause: Duration::from_secs(1),
        max: 1,
    };

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// A connection that answered one request, from `start + 1 s` to
    /// `start + 2 s`.
    fn answered(start: Instant, h2: bool) -> Doing {
        let mut doing = Doing::new(start, h2);
        doing.bytes(start + secs(1));
        doing.begin(h2);
        doing.end(start + secs(2));
        doing
    }

    // Before the first request.

    #[test]
    fn a_new_connection_is_closed_at_the_first_head_limit() {
        let start = Instant::now();
        for h2 in [false, true] {
            assert_eq!(
                Doing::new(start, h2).due(&LIMITS),
                Some((start + secs(30), End::Close)),
                "h2 {h2}"
            );
        }
    }

    #[test]
    fn a_first_head_begun_is_under_the_head_limit_from_its_first_bytes() {
        let start = Instant::now();
        let mut doing = Doing::new(start, false);
        assert!(doing.bytes(start + secs(3)), "the head began");
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(13), End::Close)),
            "ten seconds from the first bytes"
        );
    }

    #[test]
    fn a_first_head_begun_late_is_still_under_the_first_head_limit() {
        let start = Instant::now();
        let mut doing = Doing::new(start, false);
        doing.bytes(start + secs(25));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(30), End::Close)),
            "the lesser of the two"
        );
    }

    #[test]
    fn more_bytes_of_a_head_do_not_move_its_limit() {
        let start = Instant::now();
        let mut doing = Doing::new(start, false);
        doing.bytes(start + secs(3));
        assert!(!doing.bytes(start + secs(8)), "the head had begun");
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(13), End::Close)),
            "still ten seconds from the first bytes"
        );
    }

    #[test]
    fn bytes_on_http2_before_a_request_begin_no_head() {
        let start = Instant::now();
        let mut doing = Doing::new(start, true);
        assert!(!doing.bytes(start + secs(3)), "a preface is no head");
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(30), End::Close)),
            "the first head limit alone"
        );
    }

    #[test]
    fn a_connection_is_never_idle_before_its_first_request() {
        let start = Instant::now();
        let limits = ConnectionLimits {
            idle: secs(1),
            ..LIMITS
        };
        assert_eq!(
            Doing::new(start, false).due(&limits),
            Some((start + secs(30), End::Close)),
            "the first head limit, not the idle limit"
        );
    }

    // While a request is answered.

    #[test]
    fn a_connection_answering_a_request_is_under_no_limit() {
        let start = Instant::now();
        for h2 in [false, true] {
            let mut doing = Doing::new(start, h2);
            doing.begin(h2);
            assert_eq!(doing.due(&LIMITS), None, "h2 {h2}");
        }
    }

    #[test]
    fn a_connection_with_one_of_two_requests_still_answered_is_under_no_limit() {
        let start = Instant::now();
        let mut doing = Doing::new(start, true);
        doing.begin(true);
        doing.begin(true);
        doing.end(start + secs(1));
        assert_eq!(doing.due(&LIMITS), None, "one stream is open");
        doing.end(start + secs(4));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(124), End::Goodbye)),
            "idle from the end of the last"
        );
    }

    #[test]
    fn bytes_while_a_request_is_answered_begin_no_head() {
        let start = Instant::now();
        let mut doing = Doing::new(start, false);
        doing.begin(false);
        assert!(!doing.bytes(start + secs(1)), "a body, not a head");
        doing.end(start + secs(2));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(122), End::Goodbye)),
            "idle from the end of the answer"
        );
    }

    #[test]
    fn an_end_without_a_request_leaves_the_count_at_none() {
        let start = Instant::now();
        let mut doing = Doing::new(start, false);
        doing.end(start + secs(1));
        doing.begin(false);
        assert_eq!(doing.due(&LIMITS), None, "one request is answered");
    }

    // Between requests.

    #[test]
    fn a_connection_between_requests_is_told_to_close_at_the_idle_limit() {
        let start = Instant::now();
        for h2 in [false, true] {
            assert_eq!(
                answered(start, h2).due(&LIMITS),
                Some((start + secs(122), End::Goodbye)),
                "h2 {h2}"
            );
        }
    }

    #[test]
    fn the_idle_limit_counts_from_the_end_of_the_answer_not_from_its_start() {
        let start = Instant::now();
        let mut doing = Doing::new(start, false);
        doing.begin(false);
        doing.end(start + secs(500));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(620), End::Goodbye)),
            "120 seconds after the answer ended"
        );
    }

    #[test]
    fn a_later_head_begun_is_under_the_head_limit_and_not_the_idle_limit() {
        let start = Instant::now();
        let mut doing = answered(start, false);
        assert!(doing.bytes(start + secs(121)), "a head began");
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(131), End::Close)),
            "ten seconds from its first bytes, past the idle limit"
        );
    }

    #[test]
    fn a_later_head_begun_early_is_closed_at_the_head_limit_before_the_idle_limit() {
        let start = Instant::now();
        let mut doing = answered(start, false);
        doing.bytes(start + secs(3));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(13), End::Close)),
            "ten seconds from the first bytes"
        );
    }

    #[test]
    fn the_head_limit_does_not_close_a_connection_that_waits_between_requests() {
        let start = Instant::now();
        let limits = ConnectionLimits {
            head: secs(1),
            ..LIMITS
        };
        assert_eq!(
            answered(start, false).due(&limits),
            Some((start + secs(122), End::Goodbye)),
            "no head has begun"
        );
    }

    #[test]
    fn the_first_head_limit_does_not_apply_after_the_first_request() {
        let start = Instant::now();
        let limits = ConnectionLimits {
            first_head: secs(1),
            ..LIMITS
        };
        assert_eq!(
            answered(start, true).due(&limits),
            Some((start + secs(122), End::Goodbye)),
            "the idle limit"
        );
    }

    #[test]
    fn bytes_on_http2_between_requests_do_not_move_the_idle_limit() {
        let start = Instant::now();
        let mut doing = answered(start, true);
        assert!(!doing.bytes(start + secs(100)), "a ping is no head");
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(122), End::Goodbye)),
            "the idle limit, where it was"
        );
    }

    #[test]
    fn a_connection_found_to_speak_http2_at_its_first_request_begins_no_heads_after() {
        let start = Instant::now();
        // No protocol was agreed in the handshake.
        let mut doing = Doing::new(start, false);
        assert!(
            doing.bytes(start + secs(1)),
            "the preface looks like a head"
        );
        doing.begin(true);
        doing.end(start + secs(2));
        assert!(!doing.bytes(start + secs(3)), "now known to be HTTP/2");
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(122), End::Goodbye)),
            "idle like any HTTP/2 connection"
        );
    }

    #[test]
    fn a_request_clears_the_head_that_began_it() {
        let start = Instant::now();
        let mut doing = Doing::new(start, false);
        doing.bytes(start + secs(1));
        doing.begin(false);
        doing.end(start + secs(50));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(170), End::Goodbye)),
            "idle, not inside the head that was answered"
        );
    }

    // Told to close.

    #[test]
    fn a_connection_told_to_close_is_closed_after_the_goodbye_wait() {
        let start = Instant::now();
        let mut doing = answered(start, true);
        doing.tell(start + secs(122));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(127), End::Close)),
            "five seconds after it was told"
        );
    }

    #[test]
    fn a_request_that_came_as_the_connection_was_told_to_close_is_answered_whole() {
        let start = Instant::now();
        let mut doing = answered(start, true);
        doing.tell(start + secs(122));
        doing.begin(true);
        assert_eq!(doing.due(&LIMITS), None, "its answer may take its time");
        doing.end(start + secs(300));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(305), End::Close)),
            "the goodbye wait, from the end of that answer"
        );
    }

    #[test]
    fn a_connection_is_told_to_close_once() {
        let start = Instant::now();
        let mut doing = answered(start, true);
        doing.tell(start + secs(122));
        doing.tell(start + secs(126));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(127), End::Close)),
            "from the first telling"
        );
    }

    #[test]
    fn a_head_begun_after_the_telling_does_not_outlast_the_goodbye_wait() {
        let start = Instant::now();
        let mut doing = answered(start, false);
        doing.tell(start + secs(122));
        doing.bytes(start + secs(123));
        assert_eq!(
            doing.due(&LIMITS),
            Some((start + secs(127), End::Close)),
            "the goodbye wait, not the head limit"
        );
    }

    // A limit reached.

    #[test]
    fn a_limit_is_not_reached_before_its_time() {
        let start = Instant::now();
        let idle = answered(start, true);
        assert_eq!(idle.reached(&LIMITS, start + secs(2)), None, "just idle");
        assert_eq!(
            idle.reached(&LIMITS, start + secs(122) - Duration::from_millis(1)),
            None,
            "a millisecond early"
        );
        let silent = Doing::new(start, false);
        assert_eq!(
            silent.reached(&LIMITS, start + secs(30) - Duration::from_millis(1)),
            None,
            "a millisecond before the first head limit"
        );
    }

    #[test]
    fn a_limit_is_reached_at_its_time_and_after() {
        let start = Instant::now();
        let idle = answered(start, true);
        for late in [0, 1, 1000] {
            assert_eq!(
                idle.reached(&LIMITS, start + secs(122 + late)),
                Some(End::Goodbye),
                "{late} s after the idle limit"
            );
        }
        let silent = Doing::new(start, false);
        assert_eq!(
            silent.reached(&LIMITS, start + secs(30)),
            Some(End::Close),
            "at the first head limit"
        );
    }

    #[test]
    fn a_request_that_arrives_as_the_idle_limit_runs_out_is_not_cut_off() {
        let start = Instant::now();
        let mut doing = answered(start, true);
        doing.begin(true);
        assert_eq!(
            doing.reached(&LIMITS, start + secs(122)),
            None,
            "a request is being answered"
        );
        // Its answer ended as the first idle limit ran out: idle anew.
        doing.end(start + secs(122));
        assert_eq!(
            doing.reached(&LIMITS, start + secs(123)),
            None,
            "idle for a second"
        );
        assert_eq!(
            doing.reached(&LIMITS, start + secs(242)),
            Some(End::Goodbye),
            "the whole idle limit after that answer"
        );
    }

    #[test]
    fn a_connection_told_to_close_reaches_the_goodbye_wait() {
        let start = Instant::now();
        let mut doing = answered(start, true);
        doing.tell(start + secs(122));
        assert_eq!(
            doing.reached(&LIMITS, start + secs(126)),
            None,
            "a second early"
        );
        assert_eq!(
            doing.reached(&LIMITS, start + secs(127)),
            Some(End::Close),
            "at the wait's end"
        );
    }

    // The shared form.

    #[tokio::test]
    async fn a_new_connection_has_reached_no_limit() {
        let held = Held::new(false);
        assert_eq!(held.reached(&LIMITS), None, "thirty seconds to go");
        let none = ConnectionLimits {
            first_head: Duration::ZERO,
            ..LIMITS
        };
        assert_eq!(held.reached(&none), Some(End::Close), "no time at all");
    }

    #[tokio::test]
    async fn a_request_is_in_flight_until_it_is_dropped() {
        let held = Held::new(true);
        let first = held.begin(true);
        let second = held.begin(true);
        assert_eq!(held.due(&LIMITS), None, "two are answered");
        drop(first);
        assert_eq!(held.due(&LIMITS), None, "one is answered");
        drop(second);
        assert_eq!(
            held.due(&LIMITS).map(|(_, end)| end),
            Some(End::Goodbye),
            "idle once both ended"
        );
    }

    #[tokio::test]
    async fn each_change_wakes_whoever_waits_for_one() {
        let held = Held::new(false);
        held.bytes();
        tokio::time::timeout(secs(5), held.changed())
            .await
            .expect("woken by the head's first bytes");
        let request = held.begin(false);
        tokio::time::timeout(secs(5), held.changed())
            .await
            .expect("woken by the request");
        drop(request);
        tokio::time::timeout(secs(5), held.changed())
            .await
            .expect("woken by the end of the answer");
    }

    #[tokio::test]
    async fn bytes_that_begin_no_head_wake_no_one() {
        let held = Held::new(true);
        held.bytes();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), held.changed())
                .await
                .is_err(),
            "nothing changed"
        );
    }

    #[tokio::test]
    async fn a_connection_told_to_close_is_due_to_be_closed() {
        let held = Held::new(true);
        drop(held.begin(true));
        held.tell();
        assert_eq!(
            held.due(&LIMITS).map(|(_, end)| end),
            Some(End::Close),
            "after the goodbye wait"
        );
    }

    // The sparse log line.

    #[test]
    fn the_first_is_let_through() {
        assert!(
            Sparse::default().due(Instant::now(), secs(60)),
            "nothing was let through before"
        );
    }

    #[test]
    fn a_second_inside_the_interval_is_held_back() {
        let start = Instant::now();
        let mut sparse = Sparse::default();
        sparse.due(start, secs(60));
        assert!(!sparse.due(start, secs(60)), "at the same instant");
        assert!(!sparse.due(start + secs(59), secs(60)), "a second early");
    }

    #[test]
    fn one_is_let_through_again_when_the_interval_has_passed() {
        let start = Instant::now();
        let mut sparse = Sparse::default();
        sparse.due(start, secs(60));
        assert!(sparse.due(start + secs(60), secs(60)), "at the interval");
        assert!(!sparse.due(start + secs(61), secs(60)), "counted anew");
        assert!(sparse.due(start + secs(120), secs(60)), "and again");
    }

    #[test]
    fn those_held_back_do_not_put_off_the_next() {
        let start = Instant::now();
        let mut sparse = Sparse::default();
        sparse.due(start, secs(60));
        for second in 1..60 {
            assert!(!sparse.due(start + secs(second), secs(60)), "{second}");
        }
        assert!(
            sparse.due(start + secs(60), secs(60)),
            "sixty seconds after the one let through"
        );
    }

    #[test]
    fn a_clock_that_reads_earlier_lets_none_through() {
        let start = Instant::now() + secs(100);
        let mut sparse = Sparse::default();
        sparse.due(start, secs(60));
        assert!(
            !sparse.due(start - secs(50), secs(60)),
            "fifty seconds before the last"
        );
    }
}
