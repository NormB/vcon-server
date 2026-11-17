//! osv-scanner.toml, the advisories the OSV job lets through: each one
//! named here, with its reason, ending within 90 days so it is reviewed;
//! the workflow passes the file for every lockfile; and the reason's claim
//! that no crate of this repository depends on chrono itself still holds.

use std::path::{Path, PathBuf};

/// The advisories the configuration may ignore, each with words its reason
/// must hold. Adding one is a decision: it is made here, in review.
const ALLOWED: &[(&str, &[&str])] = &[(
    "RUSTSEC-2026-0543",
    &["unmaintained", "rmcp", "parquet", "no direct chrono use"],
)];

/// The longest an ignore may run from today.
const MOST_DAYS: i64 = 90;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// What is wrong with the configuration `text` on `today`.
fn problems(text: &str, today: time::Date) -> Vec<String> {
    let doc: toml::Table = match text.parse() {
        Ok(doc) => doc,
        Err(e) => return vec![format!("not TOML: {e}")],
    };
    let mut out = Vec::new();
    for key in doc.keys().filter(|k| *k != "IgnoredVulns") {
        out.push(format!("{key}: only IgnoredVulns is allowed"));
    }
    let Some(entries) = doc.get("IgnoredVulns") else {
        return out;
    };
    let Some(entries) = entries.as_array() else {
        out.push("IgnoredVulns is not a list of tables".to_owned());
        return out;
    };
    let mut seen = Vec::new();
    for entry in entries {
        let id = entry.get("id").and_then(toml::Value::as_str).unwrap_or("");
        if seen.contains(&id) {
            out.push(format!("{id} is ignored twice"));
        }
        seen.push(id);
        let Some((_, words)) = ALLOWED.iter().find(|(a, _)| *a == id) else {
            out.push(format!("{id:?} is not an allowed advisory"));
            continue;
        };
        let reason = entry
            .get("reason")
            .and_then(toml::Value::as_str)
            .unwrap_or("");
        for word in *words {
            if !reason.contains(word) {
                out.push(format!("{id}: the reason does not say {word:?}"));
            }
        }
        match entry
            .get("ignoreUntil")
            .and_then(toml::Value::as_datetime)
            .and_then(|d| d.date)
        {
            None => out.push(format!("{id}: no ignoreUntil date")),
            Some(d) => {
                let until = time::Month::try_from(d.month)
                    .ok()
                    .and_then(|m| time::Date::from_calendar_date(i32::from(d.year), m, d.day).ok());
                match until {
                    None => out.push(format!("{id}: ignoreUntil is not a date")),
                    Some(until) if (until - today).whole_days() > MOST_DAYS => out.push(format!(
                        "{id}: ignoreUntil {until} is more than {MOST_DAYS} days from {today}"
                    )),
                    Some(_) => {}
                }
            }
        }
    }
    out
}

/// Whether the manifest `text` names chrono as a dependency of its own.
fn depends_on_chrono(text: &str) -> bool {
    let Ok(doc) = text.parse::<toml::Table>() else {
        return false;
    };
    let tables = ["dependencies", "dev-dependencies", "build-dependencies"];
    let direct = tables.iter().any(|t| {
        doc.get(*t)
            .and_then(toml::Value::as_table)
            .is_some_and(|deps| deps.contains_key("chrono"))
    });
    let workspace = doc
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(toml::Value::as_table)
        .is_some_and(|deps| deps.contains_key("chrono"));
    direct || workspace
}

fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}

const GOOD: &str = "[[IgnoredVulns]]\nid = \"RUSTSEC-2026-0543\"\nignoreUntil = 2027-02-15\nreason = \
                    \"unmaintained; required by rmcp and parquet; no direct chrono use\"\n";

fn on(y: i32, m: time::Month, d: u8) -> time::Date {
    time::Date::from_calendar_date(y, m, d).expect("a date")
}

// Success.

#[test]
fn the_repositorys_configuration_passes() {
    let text = std::fs::read_to_string(root().join("osv-scanner.toml")).expect("osv-scanner.toml");
    assert_eq!(
        problems(&text, today()),
        Vec::<String>::new(),
        "osv-scanner.toml"
    );
}

#[test]
fn the_workflow_passes_the_configuration_for_every_lockfile() {
    let text = std::fs::read_to_string(root().join(".github/workflows/osv-scanner.yml"))
        .expect("the workflow");
    assert!(
        text.lines()
            .any(|l| l.trim() == "--config=osv-scanner.toml"),
        "scan-args name the file"
    );
    assert!(
        text.lines().any(|l| l.trim() == "--recursive"),
        "and every lockfile is scanned"
    );
}

