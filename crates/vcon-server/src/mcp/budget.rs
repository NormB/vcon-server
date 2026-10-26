//! The byte budget a result is held to: the call's `max_bytes`, clamped to
//! `mcp.response_bytes`, counted as the length of the result's JSON text.
//! A list drops whole rows from its end; a vCon's text is cut on a
//! character boundary. Something always remains, so a cursor always moves.

use serde_json::Value;

/// The budget for one call: `max_bytes` when it is given, more than 0 and
/// under `ceiling`, else `ceiling`.
#[must_use]
pub fn resolve(max_bytes: Option<u64>, ceiling: usize) -> usize {
    match max_bytes {
        None | Some(0) => ceiling,
        Some(n) => usize::try_from(n).map_or(ceiling, |n| n.min(ceiling)),
    }
}

/// The length of `value`'s JSON text, as the result's text block holds it.
#[must_use]
pub fn size(value: &Value) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |text| text.len())
}

/// How many of `rows` to keep: as many as `fits` takes, from the start, and
/// never fewer than one.
pub fn rows_that_fit(rows: usize, fits: impl Fn(usize) -> bool) -> usize {
    let mut kept = rows;
    while kept > 1 && !fits(kept) {
        kept -= 1;
    }
    kept
}

/// The longest piece of `text` from `start` that `fits` takes, cut on a
/// character boundary and never empty unless nothing is left, and where the
/// next piece starts, None at the end.
pub fn piece(
    text: &str,
    start: usize,
    mut fits: impl FnMut(&str, bool) -> bool,
) -> (&str, Option<usize>) {
    let rest = &text[start..];
    let ends: Vec<usize> = rest
        .char_indices()
        .map(|(at, c)| at + c.len_utf8())
        .collect();
    let Some(&whole) = ends.last() else {
        return ("", None);
    };
    if fits(rest, false) {
        return (rest, None);
    }
    // The largest end that fits, by halving; the first character always.
    let (mut low, mut high) = (0, ends.len() - 1);
    while low < high {
        let mid = (low + high).div_ceil(2);
        if fits(&rest[..ends[mid]], true) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let end = ends[low];
    let next = (end < whole).then_some(start + end);
    (&rest[..end], next)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_budget_is_the_callers_up_to_the_ceiling() {
        assert_eq!(resolve(None, 100), 100);
        assert_eq!(resolve(Some(0), 100), 100);
        assert_eq!(resolve(Some(40), 100), 40);
        assert_eq!(resolve(Some(400), 100), 100);
        assert_eq!(resolve(Some(u64::MAX), 100), 100);
    }

    #[test]
    fn the_size_is_the_json_text() {
        assert_eq!(size(&json!({"a": "\u{e9}\""})), r#"{"a":"é\""}"#.len());
    }

    #[test]
    fn rows_are_dropped_from_the_end_but_one_stays() {
        assert_eq!(rows_that_fit(5, |n| n <= 3), 3);
        assert_eq!(rows_that_fit(5, |_| true), 5);
        assert_eq!(rows_that_fit(5, |_| false), 1);
        assert_eq!(rows_that_fit(0, |_| false), 0);
    }

    #[test]
    fn pieces_join_back_and_each_fits() {
        let text = "a\u{e9}b\u{1f600}cdef";
        for limit in 1..=text.len() + 2 {
            let mut joined = String::new();
            let mut start = 0;
            loop {
                let (got, next) = piece(text, start, |p, _| p.len() <= limit);
                assert!(!got.is_empty());
                assert!(
                    got.len() <= limit || got.chars().count() == 1,
                    "{limit}: {got}"
                );
                joined.push_str(got);
                match next {
                    Some(at) => start = at,
                    None => break,
                }
            }
            assert_eq!(joined, text, "{limit}");
        }
        assert_eq!(piece(text, text.len(), |_, _| true), ("", None));
    }

    #[test]
    fn a_piece_says_whether_more_follows() {
        let mut told = Vec::new();
        let (got, next) = piece("abcd", 0, |p, more| {
            told.push((p.to_owned(), more));
            p.len() <= 2
        });
        assert_eq!((got, next), ("ab", Some(2)));
        assert!(told.contains(&("abcd".to_owned(), false)));
        assert!(
            told.iter()
                .filter(|(p, _)| p != "abcd")
                .all(|(_, more)| *more)
        );
    }
}
