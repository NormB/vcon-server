//! Reading website/content/docs/audit-log-records.md, in one place:
//! tests/docs_pages.rs holds the page's actions to the code, and
//! tests/audit_contract.rs writes contract/audit.json from its members and
//! actions. Both bring this file in by `#[path]`.

#![allow(
    dead_code,
    reason = "each test that brings this file in uses a part of it"
)]

use std::collections::BTreeSet;

/// The text of the section `## heading` of `page`, up to the next `## `
/// heading. None when the page has no such section.
pub fn section<'a>(page: &'a str, heading: &str) -> Option<&'a str> {
    let start = format!("\n## {heading}\n");
    let at = page.find(&start)?;
    let body = &page[at + start.len()..];
    Some(body.find("\n## ").map_or(body, |end| &body[..end]))
}

/// The first cell of each table row of the section `## heading`, in
/// backticks, in the page's order: empty when the section is not there.
pub fn names(page: &str, heading: &str) -> Vec<String> {
    section(page, heading)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once('`'))
        .map(|(name, _)| name.to_owned())
        .collect()
}

/// The actions the page's table names: its "Every action" section.
pub fn actions(page: &str) -> BTreeSet<String> {
    names(page, "Every action").into_iter().collect()
}

/// The members of a record the page's table names: its "A record" section.
pub fn members(page: &str) -> Vec<String> {
    names(page, "A record")
}
