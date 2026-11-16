//! The 1.0 checklist (`website/content/docs/production-checklist.md`) says
//! what vcon-server needs before release 1.0, item by item, and links each
//! item to its evidence. These tests hold every claim on the page to the
//! tree:
//!
//! - the page has the six items, once each, in their order, and no other;
//! - each item states one of three states, once;
//! - each item has at least one piece of evidence, and each piece is one
//!   link of a kind that can be checked: a page of the site (and its
//!   heading), a file on `main`, a test (its file holds the function) or a
//!   CI job (its workflow defines the job);
//!   an item stated as done names a CI job;
//! - the page names no year: target dates are the owner's plan, not facts
//!   of the tree;
//! - the roadmap, the status page, the home page and the README link it.
//!
//! The checker is a set of pure functions over the page's text and a
//! [`Repo`], so its own tests drive it with pages and trees made here.
//! A file "on main" is checked to exist in the checked-out tree; the commit
//! hook runs these tests on a copy of exactly what is staged, so a file
//! there is a tracked one.

mod markdown;

use std::collections::BTreeMap;
use std::path::PathBuf;

use markdown::{heading_ids, is_relative, link_targets, slug};

/// The page, from the repository root.
const PAGE: &str = "website/content/docs/production-checklist.md";

/// The prefix of a link to a file on `main`.
const BLOB: &str = "https://github.com/NormB/vcon-server/blob/main/";

/// The items, in the order the page lists them.
const ITEMS: [&str; 6] = [
    "Several servers on one PostgreSQL database",
    "A backup restored in CI",
    "Keys kept in a KMS or an HSM",
    "An outside review of the cryptography",
    "Published scale numbers",
    "A frozen HTTP API, configuration and on-disk format",
];

/// The states an item can be in.
const STATES: [&str; 3] = ["Done", "Partly done", "Not started"];

/// The files a checker reads: the page's links are checked against them.
trait Repo {
    /// The text of the file at `path`, from the repository root, if the
    /// tree has it.
    fn text(&self, path: &str) -> Option<String>;
}

/// The checked-out tree.
struct Disk(PathBuf);

impl Repo for Disk {
    fn text(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(path)).ok()
    }
}

/// A tree made in a test.
struct Fake(BTreeMap<String, String>);

impl Repo for Fake {
    fn text(&self, path: &str) -> Option<String> {
        self.0.get(path).cloned()
    }
}

/// The repository root, two levels above this crate.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One piece of evidence, as the page links it.
#[derive(Debug, PartialEq, Eq)]
enum Evidence {
    /// A page of the site, `website/content/<path>`, and a heading on it.
    Page {
        path: String,
        anchor: Option<String>,
    },
    /// A file on `main`.
    File { path: String },
    /// A test: the file holds `fn <name>(`.
    Test { path: String, name: String },
    /// A CI job: the workflow defines the job `<id>`.
    Job { path: String, id: String },
}

/// One item of the page: its heading, its state lines and its evidence
/// lines (each list item of the evidence list, its lines joined).
#[derive(Debug, Default, PartialEq, Eq)]
struct Item {
    heading: String,
    states: Vec<String>,
    evidence: Vec<String>,
}

/// Where the reader of [`items`] is in an evidence list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum List {
    /// Not in one.
    Out,
    /// After the line `Evidence:`, before its first list item.
    Waiting,
    /// In its list items.
    In,
}

