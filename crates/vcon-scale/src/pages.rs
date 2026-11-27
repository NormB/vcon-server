//! The list phase's rule: which page to ask for next.

/// What following the pages came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Followed {
    /// The pages asked for, the failed one included.
    pub asked: u64,
    /// The first failure, after which nothing more was asked.
    pub error: Option<String>,
}

/// Asks for `pages` pages with `fetch`, which takes a page's cursor (None:
/// the newest page) and answers that page's `next`. After the last page
/// (no `next`) it asks for the newest again, so `pages` pages are asked
/// for whatever the list holds: the performance gate (scripts/perf_gate.py)
/// follows more pages than its vCons fill. The first failure ends it.
pub fn follow(
    pages: u64,
    mut fetch: impl FnMut(Option<&str>) -> Result<Option<String>, String>,
) -> Followed {
    let mut cursor: Option<String> = None;
    for asked in 1..=pages {
        match fetch(cursor.as_deref()) {
            Ok(next) => cursor = next,
            Err(why) => {
                return Followed {
                    asked,
                    error: Some(why),
                };
            }
        }
    }
    Followed {
        asked: pages,
        error: None,
    }
}
