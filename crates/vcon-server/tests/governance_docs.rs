//! The governance documents: GOVERNANCE.md, MAINTAINERS.md,
//! .github/CODEOWNERS, the starter-task issue form and the pages that link
//! them.
//!
//! GOVERNANCE.md states facts only. Its section "If the maintainer is
//! unavailable" names no successor and makes no promise the owner has not
//! made: these tests refuse a sentence that names a successor and the words
//! of a response time or a guarantee. Where the page says releases go, each
//! place is named with the release.yml job that publishes to it, and the
//! command that checks a release is SECURITY.md's, character for character,
//! so the page cannot drift from the workflow or the security policy.
//!
//! Each rule is a function of its text, so the tests drive it with planted
//! text as well as with the files in the tree.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

#[path = "governance_docs/agreement.rs"]
mod agreement;

/// The repository root, two levels above this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A file of the repository, by its path from the root.
fn read(path: &str) -> String {
    let full = root().join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {}: {e}", full.display()))
}

/// The second-level headings GOVERNANCE.md has, each exactly once.
const HEADINGS: [&str; 6] = [
    "Roles",
    "Becoming a committer",
    "How the project decides",
    "If the maintainer is unavailable",
    "Reporting a vulnerability",
    "Changing this document",
];

/// The rows of GOVERNANCE.md's Roles table, by their first cell.
const ROLES: [&str; 3] = ["Maintainer", "Committer", "Contributor"];

/// The release.yml jobs that put a release somewhere: the GitHub release,
/// the two crates on crates.io, and the site.
const RELEASE_JOBS: [&str; 3] = ["publish", "crates-io", "site"];

/// The issue form for a starter task, and the fields it requires.
const FORM: &str = ".github/ISSUE_TEMPLATE/starter_task.yml";
const FORM_FIELDS: [&str; 4] = ["what", "where", "test-first", "done-when"];

/// The pages that link GOVERNANCE.md.
const LINKING_PAGES: [&str; 4] = [
    "README.md",
    "CONTRIBUTING.md",
    "website/content/docs/development.md",
    "MAINTAINERS.md",
];

/// Words of a commitment: a response time, a guarantee, a support period.
const PROMISES: [&str; 10] = [
    "within ",
    " hours",
    "business day",
    "will respond",
    "will reply",
    "will answer",
    "we will",
    "guarantee",
    "promise",
    "will be maintained",
];

/// The labels of a how-to, in the form the development page uses.
const HOW_TO_LABELS: [&str; 5] = [
    "**Purpose:**",
    "**Prerequisites:**",
    "**Steps:**",
    "**Verify:**",
    "**Troubleshoot:**",
];

/// The lines of `text` outside fenced code blocks.
fn prose_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    for line in text.lines() {
        let start = line.trim_start();
        match fence {
            Some(mark) => {
                if start.starts_with(mark) {
                    fence = None;
                }
            }
            None if start.starts_with("```") => fence = Some("```"),
            None if start.starts_with("~~~") => fence = Some("~~~"),
            None => out.push(line),
        }
    }
    out
}

/// The fenced code blocks of `text`, each without its fence lines and
/// without the indentation of its opening fence (a block inside a list
/// item is indented with the item).
fn fenced_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut block: Option<(&str, usize, Vec<&str>)> = None;
    for line in text.lines() {
        let start = line.trim_start();
        let indent = line.len() - start.len();
        match block.as_mut() {
            Some((mark, depth, body)) => {
                if start.starts_with(*mark) {
                    out.push(body.join("\n"));
                    block = None;
                } else {
                    body.push(&line[indent.min(*depth)..]);
                }
            }
            None if start.starts_with("```") => block = Some(("```", indent, Vec::new())),
            None if start.starts_with("~~~") => block = Some(("~~~", indent, Vec::new())),
            None => {}
        }
    }
    out
}

/// Each heading of `text` outside code blocks: its level and its title,
/// trimmed.
fn headings(text: &str) -> Vec<(usize, String)> {
    prose_lines(text)
        .into_iter()
        .filter_map(|line| {
            let level = line.bytes().take_while(|b| *b == b'#').count();
            let rest = &line[level..];
            if level == 0 || level > 6 || !rest.starts_with(' ') {
                return None;
            }
            Some((level, rest.trim().to_string()))
        })
        .collect()
}

/// What is wrong with the second-level headings of `text` against
/// `required`: a heading missing or there more than once.
fn heading_findings(text: &str, required: &[&str]) -> Vec<String> {
    let found: Vec<String> = headings(text)
        .into_iter()
        .filter(|(level, _)| *level == 2)
        .map(|(_, title)| title)
        .collect();
    required
        .iter()
        .filter_map(
            |want| match found.iter().filter(|title| title == want).count() {
                1 => None,
                0 => Some(format!("no heading \"## {want}\"")),
                n => Some(format!("\"## {want}\" is there {n} times")),
            },
        )
        .collect()
}