/// The items of a page: each `## ` heading outside a fenced block opens
/// one. In an item, a line `**State:** <state>.` states its state, and the
/// list items after the line `Evidence:` are its evidence, up to the first
/// line that is not blank, no list item and no list item's continuation.
fn items(text: &str) -> Vec<Item> {
    let mut found: Vec<Item> = Vec::new();
    let mut fenced = false;
    let mut list = List::Out;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            list = List::Out;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(heading) = line.strip_prefix("## ") {
            found.push(Item {
                heading: heading.trim().to_string(),
                ..Item::default()
            });
            list = List::Out;
            continue;
        }
        let Some(item) = found.last_mut() else {
            continue;
        };
        if let Some(state) = line.strip_prefix("**State:** ") {
            item.states
                .push(state.trim().trim_end_matches('.').to_string());
            list = List::Out;
        } else if line.trim() == "Evidence:" {
            list = List::Waiting;
        } else if list != List::Out {
            if let Some(entry) = line.strip_prefix("- ") {
                item.evidence.push(entry.trim().to_string());
                list = List::In;
            } else if line.trim().is_empty() {
                // A blank line between list items keeps the list open, as
                // Markdown reads a loose list.
            } else if list == List::In && line.starts_with("  ") {
                let last = item
                    .evidence
                    .last_mut()
                    .expect("a list item before its continuation");
                last.push(' ');
                last.push_str(line.trim());
            } else {
                list = List::Out;
            }
        }
    }
    found
}

/// The text between `marker` and the next backtick in `line`: the name in
/// "test `name`".
fn named<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let at = line.find(marker)? + marker.len();
    let rest = &line[at..];
    Some(&rest[..rest.find('`')?])
}

/// What one evidence line links, or why it is not evidence.
fn evidence(line: &str) -> Result<Evidence, String> {
    let targets = link_targets(line);
    let [target] = targets.as_slice() else {
        return Err(format!(
            "`{line}` must hold one link: a page of the site, a file on main, a test or a CI job"
        ));
    };
    if let Some(rest) = target.strip_prefix("@/") {
        let (path, anchor) = match rest.split_once('#') {
            Some((_, "")) => return Err(format!("`{target}` has an empty anchor")),
            Some((path, anchor)) => (path, Some(anchor.to_string())),
            None => (rest, None),
        };
        return Ok(Evidence::Page {
            path: format!("website/content/{path}"),
            anchor,
        });
    }
    if let Some(path) = target.strip_prefix(BLOB) {
        if path.is_empty() {
            return Err(format!("`{target}` is not evidence: it names no file"));
        }
        if let Some((_, line_anchor)) = path.split_once('#') {
            return Err(format!(
                "`{target}` links a line (#{line_anchor}), which moves when the file changes: link the file"
            ));
        }
        let path = path.to_string();
        if let Some(name) = named(line, "test `") {
            return Ok(Evidence::Test {
                path,
                name: name.to_string(),
            });
        }
        if let Some(id) = named(line, "CI job `") {
            if !path.starts_with(".github/workflows/") {
                return Err(format!(
                    "`{path}` is not a workflow, so it defines no CI job `{id}`"
                ));
            }
            return Ok(Evidence::Job {
                path,
                id: id.to_string(),
            });
        }
        return Ok(Evidence::File { path });
    }
    if is_relative(target) {
        return Err(format!(
            "`{target}` is a relative link, which the site cannot open: link the file on main ({BLOB}<path>)"
        ));
    }
    Err(format!(
        "`{target}` is not evidence: link a page of the site (@/<page>), a file on main ({BLOB}<path>), a test or a CI job"
    ))
}

/// The ids of the jobs a workflow defines: the keys two spaces in, under
/// `jobs:`.
fn job_ids(workflow: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut in_jobs = false;
    for line in workflow.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') {
            in_jobs = line.trim_end() == "jobs:";
        } else if in_jobs
            && let Some(key) = line.strip_prefix("  ")
            && !key.starts_with(' ')
            && let Some(id) = key.trim_end().strip_suffix(':')
        {
            ids.push(id.to_string());
        }
    }
    ids
}

