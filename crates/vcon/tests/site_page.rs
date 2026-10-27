//! The site's library page shows the same example as the crate's README,
//! which runs as a doc test, so the example on the page runs too.

use std::path::Path;

/// The text of each ```rust block in `markdown`, in order.
fn rust_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        match &mut current {
            None if line.trim_start() == "```rust" => current = Some(String::new()),
            Some(_) if line.trim_start() == "```" => {
                blocks.push(current.take().expect("open block"))
            }
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
            None => {}
        }
    }
    blocks
}

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn the_library_page_example_is_the_readme_example() {
    let readme = rust_blocks(&read("README.md"));
    let page = rust_blocks(&read("../../website/content/docs/library.md"));
    assert_eq!(readme.len(), 1, "the README has one Rust example");
    assert_eq!(page, readme);
}

#[test]
fn rust_blocks_reads_each_block() {
    let text = "a\n```rust\nlet x = 1;\n```\nb\n```sh\nls\n```\n```rust\nlet y = 2;\n```\n";
    assert_eq!(rust_blocks(text), ["let x = 1;\n", "let y = 2;\n"]);
}
