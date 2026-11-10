//! The documentation is built with rustdoc's warnings denied, by the commit
//! hook and by CI, with one command. A link that does not resolve, or one
//! from public documentation to a private item, fails both. Until this
//! gate, nothing built the documentation that way, and two such links in
//! `mcp/fence.rs` and four in `tui.rs` were on main.

use std::path::Path;

/// The command, as both places run it.
const GATE: &str = "RUSTDOCFLAGS=\"-D warnings\" cargo doc --workspace --no-deps --locked --quiet";

fn read(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The lines of `text` that run a command: trimmed, without comments and
/// without a workflow step's `run:` key.
fn commands(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .map(|line| line.strip_prefix("run: ").unwrap_or(line))
        .collect()
}

#[test]
fn the_commit_hook_builds_the_documentation_with_warnings_denied() {
    let hook = read(".githooks/pre-commit");
    assert!(commands(&hook).contains(&GATE), "the hook runs `{GATE}`");
}

#[test]
fn ci_builds_the_documentation_with_warnings_denied() {
    let ci = read(".github/workflows/ci.yml");
    assert!(commands(&ci).contains(&GATE), "CI runs `{GATE}`");
}

#[test]
fn the_readme_names_the_documentation_build_among_the_hooks_checks() {
    let readme = read("README.md").replace('\n', " ");
    assert!(
        readme.contains("`cargo doc --workspace --no-deps --locked --quiet`"),
        "the README's description of the hook names the command"
    );
}

#[test]
fn a_command_in_a_comment_is_not_run() {
    let text = format!("# {GATE}\n  run: echo\n");
    assert!(
        !commands(&text).contains(&GATE),
        "a comment that names the gate does not satisfy it"
    );
    assert_eq!(
        commands(&text)[0],
        "echo",
        "a step's run key is not part of its command"
    );
}

#[test]
fn the_fence_modules_public_documentation_links_to_no_private_item() {
    let source = read("crates/vcon-server/src/mcp/fence.rs");
    let linked: Vec<&str> = source
        .lines()
        .filter(|line| line.trim_start().starts_with("///") && line.contains("[`ESCAPED`]"))
        .collect();
    assert_eq!(
        linked,
        Vec::<&str>::new(),
        "ESCAPED is private: public documentation names it without a link"
    );
}
