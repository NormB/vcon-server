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
