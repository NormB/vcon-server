//! The scale page (website/content/docs/scale.md) shows exactly the result
//! files committed in website/static/scale/, in file-name order, and links
//! each one.
//!
//! Its flags table gives each flag the default `vcon_scale::args::parse`
//! uses, and each failure it describes quotes, in full, a message the
//! harness or `scripts/scale-run.sh` prints: a reader searches the page
//! for the line on their terminal, and a paraphrase or a `...` in place of
//! a value finds nothing.

use std::path::{Path, PathBuf};

use serde_json::Value;
use vcon_scale::report;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn results() -> Vec<(String, Value)> {
    let dir = root().join("website/static/scale");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(&name))
                .unwrap_or_else(|e| panic!("read {name}: {e}"));
            let json = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, json)
        })
        .collect()
}

fn page() -> String {
    let path = root().join("website/content/docs/scale.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn the_page_table_is_the_committed_results() {
    let found = results();
    assert!(
        !found.is_empty(),
        "website/static/scale holds at least one result"
    );
    let jsons: Vec<Value> = found.iter().map(|(_, json)| json.clone()).collect();
    let table = report::page_table(&jsons).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        page().contains(&table),
        "scale.md must hold this table, made from the result files:\n{table}"
    );
}

#[test]
fn the_page_links_every_result_file() {
    let page = page();
    for (name, _) in results() {
        let link = format!("(https://vcon-server.com/scale/{name})");
        assert!(page.contains(&link), "scale.md links {name} as {link}");
    }
}

#[test]
fn each_result_file_is_named_for_its_label() {
    for (name, json) in results() {
        let label = json["label"].as_str().unwrap_or_default();
        assert_eq!(
            name,
            format!("{label}.json"),
            "the file is named for the run"
        );
    }
}

/// The part of `text` under the level 2 heading `heading`, up to the next.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let start = text
        .find(&format!("\n## {heading}\n"))
        .unwrap_or_else(|| panic!("scale.md has a section {heading:?}"));
    let body = &text[start + 1..];
    let end = body[3..].find("\n## ").map_or(body.len(), |at| at + 3);
    &body[..end]
}

/// The rows of the flags table under "Run it": each flag and its default,
/// without backticks.
fn flag_rows(page: &str) -> Vec<(String, String)> {
    section(page, "Run it")
        .lines()
        .filter(|line| line.starts_with("| `--"))
        .map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            (
                cells[1].trim_matches('`').to_owned(),
                cells[2].trim_matches('`').to_owned(),
            )
        })
        .collect()
}

#[test]
fn the_flags_table_gives_the_defaults_the_harness_uses() {
    let given: Vec<String> = [
        "--url",
        "https://127.0.0.1:1",
        "--key-file",
        "k",
        "--count",
        "1",
        "--out",
        "o.json",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let o = vcon_scale::args::parse(&given).unwrap_or_else(|why| panic!("{why:?}"));
    let used = [
        ("--concurrency", o.concurrency.to_string()),
        ("--reads", o.reads.to_string()),
        ("--list-pages", o.list_pages.to_string()),
        ("--seed", o.seed.to_string()),
        ("--mix", o.mix.clone()),
        ("--max-load", o.max_load.to_string()),
    ];
    let rows = flag_rows(&page());
    let flags: Vec<&str> = rows.iter().map(|(flag, _)| flag.as_str()).collect();
    let documented: Vec<&str> = used.iter().map(|(flag, _)| *flag).collect();
    assert_eq!(
        flags, documented,
        "the table under \"Run it\" lists the flags that have a default, in this order"
    );
    for ((flag, shown), (_, value)) in rows.iter().zip(used.iter()) {
        assert_eq!(shown, value, "scale.md gives {flag} the default parse uses");
    }
}

/// The string literals of a Rust source, a `\` line continuation joined.
fn rust_literals(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '\'' => {
                // A char literal or a lifetime: skip `'"'` and `'\"'` whole.
                let rest: String = chars.clone().take(3).collect();
                if rest.starts_with("\"'") {
                    chars.nth(1);
                } else if rest.starts_with("\\\"'") {
                    chars.nth(2);
                }
            }
            '"' => {
                let mut text = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('\n') => {
                                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                                    chars.next();
                                }
                            }
                            Some('n') => text.push('\n'),
                            Some(other) => text.push(other),
                            None => break,
                        },
                        other => text.push(other),
                    }
                }
                found.push(text);
            }
            _ => {}
        }
    }
    found
}

/// The messages `scripts/scale-run.sh` fails with: `fail "<text>"`, as
/// its `fail` function prints them.
fn script_messages(script: &str) -> Vec<String> {
    script
        .split("fail \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(|text| format!("scale-run: {text}"))
        .collect()
}

/// Whether `message` is what `template` prints: each `{...}` of the
/// template stands for at least one character.
fn prints(template: &str, message: &str) -> bool {
    let mut fixed = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        fixed.push(&rest[..open]);
        match rest[open..].find('}') {
            Some(close) => rest = &rest[open + close + 1..],
            None => return false,
        }
    }
    fixed.push(rest);
    let (first, others) = fixed.split_first().expect("one part at least");
    let Some(mut at) = message.strip_prefix(first) else {
        return false;
    };
    for (i, part) in others.iter().enumerate() {
        let last = i + 1 == others.len();
        if at.is_empty() {
            return false;
        }
        let from = at.char_indices().nth(1).map_or(at.len(), |(n, _)| n);
        if last {
            return at[from..].ends_with(part) && at.len() > part.len();
        }
        match at[from..].find(part) {
            Some(n) => at = &at[from + n + part.len()..],
            None => return false,
        }
    }
    at.is_empty()
}

