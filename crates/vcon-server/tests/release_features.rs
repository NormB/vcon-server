//! What a vcon-server build holds is two rules, each written in one place.
//!
//! The package's default features (the manifest's `default` list) are what
//! `cargo install`, the build from source the install page shows and the
//! binary most how-to scenarios run are built with: none of them names a
//! feature set of its own, and the default set holds the terminal UI.
//!
//! The release binaries are the default features and the ones the release
//! build's one command names (scripts/reproducible-build.sh): `postgres`,
//! the PostgreSQL catalog preview. Every other place that names features
//! for a reader (the install page, the how-to, the CI build its scenario
//! runs) names that list, read here from the script, and no page says the
//! release binaries lack it. The preview's limits are stated once, in the
//! how-to, and the other pages link to them.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    let path = root().join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The package's default features, from its manifest.
fn default_features() -> Vec<String> {
    let manifest: toml::Table = read("crates/vcon-server/Cargo.toml")
        .parse()
        .expect("the manifest is TOML");
    manifest["features"]["default"]
        .as_array()
        .expect("default is a list")
        .iter()
        .map(|feature| feature.as_str().expect("a feature's name").to_owned())
        .collect()
}

/// The cargo commands in `text` that build or install the server: each
/// line that runs `cargo build` or `cargo install`, outside comments.
fn builds(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter(|line| line.contains("cargo build") || line.contains("cargo install vcon-server"))
        .collect()
}

/// Whether a cargo command chooses features itself.
fn names_features(command: &str) -> bool {
    [
        "--features",
        "--no-default-features",
        "--all-features",
        " -F ",
    ]
    .iter()
    .any(|flag| command.contains(flag))
}

/// Whether a cargo command changes the feature set in a way other than
/// adding features to the default ones.
fn leaves_the_defaults(command: &str) -> bool {
    ["--no-default-features", "--all-features", " -F "]
        .iter()
        .any(|flag| command.contains(flag))
}

/// The features a cargo command adds with `--features`, in the order
/// written: `--features a,b`, `--features=a,b`, quoted or not, and as
/// inline code in a page's sentence.
fn features_named(command: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut words = command.split_whitespace();
    while let Some(word) = words.next() {
        let list = if word == "--features" {
            words.next().unwrap_or("")
        } else if let Some(list) = word.strip_prefix("--features=") {
            list
        } else {
            continue;
        };
        found.extend(
            list.split(',')
                .map(|name| {
                    name.trim_matches(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
                })
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        );
    }
    found
}

/// The release build: the one cargo command of scripts/reproducible-build.sh.
fn release_build() -> String {
    let script = read("scripts/reproducible-build.sh");
    let commands = builds(&script);
    assert_eq!(commands.len(), 1, "one release build: {commands:?}");
    commands[0].to_owned()
}

/// The features the release binaries hold beyond the default ones: what
/// the release build names. This is the one place the list is written.
fn release_features() -> Vec<String> {
    features_named(&release_build())
}

/// ` --features a,b` for the release features, as a command names them.
fn release_flag() -> String {
    format!(" --features {}", release_features().join(","))
}

/// `text` in lower case with every run of white space as one space, so a
/// sentence is found whichever way its paragraph is wrapped. A changelog
/// entry's or a comment's line starts (`#`, `//`) are dropped with it.
fn flat(text: &str) -> String {
    text.lines()
        .map(|line| {
            let line = line.trim();
            line.strip_prefix("# ")
                .or_else(|| line.strip_prefix("// "))
                .unwrap_or(line)
        })
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Ways a page could say the release binaries are built without a feature.
const DENIALS: [&str; 6] = [
    "not in the release binaries",
    "release binaries do not have",
    "release binaries and `cargo install vcon-server` do not have",
    "release binaries have no",
    "release binaries lack",
    "release binaries are built without",
];

/// The phrases of `DENIALS` that `text` holds.
fn denials(text: &str) -> Vec<&'static str> {
    let text = flat(text);
    DENIALS
        .iter()
        .copied()
        .filter(|phrase| text.contains(phrase))
        .collect()
}

/// Whether one sentence of `text` says the release binaries, or the
/// release build, have the `postgres` cargo feature.
fn says_the_release_has_postgres(text: &str) -> bool {
    flat(text).split(". ").any(|sentence| {
        (sentence.contains("release binaries") || sentence.contains("release build"))
            && sentence.contains("`postgres`")
            && (sentence.contains(" have ")
                || sentence.contains(" has ")
                || sentence.contains("built with"))
            && !sentence.contains(" not ")
            && !sentence.contains(" no ")
    })
}

/// The part of a page from the heading line `heading` to the next heading
/// of the same or a higher level; `None` when the page has no such heading
/// or has it more than once.
fn section<'a>(page: &'a str, heading: &str) -> Option<&'a str> {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let mut starts = page
        .match_indices(heading)
        .filter(|(at, _)| *at == 0 || page.as_bytes().get(at.wrapping_sub(1)) == Some(&b'\n'))
        .filter(|(at, _)| page[*at..].lines().next() == Some(heading));
    let (start, _) = starts.next()?;
    if starts.next().is_some() {
        return None;
    }
    let body = &page[start.saturating_add(heading.len())..];
    // A `#` line inside a code block is a shell comment, not a heading.
    let mut in_code = false;
    let mut at = 0usize;
    for line in body.split_inclusive('\n') {
        if line.starts_with("```") {
            in_code = !in_code;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if !in_code && hashes > 0 && hashes <= level && line[hashes..].starts_with(' ') {
            return Some(&body[..at]);
        }
        at = at.saturating_add(line.len());
    }
    Some(body)
}

