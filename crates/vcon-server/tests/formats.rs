//! The on-disk formats: the page that lists each one and its version, and
//! the data each released binary wrote, opened by this build.
//!
//! Each directory under tests/data/formats/<release>/ was written by that
//! release's own binary (scripts/make-format-fixtures.py, which takes the
//! binary from the release archive pinned in scripts/docs-test-pins.txt):
//! the data directory as the release left it, the release's test master key
//! and manifest.json, which names every version kept with the SHA-256 of
//! its body and every uuid purged. Every test works on a copy.
//!
//! The rule the page states, held here for every release: this build opens
//! the data (migrating it on the first open), `check` finds nothing wrong,
//! every kept body reads back byte-equal, a second open changes no file, a
//! catalog one version past this build is refused by name, and a migration
//! that fails leaves the release's version whole.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use aws_lc_rs::digest::{SHA256, digest};
use aws_lc_rs::hkdf::{HKDF_SHA256, Salt};

use vcon_server::audit::Actor;
use vcon_server::catalog::SCHEMA_VERSION;
use vcon_server::clock::SystemClock;
use vcon_server::envelope::Key;
use vcon_server::formats::FORMATS;
use vcon_server::fs::OsFs;
use vcon_server::ids::{KeyId, TenantId, VersionId};
use vcon_server::store::{Caller, StoreError};
use vcon_server::tenants::Tenants;

/// The repository's root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The page that lists the formats.
const PAGE: &str = "website/content/docs/on-disk-format.md";

/// The directory of the fixtures.
fn formats_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/formats")
}

fn page() -> String {
    std::fs::read_to_string(root().join(PAGE)).expect("the on-disk format page is there")
}

// ---- The page's tables. ----

#[path = "support/page_table.rs"]
mod page_table;

use page_table::table;

/// The text of a cell without the backticks around it.
fn bare(cell: &str) -> &str {
    cell.trim_matches('`')
}

/// The formats table: id to the version it states.
fn page_formats(text: &str) -> Result<BTreeMap<String, String>, String> {
    let rows = table(text, "## The formats").ok_or("no table under \"## The formats\"")?;
    let header = &rows[0];
    let at = |name: &str| {
        header
            .iter()
            .position(|h| h == name)
            .ok_or(format!("the formats table has no column {name}"))
    };
    let (id, version) = (at("Format")?, at("Version")?);
    let mut out = BTreeMap::new();
    for row in &rows[1..] {
        let key = bare(&row[id]).to_owned();
        if out
            .insert(key.clone(), bare(&row[version]).to_owned())
            .is_some()
        {
            return Err(format!("{key} is listed twice"));
        }
    }
    Ok(out)
}

/// The release table: release to (format id to the version it wrote).
fn page_releases(text: &str) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let rows = table(text, "## What each release wrote")
        .ok_or("no table under \"## What each release wrote\"")?;
    let header: Vec<&str> = rows[0].iter().map(|h| bare(h)).collect();
    if header.first() != Some(&"Release") {
        return Err(format!(
            "the release table starts with {:?}",
            header.first()
        ));
    }
    let mut out = BTreeMap::new();
    for row in &rows[1..] {
        if row.len() != header.len() {
            return Err(format!("the row of {} has {} cells", row[0], row.len()));
        }
        let wrote = header[1..]
            .iter()
            .zip(&row[1..])
            .map(|(h, c)| ((*h).to_owned(), bare(c).to_owned()))
            .collect();
        if out.insert(row[0].clone(), wrote).is_some() {
            return Err(format!("{} is listed twice", row[0]));
        }
    }
    Ok(out)
}

const SAMPLE: &str = "\
intro

## The formats

| Format | What | Version |
|---|---|---|
| `catalog` | the catalogs | 9 |
| `envelope` | each sealed blob | 1 |

## What each release wrote

| Release | `catalog` | `envelope` |
|---|---|---|
| 0.4.0 | 4 | 1 |
| 0.4.8 | 9 | 1 |

## Next
";

#[test]
fn the_table_reader_reads_the_formats_of_a_page() {
    let read = page_formats(SAMPLE).expect("the sample reads");
    let want: BTreeMap<String, String> = [("catalog", "9"), ("envelope", "1")]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .into();
    assert_eq!(read, want, "both rows, without backticks");
}

#[test]
fn the_table_reader_reads_the_releases_of_a_page() {
    let read = page_releases(SAMPLE).expect("the sample reads");
    assert_eq!(read.len(), 2, "two releases: {read:?}");
    assert_eq!(read["0.4.0"]["catalog"], "4", "0.4.0 wrote catalog 4");
    assert_eq!(read["0.4.8"]["envelope"], "1", "0.4.8 wrote envelope 1");
}

#[test]
fn a_page_without_the_formats_table_is_refused() {
    let why = page_formats("## Other\n\n| a | b |\n|---|---|\n| 1 | 2 |\n").expect_err("no table");
    assert!(why.contains("The formats"), "names the heading: {why}");
}

#[test]
fn a_format_listed_twice_is_refused() {
    let text = SAMPLE.replace(
        "| `envelope` | each sealed blob | 1 |",
        "| `catalog` | again | 9 |",
    );
    let why = page_formats(&text).expect_err("twice");
    assert!(why.contains("catalog is listed twice"), "names it: {why}");
}

#[test]
fn a_release_listed_twice_is_refused() {
    let text = SAMPLE.replace("| 0.4.8 | 9 | 1 |", "| 0.4.0 | 9 | 1 |");
    let why = page_releases(&text).expect_err("twice");
    assert!(why.contains("0.4.0 is listed twice"), "names it: {why}");
}

#[test]
fn a_release_row_with_a_cell_missing_is_refused() {
    let text = SAMPLE.replace("| 0.4.8 | 9 | 1 |", "| 0.4.8 | 9 |");
    let why = page_releases(&text).expect_err("short row");
    assert!(why.contains("0.4.8 has 2 cells"), "names the row: {why}");
}

#[test]
fn a_formats_table_without_a_version_column_is_refused() {
    let text = SAMPLE.replace("| Format | What | Version |", "| Format | What | Number |");
    let why = page_formats(&text).expect_err("no column");
    assert!(why.contains("no column Version"), "names it: {why}");
}

#[test]
fn the_table_reader_stops_at_the_next_heading() {
    let text = "## The formats\n\n## Next\n\n| Format | Version |\n|---|---|\n| `x` | 1 |\n";
    assert!(
        page_formats(text).is_err(),
        "a table under the next heading is not this one"
    );
}

#[test]
fn every_format_of_this_build_is_on_the_page_with_its_version() {
    let listed = page_formats(&page()).expect("the page's formats table");
    for format in FORMATS {
        assert_eq!(
            listed.get(format.id).map(String::as_str),
            Some(format.version),
            "{PAGE} states format {} at the version this build writes",
            format.id
        );
    }
}

#[test]
fn every_format_on_the_page_is_one_this_build_has() {
    let listed = page_formats(&page()).expect("the page's formats table");
    let ids: Vec<&str> = FORMATS.iter().map(|f| f.id).collect();
    for id in listed.keys() {
        assert!(
            ids.contains(&id.as_str()),
            "{PAGE} lists {id}, which formats::FORMATS does not"
        );
    }
}

#[test]
fn the_formats_of_this_build_have_distinct_ids() {
    let mut ids: Vec<&str> = FORMATS.iter().map(|f| f.id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), before, "one entry per format");
    assert!(before >= 6, "every format there is: {before}");
}

#[test]
fn the_release_table_has_a_column_for_every_format() {
    let releases = page_releases(&page()).expect("the page's release table");
    for (release, wrote) in &releases {
        let columns: Vec<&str> = wrote.keys().map(String::as_str).collect();
        let mut ids: Vec<&str> = FORMATS.iter().map(|f| f.id).collect();
        ids.sort_unstable();
        assert_eq!(columns, ids, "the row of {release} has one cell per format");
    }
}

#[test]
fn the_release_table_lists_exactly_the_releases_with_a_fixture() {
    let releases = page_releases(&page()).expect("the page's release table");
    // The map's keys are in text order ("0.4.10" before "0.4.2"); the
    // fixtures are in release order.
    let mut listed: Vec<&String> = releases.keys().collect();
    listed.sort_by_key(|r| release_order(r));
    let fixtures = fixture_releases();
    assert_eq!(
        listed,
        fixtures.iter().collect::<Vec<_>>(),
        "one row per directory under tests/data/formats"
    );
}

#[test]
fn every_released_binary_since_0_4_0_has_a_fixture() {
    // The pins name each release's archives; each release from 0.4.0 on
    // must have its data here.
    let pins = std::fs::read_to_string(root().join("scripts/docs-test-pins.txt")).expect("pins");
    let mut pinned: Vec<String> = pins
        .lines()
        .filter_map(|l| l.split("/releases/download/v").nth(1))
        .filter_map(|rest| rest.split('/').next())
        .filter(|r| r.starts_with("0.4."))
        .map(str::to_owned)
        .collect();
    pinned.sort_by_key(|r| release_order(r));
    pinned.dedup();
    assert_eq!(
        pinned,
        fixture_releases(),
        "a fixture for every pinned 0.4 release"
    );
}

/// The text of CHANGELOG.md's `[Unreleased]` section, without its heading,
/// up to the next release heading; None without that heading.
fn unreleased(changelog: &str) -> Option<&str> {
    section(changelog, "Unreleased")
}

/// The text of the section headed `## [<name>]` at the start of a line,
/// without its heading line, up to the next release heading; None without
/// that heading. `[0.4.15]` is not `[0.4.150]`: the bracket closes the name.
fn section<'a>(changelog: &'a str, name: &str) -> Option<&'a str> {
    let heading = format!("## [{name}]");
    let at = changelog
        .match_indices(&heading)
        .map(|(at, _)| at)
        .find(|&at| at == 0 || changelog[..at].ends_with('\n'))?;
    let rest = &changelog[at + heading.len()..];
    Some(rest.find("\n## [").map_or(rest, |end| &rest[..end]))
}

/// Whether `text` holds `version <v>` with no character of a longer
/// version after it.
fn names_version(text: &str, v: &str) -> bool {
    let needle = format!("version {v}");
    text.match_indices(&needle).any(|(at, _)| {
        text[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '/'))
    })
}

/// What the page's rule asks of a format this build writes at another
/// version than `newest` (the row of the newest release in the table,
/// `release`): the build's version is not below the release's, and a
/// CHANGELOG entry names it as `version <the build's>`. The entry counts
/// under `[Unreleased]`, or under the section of `own`, this build's crate
/// version, when `own` is newer than `release`: a release commit moves the
/// entries there, and the table gains that release's row only after it is
/// published. No other section counts, so a stray or misnumbered heading
/// does not. `build` is each format's id and this build's version; the
/// result names each format that breaks it.
fn format_changes(
    build: &[(&str, &str)],
    release: &str,
    own: &str,
    newest: &BTreeMap<String, String>,
    changelog: &str,
) -> Vec<String> {
    let pending = unreleased(changelog).unwrap_or("");
    let own_newer = release_order(own) > release_order(release);
    let own_section = if own_newer {
        section(changelog, own).unwrap_or("")
    } else {
        ""
    };
    let places = if own_newer {
        format!("[Unreleased] or [{own}]")
    } else {
        "[Unreleased]".to_owned()
    };
    let mut out = Vec::new();
    for &(id, version) in build {
        let released = newest.get(id).map(String::as_str);
        if released == Some(version) {
            continue;
        }
        let below = matches!(
            (released.and_then(|r| r.parse::<u64>().ok()), version.parse::<u64>().ok()),
            (Some(r), Some(b)) if b < r
        );
        if below {
            out.push(format!(
                "{id}: this build's version {version} is below the release's {}",
                released.unwrap_or_default()
            ));
        } else if !names_version(pending, version) && !names_version(own_section, version) {
            out.push(format!(
                "{id}: this build writes version {version}, the release {release} {}; no entry under {places} names `version {version}`",
                released.unwrap_or("no version")
            ));
        }
    }
    out
}

/// One release row of `pairs`.
fn row(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

const LOG_NAMING_14: &str = "# Changelog\n\n## [Unreleased]\n\n### Changed\n\n- **A PostgreSQL catalog moves to schema version 14.**\n\n## [0.4.13] - 2026-11-21\n\n- **Old.** version 13\n";
const LOG_SILENT: &str =
    "# Changelog\n\n## [Unreleased]\n\n## [0.4.13] - 2026-11-21\n\n- **Old.** version 14\n";

// Success: what the rule lets through.

#[test]
fn a_build_at_the_newest_releases_versions_needs_no_entry() {
    let got = format_changes(
        &[("catalog", "9"), ("postgres", "13")],
        "0.4.13",
        "0.4.14",
        &row(&[("catalog", "9"), ("postgres", "13")]),
        LOG_SILENT,
    );
    assert!(got.is_empty(), "nothing changed: {got:?}");
}

#[test]
fn a_raised_version_the_unreleased_section_names_passes() {
    let got = format_changes(
        &[("catalog", "9"), ("postgres", "14")],
        "0.4.13",
        "0.4.14",
        &row(&[("catalog", "9"), ("postgres", "13")]),
        LOG_NAMING_14,
    );
    assert!(got.is_empty(), "the entry names version 14: {got:?}");
}

#[test]
fn a_format_new_since_the_release_passes_when_named() {
    let got = format_changes(
        &[("postgres", "14")],
        "0.4.13",
        "0.4.14",
        &row(&[("postgres", "none")]),
        LOG_NAMING_14,
    );
    assert!(got.is_empty(), "none to 14, named: {got:?}");
}

#[test]
fn the_unreleased_reader_takes_the_section_up_to_the_next_release() {
    let got = unreleased(LOG_NAMING_14).expect("the section is there");
    assert!(got.contains("version 14"), "the entry is in it: {got:?}");
    assert!(
        !got.contains("version 13"),
        "the release's entry is not: {got:?}"
    );
    assert!(!got.contains("[Unreleased]"), "the heading is not: {got:?}");
}

// Failure: what the rule refuses.

#[test]
fn a_raised_version_no_entry_names_is_refused() {
    let got = format_changes(
        &[("catalog", "9"), ("postgres", "14")],
        "0.4.13",
        "0.4.14",
        &row(&[("catalog", "9"), ("postgres", "13")]),
        LOG_SILENT,
    );
    assert_eq!(got.len(), 1, "one format breaks the rule: {got:?}");
    assert!(got[0].contains("postgres"), "names the format: {got:?}");
    assert!(got[0].contains("14"), "names the build's version: {got:?}");
}

#[test]
fn a_version_named_only_under_a_release_is_refused() {
    // LOG_SILENT names version 14 under 0.4.13, not under [Unreleased].
    let got = format_changes(
        &[("postgres", "14")],
        "0.4.13",
        "0.4.14",
        &row(&[("postgres", "13")]),
        LOG_SILENT,
    );
    assert_eq!(got.len(), 1, "a released section does not count: {got:?}");
}

#[test]
fn a_version_below_the_releases_is_refused_even_when_named() {
    let log = "## [Unreleased]\n\n- **Back.** schema version 12\n\n## [0.4.13]\n";
    let got = format_changes(
        &[("postgres", "12")],
        "0.4.13",
        "0.4.14",
        &row(&[("postgres", "13")]),
        log,
    );
    assert_eq!(got.len(), 1, "a build never goes below a release: {got:?}");
    assert!(got[0].contains("below"), "says why: {got:?}");
}

#[test]
fn a_changelog_without_an_unreleased_section_refuses_a_change() {
    let log = "# Changelog\n\n## [0.4.13] - 2026-11-21\n\n- **A.** version 14\n";
    assert_eq!(unreleased(log), None, "no [Unreleased] heading");
    let got = format_changes(
        &[("postgres", "14")],
        "0.4.13",
        "0.4.14",
        &row(&[("postgres", "13")]),
        log,
    );
    assert_eq!(got.len(), 1, "no section names it: {got:?}");
}

// Edge: the boundaries of the rule.

#[test]
fn a_longer_number_does_not_name_the_version() {
    let log = "## [Unreleased]\n\n- **A.** schema version 140\n\n## [0.4.13]\n";
    let got = format_changes(
        &[("postgres", "14")],
        "0.4.13",
        "0.4.14",
        &row(&[("postgres", "13")]),
        log,
    );
    assert_eq!(got.len(), 1, "version 140 is not version 14: {got:?}");
}

#[test]
fn a_version_at_the_end_of_the_section_is_named() {
    let log = "## [Unreleased]\n\n- **A.** schema version 14";
    let got = format_changes(
        &[("postgres", "14")],
        "0.4.13",
        "0.4.14",
        &row(&[("postgres", "13")]),
        log,
    );
    assert!(got.is_empty(), "the last word of the file: {got:?}");
}

#[test]
fn a_version_text_that_is_not_a_number_is_compared_as_text() {
    let log =
        "## [Unreleased]\n\n- **A.** format version vcon-server.parquet-storage/2\n\n## [0.4.13]\n";
    let build = [("parquet-storage", "vcon-server.parquet-storage/2")];
    let named = format_changes(
        &build,
        "0.4.13",
        "0.4.14",
        &row(&[("parquet-storage", "vcon-server.parquet-storage/1")]),
        log,
    );
    assert!(
        named.is_empty(),
        "named, and not below by number: {named:?}"
    );
    let silent = format_changes(
        &build,
        "0.4.13",
        "0.4.14",
        &row(&[("parquet-storage", "vcon-server.parquet-storage/1")]),
        LOG_SILENT,
    );
    assert_eq!(silent.len(), 1, "changed and not named: {silent:?}");
}

#[test]
fn a_format_the_release_row_lacks_is_a_change() {
    let got = format_changes(
        &[("postgres", "14")],
        "0.4.13",
        "0.4.14",
        &row(&[]),
        LOG_SILENT,
    );
    assert_eq!(got.len(), 1, "no cell is a change, not a pass: {got:?}");
}

// A release commit moves the [Unreleased] entries to the section of the
// build's own version, while the table's newest row stays the previous
// release's until that release's data is added. The entry then counts in
// that one section, and only when the build's version is newer than the
// row's release.

/// A CHANGELOG after a release commit for 0.4.15: the schema entry under
/// [0.4.15], none under [Unreleased] or [0.4.14].
const LOG_RELEASED_15: &str = "# Changelog\n\n## [Unreleased]\n\n## [0.4.15] - 2026-11-24\n\n### Changed\n\n- **A PostgreSQL catalog moves to schema version 14.**\n\n## [0.4.14] - 2026-11-24\n\n- **Old.** schema version 13\n";

/// The 0.4.14 row: PostgreSQL schema 13.
fn row_14() -> BTreeMap<String, String> {
    row(&[("catalog", "9"), ("postgres", "13")])
}

const BUILD_14: &[(&str, &str)] = &[("catalog", "9"), ("postgres", "14")];

// Success.

#[test]
fn an_entry_in_the_builds_own_section_passes() {
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), LOG_RELEASED_15);
    assert!(
        got.is_empty(),
        "the entry is under [0.4.15], the build's own version: {got:?}"
    );
}

