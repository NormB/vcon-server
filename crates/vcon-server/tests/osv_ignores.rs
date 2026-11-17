//! osv-scanner.toml, the advisories the OSV job lets through: each one
//! named here, with its reason, ending within 90 days so it is reviewed;
//! a copy of the file, byte for byte, beside every lockfile the scanner
//! reads; the workflow reads those copies as Scorecard does; and the
//! reason's claim that no crate of this repository depends on chrono
//! itself still holds.
//!
//! Why a copy beside each lockfile: osv-scanner reads `osv-scanner.toml`
//! only from the directory of the lockfile it scans, never from a parent
//! (internal/config/manager.go, normalizeConfigLoadPath, v2.5.1). Scorecard's
//! Vulnerabilities check (v5.6.0, clients/osv.go) scans the repository with
//! that library and no `--config`, so a root-only file left fuzz/Cargo.lock's
//! chrono reported (code-scanning alert 61) while the OSV job, which passed
//! `--config`, was green.

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

/// The configuration's name, which osv-scanner looks for beside a lockfile.
const CONFIG: &str = "osv-scanner.toml";

/// Whether osv-scanner reads the file at `path` as a lockfile. The names
/// of its extractors for the ecosystems a repository like this one can
/// hold; the seven this repository has were read from a run of
/// osv-scanner 2.7.0's `Scanned ... file` lines.
fn is_lockfile(path: &str) -> bool {
    const NAMES: &[&str] = &[
        "Cargo.lock",
        "package-lock.json",
        "npm-shrinkwrap.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "go.mod",
        "poetry.lock",
        "uv.lock",
        "Pipfile.lock",
        "Gemfile.lock",
    ];
    let name = path.rsplit('/').next().unwrap_or(path);
    NAMES.contains(&name) || (name.ends_with(".txt") && name.contains("requirements"))
}

/// The directory of a tracked `path`, "" for the root.
fn directory(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// What is wrong with the copies of the root configuration `root_text`,
/// given the `tracked` files (as `git ls-files` lists them) and `read`,
/// which gives a tracked file's text.
fn copy_problems(
    tracked: &[&str],
    root_text: &str,
    read: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut lockfile_dirs = Vec::new();
    for path in tracked.iter().filter(|p| is_lockfile(p)) {
        let dir = directory(path);
        if dir.is_empty() || lockfile_dirs.contains(&dir) {
            continue;
        }
        lockfile_dirs.push(dir);
        let copy = format!("{dir}/{CONFIG}");
        if !tracked.contains(&copy.as_str()) {
            out.push(format!("{dir}: no {CONFIG} beside {path}"));
            continue;
        }
        match read(&copy) {
            None => out.push(format!("{copy} cannot be read")),
            Some(text) if text != root_text => {
                out.push(format!("{copy} differs from the root's {CONFIG}"));
            }
            Some(_) => {}
        }
    }
    for path in tracked {
        let dir = directory(path);
        let is_copy = path.rsplit('/').next() == Some(CONFIG) && !dir.is_empty();
        if is_copy && !lockfile_dirs.contains(&dir) {
            out.push(format!("{path}: no lockfile beside it"));
        }
    }
    out
}

/// What is wrong with the OSV workflow `text`: it must scan every lockfile
/// and read the configuration beside each, as Scorecard does.
fn workflow_problems(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with("--config") {
            out.push(format!(
                "`{line}`: Scorecard reads the copy beside each lockfile; so must the OSV job"
            ));
        }
    }
    if !text.lines().any(|l| l.trim() == "--recursive") {
        out.push(
            "no `--recursive`: the scan must read every lockfile, as Scorecard's does".to_owned(),
        );
    }
    out
}

/// Directories a walk does not enter, as scripts/tree_files.py's
/// WALK_SKIPS: git's own files, build output and installed packages.
const WALK_SKIPS: &[&str] = &[".git", "target", "node_modules"];

/// Every file under `root`, sorted, named as git names a tracked file.
fn walked(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, name: &str, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("a directory of the tree") {
            let entry = entry.expect("an entry");
            let file = entry.file_name().into_string().expect("UTF-8 names");
            let path = if name.is_empty() {
                file.clone()
            } else {
                format!("{name}/{file}")
            };
            // A symlink is one entry, as git tracks it: never entered.
            if entry.file_type().expect("its type").is_dir() {
                if !WALK_SKIPS.contains(&file.as_str()) {
                    walk(&entry.path(), &path, out);
                }
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, "", &mut out);
    out.sort();
    out
}

/// The files of the tree at `root`: in a checkout, the ones `git` lists;
/// in the commit hook's copy of the staged files, which is no repository,
/// every file under it, which are exactly the staged files. A checkout has
/// `.git` at its root: a directory, or in a worktree a file.
fn listed(root: &Path, git: &dyn Fn() -> Vec<String>) -> Vec<String> {
    if root.join(".git").exists() {
        git()
    } else {
        walked(root)
    }
}

fn tracked_files() -> Vec<String> {
    listed(&root(), &git_ls_files)
}

fn git_ls_files() -> Vec<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["ls-files", "-z"])
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git ls-files: {:?}", out.status);
    String::from_utf8(out.stdout)
        .expect("tracked paths are UTF-8")
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
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
fn the_workflow_reads_the_configuration_beside_each_lockfile_as_scorecard_does() {
    let text = std::fs::read_to_string(root().join(".github/workflows/osv-scanner.yml"))
        .expect("the workflow");
    assert_eq!(
        workflow_problems(&text),
        Vec::<String>::new(),
        "osv-scanner.yml"
    );
}

