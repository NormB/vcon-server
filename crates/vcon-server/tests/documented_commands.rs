//! Every `vcon-server` command the documentation shows exists in the
//! binary: each subcommand, and each `--flag` it is shown with, is in that
//! subcommand's `--help`. A command in a shell block is one the reader
//! pastes, so it also carries `--config`, which every command but
//! `--version` and `--help` requires.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The pages read, from the repository root: the site's docs and the
/// READMEs.
fn pages() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut pages: Vec<PathBuf> = std::fs::read_dir(root.join("website/content/docs"))
        .expect("the docs directory")
        .map(|e| e.expect("an entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    pages.sort();
    pages.push(root.join("README.md"));
    pages.push(root.join("crates/vcon-server/README.md"));
    pages
}

/// One documented command: where it is, its words after `vcon-server`, and
/// whether the reader pastes it as it stands (a shell block or a unit's
/// `ExecStart=`) rather than reads it as a form.
#[derive(Debug)]
struct Documented {
    at: String,
    words: Vec<String>,
    pasted: bool,
}

/// The words after `vcon-server` when `command` runs it: after any `sudo`
/// and its options, and any `NAME=value` settings; the program may be named
/// by a path.
fn invocation(command: &str) -> Option<Vec<String>> {
    let mut words: Vec<&str> = command.split_whitespace().collect();
    // `NAME=$(command ...`: a variable keeps the command's output.
    if let Some((name, rest)) = words.first().and_then(|w| w.split_once("=$("))
        && !name.is_empty()
        && !name.starts_with('-')
    {
        words[0] = rest;
    }
    let mut words = words.into_iter().filter(|w| !w.is_empty()).peekable();
    if words.peek() == Some(&"sudo") {
        words.next();
        while let Some(w) = words.peek() {
            match *w {
                "-u" | "-g" => {
                    words.next();
                    words.next();
                }
                w if w.starts_with('-') => {
                    words.next();
                }
                _ => break,
            }
        }
    }
    while words.peek().is_some_and(|w| {
        w.split_once('=')
            .is_some_and(|(name, _)| !name.is_empty() && !name.starts_with('-'))
            && !w.starts_with("ExecStart=")
    }) {
        words.next();
    }
    let program = words.next()?;
    let program = program.strip_prefix("ExecStart=").unwrap_or(program);
    (Path::new(program).file_name()? == "vcon-server").then(|| words.map(str::to_owned).collect())
}

/// Every command on one page.
fn documented(page: &Path) -> Vec<Documented> {
    let text = std::fs::read_to_string(page).unwrap_or_else(|e| panic!("{}: {e}", page.display()));
    let name = page
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    let command_table = name == "command-line.md";
    let mut found = Vec::new();
    let mut fence: Option<String> = None;
    let mut joined = String::new();
    for (n, line) in text.lines().enumerate() {
        let at = format!("{name}:{}", n + 1);
        if let Some(rest) = line.trim_start().strip_prefix("```") {
            fence = match fence {
                None => Some(rest.trim().to_owned()),
                Some(_) => None,
            };
            continue;
        }
        match fence.as_deref() {
            Some("sh" | "bash" | "ini") => {
                // A line ending in `\` goes on.
                joined.push_str(line.trim_end_matches('\\'));
                joined.push(' ');
                if line.ends_with('\\') {
                    continue;
                }
                let full = std::mem::take(&mut joined);
                let code = full.trim();
                if code.starts_with('#') {
                    continue;
                }
                for command in code.split(['|', ';', '&']) {
                    if let Some(words) = invocation(command) {
                        found.push(Documented {
                            at: at.clone(),
                            words,
                            pasted: true,
                        });
                    }
                }
            }
            Some(_) => {}
            None => {
                let spans = code_spans(line);
                for (i, span) in spans.iter().enumerate() {
                    let words = if let Some(rest) = span.strip_prefix("vcon-server ") {
                        Some(rest)
                    } else if command_table && i == 0 && line.starts_with("| `") {
                        // The command line page's tables leave out the
                        // program's name.
                        Some(*span)
                    } else {
                        None
                    };
                    if let Some(words) = words {
                        found.push(Documented {
                            at: at.clone(),
                            words: words.split_whitespace().map(str::to_owned).collect(),
                            pasted: false,
                        });
                    }
                }
            }
        }
    }
    found
}

/// The code spans of one line of prose, as CommonMark reads them: a run of
/// backticks opens a span that the next run of the same length closes, so a
/// span may hold shorter or longer runs; a run with no such closer is text.
/// One space is stripped from each end of a span that has one at both.
fn code_spans(line: &str) -> Vec<&str> {
    let run = |at: usize| line[at..].bytes().take_while(|&b| b == b'`').count();
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(found) = line[at..].find('`') {
        let open = at + found;
        let width = run(open);
        let start = open + width;
        let mut closer = None;
        let mut look = start;
        while let Some(next) = line[look..].find('`') {
            let candidate = look + next;
            let length = run(candidate);
            if length == width {
                closer = Some(candidate);
                break;
            }
            look = candidate + length;
        }
        match closer {
            Some(end) => {
                let inner = &line[start..end];
                let inner = match inner.strip_prefix(' ').and_then(|i| i.strip_suffix(' ')) {
                    Some(trimmed) if !trimmed.trim().is_empty() => trimmed,
                    _ => inner,
                };
                spans.push(inner);
                at = end + width;
            }
            None => at = start,
        }
    }
    spans
}

/// What `vcon-server <path> --help` shows: its subcommands and its flags.
#[derive(Debug, Clone)]
struct Help {
    subcommands: Vec<String>,
    flags: Vec<String>,
}

fn help(path: &[String], cache: &mut HashMap<Vec<String>, Help>) -> Result<Help, String> {
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
    let mut h = Help {
        subcommands: Vec::new(),
        flags: Vec::new(),
    };
    let mut in_commands = false;
    for line in text.lines() {
        if !line.starts_with(' ') {
            in_commands = line == "Commands:";
            continue;
        }
        let first = line.split_whitespace().next().unwrap_or("");
        if in_commands {
            h.subcommands.push(first.to_owned());
        }
        h.flags.extend(flags(line));
    }
    cache.insert(path.to_vec(), h.clone());
    Ok(h)
}

/// Every `--name` in `text`, such as in `[--force [--override-holds]]` or
/// `--tenant=<id>`.
fn flags(text: &str) -> Vec<String> {
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

/// What is wrong with one documented command, if anything.
fn fault(command: &Documented, cache: &mut HashMap<Vec<String>, Help>) -> Option<String> {
    let mut path: Vec<String> = Vec::new();
    let mut words = command.words.iter().peekable();
    let leaf = loop {
        let h = match help(&path, cache) {
            Ok(h) => h,
            Err(e) => return Some(e),
        };
        match words.peek() {
            Some(w) if h.subcommands.contains(w) => {
                path.push((*w).clone());
                words.next();
            }
            Some(w) if !h.subcommands.is_empty() && !w.starts_with("--") => {
                return Some(format!(
                    "`vcon-server {}` has no subcommand `{w}`",
                    path.join(" ")
                ));
            }
            _ => break h,
        }
    };
    let shown: Vec<String> = command.words.iter().flat_map(|w| flags(w)).collect();
    if let Some(flag) = shown.iter().find(|f| !leaf.flags.contains(f)) {
        return Some(format!(
            "`vcon-server {}` has no flag `{flag}`",
            path.join(" ")
        ));
    }
    let informational = shown.iter().any(|f| f == "--version" || f == "--help");
    if command.pasted && !informational && !shown.iter().any(|f| f == "--config") {
        return Some(format!(
            "`vcon-server {}` is shown without `--config`, which it requires",
            command.words.join(" ")
        ));
    }
    None
}

#[test]
fn every_documented_command_and_flag_exists() {
    let mut cache = HashMap::new();
    let mut checked = 0;
    let mut faults = Vec::new();
    for page in pages() {
        for command in documented(&page) {
            checked += 1;
            if let Some(fault) = fault(&command, &mut cache) {
                faults.push(format!("{}: {fault}", command.at));
            }
        }
    }
    eprintln!("{checked} documented commands checked");
    assert!(faults.is_empty(), "{}", faults.join("\n"));
    // 39 when this test was written: 11 inline commands, 13 rows of the
    // command line page's tables and 15 commands in blocks. Fewer means the
    // reader above missed some, not that they are right.
    assert!(checked >= 39, "only {checked} documented commands found");
}

/// The reader itself: each kind of mistake is found, and the forms the
/// pages use are read.
#[test]
fn a_missing_subcommand_flag_or_config_is_found() {
    let mut cache = HashMap::new();
    let check = |words: &str, pasted: bool, cache: &mut HashMap<_, _>| {
        fault(
            &Documented {
                at: "test".into(),
                words: words.split_whitespace().map(str::to_owned).collect(),
                pasted,
            },
            cache,
        )
    };
    assert_eq!(
        check("check --config c.toml", true, &mut cache),
        None,
        "a real subcommand with --config is a sound pasted command"
    );
    assert_eq!(
        check("--version", true, &mut cache),
        None,
        "--version is the one pasted command that needs no --config"
    );
    assert_eq!(
        check(
            "tenant remove <id> [--force [--override-holds]]",
            false,
            &mut cache
        ),
        None,
        "a form from the command line page's tables, with its brackets, names real flags"
    );
    for (words, pasted, says) in [
        ("tenant purge x --config c", true, "no subcommand `purge`"),
        ("frobnicate --config c", true, "no subcommand `frobnicate`"),
        (
            "tenant create a --colour --config c",
            true,
            "no flag `--colour`",
        ),
        (
            "key create --tenant t [--scopes s]",
            false,
            "no flag `--scopes`",
        ),
        ("check", true, "without `--config`"),
    ] {
        let fault = check(words, pasted, &mut cache).unwrap_or_else(|| panic!("{words}: no fault"));
        assert!(fault.contains(says), "{words}: {fault}");
    }
    assert_eq!(
        invocation("sudo -u vcon /usr/local/bin/vcon-server check --config c"),
        Some(vec!["check".into(), "--config".into(), "c".into()]),
        "sudo, its options and a path to the program come off before the words"
    );
    assert_eq!(
        invocation("ExecStart=/usr/local/bin/vcon-server serve --config c"),
        Some(vec!["serve".into(), "--config".into(), "c".into()]),
        "a unit's ExecStart= is the command systemd runs"
    );
    // A command whose output a variable keeps.
    assert_eq!(
        invocation("TENANT=$(sudo -u vcon vcon-server tenant list --config c"),
        Some(vec![
            "tenant".into(),
            "list".into(),
            "--config".into(),
            "c".into()
        ]),
        "the pages keep ids and tokens with NAME=$(command), and that command is still documented"
    );
    assert_eq!(
        invocation("cargo install vcon-server --locked"),
        None,
        "vcon-server as an argument to another program is not a command of its own"
    );
    // A span opens and closes on backtick runs of the same length
    // (CommonMark), so a span may hold backticks of its own.
    assert_eq!(
        code_spans("see `one`, ``a span holding `inner` backticks`` and `two`"),
        ["one", "a span holding `inner` backticks", "two"],
        "CommonMark closes a span on a run as long as the one that opened it"
    );
    assert_eq!(
        code_spans("``error: `vcon-server <version>` is old``"),
        ["error: `vcon-server <version>` is old"],
        "a quoted error is one span, so the command inside it is not read as documented"
    );
    // A longer run inside does not close a span.
    assert_eq!(
        code_spans("`a ``b`` c`"),
        ["a ``b`` c"],
        "a longer run is part of the span, not its end"
    );
    // A run with no closing run of its length is text, not a span.
    assert_eq!(
        code_spans("a `` b `c`"),
        ["c"],
        "an opener with no closer of its length is text, and the scan goes on after it"
    );
    assert_eq!(
        invocation("cd vcon-server"),
        None,
        "a directory named vcon-server is not the program"
    );
}

/// Each page that downloads a release, and the variable that names its version.
const PAGES: &[(&str, &str)] = &[
    ("install.md", "VERSION="),
    ("container.md", "VERSION="),
    ("upgrading.md", "NEW_VERSION="),
];

/// The release each page downloads is this build's version, so a page names
/// a release that exists once this version is tagged: install and the
/// container image set `VERSION`, the upgrade sets `NEW_VERSION`.
#[test]
fn the_release_each_page_downloads_is_this_version() {
    let docs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../website/content/docs");
    for &(page, variable) in PAGES {
        let text = std::fs::read_to_string(docs.join(page)).expect(page);
        let versions: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix(variable))
            .collect();
        assert_eq!(
            versions,
            [env!("CARGO_PKG_VERSION")],
            "{page} names one release, this build's, so its download exists once this version is \
             tagged"
        );
    }
}