/// Why `found` does not hold in `repo`, if it does not.
fn missing(found: &Evidence, repo: &dyn Repo) -> Option<String> {
    let path = match found {
        Evidence::Page { path, .. }
        | Evidence::File { path }
        | Evidence::Test { path, .. }
        | Evidence::Job { path, .. } => path,
    };
    let Some(text) = repo.text(path) else {
        return Some(format!("`{path}` is not in the tree"));
    };
    match found {
        Evidence::Page {
            anchor: Some(anchor),
            ..
        } if !heading_ids(&text).contains(anchor) => {
            Some(format!("`{path}` has no heading `#{anchor}`"))
        }
        Evidence::Test { name, .. } if !text.contains(&format!("fn {name}(")) => {
            Some(format!("`{path}` has no test `{name}`"))
        }
        Evidence::Job { id, .. } if !job_ids(&text).contains(id) => {
            Some(format!("`{path}` defines no job `{id}`"))
        }
        _ => None,
    }
}

/// Every problem of the page `text`, checked against `repo`; none when the
/// page holds. Each problem about an item starts with its heading.
fn problems(text: &str, repo: &dyn Repo) -> Vec<String> {
    let mut found = Vec::new();
    let page = items(text);
    for want in ITEMS {
        match page.iter().filter(|i| i.heading == want).count() {
            0 => found.push(format!("{want}: the page has no such item")),
            1 => {}
            _ => found.push(format!("{want}: the item is on the page more than once")),
        }
    }
    let places: Vec<usize> = page
        .iter()
        .filter_map(|i| ITEMS.iter().position(|h| *h == i.heading))
        .collect();
    if places.windows(2).any(|w| w[0] > w[1]) {
        found.push(format!(
            "the items are not in the order of the checklist: {ITEMS:?}"
        ));
    }
    for item in &page {
        let heading = &item.heading;
        if !ITEMS.contains(&heading.as_str()) {
            found.push(format!(
                "{heading}: not an item of the 1.0 checklist {ITEMS:?}"
            ));
        }
        match item.states.as_slice() {
            [] => found.push(format!(
                "{heading}: no state (a line `**State:** <state>.`)"
            )),
            [state] if !STATES.contains(&state.as_str()) => {
                found.push(format!(
                    "{heading}: the state {state:?} is not one of {STATES:?}"
                ));
            }
            [_] => {}
            _ => found.push(format!("{heading}: more than one state")),
        }
        if item.evidence.is_empty() {
            found.push(format!(
                "{heading}: no evidence (a list after the line `Evidence:`)"
            ));
        }
        let mut names_a_job = false;
        for line in &item.evidence {
            match evidence(line) {
                Err(why) => found.push(format!("{heading}: {why}")),
                Ok(piece) => {
                    names_a_job |= matches!(piece, Evidence::Job { .. });
                    if let Some(why) = missing(&piece, repo) {
                        found.push(format!("{heading}: {why}"));
                    }
                }
            }
        }
        if item.states == ["Done"] && !names_a_job {
            found.push(format!(
                "{heading}: stated done, but names no CI job that runs its evidence"
            ));
        }
    }
    for word in text.split(|c: char| !c.is_ascii_digit()) {
        if word.len() == 4 && word.starts_with("20") {
            found.push(format!(
                "the page names the year {word}: target dates are the owner's plan, not facts of the tree"
            ));
        }
    }
    found
}

/// Whether `text` links the checklist page: by `@/` for a page of the site,
/// by its URL for the README.
fn links_the_checklist(text: &str, from_site: bool) -> bool {
    let target = if from_site {
        "@/docs/production-checklist.md"
    } else {
        "https://vcon-server.com/docs/production-checklist/"
    };
    link_targets(text)
        .iter()
        .any(|t| t.split('#').next() == Some(target))
}

// ---- The page itself ----

#[test]
fn the_checklist_page_holds() {
    let disk = Disk(root());
    let text = disk.text(PAGE).expect("the checklist page exists");
    let found = problems(&text, &disk);
    assert!(found.is_empty(), "the checklist page: {found:#?}");
    assert_eq!(
        items(&text).len(),
        ITEMS.len(),
        "the page has the six items"
    );
}

#[test]
fn the_roadmap_status_and_home_pages_link_the_checklist() {
    let disk = Disk(root());
    for page in [
        "website/content/docs/roadmap.md",
        "website/content/docs/status.md",
        "website/content/_index.md",
    ] {
        let text = disk.text(page).expect("the page exists");
        assert!(
            links_the_checklist(&text, true),
            "{page} does not link the 1.0 checklist"
        );
    }
}

