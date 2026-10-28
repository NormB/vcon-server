//! Query strings: the one rule every route that takes parameters reads
//! them by, so the HTTP routes and the tools built on the same functions
//! refuse the same queries.

use std::collections::BTreeMap;
use std::fmt;

/// A query the route does not take: a name it does not know, a name given
/// twice, or a value it cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadQuery;

impl fmt::Display for BadQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(concat!(
            "the query names a parameter the route does not take, ",
            "names one twice, or gives a value it cannot read"
        ))
    }
}

impl std::error::Error for BadQuery {}

/// The parameters of `query`, percent-decoded, by name. A name that is not
/// in `names`, or that is given twice, is refused, so a misspelt or
/// repeated parameter never goes unnoticed. No query is no parameters.
///
/// # Errors
///
/// [`BadQuery`] if a name is not in `names`, a name is given twice, or a
/// piece has no `=`.
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
        assert_eq!(
            pairs(None, NAMES),
            Ok(BTreeMap::new()),
            "no query string has no parameters"
        );
        assert_eq!(
            read(""),
            Ok(vec![]),
            "an empty query string has none either"
        );
    }

    #[test]
    fn known_names_are_read_with_their_values_decoded() {
        assert_eq!(
            read("limit=10&since=2026-10-25T12%3A00%3A00%2B01%3A00"),
            Ok(vec![
                pair("limit", "10"),
                pair("since", "2026-10-25T12:00:00+01:00")
            ]),
            "each value is percent-decoded"
        );
        assert_eq!(
            read("cursor="),
            Ok(vec![pair("cursor", "")]),
            "a name with an empty value is given, with that value"
        );
    }

    #[test]
    fn an_unknown_name_is_refused() {
        assert_eq!(
            read("limit=10&offset=5"),
            Err(BadQuery),
            "offset is not a name this query takes"
        );
        assert_eq!(read("Limit=10"), Err(BadQuery), "names are exact");
    }

    #[test]
    fn a_name_given_twice_is_refused() {
        assert_eq!(
            read("limit=10&limit=20"),
            Err(BadQuery),
            "two values would be ambiguous"
        );
        assert_eq!(
            read("limit=10&limit=10"),
            Err(BadQuery),
            "a repeated name is refused even with the same value"
        );
    }

    #[test]
    fn a_name_with_no_equals_sign_is_refused() {
        assert_eq!(
            read("limit"),
            Err(BadQuery),
            "a parameter is a name, an equals sign and a value"
        );
    }
}
