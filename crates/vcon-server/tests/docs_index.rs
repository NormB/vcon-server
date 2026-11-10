//! `docs/README.md` is the documentation index for people browsing the
//! repository. It links every page of the site's documentation,
//! `website/content/docs/*.md`, so it cannot fall behind when a page is
//! added, and every relative link it holds must reach a file in the tree.
//! The OpenSSF Best Practices site also reads it: it looks for a top-level
//! `doc`, `docs` or `documentation` folder holding a file of at most
//! 100,000 bytes that mentions installing, using and security.
//!
//! The same directory is the source of three more lists, each held to it
//! here so none falls behind when a page is added, renamed or described
//! anew:
//!
//! - the index's line for a page is the page's own title and description;
//! - `website/static/llms.txt` links every page once, and only pages;
//! - every page names its group in the sidebar (`[extra] group`), no two
//!   pages share a `weight`, and the pages of a group are next to each
//!   other, in the order the sidebar shows the groups;
//! - the how-to index links every page of the first two groups, and each of
//!   those pages has the parts a how-to needs: what to have first, and what
//!   to do when a step fails.

mod markdown;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use markdown::{is_relative, link_targets};

/// The repository root, two levels above this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The Best Practices site skips a larger file.
const BEST_PRACTICES_MAX_SIZE: usize = 100_000;

