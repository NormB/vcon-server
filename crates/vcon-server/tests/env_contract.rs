//! The environment variables' part of the contract
//! (website/content/docs/stability.md): contract/env.json holds each
//! variable the server reads, who reads it and what its value means, as the
//! table "Environment variables" of stability.md states them.
//! `scripts/contract_diff.py --kind env` compares two of these files.
//!
//! The table is held to the source. Every `std::env::var` and
//! `std::env::var_os` outside test code names a variable the table lists,
//! and each row read by vcon-server names a variable the source reads. Test
//! code is what only a test build compiles: an item under `#[cfg(test)]`
//! (or an `all(...)` that holds `test`), and every module file reached only
//! through one, found by walking the module tree from each crate's root.
//! The rows read by the AWS SDK are the variables configuration.md names
//! for its default credential chain, listed while the S3 backend loads it.
//!
//! The snapshot is written, never edited by hand: [`REGENERATE`] writes it
//! again, and the change is reviewed in the diff.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

#[path = "support/page_table.rs"]
mod page_table;

mod contract_snapshot {
    include!("support/contract_snapshot.rs");
}

/// The variable that makes [`the_snapshot_is_what_the_source_and_the_page_give`]
/// write the snapshot, as for contract/cli.json.
const WRITE: &str = "VCON_SERVER_WRITE_CONTRACT";

/// The one command that writes contract/env.json again.
const REGENERATE: &str =
    "VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --test env_contract";

/// The snapshot, from the repository root.
const SNAPSHOT: &str = "contract/env.json";

/// The page whose table lists the variables.
const PAGE: &str = "website/content/docs/stability.md";

/// The heading of that table.
const HEADING: &str = "## Environment variables";

/// The "Read by" cell of a variable the AWS SDK's default credential chain
/// reads, not the server's own code.
const SDK: &str = "the AWS SDK";

/// What loads the SDK's default credential chain in the S3 backend.
const CHAIN: &str = "aws_config::defaults(";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(root().join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
}

// ---- What only a test build compiles. ----

/// Whether the one-line attribute `attr` limits its item to test builds:
/// `#[test]`, `#[cfg(test)]`, `#[cfg(doctest)]`, or a `cfg(all(...))` with
/// `test` among its parts. `not(test)` and `any(test, ...)` do not.
fn test_only(attr: &str) -> bool {
    let a: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
    if a == "#[test]" || a == "#[cfg(test)]" || a == "#[cfg(doctest)]" {
        return true;
    }
    a.strip_prefix("#[cfg(all(")
        .and_then(|rest| rest.strip_suffix("))]"))
        .is_some_and(|inner| inner.split(',').any(|part| part == "test"))
}

/// The code of a line: the text before a `//` comment.
fn code(line: &str) -> &str {
    line.split("//").next().unwrap_or_default()
}

/// The `{` and the `}` of the code `line`, those in a string or a
/// character literal left out.
fn braces(line: &str) -> (i32, i32) {
    delimiters(line, '{', '}')
}

/// The `open` and the `close` characters of the code `line`, those in a
/// string or a character literal left out.
fn delimiters(line: &str, opener: char, closer: char) -> (i32, i32) {
    let (mut open, mut close) = (0, 0);
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '\'' if !in_string => {
                // A character literal such as '{' or '\''.
                let mut ahead = chars.clone();
                let first = ahead.next();
                let first = if first == Some('\\') {
                    ahead.next()
                } else {
                    first
                };
                if first.is_some() && ahead.next() == Some('\'') {
                    chars = ahead;
                }
            }
            c if c == opener && !in_string => open += 1,
            c if c == closer && !in_string => close += 1,
            _ => {}
        }
    }
    (open, close)
}