/// Every message the harness can print: its string literals, each also
/// with the `vcon-scale: ` its error path puts before it, and the
/// script's failures.
fn templates() -> Vec<String> {
    let dir = root().join("crates/vcon-scale/src");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read crates/vcon-scale/src") {
        let path = entry.expect("a directory entry").path();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for literal in rust_literals(&text) {
            found.push(format!("vcon-scale: {literal}"));
            found.push(literal);
        }
    }
    let script = std::fs::read_to_string(root().join("scripts/scale-run.sh"))
        .expect("read scripts/scale-run.sh");
    found.extend(script_messages(&script));
    found.retain(|t| says_something(t));
    found
}

/// Whether a template holds words of its own besides a prefix: `{e}` and
/// `vcon-scale: {why}` print whatever they are given, so they would match
/// any message and prove nothing.
fn says_something(template: &str) -> bool {
    let mut fixed = String::new();
    let mut inside = false;
    for c in template.chars() {
        match c {
            '{' => inside = true,
            '}' => inside = false,
            other if !inside => fixed.push(other),
            _ => {}
        }
    }
    let words = ["vcon-scale:", "scale-run:"]
        .iter()
        .fold(fixed.trim(), |text, prefix| {
            text.strip_prefix(prefix).unwrap_or(text).trim()
        });
    words.chars().any(char::is_alphabetic)
}

/// The bold leads of the bullets under "When a run fails".
fn failure_leads(page: &str) -> Vec<String> {
    section(page, "When a run fails")
        .lines()
        .filter_map(|line| line.strip_prefix("- **"))
        .map(|lead| lead.split(":**").next().unwrap_or(lead).to_owned())
        .collect()
}

/// The backticked spans of `text`.
fn code_spans(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// The page's message with each `<name>` placeholder as `{}`.
fn as_template(message: &str) -> String {
    let mut out = String::new();
    let mut rest = message;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        match rest[open..].find('>') {
            Some(close) => {
                out.push('X');
                rest = &rest[open + close + 1..];
            }
            None => {
                rest = &rest[open..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn each_failure_on_the_page_quotes_a_message_the_harness_prints() {
    let page = page();
    let leads = failure_leads(&page);
    assert!(
        !leads.is_empty(),
        "\"When a run fails\" lists the failures as bullets with a bold lead"
    );
    let templates = templates();
    for lead in leads {
        let quoted = code_spans(&lead);
        assert!(
            !quoted.is_empty(),
            "the lead {lead:?} quotes the message in backticks, as printed"
        );
        for message in quoted {
            assert!(
                !message.contains("..."),
                "{message:?} shows the value with <name>, never `...`"
            );
            let shown = as_template(&message);
            assert!(
                templates.iter().any(|t| prints(t, &shown)),
                "{message:?} is no message vcon-scale or scale-run.sh prints"
            );
        }
    }
}

#[test]
fn the_message_readers_read_literals_scripts_and_placeholders() {
    let source = "let a = \"one \\\n        two\"; // \"not this\"\nlet q = '\"'; f(\"{x}: y\");";
    assert_eq!(
        rust_literals(source),
        vec!["one two".to_owned(), "{x}: y".to_owned()],
        "literals are joined at a continuation, and comments and char literals skipped"
    );
    assert_eq!(
        script_messages("fail \"the server stopped\"; fail \"no key\""),
        vec![
            "scale-run: the server stopped".to_owned(),
            "scale-run: no key".to_owned()
        ],
        "each fail call is a message with the script's prefix"
    );
    assert!(
        prints("{name}: {} of {} failed", "ingest: X of X failed"),
        "placeholders match any text"
    );
    assert!(
        prints(
            "the load is {l}, above the cap of {}: not started",
            "the load is X, above the cap of 28: not started"
        ),
        "a placeholder may stand for a number the page states"
    );
    assert!(
        !prints("first error: {why}", "first error: "),
        "a placeholder stands for at least one character"
    );
    assert!(
        !prints("the server stopped", "the server stops"),
        "fixed text must match exactly"
    );
    assert!(
        !says_something("vcon-scale: {why}") && !says_something("{e}"),
        "a template of placeholders alone is no message"
    );
    assert!(
        says_something("vcon-scale: first error: {why}"),
        "a template with words of its own is a message"
    );
    assert_eq!(
        as_template("a <load> b <part>"),
        "a X b X",
        "each <name> becomes one value"
    );
    assert_eq!(
        code_spans("**`one` and `two`**"),
        vec!["one".to_owned(), "two".to_owned()],
        "the spans between backticks"
    );
}
