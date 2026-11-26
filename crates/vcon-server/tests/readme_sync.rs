//! The repository's `README.md` is the page GitHub shows first, and the site
//! at vcon-server.com is the documentation. The README states only what the
//! site and the code state, and each shared fact is read from one source:
//!
//! - the status sentence is the home page's (`website/content/_index.md`),
//!   which `site_status.rs` ties to this build's version;
//! - every release number it names is the workspace version in `Cargo.toml`;
//! - every vcon-server.com page it links is a page in `website/content/`,
//!   and every anchor a heading on that page;
//! - every path it names exists in the tree;
//! - every command it says the hooks run is a command in `.githooks/`, and it
//!   names every check script the hooks run. The site's development page
//!   describes the same hooks and is held to the same rule;
//! - its "What is built" list names the same items, in the same order, as
//!   the "Built and tested" list of the status page.
//!
//! The crates' own READMEs, which crates.io shows, keep the same status
//! sentence, name only this release, link only pages the site has, and use
//! US spelling; the server crate's README links the MCP and S3 pages. The
//! `description` of every `Cargo.toml` in the tree, which crates.io shows
//! too, the site's pages, its llms.txt and the OpenAPI description use US
//! spelling.

mod markdown;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use markdown::{heading_ids, is_relative, link_targets};

/// The site's address, as the README links it.
const SITE: &str = "https://vcon-server.com/";

/// The hook files, in `.githooks/`.
const HOOKS: [&str; 3] = ["pre-commit", "commit-msg", "pre-push"];

/// Spellings that differ in US English, the language of the documentation.
/// Each is a whole word or the stem its forms share (`normalis` finds
/// "normalised" and "normalisation"); none is part of a US word the pages
/// use.
const NON_US_SPELLINGS: [&str; 29] = [
    "authorised",
    "authorisation",
    "licence",
    "behaviour",
    "honour",
    "colour",
    "organisation",
    "recognise",
    "labelled",
    "labelling",
    "cancelled",
    "cancelling",
    "normalis",
    "serialis",
    "initialis",
    "centre",
    "catalogue",
    "analysed",
    "analysing",
    "favour",
    "neighbour",
    "artefact",
    "defence",
    "fulfil ",
    "whilst",
    "summaris",
    "minimis",
    "misspelt",
    "learnt",
];

/// The words of `NON_US_SPELLINGS` that `text` holds, in any case.
fn non_us_spellings(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    NON_US_SPELLINGS
        .iter()
        .copied()
        .filter(|word| lower.contains(word))
        .collect()
}

/// The repository root, two levels above this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    let path = root().join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn readme() -> String {
    read("README.md")
}

/// The vCon community's Conserver, which this project is not.
const CONSERVER_URL: &str = "https://github.com/vcon-dev/vcon-server";

/// The `[extra]` string `key` of a page's TOML front matter (between the
/// first two `+++` lines), or None.
fn front_matter_extra(page: &str, key: &str) -> Option<String> {
    let rest = page.strip_prefix("+++\n")?;
    let end = rest.find("\n+++")?;
    let table: toml::Table = rest[..end].parse().ok()?;
    table
        .get("extra")?
        .as_table()?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

/// `text` with every run of whitespace made one space, trimmed.
fn squeeze(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What is wrong with the home page's notice (`notice`, its front matter's
/// `[extra] notice`) and its copy in the README: it must link to the
/// Conserver, say this project is not it and what this project is, be one
/// line, and stand word for word in the README above the status line.
fn conserver_notice_findings(notice: Option<&str>, readme: &str) -> Vec<String> {
    let Some(notice) = notice.map(str::trim).filter(|n| !n.is_empty()) else {
        return vec!["the home page has no notice".to_owned()];
    };
    let mut found = Vec::new();
    if !notice.contains(&format!("]({CONSERVER_URL})")) {
        found.push(format!("the notice does not link to {CONSERVER_URL}"));
    }
    if !notice.contains("is not the vCon Conserver") {
        found.push("the notice does not say this project is not the vCon Conserver".to_owned());
    }
    if !notice.contains("an encrypted vCon store") {
        found.push("the notice does not say what this project is".to_owned());
    }
    if notice.contains('\n') {
        found.push("the notice is more than one line".to_owned());
    }
    let readme = squeeze(readme);
    match readme.find(&squeeze(notice)) {
        None => found.push("README.md does not carry the notice word for word".to_owned()),
        Some(at) => {
            if readme.find("**Status:").is_some_and(|status| status < at) {
                found.push("README.md carries the notice after its status line".to_owned());
            }
        }
    }
    found
}

/// What is wrong with how the home template shows the notice: in the
/// first hero section, as markdown, after the lead and before the buttons.
fn hero_notice_findings(template: &str) -> Vec<String> {
    let Some(start) = template.find("<section class=\"hero\">") else {
        return vec!["the home page has no hero section".to_owned()];
    };
    let hero = &template[start..];
    let hero = &hero[..hero.find("</section>").unwrap_or(hero.len())];
    let Some(notice) = hero.find("{{ x.notice | markdown(inline=true) | safe }}") else {
        return vec!["the home page's hero does not render x.notice".to_owned()];
    };
    let lead = hero.find("class=\"lead\"");
    let cta = hero.find("class=\"cta\"");
    if lead.is_none_or(|lead| lead > notice) || cta.is_none_or(|cta| cta < notice) {
        return vec!["the notice is not directly after the lead".to_owned()];
    }
    Vec::new()
}

/// The text under the `## {heading}` line, up to the next `## ` heading.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let start_line = format!("## {heading}\n");
    let Some(at) = text.find(&start_line) else {
        return "";
    };
    let body = &text[at + start_line.len()..];
    match body.find("\n## ") {
        Some(end) => &body[..end],
        None => body,
    }
}

/// The inline code spans of `text`, outside fenced blocks.
fn code_spans(text: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut fenced = false;
    let mut prose = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    let mut parts = prose.split('`');
    // The text before the first backtick is prose; after it, spans and
    // prose alternate.
    parts.next();
    while let Some(span) = parts.next() {
        spans.push(span.replace('\n', " "));
        parts.next();
    }
    spans
}

/// The bold status sentence, `**Status: ...**`, of a status note: the text
/// from `**Status:` to the closing `**`.
fn status_sentence(text: &str) -> Option<String> {
    let at = text.find("**Status:")?;
    let rest = &text[at + 2..];
    let end = rest.find("**")?;
    Some(format!("**{}**", &rest[..end]))
}

/// The release numbers, `x.y.z`, in `text`.
fn release_numbers(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let starts = chars[i].is_ascii_digit() && (i == 0 || !is_number_char(chars[i - 1]));
        if !starts {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && is_number_char(chars[j]) {
            j += 1;
        }
        let token: String = chars[i..j].iter().collect();
        let token = token.trim_end_matches('.');
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() == 3 && parts.iter().all(|p| !p.is_empty()) {
            found.push(token.to_string());
        }
        i = j;
    }
    found
}

fn is_number_char(c: char) -> bool {
    c.is_ascii_digit() || c == '.'
}

/// The site page a vcon-server.com link names, as `(source, anchor)`: the
/// Markdown file under `website/content/` that Zola builds it from, and the
/// part after `#`. `None` when the link is not to the site.
fn site_source(url: &str) -> Option<(String, Option<String>)> {
    let rest = url.strip_prefix(SITE)?;
    let (path, anchor) = match rest.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor.to_string())),
        None => (rest, None),
    };
    let path = path.trim_end_matches('/');
    let source = if path.is_empty() {
        "website/content/_index.md".to_string()
    } else if root()
        .join(format!("website/content/{path}/_index.md"))
        .exists()
    {
        format!("website/content/{path}/_index.md")
    } else {
        format!("website/content/{path}.md")
    };
    Some((source, anchor))
}