#[test]
fn every_lockfile_has_the_root_configuration_beside_it() {
    let tracked = tracked_files();
    let tracked: Vec<&str> = tracked.iter().map(String::as_str).collect();
    let root_text = std::fs::read_to_string(root().join(CONFIG)).expect("osv-scanner.toml");
    let read = |path: &str| std::fs::read_to_string(root().join(path)).ok();
    assert_eq!(
        copy_problems(&tracked, &root_text, &read),
        Vec::<String>::new(),
        "Scorecard reads only the copy beside each lockfile"
    );
}

#[test]
fn the_repository_has_lockfiles_outside_the_root() {
    // Without them the copy rule above would hold nothing.
    let tracked = tracked_files();
    let outside = tracked
        .iter()
        .filter(|p| is_lockfile(p) && !directory(p).is_empty())
        .count();
    assert!(outside >= 6, "fuzz, three npm, go, scripts: {outside}");
}

#[test]
fn copies_beside_each_lockfile_pass() {
    let tracked = [
        "Cargo.lock",
        CONFIG,
        "fuzz/Cargo.lock",
        "fuzz/osv-scanner.toml",
        "clients/go/go.mod",
        "clients/go/osv-scanner.toml",
    ];
    let read = |_: &str| Some(GOOD.to_owned());
    assert_eq!(
        copy_problems(&tracked, GOOD, &read),
        Vec::<String>::new(),
        "every copy is the root's"
    );
}

#[test]
fn a_root_lockfile_alone_needs_no_copy() {
    let read = |_: &str| None;
    assert_eq!(
        copy_problems(&["Cargo.lock", CONFIG, "README.md"], GOOD, &read),
        Vec::<String>::new(),
        "the root file is the one beside it"
    );
}

#[test]
fn two_lockfiles_in_one_directory_share_one_copy() {
    let tracked = [
        "site/package-lock.json",
        "site/requirements.txt",
        "site/osv-scanner.toml",
    ];
    let read = |_: &str| Some(GOOD.to_owned());
    assert_eq!(
        copy_problems(&tracked, GOOD, &read),
        Vec::<String>::new(),
        "one copy per directory"
    );
}

#[test]
fn the_lockfiles_osv_scanner_read_here_are_lockfiles() {
    for path in [
        "Cargo.lock",
        "fuzz/Cargo.lock",
        "site-tests/package-lock.json",
        "clients/go/go.mod",
        "scripts/icons-requirements.txt",
        "requirements.txt",
    ] {
        assert!(is_lockfile(path), "{path}");
    }
}

#[test]
fn other_lockfile_names_of_the_scanner_are_lockfiles() {
    for path in [
        "a/yarn.lock",
        "a/pnpm-lock.yaml",
        "a/npm-shrinkwrap.json",
        "a/poetry.lock",
        "a/uv.lock",
        "a/Pipfile.lock",
        "a/Gemfile.lock",
    ] {
        assert!(is_lockfile(path), "{path}");
    }
}

#[test]
fn a_workflow_that_scans_every_lockfile_without_an_override_passes() {
    let text = "          scan-args: |-\n            --format=sarif\n            --recursive\n            ./\n";
    assert_eq!(workflow_problems(text), Vec::<String>::new(), "no --config");
}

