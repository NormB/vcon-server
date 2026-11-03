//! `docs/README.md` is the documentation index for people browsing the
//! repository. It links every page of the site's documentation,
//! `website/content/docs/*.md`, so it cannot fall behind when a page is
//! added, and every relative link it holds must reach a file in the tree.
//! The OpenSSF Best Practices site also reads it: it looks for a top-level
//! `doc`, `docs` or `documentation` folder holding a file of at most
//! 100,000 bytes that mentions installing, using and security.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The repository root, two levels above this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The Best Practices site skips a larger file.
const BEST_PRACTICES_MAX_SIZE: usize = 100_000;

fn index() -> String {
    let path = root().join("docs/README.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The targets of the inline Markdown links `[text](target)` in `text`.
fn link_targets(text: &str) -> Vec<String> {
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
fn is_relative(target: &str) -> bool {
    !(target.is_empty()
        || target.starts_with('#')
        || target.starts_with('/')
        || target.contains("://")
        || target.starts_with("mailto:"))
}

#[test]
fn the_link_reader_finds_relative_links_only() {
    let text = "[a](../x.md) [b](https://e.example/) [c](#here) [d](y.md#part)";
    let relative: Vec<String> = link_targets(text)
        .into_iter()
        .filter(|t| is_relative(t))
        .collect();
    assert_eq!(
        relative,
        ["../x.md", "y.md#part"],
        "only paths in the tree are relative links"
    );
}

#[test]
fn the_index_links_every_documentation_page() {
    let dir = root().join("website/content/docs");
    let pages: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("list {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".md"))
        .collect();
    assert!(!pages.is_empty(), "no pages in {}", dir.display());
    let linked: BTreeSet<String> = link_targets(&index())
        .into_iter()
        .filter_map(|t| {
            t.strip_prefix("../website/content/docs/")
                .map(str::to_string)
        })
        .map(|t| t.split('#').next().unwrap_or_default().to_string())
        .collect();
    let missing: Vec<&String> = pages.difference(&linked).collect();
    assert!(
        missing.is_empty(),
        "docs/README.md does not link {missing:?}"
    );
}

#[test]
fn every_relative_link_in_the_index_exists() {
    let base = root().join("docs");
    let broken: Vec<String> = link_targets(&index())
        .into_iter()
        .filter(|t| is_relative(t))
        .filter(|t| !base.join(t.split('#').next().unwrap_or_default()).exists())
        .collect();
    assert!(
        broken.is_empty(),
        "docs/README.md links what does not exist: {broken:?}"
    );
}

#[test]
fn the_index_is_a_file_the_best_practices_site_reads() {
    let text = index();
    assert!(
        text.len() <= BEST_PRACTICES_MAX_SIZE,
        "docs/README.md has {} bytes; the site reads at most {BEST_PRACTICES_MAX_SIZE}",
        text.len()
    );
    let lower = text.to_lowercase();
    // The site's patterns: install(ation)?, us(e|ing), secur(e|ity).
    for words in [&["install"][..], &["use", "using"], &["secure", "security"]] {
        assert!(
            words.iter().any(|w| lower.contains(w)),
            "docs/README.md never says any of {words:?}"
        );
    }
}