/// Each vcon-server.com link in `text` whose page is not in
/// `website/content/`, or whose anchor is no heading on that page.
fn site_link_findings(text: &str) -> Vec<String> {
    let mut missing = Vec::new();
    for target in link_targets(text) {
        let Some((source, anchor)) = site_source(&target) else {
            continue;
        };
        let Ok(page) = std::fs::read_to_string(root().join(&source)) else {
            missing.push(format!("{target}: no {source}"));
            continue;
        };
        if let Some(anchor) = anchor
            && !heading_ids(&page).contains(&anchor)
        {
            missing.push(format!("{target}: no heading #{anchor} in {source}"));
        }
    }
    missing
}

/// The paths a README names: its relative link targets, the `src` and
/// `srcset` of its HTML, and each code span whose first word is a path
/// (it holds a `/` or ends in `.toml`, `.md`, `.sh` or `.py`) and is not a
/// route such as `/mcp`. A release file such as `SHA256SUMS.txt` is not a
/// path in the tree.
fn named_paths(text: &str) -> Vec<String> {
    let mut paths: Vec<String> = link_targets(text)
        .into_iter()
        .filter(|t| is_relative(t))
        .map(|t| t.split('#').next().unwrap_or_default().to_string())
        .collect();
    for attribute in ["src=\"", "srcset=\""] {
        let mut rest = text;
        while let Some(at) = rest.find(attribute) {
            rest = &rest[at + attribute.len()..];
            if let Some(end) = rest.find('"') {
                paths.push(rest[..end].to_string());
                rest = &rest[end..];
            }
        }
    }
    for span in code_spans(text) {
        let first = span.split_whitespace().next().unwrap_or_default();
        let looks_like_path = first.contains('/')
            || [".toml", ".md", ".sh", ".py"]
                .iter()
                .any(|ext| first.ends_with(ext));
        if looks_like_path && !first.starts_with('/') && !first.contains("://") {
            paths.push(first.to_string());
        }
    }
    paths
}