/// The how-to of the PostgreSQL catalog, and the heading and the anchor of
/// the one place the preview's limits are stated.
const HOW_TO: &str = "website/content/docs/postgres-catalog.md";
const LIMITS_HEADING: &str = "## The limits of the preview";
const LIMITS_ANCHOR: &str = "#the-limits-of-the-preview";

/// `vcon-server tui --config <file>` with no terminal: standard input,
/// output and error are not one.
#[cfg(feature = "tui")]
fn tui(config: &Path) -> std::process::Output {
    use std::process::{Command, Stdio};
    Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .arg("tui")
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .output()
        .expect("vcon-server runs")
}

#[test]
fn the_default_features_hold_the_terminal_ui() {
    let features = default_features();
    assert!(
        features.iter().any(|feature| feature == "tui"),
        "a release is the default features, which must hold tui: {features:?}"
    );
}

#[cfg(feature = "tui")]
#[test]
fn the_default_build_runs_the_terminal_ui_not_the_notice_of_a_build_without_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&dir.path().join("missing.toml"));
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "no client file: {said}");
    assert!(
        said.contains("client file:"),
        "the command read its client file, as the terminal UI does: {said}"
    );
    assert!(
        !said.contains("this build has no terminal UI"),
        "the build has the terminal UI: {said}"
    );
}

/// Every tracked file a build, a test run or a page could name features
/// in: the workflows, the scripts, the hooks and the pages.
fn rule_files() -> Vec<String> {
    let mut files = vec![
        "README.md".to_owned(),
        "crates/vcon-server/README.md".to_owned(),
    ];
    for dir in [
        ".github/workflows",
        "scripts",
        ".githooks",
        "website/content/docs",
    ] {
        let mut names: Vec<String> = std::fs::read_dir(root().join(dir))
            .unwrap_or_else(|e| panic!("read {dir}: {e}"))
            .map(|entry| entry.expect("an entry").path())
            .filter(|path| path.is_file())
            .map(|path| {
                let name = path.file_name().expect("a name").to_string_lossy();
                format!("{dir}/{name}")
            })
            .collect();
        names.sort();
        files.extend(names);
    }
    files
}

#[test]
fn nothing_names_the_terminal_ui_as_an_extra_feature() {
    let mut found = Vec::new();
    for file in rule_files() {
        for (n, line) in read(&file).lines().enumerate() {
            if line.contains("--features tui") {
                found.push(format!("{file}:{}", n.saturating_add(1)));
            }
        }
    }
    assert_eq!(
        found,
        Vec::<String>::new(),
        "tui is a default feature, so naming it is a second rule"
    );
}

