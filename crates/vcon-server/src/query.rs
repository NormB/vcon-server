//! Query strings: the one rule every route that takes parameters reads
//! them by, so the HTTP routes and the tools built on the same functions
//! refuse the same queries.

use std::collections::BTreeMap;

/// A query the route does not take: a name it does not know, a name given
/// twice, or a value it cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadQuery;

/// The parameters of `query`, percent-decoded, by name. A name that is not
/// in `names`, or that is given twice, is refused, so a misspelt or
/// repeated parameter never goes unnoticed. No query is no parameters.
pub fn pairs(query: Option<&str>, names: &[&str]) -> Result<BTreeMap<String, String>, BadQuery> {
    let mut read = BTreeMap::new();
    // Empty pieces, as a trailing `&` leaves, name nothing.
    for piece in query.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        if !piece.contains('=') {
            return Err(BadQuery);
        }
        let (name, value) = form_urlencoded::parse(piece.as_bytes())
            .next()
            .ok_or(BadQuery)?;
        if !names.contains(&name.as_ref()) {
            return Err(BadQuery);
        }
        if read.insert(name.into_owned(), value.into_owned()).is_some() {
            return Err(BadQuery);
        }
    }
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &["limit", "cursor", "since"];

    fn read(query: &str) -> Result<Vec<(String, String)>, BadQuery> {
        pairs(Some(query), NAMES).map(|p| p.into_iter().collect())
    }

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn no_query_and_an_empty_one_are_no_parameters() {
        assert_eq!(pairs(None, NAMES), Ok(BTreeMap::new()));
        assert_eq!(read(""), Ok(vec![]));
    }

    #[test]
    fn known_names_are_read_with_their_values_decoded() {
        assert_eq!(
            read("limit=10&since=2026-10-25T12%3A00%3A00%2B01%3A00"),
            Ok(vec![
                pair("limit", "10"),
                pair("since", "2026-10-25T12:00:00+01:00")
            ])
        );
        assert_eq!(read("cursor="), Ok(vec![pair("cursor", "")]));
    }

    #[test]
    fn an_unknown_name_is_refused() {
        assert_eq!(read("limit=10&offset=5"), Err(BadQuery));
        assert_eq!(read("Limit=10"), Err(BadQuery), "names are exact");
    }

    #[test]
    fn a_name_given_twice_is_refused() {
        assert_eq!(read("limit=10&limit=20"), Err(BadQuery));
        assert_eq!(read("limit=10&limit=10"), Err(BadQuery));
    }

    #[test]
    fn a_name_with_no_equals_sign_is_refused() {
        assert_eq!(read("limit"), Err(BadQuery));
    }
}