#[test]
fn an_entry_under_unreleased_passes_beside_an_empty_own_section() {
    let log = "## [Unreleased]\n\n- **A.** schema version 14\n\n## [0.4.15] - 2026-11-24\n\n## [0.4.14]\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert!(got.is_empty(), "[Unreleased] still counts: {got:?}");
}

#[test]
fn after_the_pins_the_new_row_needs_no_entry() {
    // The pins follow-up adds the 0.4.15 row at schema 14; the entry stays
    // under [0.4.15].
    let row_15 = row(&[("catalog", "9"), ("postgres", "14")]);
    let got = format_changes(BUILD_14, "0.4.15", "0.4.15", &row_15, LOG_RELEASED_15);
    assert!(
        got.is_empty(),
        "the build is at the new row's versions: {got:?}"
    );
}

#[test]
fn an_own_section_heading_with_no_date_counts() {
    let log = "## [Unreleased]\n\n## [0.4.15]\n\n- **A.** schema version 14\n\n## [0.4.14]\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert!(got.is_empty(), "a heading without a date: {got:?}");
}

#[test]
fn an_own_section_at_the_end_of_the_file_counts() {
    let log = "## [Unreleased]\n\n## [0.4.15]\n\n- **A.** schema version 14";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert!(got.is_empty(), "the last section of the file: {got:?}");
}

// Failure.

#[test]
fn an_entry_in_a_later_versions_section_is_refused() {
    let log = "## [Unreleased]\n\n## [0.4.16] - 2026-12-01\n\n- **A.** schema version 14\n\n## [0.4.14]\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert_eq!(got.len(), 1, "[0.4.16] is not the build's 0.4.15: {got:?}");
}

#[test]
fn an_entry_in_the_newest_rows_own_section_is_refused() {
    let log = "## [Unreleased]\n\n## [0.4.15]\n\n## [0.4.14] - 2026-11-24\n\n- **A.** schema version 14\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert_eq!(got.len(), 1, "0.4.14 is released at 13: {got:?}");
}

#[test]
fn an_own_version_equal_to_the_newest_row_does_not_open_its_section() {
    let log = "## [Unreleased]\n\n## [0.4.14] - 2026-11-24\n\n- **A.** schema version 14\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.14", &row_14(), log);
    assert_eq!(
        got.len(),
        1,
        "the build's version is the row's, not newer: {got:?}"
    );
}

#[test]
fn an_own_version_older_than_the_newest_row_does_not_open_its_section() {
    let log = "## [Unreleased]\n\n## [0.4.14]\n\n## [0.4.13] - 2026-11-21\n\n- **A.** schema version 14\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.13", &row_14(), log);
    assert_eq!(got.len(), 1, "0.4.13 is older than the row: {got:?}");
}

#[test]
fn the_refusal_names_both_sections_it_read() {
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), LOG_SILENT);
    assert_eq!(got.len(), 1, "one format breaks the rule: {got:?}");
    assert!(
        got[0].contains("[Unreleased] or [0.4.15]"),
        "names where an entry may be: {got:?}"
    );
}

// Edge.

#[test]
fn a_longer_version_heading_is_not_the_own_section() {
    let log = "## [Unreleased]\n\n## [0.4.150]\n\n- **A.** schema version 14\n\n## [0.4.14]\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert_eq!(got.len(), 1, "[0.4.150] is not [0.4.15]: {got:?}");
}

#[test]
fn a_heading_inside_a_line_opens_no_section() {
    let log = "## [Unreleased]\n\n## [0.4.14]\n\n- **Old.** see ## [0.4.15] schema version 14\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert_eq!(got.len(), 1, "a heading starts a line: {got:?}");
}

#[test]
fn the_own_section_ends_at_the_next_heading() {
    // The entry is under [0.4.14], after an empty [0.4.15].
    let log = "## [0.4.15]\n\n### Changed\n\n## [0.4.14]\n\n- **A.** schema version 14\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert_eq!(got.len(), 1, "the next release's text is not read: {got:?}");
}

#[test]
fn the_own_section_counts_with_no_unreleased_heading() {
    let log =
        "# Changelog\n\n## [0.4.15] - 2026-11-24\n\n- **A.** schema version 14\n\n## [0.4.14]\n";
    let got = format_changes(BUILD_14, "0.4.14", "0.4.15", &row_14(), log);
    assert!(got.is_empty(), "[0.4.15] alone: {got:?}");
}

#[test]
fn a_patch_number_compares_as_a_number_not_as_text() {
    // 0.4.9 < 0.4.10 as numbers; as text "0.4.9" > "0.4.10".
    let log = "## [Unreleased]\n\n## [0.4.10]\n\n- **A.** schema version 14\n\n## [0.4.9]\n";
    let got = format_changes(BUILD_14, "0.4.9", "0.4.10", &row_14(), log);
    assert!(got.is_empty(), "0.4.10 is newer than 0.4.9: {got:?}");
}

#[test]
fn this_build_keeps_the_rule_against_the_newest_release_row() {
    // A change to a format raises its version, and the same change adds a
    // CHANGELOG entry (the page's rule); the release table gains the new
    // version with the next release's row.
    let releases = page_releases(&page()).expect("the page's release table");
    let newest = fixture_releases().pop().expect("a release");
    let changelog =
        std::fs::read_to_string(root().join("CHANGELOG.md")).expect("CHANGELOG.md is there");
    let build: Vec<(&str, &str)> = FORMATS.iter().map(|f| (f.id, f.version)).collect();
    let problems = format_changes(
        &build,
        &newest,
        env!("CARGO_PKG_VERSION"),
        &releases[&newest],
        &changelog,
    );
    assert!(problems.is_empty(), "against {newest}: {problems:?}");
}

// ---- The contract snapshot, contract/formats.json. ----
//
// The on-disk formats' part of the contract (website/content/docs/stability.md):
// each format of this build with its version and whether the server reads
// it back, and the versions each release with a fixture wrote, from the
// page above. `scripts/contract_diff.py --kind formats` compares two of
// these files. The snapshot is written, never edited by hand.

mod contract_snapshot {
    include!("support/contract_snapshot.rs");
}

/// The variable that makes the snapshot test write the snapshot.
const WRITE: &str = "VCON_SERVER_WRITE_CONTRACT";

/// The one command that writes contract/formats.json again.
const REGENERATE: &str =
    "VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --test formats contract_snapshot";

/// The snapshot, from the repository root.
const SNAPSHOT: &str = "contract/formats.json";

/// The words the page may use for how many formats the server reads back.
const COUNTS: [&str; 7] = ["none", "one", "two", "three", "four", "five", "six"];

/// How many of the formats table's first rows the server reads back, as
/// the page's sentence "The server reads back the first <n>." states it.
fn read_back_count(text: &str) -> Result<usize, String> {
    let rest = text
        .split("The server reads back the first ")
        .nth(1)
        .ok_or("the page does not say which formats the server reads back")?;
    let word = rest
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default();
    COUNTS
        .iter()
        .position(|n| *n == word)
        .ok_or(format!("{word:?} is not a count this test reads"))
}

/// The format ids of the page's formats table, in its order.
fn page_order(text: &str) -> Result<Vec<String>, String> {
    let rows = table(text, "## The formats").ok_or("no table under \"## The formats\"")?;
    let id = rows[0]
        .iter()
        .position(|h| h == "Format")
        .ok_or("the formats table has no column Format")?;
    Ok(rows[1..].iter().map(|r| bare(&r[id]).to_owned()).collect())
}

/// The snapshot of the formats `versions` (id to the version this build
/// writes) and the page `text`, as contract_diff.py reads it.
fn formats_snapshot(versions: &BTreeMap<String, String>, text: &str) -> Result<String, String> {
    let order = page_order(text)?;
    let read_back = read_back_count(text)?;
    if read_back > order.len() {
        return Err(format!(
            "the page says the server reads back {read_back} of {} formats",
            order.len()
        ));
    }
    let mut formats = BTreeMap::new();
    for (i, id) in order.iter().enumerate() {
        let version = versions
            .get(id)
            .ok_or(format!("{id} is on the page but not in this build"))?;
        formats.insert(
            id.clone(),
            serde_json::json!({ "read_back": i < read_back, "version": version }),
        );
    }
    let releases = page_releases(text)?;
    let snapshot = serde_json::json!({ "formats": formats, "releases": releases });
    let mut out = serde_json::to_string_pretty(&snapshot).expect("JSON");
    out.push('\n');
    Ok(out)
}

fn build_versions() -> BTreeMap<String, String> {
    FORMATS
        .iter()
        .map(|f| (f.id.to_owned(), f.version.to_owned()))
        .collect()
}

fn formats_stale(file: &str, now: &str) -> String {
    contract_snapshot::stale(
        SNAPSHOT,
        "this build and on-disk-format.md give",
        REGENERATE,
        file,
        now,
    )
}

#[test]
fn contract_snapshot_is_what_this_build_and_the_page_give() {
    let now =
        formats_snapshot(&build_versions(), &page()).unwrap_or_else(|e| panic!("{PAGE}: {e}"));
    let path = root().join(SNAPSHOT);
    if std::env::var_os(WRITE).is_some() {
        std::fs::write(&path, &now).expect("the snapshot is written");
        return;
    }
    let file = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(file == now, "{}", formats_stale(&file, &now));
}

#[test]
fn contract_snapshot_holds_every_format_of_this_build_and_every_fixture_release() {
    let text = formats_snapshot(&build_versions(), &page()).expect("the snapshot of this tree");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let ids: Vec<&str> = value["formats"]
        .as_object()
        .expect("formats")
        .keys()
        .map(String::as_str)
        .collect();
    let mut built: Vec<&str> = FORMATS.iter().map(|f| f.id).collect();
    built.sort_unstable();
    assert_eq!(ids, built, "each format of formats::FORMATS");
    let releases: Vec<String> = value["releases"]
        .as_object()
        .expect("releases")
        .keys()
        .cloned()
        .collect();
    let mut fixtures = fixture_releases();
    fixtures.sort();
    assert_eq!(releases, fixtures, "each release with a fixture");
}

#[test]
fn the_page_says_how_many_formats_the_server_reads_back() {
    let n = read_back_count(&page()).expect("the sentence on on-disk-format.md");
    assert!(
        (1..=FORMATS.len()).contains(&n),
        "the server reads back {n} of {} formats",
        FORMATS.len()
    );
}

#[test]
fn the_read_back_count_is_read_from_its_word() {
    assert_eq!(
        read_back_count("x. The server reads back the first four. The two"),
        Ok(4),
        "four"
    );
    assert_eq!(
        read_back_count("The server reads back the first one,"),
        Ok(1),
        "one"
    );
}

#[test]
fn a_page_without_the_read_back_sentence_is_refused() {
    assert!(
        read_back_count(SAMPLE).is_err(),
        "the sample has no such sentence"
    );
}

#[test]
fn a_read_back_count_in_another_word_is_refused() {
    assert_eq!(
        read_back_count("The server reads back the first seven."),
        Err("\"seven\" is not a count this test reads".to_owned()),
        "a word past the list"
    );
    assert!(
        read_back_count("The server reads back the first 4.").is_err(),
        "a digit"
    );
}

#[test]
fn the_first_formats_of_the_page_are_read_back_in_its_order() {
    let page = format!("{SAMPLE}\nThe server reads back the first one.\n");
    let versions = BTreeMap::from([
        ("catalog".to_owned(), "9".to_owned()),
        ("envelope".to_owned(), "1".to_owned()),
    ]);
    let value: serde_json::Value =
        serde_json::from_str(&formats_snapshot(&versions, &page).expect("snapshot")).expect("JSON");
    assert_eq!(
        value["formats"]["catalog"]["read_back"], true,
        "the first row"
    );
    assert_eq!(
        value["formats"]["envelope"]["read_back"], false,
        "the second row"
    );
    assert_eq!(
        value["releases"]["0.4.0"]["catalog"], "4",
        "the release table"
    );
}

#[test]
fn a_read_back_count_past_the_table_or_a_format_not_built_is_refused() {
    let versions = BTreeMap::from([
        ("catalog".to_owned(), "9".to_owned()),
        ("envelope".to_owned(), "1".to_owned()),
    ]);
    let too_many = format!("{SAMPLE}\nThe server reads back the first three.\n");
    assert!(
        formats_snapshot(&versions, &too_many).is_err(),
        "three of two"
    );
    let one = format!("{SAMPLE}\nThe server reads back the first one.\n");
    let fewer = BTreeMap::from([("catalog".to_owned(), "9".to_owned())]);
    assert_eq!(
        formats_snapshot(&fewer, &one).err().as_deref(),
        Some("envelope is on the page but not in this build"),
        "a page row the build lacks"
    );
}

#[test]
fn a_stale_formats_snapshot_names_the_command_that_writes_it() {
    let message = formats_stale("", "{\n");
    assert!(message.contains(REGENERATE), "{message}");
    assert!(
        message.contains("contract/formats.json is not what this build and on-disk-format.md give"),
        "{message}"
    );
}

// ---- The fixtures. ----

/// The releases with a fixture, oldest first.
fn fixture_releases() -> Vec<String> {
    releases_in(&formats_dir())
}

/// The release directories under `dir`, oldest first.
fn releases_in(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{} is there: {e}", dir.display()))
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .collect();
    out.sort_by_key(|r| release_order(r));
    out
}

fn release_order(release: &str) -> Vec<u32> {
    release
        .split('.')
        .map(|p| p.parse().expect("a number"))
        .collect()
}

// ---- The release the catalog's documentation names. ----
//
// `SCHEMA_VERSION`'s documentation names the newest release that refuses a
// catalog this build has opened. That number was written by hand; it is
// read here from the page's release table, which the fixtures hold.

/// The newest release of `releases` whose `column` is a number below
/// `version`; a cell that is not a number (`none`) is not.
fn newest_below(
    releases: &BTreeMap<String, BTreeMap<String, String>>,
    column: &str,
    version: i64,
) -> Option<String> {
    releases
        .iter()
        .filter(|(_, wrote)| {
            wrote
                .get(column)
                .and_then(|c| c.parse::<i64>().ok())
                .is_some_and(|v| v < version)
        })
        .map(|(release, _)| release.clone())
        .max_by_key(|r| release_order(r))
}

fn wrote(rows: &[(&str, &str)]) -> BTreeMap<String, BTreeMap<String, String>> {
    rows.iter()
        .map(|(release, catalog)| {
            (
                (*release).to_owned(),
                BTreeMap::from([("catalog".to_owned(), (*catalog).to_owned())]),
            )
        })
        .collect()
}