/// The entries of the changelog's unreleased section that name `needle`:
/// an entry starts at a line that begins `- ` and holds the lines after it.
fn unreleased_entries(changelog: &str, needle: &str) -> Vec<String> {
    let section: Vec<&str> = changelog
        .lines()
        .skip_while(|line| !line.starts_with("## [Unreleased]"))
        .skip(1)
        .take_while(|line| !line.starts_with("## ["))
        .collect();
    let mut entries: Vec<String> = Vec::new();
    for line in section {
        if line.starts_with("- ") {
            entries.push(line.to_owned());
        } else if line.starts_with("  ")
            && let Some(entry) = entries.last_mut()
        {
            entry.push(' ');
            entry.push_str(line.trim());
        }
    }
    entries.retain(|entry| entry.contains(needle));
    entries
}

#[test]
fn the_changelog_does_not_call_the_terminal_ui_off_by_default() {
    let entries = unreleased_entries(&read("CHANGELOG.md"), "`tui`");
    let stale: Vec<&String> = entries
        .iter()
        .filter(|entry| entry.contains("off by default"))
        .collect();
    assert_eq!(
        stale,
        Vec::<&String>::new(),
        "tui is a default feature; an entry from before that says otherwise"
    );
}

#[test]
fn an_entry_is_read_with_its_following_lines_and_only_from_the_unreleased_section() {
    let changelog =
        "## [Unreleased]\n\n### Added\n\n- one `x`\n  more\n- two\n\n## [0.1.0]\n\n- old `x`\n";
    assert_eq!(
        unreleased_entries(changelog, "`x`"),
        ["- one `x` more"],
        "the entry that names it, whole; not a released one"
    );
}

/// The changelog entry that starts with `start`, with its following lines,
/// from whichever section holds it: a release moves the entry out of the
/// unreleased section and does not change it.
fn entry(changelog: &str, start: &str) -> Option<String> {
    let mut lines = changelog
        .lines()
        .skip_while(|line| !line.starts_with(start));
    let mut entry = lines.next()?.to_owned();
    // A blank line stands between a lead and its table.
    for line in lines.take_while(|line| line.starts_with("  ") || line.is_empty()) {
        if !line.is_empty() {
            entry.push(' ');
            entry.push_str(line.trim());
        }
    }
    Some(entry)
}

/// The two sizes an entry's table gives for a release target, in bytes:
/// the two cells that follow the target's name in code marks, as
/// `` | `<target>` | 1,234 bytes | 5,678 bytes | ``.
fn sizes(entry: &str, target: &str) -> Option<(u64, u64)> {
    let (_, after) = entry.split_once(&format!("`{target}`"))?;
    let mut cells = after.split('|').skip(1);
    let bytes = |cell: &str| {
        let mut words = cell.split_whitespace();
        let number = words.next()?.replace(',', "").parse::<u64>().ok()?;
        (words.next()? == "bytes" && words.next().is_none()).then_some(number)
    };
    Some((bytes(cells.next()?)?, bytes(cells.next()?)?))
}