/// The command lines of the hooks: every line that is not a comment.
fn hook_lines() -> Vec<String> {
    HOOKS
        .iter()
        .flat_map(|hook| {
            read(&format!(".githooks/{hook}"))
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether `script` (a file name in `scripts/`) is a test of a script
/// rather than a check of the tree.
fn is_script_test(script: &str) -> bool {
    script.starts_with("test_") || script.starts_with("test-")
}

/// What the hooks check: each script in `scripts/` they run that is not a
/// script's own test, as `scripts/<name>`, and each cargo subcommand they
/// run, as `cargo <name>`.
fn hook_checks(lines: &[String]) -> BTreeSet<String> {
    let mut checks = BTreeSet::new();
    for line in lines {
        let mut rest = line.as_str();
        while let Some(at) = rest.find("scripts/") {
            rest = &rest[at + "scripts/".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || "_-.".contains(*c))
                .collect();
            if !name.is_empty() && !is_script_test(&name) {
                checks.insert(format!("scripts/{name}"));
            }
        }
        if let Some(rest) = line.strip_prefix("cargo ")
            && let Some(sub) = rest.split_whitespace().next()
        {
            checks.insert(format!("cargo {sub}"));
        }
    }
    checks
}

/// A code span that names a command: one starting with `cargo ` or with a
/// path in `scripts/`.
fn is_command(span: &str) -> bool {
    span.starts_with("cargo ") || span.starts_with("scripts/")
}

/// Whether a script tests the hooks themselves: its source names the
/// `.githooks` directory. Such a script runs beside the hooks, not in them.
fn tests_the_hooks(command: &str) -> bool {
    let script = command.split_whitespace().next().unwrap_or_default();
    std::fs::read_to_string(root().join(script)).is_ok_and(|text| text.contains(".githooks"))
}

/// What a description of the hooks gets wrong: each command it names that
/// no hook runs, and each check the hooks run that it never names.
fn hook_description_findings(text: &str, lines: &[String]) -> Vec<String> {
    let spans = code_spans(text);
    let mut findings = Vec::new();
    for span in spans.iter().filter(|s| is_command(s)) {
        if !lines.iter().any(|line| line.contains(span.as_str())) && !tests_the_hooks(span) {
            findings.push(format!("names `{span}`, which no hook runs"));
        }
    }
    for check in hook_checks(lines) {
        let named = spans
            .iter()
            .any(|span| span == &check || span.starts_with(&format!("{check} ")));
        if !named {
            findings.push(format!("never names `{check}`, which a hook runs"));
        }
    }
    findings
}

// The readers themselves, on made-up text, both ways.

#[test]
fn the_section_reader_stops_at_the_next_heading() {
    let text = "# t\n\n## A\n\none\n\n## B\n\ntwo\n";
    assert_eq!(section(text, "A"), "\none\n", "A ends where B starts");
    assert_eq!(
        section(text, "B"),
        "\ntwo\n",
        "the last section runs to the end"
    );
    assert_eq!(section(text, "C"), "", "a missing section is empty");
}

#[test]
fn the_span_reader_skips_fenced_blocks() {
    let text = "run `a b` and\n```sh\nnot `this`\n```\nthen `c`, not ``";
    assert_eq!(
        code_spans(text),
        ["a b", "c", ""],
        "spans in prose only, fences skipped"
    );
}

#[test]
fn the_status_reader_takes_the_bold_sentence() {
    let note = "> **Status: 9.8.7 released, not yet for production.** More text.";
    assert_eq!(
        status_sentence(note).as_deref(),
        Some("**Status: 9.8.7 released, not yet for production.**"),
        "the bold sentence, with its markers"
    );
    assert_eq!(
        status_sentence("**Status:** in development."),
        Some("**Status:**".to_string()),
        "a bold label alone is the whole sentence it reads"
    );
    assert_eq!(status_sentence("no status"), None, "no status, no sentence");
}

#[test]
fn the_release_reader_finds_three_part_numbers_only() {
    let text = "0.3.0 released; Version 2.0; RFC 9457; 555-0100; 192.0.2.10; v1.2.3.";
    assert_eq!(
        release_numbers(text),
        ["0.3.0", "1.2.3"],
        "x.y.z only: not two parts, not four"
    );
}

#[test]
fn site_links_name_their_source_pages() {
    assert_eq!(
        site_source("https://vcon-server.com/docs/install/#download-a-release"),
        Some((
            "website/content/docs/install.md".to_string(),
            Some("download-a-release".to_string())
        )),
        "a page and its anchor"
    );
    assert_eq!(
        site_source("https://vcon-server.com/docs/"),
        Some(("website/content/docs/_index.md".to_string(), None)),
        "a section is its _index.md"
    );
    assert_eq!(
        site_source("https://example.com/docs/install/"),
        None,
        "another site is not the site"
    );
}

#[test]
fn a_site_link_needs_its_page_and_its_heading() {
    let good = "[a](https://vcon-server.com/docs/install/#download-a-release) \
                [b](https://vcon-server.com/docs/) [c](https://example.com/docs/none/)";
    assert!(
        site_link_findings(good).is_empty(),
        "pages and headings that exist, and another site, pass"
    );
    assert_eq!(
        site_link_findings(
            "[a](https://vcon-server.com/docs/no-such-page/) \
             [b](https://vcon-server.com/docs/install/#no-such-heading)"
        ),
        [
            "https://vcon-server.com/docs/no-such-page/: no website/content/docs/no-such-page.md",
            "https://vcon-server.com/docs/install/#no-such-heading: no heading #no-such-heading in website/content/docs/install.md",
        ],
        "a missing page and a missing heading are each named"
    );
}

#[test]
fn heading_ids_are_zolas() {
    let page = "## Download a release\n```sh\n# not a heading\n```\n### `check` and **more**\n";
    assert_eq!(
        heading_ids(page),
        BTreeSet::from([
            "check-and-more".to_string(),
            "download-a-release".to_string()
        ]),
        "Zola's slugs, with comment lines in fences left out"
    );
}

#[test]
fn named_paths_are_links_sources_and_path_spans() {
    let text = "<img src=\"a/b.svg\"> [x](c.md#top) [y](https://e.example/) \
                `d/e.sh --flag` `f.toml` `/mcp` `vcon-server mcp` `example.com` \
                `SHA256SUMS.txt`";
    assert_eq!(
        named_paths(text),
        ["c.md", "a/b.svg", "d/e.sh", "f.toml"],
        "routes, commands, domains and release files are not paths"
    );
}

#[test]
fn the_hook_checks_leave_out_script_tests() {
    let lines = [
        "python3 -I scripts/test_check_style.py".to_string(),
        "python3 -I scripts/check_style.py --walk".to_string(),
        "if ! \"$root/scripts/scan-commit-messages.sh\" \"$sha\"; then".to_string(),
        "cargo clippy --workspace -- -D warnings".to_string(),
        "scripts/test-retry.sh".to_string(),
    ];
    assert_eq!(
        hook_checks(&lines),
        BTreeSet::from([
            "cargo clippy".to_string(),
            "scripts/check_style.py".to_string(),
            "scripts/scan-commit-messages.sh".to_string(),
        ]),
        "checks of the tree, not tests of scripts"
    );
}

#[test]
fn a_hook_description_must_quote_commands_and_name_every_check() {
    let lines = [
        "cargo clippy --workspace -- -D warnings".to_string(),
        "python3 -I scripts/check_style.py --walk".to_string(),
    ];
    assert!(
        hook_description_findings(
            "runs `cargo clippy --workspace -- -D warnings` and `scripts/check_style.py --walk`",
            &lines
        )
        .is_empty(),
        "a description that quotes each command passes"
    );
    assert_eq!(
        hook_description_findings("runs `cargo clippy -D warnings`", &lines),
        [
            "names `cargo clippy -D warnings`, which no hook runs",
            "never names `scripts/check_style.py`, which a hook runs",
        ],
        "a shortened command is not the command, and a check left out is named"
    );
    assert_eq!(
        hook_description_findings("tests: `scripts/test-hooks.sh`", &[]),
        Vec::<String>::new(),
        "a script that tests the hooks is not a command they must run"
    );
    assert_eq!(
        hook_description_findings("runs `scripts/check_package.py`", &[]),
        ["names `scripts/check_package.py`, which no hook runs"],
        "a script that does not name the hooks must be run by one"
    );
}

// The README against the site and the code.

#[test]
fn the_readme_status_is_the_home_page_status() {
    let home = read("website/content/_index.md");
    let note = home
        .lines()
        .find(|line| line.starts_with("> **Status:"))
        .expect("the home page has a status note");
    let site = status_sentence(note).expect("the note has a bold sentence");
    let readme_status = status_sentence(&readme());
    assert_eq!(
        readme_status.as_deref(),
        Some(site.as_str()),
        "README.md's status is not the home page's"
    );
}

#[test]
fn every_release_the_readme_names_is_this_release() {
    let version = env!("CARGO_PKG_VERSION");
    let readme = readme();
    let others: Vec<String> = release_numbers(&readme)
        .into_iter()
        .filter(|number| number != version)
        .collect();
    assert!(
        others.is_empty(),
        "README.md names {others:?}; the release is {version}"
    );
    assert!(
        readme.contains(version),
        "README.md never names the release, {version}"
    );
}

#[test]
fn every_site_page_the_readme_links_exists() {
    let readme = readme();
    assert!(
        link_targets(&readme)
            .iter()
            .any(|t| site_source(t).is_some()),
        "README.md links no page of the site"
    );
    let missing = site_link_findings(&readme);
    assert!(
        missing.is_empty(),
        "README.md links what the site lacks: {missing:?}"
    );
}

#[test]
fn every_path_the_readme_names_exists() {
    let paths = named_paths(&readme());
    assert!(!paths.is_empty(), "README.md names no path");
    let missing: Vec<&String> = paths
        .iter()
        .filter(|path| !root().join(path).exists())
        .collect();
    assert!(
        missing.is_empty(),
        "README.md names paths that do not exist: {missing:?}"
    );
}

#[test]
fn the_readme_describes_the_hooks_as_they_are() {
    let readme = readme();
    let findings = hook_description_findings(section(&readme, "Contributing"), &hook_lines());
    assert!(
        findings.is_empty(),
        "README.md's Contributing: {findings:?}"
    );
}

#[test]
fn the_development_page_describes_the_hooks_as_they_are() {
    let page = read("website/content/docs/development.md");
    let findings = hook_description_findings(section(&page, "Gates"), &hook_lines());
    assert!(findings.is_empty(), "development.md's Gates: {findings:?}");
}

#[test]
fn the_readme_points_to_the_release_install_and_its_checks() {
    let readme = readme();
    let install = read("website/content/docs/install.md");
    for (readme_says, page_runs) in [
        ("`SHA256SUMS.txt`", "sha256sum -c SHA256SUMS.txt"),
        (
            "`SHA256SUMS.txt.sigstore.json`",
            "cosign verify-blob SHA256SUMS.txt",
        ),
    ] {
        assert!(
            readme.contains(readme_says),
            "README.md never names {readme_says}"
        );
        assert!(
            install.contains(page_runs),
            "the install page no longer runs {page_runs}, which README.md points to"
        );
    }
    assert!(
        link_targets(&readme)
            .iter()
            .any(|t| t.starts_with("https://vcon-server.com/docs/install/")),
        "README.md does not link the install page"
    );
}

#[test]
fn the_readme_uses_us_spelling() {
    let found = non_us_spellings(&readme());
    assert!(
        found.is_empty(),
        "README.md spells {found:?} other than in US English"
    );
}

#[test]
fn the_spelling_reader_finds_each_word_in_any_case() {
    assert_eq!(
        non_us_spellings("Returns them to Authorised readers, in colour."),
        ["authorised", "colour"],
        "each listed word, in the list's order, whatever its case"
    );
    assert!(
        non_us_spellings("Returns them to authorized readers.").is_empty(),
        "US spelling is not reported"
    );
    assert_eq!(
        non_us_spellings("Labelled keys, never re-serialised or normalised, for a contact-centre."),
        ["labelled", "normalis", "serialis", "centre"],
        "a stem finds each of its forms, and a word inside a hyphenated one"
    );
    assert!(
        non_us_spellings(
            "Labeled keys, serialized and normalized at the center, canceled, in the catalog; \
             it fulfills and analyzes the analyses, as a license allows."
        )
        .is_empty(),
        "the US forms of the listed words are not reported"
    );
}

/// Every file under `dir` whose name ends with `suffix`.
fn files_under(dir: &Path, suffix: &str, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            files_under(&path, suffix, found);
        } else if path.to_string_lossy().ends_with(suffix) {
            found.push(path);
        }
    }
}

/// The site's pages, its llms.txt and the OpenAPI description the server
/// serves are read by the same people as the README, in the same
/// language. The walk is checked to reach the pages the README links.
#[test]
fn the_site_and_the_openapi_description_use_us_spelling() {
    let mut pages = Vec::new();
    files_under(&root().join("website/content"), ".md", &mut pages);
    assert!(
        pages
            .iter()
            .any(|p| p.ends_with("website/content/docs/install.md")),
        "the walk reached the docs pages: {pages:?}"
    );
    pages.push(root().join("website/static/llms.txt"));
    pages.push(root().join("crates/vcon-server/openapi.json"));
    pages.push(root().join("website/static/openapi.json"));
    let mut wrong = Vec::new();
    for path in &pages {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let found = non_us_spellings(&text);
        if !found.is_empty() {
            wrong.push(format!("{}: {found:?}", path.display()));
        }
    }
    assert!(
        wrong.is_empty(),
        "spelled other than in US English: {wrong:#?}"
    );
}

/// Every `Cargo.toml` under `dir`, outside build output, hidden
/// directories and `node_modules`.
fn manifests_under(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry
            .file_type()
            .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()));
        if kind.is_dir() {
            if !(name.starts_with('.') || name == "target" || name == "node_modules") {
                manifests_under(&path, found);
            }
        } else if name == "Cargo.toml" {
            found.push(path);
        }
    }
}

