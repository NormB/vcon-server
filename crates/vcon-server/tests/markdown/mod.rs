//! How the documentation tests read Markdown links: one reader, shared by
//! every test that checks where a page's links lead.

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