/// The targets release.yml builds, from its build job's matrix.
fn release_targets() -> Vec<String> {
    let mut targets: Vec<String> = read(".github/workflows/release.yml")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- target: "))
        .map(str::to_owned)
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

const SIZE_ENTRY: &str =
    "- **The release binaries are about 2 MB larger with the PostgreSQL catalog.**";
const CATALOG_ENTRY: &str =
    "- **The catalogs can be kept in PostgreSQL, a preview the release binaries";

#[test]
fn the_changelog_gives_each_release_binarys_size_and_the_upgrade_step() {
    let entry = entry(&read("CHANGELOG.md"), SIZE_ENTRY)
        .unwrap_or_else(|| panic!("the changelog has the entry `{SIZE_ENTRY}`"));
    let targets = release_targets();
    assert_eq!(
        targets.len(),
        2,
        "release.yml builds two targets: {targets:?}"
    );
    for target in targets {
        let (before, after) = sizes(&entry, &target).unwrap_or_else(|| {
            panic!("the entry's table has no row `{target}` | <n> bytes | <n> bytes |")
        });
        assert!(
            before > 40_000_000 && after > before && after < before.saturating_add(8_000_000),
            "{target}: {before} to {after} bytes is not a release binary that grew by the feature"
        );
    }
    // The upgrade step is the catalog entry's, with the link to the limits.
    let catalog = super_entry(CATALOG_ENTRY);
    for words in [
        "Do: Coming from 0.4.5 with that setting, run `vcon-server migrate`",
        "a data-directory deployment changes nothing",
        "postgres-catalog/#the-limits-of-the-preview",
    ] {
        assert!(catalog.contains(words), "the catalog entry says: {words}");
    }
}

/// An entry of the repository's changelog, which must be there.
fn super_entry(start: &str) -> String {
    entry(&read("CHANGELOG.md"), start)
        .unwrap_or_else(|| panic!("the changelog has the entry `{start}`"))
}

#[test]
fn a_size_is_a_cell_of_bytes_after_the_targets_name() {
    let entry = "- **X.** | T | A | B | |---|---|---| | `a-b` | 1,234 bytes | 5,678 bytes | | `c-d` | 10 bytes | 20 bytes | | `e` | 5 bytes | | `f` | 1 bytes | x bytes | | `g` | 1 files | 2 files | | `i` | 1 bytes more | 2 bytes |";
    assert_eq!(sizes(entry, "a-b"), Some((1234, 5678)), "with commas");
    assert_eq!(sizes(entry, "c-d"), Some((10, 20)), "the second row");
    assert_eq!(sizes(entry, "e"), None, "one number is not a change");
    assert_eq!(sizes(entry, "f"), None, "a word that is not a number");
    assert_eq!(sizes(entry, "g"), None, "another unit");
    assert_eq!(sizes(entry, "h"), None, "a target the entry does not name");
    assert_eq!(
        sizes(entry, "i"),
        None,
        "a cell that holds more than a size"
    );
}

#[test]
fn a_changelog_entry_is_found_in_a_released_section_too() {
    let changelog = "## [Unreleased]\n\n## [0.1.0]\n\n- **A.** one\n  two\n\n  | three |\n- **B.** four\n\n## [0.0.1]\n";
    assert_eq!(
        entry(changelog, "- **A.**").as_deref(),
        Some("- **A.** one two | three |"),
        "the entry, the lines that follow it and its table, to the first line that is not its own"
    );
    assert_eq!(
        entry(changelog, "- **B.**").as_deref(),
        Some("- **B.** four"),
        "the last entry of a section ends at the next heading"
    );
    assert_eq!(
        entry(changelog, "- **C.**"),
        None,
        "an entry the changelog does not have"
    );
}

#[test]
fn the_release_build_is_the_default_features_and_postgres() {
    let command = release_build();
    assert!(
        command.starts_with("cargo build --release --locked -p vcon-server"),
        "the release build: {command}"
    );
    assert!(
        !leaves_the_defaults(&command),
        "the release build keeps the default features: {command}"
    );
    assert_eq!(
        release_features(),
        ["postgres"],
        "the release binaries hold the PostgreSQL catalog preview: {command}"
    );
}

#[test]
fn a_release_feature_is_a_feature_of_the_package_and_not_a_default_one() {
    let manifest: toml::Table = read("crates/vcon-server/Cargo.toml")
        .parse()
        .expect("the manifest is TOML");
    let defaults = default_features();
    let named = release_features();
    assert!(!named.is_empty(), "the release build names its features");
    for feature in named {
        assert!(
            manifest["features"].get(&feature).is_some(),
            "{feature} is a feature of the package"
        );
        assert!(
            !defaults.contains(&feature),
            "{feature} is a default feature, so naming it in the release build is a second rule"
        );
    }
}

#[test]
fn the_install_page_builds_the_default_features_and_says_how_to_add_the_release_ones() {
    let page = read("website/content/docs/install.md");
    let commands = builds(&page);
    assert!(
        commands
            .iter()
            .any(|c| c.starts_with("cargo install vcon-server"))
            && commands
                .iter()
                .any(|c| c.starts_with("cargo build --release")),
        "the page shows cargo install and a build from source: {commands:?}"
    );
    // The page's own steps (the lines that are a command and nothing else)
    // build the default features; a sentence may show the command that
    // adds the release's features, and no other feature set.
    let wanted = release_features();
    let mut adds = 0usize;
    for command in commands {
        assert!(
            !leaves_the_defaults(command),
            "no build on the page leaves the default features out: {command}"
        );
        let named = features_named(command);
        if command.starts_with("cargo ") {
            assert_eq!(
                named,
                Vec::<String>::new(),
                "a step of the page builds the default features: {command}"
            );
        } else if !named.is_empty() {
            assert_eq!(
                named, wanted,
                "a feature set that is not the release's: {command}"
            );
            adds = adds.saturating_add(1);
        }
    }
    assert_eq!(
        adds, 2,
        "the page says how cargo install and the build from source get the release's features"
    );
}

#[test]
fn the_how_to_scenarios_run_the_default_build_and_one_with_the_release_features() {
    let ci = read(".github/workflows/ci.yml");
    let job: Vec<&str> = ci
        .lines()
        .skip_while(|line| *line != "  docs-binary:")
        .skip(1)
        .take_while(|line| line.starts_with("   ") || line.is_empty())
        .collect();
    let job = job.join("\n");
    let commands = builds(&job)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    // The first build is what `cargo install` and a build from source give
    // a reader: the default features. The second is built with the
    // features the release build names, for the one scenario whose page
    // needs them (the PostgreSQL catalog preview).
    let with_release_features =
        format!("run: cargo build --locked -p vcon-server{}", release_flag());
    assert_eq!(
        commands,
        [
            "run: cargo build --locked -p vcon-server".to_owned(),
            with_release_features.clone()
        ],
        "the binaries the how-to pages run against: the default features, and the release's"
    );
    // Both builds write target/debug/vcon-server, so the default one is
    // kept, under the name the scenarios without a feature download,
    // before the feature build replaces it.
    let at = |text: &str| {
        job.find(text)
            .unwrap_or_else(|| panic!("the docs-binary job has no `{text}`"))
    };
    let default_kept = at("          name: docs-vcon-server\n");
    let feature_built = at(&with_release_features);
    let feature_kept = at(&format!(
        "          name: docs-vcon-server-{}\n",
        release_features().join("-")
    ));
    assert!(
        default_kept < feature_built && feature_built < feature_kept,
        "the default build is kept before the postgres build replaces it, and that one after"
    );
}

#[test]
fn ci_still_checks_the_build_without_the_default_features() {
    // The list of feature sets that clippy lints holds the build, and the
    // backend job runs the list's group of it; scripts/clippy_features.py
    // check holds the commit hook to running every row of the list.
    let row = "backend: -p vcon-server --all-targets --locked --no-default-features";
    assert!(
        read(".github/clippy-feature-sets.txt")
            .lines()
            .any(|line| line.trim() == row),
        "the build that leaves the terminal UI out is still in the list of linted feature sets"
    );
    let run = "python3 -I scripts/clippy_features.py run backend";
    assert!(
        read(".github/workflows/ci.yml")
            .lines()
            .any(|line| line.trim() == run || line.trim().strip_prefix("run: ") == Some(run)),
        "the build that leaves the terminal UI out is still compiled and linted by CI"
    );
}

#[test]
fn the_fuzz_package_builds_the_server_without_its_default_features() {
    let manifest: toml::Table = read("fuzz/Cargo.toml")
        .parse()
        .expect("the manifest is TOML");
    let server = &manifest["dependencies"]["vcon-server"];
    assert_eq!(
        server
            .get("default-features")
            .and_then(toml::Value::as_bool),
        Some(false),
        "the fuzz targets need no terminal UI and no S3 client: {server}"
    );
}

#[test]
fn the_notice_of_a_build_without_the_ui_is_the_one_the_pages_quote() {
    let notice = "this build has no terminal UI";
    assert!(
        read("crates/vcon-server/src/main.rs").contains(notice),
        "the binary's words"
    );
    for page in [
        "website/content/docs/tui.md",
        "website/content/docs/command-line.md",
    ] {
        let text = read(page);
        assert!(text.contains(notice), "{page} quotes the notice");
        assert!(
            text.contains("--no-default-features"),
            "{page} says which build gives it"
        );
    }
}

#[cfg(feature = "tui")]
#[test]
fn without_a_terminal_the_command_ends_1_and_names_the_terminal() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("temp dir");
    let key_file = dir.path().join("tui.token");
    std::fs::write(&key_file, "vcs_0011223344556677_secret\n").expect("write the key file");
    std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let config = dir.path().join("tui.toml");
    std::fs::write(
        &config,
        format!(
            "url = \"https://vcon.example.com:8443\"\nkey_file = \"{}\"\n",
            key_file.display()
        ),
    )
    .expect("write the client file");
    let out = tui(&config);
    let said = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "no terminal: {said}");
    assert!(said.contains("terminal"), "the terminal is named: {said}");
    assert!(
        !said.contains("this build has no terminal UI"),
        "the build has the terminal UI: {said}"
    );
    assert!(out.stdout.is_empty(), "nothing is drawn: {:?}", out.stdout);
}