#[test]
fn the_readme_links_the_checklist() {
    let text = Disk(root()).text("README.md").expect("README.md");
    assert!(
        links_the_checklist(&text, false),
        "README.md does not link the 1.0 checklist"
    );
}

// ---- The checker, on pages and trees made here ----

const CI: &str = ".github/workflows/ci.yml";
const WORKFLOW: &str = "on:\n  push:\n  pull_request:\njobs:\n  rust:\n    steps:\n      - name: build\n  backends-postgres:\n    name: pg\n  docs_build:\n    runs-on: x\n";
const SERVE_RS: &str = "crates/vcon-server/tests/serve.rs";
const SERVE_TEXT: &str =
    "#[test]\nfn a_second_server_is_refused() {}\nfn a_backup_restores_extra() {}\n";
const STATUS: &str = "website/content/docs/status.md";
const STATUS_TEXT: &str =
    "+++\n+++\n\n## Built and tested\n\n### The `postgres` feature\n\n```\n## Not a heading\n```\n";

/// A tree with the files the made pages link.
fn tree() -> Fake {
    let mut files = BTreeMap::new();
    files.insert(CI.to_string(), WORKFLOW.to_string());
    files.insert(SERVE_RS.to_string(), SERVE_TEXT.to_string());
    files.insert(STATUS.to_string(), STATUS_TEXT.to_string());
    files.insert(
        "website/content/standards.md".to_string(),
        "## Drafts\n".to_string(),
    );
    files.insert("SECURITY.md".to_string(), "# Security policy\n".to_string());
    Fake(files)
}

fn page_link() -> String {
    "- [what is built](@/docs/status.md#built-and-tested)".to_string()
}

fn file_link(path: &str) -> String {
    format!("- [{path}]({BLOB}{path})")
}

fn test_link(name: &str) -> String {
    format!("- test `{name}` in [serve.rs]({BLOB}{SERVE_RS})")
}

fn job_link(id: &str) -> String {
    format!("- CI job `{id}` in [ci.yml]({BLOB}{CI})")
}

/// One item's section.
fn section(heading: &str, state: &str, evidence: &[String]) -> String {
    let mut text = format!(
        "## {heading}\n\n**State:** {state}.\n\nWhat is done, and what is left.\n\nEvidence:\n\n"
    );
    for line in evidence {
        text.push_str(line);
        text.push('\n');
    }
    text.push('\n');
    text
}

/// A page of the given sections, after front matter and an introduction.
fn page_of(sections: &[String]) -> String {
    let mut text = String::from("+++\ntitle = \"The 1.0 checklist\"\n+++\n\nWhat 1.0 needs.\n\n");
    for s in sections {
        text.push_str(s);
    }
    text
}

/// A page with every item, each partly done with a page link.
fn good_sections() -> Vec<String> {
    ITEMS
        .iter()
        .map(|h| section(h, "Partly done", &[page_link()]))
        .collect()
}

fn good_page() -> String {
    page_of(&good_sections())
}

/// The page with item `i` replaced by `with`.
fn page_with(i: usize, with: String) -> String {
    let mut sections = good_sections();
    sections[i] = with;
    page_of(&sections)
}

fn check(text: &str) -> Vec<String> {
    problems(text, &tree())
}

fn assert_holds(text: &str) {
    let found = check(text);
    assert!(found.is_empty(), "expected no problem, found {found:#?}");
}

fn assert_refused(text: &str, part: &str) {
    let found = check(text);
    assert!(
        found.iter().any(|p| p.contains(part)),
        "expected a problem naming {part:?}, found {found:#?}"
    );
}

// Success.

#[test]
fn a_page_with_every_item_holds() {
    assert_holds(&good_page());
}

#[test]
fn a_page_link_without_an_anchor_holds() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &["- [status](@/docs/status.md)".to_string()],
    );
    assert_holds(&page_with(0, item));
}