/// Each package's `description`, which crates.io shows beside its
/// README, is held to the README's spelling rule. The walk is checked to
/// reach every workspace member, so a member it skipped would fail here
/// rather than go unread.
#[test]
fn each_package_description_uses_us_spelling() {
    let mut manifests = Vec::new();
    manifests_under(&root(), &mut manifests);
    let workspace: toml::Table = read("Cargo.toml").parse().expect("Cargo.toml parses");
    let members = workspace["workspace"]["members"]
        .as_array()
        .expect("the workspace lists its members");
    let read_paths: BTreeSet<PathBuf> = manifests
        .iter()
        .map(|p| p.canonicalize().expect("a manifest path"))
        .collect();
    for member in members {
        let member = member.as_str().expect("a member is a path");
        let path = root()
            .join(member)
            .join("Cargo.toml")
            .canonicalize()
            .expect("a member's manifest");
        assert!(
            read_paths.contains(&path),
            "the walk did not reach {member}/Cargo.toml"
        );
    }
    let mut described = 0;
    for path in &manifests {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let manifest: toml::Table = text
            .parse()
            .unwrap_or_else(|e| panic!("{} parses: {e}", path.display()));
        let Some(description) = manifest
            .get("package")
            .and_then(|p| p.get("description"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };
        described += 1;
        let found = non_us_spellings(description);
        assert!(
            found.is_empty(),
            "{}'s description spells {found:?} other than in US English",
            path.display()
        );
    }
    assert!(
        described >= members.len(),
        "{described} descriptions read, fewer than the {} workspace members",
        members.len()
    );
}

// The README's list against the status page.

/// The bold labels of the list items in `text`: each line starting
/// `- **`, the text up to the closing `**`, without a trailing `.` or `:`.
fn bold_labels(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix("- **"))
        .filter_map(|rest| rest.split_once("**"))
        .map(|(label, _)| label.trim_end_matches(['.', ':']).to_string())
        .collect()
}

#[test]
fn the_label_reader_takes_each_bold_item() {
    assert_eq!(
        bold_labels("- **One.** a\n- **`two`:** b\n  - **not** nested\n- plain\n- **Three** c\n"),
        ["One", "`two`", "Three"],
        "top-level bold items, punctuation off"
    );
}

#[test]
fn the_readme_lists_what_the_status_page_lists() {
    let status = read("website/content/docs/status.md");
    let site = bold_labels(section(&status, "Built and tested"));
    assert!(site.len() >= 10, "the status page's list: {site:?}");
    let ours = bold_labels(section(&readme(), "What is built"));
    assert_eq!(
        ours, site,
        "README.md's What is built is not the status page's Built and tested"
    );
}

// The crates' READMEs.

/// The crates' READMEs, by path.
const CRATE_READMES: [&str; 2] = ["crates/vcon-server/README.md", "crates/vcon/README.md"];

#[test]
fn each_crate_readme_has_the_home_page_status() {
    let home = read("website/content/_index.md");
    let note = home
        .lines()
        .find(|line| line.starts_with("> **Status:"))
        .expect("the home page has a status note");
    let site = status_sentence(note).expect("the note has a bold sentence");
    for path in CRATE_READMES {
        assert_eq!(
            status_sentence(&read(path)).as_deref(),
            Some(site.as_str()),
            "{path}'s status is not the home page's"
        );
    }
}