/// The last line of the item that starts at line `start`: the line that
/// closes its first `{`, or, before any `{` and outside parentheses, the
/// line that ends it with `;` or (a field, a variant, an arm) `,`.
fn item_end(lines: &[&str], start: usize) -> usize {
    let (mut depth, mut parens) = (0, 0);
    let mut opened = false;
    for (i, line) in lines.iter().enumerate().skip(start) {
        let c = code(line);
        let (open, close) = braces(c);
        let (p_open, p_close) = delimiters(c, '(', ')');
        depth += open - close;
        parens += p_open - p_close;
        opened |= open > 0;
        if opened && depth <= 0 {
            return i;
        }
        let end = c.trim_end();
        if !opened && parens <= 0 && (end.ends_with(';') || end.ends_with(',')) {
            return i;
        }
    }
    lines.len() - 1
}

/// For each line of the Rust source `text`, whether only a test build
/// compiles it: the lines of each item a test-only attribute stands on,
/// the attribute included.
fn test_lines(text: &str) -> Vec<bool> {
    let lines: Vec<&str> = text.lines().collect();
    let mut test = vec![false; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("#[") && test_only(line) {
            let mut start = i + 1;
            while start < lines.len() {
                let next = lines[start].trim();
                if next.starts_with("#[") || next.starts_with("//") || next.is_empty() {
                    start += 1;
                } else {
                    break;
                }
            }
            if start < lines.len() {
                let end = item_end(&lines, start);
                for t in &mut test[i..=end] {
                    *t = true;
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    test
}

// ---- The module tree. ----

/// A `mod` item of a file: its line, name, `#[path]` if any, whether its
/// body is inline, and the names of the inline modules around it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModItem {
    line: usize,
    name: String,
    path: Option<String>,
    inline: bool,
    within: Vec<String>,
}

/// The name and the body kind of a `mod` line: `mod x;` is (x, false),
/// `pub(crate) mod x {` is (x, true).
fn mod_line(line: &str) -> Option<(String, bool)> {
    let mut rest = code(line).trim();
    if let Some(r) = rest.strip_prefix("pub") {
        rest = match r.strip_prefix('(') {
            Some(inner) => inner.split_once(')')?.1.trim_start(),
            None => r.trim_start(),
        };
    }
    let rest = rest.strip_prefix("mod ")?;
    if let Some(name) = rest.strip_suffix(';') {
        return Some((name.trim().to_owned(), false));
    }
    let name = rest.strip_suffix('{')?;
    Some((name.trim().to_owned(), true))
}

/// The value of a `#[path = "..."]` attribute among the lines above
/// `line`, back to the item before it.
fn path_attr(lines: &[&str], line: usize) -> Option<String> {
    lines[..line]
        .iter()
        .rev()
        .map(|l| l.trim())
        .take_while(|l| l.starts_with("#[") || l.starts_with("//"))
        .find_map(|l| {
            let value = l.strip_prefix("#[path")?.trim_start().strip_prefix('=')?;
            Some(
                value
                    .trim()
                    .strip_suffix(']')?
                    .trim()
                    .trim_matches('"')
                    .to_owned(),
            )
        })
}

/// Every `mod` item of the Rust source `text`.
fn mod_items(text: &str) -> Vec<ModItem> {
    let lines: Vec<&str> = text.lines().collect();
    let mut inline: Vec<(usize, usize, String)> = Vec::new();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((name, is_inline)) = mod_line(line) else {
            continue;
        };
        if is_inline {
            inline.push((i, item_end(&lines, i), name.clone()));
        }
        let within = inline
            .iter()
            .filter(|(start, end, _)| *start < i && i <= *end)
            .map(|(_, _, n)| n.clone())
            .collect();
        found.push(ModItem {
            line: i,
            name,
            path: path_attr(&lines, i),
            inline: is_inline,
            within,
        });
    }
    found
}

/// `path` with each `.` and `..` taken out, as the module loader reads it.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The directory the `mod x;` items of `file` resolve in: the file's own
/// directory for a crate root or a `mod.rs`, else a directory named as the
/// file.
fn module_dir(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("")).to_path_buf();
    match file.file_name().and_then(|n| n.to_str()) {
        Some("lib.rs" | "main.rs" | "mod.rs") => dir,
        _ => dir.join(file.file_stem().unwrap_or_default()),
    }
}

/// The file a `mod x;` item of `file` names, of those in `files`.
fn module_file(file: &Path, item: &ModItem, files: &BTreeMap<PathBuf, String>) -> Option<PathBuf> {
    let mut dir = module_dir(file);
    for name in &item.within {
        dir.push(name);
    }
    let candidates = match &item.path {
        Some(p) if item.within.is_empty() => {
            vec![file.parent().unwrap_or(Path::new("")).join(p)]
        }
        Some(p) => vec![dir.join(p)],
        None => vec![
            dir.join(format!("{}.rs", item.name)),
            dir.join(&item.name).join("mod.rs"),
        ],
    };
    candidates
        .into_iter()
        .map(|c| normal(&c))
        .find(|c| files.contains_key(c))
}

/// Each file the module tree from `roots` reaches, with whether each line
/// is production code (compiled outside tests); and what the walk could
/// not follow: a production `mod x;` that names no file, and a file under
/// a `src/` that no root reaches.
fn walk(
    files: &BTreeMap<PathBuf, String>,
    roots: &[PathBuf],
) -> (BTreeMap<PathBuf, Vec<bool>>, Vec<String>) {
    let mut production = BTreeMap::new();
    let mut findings = Vec::new();
    let mut todo: Vec<(PathBuf, bool)> = roots.iter().map(|r| (r.clone(), false)).collect();
    while let Some((file, test)) = todo.pop() {
        if production.contains_key(&file) {
            continue;
        }
        let Some(text) = files.get(&file) else {
            findings.push(format!(
                "{}: a crate root that is not there",
                file.display()
            ));
            continue;
        };
        let tests = test_lines(text);
        production.insert(file.clone(), tests.iter().map(|t| !test && !t).collect());
        for item in mod_items(text).into_iter().filter(|m| !m.inline) {
            let child_test = test || tests.get(item.line).copied().unwrap_or(false);
            match module_file(&file, &item, files) {
                Some(child) => todo.push((child, child_test)),
                None if child_test => {}
                None => findings.push(format!(
                    "{}:{}: mod {}; names no file",
                    file.display(),
                    item.line + 1,
                    item.name
                )),
            }
        }
    }
    for file in files.keys().filter(|f| !production.contains_key(*f)) {
        findings.push(format!("{}: no crate root reaches it", file.display()));
    }
    (production, findings)
}

// ---- The reads. ----

/// The variables production code of `name` reads, and each read the scan
/// cannot name: a name that is not a literal, a read of every variable, a
/// `use` of an item of `std::env` that would hide a read.
fn reads(name: &str, text: &str, production: &[bool]) -> (BTreeSet<String>, Vec<String>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = BTreeSet::new();
    let mut findings = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !production.get(i).copied().unwrap_or(true) {
            continue;
        }
        let c = code(line);
        let at = format!("{name}:{}", i + 1);
        if c.contains("env::vars") {
            findings.push(format!("{at}: reads every variable"));
        }
        if c.contains("use std::env::") {
            findings.push(format!(
                "{at}: a use of std::env's items; write std::env::var in full"
            ));
        }
        for call in ["env::var(", "env::var_os("] {
            for (start, _) in c.match_indices(call) {
                let mut rest = c[start + call.len()..].trim_start();
                if rest.is_empty() {
                    rest = lines.get(i + 1).map_or("", |l| code(l).trim_start());
                }
                match rest
                    .strip_prefix('"')
                    .and_then(|r| r.split_once('"'))
                    .map(|(n, _)| n)
                {
                    Some(var) => {
                        found.insert(var.to_owned());
                    }
                    None => findings.push(format!("{at}: {call}) of a name that is not a literal")),
                }
            }
        }
    }
    (found, findings)
}

