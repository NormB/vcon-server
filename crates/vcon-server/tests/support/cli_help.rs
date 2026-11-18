//! Reading `vcon-server <path> --help` in a test, in one place:
//! tests/documented_commands.rs (the pages against the binary) and
//! tests/cli_contract.rs (the snapshot contract/cli.json) include this file.

#![allow(
    dead_code,
    reason = "each test that includes this file uses a part of it"
)]

use std::collections::HashMap;
use std::process::Command;

/// What `vcon-server <path> --help` shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Help {
    /// The names under `Commands:`, `help` among them.
    pub(crate) subcommands: Vec<String>,
    /// Every `--name` on an indented line, as [`flags`] finds them.
    pub(crate) flags: Vec<String>,
    /// Each entry under a heading of flags, such as `Options:`.
    pub(crate) options: Vec<Opt>,
    /// Each entry under `Arguments:`, in order.
    pub(crate) arguments: Vec<Positional>,
}

/// One flag, as its entry and the `Usage:` line show it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Opt {
    /// `--name`, or `-x` for a flag with no long name.
    pub(crate) key: String,
    /// `-x`, when it has one.
    pub(crate) short: Option<String>,
    /// The value it takes as shown: `<NAME>`, `[<NAME>]`, `<NAME>...`.
    pub(crate) value: Option<String>,
    /// It is outside every `[...]` of the `Usage:` line.
    pub(crate) required: bool,
    /// Its value ends in `...`.
    pub(crate) repeated: bool,
    /// What `[default: X]` names.
    pub(crate) default: Option<String>,
    /// What `[possible values: a, b]` or a `Possible values:` list names.
    pub(crate) values: Option<Vec<String>>,
}

/// One positional argument.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Positional {
    /// The name inside `<...>` or `[...]`.
    pub(crate) name: String,
    /// Shown as `<NAME>`, not `[NAME]`.
    pub(crate) required: bool,
    /// Shown with `...`.
    pub(crate) repeated: bool,
    /// What `[default: X]` names.
    pub(crate) default: Option<String>,
    /// What `[possible values: a, b]` or a `Possible values:` list names.
    pub(crate) values: Option<Vec<String>>,
}