#[test]
fn each_crate_readme_names_only_this_release_and_links_real_pages() {
    let version = env!("CARGO_PKG_VERSION");
    for path in CRATE_READMES {
        let text = read(path);
        let others: Vec<String> = release_numbers(&text)
            .into_iter()
            .filter(|number| number != version)
            .collect();
        assert!(
            others.is_empty(),
            "{path} names {others:?}; the release is {version}"
        );
        let missing = site_link_findings(&text);
        assert!(
            missing.is_empty(),
            "{path} links what the site lacks: {missing:?}"
        );
        let found = non_us_spellings(&text);
        assert!(
            found.is_empty(),
            "{path} spells {found:?} other than in US English"
        );
    }
}

#[test]
fn the_server_crate_readme_links_the_mcp_and_s3_pages() {
    let text = read("crates/vcon-server/README.md");
    let targets = link_targets(&text);
    for page in [
        "https://vcon-server.com/docs/mcp/",
        "https://vcon-server.com/docs/s3-storage/",
    ] {
        assert!(
            targets.iter().any(|t| t.starts_with(page)),
            "crates/vcon-server/README.md does not link {page}"
        );
    }
}

/// The value of `NAME = <value>` in a Python script, quotes removed.
fn python_constant(script: &str, name: &str) -> String {
    let start = format!("{name} = ");
    let line = script
        .lines()
        .find(|line| line.starts_with(&start))
        .unwrap_or_else(|| panic!("the script has no line `{start}`"));
    line[start.len()..].trim().trim_matches('"').to_owned()
}

#[test]
fn contributing_states_the_dependency_age_exception_as_the_gate_reads_it() {
    let script = read("scripts/check_dependency_age.py");
    let form = python_constant(&script, "FORM");
    let days = python_constant(&script, "MIN_AGE_DAYS");
    assert!(
        form.starts_with("# dependency-age-exception: <crate> "),
        "the gate's form is not the line it reads: {form}"
    );
    let contributing = read("CONTRIBUTING.md");
    let rule = section(&contributing, "Dependency versions");
    for needed in [
        format!("```toml\n{form}\n```"),
        format!("at least {days} days"),
        "`scripts/check_dependency_age.py`".to_owned(),
        "--registry --base origin/main".to_owned(),
    ] {
        assert!(
            rule.contains(&needed),
            "CONTRIBUTING.md's Dependency versions does not hold: {needed}"
        );
    }
    let page = read("website/content/docs/development.md");
    assert!(
        section(&page, "Contributing").contains("`scripts/check_dependency_age.py`"),
        "development.md's Contributing does not name the gate"
    );
}

#[test]
fn the_past_exceptions_are_written_in_the_form() {
    // parquet 60.2.0, rmcp 3.9.0 and rusqlite 0.42.0 were taken inside the
    // 14 days as security fixes; rmcp and rusqlite each with crates of its
    // own that were as young.
    let manifest = read("crates/vcon-server/Cargo.toml");
    let lines: Vec<&str> = manifest
        .lines()
        .filter(|line| line.contains("dependency-age-exception"))
        .collect();
    assert_eq!(
        lines,
        [
            "# dependency-age-exception: parquet 60.2.0 RUSTSEC-2026-0595 published 2026-11-21",
            "# dependency-age-exception: rmcp 3.9.0 RUSTSEC-2026-0462 published 2026-11-05",
            "# dependency-age-exception: rmcp-macros 3.9.0 RUSTSEC-2026-0462 published 2026-11-05",
            "# dependency-age-exception: rusqlite 0.42.0 RUSTSEC-2026-0459 published 2026-11-11",
            "# dependency-age-exception: libsqlite3-sys 0.40.0 RUSTSEC-2026-0459 published 2026-11-11",
            "# dependency-age-exception: sqlite-wasm-rs 0.6.3 RUSTSEC-2026-0459 published 2026-11-10",
        ],
        "the exception lines of crates/vcon-server/Cargo.toml"
    );
}

// The note that this project is not the vCon Conserver (owner, 2026-11-16:
// "we should note on the web page that this is not the conserver and we
// should provide a link to the conserver instead"), on the home page under
// the lead and in the README above the status line.

const NOTICE: &str = "This vcon-server, an encrypted vCon store with retention, legal holds and an audit log, \
is not the vCon Conserver. The Conserver, a pipeline that runs vCons through chains of processing steps, \
is at [github.com/vcon-dev/vcon-server](https://github.com/vcon-dev/vcon-server).";

const HOME: &str =
    "+++\ntitle = \"vcon-server\"\n[extra]\ntagline = \"t\"\nnotice = \"N\"\n+++\n\nBody.\n";

const HERO: &str = "<section class=\"hero\">\n  <h1>{{ x.headline }}</h1>\n  <p class=\"lead\">{{ section.description }}</p>\n  \
<p class=\"notice\">{{ x.notice | markdown(inline=true) | safe }}</p>\n  <p class=\"cta\">\n  </p>\n</section>\n";

fn readme_with(notice: &str) -> String {
    format!(
        "# vcon-server\n\nA server that stores vCons.\n\n{notice}\n\n**Status: 0.4.7 released.** More.\n"
    )
}

fn home_with(notice: &str) -> String {
    HOME.replace("\"N\"", &format!("{notice:?}"))
}

// What must work.

#[test]
fn the_notice_and_its_readme_copy_pass() {
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &readme_with(NOTICE)),
        Vec::<String>::new(),
        "the note as written"
    );
}

#[test]
fn a_readme_copy_wrapped_over_lines_passes() {
    let wrapped = NOTICE
        .replace(" is not the", "\nis not the")
        .replace(" a pipeline", "\na pipeline");
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &readme_with(&wrapped)),
        Vec::<String>::new(),
        "line breaks are spaces"
    );
}

#[test]
fn the_front_matter_notice_is_read() {
    assert_eq!(
        front_matter_extra(&home_with("A note."), "notice").as_deref(),
        Some("A note."),
        "[extra] notice"
    );
}

#[test]
fn another_extra_key_is_read_the_same_way() {
    assert_eq!(
        front_matter_extra(HOME, "tagline").as_deref(),
        Some("t"),
        "[extra] tagline"
    );
}

#[test]
fn the_hero_template_passes() {
    assert_eq!(
        hero_notice_findings(HERO),
        Vec::<String>::new(),
        "after the lead, before the buttons, as markdown"
    );
}

#[test]
fn the_notice_names_the_conserver_url() {
    assert!(
        NOTICE.contains(&format!("]({CONSERVER_URL})")),
        "a markdown link to the Conserver"
    );
}