/// The lines under `## heading`, up to the next first- or second-level
/// heading.
fn section(text: &str, heading: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            inside = title.trim() == heading;
        } else if line.starts_with("# ") {
            inside = false;
        } else if inside {
            out.push(line);
        }
    }
    out.join("\n")
}

/// The cells of a table row: split at each `|` outside a code span; `\|`
/// is a `|` inside a cell.
fn cells(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut code = false;
    let mut chars = row.trim().chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '`' => {
                code = !code;
                cell.push(c);
            }
            '|' if !code => {
                out.push(cell.trim().to_string());
                cell.clear();
            }
            _ => cell.push(c),
        }
    }
    out.push(cell.trim().to_string());
    if out.first().is_some_and(String::is_empty) {
        out.remove(0);
    }
    if out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// Whether `line` is a table's separator row, such as `|---|:--:|`.
fn is_separator(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('|')
        && line.contains('-')
        && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// The body rows of the first table in `text`.
fn table_rows(text: &str) -> Vec<Vec<String>> {
    let lines = prose_lines(text);
    let Some(start) = lines
        .windows(2)
        .position(|pair| pair[0].trim_start().starts_with('|') && is_separator(pair[1]))
    else {
        return Vec::new();
    };
    lines[start + 2..]
        .iter()
        .take_while(|line| line.trim_start().starts_with('|'))
        .map(|line| cells(line))
        .collect()
}

/// A cell's text without code marks, and a link's text without its target.
fn plain(cell: &str) -> String {
    let text = cell.replace('`', "");
    if let Some(rest) = text.strip_prefix('[')
        && let Some(end) = rest.find("](")
    {
        return rest[..end].to_string();
    }
    text
}

/// What is wrong with a table whose first cells must be `roles`, each
/// once: a role missing, one there twice, a row that is no role.
fn role_findings(rows: &[Vec<String>], roles: &[&str]) -> Vec<String> {
    let names: Vec<String> = rows
        .iter()
        .filter_map(|row| row.first())
        .map(|cell| plain(cell))
        .collect();
    let mut out = Vec::new();
    for role in roles {
        match names.iter().filter(|name| name == role).count() {
            1 => {}
            0 => out.push(format!("no row for {role}")),
            n => out.push(format!("{n} rows for {role}")),
        }
    }
    for name in &names {
        if !roles.contains(&name.as_str()) {
            out.push(format!("a row for {name}, which is no role"));
        }
    }
    out
}

/// `line` without its code spans.
fn without_code_spans(line: &str) -> String {
    let mut out = String::new();
    let mut code = false;
    for c in line.chars() {
        if c == '`' {
            code = !code;
        } else if !code {
            out.push(c);
        }
    }
    out
}

/// The target of each inline link in `text`, outside code blocks and code
/// spans, in order.
fn links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in prose_lines(text) {
        let line = without_code_spans(line);
        let mut rest = line.as_str();
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            let end = rest.find(')').unwrap_or(rest.len());
            let target = rest[..end].split_whitespace().next().unwrap_or("");
            let target = target.trim_start_matches('<').trim_end_matches('>');
            if !target.is_empty() {
                out.push(target.to_string());
            }
            rest = &rest[end..];
        }
    }
    out
}

/// GitHub's anchor for a heading.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            _ if c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .collect()
}

/// Whether a link target is a web, mail or Zola `@/` link.
fn is_external(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:") || target.starts_with("@/")
}

/// `path` with its `.` and `..` parts applied; None when it is absolute or
/// leaves the directory it starts in.
fn normalized(path: &Path) -> Option<PathBuf> {
    let mut parts: Vec<&OsStr> = Vec::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(name) => parts.push(name),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.iter().collect())
}

/// Each relative link of `text`, the page at `page` (a path from `root`),
/// that names no file or directory under `root`, or no heading of its
/// Markdown target. Web, mail and Zola `@/` links are not read.
fn link_findings(root: &Path, page: &str, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let dir = Path::new(page).parent().unwrap_or(Path::new(""));
    for target in links(text) {
        if is_external(&target) {
            continue;
        }
        let (path, fragment) = match target.split_once('#') {
            Some((path, fragment)) => (path, Some(fragment)),
            None => (target.as_str(), None),
        };
        let body = if path.is_empty() {
            text.to_string()
        } else {
            let Some(relative) = normalized(&dir.join(path)) else {
                out.push(format!("{page}: {target} leaves the repository"));
                continue;
            };
            let full = root.join(&relative);
            if !full.exists() {
                out.push(format!("{page}: {target}: no file {}", relative.display()));
                continue;
            }
            if fragment.is_none() {
                continue;
            }
            if full.extension() != Some(OsStr::new("md")) {
                out.push(format!(
                    "{page}: {target}: an anchor into a file that is not Markdown"
                ));
                continue;
            }
            std::fs::read_to_string(&full).unwrap_or_default()
        };
        if let Some(fragment) = fragment
            && !headings(&body)
                .iter()
                .any(|(_, title)| slug(title) == fragment)
        {
            out.push(format!(
                "{page}: {target}: no heading with the anchor #{fragment}"
            ));
        }
    }
    out
}

