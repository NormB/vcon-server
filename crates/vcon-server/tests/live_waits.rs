//! The gate on live waits: in code that only the `backend-tests` feature
//! compiles (the tests against real PostgreSQL servers and S3 services),
//! a wait for something to happen takes its bound from
//! tests/support/live_deadline.rs, by one rule, and never a fixed number
//! of seconds of its own.
//!
//! On 2026-11-26 two live tests failed at a host load of 38-45 because
//! they waited a fixed 5 s (a connect timeout); #293 put the connections
//! on `LIVE_DEADLINE`, and this gate holds every other live wait to it:
//! polls, channel waits, deadlines, curl's transfer time.
//!
//! What it refuses, inside a live region:
//! - `counted-poll`: a `for _ in 0..N` or `(0..N).` loop that sleeps, whose
//!   bound is N pauses rather than a deadline;
//! - `fixed-deadline`: `Instant::now() + ...` without `LIVE_DEADLINE`;
//! - `fixed-timeout`: `recv_timeout(`, `time::timeout(` or `.line_with(`
//!   given a `Duration::from_...` literal;
//! - `curl-bound`: a `"--max-time"` written out, or `Command::new("curl")`:
//!   a live curl is `live_deadline::curl(...)`, which sets it;
//! - `fixed-const`: a `const NAME: Duration` without `LIVE_DEADLINE`;
//! - `fixed-watch`: `elapsed() <` against a `Duration::from_...` literal;
//! - `fixed-helper`: a call of a wait helper (`poll(`, `poll_async(`,
//!   `within_of(`) whose arguments do not name `LIVE_DEADLINE`.
//!
//! And anywhere in the package, live or not: `shadowed-deadline`, a
//! `const`, `static` or `let` named `LIVE_DEADLINE` outside
//! tests/support/live_deadline.rs, which would pass the rules above with a
//! fixed value of its own.
//!
//! A wait that is fixed on purpose (a check that something does NOT
//! happen within a short time, or a duration the code under test takes as
//! input) is listed in tests/support/live_waits_allowed.txt with its
//! reason; an entry that matches nothing is refused too, so the list holds
//! only what is there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One fixed wait found: its line (1-based), the rule, and the line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Finding {
    line: usize,
    rule: &'static str,
    text: String,
}

/// One entry of the allow list: a file, a fragment of the line, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Allow {
    path: String,
    fragment: String,
    reason: String,
}

/// `text` with every comment blanked to spaces and, inside string and
/// character literals, every bracket blanked, so that brackets can be
/// counted; lines and columns are kept.
fn blank(text: &str) -> String {
    blank_with(text, false)
}