#[test]
fn the_url_is_the_conserver_repository() {
    assert_eq!(
        CONSERVER_URL, "https://github.com/vcon-dev/vcon-server",
        "the owner's link"
    );
}

#[test]
fn the_real_home_page_and_readme_carry_the_notice() {
    let home = read("website/content/_index.md");
    let notice = front_matter_extra(&home, "notice");
    assert_eq!(
        conserver_notice_findings(notice.as_deref(), &readme()),
        Vec::<String>::new(),
        "the home page and README.md"
    );
}

#[test]
fn the_real_crate_readme_carries_the_notice() {
    // crates.io shows this README on the vcon-server crate's page.
    let crate_readme = read("crates/vcon-server/README.md");
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &crate_readme),
        Vec::<String>::new(),
        "crates/vcon-server/README.md"
    );
}

#[test]
fn the_real_notice_is_the_one_written_here() {
    let home = read("website/content/_index.md");
    assert_eq!(
        front_matter_extra(&home, "notice").as_deref(),
        Some(NOTICE),
        "the home page's notice"
    );
}

#[test]
fn the_real_hero_renders_the_notice() {
    assert_eq!(
        hero_notice_findings(&read("website/templates/index.html")),
        Vec::<String>::new(),
        "index.html's hero"
    );
}

#[test]
fn the_real_producers_page_names_the_conserver_link() {
    let page = read("website/content/docs/producers.md");
    assert!(
        page.contains(&format!("]({CONSERVER_URL})")),
        "producers.md, which names a conserver pipeline, links to it"
    );
}

#[test]
fn the_real_notice_says_what_this_project_is() {
    let home = read("website/content/_index.md");
    let notice = front_matter_extra(&home, "notice").unwrap_or_default();
    assert!(
        notice.contains("an encrypted vCon store with retention, legal holds and an audit log"),
        "which project the reader is on: {notice}"
    );
}

#[test]
fn a_readme_with_the_notice_at_the_very_top_passes() {
    let readme = format!("{NOTICE}\n\n# vcon-server\n\n**Status: 0.4.7 released.**\n");
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &readme),
        Vec::<String>::new(),
        "before the status line"
    );
}

// What must be refused.

#[test]
fn no_notice_is_refused() {
    assert_eq!(
        conserver_notice_findings(None, &readme_with(NOTICE)),
        ["the home page has no notice"],
        "missing"
    );
}

#[test]
fn an_empty_notice_is_refused() {
    assert_eq!(
        conserver_notice_findings(Some("  "), &readme_with(NOTICE)),
        ["the home page has no notice"],
        "blank"
    );
}

#[test]
fn a_notice_without_the_link_is_refused() {
    let notice = NOTICE.replace(
        "[github.com/vcon-dev/vcon-server](https://github.com/vcon-dev/vcon-server)",
        "github.com/vcon-dev/vcon-server",
    );
    assert_eq!(
        conserver_notice_findings(Some(&notice), &readme_with(&notice)),
        ["the notice does not link to https://github.com/vcon-dev/vcon-server"],
        "a bare name is not a link"
    );
}

#[test]
fn a_notice_linking_elsewhere_is_refused() {
    let notice = NOTICE.replace(
        "](https://github.com/vcon-dev/vcon-server)",
        "](https://github.com/NormB/vcon-server)",
    );
    assert_eq!(
        conserver_notice_findings(Some(&notice), &readme_with(&notice)),
        ["the notice does not link to https://github.com/vcon-dev/vcon-server"],
        "this repository is not the Conserver"
    );
}

#[test]
fn a_notice_linking_into_the_repository_is_refused() {
    let notice = NOTICE.replace(
        "](https://github.com/vcon-dev/vcon-server)",
        "](https://github.com/vcon-dev/vcon-server/tree/main)",
    );
    assert_eq!(
        conserver_notice_findings(Some(&notice), &readme_with(&notice)).len(),
        1,
        "the repository itself, exactly"
    );
}

#[test]
fn a_notice_that_does_not_say_it_is_not_the_conserver_is_refused() {
    let notice = NOTICE.replace(
        "is not the vCon Conserver",
        "differs from the vCon Conserver",
    );
    assert_eq!(
        conserver_notice_findings(Some(&notice), &readme_with(&notice)),
        ["the notice does not say this project is not the vCon Conserver"],
        "the plain statement"
    );
}

#[test]
fn a_notice_that_does_not_say_what_this_is_is_refused() {
    let notice = NOTICE.replace(
        "an encrypted vCon store with retention, legal holds and an audit log",
        "a project",
    );
    assert_eq!(
        conserver_notice_findings(Some(&notice), &readme_with(&notice)),
        ["the notice does not say what this project is"],
        "the reader must know which one they are on"
    );
}

#[test]
fn a_notice_over_two_lines_is_refused() {
    let notice = NOTICE.replace(" The Conserver", "\nThe Conserver");
    assert!(
        conserver_notice_findings(Some(&notice), &readme_with(&notice))
            .contains(&"the notice is more than one line".to_owned()),
        "one line"
    );
}

#[test]
fn a_readme_without_the_notice_is_refused() {
    assert_eq!(
        conserver_notice_findings(
            Some(NOTICE),
            "# vcon-server\n\n**Status: 0.4.7 released.**\n"
        ),
        ["README.md does not carry the notice word for word"],
        "README = site"
    );
}

#[test]
fn a_readme_with_other_words_is_refused() {
    let other = NOTICE.replace("chains of processing steps", "processing steps");
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &readme_with(&other)),
        ["README.md does not carry the notice word for word"],
        "word for word"
    );
}

#[test]
fn a_readme_with_the_notice_after_the_status_is_refused() {
    let readme = format!("# vcon-server\n\n**Status: 0.4.7 released.**\n\n{NOTICE}\n");
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &readme),
        ["README.md carries the notice after its status line"],
        "near the top"
    );
}

#[test]
fn a_hero_without_the_notice_is_refused() {
    let hero = HERO.replace(
        "  <p class=\"notice\">{{ x.notice | markdown(inline=true) | safe }}</p>\n",
        "",
    );
    assert_eq!(
        hero_notice_findings(&hero),
        ["the home page's hero does not render x.notice"],
        "missing"
    );
}

#[test]
fn a_notice_before_the_lead_is_refused() {
    let hero = HERO
        .replace(
            "  <p class=\"notice\">{{ x.notice | markdown(inline=true) | safe }}</p>\n",
            "",
        )
        .replace(
            "  <h1>",
            "  <p class=\"notice\">{{ x.notice | markdown(inline=true) | safe }}</p>\n  <h1>",
        );
    assert_eq!(
        hero_notice_findings(&hero),
        ["the notice is not directly after the lead"],
        "under the lead"
    );
}

