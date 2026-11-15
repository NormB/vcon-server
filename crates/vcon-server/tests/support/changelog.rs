//! Reading CHANGELOG.md in a test, in one place: tests/release_features.rs
//! and the schema tests of the PostgreSQL catalog include this file.
//!
//! A test that asks "does the changelog say X" about a rule that holds
//! since some release must read every section from the top through that
//! release's. Three tests read the unreleased section alone; the 0.4.6
//! release moved its entries under `## [0.4.6]` and left that section
//! empty, and from then on the three checked nothing.

/// The start of a section's heading line: `## [Unreleased]`,
/// `## [0.4.6] - 2026-11-14`.
const HEADING: &str = "## [";

/// The changelog from its first section heading through the section of
/// `version` (a release as `0.4.6`, or `Unreleased`): that section and
/// every newer one, the unreleased section among them. The lines before
/// the first heading and the sections of older releases are left out.
///
/// `None` when the changelog has no section of `version`. A caller must
/// fail on it: a text that was not read holds no fault, and a test that
/// passed on it would check nothing.
pub(crate) fn through<'a>(changelog: &'a str, version: &str) -> Option<&'a str> {
    let wanted = format!("{HEADING}{version}]");
    let mut first = None;
    let mut found = false;
    let mut at = 0_usize;
    for line in changelog.split_inclusive('\n') {
        if line.starts_with(HEADING) {
            if found {
                // The heading after the wanted section ends what is read.
                return changelog.get(first?..at);
            }
            first.get_or_insert(at);
            found = line.starts_with(&wanted);
        }
        at = at.saturating_add(line.len());
    }
    if found { changelog.get(first?..) } else { None }
}
