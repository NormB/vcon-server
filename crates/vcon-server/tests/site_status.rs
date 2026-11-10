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

/// The changelog's released sections: every `## [<version>]` section after
/// `[Unreleased]`, each with its heading.
fn released_sections(changelog: &str) -> Vec<&str> {
    changelog
        .split("\n## [")
        .skip(1)
        .filter(|section| !section.starts_with("Unreleased]"))
        .collect()
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

#[test]
fn the_released_sections_reader_skips_the_preamble_and_unreleased() {
    let changelog = "# Changelog\n\nOn `main` only.\n\n## [Unreleased]\n\n- On `main` only.\n\n\
                     ## [0.4.0] - 2026-11-06\n\n- A.\n\n## [0.3.0] - 2026-11-04\n\n- B.\n";
    let sections = released_sections(changelog);
    assert_eq!(sections.len(), 2, "two released sections: {sections:#?}");
    assert!(
        sections[0].starts_with("0.4.0] - 2026-11-06"),
        "{sections:#?}"
    );
    assert!(
        sections[1].starts_with("0.3.0] - 2026-11-04"),
        "{sections:#?}"
    );
    assert!(
        sections.iter().all(|s| unreleased_phrases(s).is_empty()),
        "the preamble and [Unreleased] are not part of a released section"
    );
}

/// A released section describes what a published release holds, so it can
/// never call one of its own entries unreleased, whatever `[Unreleased]`
/// holds. Found after 0.4.0: its Parquet-storage entry still ended "On
/// `main` only.", and the page test above reads pages, not the changelog.
#[test]
fn no_released_changelog_section_calls_an_entry_unreleased() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    let sections = released_sections(&changelog);
    assert!(
        sections
            .iter()
            .any(|s| s.starts_with(&format!("{}]", env!("CARGO_PKG_VERSION")))),
        "the changelog has a section for this build's version"
    );
    let found: Vec<String> = sections
        .iter()
        .flat_map(|section| {
            let heading = section.lines().next().unwrap_or_default();
            unreleased_phrases(section)
                .into_iter()
                .map(move |phrase| format!("[{heading}: {phrase:?}"))
        })
        .collect();
    assert!(
        found.is_empty(),
        "released changelog sections call an entry unreleased: {found:#?}"
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

/// The heading of the status page's list of what is on `main` and in no
/// release. Its words are one of `UNRELEASED_PHRASES`, so the release that
/// ships the entries must take the section out
/// (`no_page_calls_a_feature_unreleased_once_the_changelog_has_none`).
const UNRELEASED_HEADING: &str = "## On `main` only";

/// The text of the changelog's `[Unreleased]` section.
fn unreleased_section(changelog: &str) -> &str {
    changelog
        .split("\n## [")
        .find(|section| section.starts_with("Unreleased]"))
        .expect("the changelog has an [Unreleased] section")
}

/// The words in backquotes in `text`, each once, in order: the routes,
/// parameters, commands and keys it names.
fn code_spans(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    for (i, part) in text.split('`').enumerate() {
        if i % 2 == 1 && !part.is_empty() && !spans.contains(&part) {
            spans.push(part);
        }
    }
    spans
}

/// The text under the status page's unreleased heading, up to the next
/// `## ` heading, or `None` when the page has no such section.
fn unreleased_on_status(status: &str) -> Option<&str> {
    let start = status.find(&format!("{UNRELEASED_HEADING}\n"))? + UNRELEASED_HEADING.len();
    let rest = &status[start..];
    Some(rest.find("\n## ").map_or(rest, |end| &rest[..end]))
}

#[test]
fn the_code_span_reader_takes_each_span_once() {
    assert_eq!(
        code_spans("Use `GET /a` or `b`; `GET /a` again, and `` nothing."),
        ["GET /a", "b"],
        "each backquoted span once, in order, and no empty one"
    );
    assert!(
        code_spans("No spans here.").is_empty(),
        "plain text has none"
    );
}

#[test]
fn the_status_section_reader_stops_at_the_next_heading() {
    let page = "## Built\n\n- A.\n\n## On `main` only\n\n- `x` is new.\n\n## Being built\n\n- B.\n";
    assert_eq!(
        unreleased_on_status(page),
        Some("\n\n- `x` is new.\n"),
        "the section's own text"
    );
    assert_eq!(
        unreleased_on_status("## Built\n\n- A.\n"),
        None,
        "a page without the heading has no such section"
    );
    assert_eq!(
        unreleased_on_status("## On `main` only\n\n- Last.\n"),
        Some("\n\n- Last.\n"),
        "a section at the end of the page runs to the end"
    );
}

/// The site is built from `main`, so a page can describe what no release
/// holds; a reader who installed the release then finds the page wrong.
/// While the changelog lists additions under `[Unreleased]`, the status
/// page says so, in one section, and names what the changelog's first words
/// name. Found on 2026-11-10: the HTTP API and audit-log pages described
/// the `reveal` parameter, added after 0.4.2, with no word that 0.4.2 does
/// not have it.
#[test]
fn the_status_page_lists_what_is_on_main_and_in_no_release() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let changelog = std::fs::read_to_string(root.join("CHANGELOG.md")).expect("read CHANGELOG.md");
    let unreleased = unreleased_section(&changelog);
    let Some(added) = unreleased
        .split("\n### ")
        .find(|part| part.starts_with("Added\n"))
    else {
        return;
    };
    let status = page("docs/status.md");
    let section = unreleased_on_status(&status).unwrap_or_else(|| {
        panic!(
            "the changelog lists unreleased additions, and docs/status.md has no section \
             {UNRELEASED_HEADING:?}"
        )
    });
    let entries: Vec<&str> = added.split("\n- ").skip(1).collect();
    assert!(!entries.is_empty(), "the Added part has entries");
    for entry in entries {
        let named = code_spans(entry);
        assert!(
            named.is_empty()
                || named
                    .iter()
                    .any(|span| section.contains(&format!("`{span}`"))),
            "docs/status.md's unreleased section names none of {named:?}, from the changelog \
             entry: {entry}"
        );
    }
}
