//! The releases the message and the pages name, read from the release
//! fixtures. These tests need no server.
//!
//! A server of every release before schema version [`SHARED_FROM`] holds
//! the servers' lock for itself, so it serves a database alone. The
//! message a refused server prints, and the how-to, name the newest such
//! release: "a server of release X or earlier". The pages also name which
//! releases leave a database at which schema version. Each of those
//! numbers was written by hand, and one went stale when release 0.4.17
//! landed between the start of the work on schema version 15 and its
//! landing: the text said 0.4.16. The one rule here reads each number from
//! the fixtures under `tests/data/formats-postgres`, one for each release,
//! each recording the schema version that release leaves.

use std::cmp::Ordering;

use super::super::super::schema::{Holds, MIGRATIONS, SCHEMA_VERSION, SERVER_LOCK_HELD, lock_text};

/// The schema version of the first release whose servers share the
/// servers' lock: its migration adds the table of the leases the servers
/// hand the purge sweep and the pack run with.
const SHARED_FROM: i32 = 15;

/// A release's version, as three numbers.
type Version = (u32, u32, u32);

/// `text` as a version, if it is three numbers with a dot between each.
fn version(text: &str) -> Option<Version> {
    let mut parts = text.split('.');
    let (Some(a), Some(b), Some(c), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let number = |p: &str| {
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    };
    Some((number(a)?, number(b)?, number(c)?))
}

/// The releases, each with the schema version it leaves, oldest first.
fn sorted(mut releases: Vec<(String, i32)>) -> Vec<(String, i32)> {
    releases.sort_by(|a, b| match (version(&a.0), version(&b.0)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => Ordering::Equal,
    });
    releases
}

/// Each release fixture of the PostgreSQL catalog: its release and the
/// schema version that release leaves, from its `manifest.json`.
fn fixtures() -> Vec<(String, i32)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/formats-postgres");
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut found = Vec::new();
    for entry in entries {
        let path = entry
            .expect("a directory entry")
            .path()
            .join("manifest.json");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let manifest: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let release = manifest["release"]
            .as_str()
            .unwrap_or_else(|| panic!("{}: no release", path.display()))
            .to_owned();
        let schema = manifest["postgres"]["schema"]
            .as_i64()
            .and_then(|s| i32::try_from(s).ok())
            .unwrap_or_else(|| panic!("{}: no postgres.schema", path.display()));
        found.push((release, schema));
    }
    sorted(found)
}

/// The newest of `releases` at a schema version before `shared_from`: the
/// newest release whose server serves a database alone.
fn alone_until(releases: &[(String, i32)], shared_from: i32) -> Option<String> {
    sorted(releases.to_vec())
        .into_iter()
        .rfind(|(_, schema)| *schema < shared_from)
        .map(|(release, _)| release)
}

/// The releases that leave one schema version: the version, its oldest
/// release and its newest, oldest version first.
fn groups(releases: &[(String, i32)]) -> Vec<(i32, String, String)> {
    let mut found: Vec<(i32, String, String)> = Vec::new();
    for (release, schema) in sorted(releases.to_vec()) {
        match found.last_mut() {
            Some((last, _, newest)) if *last == schema => *newest = release,
            _ => found.push((schema, release.clone(), release)),
        }
    }
    found
}

/// The versions named in `text`, each with where it starts and ends. A
/// version is three numbers with a dot between each; a dot after it ends
/// a sentence.
fn versions_in(text: &str) -> Vec<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let part = |b: u8| b.is_ascii_digit() || b == b'.';
        if part(bytes[at])
            && (at == 0 || !(part(bytes[at - 1]) || bytes[at - 1].is_ascii_alphanumeric()))
        {
            let mut end = at;
            while end < bytes.len() && part(bytes[end]) {
                end += 1;
            }
            let word = text[at..end].trim_end_matches('.');
            let after = bytes.get(at + word.len()).copied();
            if version(word).is_some() && !after.is_some_and(|b| b.is_ascii_alphanumeric()) {
                found.push((at, at + word.len(), word.to_owned()));
            }
            at = end;
        } else {
            at += 1;
        }
    }
    found
}

/// `text` as one line, every run of white space one space.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Each "X or earlier" in `text` whose X is not `alone`, named for `file`.
fn stale_or_earlier(file: &str, text: &str, alone: &str) -> Vec<String> {
    let text = flat(text);
    versions_in(&text)
        .into_iter()
        .filter(|(_, end, named)| text[*end..].starts_with(" or earlier") && named != alone)
        .map(|(_, _, named)| {
            format!(
                "{file}: \"{named} or earlier\": the newest release that serves a database alone is {alone}"
            )
        })
        .collect()
}