#[test]
fn a_notice_after_the_buttons_is_refused() {
    let hero = HERO
        .replace("  <p class=\"notice\">{{ x.notice | markdown(inline=true) | safe }}</p>\n", "")
        .replace("  </p>\n</section>", "  </p>\n  <p class=\"notice\">{{ x.notice | markdown(inline=true) | safe }}</p>\n</section>");
    assert_eq!(
        hero_notice_findings(&hero),
        ["the notice is not directly after the lead"],
        "above the buttons"
    );
}

#[test]
fn a_notice_not_rendered_as_markdown_is_refused() {
    let hero = HERO.replace(" | markdown(inline=true) | safe", "");
    assert_eq!(
        hero_notice_findings(&hero),
        ["the home page's hero does not render x.notice"],
        "the link must render"
    );
}

#[test]
fn a_notice_outside_the_hero_is_refused() {
    let template = HERO.replace(
        "  <p class=\"notice\">{{ x.notice | markdown(inline=true) | safe }}</p>\n",
        "",
    ) + "<p class=\"notice\">{{ x.notice | markdown(inline=true) | safe }}</p>\n";
    assert_eq!(
        hero_notice_findings(&template),
        ["the home page's hero does not render x.notice"],
        "below the fold"
    );
}

#[test]
fn a_template_without_a_hero_is_refused() {
    assert_eq!(
        hero_notice_findings("<main></main>"),
        ["the home page has no hero section"],
        "no hero"
    );
}

#[test]
fn front_matter_without_the_key_is_none() {
    assert_eq!(front_matter_extra(HOME, "notice_x"), None, "no such key");
}

#[test]
fn a_page_without_front_matter_is_none() {
    assert_eq!(
        front_matter_extra("# Title\n", "notice"),
        None,
        "no +++ block"
    );
}

// The edges.

#[test]
fn front_matter_that_is_not_toml_is_none() {
    assert_eq!(
        front_matter_extra("+++\n[extra\nnotice = 1\n+++\n", "notice"),
        None,
        "unparsable"
    );
}

#[test]
fn a_key_outside_extra_is_not_read() {
    assert_eq!(
        front_matter_extra("+++\nnotice = \"top\"\n[extra]\n+++\n", "notice"),
        None,
        "only [extra]"
    );
}

#[test]
fn a_number_is_not_a_notice() {
    assert_eq!(
        front_matter_extra("+++\n[extra]\nnotice = 3\n+++\n", "notice"),
        None,
        "a string only"
    );
}

#[test]
fn a_front_matter_end_inside_a_value_does_not_cut_it() {
    let home = home_with("a +++ b");
    assert_eq!(
        front_matter_extra(&home, "notice").as_deref(),
        Some("a +++ b"),
        "the closing line, not the text"
    );
}

#[test]
fn surrounding_spaces_of_the_notice_are_trimmed() {
    let padded = format!("  {NOTICE}  ");
    assert_eq!(
        conserver_notice_findings(Some(&padded), &readme_with(NOTICE)),
        Vec::<String>::new(),
        "trimmed"
    );
}

#[test]
fn several_findings_are_all_named() {
    let found = conserver_notice_findings(Some("A note."), "# vcon-server\n");
    assert_eq!(
        found.len(),
        4,
        "link, not-the-Conserver, what this is, README: {found:?}"
    );
}

#[test]
fn a_readme_without_a_status_line_needs_only_the_notice() {
    let readme = format!("# vcon-server\n\n{NOTICE}\n");
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &readme),
        Vec::<String>::new(),
        "nothing to be above"
    );
}

#[test]
fn a_double_space_in_the_notice_matches_one_in_the_readme() {
    let spaced = NOTICE.replace("is at [", "is at  [");
    assert_eq!(
        conserver_notice_findings(Some(&spaced), &readme_with(NOTICE)),
        Vec::<String>::new(),
        "whitespace on both sides"
    );
}

#[test]
fn tabs_in_the_readme_copy_are_spaces() {
    let tabbed = NOTICE.replace(" is not", "\tis not");
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &readme_with(&tabbed)),
        Vec::<String>::new(),
        "whitespace"
    );
}

#[test]
fn the_notice_inside_a_longer_readme_paragraph_still_counts() {
    let readme = readme_with(&format!("Before. {NOTICE} After."));
    assert_eq!(
        conserver_notice_findings(Some(NOTICE), &readme),
        Vec::<String>::new(),
        "contained"
    );
}

#[test]
fn a_hero_with_spaces_inside_the_expression_is_refused() {
    let hero = HERO.replace(
        "{{ x.notice | markdown(inline=true) | safe }}",
        "{{x.notice|markdown(inline=true)|safe}}",
    );
    assert_eq!(
        hero_notice_findings(&hero),
        ["the home page's hero does not render x.notice"],
        "the one written form"
    );
}

#[test]
fn a_second_hero_section_is_not_read() {
    let template = format!("{HERO}<section class=\"hero\"></section>\n");
    assert_eq!(
        hero_notice_findings(&template),
        Vec::<String>::new(),
        "the first hero"
    );
}

#[test]
fn the_notice_between_the_lead_and_the_buttons_with_other_lines_passes() {
    let hero = HERO.replace(
        "  <p class=\"notice\">",
        "  <!-- the note -->\n  <p class=\"notice\">",
    );
    assert_eq!(
        hero_notice_findings(&hero),
        Vec::<String>::new(),
        "a comment beside it"
    );
}

#[test]
fn the_notice_uses_us_spelling() {
    assert_eq!(non_us_spellings(NOTICE), Vec::<&str>::new(), "US English");
}

#[test]
fn the_notice_names_no_rename() {
    assert!(
        !NOTICE.contains("rename") && !NOTICE.contains("formerly"),
        "the name stays (owner, 2026-11-16)"
    );
}

#[test]
fn the_notice_judges_nothing() {
    for word in ["better", "worse", "official", "real ", "only ", "unlike"] {
        assert!(
            !NOTICE.contains(word),
            "facts only, no judgement of the other project: {word}"
        );
    }
}

#[test]
fn the_squeeze_joins_any_whitespace() {
    assert_eq!(squeeze(" a\n\tb  c "), "a b c", "one space between words");
}

#[test]
fn the_squeeze_of_nothing_is_empty() {
    assert_eq!(squeeze(" \n "), "", "no words");
}

// ---- Each place a release names its own version. ----
//
// A release commit moves these to the new version by hand. On 2026-11-26,
// putting each back to the previous version one at a time while the crate
// said the new one failed no test for these: they are held here, each to
// the crate's version (and the lists to the changelog's releases).