#[test]
fn the_catalogs_documentation_names_the_newest_release_that_refuses_it() {
    let releases = page_releases(&page()).expect("the release table reads");
    let newest = newest_below(&releases, "catalog", SCHEMA_VERSION)
        .expect("a release before this build's schema version");
    let source = std::fs::read_to_string(root().join("crates/vcon-server/src/catalog.rs"))
        .expect("catalog.rs is there");
    let flat = source
        .lines()
        .map(|l| l.trim_start().trim_start_matches("///").trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        flat.contains(&format!("is refused by release {newest} and earlier.")),
        "SCHEMA_VERSION's documentation names {newest}, the newest release at a catalog before {SCHEMA_VERSION}"
    );
}

#[test]
fn the_newest_release_below_a_version_is_picked() {
    let releases = wrote(&[("0.4.15", "9"), ("0.4.16", "9"), ("0.4.17", "10")]);
    assert_eq!(
        newest_below(&releases, "catalog", 10).as_deref(),
        Some("0.4.16"),
        "the newest at 9"
    );
}

#[test]
fn releases_are_ordered_by_number_when_picking_the_newest() {
    let releases = wrote(&[("0.4.9", "8"), ("0.4.10", "8"), ("0.4.2", "4")]);
    assert_eq!(
        newest_below(&releases, "catalog", 9).as_deref(),
        Some("0.4.10"),
        "0.4.10 is after 0.4.9, though it sorts before it as text"
    );
}

#[test]
fn a_release_at_the_version_itself_is_not_below_it() {
    let releases = wrote(&[("0.4.17", "10")]);
    assert_eq!(
        newest_below(&releases, "catalog", 10),
        None,
        "10 is not below 10"
    );
}

#[test]
fn a_cell_that_is_not_a_number_is_skipped() {
    let releases = wrote(&[("0.4.5", "none"), ("0.4.4", "6")]);
    assert_eq!(
        newest_below(&releases, "catalog", 10).as_deref(),
        Some("0.4.4"),
        "none is no version"
    );
}

#[test]
fn a_missing_column_or_table_has_no_newest() {
    assert_eq!(
        newest_below(&wrote(&[("0.4.4", "6")]), "postgres", 10),
        None,
        "no such column"
    );
    assert_eq!(
        newest_below(&BTreeMap::new(), "catalog", 10),
        None,
        "no rows"
    );
}

/// One kept version, as the manifest names it.
struct Row {
    tenant: String,
    uuid: uuid::Uuid,
    version: VersionId,
    order: usize,
    state: String,
    sha256: String,
    /// In a PostgreSQL fixture: "packed" into a Parquet file, or "object".
    place: Option<String>,
}

struct Manifest {
    archive_sha256: String,
    rows: Vec<Row>,
    purged: Vec<(String, uuid::Uuid)>,
    /// A PostgreSQL fixture's own section.
    postgres: Option<serde_json::Value>,
}

fn manifest(release: &str) -> Manifest {
    manifest_in(&formats_dir(), release)
}

fn manifest_in(dir: &Path, release: &str) -> Manifest {
    let path = dir.join(release).join("manifest.json");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("the manifest")).expect("JSON");
    let text = |v: &serde_json::Value, k: &str| v[k].as_str().expect(k).to_owned();
    let rows = doc["versions"]
        .as_array()
        .expect("versions")
        .iter()
        .map(|v| Row {
            tenant: text(v, "tenant"),
            uuid: text(v, "uuid").parse().expect("a uuid"),
            version: VersionId::parse(&text(v, "version")).expect("a version id"),
            order: usize::try_from(v["order"].as_u64().expect("order")).expect("small"),
            state: text(v, "state"),
            sha256: text(v, "sha256"),
            place: v["place"].as_str().map(str::to_owned),
        })
        .collect();
    let purged = doc["purged"]
        .as_array()
        .expect("purged")
        .iter()
        .map(|v| (text(v, "tenant"), text(v, "uuid").parse().expect("a uuid")))
        .collect();
    assert_eq!(
        text(&doc, "release"),
        release,
        "the manifest is the release's own"
    );
    Manifest {
        archive_sha256: text(&doc, "archive_sha256"),
        rows,
        purged,
        postgres: doc.get("postgres").cloned(),
    }
}

fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// Copies `from` to `to`, files 0600 and directories 0700, as the server
/// keeps them (git keeps no mode but the executable bit).
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir(to).expect("a directory");
    chmod(to, 0o700);
    for entry in std::fs::read_dir(from).expect("read") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
            chmod(&target, 0o600);
        }
    }
}

/// A copy of one release's fixture, with a configuration and a certificate
/// of its own.
struct Case {
    release: String,
    dir: tempfile::TempDir,
    manifest: Manifest,
}

impl Case {
    fn new(release: &str) -> Case {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let from = formats_dir().join(release);
        copy_tree(&from.join("data"), &dir.path().join("data"));
        let key = dir.path().join("master.key");
        std::fs::copy(from.join("master.key"), &key).expect("the master key");
        chmod(&key, 0o600);
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate");
        let (cert, tls) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        std::fs::write(&cert, made.cert.pem()).expect("write");
        std::fs::write(&tls, made.signing_key.serialize_pem()).expect("write");
        chmod(&tls, 0o600);
        std::fs::write(
            dir.path().join("vcon-server.toml"),
            format!(
                "listen = \"127.0.0.1:0\"\ndata_dir = {:?}\nmaster_key = {:?}\n\n[tls]\ncert = {:?}\nkey = {:?}\n",
                dir.path().join("data"),
                key,
                cert,
                tls
            ),
        )
        .expect("the configuration");
        Case {
            release: release.to_owned(),
            dir,
            manifest: manifest(release),
        }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn master_key(&self) -> PathBuf {
        self.dir.path().join("master.key")
    }

    /// Runs this build's `check` on the copy.
    fn check(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .arg("check")
            .arg("--config")
            .arg(self.dir.path().join("vcon-server.toml"))
            .output()
            .expect("the binary runs")
    }

    /// Runs `check` and asserts it passed, giving its report lines.
    fn checked(&self) -> Vec<serde_json::Value> {
        let out = self.check();
        assert!(
            out.status.success(),
            "check passes on {}'s data: {}",
            self.release,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout)
            .expect("UTF-8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("a JSON line"))
            .collect()
    }

    /// The SHA-256 of every file under the data directory, by its path.
    fn files(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in std::fs::read_dir(dir).expect("read") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    walk(&path, base, out);
                } else {
                    let bytes = std::fs::read(&path).expect("read");
                    let rel = path.strip_prefix(base).expect("under").to_owned();
                    out.insert(rel, digest(&SHA256, &bytes).as_ref().to_vec());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.data(), &self.data(), &mut out);
        out
    }

    /// server.sqlite, opened as SQLCipher opens it with the catalog key
    /// derived here as the format states it (HKDF-SHA256 of the master key,
    /// no salt, info `vcon-server/v1 catalog`), and with no migration.
    fn raw_registry(&self) -> rusqlite::Connection {
        let master = std::fs::read(self.master_key()).expect("the master key");
        let mut key = [0u8; 32];
        Salt::new(HKDF_SHA256, &[])
            .extract(&master)
            .expand(&[b"vcon-server/v1 catalog"], HKDF_SHA256)
            .expect("expand")
            .fill(&mut key)
            .expect("fill");
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        let conn = rusqlite::Connection::open(self.data().join("server.sqlite")).expect("opens");
        conn.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
            .expect("the key is set");
        conn
    }

    fn registry_version(&self) -> i64 {
        self.raw_registry()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .expect("the version reads under the derived key")
    }

    fn schema(&self) -> Vec<String> {
        let conn = self.raw_registry();
        let mut statement = conn
            .prepare("SELECT coalesce(sql, name) FROM sqlite_master ORDER BY name")
            .expect("prepare");
        statement
            .query_map([], |r| r.get(0))
            .expect("query")
            .map(|r| r.expect("a row"))
            .collect()
    }

    fn tenants(&self) -> Tenants {
        Tenants::open(
            &self.data(),
            Key::load_master(&self.master_key()).expect("the master key"),
            Arc::new(OsFs),
            Arc::new(SystemClock),
        )
        .expect("this build opens the registry")
    }

    /// The id of tenant `name`.
    fn tenant(&self, tenants: &Tenants, name: &str) -> TenantId {
        tenants
            .list(Actor::Cli)
            .expect("the tenants list")
            .into_iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{}: tenant {name} is there", self.release))
            .id
    }
}

const CALLER: Caller = Caller {
    key: KeyId([0x5a; 8]),
};

fn sha_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// Success.

fn check_passes_and_counts_every_kept_version(release: &str) {
    let case = Case::new(release);
    let lines = case.checked();
    for tenant in ["tenant-a", "tenant-b"] {
        let line = lines
            .iter()
            .find(|l| l["name"] == tenant)
            .unwrap_or_else(|| panic!("check reports {tenant}: {lines:?}"));
        let kept = case
            .manifest
            .rows
            .iter()
            .filter(|r| r.tenant == tenant)
            .count();
        assert_eq!(
            line["versions"].as_u64(),
            Some(u64::try_from(kept).expect("small")),
            "{release}: check counts {tenant}'s kept versions"
        );
        assert_eq!(
            line["damaged"],
            serde_json::json!([]),
            "{release}: nothing damaged"
        );
        assert_eq!(
            line["problems"],
            serde_json::json!([]),
            "{release}: no problem"
        );
    }
}

fn every_kept_version_reads_back_byte_equal(release: &str) {
    let case = Case::new(release);
    let tenants = case.tenants();
    for name in ["tenant-a", "tenant-b"] {
        let store = tenants
            .store(case.tenant(&tenants, name))
            .expect("the tenant's store");
        let rows: Vec<&Row> = case
            .manifest
            .rows
            .iter()
            .filter(|r| r.tenant == name)
            .collect();
        assert!(!rows.is_empty(), "{release}: {name} kept versions");
        for row in rows {
            let read = store
                .legal_get(&CALLER, row.uuid, Some(row.version))
                .unwrap_or_else(|e| {
                    panic!("{release}: {name} version {} reads: {e:?}", row.version)
                });
            assert_eq!(
                sha_hex(&read.stored.body),
                row.sha256,
                "{release}: {name} version {} is the body the release stored",
                row.version
            );
            assert_eq!(
                read.deleted,
                is_deleted(&case, name, row.uuid),
                "{release}: {name} version {} is of a uuid in the state the release left",
                row.version
            );
        }
    }
}

/// Whether the manifest's latest version of `uuid` is a deleted one.
fn is_deleted(case: &Case, tenant: &str, uuid: uuid::Uuid) -> bool {
    case.manifest
        .rows
        .iter()
        .filter(|r| r.tenant == tenant && r.uuid == uuid)
        .max_by_key(|r| r.order)
        .is_some_and(|r| r.state.starts_with("deleted"))
}

fn every_uuid_lists_its_kept_versions_in_order(release: &str) {
    let case = Case::new(release);
    let tenants = case.tenants();
    for name in ["tenant-a", "tenant-b"] {
        let store = tenants
            .store(case.tenant(&tenants, name))
            .expect("the tenant's store");
        let mut by_uuid: BTreeMap<uuid::Uuid, Vec<&Row>> = BTreeMap::new();
        for row in case.manifest.rows.iter().filter(|r| r.tenant == name) {
            by_uuid.entry(row.uuid).or_default().push(row);
        }
        for rows in by_uuid.values_mut() {
            rows.sort_by_key(|r| r.order);
            let listed = store
                .legal_versions(&CALLER, rows[0].uuid)
                .expect("the versions list");
            let ids: Vec<VersionId> = listed.iter().map(|v| v.version).collect();
            let want: Vec<VersionId> = rows.iter().map(|r| r.version).collect();
            assert_eq!(
                ids, want,
                "{release}: {name} keeps these versions, oldest first"
            );
            let current: Vec<bool> = listed.iter().map(|v| v.current).collect();
            let want: Vec<bool> = rows
                .iter()
                .map(|r| r.state.starts_with("current"))
                .collect();
            assert_eq!(
                current, want,
                "{release}: {name}: the current version is the release's"
            );
        }
    }
}

fn the_registry_is_at_this_builds_version_after_the_first_open(release: &str) {
    let case = Case::new(release);
    case.checked();
    assert_eq!(
        case.registry_version(),
        SCHEMA_VERSION,
        "{release}: the first open brings server.sqlite to this build's version"
    );
}

fn a_second_open_changes_no_file(release: &str) {
    let case = Case::new(release);
    case.checked();
    let first = case.files();
    case.checked();
    assert_eq!(
        case.files(),
        first,
        "{release}: a second check writes nothing"
    );
}

// Failure.

fn a_registry_one_version_past_this_build_is_refused_by_name(release: &str) {
    let case = Case::new(release);
    case.raw_registry()
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .expect("the version is set");
    let before = case.files();
    let out = case.check();
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "{release}: check refuses a later version"
    );
    assert!(
        said.contains(&format!(
            "schema version {} is not one this build knows",
            SCHEMA_VERSION + 1
        )),
        "{release}: the refusal names the version: {said}"
    );
    assert_eq!(
        case.files(),
        before,
        "{release}: the refused data is left as it was"
    );
}

fn a_wrong_master_key_opens_nothing_and_changes_nothing(release: &str) {
    let case = Case::new(release);
    std::fs::write(case.master_key(), [0x33u8; 32]).expect("another key");
    let before = case.files();
    let out = case.check();
    assert!(
        !out.status.success(),
        "{release}: check refuses another master key"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("key"),
        "{release}: the refusal names the key: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        case.files(),
        before,
        "{release}: nothing is written under the wrong key"
    );
}

fn a_purged_uuid_is_gone(release: &str) {
    let case = Case::new(release);
    assert!(
        !case.manifest.purged.is_empty(),
        "{release}: the workload purged a uuid"
    );
    let tenants = case.tenants();
    for (name, uuid) in &case.manifest.purged {
        let store = tenants
            .store(case.tenant(&tenants, name))
            .expect("the tenant's store");
        assert!(
            matches!(store.legal_versions(&CALLER, *uuid), Err(StoreError::Gone)),
            "{release}: a uuid {name} purged stays gone"
        );
    }
}

/// The registry's own version before this build opens it.
fn written_version(release: &str) -> i64 {
    Case::new(release).registry_version()
}

fn a_failed_migration_leaves_the_releases_version_whole(release: &str) {
    let case = Case::new(release);
    let old = case.registry_version();
    assert!(
        old < SCHEMA_VERSION,
        "{release} wrote an older version: {old}"
    );
    // A step of the migration adds a column: a column of that name already
    // there makes it fail, and with it the whole transaction. A registry
    // before version 9 gains the access keys' expiry; one at 9 gains only
    // the audit log's mark (a log before version 5 is made again, so a
    // column put in it would not be in the way).
    let (table, column) = if old < 9 {
        ("api_key", "expires_at")
    } else {
        ("audit", "counted")
    };
    case.raw_registry()
        .execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} INTEGER;"))
        .expect("the column in the way");
    let before = case.schema();
    let out = case.check();
    assert!(
        !out.status.success(),
        "{release}: the migration fails at the {table} step"
    );
    assert_eq!(
        case.registry_version(),
        old,
        "{release}: still the release's version"
    );
    assert_eq!(
        case.schema(),
        before,
        "{release}: no earlier step of the migration is kept"
    );
    case.raw_registry()
        .execute_batch(&format!("ALTER TABLE {table} DROP COLUMN {column};"))
        .expect("the column taken away");
    case.checked();
    assert_eq!(
        case.registry_version(),
        SCHEMA_VERSION,
        "{release}: then it migrates"
    );
}

// Edge.

fn held_versions_outlive_a_sweep_under_a_0d_period(release: &str) {
    let case = Case::new(release);
    let tenants = case.tenants();
    let id = case.tenant(&tenants, "tenant-b");
    let store = tenants.store(id).expect("the tenant's store");
    let retention = tenants
        .retention(id)
        .expect("its period")
        .expect("0d is set");
    store.purge_due(retention).expect("the sweep runs");
    for row in case.manifest.rows.iter().filter(|r| r.tenant == "tenant-b") {
        let read = store.legal_get(&CALLER, row.uuid, Some(row.version));
        assert!(
            read.is_ok(),
            "{release}: tenant-b version {} survives the sweep",
            row.version
        );
    }
    assert!(
        case.manifest
            .rows
            .iter()
            .any(|r| r.tenant == "tenant-b" && r.state.starts_with("hidden+held")),
        "{release}: a hidden held version was there to purge"
    );
}