#[test]
fn a_page_link_to_a_third_level_heading_holds() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &["- [the feature](@/docs/status.md#the-postgres-feature)".to_string()],
    );
    assert_holds(&page_with(0, item));
}

#[test]
fn a_link_to_a_page_outside_docs_holds() {
    let item = section(
        ITEMS[1],
        "Partly done",
        &["- [standards](@/standards.md#drafts)".to_string()],
    );
    assert_holds(&page_with(1, item));
}

#[test]
fn a_file_on_main_holds() {
    let item = section(ITEMS[2], "Not started", &[file_link("SECURITY.md")]);
    assert_holds(&page_with(2, item));
}

#[test]
fn a_test_its_file_holds_is_evidence() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &[test_link("a_second_server_is_refused")],
    );
    assert_holds(&page_with(0, item));
}

#[test]
fn a_job_its_workflow_defines_is_evidence() {
    let item = section(ITEMS[0], "Partly done", &[job_link("backends-postgres")]);
    assert_holds(&page_with(0, item));
}

#[test]
fn a_done_item_with_a_ci_job_holds() {
    let item = section(ITEMS[1], "Done", &[page_link(), job_link("rust")]);
    assert_holds(&page_with(1, item));
}

#[test]
fn an_evidence_line_continued_on_the_next_line_holds() {
    let line = format!("- test `a_second_server_is_refused` in\n  [serve.rs]({BLOB}{SERVE_RS})");
    let item = section(ITEMS[0], "Partly done", &[line]);
    assert_holds(&page_with(0, item));
}

#[test]
fn every_kind_of_evidence_in_one_item_holds() {
    let item = section(
        ITEMS[5],
        "Done",
        &[
            page_link(),
            file_link("SECURITY.md"),
            test_link("a_backup_restores_extra"),
            job_link("docs_build"),
        ],
    );
    assert_holds(&page_with(5, item));
}

#[test]
fn a_state_without_its_full_stop_holds() {
    let item =
        section(ITEMS[3], "Not started", &[page_link()]).replace("Not started.", "Not started");
    assert_holds(&page_with(3, item));
}

#[test]
fn the_items_are_read_in_page_order() {
    let headings: Vec<String> = items(&good_page()).into_iter().map(|i| i.heading).collect();
    assert_eq!(
        headings,
        ITEMS.map(String::from).to_vec(),
        "the headings, in order"
    );
}

#[test]
fn an_item_reads_its_state_and_evidence() {
    let found = items(&good_page());
    assert_eq!(
        found[0].states,
        vec!["Partly done".to_string()],
        "the state"
    );
    assert_eq!(
        found[0].evidence,
        vec![page_link()[2..].to_string()],
        "the evidence line"
    );
}

#[test]
fn each_kind_of_link_is_read_as_its_kind() {
    assert_eq!(
        evidence("[s](@/docs/status.md#built-and-tested)"),
        Ok(Evidence::Page {
            path: STATUS.to_string(),
            anchor: Some("built-and-tested".to_string())
        }),
        "a page"
    );
    assert_eq!(
        evidence(&file_link("SECURITY.md")[2..]),
        Ok(Evidence::File {
            path: "SECURITY.md".to_string()
        }),
        "a file"
    );
    assert_eq!(
        evidence(&test_link("t")[2..]),
        Ok(Evidence::Test {
            path: SERVE_RS.to_string(),
            name: "t".to_string()
        }),
        "a test"
    );
    assert_eq!(
        evidence(&job_link("rust")[2..]),
        Ok(Evidence::Job {
            path: CI.to_string(),
            id: "rust".to_string()
        }),
        "a job"
    );
}

#[test]
fn the_jobs_of_a_workflow_are_read() {
    assert_eq!(
        job_ids(WORKFLOW),
        vec![
            "rust".to_string(),
            "backends-postgres".to_string(),
            "docs_build".to_string()
        ],
        "the three jobs, and nothing under `on:` or in a job"
    );
}