/// Every `.rs` file under each crate's `src/`, by absolute path, with its
/// text, and each crate's roots (`lib.rs`, `main.rs`).
fn sources() -> (BTreeMap<PathBuf, String>, Vec<PathBuf>) {
    fn walk_dir(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in std::fs::read_dir(dir).expect("read") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                walk_dir(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("read");
                out.insert(path, text);
            }
        }
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("the crates directory");
    let mut files = BTreeMap::new();
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(&crates).expect("the crates") {
        let src = entry.expect("an entry").path().join("src");
        if src.is_dir() {
            walk_dir(&src, &mut files);
            for root in ["lib.rs", "main.rs"] {
                if src.join(root).is_file() {
                    roots.push(src.join(root));
                }
            }
        }
    }
    roots.sort();
    (files, roots)
}

/// What the scan of this tree finds: the variables production code reads,
/// and each finding.
fn scan() -> (BTreeSet<String>, Vec<String>) {
    let (files, roots) = sources();
    let (production, mut findings) = walk(&files, &roots);
    let mut found = BTreeSet::new();
    for (file, lines) in &production {
        let (names, more) = reads(&file.display().to_string(), &files[file], lines);
        found.extend(names);
        findings.extend(more);
    }
    (found, findings)
}

// ---- The page. ----