fn index() -> String {
    let path = root().join("docs/README.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn the_link_reader_finds_relative_links_only() {
    let text = "[a](../x.md) [b](https://e.example/) [c](#here) [d](y.md#part)";
    let relative: Vec<String> = link_targets(text)
        .into_iter()
        .filter(|t| is_relative(t))
        .collect();
    assert_eq!(
        relative,
        ["../x.md", "y.md#part"],
        "only paths in the tree are relative links"
    );
}

#[test]
fn the_index_links_every_documentation_page() {
    let dir = root().join("website/content/docs");
    let pages: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("list {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".md"))
        .collect();
    assert!(!pages.is_empty(), "no pages in {}", dir.display());
    let linked: BTreeSet<String> = link_targets(&index())
        .into_iter()
        .filter_map(|t| {
            t.strip_prefix("../website/content/docs/")
                .map(str::to_string)
        })
        .map(|t| t.split('#').next().unwrap_or_default().to_string())
        .collect();
    let missing: Vec<&String> = pages.difference(&linked).collect();
    assert!(
        missing.is_empty(),
        "docs/README.md does not link {missing:?}"
    );
}

#[test]
fn every_relative_link_in_the_index_exists() {
    let base = root().join("docs");
    let broken: Vec<String> = link_targets(&index())
        .into_iter()
        .filter(|t| is_relative(t))
        .filter(|t| !base.join(t.split('#').next().unwrap_or_default()).exists())
        .collect();
    assert!(
        broken.is_empty(),
        "docs/README.md links what does not exist: {broken:?}"
    );
}

#[test]
fn the_index_is_a_file_the_best_practices_site_reads() {
    let text = index();
    assert!(
        text.len() <= BEST_PRACTICES_MAX_SIZE,
        "docs/README.md has {} bytes; the site reads at most {BEST_PRACTICES_MAX_SIZE}",
        text.len()
    );
    let lower = text.to_lowercase();
    // The site's patterns: install(ation)?, us(e|ing), secur(e|ity).
    for words in [&["install"][..], &["use", "using"], &["secure", "security"]] {
        assert!(
            words.iter().any(|w| lower.contains(w)),
            "docs/README.md never says any of {words:?}"
        );
    }
}

/// The sidebar's groups, in the order it shows them. `website/config.toml`
/// holds the same names with their headings, for the templates, and
/// `the_site_shows_the_groups_this_file_names_in_this_order` holds the two
/// together.
const GROUPS: [&str; 5] = ["start", "how-to", "concepts", "reference", "project"];

/// The groups whose pages are step-by-step guides.
const GUIDE_GROUPS: [&str; 2] = ["start", "how-to"];

/// The page that lists the guides; it is a list, not a guide.
const GUIDE_INDEX: &str = "how-to.md";

/// One documentation page: its file name and what its front matter says.
#[derive(Debug, Clone, PartialEq)]
struct Page {
    file: String,
    title: String,
    description: String,
    weight: i64,
    group: String,
    text: String,
}

/// The front matter of a page: the TOML between its two `+++` lines.
fn front_matter(text: &str) -> Option<toml::Table> {
    let rest = text.strip_prefix("+++\n")?;
    let end = rest.find("\n+++")?;
    rest[..end].parse().ok()
}

/// A page read from its text. A member the front matter lacks is empty, or
/// `i64::MIN` for the weight, so the tests name the page that lacks it.
fn page_of(file: &str, text: &str) -> Page {
    let table = front_matter(text).unwrap_or_default();
    let word = |key: &str| {
        table
            .get(key)
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Page {
        file: file.to_string(),
        title: word("title"),
        description: word("description"),
        weight: table
            .get("weight")
            .and_then(toml::Value::as_integer)
            .unwrap_or(i64::MIN),
        group: table
            .get("extra")
            .and_then(|extra| extra.get("group"))
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        text: text.to_string(),
    }
}

/// Every documentation page but the section's own `_index.md`, in the
/// order the sidebar lists them: by weight.
fn pages() -> Vec<Page> {
    let dir = root().join("website/content/docs");
    let mut pages: Vec<Page> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("list {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".md") && name != "_index.md")
        .map(|name| {
            let path = dir.join(&name);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            page_of(&name, &text)
        })
        .collect();
    pages.sort_by(|a, b| (a.weight, &a.file).cmp(&(b.weight, &b.file)));
    pages
}

/// What is wrong with the pages' places in the sidebar: a page with no
/// group or one the sidebar does not show, two pages with one weight, and
/// a group whose pages are not together or come before an earlier group's.
/// `pages` is in weight order.
fn sidebar_faults(pages: &[Page]) -> Vec<String> {
    let mut faults = Vec::new();
    let place = |page: &Page| GROUPS.iter().position(|g| *g == page.group);
    for page in pages {
        if place(page).is_none() {
            faults.push(format!(
                "{}: group {:?} is not one of {GROUPS:?}",
                page.file, page.group
            ));
        }
    }
    for pair in pages.windows(2) {
        if pair[0].weight == pair[1].weight {
            faults.push(format!(
                "{} and {} share weight {}",
                pair[0].file, pair[1].file, pair[0].weight
            ));
        }
        if let (Some(before), Some(after)) = (place(&pair[0]), place(&pair[1]))
            && after < before
        {
            faults.push(format!(
                "{} ({}) sorts after {} ({})",
                pair[1].file, pair[1].group, pair[0].file, pair[0].group
            ));
        }
    }
    faults
}

/// The `##` and `###` headings of a page, without their marks.
fn headings(text: &str) -> Vec<&str> {
    let mut fenced = false;
    let mut found = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced
            && let Some(heading) = line
                .strip_prefix("## ")
                .or_else(|| line.strip_prefix("### "))
        {
            found.push(heading.trim());
        }
    }
    found
}

/// Whether a heading opens the part of a guide that says what to have
/// before its first step.
fn is_prerequisites(heading: &str) -> bool {
    heading.starts_with("Before you ") || heading == "What the host needs"
}

/// Whether a heading opens the part of a guide that says what to do when a
/// step fails.
fn is_troubleshooting(heading: &str) -> bool {
    heading.starts_with("When ") || heading == "Troubleshooting"
}

/// The parts a guide lacks, of the two every guide has.
fn missing_guide_parts(text: &str) -> Vec<&'static str> {
    let headings = headings(text);
    let mut missing = Vec::new();
    if !headings.iter().any(|h| is_prerequisites(h)) {
        missing.push("what to have first (a heading that starts \"Before you\")");
    }
    if !headings.iter().any(|h| is_troubleshooting(h)) {
        missing.push("what to do when a step fails (a heading that starts \"When\")");
    }
    missing
}

/// The index lines of `text`: `- [title](../website/content/docs/file): description`.
fn index_lines(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("- [")?;
            let (title, rest) = rest.split_once("](../website/content/docs/")?;
            let (file, description) = rest.split_once("): ")?;
            Some((title.to_string(), file.to_string(), description.to_string()))
        })
        .collect()
}