#[test]
fn the_link_readers_find_a_link_to_the_page() {
    assert!(
        links_the_checklist(
            "See [the 1.0 checklist](@/docs/production-checklist.md).",
            true
        ),
        "a site page's link"
    );
    assert!(
        links_the_checklist(
            "[The 1.0 checklist](https://vcon-server.com/docs/production-checklist/) says it.",
            false
        ),
        "the README's link"
    );
    let to_item = format!("[kms](@/docs/production-checklist.md#{})", slug(ITEMS[2]));
    assert!(
        links_the_checklist(&to_item, true),
        "a link to one of its headings: {to_item}"
    );
}

#[test]
fn a_relative_link_is_refused_as_evidence() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &["- [security](../../SECURITY.md)".to_string()],
    );
    assert_refused(&page_with(0, item), "relative");
}

// Failure.

#[test]
fn a_missing_item_is_refused() {
    let mut sections = good_sections();
    sections.remove(4);
    assert_refused(&page_of(&sections), ITEMS[4]);
}

#[test]
fn an_item_listed_twice_is_refused() {
    let mut sections = good_sections();
    sections.push(sections[0].clone());
    assert_refused(&page_of(&sections), "more than once");
}

#[test]
fn items_out_of_order_are_refused() {
    let mut sections = good_sections();
    sections.swap(0, 1);
    assert_refused(&page_of(&sections), "order");
}

#[test]
fn an_item_not_on_the_list_is_refused() {
    let mut sections = good_sections();
    sections.push(section("Faster builds", "Partly done", &[page_link()]));
    assert_refused(&page_of(&sections), "Faster builds");
}

#[test]
fn an_item_without_a_state_is_refused() {
    let item =
        section(ITEMS[0], "Partly done", &[page_link()]).replace("**State:** Partly done.\n", "");
    assert_refused(&page_with(0, item), "no state");
}

#[test]
fn an_unknown_state_is_refused() {
    let item = section(ITEMS[0], "Nearly there", &[page_link()]);
    assert_refused(&page_with(0, item), "Nearly there");
}

#[test]
fn two_states_in_one_item_are_refused() {
    let item = section(ITEMS[0], "Partly done", &[page_link()])
        .replace("Evidence:", "**State:** Done.\n\nEvidence:");
    assert_refused(&page_with(0, item), "more than one state");
}

#[test]
fn an_item_without_evidence_is_refused() {
    let item = section(ITEMS[2], "Not started", &[]);
    assert_refused(&page_with(2, item), "no evidence");
}

#[test]
fn a_link_to_a_page_the_site_lacks_is_refused() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &["- [kms](@/docs/kms.md)".to_string()],
    );
    assert_refused(&page_with(0, item), "website/content/docs/kms.md");
}

#[test]
fn a_link_to_a_heading_the_page_lacks_is_refused() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &["- [s](@/docs/status.md#being-built)".to_string()],
    );
    assert_refused(&page_with(0, item), "being-built");
}

#[test]
fn a_file_the_tree_lacks_is_refused() {
    let item = section(ITEMS[0], "Partly done", &[file_link("docs/kms.md")]);
    assert_refused(&page_with(0, item), "docs/kms.md");
}

#[test]
fn a_link_to_a_line_of_a_file_is_refused() {
    let line = format!("- [envelope]({BLOB}SECURITY.md#L12)");
    let item = section(ITEMS[0], "Partly done", &[line]);
    assert_refused(&page_with(0, item), "links a line (#L12)");
}

#[test]
fn a_test_its_file_lacks_is_refused() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &[test_link("a_third_server_is_refused")],
    );
    assert_refused(&page_with(0, item), "a_third_server_is_refused");
}

#[test]
fn a_job_the_workflow_lacks_is_refused() {
    let item = section(ITEMS[0], "Partly done", &[job_link("scale")]);
    assert_refused(&page_with(0, item), "scale");
}

