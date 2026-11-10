//! What a released vcon-server holds is one rule: the package's default
//! features. The release build, `cargo install`, the build from source the
//! install page shows and the binary the how-to scenarios run all name no
//! feature set of their own, so each is the default set, and the default
//! set holds the terminal UI.

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

#[test]
fn the_release_build_names_no_features_of_its_own() {
    let script = read("scripts/reproducible-build.sh");
    let commands = builds(&script);
    assert_eq!(commands.len(), 1, "one release build: {commands:?}");
    assert!(
        commands[0].starts_with("cargo build --release --locked -p vcon-server"),
        "the release build: {}",
        commands[0]
    );
    assert!(
        !names_features(commands[0]),
        "the release build is the default features: {}",
        commands[0]
    );
}

#[test]
fn the_install_page_builds_and_installs_the_default_features() {
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
    for command in commands {
        assert!(
            !names_features(command),
            "a reader's build is the release's: {command}"
        );
    }
}

#[test]
fn the_how_to_scenarios_run_a_build_of_the_default_features() {
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
    // The second build is for the one scenario whose page says to build
    // with the postgres feature (the PostgreSQL catalog preview, which the
    // default build does not have). Every other scenario gets the first.
    assert_eq!(
        commands,
        [
            "run: cargo build --locked -p vcon-server",
            "run: cargo build --locked -p vcon-server --features postgres"
        ],
        "the binary the how-to pages run against is built as a reader's is, and the \
         PostgreSQL page's as that page says"
    );
    // Both builds write target/debug/vcon-server, so the default one is
    // kept, under the name the scenarios without a feature download,
    // before the feature build replaces it.
    let at = |text: &str| {
        job.find(text)
            .unwrap_or_else(|| panic!("the docs-binary job has no `{text}`"))
    };
    let default_kept = at("          name: docs-vcon-server\n");
    let feature_built = at("run: cargo build --locked -p vcon-server --features postgres");
    let feature_kept = at("          name: docs-vcon-server-postgres\n");
    assert!(
        default_kept < feature_built && feature_built < feature_kept,
        "the default build is kept before the postgres build replaces it, and that one after"
    );
}

#[test]
fn ci_still_checks_the_build_without_the_default_features() {
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.lines().any(|line| {
            line.trim()
                == "cargo clippy -p vcon-server --all-targets --locked --no-default-features -- -D warnings"
        }),
        "the build that leaves the terminal UI out is still compiled and linted"
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
        "the release build's own line"
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