/// The pages `text` links on the site, by file name, one for each link:
/// `https://vcon-server.com/docs/<name>/` is `<name>.md`.
fn site_doc_links(text: &str) -> Vec<String> {
    link_targets(text)
        .into_iter()
        .filter_map(|t| {
            let name = t.strip_prefix("https://vcon-server.com/docs/")?;
            let name = name
                .split('#')
                .next()
                .unwrap_or_default()
                .trim_end_matches('/');
            (!name.is_empty()).then(|| format!("{name}.md"))
        })
        .collect()
}

#[test]
fn the_front_matter_reader_reads_title_weight_and_group() {
    let page = page_of(
        "a.md",
        "+++\ntitle = \"A\"\ndescription = \"About a.\"\nweight = 7\n[extra]\ngroup = \"start\"\n+++\n\nBody.\n",
    );
    assert_eq!(
        (
            page.title.as_str(),
            page.description.as_str(),
            page.weight,
            page.group.as_str()
        ),
        ("A", "About a.", 7, "start"),
        "each member is read from the front matter"
    );
    let bare = page_of("b.md", "No front matter.\n");
    assert_eq!(
        (bare.weight, bare.group.as_str()),
        (i64::MIN, ""),
        "a page without front matter has no weight and no group"
    );
}

#[test]
fn the_sidebar_reader_finds_each_kind_of_fault() {
    let page = |file: &str, weight: i64, group: &str| Page {
        file: file.to_string(),
        title: String::new(),
        description: String::new(),
        weight,
        group: group.to_string(),
        text: String::new(),
    };
    let good = [
        page("a.md", 1, "start"),
        page("b.md", 2, "how-to"),
        page("c.md", 3, "project"),
    ];
    assert_eq!(
        sidebar_faults(&good),
        Vec::<String>::new(),
        "groups in order, weights apart"
    );
    let none = [page("a.md", 1, "")];
    assert_eq!(
        sidebar_faults(&none),
        [format!("a.md: group \"\" is not one of {GROUPS:?}")],
        "a page with no group is named"
    );
    let unknown = [page("a.md", 1, "tutorial")];
    assert_eq!(
        sidebar_faults(&unknown).len(),
        1,
        "a group the sidebar does not show is a fault"
    );
    let shared = [page("a.md", 4, "start"), page("b.md", 4, "start")];
    assert_eq!(
        sidebar_faults(&shared),
        ["a.md and b.md share weight 4"],
        "two pages with one weight have no fixed order"
    );
    let split = [
        page("a.md", 1, "how-to"),
        page("b.md", 2, "start"),
        page("c.md", 3, "how-to"),
    ];
    assert_eq!(
        sidebar_faults(&split),
        ["b.md (start) sorts after a.md (how-to)"],
        "a page of an earlier group after a later group's page is named"
    );
}

#[test]
fn every_page_has_its_place_in_the_sidebar() {
    let pages = pages();
    assert!(
        pages.len() > 20,
        "the walk reached the pages: {}",
        pages.len()
    );
    let faults = sidebar_faults(&pages);
    assert!(
        faults.is_empty(),
        "the sidebar's order is not fixed: {faults:#?}"
    );
    for group in GROUPS {
        assert!(
            pages.iter().any(|p| p.group == group),
            "no page is in the group {group}"
        );
    }
}

#[test]
fn the_first_pages_are_the_first_visit_path() {
    let pages = pages();
    let first: Vec<&str> = pages.iter().take(3).map(|p| p.file.as_str()).collect();
    assert_eq!(
        first,
        ["install.md", "getting-started.md", "first-vcon.md"],
        "a new reader installs, starts the server and stores a first vCon, in that order"
    );
    // The first group holds that path and nothing else: the site's test of
    // the sidebar expects those three links there, and a guide for one
    // deployment choice belongs with the how-to guides.
    let start: Vec<&str> = pages
        .iter()
        .filter(|p| p.group == GROUPS[0])
        .map(|p| p.file.as_str())
        .collect();
    assert_eq!(
        start, first,
        "the group {:?} holds the first-visit path only",
        GROUPS[0]
    );
}