/// Each range of releases in `text` ("A and B", "A to B", "A or B") that
/// starts at the oldest release of a schema version and does not end at
/// that version's newest, named for `file`.
fn stale_ranges(file: &str, text: &str, groups: &[(i32, String, String)]) -> Vec<String> {
    let text = flat(text);
    let named = versions_in(&text);
    let mut stale = Vec::new();
    for pair in named.windows(2) {
        let ((_, end, first), (start, _, last)) = (&pair[0], &pair[1]);
        let between = &text[*end..*start];
        if ![" and ", " to ", " or "].contains(&between) {
            continue;
        }
        if let Some((schema, _, newest)) = groups.iter().find(|(_, oldest, _)| oldest == first)
            && last != newest
            && version(last) > version(first)
        {
            stale.push(format!(
                "{file}: \"{first}{between}{last}\": the releases at schema version {schema} are {first} to {newest}"
            ));
        }
    }
    stale
}

/// A file of the repository, by its path from the crate.
fn read(path: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The CHANGELOG's section of the changes not yet released.
fn unreleased() -> String {
    let changelog = read("../../CHANGELOG.md");
    let (_, rest) = changelog
        .split_once("## [Unreleased]")
        .expect("the CHANGELOG has an [Unreleased] section");
    rest.split("\n## [").next().unwrap_or_default().to_owned()
}

/// What `alone_until` answers on the fixtures.
fn alone() -> String {
    alone_until(&fixtures(), SHARED_FROM).expect("a fixture of a release that serves alone")
}

fn of(releases: &[(&str, i32)]) -> Vec<(String, i32)> {
    releases
        .iter()
        .map(|(r, s)| ((*r).to_owned(), *s))
        .collect()
}

// What must work.

#[test]
fn the_message_names_the_newest_release_that_serves_a_database_alone() {
    let alone = alone();
    assert!(
        SERVER_LOCK_HELD.contains(&format!("a server of release {alone} or earlier holds")),
        "the message names {alone}: {SERVER_LOCK_HELD}"
    );
}

#[test]
fn the_how_to_names_that_release_wherever_it_says_or_earlier() {
    let stale = stale_or_earlier(
        "postgres-catalog.md",
        &read("../../website/content/docs/postgres-catalog.md"),
        &alone(),
    );
    assert!(
        stale.is_empty(),
        "stale release names:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_schema_source_names_that_release_wherever_it_says_or_earlier() {
    let stale = stale_or_earlier(
        "backend/postgres/schema.rs",
        &read("src/backend/postgres/schema.rs"),
        &alone(),
    );
    assert!(
        stale.is_empty(),
        "stale release names:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_unreleased_changes_name_that_release_wherever_they_say_or_earlier() {
    let stale = stale_or_earlier("CHANGELOG.md [Unreleased]", &unreleased(), &alone());
    assert!(
        stale.is_empty(),
        "stale release names:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_how_to_and_the_status_page_name_each_schema_versions_releases_whole() {
    let groups = groups(&fixtures());
    let mut stale = Vec::new();
    for page in ["postgres-catalog.md", "status.md"] {
        stale.extend(stale_ranges(
            page,
            &read(&format!("../../website/content/docs/{page}")),
            &groups,
        ));
    }
    assert!(stale.is_empty(), "stale ranges:\n{}", stale.join("\n"));
}

#[test]
fn the_first_shared_version_is_the_one_whose_migration_adds_the_leases() {
    let index = usize::try_from(SHARED_FROM - 1).expect("a positive version");
    assert!(
        MIGRATIONS.len() > index,
        "this build has the migration to version {SHARED_FROM}"
    );
    assert!(
        MIGRATIONS[index].contains("CREATE TABLE vcon.lease"),
        "version {SHARED_FROM}'s migration adds the leases"
    );
    for (at, migration) in MIGRATIONS[..index].iter().enumerate() {
        assert!(
            !migration.contains("vcon.lease"),
            "version {} has no lease table",
            at + 1
        );
    }
}

#[test]
fn this_build_shares_the_servers_lock() {
    const {
        assert!(
            SCHEMA_VERSION >= SHARED_FROM,
            "this build is at the first shared version or later"
        );
    }
    assert!(
        lock_text(Holds::Serve).contains("pg_try_advisory_lock_shared"),
        "a server of this build shares the lock"
    );
}

#[test]
fn every_fixture_is_at_a_version_this_build_knows() {
    let releases = fixtures();
    assert!(!releases.is_empty(), "there are release fixtures");
    for (release, schema) in &releases {
        assert!(version(release).is_some(), "{release} is a version");
        assert!(
            (1..=SCHEMA_VERSION).contains(schema),
            "{release} leaves version {schema}, at most {SCHEMA_VERSION}"
        );
    }
}

#[test]
fn a_newer_release_never_leaves_an_older_schema_version() {
    let releases = fixtures();
    for pair in releases.windows(2) {
        assert!(
            pair[0].1 <= pair[1].1,
            "{} leaves {} and {} leaves {}",
            pair[0].0,
            pair[0].1,
            pair[1].0,
            pair[1].1
        );
    }
}

#[test]
fn the_newest_alone_release_is_older_than_every_shared_one() {
    let releases = fixtures();
    let alone = version(&alone()).expect("a version");
    for (release, schema) in &releases {
        if *schema >= SHARED_FROM {
            assert!(
                version(release).expect("a version") > alone,
                "{release} at {schema} is newer than the alone release"
            );
        }
    }
}

#[test]
fn the_rule_picks_the_newest_release_before_the_shared_version() {
    let releases = of(&[
        ("0.4.15", 14),
        ("0.4.17", 14),
        ("0.4.16", 14),
        ("0.4.18", 15),
    ]);
    assert_eq!(
        alone_until(&releases, 15).as_deref(),
        Some("0.4.17"),
        "the newest at 14"
    );
}

#[test]
fn the_rule_keeps_its_answer_when_a_shared_release_is_added() {
    let before = of(&[("0.4.16", 14), ("0.4.17", 14)]);
    let after = of(&[
        ("0.4.16", 14),
        ("0.4.17", 14),
        ("0.4.18", 15),
        ("0.4.19", 16),
    ]);
    assert_eq!(
        alone_until(&before, 15),
        alone_until(&after, 15),
        "a later release does not move it"
    );
}

#[test]
fn the_rule_orders_by_number_not_by_text() {
    let releases = of(&[("0.4.9", 13), ("0.4.10", 13), ("0.4.2", 6)]);
    assert_eq!(
        alone_until(&releases, 15).as_deref(),
        Some("0.4.10"),
        "0.4.10 is after 0.4.9"
    );
}

#[test]
fn groups_hold_each_versions_oldest_and_newest_release() {
    let releases = of(&[
        ("0.4.14", 13),
        ("0.4.6", 13),
        ("0.4.15", 14),
        ("0.4.17", 14),
        ("0.4.18", 15),
    ]);
    assert_eq!(
        groups(&releases),
        vec![
            (13, "0.4.6".to_owned(), "0.4.14".to_owned()),
            (14, "0.4.15".to_owned(), "0.4.17".to_owned()),
            (15, "0.4.18".to_owned(), "0.4.18".to_owned()),
        ],
        "three versions"
    );
}

#[test]
fn a_version_is_read_with_a_sentence_end_after_it() {
    assert_eq!(
        versions_in("up to 0.4.17. Then"),
        vec![(6, 12, "0.4.17".to_owned())],
        "the dot ends the sentence"
    );
}

#[test]
fn a_version_across_a_line_break_reads_as_one_phrase() {
    let stale = stale_or_earlier("p", "a server of release\n  0.4.16 or\n  earlier", "0.4.17");
    assert_eq!(
        stale.len(),
        1,
        "the phrase is found across lines: {stale:?}"
    );
}

#[test]
fn the_right_release_passes() {
    assert!(
        stale_or_earlier("p", "a server of release 0.4.17 or earlier holds", "0.4.17").is_empty(),
        "0.4.17 is the right one"
    );
}

#[test]
fn a_whole_range_passes() {
    let groups = vec![(14, "0.4.15".to_owned(), "0.4.17".to_owned())];
    assert!(
        stale_ranges("p", "releases 0.4.15 to 0.4.17 leave 14", &groups).is_empty(),
        "the whole range"
    );
}

#[test]
fn a_range_of_another_start_is_not_read() {
    let groups = vec![(14, "0.4.15".to_owned(), "0.4.17".to_owned())];
    assert!(
        stale_ranges("p", "the screens of releases 0.4.7 to 0.4.17", &groups).is_empty(),
        "0.4.7 starts no schema version's releases"
    );
}

// What must be refused.

#[test]
fn an_older_release_before_or_earlier_is_refused() {
    let stale = stale_or_earlier("p", "a server of release 0.4.16 or earlier holds", "0.4.17");
    assert_eq!(stale.len(), 1, "0.4.16 is stale: {stale:?}");
    assert!(
        stale[0].contains("0.4.17"),
        "it names the right one: {stale:?}"
    );
}

#[test]
fn a_newer_release_before_or_earlier_is_refused() {
    let stale = stale_or_earlier("p", "of release 0.4.18 or earlier", "0.4.17");
    assert_eq!(stale.len(), 1, "0.4.18 is not the alone release: {stale:?}");
}

#[test]
fn each_stale_phrase_is_named() {
    let stale = stale_or_earlier("p", "0.4.16 or earlier, and 0.4.15 or earlier", "0.4.17");
    assert_eq!(stale.len(), 2, "both: {stale:?}");
}

#[test]
fn a_range_that_stops_short_is_refused() {
    let groups = vec![(14, "0.4.15".to_owned(), "0.4.17".to_owned())];
    for phrase in [
        "releases 0.4.15 and 0.4.16 leave 14",
        "releases 0.4.15 to 0.4.16 leave 14",
        "release 0.4.15 or 0.4.16 serves",
    ] {
        let stale = stale_ranges("p", phrase, &groups);
        assert_eq!(stale.len(), 1, "{phrase}: {stale:?}");
        assert!(
            stale[0].contains("0.4.15 to 0.4.17"),
            "it names the whole range: {stale:?}"
        );
    }
}

#[test]
fn a_range_across_a_line_break_that_stops_short_is_refused() {
    let groups = vec![(14, "0.4.15".to_owned(), "0.4.17".to_owned())];
    let stale = stale_ranges("p", "releases 0.4.15\n  and 0.4.16 leave it at 14", &groups);
    assert_eq!(stale.len(), 1, "read across the line: {stale:?}");
}

#[test]
fn a_range_past_the_newest_is_refused() {
    let groups = vec![(13, "0.4.6".to_owned(), "0.4.14".to_owned())];
    let stale = stale_ranges("p", "releases 0.4.6 to 0.4.15 make 13", &groups);
    assert_eq!(stale.len(), 1, "0.4.15 is not at 13: {stale:?}");
}

#[test]
fn a_fixture_set_with_only_shared_releases_has_no_alone_release() {
    assert_eq!(
        alone_until(&of(&[("0.4.18", 15)]), 15),
        None,
        "no release serves alone"
    );
}

// Edges.

#[test]
fn no_fixtures_have_no_alone_release_and_no_groups() {
    assert_eq!(alone_until(&[], 15), None, "nothing to read");
    assert!(groups(&[]).is_empty(), "no versions");
}

#[test]
fn a_single_release_is_a_group_of_its_own() {
    assert_eq!(
        groups(&of(&[("0.4.5", 6)])),
        vec![(6, "0.4.5".to_owned(), "0.4.5".to_owned())],
        "oldest and newest are the one"
    );
}

#[test]
fn a_version_is_three_numbers() {
    assert_eq!(version("0.4.17"), Some((0, 4, 17)), "three numbers");
    assert_eq!(version("0.4"), None, "two are not a version");
    assert_eq!(version("127.0.0.1"), None, "four are not");
    assert_eq!(version("0.4.x"), None, "a letter is not");
    assert_eq!(version("0..1"), None, "an empty part is not");
}

#[test]
fn numbers_inside_other_words_are_not_versions() {
    assert!(
        versions_in("v0.4.17 and 192.0.2.40 and a0.4.1").is_empty(),
        "a tag, an address and a word: {:?}",
        versions_in("v0.4.17 and 192.0.2.40 and a0.4.1")
    );
}

#[test]
fn a_range_that_runs_backwards_is_not_read_as_stale() {
    let groups = vec![(14, "0.4.15".to_owned(), "0.4.17".to_owned())];
    assert!(
        stale_ranges("p", "from 0.4.15 to 0.4.14", &groups).is_empty(),
        "a downgrade is not a range of releases"
    );
}

#[test]
fn a_range_with_other_words_between_is_not_read() {
    let groups = vec![(14, "0.4.15".to_owned(), "0.4.17".to_owned())];
    assert!(
        stale_ranges("p", "0.4.15, then 0.4.16", &groups).is_empty(),
        "only and, to and or make a range"
    );
}

#[test]
fn a_page_with_no_versions_is_clean() {
    let groups = vec![(14, "0.4.15".to_owned(), "0.4.17".to_owned())];
    assert!(
        stale_or_earlier("p", "no release here", "0.4.17").is_empty(),
        "or earlier"
    );
    assert!(
        stale_ranges("p", "no release here", &groups).is_empty(),
        "ranges"
    );
}

#[test]
fn the_rule_ignores_a_release_at_the_shared_version_itself() {
    let releases = of(&[("0.4.17", 14), ("0.4.18", 15)]);
    assert_eq!(
        alone_until(&releases, 15).as_deref(),
        Some("0.4.17"),
        "15 is shared, 14 is not"
    );
}