/// A file and the words around the version it names, `{v}` where the
/// version stands.
const PINS: [(&str, &str); 5] = [
    (
        "website/content/docs/library.md",
        "This page describes vcon {v}.",
    ),
    (
        "website/content/docs/tui.md",
        "The screens on this page are those of releases 0.4.7 to {v}.",
    ),
    (
        "website/content/docs/development.md",
        "python3 -I scripts/release_notes.py {v} --repo NormB/vcon-server",
    ),
    ("website/content/_index.md", "The {v} binaries need glibc"),
    (
        ".bestpractices.json",
        "the current version is {v} (Cargo.toml)",
    ),
];

/// The `.bestpractices.json` answers that list every release.
const RELEASE_LISTS: [&str; 3] = [
    "version_unique_justification",
    "version_tags_justification",
    "release_notes_justification",
];

/// What is wrong with `text` (of `file`) for the pin `template` and the
/// release `version`: it never says the words, or says them with another
/// version.
fn pin_findings(file: &str, text: &str, template: &str, version: &str) -> Vec<String> {
    let text = squeeze(text);
    let (before, after) = template.split_once("{v}").expect("a template has {v}");
    let mut found = Vec::new();
    let mut seen = false;
    let mut rest = text.as_str();
    while let Some(at) = rest.find(before) {
        let tail = &rest[at + before.len()..];
        rest = tail;
        // The version is the run of digits and dots there, less a dot
        // that ends a sentence, and the template's words follow it.
        let run = tail.len() - tail.trim_start_matches(is_number_char).len();
        let named = tail[..run].trim_end_matches('.');
        if release_numbers(named) != [named.to_owned()] || !tail[named.len()..].starts_with(after) {
            continue;
        }
        seen = true;
        if named != version {
            found.push(format!(
                "{file} says {:?}; the release is {version}",
                template.replace("{v}", named)
            ));
        }
    }
    if !seen {
        found.push(format!(
            "{file} never says {:?}",
            template.replace("{v}", version)
        ));
    }
    found
}

/// The released versions the changelog has a section for, oldest first.
fn released_versions(changelog: &str) -> Vec<String> {
    let mut versions: Vec<String> = changelog
        .lines()
        .filter_map(|line| line.strip_prefix("## ["))
        .filter_map(|rest| rest.split_once(']'))
        .map(|(name, _)| name.to_owned())
        .filter(|name| release_numbers(name) == [name.clone()])
        .collect();
    versions.sort_by_key(|v| {
        v.split('.')
            .map(|p| p.parse::<u32>().expect("a number"))
            .collect::<Vec<_>>()
    });
    versions
}

/// The `info.version` of an OpenAPI description.
fn openapi_version(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value["info"]["version"].as_str().map(str::to_owned)
}

#[test]
fn every_page_pin_names_this_release() {
    let version = env!("CARGO_PKG_VERSION");
    let mut findings = Vec::new();
    for (file, template) in PINS {
        findings.extend(pin_findings(file, &read(file), template, version));
    }
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn both_openapi_descriptions_name_this_release() {
    let version = env!("CARGO_PKG_VERSION");
    for file in [
        "crates/vcon-server/openapi.json",
        "website/static/openapi.json",
    ] {
        assert_eq!(
            openapi_version(&read(file)).as_deref(),
            Some(version),
            "{file}'s info.version is the crate's"
        );
    }
}

#[test]
fn the_best_practices_lists_name_every_released_version() {
    let released = released_versions(&read("CHANGELOG.md"));
    assert!(
        released.last().map(String::as_str) == Some(env!("CARGO_PKG_VERSION")),
        "the changelog's newest release is the crate's: {:?}",
        released.last()
    );
    let answers: serde_json::Value =
        serde_json::from_str(&read(".bestpractices.json")).expect("valid JSON");
    for key in RELEASE_LISTS {
        let text = answers[key]
            .as_str()
            .unwrap_or_else(|| panic!("{key} is text"));
        assert_eq!(
            release_numbers(text),
            released,
            "{key} lists the changelog's releases, oldest first"
        );
    }
}

#[test]
fn a_pin_with_this_version_passes() {
    assert!(
        pin_findings(
            "p",
            "This page describes vcon 0.4.18. In",
            PINS[0].1,
            "0.4.18"
        )
        .is_empty(),
        "the version stands where the template has it"
    );
}

#[test]
fn a_pin_across_a_line_break_passes() {
    assert!(
        pin_findings(
            "p",
            "those of releases\n0.4.7 to 0.4.18.",
            "those of releases 0.4.7 to {v}.",
            "0.4.18"
        )
        .is_empty(),
        "line breaks are spaces"
    );
}

#[test]
fn a_pin_with_another_version_is_named() {
    let found = pin_findings("p", "This page describes vcon 0.4.17.", PINS[0].1, "0.4.18");
    assert_eq!(
        found,
        ["p says \"This page describes vcon 0.4.17.\"; the release is 0.4.18"],
        "the stale words and the release"
    );
}

#[test]
fn a_missing_pin_is_named() {
    let found = pin_findings("p", "no such words", PINS[0].1, "0.4.18");
    assert_eq!(
        found,
        ["p never says \"This page describes vcon 0.4.18.\""],
        "the words it should say"
    );
}

#[test]
fn two_pins_one_stale_name_the_stale_one() {
    let found = pin_findings(
        "p",
        "The 0.4.18 binaries need glibc. The 0.4.17 binaries need glibc.",
        "The {v} binaries need glibc",
        "0.4.18",
    );
    assert_eq!(found.len(), 1, "one is stale: {found:?}");
    assert!(found[0].contains("0.4.17"), "names it: {found:?}");
}

#[test]
fn words_with_no_version_between_are_not_a_pin() {
    let found = pin_findings(
        "p",
        "The new binaries need glibc",
        "The {v} binaries need glibc",
        "0.4.18",
    );
    assert_eq!(
        found,
        ["p never says \"The 0.4.18 binaries need glibc\""],
        "a word is not a version"
    );
}

#[test]
fn released_versions_are_the_headings_oldest_first() {
    let changelog = "# Changelog\n\n## [Unreleased]\n\n## [0.4.10] - x\n\n## [0.4.9] - x\n\n## [0.1.0] - x\n\n[0.4.9]: https://example.com\n";
    assert_eq!(
        released_versions(changelog),
        ["0.1.0", "0.4.9", "0.4.10"],
        "by number, and not the link lines or [Unreleased]"
    );
}

#[test]
fn an_openapi_version_is_read_from_info() {
    assert_eq!(
        openapi_version(r#"{"info": {"version": "0.4.18"}}"#).as_deref(),
        Some("0.4.18"),
        "info.version"
    );
    assert_eq!(openapi_version(r#"{"info": {}}"#), None, "none there");
    assert_eq!(openapi_version("not json"), None, "not JSON");
}

#[test]
fn every_pin_template_has_one_version_place() {
    for (file, template) in PINS {
        assert_eq!(template.matches("{v}").count(), 1, "{file}: one {{v}}");
    }
}