#[test]
fn no_crate_of_this_repository_depends_on_chrono_itself() {
    let mut manifests = vec![root().join("Cargo.toml"), root().join("fuzz/Cargo.toml")];
    for entry in std::fs::read_dir(root().join("crates")).expect("the crates") {
        manifests.push(entry.expect("an entry").path().join("Cargo.toml"));
    }
    for manifest in manifests {
        let text = std::fs::read_to_string(&manifest).expect("a manifest");
        assert!(
            !depends_on_chrono(&text),
            "{}: the ignore's reason says no direct chrono use",
            manifest.display()
        );
    }
}

#[test]
fn a_good_entry_has_no_problem() {
    assert!(
        problems(GOOD, on(2026, time::Month::November, 17)).is_empty(),
        "good"
    );
}

#[test]
fn a_configuration_with_no_ignore_has_no_problem() {
    assert!(problems("", today()).is_empty(), "nothing ignored");
}

// Failure.

#[test]
fn another_advisory_is_refused() {
    let text = GOOD.replace("RUSTSEC-2026-0543", "RUSTSEC-2026-0001");
    assert_eq!(
        problems(&text, on(2026, time::Month::November, 17)),
        ["\"RUSTSEC-2026-0001\" is not an allowed advisory"],
        "only an allowed id"
    );
}

#[test]
fn an_ignore_without_a_date_is_refused() {
    let text = GOOD.replace("ignoreUntil = 2027-02-15\n", "");
    assert_eq!(
        problems(&text, today()),
        ["RUSTSEC-2026-0543: no ignoreUntil date"],
        "it must end"
    );
}

#[test]
fn an_ignore_longer_than_90_days_is_refused() {
    let found = problems(GOOD, on(2026, time::Month::November, 16));
    assert_eq!(found.len(), 1, "91 days: {found:?}");
    assert!(found[0].contains("more than 90 days"), "{found:?}");
}

#[test]
fn a_reason_missing_a_word_is_refused() {
    let text = GOOD.replace("rmcp and parquet", "dependencies");
    assert_eq!(
        problems(&text, on(2026, time::Month::November, 17)),
        [
            "RUSTSEC-2026-0543: the reason does not say \"rmcp\"",
            "RUSTSEC-2026-0543: the reason does not say \"parquet\""
        ],
        "each missing word named"
    );
}

#[test]
fn another_table_is_refused() {
    let text = format!("{GOOD}\n[[PackageOverrides]]\nname = \"chrono\"\nignore = true\n");
    assert_eq!(
        problems(&text, on(2026, time::Month::November, 17)),
        ["PackageOverrides: only IgnoredVulns is allowed"],
        "a package ignore would hide every advisory of it"
    );
}

#[test]
fn an_advisory_ignored_twice_is_refused() {
    let text = format!("{GOOD}{GOOD}");
    assert_eq!(
        problems(&text, on(2026, time::Month::November, 17)),
        ["RUSTSEC-2026-0543 is ignored twice"],
        "once"
    );
}

#[test]
fn a_direct_chrono_dependency_is_found() {
    assert!(
        depends_on_chrono("[dependencies]\nchrono = \"0.4\"\n"),
        "dependencies"
    );
    assert!(
        depends_on_chrono("[dev-dependencies]\nchrono = \"0.4\"\n"),
        "dev-dependencies"
    );
    assert!(
        depends_on_chrono("[workspace.dependencies]\nchrono = \"0.4\"\n"),
        "the workspace's"
    );
}

#[test]
fn a_file_that_is_not_toml_is_refused() {
    let found = problems("[[IgnoredVulns", today());
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("not TOML"), "{found:?}");
}

// Edge.

#[test]
fn an_ignore_of_exactly_90_days_passes() {
    assert!(
        problems(GOOD, on(2026, time::Month::November, 17)).is_empty(),
        "2026-11-17 to 2027-02-15 is 90 days"
    );
}

#[test]
fn an_ignore_already_past_its_date_passes_the_file_check() {
    // osv-scanner stops ignoring it on that date, so the job fails again.
    assert!(
        problems(GOOD, on(2027, time::Month::March, 1)).is_empty(),
        "past dates are the scanner's"
    );
}

#[test]
fn a_chrono_named_only_in_a_feature_is_not_a_dependency() {
    assert!(
        !depends_on_chrono("[features]\nchrono = []\n"),
        "a feature name"
    );
}

#[test]
fn a_date_and_time_counts_by_its_date() {
    let text = GOOD.replace("2027-02-15", "2027-02-15T23:59:59Z");
    assert!(
        problems(&text, on(2026, time::Month::November, 17)).is_empty(),
        "the date part"
    );
}