/// Whether `text` links the file `file`, by a relative path or a URL.
fn links_to(text: &str, file: &str) -> bool {
    let suffix = format!("/{file}");
    links(text).iter().any(|target| {
        let path = target.split(['#', '?']).next().unwrap_or("");
        path == file || path.ends_with(&suffix)
    })
}

/// Whether `word` has the form of an email address.
fn is_email(word: &str) -> bool {
    let Some((local, domain)) = word.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
}

/// The email addresses in `text`.
fn emails(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || "<>()[],;\"'`".contains(c))
        .map(|word| word.trim_end_matches(['.', ':']))
        .filter(|word| is_email(word))
        .map(str::to_string)
        .collect()
}

/// The sentences of `text`'s prose.
fn sentences(text: &str) -> Vec<String> {
    prose_lines(text)
        .join(" ")
        .split(". ")
        .map(|sentence| sentence.trim().to_string())
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

/// Each sentence of `text` that speaks of a successor without a "no" or
/// "not": a sentence that names one.
fn successor_findings(text: &str) -> Vec<String> {
    sentences(text)
        .into_iter()
        .filter(|sentence| {
            let lower = sentence.to_lowercase();
            lower.contains("successor")
                && !lower
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| word == "no" || word == "not")
        })
        .collect()
}

/// The words of a commitment `text` holds, from `PROMISES`.
fn promise_findings(text: &str) -> Vec<&'static str> {
    let lower = prose_lines(text).join(" ").to_lowercase();
    PROMISES
        .into_iter()
        .filter(|words| lower.contains(words))
        .collect()
}

/// The names of the jobs of a workflow: the keys right under `jobs:`.
fn workflow_jobs(workflow: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in workflow.lines() {
        if line == "jobs:" {
            inside = true;
            continue;
        }
        if !line.is_empty() && !line.starts_with(' ') && !line.starts_with('#') {
            inside = false;
        }
        let Some(key) = line.strip_prefix("  ") else {
            continue;
        };
        if !inside || key.starts_with(' ') || key.starts_with('#') {
            continue;
        }
        if let Some(name) = key.strip_suffix(':')
            && !name.contains(' ')
        {
            out.push(name.to_string());
        }
    }
    out
}

/// The release jobs GOVERNANCE.md's table of release places names: the
/// second column of the first table of "If the maintainer is unavailable".
fn destination_jobs(governance: &str) -> Vec<String> {
    table_rows(&section(governance, "If the maintainer is unavailable"))
        .iter()
        .filter_map(|row| row.get(1))
        .map(|cell| plain(cell))
        .collect()
}

/// What is wrong with the named release jobs: one that is not a job of the
/// workflow, or a job of `required` that is not named.
fn destination_findings(named: &[String], jobs: &[String], required: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = named
        .iter()
        .filter(|job| !jobs.contains(job))
        .map(|job| format!("`{job}` is not a job of release.yml"))
        .collect();
    for job in required {
        if !named.iter().any(|name| name == job) {
            out.push(format!("no row for the release job `{job}`"));
        }
    }
    out
}

/// The fenced code block of `text` that starts `gh attestation verify`.
fn verify_command(text: &str) -> Option<String> {
    fenced_blocks(text)
        .into_iter()
        .find(|block| block.trim_start().starts_with("gh attestation verify"))
}

/// The owners of the pattern `*` in a CODEOWNERS file: the last `*` line
/// wins, as GitHub reads it. None when no line has the pattern.
fn owners_of_everything(codeowners: &str) -> Option<Vec<String>> {
    let mut found = None;
    for line in codeowners.lines() {
        let line = line.split_once('#').map_or(line, |(before, _)| before);
        let mut words = line.split_whitespace();
        if words.next() == Some("*") {
            found = Some(words.map(str::to_string).collect());
        }
    }
    found
}

/// Each field of an issue form's `body` that has an id: the id, and
/// whether the field is required.
fn form_fields(form: &str) -> Vec<(String, bool)> {
    let mut items: Vec<(Option<String>, bool)> = Vec::new();
    let mut inside = false;
    for line in form.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line == "body:" {
            inside = true;
            continue;
        }
        if !line.starts_with(' ') && !line.starts_with('-') {
            inside = false;
        }
        if !inside {
            continue;
        }
        if trimmed.starts_with("- type:") {
            items.push((None, false));
        } else if let Some(item) = items.last_mut() {
            if let Some(value) = trimmed.strip_prefix("id:") {
                item.0 = Some(value.trim().trim_matches(['"', '\'']).to_string());
            } else if trimmed == "required: true" {
                item.1 = true;
            }
        }
    }
    items
        .into_iter()
        .filter_map(|(field, required)| field.map(|field| (field, required)))
        .collect()
}