#[test]
fn a_command_that_chooses_features_is_seen() {
    for command in [
        "cargo build --release --features postgres",
        "cargo build --no-default-features",
        "cargo install vcon-server --all-features",
        "cargo build -F tui -p vcon-server",
    ] {
        assert!(names_features(command), "{command}");
    }
    assert!(
        !names_features("cargo build --release --locked -p vcon-server --target \"$target\" >&2"),
        "a build of the default features"
    );
}

#[test]
fn a_command_that_leaves_the_default_features_is_seen() {
    for command in [
        "cargo build --no-default-features",
        "cargo build --release --no-default-features --features postgres",
        "cargo install vcon-server --all-features",
        "cargo build -F tui -p vcon-server",
    ] {
        assert!(leaves_the_defaults(command), "{command}");
    }
    for command in [
        "cargo build --release --locked -p vcon-server --features postgres --target \"$target\" >&2",
        "cargo install vcon-server --locked",
    ] {
        assert!(!leaves_the_defaults(command), "{command}");
    }
}

#[test]
fn the_features_a_command_adds_are_read_in_each_way_of_writing_them() {
    let cases: [(&str, &[&str]); 8] = [
        ("cargo build --features postgres", &["postgres"]),
        (
            "cargo build --features=postgres,backend-tests -p x",
            &["postgres", "backend-tests"],
        ),
        ("cargo build --features \"postgres\" >&2", &["postgres"]),
        (
            "add it: `cargo install vcon-server --locked --features postgres`.",
            &["postgres"],
        ),
        ("cargo build --features a --features b_c", &["a", "b_c"]),
        ("cargo build --release --locked -p vcon-server", &[]),
        ("cargo build --features", &[]),
        ("cargo build --no-default-features", &[]),
    ];
    for (command, wanted) in cases {
        assert_eq!(features_named(command), wanted, "{command}");
    }
}