#[test]
fn the_site_shows_the_groups_this_file_names_in_this_order() {
    let path = root().join("website/config.toml");
    let config: toml::Table = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .parse()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let shown: Vec<&str> = config["extra"]["docs_groups"]
        .as_array()
        .expect("extra.docs_groups is a list")
        .iter()
        .map(|pair| pair[0].as_str().expect("a group's name"))
        .collect();
    assert_eq!(
        shown, GROUPS,
        "config.toml's docs_groups, which the sidebar and the index show"
    );
    for template in ["components.html", "section.html"] {
        let path = root().join("website/templates").join(template);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            text.contains("p.extra.group == g[0]"),
            "{template} no longer lists each group's pages under its heading"
        );
    }
}

#[test]
fn the_heading_reader_skips_code_blocks() {
    let text = "## One\n\n```sh\n## not a heading\n```\n\n### Two\n\n# Top\n#### Deep\n";
    assert_eq!(
        headings(text),
        ["One", "Two"],
        "second- and third-level headings outside code"
    );
}

#[test]
fn the_guide_reader_names_each_missing_part() {
    assert!(
        missing_guide_parts("## Before you start\n\n## Steps\n\n## When a step fails\n").is_empty(),
        "a guide with both parts lacks none"
    );
    assert!(
        missing_guide_parts("## What the host needs\n\n### Troubleshooting\n").is_empty(),
        "the install page's and the export page's headings count"
    );
    assert_eq!(
        missing_guide_parts("## Before you upgrade\n\n## Steps\n").len(),
        1,
        "a guide with no troubleshooting part lacks one"
    );
    assert_eq!(
        missing_guide_parts("## Steps\n\n## When it does not start\n").len(),
        1,
        "a guide with no prerequisites part lacks one"
    );
    assert_eq!(
        missing_guide_parts("```sh\n## Before you start\n## When it fails\n```\n").len(),
        2,
        "headings inside a code block are not parts"
    );
}

#[test]
fn every_guide_says_what_to_have_first_and_what_to_do_when_a_step_fails() {
    let mut lacking = Vec::new();
    let mut guides = 0;
    for page in pages() {
        if !GUIDE_GROUPS.contains(&page.group.as_str()) || page.file == GUIDE_INDEX {
            continue;
        }
        guides += 1;
        for part in missing_guide_parts(&page.text) {
            lacking.push(format!("{}: {part}", page.file));
        }
    }
    assert!(guides >= 10, "the walk reached the guides: {guides}");
    assert!(lacking.is_empty(), "guides lack a part: {lacking:#?}");
}

#[test]
fn the_how_to_index_links_every_guide() {
    let path = root().join("website/content/docs").join(GUIDE_INDEX);
    let index =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let linked: BTreeSet<String> = link_targets(&index)
        .into_iter()
        .filter_map(|t| t.strip_prefix("@/docs/").map(str::to_string))
        .map(|t| t.split('#').next().unwrap_or_default().to_string())
        .collect();
    let missing: Vec<String> = pages()
        .into_iter()
        .filter(|p| GUIDE_GROUPS.contains(&p.group.as_str()) && p.file != GUIDE_INDEX)
        .map(|p| p.file)
        .filter(|file| !linked.contains(file))
        .collect();
    assert!(
        missing.is_empty(),
        "how-to.md does not link the guides {missing:?}"
    );
}

#[test]
fn the_index_line_reader_takes_title_file_and_description() {
    let text = "Intro.\n- [A page](../website/content/docs/a.md): About a: all of it.\n- [Policy](../SECURITY.md): elsewhere\n";
    assert_eq!(
        index_lines(text),
        [(
            "A page".to_string(),
            "a.md".to_string(),
            "About a: all of it.".to_string()
        )],
        "only lines for documentation pages, with the whole description"
    );
}