#[test]
fn a_comment_that_names_the_option_is_not_the_option() {
    let text = "      # Not --config: Scorecard has none.\n            --recursive\n";
    assert_eq!(workflow_problems(text), Vec::<String>::new(), "a comment");
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
fn a_lockfile_without_a_copy_beside_it_is_refused() {
    let read = |_: &str| Some(GOOD.to_owned());
    assert_eq!(
        copy_problems(&["Cargo.lock", CONFIG, "fuzz/Cargo.lock"], GOOD, &read),
        ["fuzz: no osv-scanner.toml beside fuzz/Cargo.lock"],
        "the case of alert 61"
    );
}

#[test]
fn each_directory_without_a_copy_is_named_once() {
    let tracked = [
        "fuzz/Cargo.lock",
        "web/package-lock.json",
        "web/requirements.txt",
    ];
    let read = |_: &str| None;
    assert_eq!(
        copy_problems(&tracked, GOOD, &read),
        [
            "fuzz: no osv-scanner.toml beside fuzz/Cargo.lock",
            "web: no osv-scanner.toml beside web/package-lock.json"
        ],
        "by its first lockfile"
    );
}

#[test]
fn a_copy_that_differs_from_the_root_is_refused() {
    let tracked = ["fuzz/Cargo.lock", "fuzz/osv-scanner.toml"];
    let read = |_: &str| Some(GOOD.replace("2027-02-15", "2027-02-14"));
    assert_eq!(
        copy_problems(&tracked, GOOD, &read),
        ["fuzz/osv-scanner.toml differs from the root's osv-scanner.toml"],
        "byte for byte"
    );
}

#[test]
fn a_copy_on_disk_that_git_does_not_track_is_refused() {
    // A clone, as Scorecard's, has only what is tracked.
    let read = |_: &str| Some(GOOD.to_owned());
    assert_eq!(
        copy_problems(&["fuzz/Cargo.lock"], GOOD, &read),
        ["fuzz: no osv-scanner.toml beside fuzz/Cargo.lock"],
        "tracked, not on disk"
    );
}

#[test]
fn a_tracked_copy_that_cannot_be_read_is_refused() {
    let tracked = ["fuzz/Cargo.lock", "fuzz/osv-scanner.toml"];
    let read = |_: &str| None;
    assert_eq!(
        copy_problems(&tracked, GOOD, &read),
        ["fuzz/osv-scanner.toml cannot be read"],
        "named, not skipped"
    );
}

#[test]
fn a_copy_where_no_lockfile_is_is_refused() {
    let read = |_: &str| Some(GOOD.to_owned());
    assert_eq!(
        copy_problems(
            &["Cargo.lock", CONFIG, "docs/osv-scanner.toml"],
            GOOD,
            &read
        ),
        ["docs/osv-scanner.toml: no lockfile beside it"],
        "a left-over copy"
    );
}

#[test]
fn a_workflow_that_overrides_the_configuration_is_refused() {
    let text = "            --config=osv-scanner.toml\n            --recursive\n";
    assert_eq!(
        workflow_problems(text),
        [
            "`--config=osv-scanner.toml`: Scorecard reads the copy beside each lockfile; so must the OSV job"
        ],
        "the job would be green while Scorecard reports"
    );
}

#[test]
fn a_workflow_that_does_not_recurse_is_refused() {
    assert_eq!(
        workflow_problems("            --format=sarif\n            ./\n"),
        ["no `--recursive`: the scan must read every lockfile, as Scorecard's does"],
        "every lockfile"
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
fn a_copy_that_differs_only_in_its_last_newline_is_refused() {
    let tracked = ["fuzz/Cargo.lock", "fuzz/osv-scanner.toml"];
    let read = |_: &str| Some(GOOD.trim_end().to_owned());
    assert_eq!(
        copy_problems(&tracked, GOOD, &read),
        ["fuzz/osv-scanner.toml differs from the root's osv-scanner.toml"],
        "byte for byte, so the copies cannot drift"
    );
}

#[test]
fn a_deep_directory_needs_its_own_copy() {
    let read = |_: &str| Some(GOOD.to_owned());
    assert_eq!(
        copy_problems(&["a/b/c/Cargo.lock", "a/osv-scanner.toml"], GOOD, &read),
        [
            "a/b/c: no osv-scanner.toml beside a/b/c/Cargo.lock",
            "a/osv-scanner.toml: no lockfile beside it"
        ],
        "a parent's copy is not read"
    );
}

#[test]
fn names_that_only_look_like_lockfiles_are_not_lockfiles() {
    for path in [
        "Cargo.toml",
        "fuzz/Cargo.lock.orig",
        "package.json",
        "go.sum",
        "requirements.in",
        "docs/requirements.md",
        "notCargo.lock",
    ] {
        assert!(!is_lockfile(path), "{path}");
    }
}

#[test]
fn the_root_directory_is_the_empty_path() {
    assert_eq!(directory("Cargo.lock"), "", "the root");
    assert_eq!(directory("a/b/go.mod"), "a/b", "a nested one");
}

#[test]
fn an_empty_tree_has_no_problem() {
    let read = |_: &str| None;
    assert!(
        copy_problems(&[], GOOD, &read).is_empty(),
        "nothing tracked"
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

// The commit hook runs these tests in a copy of the staged files
// (`git checkout-index`), which is no repository; run by hand, as the
// gates are, `git ls-files` there exits 128 and two tests above failed on
// 2026-11-17. Under `git commit` the hook inherits GIT_DIR, which hid it.

/// A tree under a new temporary directory, with `files` (paths with `/`)
/// written empty.
fn tree(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary directory");
    for file in files {
        let path = dir.path().join(file);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("its directory");
        std::fs::write(&path, "").expect("the file");
    }
    dir
}

fn no_git() -> Vec<String> {
    panic!("a copy that is no repository must not ask git")
}

#[test]
fn a_copy_lists_every_file_under_it() {
    let dir = tree(&["Cargo.lock", "fuzz/Cargo.lock", "fuzz/osv-scanner.toml"]);
    assert_eq!(
        listed(dir.path(), &no_git),
        ["Cargo.lock", "fuzz/Cargo.lock", "fuzz/osv-scanner.toml"],
        "the staged files, as git names them"
    );
}

#[test]
fn a_walk_is_sorted() {
    let dir = tree(&["b", "a/z", "a/b", "c/a"]);
    assert_eq!(
        walked(dir.path()),
        ["a/b", "a/z", "b", "c/a"],
        "sorted by path"
    );
}

#[test]
fn a_walk_names_nested_paths_with_slashes_from_the_root() {
    let dir = tree(&["clients/go/go.mod"]);
    assert_eq!(
        walked(dir.path()),
        ["clients/go/go.mod"],
        "relative, with `/`, no leading ./"
    );
}

#[test]
fn a_walk_skips_git_target_and_node_modules() {
    let dir = tree(&[
        ".git/HEAD",
        "target/debug/x",
        "node_modules/a/package-lock.json",
        "site-tests/node_modules/b/package-lock.json",
        "site-tests/package-lock.json",
    ]);
    assert_eq!(
        walked(dir.path()),
        ["site-tests/package-lock.json"],
        "at any depth, as scripts/tree_files.py's WALK_SKIPS"
    );
}

#[test]
fn a_file_named_like_a_skipped_directory_is_listed() {
    let dir = tree(&["target", "docs/node_modules"]);
    assert_eq!(
        walked(dir.path()),
        ["docs/node_modules", "target"],
        "only directories are skipped"
    );
}

#[test]
fn an_empty_copy_lists_nothing() {
    let dir = tree(&[]);
    assert!(listed(dir.path(), &no_git).is_empty(), "no file staged");
}

#[test]
fn a_checkout_lists_what_git_tracks() {
    let dir = tree(&[".git/HEAD", "Cargo.lock", "untracked.txt"]);
    let git = || vec!["Cargo.lock".to_owned()];
    assert_eq!(
        listed(dir.path(), &git),
        ["Cargo.lock"],
        "git's list, not the files on disk"
    );
}

#[test]
fn a_worktree_checkout_lists_what_git_tracks() {
    let dir = tree(&[".git", "Cargo.lock"]);
    let git = || vec!["Cargo.lock".to_owned(), "fuzz/Cargo.lock".to_owned()];
    assert_eq!(
        listed(dir.path(), &git),
        ["Cargo.lock", "fuzz/Cargo.lock"],
        "a worktree's .git is a file"
    );
}

#[test]
fn a_symlinked_directory_is_listed_and_not_entered() {
    let dir = tree(&["real/Cargo.lock"]);
    std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).expect("a link");
    assert_eq!(
        walked(dir.path()),
        ["link", "real/Cargo.lock"],
        "git tracks a symlink as one entry"
    );
}

#[test]
fn the_copy_rule_holds_in_a_copy() {
    let dir = tree(&["Cargo.lock", "fuzz/Cargo.lock", "fuzz/osv-scanner.toml"]);
    std::fs::write(dir.path().join("fuzz/osv-scanner.toml"), GOOD).expect("the copy");
    let files = listed(dir.path(), &no_git);
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let read = |path: &str| std::fs::read_to_string(dir.path().join(path)).ok();
    assert!(
        copy_problems(&files, GOOD, &read).is_empty(),
        "the copy beside fuzz/Cargo.lock is found"
    );
}

#[test]
fn a_missing_copy_is_found_in_a_copy() {
    let dir = tree(&["Cargo.lock", "fuzz/Cargo.lock"]);
    let files = listed(dir.path(), &no_git);
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let read = |path: &str| std::fs::read_to_string(dir.path().join(path)).ok();
    assert_eq!(
        copy_problems(&files, GOOD, &read),
        ["fuzz: no osv-scanner.toml beside fuzz/Cargo.lock"],
        "the rule still refuses in the hook's copy"
    );
}

#[test]
fn this_tree_is_listed_either_way() {
    let files = tracked_files();
    assert!(
        files.iter().any(|f| f == "fuzz/Cargo.lock"),
        "a checkout or the hook's copy holds fuzz/Cargo.lock: {} files",
        files.len()
    );
}