#[test]
fn a_job_in_a_file_that_is_no_workflow_is_refused() {
    let line = format!("- CI job `rust` in [serve.rs]({BLOB}{SERVE_RS})");
    let item = section(ITEMS[0], "Partly done", &[line]);
    assert_refused(&page_with(0, item), "not a workflow");
}

#[test]
fn a_done_item_without_a_ci_job_is_refused() {
    let item = section(
        ITEMS[1],
        "Done",
        &[page_link(), test_link("a_second_server_is_refused")],
    );
    assert_refused(&page_with(1, item), "names no CI job");
}

#[test]
fn a_link_to_another_site_is_refused_as_evidence() {
    let line = "- [the conserver](https://github.com/vcon-dev/vcon-server)".to_string();
    let item = section(ITEMS[0], "Partly done", &[line]);
    assert_refused(&page_with(0, item), "is not evidence");
}

#[test]
fn an_evidence_line_with_two_links_is_refused() {
    let line = "- [a](@/docs/status.md) and [b](@/docs/status.md)".to_string();
    let item = section(ITEMS[0], "Partly done", &[line]);
    assert_refused(&page_with(0, item), "one link");
}

#[test]
fn an_evidence_line_with_no_link_is_refused() {
    let item = section(ITEMS[0], "Partly done", &["- the CI runs it".to_string()]);
    assert_refused(&page_with(0, item), "one link");
}

#[test]
fn a_year_on_the_page_is_refused() {
    let item = section(ITEMS[4], "Not started", &[page_link()])
        .replace("What is done", "Due in 2027. What is done");
    assert_refused(&page_with(4, item), "2027");
}

#[test]
fn a_page_without_the_checklist_link_is_not_linked() {
    assert!(
        !links_the_checklist("See [what is built](@/docs/status.md).", true),
        "a page linking another page"
    );
    assert!(
        !links_the_checklist("the production-checklist page", true),
        "the name without a link"
    );
    assert!(
        !links_the_checklist("[c](@/docs/production-checklist.md)", false),
        "the README needs the site's URL, which GitHub can open"
    );
}

// Edge.

#[test]
fn an_empty_page_is_refused_for_each_item() {
    let found = check("");
    for item in ITEMS {
        assert!(
            found.iter().any(|p| p.contains(item)),
            "{item} is named in {found:#?}"
        );
    }
}

#[test]
fn a_heading_in_a_fenced_block_is_not_an_item() {
    let mut sections = good_sections();
    sections[2].push_str("```\n## Faster builds\n```\n\n");
    assert_holds(&page_of(&sections));
}

#[test]
fn a_third_level_heading_stays_in_its_item() {
    let mut sections = good_sections();
    sections[3] = sections[3].replace("What is done", "### What is left\n\nWhat is done");
    assert_holds(&page_of(&sections));
    assert_eq!(
        items(&page_of(&sections)).len(),
        ITEMS.len(),
        "no extra item"
    );
}

#[test]
fn evidence_in_a_fenced_block_is_not_read() {
    let item = section(ITEMS[0], "Partly done", &[]).replace(
        "Evidence:\n",
        "```\nEvidence:\n- [s](@/docs/status.md)\n```\n",
    );
    assert_refused(&page_with(0, item), "no evidence");
}

#[test]
fn an_evidence_list_with_blank_lines_between_its_items_is_read_whole() {
    let lines = [
        page_link(),
        String::new(),
        "- [kms](@/docs/kms.md)".to_string(),
    ];
    let item = section(ITEMS[0], "Partly done", &lines);
    assert_refused(&page_with(0, item), "website/content/docs/kms.md");
}

#[test]
fn a_list_after_the_evidence_list_is_not_evidence() {
    let mut item = section(ITEMS[0], "Partly done", &[page_link()]);
    item.push_str("Later work:\n\n- [kms](@/docs/kms.md)\n\n");
    assert_holds(&page_with(0, item));
}