fn a_deleted_vcon_is_gone_to_an_ordinary_read(release: &str) {
    let case = Case::new(release);
    let tenants = case.tenants();
    let store = tenants
        .store(case.tenant(&tenants, "tenant-a"))
        .expect("the store");
    let deleted: Vec<&Row> = case
        .manifest
        .rows
        .iter()
        .filter(|r| r.tenant == "tenant-a" && r.state.starts_with("deleted"))
        .collect();
    assert!(deleted.len() >= 2, "{release}: deleted vCons were kept");
    for row in deleted {
        let refused = store.get(&CALLER, row.uuid).err();
        assert!(
            matches!(refused, Some(StoreError::Gone)),
            "{release}: an ordinary read answers gone for deleted version {}: {refused:?}",
            row.version
        );
    }
}

fn the_fixture_was_written_by_the_pinned_release(release: &str) {
    written_by_the_pinned_release(&manifest(release), release);
}

fn written_by_the_pinned_release(manifest: &Manifest, release: &str) {
    let pins = std::fs::read_to_string(root().join("scripts/docs-test-pins.txt")).expect("pins");
    let sha = &manifest.archive_sha256;
    let line = pins
        .lines()
        .find(|l| l.starts_with(sha.as_str()))
        .unwrap_or_else(|| panic!("{release}: the archive hash is a pin"));
    assert!(
        line.contains(&format!("/v{release}/vcon-server-{release}-"))
            && line.ends_with("-unknown-linux-gnu.tar.gz"),
        "{release}: the pin is the release's archive: {line}"
    );
}

fn every_object_starts_with_the_envelope_version_the_page_states(release: &str) {
    let releases = page_releases(&page()).expect("the release table");
    let stated: u8 = releases[release]["envelope"].parse().expect("a number");
    let case = Case::new(release);
    let objects: Vec<PathBuf> = case
        .files()
        .into_keys()
        .filter(|p| p.components().any(|c| c.as_os_str() == "objects"))
        .collect();
    assert!(
        !objects.is_empty(),
        "{release}: the release wrote object files"
    );
    for path in objects {
        let bytes = std::fs::read(case.data().join(&path)).expect("read");
        assert_eq!(
            bytes.first(),
            Some(&stated),
            "{release}: {} opens with the envelope's version",
            path.display()
        );
    }
}

fn the_page_states_the_catalog_version_the_release_wrote(release: &str) {
    let releases = page_releases(&page()).expect("the release table");
    assert_eq!(
        releases[release]["catalog"],
        written_version(release).to_string(),
        "{PAGE}: the catalog version {release} wrote"
    );
}

/// The tests of one release's fixture; `older` adds the ones only data at
/// an earlier catalog version than this build's can have.
macro_rules! release {
    ($name:ident, $release:literal $(, $older:ident)?) => {
        mod $name {
            #[test]
            fn check_passes_and_counts_every_kept_version() {
                super::check_passes_and_counts_every_kept_version($release);
            }
            #[test]
            fn every_kept_version_reads_back_byte_equal() {
                super::every_kept_version_reads_back_byte_equal($release);
            }
            #[test]
            fn every_uuid_lists_its_kept_versions_in_order() {
                super::every_uuid_lists_its_kept_versions_in_order($release);
            }
            #[test]
            fn the_registry_is_at_this_builds_version_after_the_first_open() {
                super::the_registry_is_at_this_builds_version_after_the_first_open($release);
            }
            #[test]
            fn a_second_open_changes_no_file() {
                super::a_second_open_changes_no_file($release);
            }
            #[test]
            fn a_registry_one_version_past_this_build_is_refused_by_name() {
                super::a_registry_one_version_past_this_build_is_refused_by_name($release);
            }
            #[test]
            fn a_wrong_master_key_opens_nothing_and_changes_nothing() {
                super::a_wrong_master_key_opens_nothing_and_changes_nothing($release);
            }
            #[test]
            fn a_purged_uuid_is_gone() {
                super::a_purged_uuid_is_gone($release);
            }
            #[test]
            fn held_versions_outlive_a_sweep_under_a_0d_period() {
                super::held_versions_outlive_a_sweep_under_a_0d_period($release);
            }
            #[test]
            fn a_deleted_vcon_is_gone_to_an_ordinary_read() {
                super::a_deleted_vcon_is_gone_to_an_ordinary_read($release);
            }
            #[test]
            fn the_fixture_was_written_by_the_pinned_release() {
                super::the_fixture_was_written_by_the_pinned_release($release);
            }
            #[test]
            fn every_object_starts_with_the_envelope_version_the_page_states() {
                super::every_object_starts_with_the_envelope_version_the_page_states($release);
            }
            #[test]
            fn the_page_states_the_catalog_version_the_release_wrote() {
                super::the_page_states_the_catalog_version_the_release_wrote($release);
            }
            $(
                #[test]
                fn a_failed_migration_leaves_the_releases_version_whole() {
                    let _ = stringify!($older);
                    super::a_failed_migration_leaves_the_releases_version_whole($release);
                }
            )?
        }
    };
}

release!(release_0_4_0, "0.4.0", older);
release!(release_0_4_1, "0.4.1", older);
release!(release_0_4_2, "0.4.2", older);
release!(release_0_4_3, "0.4.3", older);
release!(release_0_4_4, "0.4.4", older);
release!(release_0_4_5, "0.4.5", older);
release!(release_0_4_6, "0.4.6", older);
release!(release_0_4_7, "0.4.7", older);
release!(release_0_4_8, "0.4.8", older);
release!(release_0_4_9, "0.4.9", older);
release!(release_0_4_10, "0.4.10", older);
release!(release_0_4_11, "0.4.11", older);
release!(release_0_4_12, "0.4.12", older);
release!(release_0_4_13, "0.4.13", older);
release!(release_0_4_14, "0.4.14", older);
release!(release_0_4_15, "0.4.15", older);
release!(release_0_4_16, "0.4.16", older);
release!(release_0_4_17, "0.4.17");

#[test]
fn the_older_tests_cover_exactly_the_releases_that_wrote_an_older_version() {
    let older: Vec<String> = fixture_releases()
        .into_iter()
        .filter(|r| written_version(r) < SCHEMA_VERSION)
        .collect();
    // The releases whose `release!` line carries `older`, in the file's order.
    let marked: Vec<String> = SOURCE
        .lines()
        .filter(|line| line.starts_with("release!(") && line.ends_with(", older);"))
        .filter_map(|line| line.split('"').nth(1))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        older, marked,
        "the releases whose fixtures get the failed-migration test"
    );
    assert!(
        older.contains(&"0.4.16".to_owned()),
        "0.4.16 wrote version 9, older than this build's {SCHEMA_VERSION}"
    );
}

/// The releases named by the lines `<name>!(<module>, "<release>"...)` of
/// `source`, in the file's order: the test modules a macro makes.
fn macro_releases(source: &str, name: &str) -> Vec<String> {
    let call = format!("{name}!(");
    source
        .lines()
        .filter_map(|line| line.strip_prefix(&call))
        .filter_map(|rest| rest.split('"').nth(1))
        .map(str::to_owned)
        .collect()
}

/// This file's own text, to read its macro lines.
const SOURCE: &str = include_str!("formats.rs");

#[test]
fn every_fixture_has_its_test_module_and_every_module_a_fixture() {
    let mut fixtures = fixture_releases();
    fixtures.sort_by_key(|r| release_order(r));
    assert_eq!(
        macro_releases(SOURCE, "release"),
        fixtures,
        "one release! line per directory under tests/data/formats, oldest first"
    );
    let mut pg = releases_in(&pg_formats_dir());
    pg.sort_by_key(|r| release_order(r));
    assert_eq!(
        macro_releases(SOURCE, "pg_release"),
        pg,
        "one pg_release! line per directory under tests/data/formats-postgres, oldest first"
    );
}

#[test]
fn a_macro_line_names_its_release() {
    let source =
        "release!(release_0_4_0, \"0.4.0\");\nrelease!(release_0_4_1, \"0.4.1\", older);\n";
    assert_eq!(
        macro_releases(source, "release"),
        ["0.4.0", "0.4.1"],
        "with and without `older`"
    );
}

#[test]
fn a_macro_line_of_another_macro_is_not_read() {
    let source = "pg_release!(pg_release_0_4_6, \"0.4.6\");\nrelease!(release_0_4_0, \"0.4.0\");\n";
    assert_eq!(macro_releases(source, "release"), ["0.4.0"], "release!");
    assert_eq!(
        macro_releases(source, "pg_release"),
        ["0.4.6"],
        "pg_release!"
    );
}

#[test]
fn the_macro_definition_and_indented_text_are_not_lines() {
    let source = "macro_rules! release {\n    release!(release_x, \"9.9.9\");\n// release!(release_0_1_0, \"0.1.0\");\n";
    assert!(
        macro_releases(source, "release").is_empty(),
        "only a line that starts with the call"
    );
}

#[test]
fn a_macro_line_without_a_quoted_release_is_skipped() {
    assert!(
        macro_releases("release!(release_0_4_0);\n", "release").is_empty(),
        "no release text"
    );
}

// ---- The PostgreSQL deployments. ----
//
// Each directory under tests/data/formats-postgres/<release>/ was written
// by that release's binary with its catalogs in PostgreSQL 16 and its
// objects in a RustFS bucket (scripts/make-format-fixtures.py --postgres):
// `dump.sql` (pg_dump of the database), `bucket/` (every object the bucket
// held, by its key: the versions packed into Parquet files by a pack run,
// the versions stored after it as objects, the manifests and the claim),
// `data/` (the lock files), the release's test master key and
// manifest.json. The tests below without a database run in every build;
// the ones in each `postgres` module load the dump into a database of
// their own and the objects into a bucket of their own, on the servers of
// CI's backends-postgres job.

fn pg_formats_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/formats-postgres")
}

/// The releases the page's release table says wrote a PostgreSQL schema:
/// each row whose `postgres` cell is not `none`, oldest first.
fn releases_with_postgres(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (release, wrote) in page_releases(text)? {
        match wrote.get("postgres").map(String::as_str) {
            None => return Err(format!("the row of {release} has no postgres cell")),
            Some("none") => {}
            Some(_) => out.push(release),
        }
    }
    out.sort_by_key(|r| release_order(r));
    Ok(out)
}

/// The numbers of the lines of `dump` that start a psql meta-command or end
/// COPY data: a server given the dump as one query refuses them.
fn meta_lines(dump: &str) -> Vec<usize> {
    dump.lines()
        .enumerate()
        .filter(|(_, l)| l.trim_start().starts_with('\\'))
        .map(|(i, _)| i + 1)
        .collect()
}

/// What a key of the bucket names: the third part of
/// `tenants/<tenant>/<kind>/...`, or None for a key outside a tenant.
fn key_kind(key: &str) -> Option<&str> {
    let mut parts = key.split('/');
    (parts.next() == Some("tenants")).then_some(())?;
    parts.next().filter(|t| !t.is_empty())?;
    let kind = parts.next()?;
    parts.next().filter(|rest| !rest.is_empty())?;
    Some(kind)
}

/// The `format` a Parquet storage manifest states.
fn manifest_format(text: &str) -> Result<String, String> {
    let doc: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("the manifest is not JSON: {e}"))?;
    doc["format"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "the manifest states no format".to_owned())
}

/// Every file under `bucket`, as its key and its path, by key.
fn bucket_files(bucket: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
        for entry in std::fs::read_dir(dir).expect("read") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let key = path
                    .strip_prefix(base)
                    .expect("under")
                    .to_str()
                    .expect("a UTF-8 key")
                    .to_owned();
                out.push((key, path));
            }
        }
    }
    let mut out = Vec::new();
    walk(bucket, bucket, &mut out);
    out.sort();
    out
}

fn pg_manifest(release: &str) -> Manifest {
    manifest_in(&pg_formats_dir(), release)
}

/// The schema version a PostgreSQL fixture's manifest records, which the
/// script read from the database.
fn pg_schema(release: &str) -> i64 {
    pg_manifest(release)
        .postgres
        .unwrap_or_else(|| panic!("{release}: the manifest has a postgres section"))["schema"]
        .as_i64()
        .unwrap_or_else(|| panic!("{release}: the postgres section states the schema"))
}

/// The tables the migrations after schema version 13 make, each with the
/// version that makes it, oldest first: a release's restored rows are
/// compared without the tables made after its schema, each such table
/// must be there and empty after `migrate`, and the first table the
/// build's last migration makes is the one a planted table breaks.
const LATER_TABLES: &[(i64, &str)] = &[
    (14, "pending_write"),
    (15, "pending_tenant"),
    (15, "pending_file"),
    (15, "lease"),
];

/// The tables of [`LATER_TABLES`] a migration past `release_schema` makes.
fn tables_made_after(release_schema: i64) -> Vec<&'static str> {
    LATER_TABLES
        .iter()
        .filter(|(version, _)| *version > release_schema)
        .map(|(_, table)| *table)
        .collect()
}

/// The first table the migration to `build_schema` makes, if it makes one.
fn last_migration_table(build_schema: i64) -> Option<&'static str> {
    LATER_TABLES
        .iter()
        .find(|(version, _)| *version == build_schema)
        .map(|(_, table)| *table)
}

#[test]
fn the_tables_made_after_a_schema_are_the_later_migrations_tables() {
    assert_eq!(
        tables_made_after(13),
        ["pending_write", "pending_tenant", "pending_file", "lease"],
        "a release at 13 lacks both"
    );
    assert_eq!(
        tables_made_after(14),
        ["pending_tenant", "pending_file", "lease"],
        "one at 14 lacks the newer"
    );
}

#[test]
fn a_release_at_the_build_s_schema_lacks_no_later_table() {
    assert!(
        tables_made_after(i64::from(vcon_server::formats::POSTGRES_SCHEMA)).is_empty(),
        "the build's own schema has every table"
    );
}

#[test]
fn the_build_s_last_migration_makes_a_table_a_planted_one_breaks() {
    assert_eq!(
        last_migration_table(i64::from(vcon_server::formats::POSTGRES_SCHEMA)),
        Some("pending_tenant"),
        "the first table of the build's last migration"
    );
}

#[test]
fn a_schema_no_later_migration_reached_has_no_last_table() {
    assert_eq!(last_migration_table(13), None, "13 is before the list");
    assert_eq!(last_migration_table(99), None, "and 99 after it");
}

#[test]
fn the_later_tables_are_in_order_and_end_at_the_build_s_schema() {
    assert!(
        LATER_TABLES.windows(2).all(|w| w[0].0 <= w[1].0),
        "oldest first: {LATER_TABLES:?}"
    );
    assert_eq!(
        LATER_TABLES.last().map(|(version, _)| *version),
        Some(i64::from(vcon_server::formats::POSTGRES_SCHEMA)),
        "a new migration's table is added here"
    );
}

/// Whether `migrate` of a build at `build_schema` has a migration left to
/// apply to a database at `release_schema`. A release at the build's own
/// schema has none, so no migration of it can fail part way.
fn a_migration_is_left(release_schema: i64, build_schema: i64) -> bool {
    release_schema < build_schema
}

#[test]
fn a_fixture_one_schema_below_the_builds_runs_the_failed_migration_case() {
    // The build's last migration is the one a planted table can break;
    // a fixture one schema below runs it alone, so its failure is tested.
    let build = i64::from(vcon_server::formats::POSTGRES_SCHEMA);
    let below: Vec<String> = releases_in(&pg_formats_dir())
        .into_iter()
        .filter(|r| pg_schema(r) == build - 1)
        .collect();
    assert!(
        !below.is_empty(),
        "a PostgreSQL fixture at schema {} runs the last migration's failure",
        build - 1
    );
    assert!(
        below
            .iter()
            .all(|r| a_migration_is_left(pg_schema(r), build)),
        "each of {below:?} has the last migration left"
    );
}

#[test]
fn the_0_4_14_fixture_runs_the_planted_table_case_against_version_14() {
    assert_eq!(pg_schema("0.4.14"), 13, "0.4.14 wrote schema 13");
    assert!(
        a_migration_is_left(
            pg_schema("0.4.14"),
            i64::from(vcon_server::formats::POSTGRES_SCHEMA)
        ),
        "0.4.14's fixture has the migrations from version 14 left, which a planted table breaks"
    );
}

#[test]
fn a_schema_one_below_the_builds_leaves_a_migration() {
    assert!(a_migration_is_left(13, 14), "13 to 14 is one migration");
}

#[test]
fn a_schema_several_below_the_builds_leaves_migrations() {
    assert!(a_migration_is_left(6, 14), "6 to 14 is eight migrations");
}

#[test]
fn a_database_no_migration_touched_leaves_every_migration() {
    assert!(
        a_migration_is_left(0, 14),
        "version 0: every migration is left"
    );
}

#[test]
fn a_schema_at_the_builds_leaves_no_migration() {
    assert!(
        !a_migration_is_left(14, 14),
        "a release at the build's schema has nothing to migrate"
    );
}

#[test]
fn a_schema_past_the_builds_leaves_no_migration() {
    assert!(
        !a_migration_is_left(15, 14),
        "a newer schema is refused, not migrated"
    );
}