/// Runs `vcon-server <path> --help` once per path and reads it.
pub(crate) fn help(
    path: &[String],
    cache: &mut HashMap<Vec<String>, Help>,
) -> Result<Help, String> {
    if let Some(h) = cache.get(path) {
        return Ok(h.clone());
    }
    let out = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .args(path)
        .arg("--help")
        .output()
        .expect("the binary runs");
    if !out.status.success() {
        return Err(format!(
            "`vcon-server {} --help` exits {:?}: {}",
            path.join(" "),
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8(out.stdout).expect("text");
    let h = parse_help(&text);
    cache.insert(path.to_vec(), h.clone());
    Ok(h)
}

/// The deepest indent of an entry's first line: clap puts `-x, --name` at
/// two spaces and `--name` alone at six; the text of an entry in the long
/// layout starts at ten.
const ENTRY_INDENT: usize = 6;

/// Where a line of `--help` is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    /// Before the first heading, or the `Usage:` lines.
    Other,
    Commands,
    Arguments,
    /// Any other heading: `Options:`, or one a command names.
    Flags,
}

/// One entry being read: its first line and the text that follows it.
enum Entry {
    Flag(Opt, String),
    Argument(Positional, String),
}

/// Reads the text of one `--help`.
pub(crate) fn parse_help(text: &str) -> Help {
    let mut h = Help::default();
    let mut section = Section::Other;
    let mut usage = String::new();
    let mut in_usage = false;
    let mut entry: Option<Entry> = None;
    for line in text.lines() {
        if line.trim().is_empty() {
            // The long layout keeps an entry's paragraphs apart with an
            // empty line; a heading ends the entry.
            in_usage = false;
            if let Some(Entry::Flag(_, text) | Entry::Argument(_, text)) = &mut entry {
                text.push('\n');
            }
            continue;
        }
        if !line.starts_with(' ') {
            finish(&mut h, entry.take());
            in_usage = false;
            if let Some(rest) = line.strip_prefix("Usage:") {
                usage.push_str(rest);
                in_usage = true;
                section = Section::Other;
            } else if line == "Commands:" {
                section = Section::Commands;
            } else if line == "Arguments:" {
                section = Section::Arguments;
            } else if line.ends_with(':') {
                section = Section::Flags;
            } else if !line.is_empty() {
                section = Section::Other;
            }
            continue;
        }
        h.flags.extend(flags(line));
        if in_usage {
            usage.push(' ');
            usage.push_str(line);
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        match section {
            Section::Commands => {
                let first = trimmed.split_whitespace().next().unwrap_or("");
                h.subcommands.push(first.to_owned());
            }
            Section::Flags if indent <= ENTRY_INDENT && trimmed.starts_with('-') => {
                finish(&mut h, entry.take());
                let (spec, rest) = split_entry(trimmed);
                entry = Some(Entry::Flag(flag_spec(spec), rest.to_owned()));
            }
            Section::Arguments
                if indent <= ENTRY_INDENT
                    && (trimmed.starts_with('<') || trimmed.starts_with('[')) =>
            {
                finish(&mut h, entry.take());
                let (spec, rest) = split_entry(trimmed);
                entry = Some(Entry::Argument(argument_spec(spec), rest.to_owned()));
            }
            _ => match &mut entry {
                Some(Entry::Flag(_, text) | Entry::Argument(_, text)) => {
                    text.push('\n');
                    text.push_str(trimmed);
                }
                None => {}
            },
        }
    }
    finish(&mut h, entry.take());
    let required = outside_brackets(&usage);
    for o in &mut h.options {
        o.required = required.iter().any(|w| *w == o.key);
    }
    h
}

/// An entry's first line, split where its text starts: at the first run of
/// two spaces.
fn split_entry(line: &str) -> (&str, &str) {
    match line.find("  ") {
        Some(at) => (&line[..at], line[at..].trim_start()),
        None => (line.trim_end(), ""),
    }
}

/// `-x, --name <VALUE>` as a flag.
fn flag_spec(spec: &str) -> Opt {
    let mut o = Opt::default();
    let mut long = None;
    let mut value = Vec::new();
    for word in spec.split_whitespace() {
        let word = word.trim_end_matches(',');
        if value.is_empty() && word.starts_with("--") {
            long = Some(word.to_owned());
        } else if value.is_empty() && word.starts_with('-') {
            o.short = Some(word.to_owned());
        } else {
            value.push(word);
        }
    }
    o.key = long.or_else(|| o.short.clone()).unwrap_or_default();
    if !value.is_empty() {
        let value = value.join(" ");
        o.repeated = value.ends_with("...");
        o.value = Some(value);
    }
    o
}

/// `<NAME>`, `[NAME]` or `<NAME>...` as an argument.
fn argument_spec(spec: &str) -> Positional {
    let repeated = spec.ends_with("...");
    let bare = spec.trim_end_matches("...");
    Positional {
        name: bare
            .trim_matches(|c| matches!(c, '<' | '>' | '[' | ']'))
            .to_owned(),
        required: bare.starts_with('<'),
        repeated,
        ..Positional::default()
    }
}

/// Adds a finished entry to `h`, with the default and the possible values
/// its text names.
fn finish(h: &mut Help, entry: Option<Entry>) {
    match entry {
        Some(Entry::Flag(mut o, text)) => {
            o.default = bracketed(&text, "[default: ");
            o.values = possible_values(&text);
            h.options.push(o);
        }
        Some(Entry::Argument(mut a, text)) => {
            a.default = bracketed(&text, "[default: ");
            a.values = possible_values(&text);
            h.arguments.push(a);
        }
        None => {}
    }
}

/// What `[<label>X]` holds in `text`.
fn bracketed(text: &str, label: &str) -> Option<String> {
    let start = text.find(label)? + label.len();
    let end = text[start..].find(']')?;
    Some(text[start..start + end].to_owned())
}

/// `[possible values: a, b]`, or a `Possible values:` line followed by one
/// `- value` or `- value: text` line each.
fn possible_values(text: &str) -> Option<Vec<String>> {
    if let Some(list) = bracketed(text, "[possible values: ") {
        return Some(list.split(", ").map(str::to_owned).collect());
    }
    let after = text.split_once("Possible values:")?.1;
    let values: Vec<String> = after
        .lines()
        .map(str::trim)
        .skip_while(|l| l.is_empty())
        .map_while(|l| l.strip_prefix("- "))
        .map(|v| v.split(':').next().unwrap_or(v).trim().to_owned())
        .collect();
    Some(values)
}

/// The words of a `Usage:` line outside every `[...]`.
fn outside_brackets(usage: &str) -> Vec<&str> {
    let mut depth = 0usize;
    let mut kept = Vec::new();
    for word in usage.split_whitespace() {
        let opens = word.matches('[').count();
        let closes = word.matches(']').count();
        if depth == 0 && opens == 0 {
            kept.push(word);
        }
        depth = (depth + opens).saturating_sub(closes);
    }
    kept
}

/// Every `--name` in `text`, such as in `[--force [--override-holds]]` or
/// `--tenant=<id>`.
pub(crate) fn flags(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("--") {
        let after = &rest[i + 2..];
        let end = after
            .find(|c: char| !(c.is_ascii_lowercase() || c == '-'))
            .unwrap_or(after.len());
        let name = &after[..end];
        let starts_word = i == 0 || !rest[..i].ends_with(|c: char| c.is_alphanumeric());
        if starts_word && !name.is_empty() && !name.starts_with('-') {
            out.push(format!("--{}", name.trim_end_matches('-')));
        }
        rest = &after[end..];
    }
    out
}

/// Every command of the binary that has no subcommands of its own, as its
/// words after `vcon-server`, found by walking `--help`.
pub(crate) fn leaves(
    path: Vec<String>,
    cache: &mut HashMap<Vec<String>, Help>,
) -> Vec<Vec<String>> {
    let h = help(&path, cache).unwrap_or_else(|e| panic!("{e}"));
    let subcommands: Vec<&String> = h.subcommands.iter().filter(|s| *s != "help").collect();
    if subcommands.is_empty() {
        return vec![path];
    }
    subcommands
        .into_iter()
        .flat_map(|sub| {
            let mut next = path.clone();
            next.push(sub.clone());
            leaves(next, cache)
        })
        .collect()
}

/// Every command of the binary, `path` first, then each subcommand's
/// tree in the order `--help` lists them; `help` is left out.
pub(crate) fn commands(
    path: Vec<String>,
    cache: &mut HashMap<Vec<String>, Help>,
) -> Vec<Vec<String>> {
    let h = help(&path, cache).unwrap_or_else(|e| panic!("{e}"));
    let mut all = vec![path.clone()];
    for sub in h.subcommands.iter().filter(|s| *s != "help") {
        let mut next = path.clone();
        next.push(sub.clone());
        all.extend(commands(next, cache));
    }
    all
}
