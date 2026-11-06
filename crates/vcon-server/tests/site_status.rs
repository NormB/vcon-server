//! The site's status statements name this build's version, so a version
//! bump cannot leave them describing an older release.

use std::path::PathBuf;

fn page(path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/content")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_status_page_names_this_release() {
    let version = env!("CARGO_PKG_VERSION");
    let text = page("docs/status.md");
    let says = format!("vcon-server {version} is released");
    assert!(text.contains(&says), "docs/status.md does not say: {says}");
}

#[test]
fn the_home_page_status_note_names_this_release() {
    let version = env!("CARGO_PKG_VERSION");
    let text = page("_index.md");
    let note = text
        .lines()
        .find(|l| l.starts_with("> **Status:"))
        .expect("a status note");
    let says = format!("> **Status: {version} released");
    assert!(note.starts_with(&says), "the note is: {note}");
}

/// Words that say a feature is built but not released yet. They are true
/// only while the changelog's `[Unreleased]` section has entries; the
/// release that ships those entries must turn each into released wording.
const UNRELEASED_PHRASES: [&str; 7] = [
    "in no release",
    "on `main` only",
    "on `main`,",
    "from `main`",
    "for the next release",
    "until the next release",
    "before the next release",
];

/// The phrases of `UNRELEASED_PHRASES` that `text` holds, in any case and
/// with lines joined, since a phrase may break across lines of a paragraph.
fn unreleased_phrases(text: &str) -> Vec<&'static str> {
    let joined = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    UNRELEASED_PHRASES
        .iter()
        .copied()
        .filter(|phrase| joined.contains(phrase))
        .collect()
}

/// Whether the changelog's `[Unreleased]` section lists any change.
fn has_unreleased_entries(changelog: &str) -> bool {
    changelog
        .split("\n## [")
        .find(|section| section.starts_with("Unreleased]"))
        .expect("the changelog has an [Unreleased] section")
        .lines()
        .any(|line| line.starts_with("- "))
}

/// Every Markdown file under `dir`.
fn markdown_under(dir: &std::path::Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            markdown_under(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.push(path);
        }
    }
}

#[test]
fn the_unreleased_reader_finds_a_phrase_across_lines() {
    assert_eq!(
        unreleased_phrases("This is on `main` and in no\n  release yet."),
        ["in no release"],
        "a phrase broken across lines is found"
    );
    assert_eq!(
        unreleased_phrases("Parquet storage is on `main` only."),
        ["on `main` only"],
        "the main-only phrase is found"
    );
    assert_eq!(
        unreleased_phrases("On\n  `main`, the objects can go to a bucket. Build it from `main`."),
        ["on `main`,", "from `main`"],
        "a sentence that starts with the phrase, and a build from main, are found"
    );
    assert!(
        unreleased_phrases(
            "Released in 0.4.0. Fixes ship in the next release. Every commit on `main` is signed."
        )
        .is_empty(),
        "released wording, the security policy's promise and the signing rule are not reported"
    );
}

#[test]
fn the_changelog_reader_sees_only_the_unreleased_section() {
    let empty = "# Changelog\n\n## [Unreleased]\n\n## [0.4.0] - 2026-11-06\n\n### Added\n\n- A.\n";
    assert!(
        !has_unreleased_entries(empty),
        "an entry under a released version is not an unreleased one"
    );
    let open = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- B.\n\n## [0.4.0] - 2026-11-06\n";
    assert!(
        has_unreleased_entries(open),
        "an entry under [Unreleased] counts"
    );
}

/// A release moves every `[Unreleased]` entry under its version, and with
/// them every page that said "in no release yet" must say what the release
/// holds. Found on 0.4.0: two READMEs and 18 site pages still said that S3
/// and Parquet storage were on `main` only, and no test read them.
#[test]
fn no_page_calls_a_feature_unreleased_once_the_changelog_has_none() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    if has_unreleased_entries(&changelog) {
        return;
    }
    let mut pages = vec![
        root.join("README.md"),
        root.join("crates/vcon-server/README.md"),
        root.join("crates/vcon/README.md"),
        root.join("website/static/llms.txt"),
    ];
    markdown_under(&root.join("website/content"), &mut pages);
    assert!(
        pages.iter().any(|p| p.ends_with("docs/s3-storage.md")),
        "the walk reached the docs pages"
    );
    let mut found = Vec::new();
    for path in &pages {
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for phrase in unreleased_phrases(&text) {
            found.push(format!("{}: {phrase:?}", path.display()));
        }
    }
    assert!(
        found.is_empty(),
        "the changelog lists nothing unreleased, yet these pages say a feature is: {found:#?}"
    );
}