const PG_SAMPLE: &str = "\
## What each release wrote

| Release | `catalog` | `postgres` |
|---|---|---|
| 0.4.5 | 9 | none |
| 0.4.10 | 9 | 14 |
| 0.4.6 | 9 | 13 |
";

#[test]
fn the_releases_with_a_schema_are_the_rows_not_none_oldest_first() {
    assert_eq!(
        releases_with_postgres(PG_SAMPLE).expect("the sample reads"),
        ["0.4.6", "0.4.10"],
        "0.4.5 wrote none; 0.4.10 sorts after 0.4.6 by number"
    );
}

#[test]
fn a_release_table_without_a_postgres_column_is_refused() {
    let text = PG_SAMPLE
        .replace(" `postgres` |", " `other` |")
        .replace("| none |", "| x |");
    let why = releases_with_postgres(&text).expect_err("no column");
    assert!(why.contains("has no postgres cell"), "names it: {why}");
}

#[test]
fn a_page_where_every_release_wrote_none_has_no_postgres_release() {
    let text = PG_SAMPLE
        .replace("| 14 |", "| none |")
        .replace("| 13 |", "| none |");
    assert_eq!(
        releases_with_postgres(&text).expect("reads"),
        Vec::<String>::new(),
        "none"
    );
}

#[test]
fn a_meta_command_line_is_found_by_its_number() {
    assert_eq!(
        meta_lines("SET a = 1;\n\\restrict K\nSELECT 1;\n"),
        [2],
        "the second line"
    );
}

#[test]
fn copy_data_and_an_indented_meta_command_are_found() {
    assert_eq!(
        meta_lines("COPY t FROM stdin;\n1\n\\.\n  \\connect x\n"),
        [3, 4],
        "the end of COPY data, and a meta-command after spaces"
    );
}

#[test]
fn a_backslash_inside_a_line_is_not_a_meta_command() {
    assert!(
        meta_lines("INSERT INTO t VALUES ('\\x00ff');\n").is_empty(),
        "a bytea literal is data"
    );
}

#[test]
fn the_kind_of_each_tenant_key_is_its_third_part() {
    for (key, kind) in [
        ("tenants/ab/objects/cd", "objects"),
        (
            "tenants/ab/parquet/date=2026-11-17/part-01.parquet",
            "parquet",
        ),
        ("tenants/ab/manifest/00000000000000000001.json", "manifest"),
    ] {
        assert_eq!(key_kind(key), Some(kind), "{key}");
    }
}

#[test]
fn a_key_outside_a_tenant_has_no_kind() {
    for key in [
        "vcon-server.claim",
        "other/ab/objects/cd",
        "tenants//objects/cd",
    ] {
        assert_eq!(key_kind(key), None, "{key}");
    }
}

#[test]
fn a_tenant_key_with_nothing_after_its_kind_has_no_kind() {
    for key in ["tenants/ab/objects", "tenants/ab/objects/", "tenants/ab"] {
        assert_eq!(key_kind(key), None, "{key}");
    }
}

#[test]
fn the_format_of_a_manifest_is_read() {
    assert_eq!(
        manifest_format(r#"{"format":"vcon-server.parquet-storage/1","rows":1}"#),
        Ok("vcon-server.parquet-storage/1".to_owned()),
        "its format"
    );
}

#[test]
fn a_manifest_that_is_not_json_is_refused() {
    let why = manifest_format("{").expect_err("not JSON");
    assert!(why.contains("not JSON"), "says so: {why}");
}

#[test]
fn a_manifest_with_no_format_or_a_number_for_one_is_refused() {
    for text in [r#"{"rows":1}"#, r#"{"format":1}"#] {
        let why = manifest_format(text).expect_err("no format");
        assert!(why.contains("no format"), "{text}: {why}");
    }
}

#[test]
fn the_postgres_fixtures_are_the_releases_the_page_says_wrote_a_schema() {
    assert_eq!(
        releases_in(&pg_formats_dir()),
        releases_with_postgres(&page()).expect("the release table"),
        "one PostgreSQL fixture for each release whose postgres cell is not none"
    );
}

#[test]
fn no_postgres_fixture_wrote_a_newer_schema_than_this_build() {
    // A release's schema is this build's or one that `migrate` brings up:
    // each fixture's live tests run `migrate` first when it is older
    // (live::Restored::new), and check that a migrate made to fail leaves
    // the release's schema whole.
    for release in releases_in(&pg_formats_dir()) {
        assert!(
            pg_schema(&release) <= i64::from(vcon_server::formats::POSTGRES_SCHEMA),
            "{release} wrote schema {}, newer than this build's",
            pg_schema(&release)
        );
    }
}

#[test]
fn a_fixture_wrote_an_older_schema_so_the_upgrade_is_tested() {
    let older: Vec<String> = releases_in(&pg_formats_dir())
        .into_iter()
        .filter(|r| pg_schema(r) < i64::from(vcon_server::formats::POSTGRES_SCHEMA))
        .collect();
    assert!(
        !older.is_empty(),
        "at least one release's fixture is migrated by its live tests"
    );
}

// ---- Where CI runs the `postgres` modules. ----

/// Whether the `backends-postgres` job of the workflow `ci` runs the
/// `postgres::` tests of the test target `target`: a nextest line of that
/// job with the postgres and backend-tests features, `--test <target>`
/// and the `"postgres::"` filter.
fn ci_runs_postgres_modules_of(ci: &str, target: &str) -> bool {
    let mut in_job = false;
    for line in ci.lines() {
        if line.starts_with("  ") && !line.starts_with("   ") && line.trim_end().ends_with(':') {
            in_job = line.trim() == "backends-postgres:";
            continue;
        }
        if !in_job || !line.contains("scripts/nextest.sh run") {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let names_target = words.windows(2).any(|w| w[0] == "--test" && w[1] == target);
        let features = words.windows(2).any(|w| {
            w[0] == "--features"
                && w[1].split(',').any(|f| f == "postgres")
                && w[1].split(',').any(|f| f == "backend-tests")
        });
        if names_target && features && words.contains(&"\"postgres::\"") {
            return true;
        }
    }
    false
}

#[test]
fn ci_runs_the_postgres_modules_of_this_file() {
    let ci = std::fs::read_to_string(root().join(".github/workflows/ci.yml")).expect("ci.yml");
    assert!(
        ci_runs_postgres_modules_of(&ci, "formats"),
        "backends-postgres runs `--test formats` with the \"postgres::\" filter, on PostgreSQL 16 and 18"
    );
}

const CI_SAMPLE: &str = "\
jobs:
  backends:
    steps:
      - run: scripts/nextest.sh run -p vcon-server --features postgres,backend-tests --test formats \"postgres::\"
  backends-postgres:
    steps:
      - run: |
          scripts/nextest.sh run -p vcon-server --locked --features postgres,backend-tests --no-fail-fast --test serve --test formats \"postgres::\"
  docs:
    steps: []
";

#[test]
fn a_nextest_line_of_the_job_names_the_target() {
    assert!(ci_runs_postgres_modules_of(CI_SAMPLE, "formats"), "formats");
    assert!(ci_runs_postgres_modules_of(CI_SAMPLE, "serve"), "serve");
}

#[test]
fn a_target_the_job_does_not_run_is_not_found() {
    assert!(
        !ci_runs_postgres_modules_of(CI_SAMPLE, "rotation"),
        "rotation is not on the line"
    );
}

#[test]
fn a_line_in_another_job_does_not_count() {
    let ci = CI_SAMPLE.replace("--test serve --test formats", "--test serve");
    assert!(
        !ci_runs_postgres_modules_of(&ci, "formats"),
        "only backends-postgres has the servers"
    );
}

#[test]
fn a_line_without_the_filter_or_the_features_does_not_count() {
    let ci = CI_SAMPLE.replace(
        "--test formats \"postgres::\"\n  docs",
        "--test formats\n  docs",
    );
    assert!(!ci_runs_postgres_modules_of(&ci, "formats"), "no filter");
    let ci = CI_SAMPLE.replace(
        "--locked --features postgres,backend-tests",
        "--locked --features postgres",
    );
    assert!(
        !ci_runs_postgres_modules_of(&ci, "formats"),
        "the live module needs backend-tests"
    );
}

#[test]
fn a_target_whose_name_starts_another_does_not_count() {
    let ci = CI_SAMPLE.replace(
        "--test formats \"postgres::\"\n  docs",
        "--test formats_x \"postgres::\"\n  docs",
    );
    assert!(!ci_runs_postgres_modules_of(&ci, "formats"), "formats_x");
}

#[test]
fn a_workflow_without_the_job_runs_nothing() {
    assert!(
        !ci_runs_postgres_modules_of("jobs:\n  rust:\n    steps: []\n", "formats"),
        "no job"
    );
}

// ---- How git reads the fixtures. ----
//
// The fixtures are bytes: git must not diff, merge or convert them, and the
// commit hook's scan of staged lines must not read them as UTF-8. Git's own
// test for binary looks for a NUL byte, which a key, a lock file or a SQL
// dump lacks, so .gitattributes says it. A release directory's own
// manifest.json is the exception: JSON written for reviewers, kept as text.
// A manifest object in a bucket is a stored object and stays bytes.

/// Whether git must read the fixture file `rel`, a path under a formats
/// directory (`<release>/...`), as text.
fn git_reads_as_text(rel: &str) -> bool {
    let mut parts = rel.split('/');
    let release = parts.next().unwrap_or("");
    !release.is_empty() && parts.next() == Some("manifest.json") && parts.next().is_none()
}

/// What is wrong in the output of `git check-attr text -- <paths>`, where
/// each path is a fixture under `tests/data/formats/` or
/// `tests/data/formats-postgres/`.
fn text_attribute_problems(check_attr: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in check_attr.lines() {
        let Some((path, value)) = line.rsplit_once(": text: ") else {
            out.push(format!("`{line}`: not a check-attr line"));
            continue;
        };
        let rel = [
            "crates/vcon-server/tests/data/formats/",
            "crates/vcon-server/tests/data/formats-postgres/",
        ]
        .iter()
        .find_map(|base| path.strip_prefix(base));
        let Some(rel) = rel else {
            out.push(format!("{path}: not under a formats directory"));
            continue;
        };
        if git_reads_as_text(rel) {
            if value != "set" {
                out.push(format!("{path}: git must read it as text (text: {value})"));
            }
        } else if value != "unset" {
            out.push(format!(
                "{path}: git must treat it as bytes (text: {value})"
            ));
        }
    }
    out
}

/// Every `.gitattributes` git reads for `paths` (each relative to `root`):
/// the one in each directory from `root` down to the path's own, as
/// (path relative to `root`, content), each once, in the order the paths
/// reach them.
fn attribute_files(root: &Path, paths: &[String]) -> Vec<(String, String)> {
    let mut dirs: Vec<String> = Vec::new();
    for path in paths {
        let mut dir = String::new();
        if !dirs.contains(&dir) {
            dirs.push(dir.clone());
        }
        let parts: Vec<&str> = path.split('/').collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            dir = if dir.is_empty() {
                (*part).to_owned()
            } else {
                format!("{dir}/{part}")
            };
            if !dirs.contains(&dir) {
                dirs.push(dir.clone());
            }
        }
    }
    let mut out = Vec::new();
    for dir in dirs {
        let rel = if dir.is_empty() {
            ".gitattributes".to_owned()
        } else {
            format!("{dir}/.gitattributes")
        };
        if let Ok(text) = std::fs::read_to_string(root.join(&rel)) {
            out.push((rel, text));
        }
    }
    out
}

/// `git check-attr text -- <paths>` in a new repository that holds only
/// `files` (each a `.gitattributes`, as [`attribute_files`] gives them),
/// run with `env` as its environment but every `GIT_*` variable of it.
/// The commit hook runs the tests in a copy of the staged tree that is no
/// repository, and git exports GIT_DIR to a hook only in a linked worktree
/// (VCON-HANDOFF-37), so a `git -C <root>` answers in one checkout and fails
/// in another.
fn check_attr_text(
    files: &[(String, String)],
    paths: &[String],
    env: &[(std::ffi::OsString, std::ffi::OsString)],
) -> String {
    let dir = tempfile::tempdir().expect("a temporary directory");
    for (rel, text) in files {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the attribute file's directory");
        }
        std::fs::write(&path, text).expect("the attribute file");
    }
    let git = |args: &[&str], paths: &[String]| {
        let mut command = Command::new("git");
        command.env_clear();
        for (name, value) in env {
            if !name.to_string_lossy().starts_with("GIT_") {
                command.env(name, value);
            }
        }
        let out = command
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .args(paths)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("UTF-8")
    };
    git(&["init", "-q"], &[]);
    git(&["check-attr", "text", "--"], paths)
}

/// The environment of this test process.
fn this_env() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    std::env::vars_os().collect()
}

fn owned(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| (*p).to_owned()).collect()
}

#[test]
fn check_attr_text_answers_outside_any_repository() {
    let files = vec![(".gitattributes".to_owned(), "*.bin binary\n".to_owned())];
    let out = check_attr_text(&files, &owned(&["a.bin", "a.txt"]), &this_env());
    assert_eq!(
        out, "a.bin: text: unset\na.txt: text: unspecified\n",
        "the answer of the given file, from a repository of its own"
    );
}

#[test]
fn check_attr_text_ignores_the_callers_git_variables() {
    let missing = tempfile::tempdir().expect("a temporary directory");
    let mut env = this_env();
    env.push(("GIT_DIR".into(), missing.path().join("no-repo").into()));
    env.push((
        "GIT_INDEX_FILE".into(),
        missing.path().join("no-index").into(),
    ));
    env.push((
        "GIT_WORK_TREE".into(),
        missing.path().join("no-tree").into(),
    ));
    let files = vec![(".gitattributes".to_owned(), "*.bin binary\n".to_owned())];
    let out = check_attr_text(&files, &owned(&["a.bin"]), &env);
    assert_eq!(
        out, "a.bin: text: unset\n",
        "a hook's GIT_DIR, GIT_INDEX_FILE and GIT_WORK_TREE do not reach the check"
    );
}

#[test]
fn check_attr_text_reads_a_nested_file_over_the_root_one() {
    let files = vec![
        (".gitattributes".to_owned(), "*.bin binary\n".to_owned()),
        ("a/.gitattributes".to_owned(), "*.bin text\n".to_owned()),
    ];
    let out = check_attr_text(&files, &owned(&["a/x.bin", "x.bin"]), &this_env());
    assert_eq!(
        out, "a/x.bin: text: set\nx.bin: text: unset\n",
        "the deeper file wins for its own directory only"
    );
}

#[test]
fn check_attr_text_without_any_file_leaves_every_path_unspecified() {
    let out = check_attr_text(&[], &owned(&["a.bin"]), &this_env());
    assert_eq!(out, "a.bin: text: unspecified\n", "no attributes given");
}

#[test]
fn the_fixtures_without_the_root_attributes_are_refused() {
    let paths = owned(&[
        "crates/vcon-server/tests/data/formats-postgres/0.4.6/dump.sql",
        "crates/vcon-server/tests/data/formats-postgres/0.4.6/manifest.json",
    ]);
    let out = check_attr_text(&[], &paths, &this_env());
    assert_eq!(
        text_attribute_problems(&out).len(),
        2,
        "with no .gitattributes the dump is not bytes and the manifest not text: {out}"
    );
}

#[test]
fn attribute_files_collects_each_directory_on_the_way_down() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    std::fs::create_dir_all(dir.path().join("a/b")).expect("a/b");
    std::fs::write(dir.path().join(".gitattributes"), "root\n").expect("root file");
    std::fs::write(dir.path().join("a/b/.gitattributes"), "deep\n").expect("deep file");
    let found = attribute_files(dir.path(), &owned(&["a/b/c.bin", "a/b/d.bin", "e.bin"]));
    assert_eq!(
        found,
        vec![
            (".gitattributes".to_owned(), "root\n".to_owned()),
            ("a/b/.gitattributes".to_owned(), "deep\n".to_owned()),
        ],
        "each file once, shallowest first, a/ has none"
    );
}

#[test]
fn attribute_files_finds_none_where_there_is_none() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    assert_eq!(
        attribute_files(dir.path(), &owned(&["a/b.bin"])),
        Vec::<(String, String)>::new(),
        "no .gitattributes anywhere"
    );
}

#[test]
fn attribute_files_of_the_fixtures_hold_the_root_file() {
    let found = attribute_files(
        &root(),
        &owned(&["crates/vcon-server/tests/data/formats-postgres/0.4.6/dump.sql"]),
    );
    assert!(
        found.iter().any(|(path, text)| path == ".gitattributes"
            && text.contains("crates/vcon-server/tests/data/formats-postgres/** binary")),
        "the repository's own .gitattributes is read: {found:?}"
    );
}