#[test]
fn a_state_line_belongs_to_its_own_item() {
    let mut sections = good_sections();
    sections[0] = sections[0].replace("**State:** Partly done.\n", "");
    sections[1] = sections[1].replace("Evidence:", "**State:** Done.\n\nEvidence:");
    let found = check(&page_of(&sections));
    assert!(
        found
            .iter()
            .any(|p| p.contains(ITEMS[0]) && p.contains("no state")),
        "the first item has none: {found:#?}"
    );
}

#[test]
fn a_state_in_another_case_is_refused() {
    let item = section(ITEMS[0], "partly done", &[page_link()]);
    assert_refused(&page_with(0, item), "partly done");
}

#[test]
fn a_key_under_on_or_in_a_step_is_not_a_job() {
    let jobs = job_ids(WORKFLOW);
    for key in ["push", "pull_request", "steps", "name", "runs-on"] {
        assert!(
            !jobs.contains(&key.to_string()),
            "{key} is not a job: {jobs:?}"
        );
    }
    assert!(
        job_ids("on:\n  push:\n").is_empty(),
        "a workflow without jobs has none"
    );
}

#[test]
fn a_comment_in_the_jobs_is_neither_a_job_nor_their_end() {
    let workflow = "jobs:\n  # The build, then:\n  build:\n# a note at the margin:\n  test:\n";
    assert_eq!(
        job_ids(workflow),
        vec!["build".to_string(), "test".to_string()],
        "the comments are skipped"
    );
}

#[test]
fn a_test_name_that_is_only_the_start_of_a_function_is_refused() {
    let item = section(ITEMS[0], "Partly done", &[test_link("a_backup_restores")]);
    assert_refused(&page_with(0, item), "a_backup_restores");
}

#[test]
fn numbers_that_are_not_years_are_allowed() {
    let item = section(ITEMS[0], "Partly done", &[page_link()]).replace(
        "What is done",
        "PostgreSQL 16 and 18, RFC 9457, 1999 rows, 20260 bytes. What is done",
    );
    assert_holds(&page_with(0, item));
}

#[test]
fn a_tree_link_in_place_of_a_blob_link_is_refused() {
    let line = "- [crates](https://github.com/NormB/vcon-server/tree/main/crates)".to_string();
    let item = section(ITEMS[0], "Partly done", &[line]);
    assert_refused(&page_with(0, item), "is not evidence");
}

#[test]
fn an_empty_anchor_is_refused() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &["- [s](@/docs/status.md#)".to_string()],
    );
    assert_refused(&page_with(0, item), "empty anchor");
}

#[test]
fn a_heading_in_a_fenced_block_of_the_linked_page_is_no_anchor() {
    let item = section(
        ITEMS[0],
        "Partly done",
        &["- [s](@/docs/status.md#not-a-heading)".to_string()],
    );
    assert_refused(&page_with(0, item), "not-a-heading");
}

#[test]
fn each_problem_names_its_item() {
    let item = section(ITEMS[5], "Partly done", &[file_link("docs/kms.md")]);
    let found = check(&page_with(5, item));
    assert!(
        found.iter().any(|p| p.starts_with(ITEMS[5])),
        "the problem starts with the item's heading: {found:#?}"
    );
}

#[test]
fn a_blob_link_with_no_path_is_refused() {
    let line = format!("- [main]({BLOB})");
    let item = section(ITEMS[0], "Partly done", &[line]);
    assert_refused(&page_with(0, item), "is not evidence");
}

#[test]
fn a_test_link_to_a_file_the_tree_lacks_is_refused() {
    let line = format!("- test `t` in [x.rs]({BLOB}crates/x.rs)");
    let item = section(ITEMS[0], "Partly done", &[line]);
    assert_refused(&page_with(0, item), "crates/x.rs");
}

#[test]
fn the_shared_heading_reader_skips_fenced_blocks() {
    let ids = heading_ids(STATUS_TEXT);
    assert!(ids.contains("built-and-tested"), "a heading: {ids:?}");
    assert!(
        ids.contains("the-postgres-feature"),
        "a heading with code: {ids:?}"
    );
    assert!(!ids.contains("not-a-heading"), "a fenced line: {ids:?}");
}
