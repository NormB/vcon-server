//! The rule of src/stderr.rs, held over the source of every crate: nothing
//! writes to stderr in a way that panics when the write fails. That is,
//! no `eprintln!` or `eprint!` (they panic on a closed stderr; use
//! `vcon_server::stderr::line`); no `io::stderr` but in src/stderr.rs, so
//! every write goes through its writer; and each `tracing_subscriber::fmt()`
//! builder turns off its fallback for a failed write
//! (`.log_internal_errors(false)`), which is an `eprintln!`.

use std::path::{Path, PathBuf};

/// What breaks the rule in the Rust source `text` of `name`: one finding
/// per line, with its number.
fn findings(name: &str, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or_default();
        for banned in ["eprintln!", "eprint!"] {
            if code.contains(banned) {
                out.push(format!("{name}:{}: {banned}", i + 1));
            }
        }
        if code.contains("io::stderr") && !name.ends_with("crates/vcon-server/src/stderr.rs") {
            out.push(format!(
                "{name}:{}: io::stderr outside src/stderr.rs",
                i + 1
            ));
        }
        if code.contains("tracing_subscriber::fmt()") {
            let builder: String = lines[i..]
                .iter()
                .take_while(|l| !l.contains(';'))
                .chain(lines[i..].iter().find(|l| l.contains(';')))
                .copied()
                .collect();
            if !builder.contains(".log_internal_errors(false)") {
                out.push(format!(
                    "{name}:{}: tracing_subscriber::fmt() without .log_internal_errors(false)",
                    i + 1
                ));
            }
        }
    }
    out
}

/// Every `.rs` file under each crate's `src/`.
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    // Canonical, so each name reads crates/<crate>/src/... with no `..`.
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("the crates directory");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&crates).expect("the crates") {
        let src = entry.expect("an entry").path().join("src");
        if src.is_dir() {
            walk(&src, &mut out);
        }
    }
    out.sort();
    out
}

// Success.

#[test]
fn no_source_file_breaks_the_rule() {
    let files = sources();
    assert!(files.len() > 50, "every crate's source: {}", files.len());
    let found: Vec<String> = files
        .iter()
        .flat_map(|p| {
            findings(
                &p.display().to_string(),
                &std::fs::read_to_string(p).expect("read"),
            )
        })
        .collect();
    assert!(found.is_empty(), "panicking stderr writes: {found:#?}");
}

#[test]
fn the_server_log_is_found_and_keeps_the_rule() {
    let main = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
        .expect("main.rs");
    assert!(
        main.contains("tracing_subscriber::fmt()"),
        "the scan has the builder to check"
    );
    assert!(
        findings("main.rs", &main).is_empty(),
        "and it keeps the rule"
    );
}

#[test]
fn the_scan_reads_the_source_of_every_crate() {
    let files = sources();
    for krate in ["crates/vcon/src/", "crates/vcon-server/src/"] {
        assert!(
            files
                .iter()
                .any(|p| p.to_str().is_some_and(|s| s.contains(krate))),
            "{krate} is scanned"
        );
    }
}

#[test]
fn a_quiet_line_is_not_a_finding() {
    assert!(
        findings("a.rs", "vcon_server::stderr::line(\"x\");\n").is_empty(),
        "the rule's own writer"
    );
}

#[test]
fn a_builder_with_the_fallback_off_is_not_a_finding() {
    let text = "tracing_subscriber::fmt()\n    .with_writer(w)\n    .log_internal_errors(false)\n    .init();\n";
    assert!(findings("a.rs", text).is_empty(), "the fallback is off");
}

// Failure.

#[test]
fn eprintln_is_found_with_its_line() {
    assert_eq!(
        findings("a.rs", "fn f() {\n    eprintln!(\"x\");\n}\n"),
        ["a.rs:2: eprintln!"],
        "named by line"
    );
}

#[test]
fn eprint_is_found() {
    assert_eq!(
        findings("a.rs", "eprint!(\"x\");\n"),
        ["a.rs:1: eprint!"],
        "eprint! too"
    );
}