#[test]
fn no_page_says_the_release_binaries_lack_a_feature() {
    let mut files = rule_files();
    files.push("crates/vcon-server/Cargo.toml".to_owned());
    let mut found = Vec::new();
    for file in files {
        for phrase in denials(&read(&file)) {
            found.push(format!("{file}: {phrase}"));
        }
    }
    let changelog = read("CHANGELOG.md");
    let unreleased: Vec<&str> = changelog
        .lines()
        .skip_while(|line| !line.starts_with("## [Unreleased]"))
        .skip(1)
        .take_while(|line| !line.starts_with("## ["))
        .collect();
    for phrase in denials(&unreleased.join("\n")) {
        found.push(format!("CHANGELOG.md, unreleased: {phrase}"));
    }
    assert_eq!(
        found,
        Vec::<String>::new(),
        "the release binaries are built with {:?}; these say they are not",
        release_features()
    );
}

#[test]
fn a_sentence_that_denies_the_feature_is_found_however_it_is_wrapped() {
    // The three sentences the pages held while the release build named no
    // feature, as they were wrapped.
    let readme = "The `postgres` cargo feature, which is not on by default and not in the\nrelease binaries, adds a preview";
    let how_to = "The release binaries and `cargo install vcon-server` do not have the\n`postgres` cargo feature.";
    let configuration =
        "It needs the `postgres` cargo\nfeature, which the release binaries do not have yet.";
    let manifest = "# a preview, which the release\n# binaries do not have";
    assert_eq!(
        denials(readme),
        ["not in the release binaries"],
        "the README's"
    );
    assert_eq!(
        denials(how_to),
        ["release binaries and `cargo install vcon-server` do not have"],
        "the how-to's"
    );
    assert_eq!(
        denials(configuration),
        ["release binaries do not have"],
        "the configuration page's"
    );
    assert_eq!(
        denials(manifest),
        ["release binaries do not have"],
        "a comment's line starts are not part of the sentence"
    );
    assert!(
        denials("The release build on `main` has the `postgres` cargo feature. Release 0.4.5's binaries do not have it.").is_empty(),
        "what one named release lacks is a fact about that release"
    );
}