/// One row of the table: who reads the variable, and what its value means.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    read_by: String,
    meaning: String,
}

/// The rows of the table under [`HEADING`] in `page`, by variable.
fn page_rows(page: &str) -> Result<BTreeMap<String, Row>, String> {
    let rows = page_table::table(page, HEADING).ok_or(format!("no table under {HEADING:?}"))?;
    if rows[0] != ["Variable", "Read by", "Meaning"] {
        return Err(format!("the table's header is {:?}", rows[0]));
    }
    let mut out = BTreeMap::new();
    for row in &rows[1..] {
        if row.len() != 3 {
            return Err(format!("the row {row:?} has {} cells", row.len()));
        }
        let name = row[0].trim_matches('`').to_owned();
        let entry = Row {
            read_by: row[1].clone(),
            meaning: row[2].clone(),
        };
        if out.insert(name.clone(), entry).is_some() {
            return Err(format!("{name} is listed twice"));
        }
    }
    Ok(out)
}

/// The rows the server's own code reads: those not read by [`SDK`].
fn own(rows: &BTreeMap<String, Row>) -> BTreeSet<String> {
    rows.iter()
        .filter(|(_, r)| r.read_by != SDK)
        .map(|(n, _)| n.clone())
        .collect()
}

/// The snapshot of `rows`, as contract_diff.py reads it.
fn render(rows: &BTreeMap<String, Row>) -> String {
    let variables: BTreeMap<&str, serde_json::Value> = rows
        .iter()
        .map(|(n, r)| {
            (
                n.as_str(),
                serde_json::json!({ "meaning": r.meaning, "read_by": r.read_by }),
            )
        })
        .collect();
    let mut text =
        serde_json::to_string_pretty(&serde_json::json!({ "variables": variables })).expect("JSON");
    text.push('\n');
    text
}

fn stale(file: &str, now: &str) -> String {
    contract_snapshot::stale(
        SNAPSHOT,
        "the source and this page give",
        REGENERATE,
        file,
        now,
    )
}

/// The rows of the page in this tree.
fn rows_now() -> BTreeMap<String, Row> {
    page_rows(&read(PAGE)).unwrap_or_else(|e| panic!("{PAGE}: {e}"))
}

// ---- Success: this tree. ----

#[test]
fn the_snapshot_is_what_the_source_and_the_page_give() {
    let now = render(&rows_now());
    let path = root().join(SNAPSHOT);
    if std::env::var_os(WRITE).is_some() {
        std::fs::write(&path, &now).expect("the snapshot is written");
        return;
    }
    let file = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(file == now, "{}", stale(&file, &now));
}

#[test]
fn the_source_reads_exactly_the_variables_the_page_lists_for_vcon_server() {
    let (found, _) = scan();
    let listed = own(&rows_now());
    assert_eq!(
        found, listed,
        "the variables the source reads outside tests, and those stability.md lists as read by vcon-server"
    );
}

#[test]
fn the_scan_finds_no_read_it_cannot_name_and_reaches_every_file() {
    let (_, findings) = scan();
    assert!(findings.is_empty(), "the scan's findings: {findings:#?}");
}