#[test]
fn git_reads_every_fixture_as_bytes_but_the_release_manifests() {
    let mut paths = Vec::new();
    for (dir, name) in [
        (formats_dir(), "formats"),
        (pg_formats_dir(), "formats-postgres"),
    ] {
        for (key, _) in bucket_files(&dir) {
            paths.push(format!("crates/vcon-server/tests/data/{name}/{key}"));
        }
    }
    assert!(
        paths.len() > 100,
        "the fixtures were found: {}",
        paths.len()
    );
    let text = check_attr_text(&attribute_files(&root(), &paths), &paths, &this_env());
    assert_eq!(
        text.lines().count(),
        paths.len(),
        "one answer per fixture file"
    );
    assert_eq!(
        text_attribute_problems(&text),
        Vec::<String>::new(),
        ".gitattributes"
    );
}

#[test]
fn a_release_manifest_is_text() {
    assert!(
        git_reads_as_text("0.4.6/manifest.json"),
        "the release's own"
    );
    assert!(
        git_reads_as_text("0.4.10/manifest.json"),
        "a two-digit part"
    );
}

#[test]
fn keys_dumps_locks_and_bucket_objects_are_bytes() {
    for rel in [
        "0.4.6/master.key",
        "0.4.6/dump.sql",
        "0.4.6/data/lock",
        "0.4.6/data/lock.commands",
        "0.4.6/bucket/vcon-server.claim",
        "0.4.6/bucket/tenants/t/objects/a",
        "0.4.6/bucket/tenants/t/parquet/date=2026-11-17/part-a.parquet",
    ] {
        assert!(!git_reads_as_text(rel), "{rel}");
    }
}

#[test]
fn set_and_unset_where_each_belongs_has_no_problem() {
    let out = "crates/vcon-server/tests/data/formats/0.4.8/manifest.json: text: set\n\
               crates/vcon-server/tests/data/formats/0.4.8/master.key: text: unset\n\
               crates/vcon-server/tests/data/formats-postgres/0.4.6/dump.sql: text: unset\n";
    assert_eq!(
        text_attribute_problems(out),
        Vec::<String>::new(),
        "as .gitattributes says"
    );
}

#[test]
fn an_empty_answer_has_no_problem() {
    assert!(text_attribute_problems("").is_empty(), "no paths asked");
}

#[test]
fn a_dump_git_would_read_as_text_is_refused() {
    let out = "crates/vcon-server/tests/data/formats-postgres/0.4.6/dump.sql: text: unspecified\n";
    assert_eq!(
        text_attribute_problems(out),
        [
            "crates/vcon-server/tests/data/formats-postgres/0.4.6/dump.sql: git must treat it as bytes \
             (text: unspecified)"
        ],
        "the case F1b's .gitattributes line fixes"
    );
}

#[test]
fn a_release_manifest_marked_binary_is_refused() {
    let out = "crates/vcon-server/tests/data/formats/0.4.8/manifest.json: text: unset\n";
    assert_eq!(
        text_attribute_problems(out),
        [
            "crates/vcon-server/tests/data/formats/0.4.8/manifest.json: git must read it as text (text: unset)"
        ],
        "reviewers read it"
    );
}

#[test]
fn a_release_manifest_left_unspecified_is_refused() {
    let out =
        "crates/vcon-server/tests/data/formats-postgres/0.4.6/manifest.json: text: unspecified\n";
    assert_eq!(text_attribute_problems(out).len(), 1, "it must be set");
}

#[test]
fn a_line_check_attr_would_not_write_is_refused() {
    let out = "crates/vcon-server/tests/data/formats/0.4.8/master.key\n";
    assert_eq!(
        text_attribute_problems(out),
        ["`crates/vcon-server/tests/data/formats/0.4.8/master.key`: not a check-attr line"],
        "named, not skipped"
    );
}

#[test]
fn a_path_outside_the_fixtures_is_refused() {
    let out = "README.md: text: unspecified\n";
    assert_eq!(
        text_attribute_problems(out),
        ["README.md: not under a formats directory"],
        "only fixtures are asked"
    );
}

#[test]
fn a_bucket_manifest_object_is_bytes() {
    assert!(
        !git_reads_as_text("0.4.6/bucket/tenants/t/manifest/00000000000000000004.json"),
        "a stored object, though it is named .json"
    );
}

#[test]
fn a_manifest_below_a_release_directory_is_bytes() {
    assert!(
        !git_reads_as_text("0.4.6/data/manifest.json"),
        "one level down"
    );
    assert!(!git_reads_as_text("manifest.json"), "no release directory");
    // `*/manifest.json` in .gitattributes names that path only, not the
    // files of a directory so named.
    assert!(
        !git_reads_as_text("0.4.6/manifest.json/x"),
        "a file under a directory named manifest.json"
    );
}

#[test]
fn a_path_with_colons_is_split_at_the_attribute() {
    let out = "crates/vcon-server/tests/data/formats-postgres/0.4.6/bucket/a:b: text: unset\n";
    assert_eq!(
        text_attribute_problems(out),
        Vec::<String>::new(),
        "the path keeps its colon"
    );
}

// Each release, without a database.

fn pg_the_fixture_was_written_by_the_pinned_release(release: &str) {
    written_by_the_pinned_release(&pg_manifest(release), release);
}

fn the_page_states_the_postgres_schema_the_release_wrote(release: &str) {
    let releases = page_releases(&page()).expect("the release table");
    assert_eq!(
        releases[release]["postgres"],
        pg_schema(release).to_string(),
        "{PAGE}: the postgres schema {release} wrote"
    );
}

fn every_object_in_the_bucket_starts_with_the_envelope_version(release: &str) {
    let releases = page_releases(&page()).expect("the release table");
    let stated: u8 = releases[release]["envelope"].parse().expect("a number");
    let objects: Vec<(String, PathBuf)> =
        bucket_files(&pg_formats_dir().join(release).join("bucket"))
            .into_iter()
            .filter(|(key, _)| key_kind(key) == Some("objects"))
            .collect();
    assert!(!objects.is_empty(), "{release}: the bucket holds objects");
    for (key, path) in objects {
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(
            bytes.first(),
            Some(&stated),
            "{release}: {key} opens with the envelope's version"
        );
    }
}

fn every_parquet_manifest_states_the_format_the_page_states(release: &str) {
    let releases = page_releases(&page()).expect("the release table");
    let stated = &releases[release]["parquet-storage"];
    let manifests: Vec<(String, PathBuf)> =
        bucket_files(&pg_formats_dir().join(release).join("bucket"))
            .into_iter()
            .filter(|(key, _)| key_kind(key) == Some("manifest"))
            .collect();
    assert_eq!(manifests.len(), 2, "{release}: one manifest per tenant");
    for (key, path) in manifests {
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(
            manifest_format(&text).as_ref(),
            Ok(stated),
            "{release}: {key} states the format the page does"
        );
    }
}

fn every_parquet_file_is_a_parquet_file(release: &str) {
    let files: Vec<(String, PathBuf)> =
        bucket_files(&pg_formats_dir().join(release).join("bucket"))
            .into_iter()
            .filter(|(key, _)| key_kind(key) == Some("parquet"))
            .collect();
    assert_eq!(files.len(), 2, "{release}: one pack file per tenant");
    for (key, path) in files {
        let bytes = std::fs::read(&path).expect("read");
        assert!(
            bytes.len() > 8 && bytes.starts_with(b"PAR1") && bytes.ends_with(b"PAR1"),
            "{release}: {key} starts and ends with Parquet's magic"
        );
    }
}

fn the_dump_loads_as_one_query(release: &str) {
    let dump =
        std::fs::read_to_string(pg_formats_dir().join(release).join("dump.sql")).expect("the dump");
    assert_eq!(
        meta_lines(&dump),
        Vec::<usize>::new(),
        "{release}: no line a server would refuse"
    );
    assert!(
        dump.contains("CREATE TABLE vcon.schema_version"),
        "{release}: the dump holds the vCon schema"
    );
}

fn the_manifest_names_packed_versions_and_one_object_per_object_version(release: &str) {
    let manifest = pg_manifest(release);
    let count = |place: &str| {
        manifest
            .rows
            .iter()
            .filter(|r| r.place.as_deref() == Some(place))
            .count()
    };
    assert_eq!(
        count("packed") + count("object"),
        manifest.rows.len(),
        "{release}: every version has a place"
    );
    assert!(count("packed") > 0, "{release}: packed versions");
    let objects = bucket_files(&pg_formats_dir().join(release).join("bucket"))
        .into_iter()
        .filter(|(key, _)| key_kind(key) == Some("objects"))
        .count();
    assert_eq!(
        objects,
        count("object"),
        "{release}: one object in the bucket for each version kept as one"
    );
}

/// With the catalogs in PostgreSQL: against the servers
/// `scripts/pg-test-server.sh` starts (`VCON_TEST_PG_*`) and the
/// S3-compatible service of `VCON_TEST_S3_*`.
#[cfg(all(feature = "backend-tests", feature = "postgres"))]
mod pg;

#[cfg(all(feature = "backend-tests", feature = "postgres"))]
#[path = "support/joined.rs"]
mod joined;

#[cfg(all(feature = "backend-tests", feature = "postgres"))]
mod live {
    use super::joined::Joined;
    use super::*;
    use std::ffi::OsStr;
    use std::process::{Child, Stdio};
    use std::time::{Duration, Instant};

    /// How long a server may take to say where it listens, or to stop.
    const DEADLINE: Duration = Duration::from_secs(30);

    /// curl, signing its request for S3 with the test service's keys, with
    /// `extra` before the URL.
    fn s3(method: &str, path: &str, extra: &[&OsStr]) -> Output {
        Command::new("curl")
            .args(["-sS", "--fail-with-body", "-X", method])
            .args(["--aws-sigv4", "aws:amz:us-east-1:s3", "--user"])
            .arg(format!(
                "{}:{}",
                pg::var("VCON_TEST_S3_ACCESS_KEY"),
                pg::var("VCON_TEST_S3_SECRET_KEY")
            ))
            .args(extra)
            .arg(format!("{}/{path}", pg::var("VCON_TEST_S3_ENDPOINT")))
            .output()
            .expect("runs curl")
    }

    fn s3_ok(method: &str, path: &str, extra: &[&OsStr]) -> String {
        let out = s3(method, path, extra);
        assert!(out.status.success(), "{method} {path}: {out:?}");
        String::from_utf8(out.stdout).expect("UTF-8")
    }

    /// The columns a migration added to a table that releases already had,
    /// each with the schema version that added it: a release's rows are
    /// compared without the columns added after its schema, which its
    /// rows did not have, and with every other column.
    const ADDED_COLUMNS: &[(&str, &str, i64)] = &[("tenant_lock", "removing", 14)];

    /// A release's deployment restored: its dump in a database of the
    /// test's own, its bucket's objects in a bucket of the test's own, its
    /// data directory and master key in a temporary directory, with a
    /// configuration naming them.
    struct Restored {
        release: String,
        dir: tempfile::TempDir,
        db: pg::Db,
        bucket: String,
        manifest: Manifest,
    }