#[test]
fn a_sentence_that_says_the_release_has_the_feature_is_told_from_one_that_does_not() {
    for text in [
        "The release binaries have the `postgres` cargo feature.",
        "The release build on `main` has the\n`postgres` cargo feature, and more.",
        "First. The release binaries are built with the `postgres` cargo feature, a preview.",
    ] {
        assert!(says_the_release_has_postgres(text), "{text}");
    }
    for text in [
        "The release binaries do not have the `postgres` cargo feature.",
        "The `postgres` cargo feature is a preview.",
        "The release binaries have the terminal UI. The `postgres` cargo feature is a preview.",
        "The release binaries have no `postgres` cargo feature.",
    ] {
        assert!(!says_the_release_has_postgres(text), "{text}");
    }
}

/// The pages that say which builds hold the feature.
const FEATURE_PAGES: [&str; 5] = [
    "README.md",
    HOW_TO,
    "website/content/docs/install.md",
    "website/content/docs/configuration.md",
    "website/content/docs/status.md",
];

#[test]
fn the_pages_say_the_release_build_has_the_feature() {
    for page in FEATURE_PAGES {
        assert!(
            says_the_release_has_postgres(&read(page)),
            "{page} has no sentence that says the release binaries have the `postgres` cargo feature"
        );
    }
}

#[test]
fn the_how_to_has_no_step_that_builds_the_server() {
    // A reader with a release binary has the feature, so the page's steps
    // start from the installed server; the build that adds the feature to
    // `cargo install` is the install page's.
    let page = read(HOW_TO);
    let steps: Vec<&str> = builds(&page)
        .into_iter()
        .filter(|line| line.starts_with("cargo "))
        .collect();
    assert_eq!(steps, Vec::<&str>::new(), "the page builds nothing");
    assert!(
        !page.contains("docs-test: skip the run is given a server built"),
        "no step is skipped for the build"
    );
    assert!(
        page.contains("@/docs/install.md#"),
        "the page sends a reader who builds to the install page"
    );
}

/// The comment lines directly above `key = ` in the manifest, as one flat
/// sentence.
fn manifest_comment(manifest: &str, key: &str) -> String {
    let lines: Vec<&str> = manifest.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.starts_with(&format!("{key} = ")))
        .unwrap_or_else(|| panic!("the manifest has no `{key} = `"));
    let mut comment: Vec<&str> = lines[..at]
        .iter()
        .rev()
        .take_while(|line| line.starts_with('#'))
        .copied()
        .collect();
    comment.reverse();
    flat(&comment.join("\n"))
}

#[test]
fn the_manifest_describes_the_postgres_feature_as_it_is() {
    let comment = manifest_comment(&read("crates/vcon-server/Cargo.toml"), "postgres");
    assert!(
        !comment.contains("and stops"),
        "a server with its catalogs in PostgreSQL serves; it no longer stops after the checks: {comment}"
    );
    assert!(
        comment.contains("preview"),
        "it is still a preview: {comment}"
    );
    assert!(
        comment.contains("release binaries") && comment.contains("scripts/reproducible-build.sh"),
        "the comment says the release binaries are built with it, and where: {comment}"
    );
    assert!(
        comment.contains("not a default feature") || comment.contains("not on by default"),
        "the comment says cargo install does not get it by default: {comment}"
    );
}

#[test]
fn a_manifest_comment_is_the_lines_directly_above_its_key() {
    let manifest = "# Other.\na = []\n# The first\n# line and the second.\nb = [\"a\"]\n";
    assert_eq!(
        manifest_comment(manifest, "b"),
        "the first line and the second.",
        "the comment of `b`, without the one of `a`"
    );
    assert_eq!(manifest_comment("a = []\nb = []\n", "b"), "", "no comment");
}

/// What the limits section must name, each as the flat text holds it.
const LIMITS: [(&str, &[&str]); 9] = [
    (
        "one server per database",
        &["a second server", "refused at start"],
    ),
    (
        "no tenant, and none moved",
        &[
            "starts with no tenant",
            "no command moves",
            "`catalog.backend`",
            "hides",
        ],
    ),
    ("a bucket prefix of its own", &["prefix of its own"]),
    (
        "the backup",
        &[
            "`pg_dump`",
            "base backup",
            "master key file",
            "`vcon-server backup` is refused",
        ],
    ),
    (
        "the commands that run with every server stopped",
        &[
            "`rekey master`",
            "`rekey tenant`",
            "`tenant create`",
            "`tenant remove`",
            "`check`",
            "`migrate`",
            "every server stopped",
        ],
    ),
    ("poolers", &["transaction mode", "no pooler was tested"]),
    (
        "what it was tested on",
        &["postgresql 16, 17 and 18", "managed services"],
    ),
    ("the pool", &["`catalog.postgres.pool`", "at least 2"]),
    (
        "migrate after an upgrade",
        &["run `migrate` first", "0.4.5", "version 6", "version 13"],
    ),
];

