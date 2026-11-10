//! A reveal: a read of a vCon that says which part of it the caller is
//! about to show, so the audit log tells it from any other read. The
//! marker changes the record's action and nothing else: the route, the
//! scope it needs and the answer are those of the read without it.

use crate::query::{self, BadQuery};

/// The query parameter that marks a read as a reveal.
pub const MARKER: &str = "reveal";

/// A part of a vCon that a reveal shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The parties: names, numbers and addresses.
    Parties,
    /// The subject.
    Subject,
    /// The dialog: what was said.
    Dialog,
    /// The analysis, transcripts among it.
    Analysis,
    /// Where the vCon links to.
    Links,
    /// The whole vCon.
    All,
}

impl Part {
    /// Every part, in the order the console offers them.
    pub const ALL: [Part; 6] = [
        Part::Parties,
        Part::Subject,
        Part::Dialog,
        Part::Analysis,
        Part::Links,
        Part::All,
    ];

    /// The part's name, as the marker gives it and the log records it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Part::Parties => "parties",
            Part::Subject => "subject",
            Part::Dialog => "dialog",
            Part::Analysis => "analysis",
            Part::Links => "links",
            Part::All => "all",
        }
    }

    /// The part a read's query marks it as a reveal of, or None for a read
    /// that carries no marker.
    ///
    /// # Errors
    ///
    /// [`BadQuery`] if the query names the marker and the marker is not
    /// exactly one known part.
    pub fn from_query(query: Option<&str>) -> Result<Option<Part>, BadQuery> {
        let marked = query.unwrap_or("").split('&').any(|piece| {
            form_urlencoded::parse(piece.as_bytes())
                .next()
                .is_some_and(|(name, _)| name == MARKER)
        });
        if !marked {
            return Ok(None);
        }
        let read = query::pairs(query, &[MARKER])?;
        read.get(MARKER)
            .and_then(|value| Part::named(value))
            .map(Some)
            .ok_or(BadQuery)
    }

    /// The part with this name, if there is one.
    fn named(name: &str) -> Option<Part> {
        Part::ALL.into_iter().find(|part| part.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::Action;

    #[test]
    fn each_part_has_the_name_the_marker_and_the_log_use() {
        let named: Vec<&str> = Part::ALL.into_iter().map(Part::name).collect();
        assert_eq!(
            named,
            ["parties", "subject", "dialog", "analysis", "links", "all"],
            "the names"
        );
    }

    #[test]
    fn a_read_with_no_query_is_not_a_reveal() {
        assert_eq!(Part::from_query(None), Ok(None), "no query");
    }

    #[test]
    fn a_query_that_names_a_part_is_a_reveal_of_it() {
        for part in Part::ALL {
            let query = format!("reveal={}", part.name());
            assert_eq!(Part::from_query(Some(&query)), Ok(Some(part)), "{query}");
        }
    }

    #[test]
    fn a_marker_that_names_no_known_part_is_refused() {
        for unknown in ["everything", "party", "audio2", "*"] {
            let query = format!("reveal={unknown}");
            assert_eq!(Part::from_query(Some(&query)), Err(BadQuery), "{query}");
        }
    }

    /// A read has never looked at its query, so one that does not name
    /// the marker is the read it always was.
    #[test]
    fn a_query_that_does_not_name_the_marker_is_not_a_reveal() {
        for query in ["limit=3", "x=1&y=2", "revealed=all", "part=all"] {
            assert_eq!(Part::from_query(Some(query)), Ok(None), "{query}");
        }
    }

    /// A marked read takes the marker and nothing else, so a marker that
    /// is not first is not passed over as an unmarked read.
    #[test]
    fn a_marker_beside_another_parameter_is_refused() {
        for query in [
            "x=1&reveal=links",
            "reveal=links&x=1",
            "limit=3&reveal=all&y=2",
        ] {
            assert_eq!(Part::from_query(Some(query)), Err(BadQuery), "{query}");
        }
    }

    #[test]
    fn a_marker_given_twice_is_refused() {
        for query in ["reveal=links&reveal=links", "reveal=links&reveal=all"] {
            assert_eq!(Part::from_query(Some(query)), Err(BadQuery), "{query}");
        }
    }

    /// As in every query this server reads, an empty piece names nothing.
    #[test]
    fn an_empty_piece_beside_the_marker_names_nothing() {
        for query in ["reveal=links&", "&reveal=links", "&&reveal=links&&"] {
            assert_eq!(
                Part::from_query(Some(query)),
                Ok(Some(Part::Links)),
                "{query}"
            );
        }
    }

    /// The marker's name with no value is still the marker: it is refused,
    /// not read as an unmarked read.
    #[test]
    fn a_marker_with_no_value_is_refused() {
        for query in ["reveal", "reveal=", "reveal&x=1"] {
            assert_eq!(Part::from_query(Some(query)), Err(BadQuery), "{query}");
        }
    }

    /// The name and the value are percent-decoded as every query's are, so
    /// an encoded marker is the marker and cannot pass as an unmarked read.
    #[test]
    fn a_percent_encoded_marker_is_the_marker() {
        for query in ["%72eveal=links", "reveal=%6Cinks", "r%65veal=li%6Eks"] {
            assert_eq!(
                Part::from_query(Some(query)),
                Ok(Some(Part::Links)),
                "{query}"
            );
        }
    }

    #[test]
    fn a_reveal_is_recorded_under_an_action_that_names_its_part() {
        let named: Vec<&str> = Part::ALL
            .into_iter()
            .map(|part| Action::Reveal(part).as_str())
            .collect();
        assert_eq!(
            named,
            [
                "reveal_parties",
                "reveal_subject",
                "reveal_dialog",
                "reveal_analysis",
                "reveal_links",
                "reveal_all"
            ],
            "the names"
        );
    }

    #[test]
    fn a_legal_reveal_is_recorded_under_a_legal_action_that_names_its_part() {
        let named: Vec<&str> = Part::ALL
            .into_iter()
            .map(|part| Action::LegalReveal(part).as_str())
            .collect();
        assert_eq!(
            named,
            [
                "legal_reveal_parties",
                "legal_reveal_subject",
                "legal_reveal_dialog",
                "legal_reveal_analysis",
                "legal_reveal_links",
                "legal_reveal_all"
            ],
            "the names"
        );
    }
}