#[test]
fn a_builder_with_the_fallback_on_is_found() {
    let text = "tracing_subscriber::fmt()\n    .with_writer(capture)\n    .init();\n";
    assert_eq!(
        findings("a.rs", text),
        ["a.rs:1: tracing_subscriber::fmt() without .log_internal_errors(false)"],
        "the default fallback is eprintln!"
    );
}

#[test]
fn a_builder_that_turns_the_fallback_on_is_found() {
    let text = "tracing_subscriber::fmt().log_internal_errors(true).init();\n";
    assert_eq!(findings("a.rs", text).len(), 1, "true is the default");
}

// Edge.

#[test]
fn eprintln_in_a_comment_is_not_a_finding() {
    assert!(
        findings(
            "a.rs",
            "// eprintln! panics on a closed stderr\nlet x = 1; // not eprint!\n"
        )
        .is_empty(),
        "a comment names it"
    );
}

#[test]
fn the_fallback_off_after_the_builders_end_does_not_count() {
    let text = "tracing_subscriber::fmt().init();\nother.log_internal_errors(false);\n";
    assert_eq!(
        findings("a.rs", text).len(),
        1,
        "the next statement is not the builder"
    );
}

#[test]
fn a_one_line_builder_with_the_fallback_off_is_not_a_finding() {
    let text = "tracing_subscriber::fmt().log_internal_errors(false).init();\n";
    assert!(findings("a.rs", text).is_empty(), "one line");
}

#[test]
fn stderr_named_outside_the_rules_module_is_found() {
    assert_eq!(
        findings("src/main.rs", "    .with_writer(std::io::stderr)\n"),
        ["src/main.rs:1: io::stderr outside src/stderr.rs"],
        "only the rule's module opens stderr"
    );
}

#[test]
fn a_locked_stderr_is_found() {
    assert_eq!(
        findings("src/x.rs", "let mut e = std::io::stderr().lock();\n").len(),
        1,
        "the lock is a way around the rule's writer"
    );
}

#[test]
fn a_builder_that_turns_the_fallback_on_over_lines_is_found() {
    let text = "tracing_subscriber::fmt()\n    .with_writer(w)\n    .log_internal_errors(true)\n    .finish();\n";
    assert_eq!(findings("a.rs", text).len(), 1, "true, on its own line");
}

#[test]
fn a_file_named_stderr_rs_in_another_crate_is_not_exempt() {
    assert_eq!(
        findings("crates/vcon/src/stderr.rs", "    io::stderr()\n"),
        ["crates/vcon/src/stderr.rs:1: io::stderr outside src/stderr.rs"],
        "only vcon-server's own module"
    );
}

#[test]
fn each_forbidden_form_is_found_in_the_vcon_crate() {
    for text in [
        "eprintln!(\"x\");\n",
        "eprint!(\"x\");\n",
        "let e = std::io::stderr();\n",
        "tracing_subscriber::fmt().init();\n",
    ] {
        assert_eq!(
            findings("crates/vcon/src/lib.rs", text).len(),
            1,
            "{text:?} in the vcon crate"
        );
    }
}

#[test]
fn the_rules_module_may_name_stderr() {
    assert!(
        findings(
            "crates/vcon-server/src/stderr.rs",
            "    Quiet(io::stderr())\n"
        )
        .is_empty(),
        "its own writer"
    );
}

#[test]
fn two_findings_on_two_lines_are_both_named() {
    assert_eq!(
        findings("a.rs", "eprintln!(\"a\");\neprint!(\"b\");\n").len(),
        2,
        "each"
    );
}

// The one `eprintln!` that `.log_internal_errors(false)` does not reach:
// tracing-subscriber 0.3.23's fmt layer prints "Unable to format the
// following event" with `eprintln!` when a NEW SPAN's fields fail to
// format (fmt_layer.rs `on_new_span`), and that panics on a closed stderr.
// A field fails only through a `Debug` or `Display` impl that returns
// `fmt::Error` itself (the buffer is a `String`). So: no source here opens a
// span or names `fmt::Error` outside an `impl ... Write for` block, the
// server's log keeps tracing-subscriber's default level (INFO), and the
// crates of Cargo.lock that use `tracing` are the ones whose spans at INFO
// or above were read (2026-11-20): only rmcp 3.9.0 opens any, one with no
// fields (`#[instrument(skip_all)]`) and one whose one field is a `&str`.

