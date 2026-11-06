//! The site's library page shows the same examples the crate runs as doc
//! tests: the README's reading example, and the writing example of the
//! `vcon::write` module's documentation, so each example on the page runs
//! too.

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

/// The first example of a module's `//!` documentation: the lines of its
/// first fenced block, without the `//! ` before each.
fn module_example(source: &str) -> String {
    let mut example = String::new();
    let mut open = false;
    for line in source.lines() {
        let Some(doc) = line.strip_prefix("//!") else {
            break;
        };
        let doc = doc.strip_prefix(' ').unwrap_or(doc);
        if doc.starts_with("```") {
            if open {
                return example;
            }
            open = true;
            continue;
        }
        if open {
            example.push_str(doc);
            example.push('\n');
        }
    }
    panic!("the module documentation has no example")
}

#[test]
fn the_library_page_examples_are_the_doc_tests() {
    let readme = rust_blocks(&read("README.md"));
    let page = rust_blocks(&read("../../website/content/docs/library.md"));
    let write = module_example(&read("src/write/mod.rs"));
    assert_eq!(readme.len(), 1, "the README has one Rust example");
    assert_eq!(
        page,
        [readme[0].clone(), write],
        "the site shows the README's example, then vcon::write's, as their doc tests run them"
    );
}

#[test]
fn module_example_reads_the_first_block() {
    let source =
        "//! Text.\n//!\n//! ```\n//! let a = 1;\n//!\n//! ```\n//! ```\n//! no\n//! ```\nuse x;\n";
    assert_eq!(
        module_example(source),
        "let a = 1;\n\n",
        "the first block only"
    );
}

#[test]
fn rust_blocks_reads_each_block() {
    let text = "a\n```rust\nlet x = 1;\n```\nb\n```sh\nls\n```\n```rust\nlet y = 2;\n```\n";
    assert_eq!(
        rust_blocks(text),
        ["let x = 1;\n", "let y = 2;\n"],
        "only rust blocks are read, in order, and the sh block between them is skipped"
    );
}
