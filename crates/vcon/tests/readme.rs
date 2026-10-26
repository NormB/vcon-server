//! The README is the crate's page on crates.io and docs.rs, and its example
//! runs as a doc test. A line rustdoc hides (one starting with `# `) would
//! show on crates.io, which renders the README as plain markdown, so the
//! example has none.

use std::path::Path;

#[test]
fn the_readme_example_hides_no_lines() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md");
    let readme = std::fs::read_to_string(&path).expect("read the README");
    let mut in_rust = false;
    let mut hidden = Vec::new();
    for line in readme.lines() {
        match line.trim_start() {
            "```rust" => in_rust = true,
            "```" => in_rust = false,
            text if in_rust && (text == "#" || text.starts_with("# ")) => {
                hidden.push(line.to_string())
            }
            _ => {}
        }
    }
    assert!(
        hidden.is_empty(),
        "hidden lines in the README example: {hidden:?}"
    );
}