#[test]
fn the_scan_finds_the_tuis_variables_and_none_of_the_tests() {
    let (found, _) = scan();
    for name in ["NO_COLOR", "TERM"] {
        assert!(found.contains(name), "src/tui.rs reads {name}: {found:?}");
    }
    for name in [
        "VCON_SERVER_WRITE_SCREENS",
        "VCON_SERVER_WRITE_CONTRACT",
        "VCON_TEST_DUCKDB",
    ] {
        assert!(
            !found.contains(name),
            "{name} is read by tests only: {found:?}"
        );
    }
}

#[test]
fn the_scan_walks_every_source_file_of_both_crates() {
    let (files, roots) = sources();
    let (production, _) = walk(&files, &roots);
    assert!(files.len() > 50, "every crate's source: {}", files.len());
    assert_eq!(production.len(), files.len(), "the walk reaches each file");
    assert!(
        roots.iter().any(|r| r.ends_with("crates/vcon/src/lib.rs")),
        "the vcon crate is a root: {roots:?}"
    );
}

#[test]
fn the_sdk_rows_are_the_variables_configuration_md_names() {
    let rows = rows_now();
    let sdk: BTreeSet<String> = rows
        .iter()
        .filter(|(_, r)| r.read_by == SDK)
        .map(|(n, _)| n.clone())
        .collect();
    let page = read("website/content/docs/configuration.md");
    let named: BTreeSet<String> = page
        .split('`')
        .filter(|w| w.starts_with("AWS_") && w.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
        .map(str::to_owned)
        .collect();
    assert_eq!(
        sdk, named,
        "the AWS variables of stability.md and of configuration.md"
    );
}

#[test]
fn the_page_lists_sdk_rows_while_the_s3_backend_loads_the_default_chain() {
    let loads = read("crates/vcon-server/src/backend/s3.rs").contains(CHAIN);
    let listed = rows_now().values().any(|r| r.read_by == SDK);
    assert!(
        loads,
        "the S3 backend loads the SDK's default chain with {CHAIN}"
    );
    assert_eq!(
        listed, loads,
        "stability.md lists the SDK's variables while the chain is loaded"
    );
}

#[test]
fn the_tui_page_gives_each_tui_variable_the_meaning_the_contract_does() {
    let tui = read("website/content/docs/tui.md");
    for (name, row) in rows_now()
        .iter()
        .filter(|(_, r)| r.read_by == "`vcon-server tui`")
    {
        let line = format!("| `{name}` | {} |", row.meaning);
        assert!(tui.contains(&line), "tui.md has the row {line:?}");
    }
}

// ---- What only a test build compiles. ----

#[test]
fn cfg_test_doctest_test_and_all_with_test_are_test_only() {
    for attr in [
        "#[cfg(test)]",
        "#[cfg(doctest)]",
        "#[test]",
        "#[cfg(all(test, feature = \"s3\"))]",
        "#[cfg(all(feature = \"postgres\", test))]",
        "  #[cfg( test )]",
    ] {
        assert!(test_only(attr), "{attr} is test-only");
    }
}

#[test]
fn not_test_any_test_and_a_feature_are_not_test_only() {
    for attr in [
        "#[cfg(not(test))]",
        "#[cfg(any(test, feature = \"x\"))]",
        "#[cfg(feature = \"backend-tests\")]",
        "#[cfg(all(feature = \"testing\"))]",
        "#[derive(Debug)]",
    ] {
        assert!(!test_only(attr), "{attr} is compiled outside tests");
    }
}

#[test]
fn a_test_module_is_test_code_to_its_closing_brace() {
    let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() {\n    }\n}\nfn c() {}\n";
    assert_eq!(
        test_lines(text),
        [false, true, true, true, true, true, false],
        "the attribute to the module's own closing brace"
    );
}

#[test]
fn a_test_function_with_a_signature_over_lines_is_test_code() {
    let text = "#[test]\nfn long(\n    a: u8,\n) {\n    x();\n}\nfn after() {}\n";
    assert_eq!(
        test_lines(text),
        [true, true, true, true, true, true, false],
        "the whole function, from its attribute"
    );
}