/// `blank`, and with `strings` every byte inside a literal blanked too.
fn blank_with(text: &str, strings: bool) -> String {
    let src = text.as_bytes();
    let mut out = src.to_vec();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bracket = |b: u8| {
        if strings {
            b != b'\n'
        } else {
            matches!(b, b'{' | b'}' | b'(' | b')' | b'[' | b']')
        }
    };
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with(b"//") {
            while i < src.len() && src[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if rest.starts_with(b"/*") {
            let mut depth = 0;
            while i < src.len() {
                if src[i..].starts_with(b"/*") {
                    depth += 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                } else if src[i..].starts_with(b"*/") {
                    depth -= 1;
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if src[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
        } else if (src[i] == b'r' || rest.starts_with(b"br")) && (i == 0 || !ident(src[i - 1])) && {
            let start = if src[i] == b'r' { i + 1 } else { i + 2 };
            let hashes = src[start..].iter().take_while(|&&b| b == b'#').count();
            src.get(start + hashes) == Some(&b'"')
        } {
            let start = if src[i] == b'r' { i + 1 } else { i + 2 };
            let hashes = src[start..].iter().take_while(|&&b| b == b'#').count();
            let mut end_mark = vec![b'"'];
            end_mark.extend(std::iter::repeat_n(b'#', hashes));
            let mut j = start + hashes + 1;
            while j < src.len() && !src[j..].starts_with(&end_mark) {
                if bracket(src[j]) {
                    out[j] = b' ';
                }
                j += 1;
            }
            i = j + end_mark.len();
        } else if src[i] == b'"' {
            let mut j = i + 1;
            while j < src.len() && src[j] != b'"' {
                if src[j] == b'\\' {
                    if strings {
                        out[j] = b' ';
                    }
                    if j + 1 < src.len() && bracket(src[j + 1]) {
                        out[j + 1] = b' ';
                    }
                    j += 2;
                    continue;
                }
                if bracket(src[j]) {
                    out[j] = b' ';
                }
                j += 1;
            }
            i = j + 1;
        } else if src[i] == b'\'' {
            // A character literal is '\x..' or one character then a quote;
            // anything else is a lifetime or a label.
            let end = if src.get(i + 1) == Some(&b'\\') {
                src[i + 2..]
                    .iter()
                    .position(|&b| b == b'\'')
                    .map(|p| i + 2 + p)
            } else {
                let width = text[i + 1..].chars().next().map_or(1, char::len_utf8);
                (src.get(i + 1 + width) == Some(&b'\'')).then_some(i + 1 + width)
            };
            match end {
                Some(end) => {
                    for b in &mut out[i + 1..end] {
                        if bracket(*b) {
                            *b = b' ';
                        }
                    }
                    i = end + 1;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("only ASCII bytes were replaced, by spaces")
}

/// The offset in `text` of the bracket that closes the one at `open`.
fn closing(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let (up, down) = match bytes[open] {
        b'{' => (b'{', b'}'),
        b'(' => (b'(', b')'),
        _ => return None,
    };
    let mut depth = 0_usize;
    for (k, &b) in bytes.iter().enumerate().skip(open) {
        if b == up {
            depth += 1;
        } else if b == down {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

/// The offset where each line of `text` starts.
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(k, _)| k + 1))
        .collect()
}

/// The line (0-based) of `offset`.
fn line_of(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&s| s <= offset) - 1
}

/// Whether `line` is an attribute that compiles what follows only with
/// the `backend-tests` feature.
fn gates_on_backend_tests(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("#[cfg(") && t.contains("feature = \"backend-tests\"") && !t.contains("not(")
}

/// For each gating attribute of `text`: its line, and the line of the
/// item it gates, past other attributes and doc comments.
fn gated_items(text: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !gates_on_backend_tests(line) {
            continue;
        }
        let mut j = i + 1;
        while j < lines.len() {
            let t = lines[j].trim_start();
            if t.starts_with("#[") {
                let (mut opened, mut closed) = (0_usize, 0_usize);
                loop {
                    opened += lines[j].matches('[').count();
                    closed += lines[j].matches(']').count();
                    if closed >= opened || j + 1 >= lines.len() {
                        break;
                    }
                    j += 1;
                }
                j += 1;
            } else if t.starts_with("//") || t.is_empty() {
                j += 1;
            } else {
                break;
            }
        }
        if j < lines.len() {
            out.push((i, j));
        }
    }
    out
}

/// The line ranges (0-based, inclusive) of the items `text` compiles only
/// with the `backend-tests` feature.
fn live_regions(text: &str) -> Vec<(usize, usize)> {
    let code = blank(text);
    let starts = line_starts(&code);
    let mut out = Vec::new();
    for (attr, item) in gated_items(text) {
        let from = starts[item];
        let brace = code[from..].find('{').map(|k| from + k);
        let semi = code[from..].find(';').map(|k| from + k);
        let Some(brace) = brace else { continue };
        if semi.is_some_and(|s| s < brace) {
            continue;
        }
        if let Some(end) = closing(&code, brace) {
            out.push((attr, line_of(&starts, end)));
        }
    }
    out
}

/// The files that `mod name;` items of `path` gated on `backend-tests`
/// name: live as a whole.
fn live_files(path: &Path, text: &str) -> Vec<PathBuf> {
    let lines: Vec<&str> = text.lines().collect();
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let root_like = matches!(name, "mod.rs" | "lib.rs" | "main.rs")
        || dir.file_name().is_some_and(|d| d == "tests");
    let base = if root_like {
        dir.to_path_buf()
    } else {
        dir.join(path.file_stem().unwrap_or_default())
    };
    let mut out = Vec::new();
    for (attr, item) in gated_items(text) {
        let t = lines[item].trim();
        let Some(rest) = t
            .strip_prefix("pub(crate) mod ")
            .or_else(|| t.strip_prefix("pub(super) mod "))
            .or_else(|| t.strip_prefix("pub mod "))
            .or_else(|| t.strip_prefix("mod "))
        else {
            continue;
        };
        let Some(module) = rest.strip_suffix(';') else {
            continue;
        };
        let explicit = lines[attr..item].iter().find_map(|l| {
            let v = l
                .trim()
                .strip_prefix("#[path")?
                .trim_start()
                .strip_prefix('=')?;
            Some(
                v.trim()
                    .trim_end_matches(']')
                    .trim()
                    .trim_matches('"')
                    .to_owned(),
            )
        });
        match explicit {
            Some(p) => out.push(dir.join(p)),
            None => {
                out.push(base.join(module.trim()).join("mod.rs"));
                out.push(base.join(format!("{}.rs", module.trim())));
            }
        }
    }
    out
}

/// The text from `from` in `code` to the end of its statement: the next
/// `;`, or the end of the text.
fn statement(code: &str, from: usize) -> &str {
    let rest = &code[from..];
    let end = rest.find(';').unwrap_or(rest.len());
    &rest[..end]
}

/// The arguments of the call whose `(` is the last byte of `needle` at
/// `at`, or the rest of the statement.
fn arguments<'a>(code: &'a str, at: usize, needle: &str) -> &'a str {
    let open = at + needle.len() - 1;
    match closing(code, open) {
        Some(close) => &code[open..=close],
        None => statement(code, at),
    }
}

/// The fixed waits of `text` inside `regions`.
fn findings(text: &str, regions: &[(usize, usize)]) -> Vec<Finding> {
    let code = blank(text);
    let starts = line_starts(&code);
    let raw: Vec<&str> = text.lines().collect();
    let live = |line: usize| regions.iter().any(|&(a, b)| a <= line && line <= b);
    let mut found: BTreeMap<(usize, &'static str), ()> = BTreeMap::new();
    let mut note = |offset: usize, rule: &'static str| {
        let line = line_of(&starts, offset);
        if live(line) {
            found.insert((line, rule), ());
        }
    };
    for (at, _) in code.match_indices("0..") {
        let counted = code[at + 3..].starts_with(|c: char| c.is_ascii_digit());
        let before = code[..at].trim_end();
        let header = before.ends_with("for _ in") || before.ends_with('(');
        if !counted || !header {
            continue;
        }
        // The loop's body opens before its statement ends; a range with
        // no block of its own (`(0..3).map(f);`) has none.
        let Some(open) = code[at..].find('{').map(|k| at + k) else {
            continue;
        };
        if code[at..open].contains(';') {
            continue;
        }
        if let Some(close) = closing(&code, open)
            && code[open..close].contains("sleep(")
        {
            note(at, "counted-poll");
        }
    }
    for (at, _) in code.match_indices("Instant::now() +") {
        if !statement(&code, at).contains("LIVE_DEADLINE") {
            note(at, "fixed-deadline");
        }
    }
    for needle in ["recv_timeout(", "time::timeout(", ".line_with("] {
        for (at, _) in code.match_indices(needle) {
            if arguments(&code, at, needle).contains("Duration::from_") {
                note(at, "fixed-timeout");
            }
        }
    }
    for needle in ["poll(", "poll_async(", "within_of("] {
        for (at, _) in code.match_indices(needle) {
            let before = &code[..at];
            let joined = before.ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
            if joined || before.trim_end().ends_with("fn") {
                continue;
            }
            if !arguments(&code, at, needle).contains("LIVE_DEADLINE") {
                note(at, "fixed-helper");
            }
        }
    }
    for needle in ["\"--max-time\"", "Command::new(\"curl\")"] {
        for (at, _) in code.match_indices(needle) {
            note(at, "curl-bound");
        }
    }
    for (at, _) in code.match_indices("const ") {
        // A constant's name is upper case; `const fn` is a function.
        let named = code[at + 6..].starts_with(|c: char| c.is_ascii_uppercase());
        let word =
            at == 0 || !code[..at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
        if !named || !word {
            continue;
        }
        let stmt = statement(&code, at);
        let typed = stmt.split('=').next().unwrap_or("").contains("Duration");
        if typed && !stmt.contains("LIVE_DEADLINE") {
            note(at, "fixed-const");
        }
    }
    for (at, _) in code.match_indices("elapsed() <") {
        // A watch is a loop's condition; an assertion on elapsed time
        // bounds nothing it waits for.
        let line_start = code[..at].rfind('\n').map_or(0, |k| k + 1);
        let looped = code[line_start..at].contains("while ");
        if looped && statement(&code, at).contains("Duration::from_") {
            note(at, "fixed-watch");
        }
    }
    found
        .into_keys()
        .map(|(line, rule)| Finding {
            line: line + 1,
            rule,
            text: raw.get(line).copied().unwrap_or("").to_owned(),
        })
        .collect()
}

/// The definitions of a name `LIVE_DEADLINE` in `text`, string contents
/// and comments aside.
fn shadows(text: &str) -> Vec<Finding> {
    let code = blank_with(text, true);
    let starts = line_starts(&code);
    let raw: Vec<&str> = text.lines().collect();
    let mut lines = std::collections::BTreeSet::new();
    for keyword in ["const ", "static ", "let ", "let mut "] {
        for (at, _) in code.match_indices(&format!("{keyword}LIVE_DEADLINE")) {
            let after = code[at + keyword.len() + "LIVE_DEADLINE".len()..]
                .chars()
                .next();
            let whole = !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            let word =
                at == 0 || !code[..at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
            if whole && word {
                lines.insert(line_of(&starts, at));
            }
        }
    }
    lines
        .into_iter()
        .map(|line| Finding {
            line: line + 1,
            rule: "shadowed-deadline",
            text: raw.get(line).copied().unwrap_or("").to_owned(),
        })
        .collect()
}

/// Whether the gate judges `path`: every file but the one that writes the
/// rule.
fn judged(path: &Path) -> bool {
    !path.ends_with("tests/support/live_deadline.rs")
}

/// The allow list's entries, or why a line of it is refused.
fn allowed(list: &str) -> Result<Vec<Allow>, String> {
    let mut out = Vec::new();
    for (n, line) in list.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = t.splitn(3, '|').map(str::trim).collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
            return Err(format!(
                "line {}: write path | fragment | reason, none of them empty: {t}",
                n + 1
            ));
        }
        out.push(Allow {
            path: parts[0].to_owned(),
            fragment: parts[1].to_owned(),
            reason: parts[2].to_owned(),
        });
    }
    Ok(out)
}

/// The findings no entry allows, and the entries that allow nothing.
fn judge(
    found: &BTreeMap<String, Vec<Finding>>,
    allows: &[Allow],
) -> (Vec<(String, Finding)>, Vec<Allow>) {
    let allows_it =
        |a: &Allow, path: &str, f: &Finding| a.path == path && f.text.contains(&a.fragment);
    let unallowed = found
        .iter()
        .flat_map(|(path, fs)| fs.iter().map(move |f| (path.clone(), f.clone())))
        .filter(|(path, f)| !allows.iter().any(|a| allows_it(a, path, f)))
        .collect();
    let stale = allows
        .iter()
        .filter(|a| {
            !found
                .iter()
                .any(|(path, fs)| fs.iter().any(|f| allows_it(a, path, f)))
        })
        .cloned()
        .collect();
    (unallowed, stale)
}

/// The package's root.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every .rs file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir)
        .expect("reads the directory")
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The findings of the package's tree, by path relative to the package.
fn tree() -> BTreeMap<String, Vec<Finding>> {
    let base = root();
    let mut files = Vec::new();
    rust_files(&base.join("src"), &mut files);
    rust_files(&base.join("tests"), &mut files);
    let texts: BTreeMap<PathBuf, String> = files
        .iter()
        .map(|p| (p.clone(), std::fs::read_to_string(p).expect("reads")))
        .collect();
    let mut whole: Vec<PathBuf> = Vec::new();
    for (path, text) in &texts {
        whole.extend(live_files(path, text));
    }
    let mut found = BTreeMap::new();
    for (path, text) in texts.iter().filter(|(p, _)| judged(p)) {
        let regions = if whole.contains(path) {
            vec![(0, text.lines().count().saturating_sub(1))]
        } else {
            live_regions(text)
        };
        let mut fs = findings(text, &regions);
        fs.extend(shadows(text));
        if !fs.is_empty() {
            let relative = path.strip_prefix(&base).expect("in the package");
            found.insert(relative.display().to_string(), fs);
        }
    }
    found
}

/// The allow list of the package.
fn the_list() -> Vec<Allow> {
    let path = root().join("tests/support/live_waits_allowed.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    allowed(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A live module around `body`.
fn live(body: &str) -> String {
    format!("#[cfg(feature = \"backend-tests\")]\nmod on_a_server {{\n{body}\n}}\n")
}

/// The rules `findings` gives for `text`, its live regions found.
fn rules(text: &str) -> Vec<&'static str> {
    findings(text, &live_regions(text))
        .into_iter()
        .map(|f| f.rule)
        .collect()
}

mod success {
    use super::*;

    #[test]
    fn the_tree_has_no_fixed_live_wait_the_list_does_not_allow() {
        let (unallowed, _) = judge(&tree(), &the_list());
        assert!(
            unallowed.is_empty(),
            "fixed live waits; bound each by live_deadline.rs or list it with its reason:\n{}",
            unallowed
                .iter()
                .map(|(p, f)| format!("{p}:{}: {}: {}", f.line, f.rule, f.text.trim()))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn every_entry_of_the_list_allows_a_wait_that_is_there() {
        let (_, stale) = judge(&tree(), &the_list());
        assert!(stale.is_empty(), "entries that match nothing: {stale:?}");
    }

    #[test]
    fn a_poll_bounded_by_the_live_deadline_passes() {
        let text = live(
            "fn f() { live_deadline::poll(LIVE_DEADLINE, Duration::from_millis(50), || true); }",
        );
        assert!(
            rules(&text).is_empty(),
            "a poll on the deadline: {:?}",
            rules(&text)
        );
    }

    #[test]
    fn a_deadline_from_the_live_deadline_passes() {
        let text = live("fn f() { let deadline = Instant::now() + LIVE_DEADLINE * 2; }");
        assert!(
            rules(&text).is_empty(),
            "a multiple of the deadline: {:?}",
            rules(&text)
        );
    }

    #[test]
    fn a_channel_wait_for_the_time_left_passes() {
        let text =
            live("fn f() { rx.recv_timeout(deadline.saturating_duration_since(Instant::now())); }");
        assert!(rules(&text).is_empty(), "the time left: {:?}", rules(&text));
    }

    #[test]
    fn a_curl_from_the_shared_helper_passes() {
        let text = live("fn f() { live_deadline::curl(LIVE_DEADLINE).args([\"-sS\"]); }");
        assert!(
            rules(&text).is_empty(),
            "the shared curl: {:?}",
            rules(&text)
        );
    }

    #[test]
    fn a_counted_loop_that_does_not_sleep_passes() {
        let text = live("fn f() { for _ in 0..3 { make(); } }");
        assert!(
            rules(&text).is_empty(),
            "three repetitions: {:?}",
            rules(&text)
        );
    }

    #[test]
    fn a_const_on_the_live_deadline_passes() {
        let text = live("const PATIENCE: Duration = LIVE_DEADLINE;");
        assert!(
            rules(&text).is_empty(),
            "named from the deadline: {:?}",
            rules(&text)
        );
    }

    #[test]
    fn a_fixed_wait_outside_live_code_is_not_this_gates() {
        let text =
            "fn f() { for _ in 0..50 { std::thread::sleep(Duration::from_millis(100)); } }\n";
        assert!(
            rules(text).is_empty(),
            "code without backend-tests: {:?}",
            rules(text)
        );
    }

    #[test]
    fn a_gated_test_function_is_a_live_region() {
        let text = "fn a() {}\n#[cfg(feature = \"backend-tests\")]\n#[test]\nfn b() {\n    x();\n}\nfn c() {}\n";
        assert_eq!(
            live_regions(text),
            vec![(1, 5)],
            "the attribute to the closing brace"
        );
    }

    #[test]
    fn a_gated_module_with_all_is_a_live_region() {
        let text =
            "#[cfg(all(feature = \"backend-tests\", feature = \"postgres\"))]\nmod live {\n}\n";
        assert_eq!(live_regions(text), vec![(0, 2)], "cfg(all(...)) counts");
    }

    #[test]
    fn a_gated_mod_declaration_makes_its_file_live() {
        let text = "#[cfg(all(feature = \"backend-tests\", feature = \"postgres\"))]\nmod pg;\n";
        assert_eq!(
            live_files(Path::new("/c/tests/serve.rs"), text),
            vec![
                PathBuf::from("/c/tests/pg/mod.rs"),
                PathBuf::from("/c/tests/pg.rs")
            ],
            "either file the declaration can name"
        );
    }

    #[test]
    fn the_list_is_read_with_its_reasons() {
        let list = "# why\ntests/serve.rs | elapsed() < Duration::from_secs(3) | the holder is watched for 3 s\n";
        assert_eq!(
            allowed(list),
            Ok(vec![Allow {
                path: "tests/serve.rs".to_owned(),
                fragment: "elapsed() < Duration::from_secs(3)".to_owned(),
                reason: "the holder is watched for 3 s".to_owned(),
            }]),
            "path, fragment, reason"
        );
    }

    #[test]
    fn an_allowed_finding_is_not_refused() {
        let found = BTreeMap::from([(
            "tests/serve.rs".to_owned(),
            vec![Finding {
                line: 9,
                rule: "fixed-watch",
                text: "while watched.elapsed() < Duration::from_secs(3) {".to_owned(),
            }],
        )]);
        let list = allowed("tests/serve.rs | elapsed() < Duration::from_secs(3) | a watch\n")
            .expect("read");
        assert_eq!(
            judge(&found, &list),
            (vec![], vec![]),
            "allowed, and the entry is used"
        );
    }
}

mod failure {
    use super::*;

    #[test]
    fn a_counted_poll_that_sleeps_is_refused() {
        let text = live(
            "fn f() {\n for _ in 0..50 {\n if done() { return; }\n std::thread::sleep(Duration::from_millis(100));\n }\n}",
        );
        assert_eq!(
            rules(&text),
            vec!["counted-poll"],
            "fifty pauses are not a deadline"
        );
    }

    #[test]
    fn a_counted_any_that_sleeps_is_refused() {
        let text = live(
            "fn f() { let ok = (0..60).any(|_| { std::thread::sleep(Duration::from_millis(500)); done() }); }",
        );
        assert_eq!(
            rules(&text),
            vec!["counted-poll"],
            "(0..60).any is a count too"
        );
    }

    #[test]
    fn an_async_counted_poll_is_refused() {
        let text = live(
            "async fn f() {\n for _ in 0..50 {\n tokio::time::sleep(Duration::from_millis(100)).await;\n }\n}",
        );
        assert_eq!(
            rules(&text),
            vec!["counted-poll"],
            "tokio's sleep counts too"
        );
    }

    #[test]
    fn a_fixed_deadline_is_refused() {
        let text = live("fn f() { let deadline = Instant::now() + Duration::from_secs(20); }");
        assert_eq!(rules(&text), vec!["fixed-deadline"], "20 s of its own");
    }

    #[test]
    fn a_deadline_from_a_fixed_const_is_refused() {
        let text = live("fn f() { let deadline = std::time::Instant::now() + PATIENCE * 6; }");
        assert_eq!(
            rules(&text),
            vec!["fixed-deadline"],
            "a name that is not the deadline"
        );
    }

    #[test]
    fn a_fixed_channel_wait_is_refused() {
        let text = live("fn f() { rx.recv_timeout(Duration::from_secs(30)).expect(\"x\"); }");
        assert_eq!(rules(&text), vec!["fixed-timeout"], "30 s of its own");
    }

    #[test]
    fn a_fixed_tokio_timeout_is_refused() {
        let text = live("async fn f() { tokio::time::timeout(Duration::from_secs(20), x).await; }");
        assert_eq!(rules(&text), vec!["fixed-timeout"], "20 s of its own");
    }

    #[test]
    fn a_fixed_line_wait_is_refused() {
        let text = live("fn f() { s.line_with(\"serving\", Duration::from_secs(20)); }");
        assert_eq!(rules(&text), vec!["fixed-timeout"], "20 s of its own");
    }

    #[test]
    fn a_written_out_max_time_is_refused() {
        let text = live("fn f() { cmd.args([\"-sS\", \"--max-time\", \"20\"]); }");
        assert_eq!(rules(&text), vec!["curl-bound"], "20 s of its own");
    }

    #[test]
    fn a_curl_of_its_own_is_refused() {
        let text = live("fn f() { Command::new(\"curl\").args([\"-sS\"]).output(); }");
        assert_eq!(rules(&text), vec!["curl-bound"], "a curl with no bound");
    }

    #[test]
    fn a_fixed_const_is_refused() {
        let text =
            live("const PATIENCE: std::time::Duration = std::time::Duration::from_secs(20);");
        assert_eq!(rules(&text), vec!["fixed-const"], "a fixed name for a wait");
    }

    #[test]
    fn a_wait_helper_given_a_fixed_deadline_is_refused() {
        let text = live("fn f() { within_of(&c, \"SELECT 1\", \"1\", Duration::from_secs(5)); }");
        assert_eq!(
            rules(&text),
            vec!["fixed-helper"],
            "the helper's bound is the deadline"
        );
    }

    #[test]
    fn a_poll_given_a_fixed_deadline_is_refused() {
        let text =
            live("fn f() { poll(Duration::from_secs(5), Duration::from_millis(50), || true); }");
        assert_eq!(rules(&text), vec!["fixed-helper"], "5 s of its own");
    }

    #[test]
    fn a_fixed_watch_is_refused() {
        let text = live("fn f() { while watched.elapsed() < Duration::from_secs(3) { look(); } }");
        assert_eq!(rules(&text), vec!["fixed-watch"], "3 s of its own");
    }

    #[test]
    fn a_local_live_deadline_is_refused() {
        let text = "mod t {\n    const LIVE_DEADLINE: Duration = Duration::from_secs(20);\n}\n";
        let found: Vec<_> = shadows(text)
            .into_iter()
            .map(|f| (f.line, f.rule))
            .collect();
        assert_eq!(
            found,
            vec![(2, "shadowed-deadline")],
            "a fixed value under the shared name"
        );
    }

    #[test]
    fn a_local_live_deadline_binding_is_refused() {
        let text = "fn f() { let LIVE_DEADLINE = Duration::from_secs(5); }\n";
        assert_eq!(
            shadows(text).len(),
            1,
            "a binding hides the shared value as well"
        );
    }

    #[test]
    fn a_line_of_the_list_without_a_reason_is_refused() {
        let refused = allowed("tests/serve.rs | elapsed() <\n").expect_err("no reason");
        assert!(refused.contains("line 1"), "names the line: {refused}");
    }

    #[test]
    fn a_line_of_the_list_with_an_empty_reason_is_refused() {
        let refused = allowed("tests/serve.rs | elapsed() < |  \n").expect_err("empty reason");
        assert!(refused.contains("line 1"), "names the line: {refused}");
    }

    #[test]
    fn a_finding_no_entry_allows_is_refused() {
        let found = BTreeMap::from([(
            "tests/serve.rs".to_owned(),
            vec![Finding {
                line: 9,
                rule: "fixed-deadline",
                text: "let deadline = Instant::now() + Duration::from_secs(20);".to_owned(),
            }],
        )]);
        let list =
            allowed("tests/rotation.rs | Duration::from_secs(20) | other file\n").expect("read");
        let (unallowed, stale) = judge(&found, &list);
        assert_eq!(
            unallowed.len(),
            1,
            "the entry is for another file: {unallowed:?}"
        );
        assert_eq!(stale.len(), 1, "and it matches nothing: {stale:?}");
    }

    #[test]
    fn an_entry_that_matches_nothing_is_stale() {
        let list = allowed("tests/serve.rs | gone() | it was removed\n").expect("read");
        let (_, stale) = judge(&BTreeMap::new(), &list);
        assert_eq!(stale, list, "the entry is reported");
    }
}

mod edge {
    use super::*;

    #[test]
    fn a_brace_inside_a_string_does_not_end_a_region() {
        let text = live("fn f() { let s = \"}\"; for _ in 0..9 { sleep(x); } }");
        assert_eq!(
            rules(&text),
            vec!["counted-poll"],
            "the string's brace is not counted"
        );
    }

    #[test]
    fn a_brace_inside_a_char_does_not_end_a_region() {
        let text = live("fn f() { let c = '}'; for _ in 0..9 { sleep(x); } }");
        assert_eq!(
            rules(&text),
            vec!["counted-poll"],
            "the char's brace is not counted"
        );
    }

    #[test]
    fn a_lifetime_is_not_a_char() {
        let text = live("fn f<'a>(x: &'a str) { for _ in 0..9 { sleep(x); } }");
        assert_eq!(rules(&text), vec!["counted-poll"], "'a is a lifetime");
    }

    #[test]
    fn a_lifetime_right_before_a_string_leaves_the_string_a_string() {
        // Read as a character literal, `'a"` would swallow the string's
        // opening quote and leave its brace to be counted.
        let blanked = blank("x::<'a\"{\">");
        assert!(
            !blanked.contains('{'),
            "the string's brace is blanked: {blanked}"
        );
    }

    #[test]
    fn a_wait_in_a_comment_is_not_code() {
        let text = live("// let deadline = Instant::now() + Duration::from_secs(20);\nfn f() {}");
        assert!(rules(&text).is_empty(), "a comment: {:?}", rules(&text));
    }

    #[test]
    fn a_wait_in_a_block_comment_is_not_code() {
        let text = live("/* Command::new(\"curl\") */ fn f() {}");
        assert!(
            rules(&text).is_empty(),
            "a block comment: {:?}",
            rules(&text)
        );
    }

    #[test]
    fn a_raw_string_holding_a_quote_and_a_brace_is_skipped() {
        let text = live("fn f() { let s = r#\"a \" } b\"#; for _ in 0..9 { sleep(x); } }");
        assert_eq!(
            rules(&text),
            vec!["counted-poll"],
            "the raw string is one literal"
        );
    }

    #[test]
    fn blanking_keeps_lines_and_columns() {
        let text = "a(\"{\") // }\nb";
        let blanked = blank(text);
        assert_eq!(blanked.len(), text.len(), "the same length");
        assert_eq!(blanked.lines().count(), 2, "the same lines");
        assert!(
            !blanked.contains('{') && !blanked.contains('}'),
            "no brace left: {blanked}"
        );
    }

    #[test]
    fn a_not_backend_tests_cfg_is_not_live() {
        let text = "#[cfg(not(feature = \"backend-tests\"))]\nmod off {\n}\n";
        assert!(
            live_regions(text).is_empty(),
            "cfg(not(...)) is the other build"
        );
    }

    #[test]
    fn a_doc_comment_between_the_cfg_and_the_item_is_skipped() {
        let text = "#[cfg(feature = \"backend-tests\")]\n/// Docs.\nmod s3 {\n}\n";
        assert_eq!(
            live_regions(text),
            vec![(0, 3)],
            "the doc line is part of the item"
        );
    }

    #[test]
    fn a_finding_names_its_one_based_line() {
        let text = live("fn f() {\n let deadline = Instant::now() + Duration::from_secs(20);\n}");
        let found = findings(&text, &live_regions(&text));
        assert_eq!(found.len(), 1, "one: {found:?}");
        assert_eq!(found[0].line, 4, "the fourth line of the text");
    }

    #[test]
    fn a_nested_counted_poll_is_one_finding() {
        let text = live("fn f() { for _ in 0..9 { for _ in 0..3 { make(); } sleep(x); } }");
        assert_eq!(
            rules(&text),
            vec!["counted-poll"],
            "the outer loop sleeps; the inner does not"
        );
    }

    #[test]
    fn an_assertion_on_elapsed_time_is_not_a_watch() {
        let text = live("fn f() { assert!(started.elapsed() < Duration::from_secs(5), \"x\"); }");
        assert!(
            rules(&text).is_empty(),
            "a bound asserted, not waited: {:?}",
            rules(&text)
        );
    }

    #[test]
    fn the_shared_deadline_file_is_not_judged() {
        assert!(
            !judged(Path::new("/c/tests/support/live_deadline.rs")),
            "the one place the rule is written"
        );
        assert!(
            judged(Path::new("/c/tests/serve.rs")),
            "every other file is"
        );
    }

    #[test]
    fn a_use_of_the_shared_deadline_is_not_a_shadow() {
        let text = "use crate::live_deadline::LIVE_DEADLINE;\nfn f() { g(LIVE_DEADLINE); }\n";
        assert!(
            shadows(text).is_empty(),
            "an import and a use: {:?}",
            shadows(text)
        );
    }

    #[test]
    fn a_shadow_written_in_a_string_is_not_code() {
        let text = "fn f() { let s = \"const LIVE_DEADLINE: Duration = x;\"; }\n";
        assert!(
            shadows(text).is_empty(),
            "text in a string: {:?}",
            shadows(text)
        );
    }

    #[test]
    fn comment_and_blank_lines_in_the_list_are_skipped() {
        assert_eq!(
            allowed("# a comment\n\n   \n"),
            Ok(vec![]),
            "nothing to read"
        );
    }

    #[test]
    fn the_list_matches_by_fragment_within_the_line() {
        let found = BTreeMap::from([(
            "tests/serve.rs".to_owned(),
            vec![Finding {
                line: 9,
                rule: "fixed-timeout",
                text: "        .recv_timeout(Duration::from_millis(400)),".to_owned(),
            }],
        )]);
        let list = allowed("tests/serve.rs | recv_timeout(Duration::from_millis(400)) | absent\n")
            .expect("read");
        assert_eq!(
            judge(&found, &list).0,
            vec![],
            "the fragment is inside the line"
        );
    }
}