/// What is wrong with an issue form's fields against `wanted`: a field
/// missing or not required.
fn form_findings(fields: &[(String, bool)], wanted: &[&str]) -> Vec<String> {
    wanted
        .iter()
        .filter_map(
            |want| match fields.iter().find(|(field, _)| field == want) {
                None => Some(format!("no field `{want}`")),
                Some((_, false)) => Some(format!("the field `{want}` is not required")),
                Some((_, true)) => None,
            },
        )
        .collect()
}

/// A tree of planted files under a new temporary directory.
fn planted(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("make a temporary directory");
    for (path, text) in files {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().expect("a planted file has a parent"))
            .expect("make the planted file's directory");
        std::fs::write(&full, text).expect("write a planted file");
    }
    dir
}

/// GOVERNANCE.md's text with one line removed or replaced.
fn governance_with(old: &str, new: &str) -> String {
    let text = read("GOVERNANCE.md");
    assert!(text.contains(old), "GOVERNANCE.md no longer holds {old:?}");
    text.replacen(old, new, 1)
}

/// `text` with every link to `file` turned into plain text.
fn without_links_to(text: &str, file: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut line = line.to_string();
        let mut from = 0;
        while let Some(found) = line[from..].find("](") {
            let open = from + found;
            let end = open + line[open..].find(')').expect("a link target ends with `)`");
            let path = line[open + 2..end].split('#').next().unwrap_or("");
            if path == file || path.ends_with(&format!("/{file}")) {
                let start = line[..open]
                    .rfind('[')
                    .expect("a link text starts with `[`");
                let label = line[start + 1..open].to_string();
                line.replace_range(start..=end, &label);
                from = start + label.len();
            } else {
                from = end;
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

// Success: the files in the tree.

#[test]
fn governance_has_each_heading_once() {
    let findings = heading_findings(&read("GOVERNANCE.md"), &HEADINGS);
    assert!(findings.is_empty(), "GOVERNANCE.md: {findings:?}");
}

#[test]
fn the_roles_table_has_the_three_roles() {
    let rows = table_rows(&section(&read("GOVERNANCE.md"), "Roles"));
    let findings = role_findings(&rows, &ROLES);
    assert!(
        findings.is_empty(),
        "GOVERNANCE.md's Roles table: {findings:?}"
    );
    assert_eq!(
        rows.len(),
        3,
        "the Roles table has one row per role: {rows:?}"
    );
}

#[test]
fn every_relative_link_of_governance_resolves() {
    let findings = link_findings(&root(), "GOVERNANCE.md", &read("GOVERNANCE.md"));
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn every_relative_link_of_maintainers_resolves() {
    let findings = link_findings(&root(), "MAINTAINERS.md", &read("MAINTAINERS.md"));
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn every_relative_link_of_contributing_resolves() {
    let findings = link_findings(&root(), "CONTRIBUTING.md", &read("CONTRIBUTING.md"));
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn every_relative_link_of_the_readme_resolves() {
    let findings = link_findings(&root(), "README.md", &read("README.md"));
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn each_release_place_is_a_job_of_release_yml() {
    let named = destination_jobs(&read("GOVERNANCE.md"));
    let jobs = workflow_jobs(&read(".github/workflows/release.yml"));
    let findings = destination_findings(&named, &jobs, &RELEASE_JOBS);
    assert!(
        findings.is_empty(),
        "GOVERNANCE.md's release places: {findings:?}"
    );
    assert_eq!(
        named.len(),
        RELEASE_JOBS.len(),
        "one row per release place: {named:?}"
    );
}

#[test]
fn the_verify_command_is_the_security_policys() {
    let governance = verify_command(&read("GOVERNANCE.md"));
    let security = verify_command(&read("SECURITY.md"));
    assert!(
        security.is_some(),
        "SECURITY.md has no `gh attestation verify` block"
    );
    assert_eq!(
        governance, security,
        "GOVERNANCE.md must quote SECURITY.md's command unchanged"
    );
}

#[test]
fn governance_links_both_licence_files() {
    let text = section(&read("GOVERNANCE.md"), "If the maintainer is unavailable");
    for file in ["LICENSE-MIT", "LICENSE-APACHE"] {
        assert!(
            links_to(&text, file),
            "the unavailable section does not link {file}"
        );
        assert!(root().join(file).is_file(), "{file} is not in the tree");
    }
}

#[test]
fn codeowners_gives_everything_to_the_maintainer() {
    let owners = owners_of_everything(&read(".github/CODEOWNERS"));
    assert_eq!(
        owners,
        Some(vec!["@NormB".to_string()]),
        ".github/CODEOWNERS must give `*` to the maintainer"
    );
}

#[test]
fn the_code_owners_are_the_listed_maintainers() {
    let rows = table_rows(&section(&read("MAINTAINERS.md"), "Maintainers"));
    let mut listed: Vec<String> = rows
        .iter()
        .filter_map(|row| row.first())
        .map(|cell| plain(cell))
        .collect();
    listed.sort();
    let mut owners = owners_of_everything(&read(".github/CODEOWNERS")).unwrap_or_default();
    owners.sort();
    assert!(!listed.is_empty(), "MAINTAINERS.md lists no maintainer");
    assert_eq!(
        listed, owners,
        "CODEOWNERS and MAINTAINERS.md must name the same people"
    );
}

#[test]
fn the_starter_form_requires_its_four_fields() {
    let fields = form_fields(&read(FORM));
    let findings = form_findings(&fields, &FORM_FIELDS);
    assert!(findings.is_empty(), "{FORM}: {findings:?}");
}

#[test]
fn the_readme_links_governance() {
    assert!(
        links_to(&read("README.md"), "GOVERNANCE.md"),
        "README.md does not link GOVERNANCE.md"
    );
}

#[test]
fn contributing_links_governance() {
    assert!(
        links_to(&read("CONTRIBUTING.md"), "GOVERNANCE.md"),
        "CONTRIBUTING.md does not link GOVERNANCE.md"
    );
}

#[test]
fn the_development_page_links_governance() {
    let page = read(LINKING_PAGES[2]);
    assert!(
        links_to(&section(&page, "Contributing"), "GOVERNANCE.md"),
        "development.md's Contributing does not link GOVERNANCE.md"
    );
}

#[test]
fn maintainers_links_governance() {
    assert!(
        links_to(&read("MAINTAINERS.md"), "GOVERNANCE.md"),
        "MAINTAINERS.md does not link GOVERNANCE.md"
    );
}

#[test]
fn no_governance_document_holds_an_email_address() {
    for file in [
        "GOVERNANCE.md",
        "MAINTAINERS.md",
        ".github/CODEOWNERS",
        FORM,
    ] {
        let found = emails(&read(file));
        assert!(found.is_empty(), "{file} holds an email address: {found:?}");
    }
}

#[test]
fn the_unavailable_section_names_no_successor_and_promises_nothing() {
    let governance = read("GOVERNANCE.md");
    let text = section(&governance, "If the maintainer is unavailable");
    assert!(
        text.contains("no named successor"),
        "the section must say there is no named successor"
    );
    for file in ["GOVERNANCE.md", "MAINTAINERS.md"] {
        let page = read(file);
        let named = successor_findings(&page);
        assert!(named.is_empty(), "{file} names a successor: {named:?}");
        let promised = promise_findings(&page);
        assert!(
            promised.is_empty(),
            "{file} makes a commitment: {promised:?}"
        );
    }
}

#[test]
fn contributing_first_change_is_a_how_to_with_the_form() {
    let text = section(&read("CONTRIBUTING.md"), "Your first change");
    for label in HOW_TO_LABELS {
        assert!(
            text.contains(label),
            "CONTRIBUTING.md's Your first change has no {label}"
        );
    }
    assert!(
        links_to(&text, "starter_task.yml"),
        "Your first change does not link the starter-task form"
    );
}

#[test]
fn governance_links_the_maintainer_list() {
    let text = section(&read("GOVERNANCE.md"), "Roles");
    assert!(
        links_to(&text, "MAINTAINERS.md"),
        "the Roles section does not link MAINTAINERS.md"
    );
}

// Failure: each rule on planted text.

#[test]
fn a_missing_heading_is_found() {
    let text = "## Roles\n\n## Changing this document\n";
    let findings = heading_findings(
        text,
        &["Roles", "Becoming a committer", "Changing this document"],
    );
    assert_eq!(
        findings,
        ["no heading \"## Becoming a committer\""],
        "the one heading missing is named"
    );
}

#[test]
fn a_heading_there_twice_is_found() {
    let text = "## Roles\n\ntext\n\n## Roles\n";
    let findings = heading_findings(text, &["Roles"]);
    assert_eq!(
        findings,
        ["\"## Roles\" is there 2 times"],
        "a repeated heading is named"
    );
}

#[test]
fn a_link_to_a_missing_file_is_found() {
    let tree = planted(&[("GOVERNANCE.md", "x\n")]);
    let findings = link_findings(
        tree.path(),
        "GOVERNANCE.md",
        "See [the list](MAINTAINERS.md).\n",
    );
    assert_eq!(
        findings.len(),
        1,
        "the missing target is found: {findings:?}"
    );
    assert!(
        findings[0].contains("MAINTAINERS.md"),
        "the finding names the link: {findings:?}"
    );
}

#[test]
fn a_link_to_a_missing_heading_is_found() {
    let tree = planted(&[("SECURITY.md", "# Security\n\n## Verifying a release\n")]);
    let findings = link_findings(
        tree.path(),
        "GOVERNANCE.md",
        "[check](SECURITY.md#verifying-releases)\n",
    );
    assert_eq!(
        findings.len(),
        1,
        "the broken anchor is found: {findings:?}"
    );
    assert!(
        findings[0].contains("verifying-releases"),
        "the finding names the anchor: {findings:?}"
    );
}

#[test]
fn a_link_out_of_the_repository_is_found() {
    let tree = planted(&[("GOVERNANCE.md", "x\n"), ("etc/hosts", "x\n")]);
    let findings = link_findings(tree.path(), "GOVERNANCE.md", "[up](../../etc/hosts)\n");
    assert_eq!(
        findings.len(),
        1,
        "a link above the root is refused: {findings:?}"
    );
    assert!(
        findings[0].contains("leaves the repository"),
        "the link is refused for leaving the root, not read as etc/hosts inside it: {findings:?}"
    );
}

#[test]
fn an_email_address_in_maintainers_is_found() {
    let text = "| [@NormB](https://github.com/NormB) | all | maintainer@example.com |\n";
    assert_eq!(
        emails(text),
        ["maintainer@example.com"],
        "the address is found"
    );
    assert_eq!(
        emails("Write to someone@example.com."),
        ["someone@example.com"],
        "the full stop that ends a sentence is not part of the address"
    );
}

#[test]
fn a_row_that_is_no_role_is_found() {
    let rows = table_rows(
        "| Role | Who |\n|---|---|\n| Maintainer | a |\n| Committer | b |\n| Contributor | c |\n| Owner | d |\n",
    );
    assert_eq!(
        role_findings(&rows, &ROLES),
        ["a row for Owner, which is no role"],
        "a fourth row is refused"
    );
}

#[test]
fn a_named_successor_is_found() {
    let text =
        "If the maintainer is unavailable, the successor is @example-user. The license stays.";
    let findings = successor_findings(text);
    assert_eq!(
        findings.len(),
        1,
        "the sentence naming a successor is found: {findings:?}"
    );
}

#[test]
fn a_response_time_is_found() {
    let found = promise_findings("The maintainer will respond within 48 hours.");
    assert_eq!(
        found,
        ["within ", " hours", "will respond"],
        "each word of the promise is found"
    );
}

#[test]
fn a_guarantee_is_found() {
    let found = promise_findings("Releases are guaranteed for two years.");
    assert_eq!(found, ["guarantee"], "a guarantee is a commitment");
}

#[test]
fn a_place_that_is_no_release_job_is_found() {
    let named = vec!["publish".to_string(), "homebrew".to_string()];
    let jobs = vec!["publish".to_string(), "crates-io".to_string()];
    let findings = destination_findings(&named, &jobs, &["publish"]);
    assert_eq!(
        findings.len(),
        1,
        "the job release.yml does not have is found: {findings:?}"
    );
    assert!(
        findings[0].contains("homebrew"),
        "the finding names the job: {findings:?}"
    );
}

#[test]
fn a_release_place_left_out_is_found() {
    let named = vec!["publish".to_string()];
    let jobs = vec![
        "publish".to_string(),
        "crates-io".to_string(),
        "site".to_string(),
    ];
    let findings = destination_findings(&named, &jobs, &RELEASE_JOBS);
    assert_eq!(
        findings.len(),
        2,
        "each place left out is found: {findings:?}"
    );
}

#[test]
fn a_form_field_that_is_not_required_is_found() {
    let fields = vec![("what".to_string(), true), ("where".to_string(), false)];
    let findings = form_findings(&fields, &["what", "where"]);
    assert_eq!(
        findings.len(),
        1,
        "the optional field is found: {findings:?}"
    );
    assert!(
        findings[0].contains("where"),
        "the finding names the field: {findings:?}"
    );
}

#[test]
fn a_form_field_left_out_is_found() {
    let form = "name: Starter task\nbody:\n  - type: textarea\n    id: what\n    validations:\n      required: true\n";
    let findings = form_findings(&form_fields(form), &FORM_FIELDS);
    assert_eq!(
        findings.len(),
        3,
        "the three fields left out are found: {findings:?}"
    );
}

#[test]
fn codeowners_without_everything_is_found() {
    assert_eq!(
        owners_of_everything("/docs/ @NormB\n*.rs @NormB\n"),
        None,
        "no line gives `*` to anyone"
    );
}

#[test]
fn each_page_without_its_governance_link_is_found() {
    for page in LINKING_PAGES {
        let text = without_links_to(&read(page), "GOVERNANCE.md");
        assert!(
            !links_to(&text, "GOVERNANCE.md"),
            "{page} still links GOVERNANCE.md after removal"
        );
    }
}

#[test]
fn governance_without_the_maintainer_link_is_found() {
    let text = without_links_to(&read("GOVERNANCE.md"), "MAINTAINERS.md");
    assert!(
        !links_to(&section(&text, "Roles"), "MAINTAINERS.md"),
        "the Roles section still links MAINTAINERS.md"
    );
}

#[test]
fn a_missing_role_is_found() {
    let rows = table_rows("| Role | Who |\n|---|---|\n| Maintainer | x |\n| Contributor | y |\n");
    let findings = role_findings(&rows, &ROLES);
    assert_eq!(
        findings,
        ["no row for Committer"],
        "the role left out is named"
    );
}

#[test]
fn a_changed_verify_command_differs() {
    let governance = governance_with("--repo NormB/vcon-server", "--repo NormB/vcon-server-fork");
    assert_ne!(
        verify_command(&governance),
        verify_command(&read("SECURITY.md")),
        "a changed repository is a different command"
    );
}

#[test]
fn a_heading_removed_from_governance_is_found() {
    let text = governance_with("## Becoming a committer", "## Joining");
    let findings = heading_findings(&text, &HEADINGS);
    assert_eq!(
        findings,
        ["no heading \"## Becoming a committer\""],
        "the renamed heading is missing"
    );
}

// Edge cases.

#[test]
fn a_heading_with_a_trailing_space_counts() {
    assert!(
        heading_findings("## Roles \n", &["Roles"]).is_empty(),
        "trailing space is trimmed"
    );
}

#[test]
fn a_heading_in_another_case_does_not_count() {
    assert_eq!(
        heading_findings("## roles\n", &["Roles"]),
        ["no heading \"## Roles\""],
        "headings compare exactly"
    );
}

#[test]
fn a_heading_inside_a_code_block_does_not_count() {
    let text = "```markdown\n## Roles\n```\n";
    assert_eq!(
        heading_findings(text, &["Roles"]).len(),
        1,
        "the fenced heading is not read"
    );
}

#[test]
fn an_anchor_drops_punctuation_and_keeps_hyphens() {
    assert_eq!(
        slug("How the project decides (today)"),
        "how-the-project-decides-today",
        "parentheses dropped"
    );
    assert_eq!(
        slug("`DCO` sign-off, step 2"),
        "dco-sign-off-step-2",
        "code marks and commas dropped"
    );
    assert_eq!(
        slug("If the maintainer is unavailable"),
        "if-the-maintainer-is-unavailable",
        "spaces become hyphens"
    );
}

#[test]
fn a_fragment_only_link_reads_its_own_page() {
    let tree = planted(&[]);
    let text = "## Reporting a vulnerability\n\nSee [above](#reporting-a-vulnerability) and [here](#nowhere).\n";
    let findings = link_findings(tree.path(), "GOVERNANCE.md", text);
    assert_eq!(
        findings.len(),
        1,
        "only the missing anchor is found: {findings:?}"
    );
    assert!(
        findings[0].contains("#nowhere"),
        "the finding names the anchor: {findings:?}"
    );
}

#[test]
fn crlf_line_ends_read_the_same() {
    let text = "## Roles\r\n\r\n| Role | Who |\r\n|---|---|\r\n| Maintainer | a |\r\n| Committer | b |\r\n| Contributor | c |\r\n";
    assert!(
        heading_findings(text, &["Roles"]).is_empty(),
        "a CRLF heading is read"
    );
    assert!(
        role_findings(&table_rows(text), &ROLES).is_empty(),
        "a CRLF table is read"
    );
}

#[test]
fn an_empty_table_has_no_roles() {
    let rows = table_rows("| Role | Who |\n|---|---|\n\nText.\n");
    assert!(
        rows.is_empty(),
        "a header and separator only have no rows: {rows:?}"
    );
    assert_eq!(
        role_findings(&rows, &ROLES).len(),
        3,
        "every role is missing"
    );
}

#[test]
fn a_role_there_twice_is_found() {
    let rows = table_rows(
        "|Role|Who|\n|---|---|\n|Maintainer|a|\n|Maintainer|b|\n|Committer|c|\n|Contributor|d|\n",
    );
    assert_eq!(
        role_findings(&rows, &ROLES),
        ["2 rows for Maintainer"],
        "the repeated role is named"
    );
}

#[test]
fn a_pipe_inside_code_stays_in_its_cell() {
    assert_eq!(
        cells("| `a|b` | c \\| d | e |"),
        ["`a|b`", "c | d", "e"],
        "the cell holds the pipe"
    );
}

#[test]
fn a_form_with_an_extra_optional_field_passes() {
    let mut form = String::from(
        "name: Starter task\nbody:\n  - type: markdown\n    attributes:\n      value: Read CONTRIBUTING.md.\n",
    );
    for field in FORM_FIELDS {
        form.push_str(&format!(
            "  - type: textarea\n    id: {field}\n    validations:\n      required: true\n"
        ));
    }
    form.push_str("  - type: input\n    id: notes\n    validations:\n      required: false\n");
    assert!(
        form_findings(&form_fields(&form), &FORM_FIELDS).is_empty(),
        "an optional extra field is allowed"
    );
    assert_eq!(
        form_fields(&form).len(),
        5,
        "the markdown block has no id and is not a field"
    );
    assert_eq!(
        form_fields(&form).last(),
        Some(&("notes".to_string(), false)),
        "`required: false` leaves the field optional"
    );
}

#[test]
fn a_commented_codeowners_line_is_not_read() {
    assert_eq!(
        owners_of_everything("# * @someone-else\n* @NormB # everything\n"),
        Some(vec!["@NormB".to_string()]),
        "comments are not owners"
    );
}

#[test]
fn the_last_codeowners_line_for_everything_wins() {
    assert_eq!(
        owners_of_everything("* @first\n/docs/ @docs\n* @second\n"),
        Some(vec!["@second".to_string()]),
        "GitHub applies the last matching line"
    );
}

#[test]
fn a_dot_slash_link_resolves() {
    let tree = planted(&[("MAINTAINERS.md", "## Maintainers\n")]);
    let text = "[list](./MAINTAINERS.md#maintainers)\n";
    assert!(
        link_findings(tree.path(), "GOVERNANCE.md", text).is_empty(),
        "./ is the page's directory"
    );
}

#[test]
fn links_inside_code_are_not_read() {
    let text = "```\n[a](missing-one.md)\n```\nUse `[b](missing-two.md)` as text. [c](found.md \"title\")\n";
    assert_eq!(
        links(text),
        ["found.md"],
        "only the prose link is read, without its title"
    );
}

#[test]
fn web_and_site_links_are_not_checked() {
    let tree = planted(&[]);
    let text = "[a](https://vcon-server.com/docs/) [b](mailto:security@example.com) [c](@/docs/security.md)\n";
    assert!(
        link_findings(tree.path(), "README.md", text).is_empty(),
        "only relative links are checked"
    );
}

#[test]
fn a_page_without_a_final_newline_is_read() {
    assert!(
        heading_findings("text\n\n## Roles", &["Roles"]).is_empty(),
        "the last line is a heading"
    );
}

#[test]
fn a_github_login_is_not_an_email_address() {
    assert!(
        emails("Ask @NormB on GitHub, or see `@/docs/security.md`.").is_empty(),
        "a login has no local part"
    );
    assert!(
        emails("A mention of @example.com is no address.").is_empty(),
        "an empty local part is no address"
    );
    // Assembled at run time: the commit hook refuses any real-looking
    // address in a tracked file, a GitHub noreply address included.
    let noreply = format!("12345+a{}users.noreply.github.com", '@');
    assert_eq!(
        emails(&format!("Signed-off-by: A <{noreply}>.")),
        [noreply.as_str()],
        "a noreply address is still an address"
    );
}

#[test]
fn a_sentence_that_denies_a_successor_passes() {
    let text =
        "There is one maintainer and no named successor. GOVERNANCE.md does not name a successor.";
    assert!(
        successor_findings(text).is_empty(),
        "both sentences say there is none"
    );
}

#[test]
fn a_section_ends_at_a_first_level_heading() {
    let text = "## Roles\n\nkept\n\n# Appendix\n\ndropped\n";
    assert_eq!(
        section(text, "Roles").trim(),
        "kept",
        "the first-level heading ends the section"
    );
}

#[test]
fn a_hash_without_a_space_is_no_heading() {
    let text = "#Roles\n\n## Roles\n";
    assert_eq!(
        headings(text),
        [(2, "Roles".to_string())],
        "`#Roles` is text, as GitHub renders it"
    );
}

#[test]
fn a_block_inside_a_list_item_reads_without_its_indent() {
    let listed = "- Check it:\n\n  ```sh\n  gh attestation verify a \\\n    --repo NormB/vcon-server\n  ```\n";
    let flat = "```sh\ngh attestation verify a \\\n  --repo NormB/vcon-server\n```\n";
    assert!(verify_command(flat).is_some(), "the flat block is found");
    assert_eq!(
        verify_command(listed),
        verify_command(flat),
        "the item's indent is not part of the command"
    );
}

#[test]
fn workflow_jobs_are_only_the_keys_under_jobs() {
    let workflow = "name: release\non:\n  push:\njobs:\n  publish:\n    name: GitHub release\n    steps:\n      - name: x\n  # a comment:\n  site:\n    needs: [publish]\n";
    assert_eq!(
        workflow_jobs(workflow),
        ["publish", "site"],
        "trigger keys and step names are not jobs"
    );
}