#[test]
fn the_how_to_states_each_limit_of_the_preview_in_one_section() {
    let page = read(HOW_TO);
    let limits = section(&page, LIMITS_HEADING)
        .unwrap_or_else(|| panic!("{HOW_TO} has `{LIMITS_HEADING}` once"));
    let limits = flat(limits);
    for (limit, words) in LIMITS {
        for word in words {
            assert!(
                limits.contains(word),
                "{limit}: the section does not say {word}"
            );
        }
    }
    assert!(
        flat(&page).contains("this page is a preview"),
        "the page keeps the word"
    );
}

/// The pages that sum the preview up and send the reader to its limits.
const SUMMARY_PAGES: [&str; 7] = [
    "README.md",
    "website/content/docs/how-to.md",
    "website/content/docs/install.md",
    "website/content/docs/configuration.md",
    "website/content/docs/status.md",
    "website/content/docs/roadmap.md",
    "website/content/docs/development.md",
];

/// Limits a summary page would be stating a second time.
const RESTATED: [&str; 4] = [
    "one server serves a database",
    "one server per database",
    "starts with no tenant",
    "no command moves",
];

#[test]
fn the_other_pages_link_to_the_limits_and_do_not_state_them_again() {
    for page in SUMMARY_PAGES {
        let text = read(page);
        let link = if page == "README.md" {
            format!("https://vcon-server.com/docs/postgres-catalog/{LIMITS_ANCHOR}")
        } else {
            format!("@/docs/postgres-catalog.md{LIMITS_ANCHOR}")
        };
        assert!(text.contains(&link), "{page} does not link to {link}");
        let text = flat(&text);
        assert!(text.contains("preview"), "{page} keeps the word preview");
        for phrase in RESTATED {
            assert!(
                !text.contains(phrase),
                "{page} states a limit again (`{phrase}`); the how-to states it once"
            );
        }
    }
}

#[test]
fn every_page_that_calls_the_catalog_a_preview_links_to_its_how_to() {
    let dir = "website/content/docs";
    let mut pages: Vec<String> = rule_files()
        .into_iter()
        .filter(|file| file.starts_with(dir) && file.ends_with(".md") && file != HOW_TO)
        .collect();
    pages.sort();
    assert!(pages.len() > 20, "the pages are read: {}", pages.len());
    for page in pages {
        let text = read(&page);
        let calls_it_one = text
            .split("\n\n")
            .map(flat)
            .any(|paragraph| paragraph.contains("preview") && paragraph.contains("postgres"));
        if calls_it_one {
            assert!(
                text.contains("@/docs/postgres-catalog.md"),
                "{page} calls the PostgreSQL catalog a preview and does not link to its how-to"
            );
        }
    }
}

#[test]
fn a_section_ends_at_the_next_heading_of_its_level_and_not_at_a_comment_in_a_code_block() {
    let page =
        "# T\n\n## A\n\none\n\n```sh\n# a comment\nrun\n```\n\n### A1\n\ntwo\n\n## B\n\nthree\n";
    assert_eq!(
        section(page, "## A"),
        Some("\n\none\n\n```sh\n# a comment\nrun\n```\n\n### A1\n\ntwo\n\n"),
        "the section holds its code block and its sub-section"
    );
    assert_eq!(
        section(page, "## B"),
        Some("\n\nthree\n"),
        "the last section ends with the page"
    );
    assert_eq!(
        section(page, "## C"),
        None,
        "a heading the page does not have"
    );
    assert_eq!(
        section("## A\n\none\n\n## A\n\ntwo\n", "## A"),
        None,
        "a heading the page has twice is not one place"
    );
    assert_eq!(
        section("text ## A\n\n## A more\n", "## A"),
        None,
        "a heading is a whole line"
    );
}

#[test]
fn a_build_named_in_a_comment_is_not_a_build() {
    let text = "# cargo build --features x\n  cargo build --locked\ncargo test\n";
    assert_eq!(
        builds(text),
        ["cargo build --locked"],
        "comments and other commands are left out"
    );
}