    impl Restored {
        /// The release's fixture restored, and brought to this build's
        /// schema by `migrate` when the release wrote an older one, as the
        /// how-to's upgrade step does.
        fn new(release: &str) -> Restored {
            let restored = Restored::as_written(release);
            if super::pg_schema(release) < i64::from(vcon_server::formats::POSTGRES_SCHEMA) {
                let out = restored.migrate();
                assert!(
                    out.status.success(),
                    "{release}: migrate brings the schema up: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            restored
        }

        /// `migrate`, run as the how-to runs it.
        fn migrate(&self) -> std::process::Output {
            self.command(&["migrate"], "vcon-migrate.toml")
                .output()
                .expect("migrate runs")
        }

        /// The release's fixture restored as the release wrote it.
        fn as_written(release: &str) -> Restored {
            let from = pg_formats_dir().join(release);
            let db = pg::Db::fresh();
            let dump = std::fs::read_to_string(from.join("dump.sql")).expect("the dump");
            db.as_owner(&dump);
            let bucket = format!("vcon-fmt-{}", &TenantId::generate().to_string()[..12]);
            s3_ok("PUT", &bucket, &[]);
            for (key, path) in bucket_files(&from.join("bucket")) {
                s3_ok(
                    "PUT",
                    &format!("{bucket}/{key}"),
                    &[OsStr::new("-T"), path.as_os_str()],
                );
            }
            let dir = tempfile::tempdir().expect("a temporary directory");
            copy_tree(&from.join("data"), &dir.path().join("data"));
            let key = dir.path().join("master.key");
            std::fs::copy(from.join("master.key"), &key).expect("the master key");
            chmod(&key, 0o600);
            let password = dir.path().join("pg.password");
            std::fs::copy(pg::dir().join("password"), &password).expect("the password");
            chmod(&password, 0o600);
            let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
                .expect("a certificate");
            std::fs::write(dir.path().join("cert.pem"), made.cert.pem()).expect("write");
            let tls = dir.path().join("key.pem");
            std::fs::write(&tls, made.signing_key.serialize_pem()).expect("write");
            chmod(&tls, 0o600);
            let restored = Restored {
                release: release.to_owned(),
                dir,
                db,
                bucket,
                manifest: pg_manifest(release),
            };
            restored.configure(true);
            restored
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        /// Writes the server's configuration, with the [storage.parquet]
        /// table when `parquet`, and `migrate`'s.
        fn configure(&self, parquet: bool) {
            let text = |user: &str| {
                let mut text = format!(
                    "listen = \"127.0.0.1:0\"\ndata_dir = {:?}\nmaster_key = {:?}\n\n[tls]\ncert = \
                     {:?}\nkey = {:?}\n\n[storage]\nobjects = \"s3\"\n\n[storage.s3]\nendpoint = \
                     \"{}\"\nregion = \"us-east-1\"\nbucket = \"{}\"\npath_style = true\nallow_http = \
                     true\n",
                    self.path("data"),
                    self.path("master.key"),
                    self.path("cert.pem"),
                    self.path("key.pem"),
                    pg::var("VCON_TEST_S3_ENDPOINT"),
                    self.bucket
                );
                if parquet {
                    text.push_str("\n[storage.parquet]\npack_interval = 3600\n");
                }
                text.push_str(&format!(
                    "\n[catalog]\nbackend = \"postgres\"\n\n[catalog.postgres]\nhosts = \
                     [\"localhost\"]\nport = {}\ndbname = \"{}\"\nuser = \"{user}\"\nca_file = \
                     {:?}\npassword_file = {:?}\n",
                    pg::var("VCON_TEST_PG_PORT"),
                    self.db.name,
                    pg::dir().join("ca.pem"),
                    self.path("pg.password")
                ));
                text
            };
            std::fs::write(self.path("vcon-server.toml"), text("vcon_app")).expect("write");
            std::fs::write(self.path("vcon-migrate.toml"), text("vcon_migrate")).expect("write");
        }

        /// The binary with `args` on the server's configuration (or
        /// `config`), with the bucket's keys in the environment only.
        fn command(&self, args: &[&str], config: &str) -> Command {
            let mut command = Command::new(env!("CARGO_BIN_EXE_vcon-server"));
            command.args(args).arg("--config").arg(self.path(config));
            for name in ["AWS_PROFILE", "AWS_SESSION_TOKEN", "AWS_CONFIG_FILE"] {
                command.env_remove(name);
            }
            command
                .env("AWS_ACCESS_KEY_ID", pg::var("VCON_TEST_S3_ACCESS_KEY"))
                .env("AWS_SECRET_ACCESS_KEY", pg::var("VCON_TEST_S3_SECRET_KEY"))
                .env("AWS_SHARED_CREDENTIALS_FILE", "/nonexistent")
                .env("AWS_EC2_METADATA_DISABLED", "true");
            command
        }

        fn run(&self, args: &[&str]) -> Output {
            self.command(args, "vcon-server.toml")
                .output()
                .expect("the binary runs")
        }

        /// The JSON lines a command that must succeed prints. What it
        /// printed is not shown when it fails: `key create` prints a token.
        fn ok(&self, args: &[&str]) -> Vec<serde_json::Value> {
            let out = self.run(args);
            assert!(
                out.status.success(),
                "{}: {args:?}: {}",
                self.release,
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8(out.stdout)
                .expect("UTF-8")
                .lines()
                .map(|l| serde_json::from_str(l).expect("a JSON line"))
                .collect()
        }

        fn checked(&self) -> Vec<serde_json::Value> {
            self.ok(&["check"])
        }

        /// What a command that is refused says on stderr.
        fn refusal(&self, args: &[&str]) -> String {
            let out = self.run(args);
            assert!(
                !out.status.success(),
                "{}: {args:?} is refused",
                self.release
            );
            String::from_utf8_lossy(&out.stderr).into_owned()
        }

        fn tenant_id(&self, name: &str) -> String {
            self.ok(&["tenant", "list"])
                .into_iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("{}: tenant {name} is there", self.release))["id"]
                .as_str()
                .expect("an id")
                .to_owned()
        }

        /// A new key of tenant `name` with `scopes`, its token in a header
        /// file, so that no command line holds it; the file's path.
        fn key(&self, name: &str, scopes: &[&str]) -> PathBuf {
            let id = self.tenant_id(name);
            let mut args = vec!["key", "create", "--tenant", &id, "--label", "formats-test"];
            for scope in scopes {
                args.extend(["--scope", scope]);
            }
            let token = self.ok(&args)[0]["token"]
                .as_str()
                .expect("a token")
                .to_owned();
            let path = self.path(&format!("{name}.header"));
            std::fs::write(&path, format!("Authorization: Bearer {token}\n")).expect("write");
            chmod(&path, 0o600);
            path
        }

        /// Starts `serve` and waits for its `serving` line.
        fn serve(&self) -> Server {
            let mut child = self
                .command(&["serve"], "vcon-server.toml")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("serve starts");
            let stderr = child.stderr.take().expect("piped");
            let (tx, lines) = std::sync::mpsc::channel();
            let reader = Joined::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(stderr)
                    .lines()
                    .map_while(Result::ok)
                {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
            let deadline = Instant::now() + DEADLINE;
            let mut early = Vec::new();
            let port = loop {
                match lines.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(line) => {
                        let bound = line
                            .split_whitespace()
                            .skip_while(|f| *f != "serving")
                            .find_map(|f| f.strip_prefix("listen="))
                            .and_then(|a| a.parse::<std::net::SocketAddr>().ok());
                        early.push(line);
                        if let Some(addr) = bound {
                            break addr.port();
                        }
                    }
                    Err(_) => panic!(
                        "{}: serve never said where it listens: {early:?}",
                        self.release
                    ),
                }
            };
            Server {
                child,
                port,
                cert: self.path("cert.pem"),
                out: self.path("read.body"),
                lines,
                log: std::cell::RefCell::new(early),
                _reader: reader,
            }
        }

        /// What `serve` says when it refuses to start.
        fn serve_refusal(&self) -> String {
            let mut child = self
                .command(&["serve"], "vcon-server.toml")
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("serve starts");
            let deadline = Instant::now() + DEADLINE;
            let status = loop {
                if let Some(status) = child.try_wait().expect("wait") {
                    break status;
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{}: serve did not refuse to start", self.release);
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            let mut said = String::new();
            use std::io::Read;
            child
                .stderr
                .take()
                .expect("piped")
                .read_to_string(&mut said)
                .expect("read");
            assert!(!status.success(), "{}: serve refused: {said}", self.release);
            said
        }

        /// What can change: each table's rows (read as the superuser, past
        /// row-level security), the bucket's listing and the data
        /// directory's files.
        fn state(&self) -> (Vec<String>, Vec<String>, BTreeMap<PathBuf, Vec<u8>>) {
            let tables = pg::superuser(
                &self.db.name,
                "SELECT tablename FROM pg_tables WHERE schemaname = 'vcon' ORDER BY tablename",
            )
            .expect("the tables");
            assert!(tables.len() > 10, "{}: the schema's tables", self.release);
            let schema = super::pg_schema(&self.release);
            let rows = tables
                .iter()
                .map(|t| {
                    let skipped: Vec<String> = ADDED_COLUMNS
                        .iter()
                        .filter(|(table, _, added)| *table == t.as_str() && *added > schema)
                        .map(|(_, column, _)| format!("'{column}'"))
                        .collect();
                    let columns = pg::superuser(
                        &self.db.name,
                        &format!(
                            "SELECT string_agg(quote_ident(column_name), ',' \
                             ORDER BY ordinal_position) FROM information_schema.columns \
                             WHERE table_schema = 'vcon' AND table_name = '{t}' \
                             AND column_name NOT IN ({})",
                            if skipped.is_empty() {
                                "''".to_owned()
                            } else {
                                skipped.join(",")
                            }
                        ),
                    )
                    .expect("the columns")
                    .concat();
                    let sum = pg::superuser(
                        &self.db.name,
                        &format!(
                            "SELECT md5(coalesce(string_agg(x, E'\\n' ORDER BY x), '')) \
                             FROM (SELECT ROW({columns})::text AS x FROM vcon.{t}) r"
                        ),
                    )
                    .expect("the rows");
                    format!("{t} {}", sum.concat())
                })
                .collect();
            let listing =
                listing_entries(&s3_ok("GET", &format!("{}?list-type=2", self.bucket), &[]));
            let mut files = BTreeMap::new();
            for entry in std::fs::read_dir(self.path("data")).expect("read") {
                let path = entry.expect("an entry").path();
                let bytes = std::fs::read(&path).expect("read");
                files.insert(path, digest(&SHA256, &bytes).as_ref().to_vec());
            }
            (rows, listing, files)
        }

        /// The keys of the bucket that hold one version each.
        fn objects(&self) -> Vec<String> {
            listing_entries(&s3_ok("GET", &format!("{}?list-type=2", self.bucket), &[]))
                .iter()
                .map(|e| entry_key(e).expect("an entry names its key").to_owned())
                .filter(|k| key_kind(k) == Some("objects"))
                .collect()
        }

        fn rows(&self, tenant: &str) -> Vec<&Row> {
            self.manifest
                .rows
                .iter()
                .filter(|r| r.tenant == tenant)
                .collect()
        }
    }

    /// A running server, killed when dropped, so a failed assertion leaves
    /// no server behind.
    struct Server {
        child: Child,
        port: u16,
        cert: PathBuf,
        out: PathBuf,
        lines: std::sync::mpsc::Receiver<String>,
        /// Every line the server wrote to stderr so far.
        log: std::cell::RefCell<Vec<String>>,
        /// The thread that reads the server's stderr: the last field, so
        /// it is joined after the drop has killed the server, which ends
        /// the pipe.
        _reader: Joined<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            if matches!(self.child.try_wait(), Ok(None)) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    impl Server {
        /// The server's stderr so far, for a failure's message.
        fn log(&self) -> String {
            let mut log = self.log.borrow_mut();
            log.extend(self.lines.try_iter());
            log.join("\n")
        }

        /// GET `path` with the key in the header file `key`: the status and
        /// the body.
        fn get(&self, path: &str, key: &Path) -> (u16, Vec<u8>) {
            let _ = std::fs::remove_file(&self.out);
            let out = Command::new("curl")
                .args(["-sS", "--max-time", "20", "--cacert"])
                .arg(&self.cert)
                .arg("--resolve")
                .arg(format!("localhost:{}:127.0.0.1", self.port))
                .arg("-H")
                .arg(format!("@{}", key.display()))
                .arg("-o")
                .arg(&self.out)
                .args(["-w", "%{http_code}"])
                .arg(format!("https://localhost:{}{path}", self.port))
                .output()
                .expect("runs curl");
            // The message does not name `path`: it holds a vCon's uuid, a
            // value the catalog seals, which is not written out
            // (rust/cleartext-logging; scripts/check_style.py uuid-in-message
            // sees a uuid only under its own name).
            assert!(
                out.status.success(),
                "GET: curl failed: {out:?}\n{}",
                self.log()
            );
            let status = String::from_utf8_lossy(&out.stdout)
                .parse()
                .expect("a status");
            (status, std::fs::read(&self.out).unwrap_or_default())
        }

        /// POST `body` as a vCon to /vcons: the status.
        fn post(&self, body: &[u8], key: &Path) -> u16 {
            std::fs::write(&self.out, body).expect("write");
            let out = Command::new("curl")
                .args(["-sS", "--max-time", "20", "-o", "/dev/null", "--cacert"])
                .arg(&self.cert)
                .arg("--resolve")
                .arg(format!("localhost:{}:127.0.0.1", self.port))
                .arg("-H")
                .arg(format!("@{}", key.display()))
                .args(["-H", "Content-Type: application/vcon", "--data-binary"])
                .arg(format!("@{}", self.out.display()))
                .args(["-w", "%{http_code}"])
                .arg(format!("https://localhost:{}/vcons", self.port))
                .output()
                .expect("runs curl");
            assert!(out.status.success(), "POST /vcons: {out:?}\n{}", self.log());
            String::from_utf8_lossy(&out.stdout)
                .parse()
                .expect("a status")
        }

        /// Sends SIGTERM and waits for the server to end; its exit status.
        fn stop(mut self) -> std::process::ExitStatus {
            let pid = libc::pid_t::try_from(self.child.id()).expect("a pid");
            // SAFETY: the pid is our running child's.
            let sent = unsafe { libc::kill(pid, libc::SIGTERM) };
            assert_eq!(sent, 0, "SIGTERM is sent to the server");
            let deadline = Instant::now() + DEADLINE;
            loop {
                if let Some(status) = self.child.try_wait().expect("wait") {
                    return status;
                }
                assert!(Instant::now() < deadline, "serve did not stop on SIGTERM");
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    // Success.

    pub(super) fn check_passes_and_counts_every_kept_version(release: &str) {
        let r = Restored::new(release);
        let lines = r.checked();
        for tenant in ["tenant-a", "tenant-b"] {
            let line = lines
                .iter()
                .find(|l| l["name"] == tenant)
                .unwrap_or_else(|| panic!("{release}: check reports {tenant}: {lines:?}"));
            let rows = r.rows(tenant);
            let packed = rows
                .iter()
                .filter(|row| row.place.as_deref() == Some("packed"))
                .count();
            assert_eq!(
                line["versions"].as_u64(),
                Some(u64::try_from(rows.len()).expect("small")),
                "{release}: check counts {tenant}'s kept versions"
            );
            assert_eq!(
                line["packed"].as_u64(),
                Some(u64::try_from(packed).expect("small")),
                "{release}: check counts {tenant}'s packed versions"
            );
            assert_eq!(
                line["damaged"],
                serde_json::json!([]),
                "{release}: nothing damaged"
            );
            assert_eq!(
                line["problems"],
                serde_json::json!([]),
                "{release}: no problem"
            );
        }
    }

    pub(super) fn every_kept_version_reads_back_byte_equal(release: &str) {
        let r = Restored::new(release);
        let keys = [
            ("tenant-a", r.key("tenant-a", &["legal"])),
            ("tenant-b", r.key("tenant-b", &["legal"])),
        ];
        let server = r.serve();
        for (tenant, key) in &keys {
            for row in r.rows(tenant) {
                let (status, body) = server.get(
                    &format!("/legal/vcons/{}/versions/{}", row.uuid, row.version),
                    key,
                );
                assert_eq!(
                    status, 200,
                    "{release}: {tenant} version {} reads",
                    row.version
                );
                assert_eq!(
                    sha_hex(&body),
                    row.sha256,
                    "{release}: {tenant} version {} ({:?}) is the body the release stored",
                    row.version,
                    row.place
                );
            }
        }
        assert!(server.stop().success(), "{release}: serve stops cleanly");
    }

    pub(super) fn every_uuid_lists_its_kept_versions_in_order(release: &str) {
        let r = Restored::new(release);
        let keys = [
            ("tenant-a", r.key("tenant-a", &["legal"])),
            ("tenant-b", r.key("tenant-b", &["legal"])),
        ];
        let server = r.serve();
        for (tenant, key) in &keys {
            let mut by_uuid: BTreeMap<uuid::Uuid, Vec<&Row>> = BTreeMap::new();
            for row in r.rows(tenant) {
                by_uuid.entry(row.uuid).or_default().push(row);
            }
            // A vCon is named by its place in the uuid order, not its uuid.
            for (n, (uuid, rows)) in by_uuid.iter_mut().enumerate() {
                rows.sort_by_key(|row| row.order);
                let (status, body) = server.get(&format!("/legal/vcons/{uuid}/versions"), key);
                assert_eq!(
                    status, 200,
                    "{release}: {tenant} vCon {n} lists its versions"
                );
                let listed: Vec<serde_json::Value> =
                    serde_json::from_slice(&body).expect("a JSON array");
                let ids: Vec<String> = listed
                    .iter()
                    .map(|v| v["version"].as_str().expect("a version").to_owned())
                    .collect();
                let want: Vec<String> = rows.iter().map(|row| row.version.to_string()).collect();
                assert_eq!(
                    ids, want,
                    "{release}: {tenant} vCon {n}: these versions, oldest first"
                );
                let current: Vec<bool> = listed
                    .iter()
                    .map(|v| v["current"].as_bool().expect("current"))
                    .collect();
                let want: Vec<bool> = rows
                    .iter()
                    .map(|row| row.state.starts_with("current"))
                    .collect();
                assert_eq!(
                    current, want,
                    "{release}: {tenant} vCon {n}: the release's current version"
                );
            }
        }
        assert!(server.stop().success(), "{release}: serve stops cleanly");
    }

    pub(super) fn migrate_finds_the_schema_current_and_changes_nothing(release: &str) {
        let r = Restored::new(release);
        let before = r.state();
        let out = r
            .command(&["migrate"], "vcon-migrate.toml")
            .output()
            .expect("migrate runs");
        assert!(
            out.status.success(),
            "{release}: migrate passes: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let said: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a JSON line");
        let schema = i64::from(vcon_server::formats::POSTGRES_SCHEMA);
        assert_eq!(
            (said["from"].as_i64(), said["to"].as_i64()),
            (Some(schema), Some(schema)),
            "{release}: from and to this build's schema: {said}"
        );
        assert_eq!(r.state(), before, "{release}: migrate wrote nothing");
    }

    /// The tables' rows of `state`, without the tables this build's
    /// migrations added after the release's schema and without
    /// `schema_version`, where each migration records itself: what a
    /// migration must leave as it was.
    fn rows_but_new(rows: &[String], release: &str) -> Vec<String> {
        let schema = super::pg_schema(release);
        let mut new = super::tables_made_after(schema);
        if schema < i64::from(vcon_server::formats::POSTGRES_SCHEMA) {
            new.push("schema_version");
        }
        rows.iter()
            .filter(|row| !new.iter().any(|t| row.split(' ').next() == Some(*t)))
            .cloned()
            .collect()
    }

    pub(super) fn migrate_brings_the_release_s_schema_to_this_build_s_and_keeps_every_row(
        release: &str,
    ) {
        let r = Restored::as_written(release);
        let (rows, listing, files) = r.state();
        let out = r.migrate();
        assert!(
            out.status.success(),
            "{release}: migrate passes: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let said: serde_json::Value = serde_json::from_slice(&out.stdout).expect("a JSON line");
        let (from, to) = (
            super::pg_schema(release),
            i64::from(vcon_server::formats::POSTGRES_SCHEMA),
        );
        assert_eq!(
            (
                said["from"].as_i64(),
                said["to"].as_i64(),
                said["migrated"].as_bool()
            ),
            (Some(from), Some(to), Some(from < to)),
            "{release}: from the release's schema to this build's: {said}"
        );
        let (after_rows, after_listing, after_files) = r.state();
        assert_eq!(
            rows_but_new(&after_rows, release),
            rows_but_new(&rows, release),
            "{release}: every row of every table the release made is as it was"
        );
        assert_eq!(
            pg::superuser(
                &r.db.name,
                "SELECT count(*) FROM vcon.tenant_lock WHERE removing"
            )
            .expect("the marks")
            .concat(),
            "0",
            "{release}: no tenant is marked as being removed after the migration"
        );
        assert_eq!(
            (after_listing, after_files),
            (listing, files),
            "{release}: the bucket and the data directory are as they were"
        );
        assert_eq!(
            pg::superuser(&r.db.name, "SELECT max(version) FROM vcon.schema_version")
                .expect("the version"),
            [to.to_string()],
            "{release}: the schema records this build's version"
        );
        for table in super::tables_made_after(from) {
            let empty = format!("{table} d41d8cd98f00b204e9800998ecf8427e");
            assert!(
                after_rows.contains(&empty),
                "{release}: the table {table} is there, and empty: {after_rows:?}"
            );
        }
    }

    pub(super) fn a_migrate_that_fails_leaves_the_release_s_schema_whole(release: &str) {
        let r = Restored::as_written(release);
        let before = r.state();
        let build = i64::from(vcon_server::formats::POSTGRES_SCHEMA);
        if !super::a_migration_is_left(super::pg_schema(release), build) {
            // A release at the build's schema has the last migration's
            // table already and no migration left to fail: migrate applies
            // nothing and changes nothing. The failure case runs on every
            // older fixture (a_fixture_one_schema_below_the_builds_runs_the_
            // failed_migration_case holds that one exists).
            let out = r.migrate();
            assert!(
                out.status.success(),
                "{release}: migrate at the build's schema succeeds: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                r.state(),
                before,
                "{release}: migrate with nothing to apply changes nothing"
            );
            return;
        }
        let version = || {
            pg::superuser(&r.db.name, "SELECT max(version) FROM vcon.schema_version")
                .expect("the version")
                .concat()
        };
        // A table in the way of the last migration's CREATE TABLE, so that
        // migration fails. Each migration runs in a transaction of its own
        // (backend/postgres/schema.rs), so the ones before it are kept and
        // the database is whole at the version before the build's.
        let planted = super::last_migration_table(build)
            .unwrap_or_else(|| panic!("schema {build}'s migration makes a table (LATER_TABLES)"));
        r.db.as_owner(&format!("CREATE TABLE vcon.{planted} (planted int)"));
        let out = r.migrate();
        assert!(
            !out.status.success(),
            "{release}: migrate fails on the planted table: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(
            version(),
            (build - 1).to_string(),
            "{release}: the schema's version is the one before the failed migration"
        );
        r.db.as_owner(&format!("DROP TABLE vcon.{planted}"));
        let (rows, listing, files) = r.state();
        assert_eq!(
            (rows_but_new(&rows, release), listing.clone(), files.clone()),
            (
                rows_but_new(&before.0, release),
                before.1.clone(),
                before.2.clone()
            ),
            "{release}: no row of the release's tables changed"
        );
        // Run again with the table out of the way, migrate starts from the
        // version the failure left and finishes.
        let again = r.migrate();
        assert!(
            again.status.success(),
            "{release}: a second migrate passes: {}",
            String::from_utf8_lossy(&again.stderr)
        );
        let said: serde_json::Value = serde_json::from_slice(&again.stdout).expect("a JSON line");
        assert_eq!(
            (said["from"].as_i64(), said["to"].as_i64()),
            (Some(build - 1), Some(build)),
            "{release}: from the version the failure left to this build's: {said}"
        );
        assert_eq!(
            version(),
            build.to_string(),
            "{release}: the schema is at this build's version"
        );
        let (rows, after_listing, after_files) = r.state();
        assert_eq!(
            (rows_but_new(&rows, release), after_listing, after_files),
            (rows_but_new(&before.0, release), before.1, before.2),
            "{release}: and still no row of the release's tables changed"
        );
    }

    pub(super) fn a_second_check_changes_nothing(release: &str) {
        let r = Restored::new(release);
        r.checked();
        let first = r.state();
        r.checked();
        assert_eq!(r.state(), first, "{release}: a second check writes nothing");
    }

    pub(super) fn the_restored_deployment_stores_and_reads_a_new_vcon(release: &str) {
        let r = Restored::new(release);
        let key = r.key("tenant-a", &["write", "read"]);
        let server = r.serve();
        let uuid = "6f1c1e0a-6d4b-4c3e-9a51-0d2b6a7c8e11";
        let body = serde_json::to_vec(&serde_json::json!({
            "vcon": "0.4.0",
            "uuid": uuid,
            "created_at": "2026-11-17T12:00:00Z",
            "subject": "stored after the restore",
            "parties": [{"tel": "+15555550123", "name": "Caller Example"}],
            "dialog": [],
            "attachments": [],
            "analysis": []
        }))
        .expect("JSON");
        assert_eq!(
            server.post(&body, &key),
            201,
            "{release}: the new vCon is stored"
        );
        let (status, read) = server.get(&format!("/vcons/{uuid}"), &key);
        assert_eq!(
            (status, read),
            (200, body),
            "{release}: and reads back byte-equal"
        );
        assert!(server.stop().success(), "{release}: serve stops cleanly");
    }

    // Failure.

    pub(super) fn a_schema_one_version_past_this_build_is_refused_by_name(release: &str) {
        let r = Restored::new(release);
        let next = vcon_server::formats::POSTGRES_SCHEMA + 1;
        r.db.as_owner(&format!(
            "INSERT INTO vcon.schema_version (version) VALUES ({next})"
        ));
        let before = r.state();
        let named = format!(
            "the database schema is newer than this binary (version {next}, this binary {})",
            vcon_server::formats::POSTGRES_SCHEMA
        );
        let said = r.refusal(&["check"]);
        assert!(
            said.contains(&named),
            "{release}: check names the version: {said}"
        );
        let said = r.serve_refusal();
        assert!(
            said.contains(&named),
            "{release}: serve names the version: {said}"
        );
        assert_eq!(
            r.state(),
            before,
            "{release}: the refused deployment is left as it was"
        );
    }

    pub(super) fn a_wrong_master_key_opens_nothing_and_changes_nothing(release: &str) {
        let r = Restored::new(release);
        std::fs::write(r.path("master.key"), [0x33u8; 32]).expect("another key");
        let before = r.state();
        let said = r.refusal(&["check"]);
        assert!(
            said.contains("master key"),
            "{release}: the refusal names the key: {said}"
        );
        let said = r.serve_refusal();
        assert!(
            said.contains("master key"),
            "{release}: serve names the key: {said}"
        );
        assert_eq!(
            r.state(),
            before,
            "{release}: nothing is written under the wrong key"
        );
    }

    pub(super) fn a_purged_uuid_is_gone(release: &str) {
        let r = Restored::new(release);
        assert!(
            !r.manifest.purged.is_empty(),
            "{release}: the workload purged a uuid"
        );
        let key = r.key("tenant-b", &["legal"]);
        let server = r.serve();
        for (n, (tenant, uuid)) in r.manifest.purged.iter().enumerate() {
            assert_eq!(tenant, "tenant-b", "{release}: only the 0d tenant purges");
            let (status, _) = server.get(&format!("/legal/vcons/{uuid}/versions"), &key);
            assert_eq!(
                status, 410,
                "{release}: purged uuid {n} of the manifest stays gone"
            );
        }
        assert!(server.stop().success(), "{release}: serve stops cleanly");
    }

    pub(super) fn a_deleted_vcon_is_gone_to_an_ordinary_read(release: &str) {
        let r = Restored::new(release);
        let key = r.key("tenant-a", &["read"]);
        let server = r.serve();
        let deleted: Vec<&Row> = r
            .rows("tenant-a")
            .into_iter()
            .filter(|row| row.state.starts_with("deleted"))
            .collect();
        assert!(deleted.len() >= 2, "{release}: deleted vCons were kept");
        for (n, row) in deleted.into_iter().enumerate() {
            let (status, _) = server.get(&format!("/vcons/{}", row.uuid), &key);
            assert_eq!(
                status, 410,
                "{release}: an ordinary read of deleted vCon {n} answers gone"
            );
        }
        assert!(server.stop().success(), "{release}: serve stops cleanly");
    }

    pub(super) fn a_missing_object_is_named_damaged_by_check(release: &str) {
        let r = Restored::new(release);
        let objects = r.objects();
        let gone = objects.first().expect("an object");
        s3_ok("DELETE", &format!("{}/{gone}", r.bucket), &[]);
        let out = r.run(&["check"]);
        assert!(
            !out.status.success(),
            "{release}: check fails on a missing object"
        );
        let lines: Vec<serde_json::Value> = String::from_utf8(out.stdout)
            .expect("UTF-8")
            .lines()
            .map(|l| serde_json::from_str(l).expect("a JSON line"))
            .collect();
        let damaged: Vec<String> = lines
            .iter()
            .filter_map(|l| l["damaged"].as_array())
            .flatten()
            .map(|v| v.as_str().expect("a version").to_owned())
            .collect();
        assert_eq!(
            damaged.len(),
            1,
            "{release}: one version damaged: {lines:?}"
        );
        assert!(
            r.manifest
                .rows
                .iter()
                .any(|row| row.version.to_string() == damaged[0]
                    && row.place.as_deref() == Some("object")),
            "{release}: the damaged version is one kept as an object: {damaged:?}"
        );
    }

    pub(super) fn serve_without_the_parquet_table_is_refused(release: &str) {
        let r = Restored::new(release);
        r.configure(false);
        let before = r.state();
        let said = r.serve_refusal();
        assert!(
            said.contains(
                "versions packed into Parquet files, and the configuration has no [storage.parquet]"
            ),
            "{release}: serve names what is missing: {said}"
        );
        assert_eq!(
            r.state(),
            before,
            "{release}: the refused start wrote nothing"
        );
    }

    // Edge.

    pub(super) fn held_versions_outlive_the_start_up_sweep_under_a_0d_period(release: &str) {
        let r = Restored::new(release);
        assert!(
            r.rows("tenant-b")
                .iter()
                .any(|row| row.state.starts_with("hidden+held")),
            "{release}: a hidden held version is there to purge"
        );
        let key = r.key("tenant-b", &["legal"]);
        let server = r.serve();
        for row in r.rows("tenant-b") {
            let (status, _) = server.get(
                &format!("/legal/vcons/{}/versions/{}", row.uuid, row.version),
                &key,
            );
            assert_eq!(
                status, 200,
                "{release}: tenant-b version {} survives the sweep",
                row.version
            );
        }
        assert!(server.stop().success(), "{release}: serve stops cleanly");
    }

    pub(super) fn the_start_pack_run_packs_the_late_objects_and_each_reads_back(release: &str) {
        let r = Restored::new(release);
        assert!(
            !r.objects().is_empty(),
            "{release}: versions kept as objects"
        );
        let keys = [
            ("tenant-a", r.key("tenant-a", &["legal"])),
            ("tenant-b", r.key("tenant-b", &["legal"])),
        ];
        let server = r.serve();
        let deadline = Instant::now() + DEADLINE;
        while !r.objects().is_empty() {
            assert!(
                Instant::now() < deadline,
                "{release}: the start pack run packs every object"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        for (tenant, key) in &keys {
            for row in r
                .rows(tenant)
                .into_iter()
                .filter(|row| row.place.as_deref() == Some("object"))
            {
                let (status, body) = server.get(
                    &format!("/legal/vcons/{}/versions/{}", row.uuid, row.version),
                    key,
                );
                assert_eq!(
                    (status, sha_hex(&body)),
                    (200, row.sha256.clone()),
                    "{release}: {tenant} version {} reads back from its row",
                    row.version
                );
            }
        }
        assert!(server.stop().success(), "{release}: serve stops cleanly");
        for line in r.checked().iter().filter(|l| l["name"].is_string()) {
            assert_eq!(
                line["packed"], line["versions"],
                "{release}: every version packed: {line}"
            );
        }
    }

    pub(super) fn the_restored_schema_is_the_one_the_release_wrote(release: &str) {
        let r = Restored::as_written(release);
        let read = pg::superuser(&r.db.name, "SELECT max(version) FROM vcon.schema_version")
            .expect("the version");
        assert_eq!(
            read,
            [pg_schema(release).to_string()],
            "{release}: the dump restores the schema version the manifest records"
        );
    }
}

/// Each `<Contents>` entry of a bucket listing as its text: the key, the
/// ETag, the size and the time, which change when an object does.
fn listing_entries(xml: &str) -> Vec<String> {
    xml.split("<Contents>")
        .skip(1)
        .filter_map(|rest| rest.split_once("</Contents>").map(|(e, _)| e.to_owned()))
        .collect()
}

/// The key of a listing entry, or None when it names none.
fn entry_key(entry: &str) -> Option<&str> {
    entry
        .split_once("<Key>")
        .and_then(|(_, rest)| rest.split_once("</Key>"))
        .map(|(key, _)| key)
}

#[test]
fn a_listing_entry_names_its_key() {
    let xml = "<ListBucketResult><Contents><Key>a/b</Key><ETag>\"x\"</ETag></Contents>\
               <Contents><Key>c</Key></Contents></ListBucketResult>";
    let entries = listing_entries(xml);
    assert_eq!(entries.len(), 2, "two entries");
    assert_eq!(
        (entry_key(&entries[0]), entry_key(&entries[1])),
        (Some("a/b"), Some("c")),
        "their keys"
    );
}

#[test]
fn an_empty_listing_has_no_entry() {
    assert!(
        listing_entries("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>")
            .is_empty(),
        "none"
    );
}

#[test]
fn an_entry_without_a_key_names_none() {
    assert_eq!(entry_key("<ETag>\"x\"</ETag>"), None, "no key");
    assert_eq!(entry_key("<Key>unclosed"), None, "an unclosed key");
}

/// The tests of one release's PostgreSQL fixture: the ones that read the
/// files, and in `postgres` the ones that restore it.
macro_rules! pg_release {
    ($name:ident, $release:literal) => {
        mod $name {
            #[test]
            fn the_fixture_was_written_by_the_pinned_release() {
                super::pg_the_fixture_was_written_by_the_pinned_release($release);
            }
            #[test]
            fn the_page_states_the_postgres_schema_the_release_wrote() {
                super::the_page_states_the_postgres_schema_the_release_wrote($release);
            }
            #[test]
            fn every_object_in_the_bucket_starts_with_the_envelope_version() {
                super::every_object_in_the_bucket_starts_with_the_envelope_version($release);
            }
            #[test]
            fn every_parquet_manifest_states_the_format_the_page_states() {
                super::every_parquet_manifest_states_the_format_the_page_states($release);
            }
            #[test]
            fn every_parquet_file_is_a_parquet_file() {
                super::every_parquet_file_is_a_parquet_file($release);
            }
            #[test]
            fn the_dump_loads_as_one_query() {
                super::the_dump_loads_as_one_query($release);
            }
            #[test]
            fn the_manifest_names_packed_versions_and_one_object_per_object_version() {
                super::the_manifest_names_packed_versions_and_one_object_per_object_version(
                    $release,
                );
            }
            #[cfg(all(feature = "backend-tests", feature = "postgres"))]
            mod postgres {
                use super::super::live;
                #[test]
                fn check_passes_and_counts_every_kept_version() {
                    live::check_passes_and_counts_every_kept_version($release);
                }
                #[test]
                fn every_kept_version_reads_back_byte_equal() {
                    live::every_kept_version_reads_back_byte_equal($release);
                }
                #[test]
                fn every_uuid_lists_its_kept_versions_in_order() {
                    live::every_uuid_lists_its_kept_versions_in_order($release);
                }
                #[test]
                fn migrate_finds_the_schema_current_and_changes_nothing() {
                    live::migrate_finds_the_schema_current_and_changes_nothing($release);
                }
                #[test]
                fn migrate_brings_the_release_s_schema_to_this_build_s_and_keeps_every_row() {
                    live::migrate_brings_the_release_s_schema_to_this_build_s_and_keeps_every_row(
                        $release,
                    );
                }
                #[test]
                fn a_migrate_that_fails_leaves_the_release_s_schema_whole() {
                    live::a_migrate_that_fails_leaves_the_release_s_schema_whole($release);
                }
                #[test]
                fn a_second_check_changes_nothing() {
                    live::a_second_check_changes_nothing($release);
                }
                #[test]
                fn the_restored_deployment_stores_and_reads_a_new_vcon() {
                    live::the_restored_deployment_stores_and_reads_a_new_vcon($release);
                }
                #[test]
                fn a_schema_one_version_past_this_build_is_refused_by_name() {
                    live::a_schema_one_version_past_this_build_is_refused_by_name($release);
                }
                #[test]
                fn a_wrong_master_key_opens_nothing_and_changes_nothing() {
                    live::a_wrong_master_key_opens_nothing_and_changes_nothing($release);
                }
                #[test]
                fn a_purged_uuid_is_gone() {
                    live::a_purged_uuid_is_gone($release);
                }
                #[test]
                fn a_deleted_vcon_is_gone_to_an_ordinary_read() {
                    live::a_deleted_vcon_is_gone_to_an_ordinary_read($release);
                }
                #[test]
                fn a_missing_object_is_named_damaged_by_check() {
                    live::a_missing_object_is_named_damaged_by_check($release);
                }
                #[test]
                fn serve_without_the_parquet_table_is_refused() {
                    live::serve_without_the_parquet_table_is_refused($release);
                }
                #[test]
                fn held_versions_outlive_the_start_up_sweep_under_a_0d_period() {
                    live::held_versions_outlive_the_start_up_sweep_under_a_0d_period($release);
                }
                #[test]
                fn the_start_pack_run_packs_the_late_objects_and_each_reads_back() {
                    live::the_start_pack_run_packs_the_late_objects_and_each_reads_back($release);
                }
                #[test]
                fn the_restored_schema_is_the_one_the_release_wrote() {
                    live::the_restored_schema_is_the_one_the_release_wrote($release);
                }
            }
        }
    };
}

pg_release!(pg_release_0_4_6, "0.4.6");
pg_release!(pg_release_0_4_7, "0.4.7");
pg_release!(pg_release_0_4_8, "0.4.8");
pg_release!(pg_release_0_4_9, "0.4.9");
pg_release!(pg_release_0_4_10, "0.4.10");
pg_release!(pg_release_0_4_11, "0.4.11");
pg_release!(pg_release_0_4_12, "0.4.12");
pg_release!(pg_release_0_4_13, "0.4.13");
pg_release!(pg_release_0_4_14, "0.4.14");
pg_release!(pg_release_0_4_15, "0.4.15");
pg_release!(pg_release_0_4_16, "0.4.16");
pg_release!(pg_release_0_4_17, "0.4.17");