#[test]
fn a_one_line_test_function_and_a_test_field_end_on_their_line() {
    let text =
        "#[test]\nfn a() {}\nfn b() {}\nstruct S {\n    #[cfg(test)]\n    f: u8,\n    g: u8,\n}\n";
    assert_eq!(
        test_lines(text),
        [true, true, false, false, true, true, false, false],
        "the item ends where its braces close, or at its comma"
    );
}

#[test]
fn braces_in_a_string_or_a_character_do_not_count() {
    assert_eq!(braces("format!(\"{x}\") {"), (1, 0), "a format string");
    assert_eq!(braces("if c == '{' {"), (1, 0), "a character literal");
    assert_eq!(braces("let s = \"\\\"{\"; }"), (0, 1), "an escaped quote");
    assert_eq!(
        braces("fn f<'a>(x: &'a str) {}"),
        (1, 1),
        "a lifetime is not a character"
    );
}

#[test]
fn an_attribute_over_another_attribute_and_a_comment_covers_the_item() {
    let text = "#[cfg(test)]\n#[allow(dead_code)]\n// why\nconst X: u8 = 1;\nconst Y: u8 = 2;\n";
    assert_eq!(
        test_lines(text),
        [true, true, true, true, false],
        "the item after the attributes and the comment"
    );
}

// ---- The reads. ----

fn all_production(text: &str) -> Vec<bool> {
    vec![true; text.lines().count()]
}

