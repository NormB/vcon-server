//! The site's status statements name this build's version, so a version
//! bump cannot leave them describing an older release.

use std::path::PathBuf;

fn page(path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/content")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_status_page_names_this_release() {
    let version = env!("CARGO_PKG_VERSION");
    let text = page("docs/status.md");
    let says = format!("vcon-server {version} is released");
    assert!(text.contains(&says), "docs/status.md does not say: {says}");
}

#[test]
fn the_home_page_status_note_names_this_release() {
    let version = env!("CARGO_PKG_VERSION");
    let text = page("_index.md");
    let note = text
        .lines()
        .find(|l| l.starts_with("> **Status:"))
        .expect("a status note");
    let says = format!("> **Status: {version} released");
    assert!(note.starts_with(&says), "the note is: {note}");
}