#[test]
fn the_index_describes_each_page_as_the_page_does() {
    let pages = pages();
    let mut wrong = Vec::new();
    let lines = index_lines(&index());
    assert!(
        lines.len() > 20,
        "the index's lines were read: {}",
        lines.len()
    );
    for (title, file, description) in lines {
        let Some(page) = pages.iter().find(|p| p.file == file) else {
            continue;
        };
        if title != page.title {
            wrong.push(format!(
                "{file}: titled {title:?}, the page says {:?}",
                page.title
            ));
        }
        if description != page.description {
            wrong.push(format!(
                "{file}: described as {description:?}, the page says {:?}",
                page.description
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "docs/README.md has fallen behind: {wrong:#?}"
    );
}

#[test]
fn the_site_link_reader_names_each_linked_page() {
    let text = "[a](https://vcon-server.com/docs/a/) [b](https://vcon-server.com/docs/b/#part) \
                [docs](https://vcon-server.com/docs/) [p](https://vcon-server.com/problems/) [a](https://vcon-server.com/docs/a/)";
    assert_eq!(
        site_doc_links(text),
        ["a.md", "b.md", "a.md"],
        "one name for each link to a documentation page, a repeated link twice"
    );
}

#[test]
fn llms_txt_links_every_page_once_and_only_pages() {
    let path = root().join("website/static/llms.txt");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let linked = site_doc_links(&text);
    let pages: BTreeSet<String> = pages().into_iter().map(|p| p.file).collect();
    let once: BTreeSet<String> = linked.iter().cloned().collect();
    let missing: Vec<&String> = pages.difference(&once).collect();
    assert!(missing.is_empty(), "llms.txt does not link {missing:?}");
    let unknown: Vec<&String> = once.difference(&pages).collect();
    assert!(
        unknown.is_empty(),
        "llms.txt links pages the site does not have: {unknown:?}"
    );
    let repeated: Vec<&String> = once
        .iter()
        .filter(|name| linked.iter().filter(|l| l == name).count() > 1)
        .collect();
    assert!(
        repeated.is_empty(),
        "llms.txt links more than once: {repeated:?}"
    );
}

// Three pages each held more than one kind of writing: explanation, steps
// to follow, and tables to look things up in. Each now keeps one kind, and
// the rest is on a page of its own kind. A link published before the split
// still lands on a heading of the page it names.

/// The headings each split page had before the split, as the ids in its
/// URLs (`/docs/<page>/#<id>`), from the site as release 0.4.2 published
/// it. Other sites and this repository's changelog link them, so each stays
/// a heading of its page.
const PUBLISHED_ANCHORS: [(&str, &[&str]); 3] = [
    (
        "audit-logs.md",
        &[
            "two-kinds-of-log",
            "a-record",
            "every-action",
            "reveals",
            "see-who-revealed-what",
            "counted-records",
            "reading-the-logs",
            "the-request-log-is-not-the-audit-log",
        ],
    ),
    (
        "operations.md",
        &[
            "the-data-directory",
            "objects-in-an-s3-bucket",
            "one-server-per-directory",
            "the-purge-sweep",
            "logs",
            "health-checks",
            "stopping",
            "checking-the-data",
            "key-rotation",
            "back-up-and-restore",
            "back-up-the-master-key",
            "back-up-the-data-directory",
            "or-copy-it-with-the-server-stopped",
            "or-take-a-filesystem-snapshot",
            "restore-the-data-directory",
            "what-a-key-rotation-does-to-backups",
        ],
    ),
    (
        "mcp.md",
        &[
            "connect-over-https",
            "connect-over-stdio",
            "client-configuration",
            "tool-calls",
            "what-every-tool-shares",
            "tools",
            "vcon-tools",
            "vcon-store",
            "vcon-get",
            "vcon-list",
            "vcon-replace",
            "vcon-delete",
            "legal-tools",
            "legal-versions",
            "legal-get",
            "legal-hold",
            "legal-release",
            "legal-restore",
            "audit-tool",
            "audit-read",
            "admin-tools",
            "admin-tenant-create",
            "admin-tenant-list",
            "admin-tenant-retention",
            "admin-key-create",
            "admin-key-list",
            "admin-key-revoke",
            "admin-audit-read",
            "admin-tenant-remove",
        ],
    ),
];

/// The id Zola gives a heading: lower case, with each run of other
/// characters as one hyphen.
fn slug(heading: &str) -> String {
    let mut slug = String::new();
    for c in heading.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

/// The text of one documentation page.
fn page_text(file: &str) -> String {
    let path = root().join("website/content/docs").join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The lines of `text` that open a fenced block of the language `lang`.
fn blocks_of(text: &str, lang: &str) -> usize {
    let open = format!("```{lang}");
    text.lines().filter(|line| line.trim() == open).count()
}

#[test]
fn the_slug_reader_writes_ids_as_zola_does() {
    assert_eq!(
        slug(" What a key rotation does to backups"),
        "what-a-key-rotation-does-to-backups",
        "words joined by hyphens"
    );
    assert_eq!(
        slug(" `vcon_store`"),
        "vcon-store",
        "marks and underscores are not kept"
    );
    assert_eq!(
        slug(" From 0.4.0 to 0.4.1: the secret-file rule"),
        "from-0-4-0-to-0-4-1-the-secret-file-rule",
        "each run of marks is one hyphen"
    );
}

#[test]
fn every_anchor_published_before_the_split_is_still_a_heading_of_its_page() {
    let mut gone = Vec::new();
    for (file, anchors) in PUBLISHED_ANCHORS {
        let ids: BTreeSet<String> = headings(&page_text(file)).into_iter().map(slug).collect();
        for anchor in anchors {
            if !ids.contains(*anchor) {
                gone.push(format!("/docs/{}/#{anchor}", file.trim_end_matches(".md")));
            }
        }
    }
    assert!(
        gone.is_empty(),
        "links published before the split lead nowhere: {gone:#?}"
    );
}

#[test]
fn the_pages_the_split_made_are_in_the_groups_of_their_kind() {
    let pages = pages();
    let group = |file: &str| {
        pages
            .iter()
            .find(|p| p.file == file)
            .map(|p| p.group.clone())
            .unwrap_or_else(|| format!("no page {file}"))
    };
    for (file, kind) in [
        ("audit-logs.md", "concepts"),
        ("audit-log-records.md", "reference"),
        ("read-audit-logs.md", "how-to"),
        ("operations.md", "concepts"),
        ("mcp.md", "reference"),
        ("connect-mcp.md", "how-to"),
    ] {
        assert_eq!(group(file), kind, "{file} is in the group of its kind");
    }
}

/// The explanation pages hold no steps to run and no table to look a value
/// up in; the reference page holds no steps; the guides hold the steps.
#[test]
fn each_split_page_keeps_to_one_kind_of_writing() {
    let audit = page_text("audit-logs.md");
    assert_eq!(
        blocks_of(&audit, "sh"),
        0,
        "audit-logs.md explains; the commands are on read-audit-logs.md"
    );
    assert!(
        !audit.contains("| `create` |") && !audit.contains("| `seq` |"),
        "audit-logs.md explains; the action and field tables are on audit-log-records.md"
    );
    let operations = page_text("operations.md");
    assert_eq!(
        blocks_of(&operations, "sh"),
        0,
        "operations.md explains; the steps are on the rotation and backup guides"
    );
    assert!(
        !operations.contains("`purges_awaiting_rotation` and"),
        "operations.md explains; check's members are on command-line.md"
    );
    let mcp = page_text("mcp.md");
    assert!(
        !mcp.contains("\"mcpServers\""),
        "mcp.md is the reference; the client configuration is a step of connect-mcp.md"
    );
    let records = page_text("audit-log-records.md");
    assert_eq!(
        blocks_of(&records, "sh"),
        0,
        "audit-log-records.md is tables, not steps"
    );
    assert!(
        records.contains("| `create` |") && records.contains("| `seq` |"),
        "audit-log-records.md holds the action and the field tables"
    );
    let guide = page_text("read-audit-logs.md");
    assert!(
        blocks_of(&guide, "sh") >= 3,
        "read-audit-logs.md holds the commands"
    );
    assert!(
        page_text("connect-mcp.md").contains("\"mcpServers\""),
        "connect-mcp.md holds the client configuration"
    );
    assert!(
        page_text("command-line.md").contains("`purges_awaiting_rotation`"),
        "command-line.md says what each member of check's report means"
    );
}
