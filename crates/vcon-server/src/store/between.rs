//! The stop test of one tenant's step, asked between the step's items.
//!
//! The server asks whether it is stopping before each tenant's step
//! (serve.rs), so a step that has begun does its first item without
//! asking again. Before each later item it asks; once the answer is true,
//! the step ends after the item under way, and every later ask answers
//! true without asking again. An item is a unit that leaves nothing half
//! done when the next one is not begun: one version's purge, one pack
//! file's rewrite, one object read into a pack file, one packed object's
//! removal, one date's merge.

use std::cell::Cell;

/// A step's stop test, as the module describes.
pub(crate) struct Between<'a> {
    stopping: &'a dyn Fn() -> bool,
    begun: Cell<bool>,
    stopped: Cell<bool>,
}

impl<'a> Between<'a> {
    pub(crate) fn new(stopping: &'a dyn Fn() -> bool) -> Between<'a> {
        Between {
            stopping,
            begun: Cell::new(false),
            stopped: Cell::new(false),
        }
    }

    /// Whether the step ends before its next item: false for its first;
    /// then what the stop test answers, until it answers true.
    pub(crate) fn stop(&self) -> bool {
        if self.stopped.get() {
            return true;
        }
        if !self.begun.replace(true) {
            return false;
        }
        let stop = (self.stopping)();
        self.stopped.set(stop);
        stop
    }

    /// Whether the step was told to stop.
    pub(crate) fn stopped(&self) -> bool {
        self.stopped.get()
    }
}

/// A test that never answers true: a step that is not stopped.
pub(crate) fn never() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn the_first_item_is_not_asked_before() {
        let asked = Cell::new(0);
        let test = || {
            asked.set(asked.get() + 1);
            true
        };
        let between = Between::new(&test);
        assert!(!between.stop(), "the first item goes");
        assert_eq!(asked.get(), 0, "without an ask");
    }

    #[test]
    fn a_later_item_is_asked_before() {
        let asked = Cell::new(0);
        let test = || {
            asked.set(asked.get() + 1);
            false
        };
        let between = Between::new(&test);
        let answers = [between.stop(), between.stop(), between.stop()];
        assert_eq!(answers, [false, false, false], "nothing stops");
        assert_eq!(asked.get(), 2, "asked before the second and third");
        assert!(!between.stopped(), "not stopped");
    }

    #[test]
    fn once_stopped_it_asks_no_more() {
        let asked = Cell::new(0);
        let test = || {
            asked.set(asked.get() + 1);
            true
        };
        let between = Between::new(&test);
        between.stop();
        assert!(between.stop(), "the second item stops");
        assert!(between.stop(), "and every one after");
        assert_eq!(asked.get(), 1, "asked once");
        assert!(between.stopped(), "stopped");
    }

    #[test]
    fn never_never_stops() {
        let between = Between::new(&never);
        assert!((0..5).all(|_| !between.stop()), "every item goes");
        assert!(!between.stopped(), "not stopped");
    }
}