/// Each span the Rust source `text` of `name` opens, and each `fmt::Error`
/// it names outside an `impl ... Write for` block: one finding per line.
fn span_findings(name: &str, text: &str) -> Vec<String> {
    const OPENERS: [&str; 11] = [
        "span!(",
        "trace_span!(",
        "debug_span!(",
        "info_span!(",
        "warn_span!(",
        "error_span!(",
        "#[instrument",
        "#[tracing::instrument",
        "Span::new(",
        "Span::new_root(",
        "Span::child_of(",
    ];
    let mut out = Vec::new();
    // The brace depth at which an `impl ... Write for` body is open.
    let mut sink: Option<usize> = None;
    let mut depth = 0usize;
    for (i, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or_default();
        // A string is not code: drop each "..." part.
        let code: String = code.split('"').step_by(2).collect();
        if OPENERS.iter().any(|o| {
            code.match_indices(o).any(|(at, _)| {
                // `span!(` inside `info_span!(` is that one opener.
                let before = code[..at].chars().next_back();
                !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
            })
        }) {
            out.push(format!("{name}:{}: a span is opened", i + 1));
        }
        if sink.is_none() && code.contains("impl") && code.contains("Write for") {
            sink = Some(depth);
        }
        if code.contains("fmt::Error") && sink.is_none() {
            out.push(format!(
                "{name}:{}: fmt::Error outside an impl ... Write for block",
                i + 1
            ));
        }
        for c in code.chars() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if sink == Some(depth) {
                        sink = None;
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The packages of the Cargo.lock `text` that depend on `tracing`, sorted.
fn tracing_users(text: &str) -> Vec<String> {
    let mut users = Vec::new();
    for package in text.split("[[package]]") {
        let name = package
            .lines()
            .find_map(|l| l.strip_prefix("name = \""))
            .and_then(|l| l.strip_suffix('"'));
        let uses = package.lines().any(|l| {
            let dep = l.trim().trim_end_matches(',').trim_matches('"');
            dep == "tracing" || dep.starts_with("tracing ")
        });
        if let (Some(name), true) = (name, uses) {
            users.push(name.to_owned());
        }
    }
    users.sort();
    users.dedup();
    users
}

/// The packages whose spans were read, each with what was found.
const READ_TRACING_USERS: [(&str, &str); 18] = [
    ("aws-config", "no span at INFO or above"),
    ("aws-runtime", "no span at INFO or above"),
    ("aws-sdk-s3", "no span at INFO or above"),
    ("aws-sdk-sts", "no span at INFO or above"),
    ("aws-sigv4", "no span at INFO or above"),
    ("aws-smithy-checksums", "no span at INFO or above"),
    ("aws-smithy-http", "no span at INFO or above"),
    ("aws-smithy-http-client", "no span at INFO or above"),
    ("aws-smithy-runtime", "no span at INFO or above"),
    ("aws-smithy-runtime-api", "no span at INFO or above"),
    ("aws-types", "no span at INFO or above"),
    ("h2", "no span at INFO or above"),
    (
        "hyper-util",
        "INFO spans only in its tests and doc examples",
    ),
    ("quinn", "INFO and ERROR spans only in its tests"),
    ("quinn-proto", "INFO spans only in its tests"),
    ("quinn-udp", "no span at INFO or above"),
    ("rmcp", "two INFO spans: no fields, and one `&str` field"),
    ("vcon-server", "no span (span_findings)"),
];

fn workspace_file(rel: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel),
    )
    .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

// Success.

#[test]
fn no_source_file_opens_a_span_or_names_fmt_error() {
    let found: Vec<String> = sources()
        .iter()
        .flat_map(|p| {
            span_findings(
                &p.display().to_string(),
                &std::fs::read_to_string(p).expect("read"),
            )
        })
        .collect();
    assert!(found.is_empty(), "spans or fmt::Error: {found:#?}");
}

#[test]
fn the_crates_that_use_tracing_are_the_ones_read() {
    let read: Vec<String> = READ_TRACING_USERS
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    assert_eq!(
        tracing_users(&workspace_file("Cargo.lock")),
        read,
        "a crate that now uses tracing: read its spans at INFO or above, then list it"
    );
}

#[test]
fn the_server_log_keeps_the_default_level() {
    let main = workspace_file("crates/vcon-server/src/main.rs");
    for setter in ["with_max_level", "with_env_filter", "with_filter"] {
        assert!(
            !main.contains(setter),
            "{setter}: a lower level lets DEBUG and TRACE spans of every crate reach the eprintln!"
        );
    }
}

#[test]
fn a_write_sink_may_name_fmt_error() {
    let text = "impl fmt::Write for Sink {\n    fn write_str(&mut self, s: &str) -> fmt::Result {\n        Err(fmt::Error)\n    }\n}\n";
    assert!(span_findings("a.rs", text).is_empty(), "a sink's own error");
}

// Failure.

#[test]
fn each_way_to_open_a_span_is_found() {
    for text in [
        "let s = info_span!(\"x\");\n",
        "let s = tracing::debug_span!(\"x\");\n",
        "let s = span!(Level::INFO, \"x\");\n",
        "let s = trace_span!(\"x\");\n",
        "let s = warn_span!(\"x\");\n",
        "let s = error_span!(\"x\");\n",
        "#[instrument]\nfn f() {}\n",
        "#[tracing::instrument(skip(a))]\nfn f() {}\n",
        "let s = Span::new(meta, &values);\n",
        "let s = Span::new_root(meta, &values);\n",
        "let s = Span::child_of(p, meta, &values);\n",
    ] {
        assert_eq!(span_findings("a.rs", text).len(), 1, "{text:?}");
    }
}

#[test]
fn fmt_error_in_a_display_impl_is_found_with_its_line() {
    let text = "impl fmt::Display for X {\n    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {\n        Err(fmt::Error)\n    }\n}\n";
    assert_eq!(
        span_findings("a.rs", text),
        ["a.rs:3: fmt::Error outside an impl ... Write for block"],
        "a field of that type would fail to format"
    );
}

#[test]
fn a_new_crate_that_uses_tracing_is_listed() {
    let lock = "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\ndependencies = [\n \"b\",\n \"tracing\",\n]\n\n[[package]]\nname = \"c\"\nversion = \"1.0.0\"\ndependencies = [\n \"tracing 0.1.44\",\n]\n\n[[package]]\nname = \"d\"\nversion = \"1.0.0\"\n";
    assert_eq!(tracing_users(lock), ["a", "c"], "with or without a version");
}

// Edge.

#[test]
fn a_span_in_a_comment_or_a_method_named_span_is_not_found() {
    let text = "// info_span!(x) would open one\nlet s = req.span();\nlet t = x.in_current_span();\nlet n = \"debug_span!(x)\";\nlet m = time_span!(x);\n";
    assert!(span_findings("a.rs", text).is_empty(), "no span is opened");
}

#[test]
fn fmt_error_after_a_write_sink_closes_is_found() {
    let text = "impl fmt::Write for Sink {\n    fn write_str(&mut self, _: &str) -> fmt::Result { Ok(()) }\n}\nfn f() -> fmt::Result { Err(fmt::Error) }\n";
    assert_eq!(
        span_findings("a.rs", text).len(),
        1,
        "the sink ended at line 3"
    );
}

#[test]
fn tracing_subscriber_and_tracing_core_are_not_users() {
    let lock = "[[package]]\nname = \"a\"\nversion = \"1.0.0\"\ndependencies = [\n \"tracing-core\",\n \"tracing-subscriber\",\n]\n";
    assert!(
        tracing_users(lock).is_empty(),
        "only the crate `tracing` itself"
    );
}

#[test]
fn every_read_crate_has_a_reason() {
    for (name, why) in READ_TRACING_USERS {
        assert!(!why.trim().is_empty(), "{name} has no reason");
    }
}