#[test]
fn a_read_in_production_code_is_found() {
    let text = "let a = std::env::var(\"ALPHA\");\nlet b = std::env::var_os(\"BETA\");\n";
    let (found, findings) = reads("x.rs", text, &all_production(text));
    assert_eq!(
        found,
        BTreeSet::from(["ALPHA".to_owned(), "BETA".to_owned()]),
        "var and var_os"
    );
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn a_read_in_test_code_is_not_found() {
    let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() { std::env::var(\"T\"); }\n}\n";
    let production: Vec<bool> = test_lines(text).iter().map(|t| !t).collect();
    let (found, _) = reads("x.rs", text, &production);
    assert!(found.is_empty(), "a test module's read: {found:?}");
}

#[test]
fn a_read_in_a_comment_is_not_found() {
    let text = "// std::env::var(\"GONE\")\nlet a = 1; // std::env::var_os(\"ALSO\")\n";
    let (found, findings) = reads("x.rs", text, &all_production(text));
    assert!(
        found.is_empty() && findings.is_empty(),
        "{found:?} {findings:?}"
    );
}

#[test]
fn a_read_with_its_name_on_the_next_line_is_found() {
    let text = "let a = std::env::var(\n    \"WRAPPED\",\n);\n";
    let (found, findings) = reads("x.rs", text, &all_production(text));
    assert_eq!(
        found,
        BTreeSet::from(["WRAPPED".to_owned()]),
        "{findings:?}"
    );
}

#[test]
fn two_reads_on_one_line_are_both_found() {
    let text = "f(std::env::var(\"A\").ok(), std::env::var(\"B\").ok());\n";
    let (found, _) = reads("x.rs", text, &all_production(text));
    assert_eq!(found.len(), 2, "both: {found:?}");
}

#[test]
fn a_read_of_a_name_that_is_not_a_literal_is_a_finding() {
    let text = "let a = std::env::var(name);\n";
    let (found, findings) = reads("x.rs", text, &all_production(text));
    assert!(found.is_empty(), "{found:?}");
    assert_eq!(
        findings,
        ["x.rs:1: env::var() of a name that is not a literal"],
        "the line is named"
    );
}

#[test]
fn reading_every_variable_is_a_finding() {
    let text = "for (k, v) in std::env::vars() {}\n";
    let (_, findings) = reads("x.rs", text, &all_production(text));
    assert_eq!(
        findings,
        ["x.rs:1: reads every variable"],
        "vars() names no variable"
    );
}

#[test]
fn a_use_of_std_envs_items_is_a_finding() {
    let text = "use std::env::var;\nlet a = var(\"HIDDEN\");\n";
    let (found, findings) = reads("x.rs", text, &all_production(text));
    assert!(found.is_empty(), "{found:?}");
    assert_eq!(findings.len(), 1, "the use is named: {findings:?}");
}

#[test]
fn env_and_option_env_at_compile_time_are_not_reads() {
    let text = "let v = env!(\"CARGO_PKG_VERSION\");\nlet w = option_env!(\"X\");\n";
    let (found, findings) = reads("x.rs", text, &all_production(text));
    assert!(
        found.is_empty() && findings.is_empty(),
        "{found:?} {findings:?}"
    );
}

// ---- The module tree. ----

fn tree(files: &[(&str, &str)]) -> BTreeMap<PathBuf, String> {
    files
        .iter()
        .map(|(p, t)| (PathBuf::from(p), (*t).to_owned()))
        .collect()
}

#[test]
fn mod_lines_with_and_without_a_body_and_visibility_are_read() {
    assert_eq!(
        mod_line("mod a;"),
        Some(("a".to_owned(), false)),
        "a file module"
    );
    assert_eq!(
        mod_line("pub mod b {"),
        Some(("b".to_owned(), true)),
        "an inline module"
    );
    assert_eq!(
        mod_line("    pub(crate) mod c;"),
        Some(("c".to_owned(), false)),
        "a restricted one"
    );
    assert_eq!(mod_line("// mod d;"), None, "a comment");
    assert_eq!(
        mod_line("let model = 1;"),
        None,
        "a word that starts with mod"
    );
}

#[test]
fn a_module_beside_lib_rs_and_under_a_named_file_is_reached() {
    let files = tree(&[
        ("/c/src/lib.rs", "mod a;\nmod b;\n"),
        ("/c/src/a.rs", "mod inner;\n"),
        ("/c/src/a/inner.rs", ""),
        ("/c/src/b/mod.rs", "mod deep;\n"),
        ("/c/src/b/deep.rs", ""),
    ]);
    let (production, findings) = walk(&files, &[PathBuf::from("/c/src/lib.rs")]);
    assert!(findings.is_empty(), "{findings:?}");
    assert_eq!(production.len(), 5, "every file: {:?}", production.keys());
}

#[test]
fn a_test_only_module_file_is_test_code_with_its_children() {
    let files = tree(&[
        ("/c/src/lib.rs", "#[cfg(test)]\nmod tests;\n"),
        (
            "/c/src/tests.rs",
            "mod more;\nfn a() { std::env::var(\"T\"); }\n",
        ),
        ("/c/src/tests/more.rs", "fn b() {}\n"),
    ]);
    let (production, findings) = walk(&files, &[PathBuf::from("/c/src/lib.rs")]);
    assert!(findings.is_empty(), "{findings:?}");
    for file in ["/c/src/tests.rs", "/c/src/tests/more.rs"] {
        assert!(
            production[Path::new(file)].iter().all(|p| !p),
            "{file} is test code"
        );
    }
}

#[test]
fn a_path_attribute_is_followed_from_the_files_directory() {
    let files = tree(&[
        ("/c/src/lib.rs", "mod a;\n"),
        ("/c/src/a.rs", "#[path = \"other/b.rs\"]\nmod b;\n"),
        ("/c/src/other/b.rs", ""),
    ]);
    let (production, findings) = walk(&files, &[PathBuf::from("/c/src/lib.rs")]);
    assert!(findings.is_empty(), "{findings:?}");
    assert!(
        production.contains_key(Path::new("/c/src/other/b.rs")),
        "the path names it"
    );
}

#[test]
fn a_module_under_an_inline_module_is_found_under_its_name() {
    let files = tree(&[
        ("/c/src/lib.rs", "mod outer {\n    mod leaf;\n}\n"),
        ("/c/src/outer/leaf.rs", ""),
    ]);
    let (production, findings) = walk(&files, &[PathBuf::from("/c/src/lib.rs")]);
    assert!(findings.is_empty(), "{findings:?}");
    assert!(
        production.contains_key(Path::new("/c/src/outer/leaf.rs")),
        "{:?}",
        production.keys()
    );
}

#[test]
fn a_production_module_that_names_no_file_is_a_finding() {
    let files = tree(&[("/c/src/lib.rs", "mod gone;\n")]);
    let (_, findings) = walk(&files, &[PathBuf::from("/c/src/lib.rs")]);
    assert_eq!(
        findings,
        ["/c/src/lib.rs:1: mod gone; names no file"],
        "named with its line"
    );
}

#[test]
fn a_test_module_that_names_a_file_outside_the_scan_is_no_finding() {
    let files = tree(&[(
        "/c/src/lib.rs",
        "#[cfg(test)]\n#[path = \"../tests/support/x.rs\"]\nmod x;\n",
    )]);
    let (_, findings) = walk(&files, &[PathBuf::from("/c/src/lib.rs")]);
    assert!(findings.is_empty(), "a test build's file: {findings:?}");
}

#[test]
fn a_file_no_root_reaches_is_a_finding() {
    let files = tree(&[("/c/src/lib.rs", ""), ("/c/src/stray.rs", "fn a() {}\n")]);
    let (_, findings) = walk(&files, &[PathBuf::from("/c/src/lib.rs")]);
    assert_eq!(
        findings,
        ["/c/src/stray.rs: no crate root reaches it"],
        "the stray file"
    );
}

#[test]
fn production_lines_of_a_file_follow_its_test_items() {
    let files = tree(&[(
        "/c/src/lib.rs",
        "fn a() {}\n#[cfg(test)]\nmod tests {\n}\nfn b() {}\n",
    )]);
    let (production, _) = walk(&files, &[PathBuf::from("/c/src/lib.rs")]);
    assert_eq!(
        production[Path::new("/c/src/lib.rs")],
        [true, false, false, false, true],
        "production around the test module"
    );
}

// ---- The page and the snapshot. ----

const SAMPLE: &str = "\
## Environment variables

| Variable | Read by | Meaning |
|---|---|---|
| `TERM` | `vcon-server tui` | `dumb` turns color off. |
| `AWS_ACCESS_KEY_ID` | the AWS SDK | The key id. |

## Next
";

#[test]
fn the_table_reader_reads_each_row() {
    let rows = page_rows(SAMPLE).expect("the sample's table");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows["TERM"].read_by, "`vcon-server tui`", "the reader");
    assert_eq!(
        own(&rows),
        BTreeSet::from(["TERM".to_owned()]),
        "the server's own"
    );
}

#[test]
fn a_page_without_the_table_is_refused() {
    assert!(page_rows("## Other\n\n| a |\n").is_err(), "no section");
}

#[test]
fn a_table_with_another_header_is_refused() {
    let page = SAMPLE.replace("| Variable | Read by | Meaning |", "| Name | Meaning |");
    assert!(page_rows(&page).is_err(), "the header is checked");
}

#[test]
fn a_variable_listed_twice_is_refused() {
    let page = SAMPLE.replace(
        "| `AWS_ACCESS_KEY_ID`",
        "| `TERM` | x | y |\n| `AWS_ACCESS_KEY_ID`",
    );
    assert_eq!(
        page_rows(&page).err().as_deref(),
        Some("TERM is listed twice"),
        "a duplicate"
    );
}

#[test]
fn the_rendered_snapshot_is_sorted_and_ends_with_a_newline() {
    let text = render(&page_rows(SAMPLE).expect("rows"));
    assert!(text.ends_with("}\n"), "a newline at the end");
    let aws = text.find("AWS_ACCESS_KEY_ID").expect("the SDK's row");
    let term = text.find("\"TERM\"").expect("the TUI's row");
    assert!(aws < term, "sorted by name");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(
        value["variables"]["TERM"]["read_by"], "`vcon-server tui`",
        "the shape contract_diff.py reads"
    );
}

#[test]
fn a_stale_snapshot_names_the_command_that_writes_it() {
    let message = stale("{\n  \"a\": 1\n}\n", "{\n  \"a\": 2\n}\n");
    assert!(message.contains(REGENERATE), "{message}");
    assert!(
        message.contains("contract/env.json is not what the source and this page give"),
        "{message}"
    );
}
