//! How the documentation tests read Markdown links and headings: one reader
//! of each, shared by every test that checks where a page's links lead.

use std::collections::BTreeSet;

/// Zola's id for a heading: lower case, each run of other characters than
/// letters and digits made one `-`, none at either end.
pub fn slug(heading: &str) -> String {
    let mut slug = String::new();
    for c in heading.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

/// The ids of the headings of a Markdown page, outside fenced blocks.
pub fn heading_ids(text: &str) -> BTreeSet<String> {
    let mut fenced = false;
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced && line.starts_with('#') {
            ids.insert(slug(line.trim_start_matches('#')));
        }
    }
    ids
}

/// The targets of the inline Markdown links `[text](target)` in `text`.
pub fn link_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("](") {
        rest = &rest[at + 2..];
        if let Some(end) = rest.find(')') {
            targets.push(rest[..end].to_string());
            rest = &rest[end..];
        }
    }
    targets
}

/// Whether a link target is a path in the repository, not a URL or an anchor.
pub fn is_relative(target: &str) -> bool {
    !(target.is_empty()
        || target.starts_with('#')
        || target.starts_with('/')
        || target.contains("://")
        || target.starts_with("mailto:"))
}
