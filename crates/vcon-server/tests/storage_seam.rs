//! The storage rules live in Store and Tenants, written once; the SQL and
//! the object files they need live behind the backend traits. Nothing
//! outside the local backend, the catalog and the audit log's rendering
//! speaks SQLite.

use std::path::{Path, PathBuf};

/// Every Rust source file under `dir`.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).expect("reads the source directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            found.extend(sources(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
    found
}

/// The part of a source file before its test modules.
fn production(text: &str) -> &str {
    let end = [
        "\nmod tests",
        "\npub(crate) mod tests",
        "\nmod legal_tests",
        "\nmod rekey_tests",
        "\nmod examine_tests",
    ]
    .iter()
    .filter_map(|m| text.find(m))
    .min()
    .unwrap_or(text.len());
    &text[..end]
}

#[test]
fn only_the_local_backend_speaks_sqlite_for_the_store() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut store = vec![src.join("store.rs")];
    store.extend(sources(&src.join("store")));
    let speaking: Vec<String> = store
        .iter()
        .filter(|path| {
            let text = std::fs::read_to_string(path).expect("reads a source file");
            let code = production(&text);
            [
                "rusqlite",
                "query_row",
                ".execute(",
                ".prepare(",
                "self.objects.join",
            ]
            .iter()
            .any(|word| code.contains(word))
        })
        .map(|path| {
            path.strip_prefix(&src)
                .expect("under src")
                .display()
                .to_string()
        })
        .collect();
    assert!(
        speaking.is_empty(),
        "SQL or object files outside the backend: {speaking:?}"
    );
}

#[test]
fn only_the_local_backend_speaks_sqlite_for_the_registry() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut registry = vec![src.join("tenants.rs")];
    registry.extend(sources(&src.join("tenants")));
    let speaking: Vec<String> = registry
        .iter()
        .filter(|path| {
            let text = std::fs::read_to_string(path).expect("reads a source file");
            let code = production(&text);
            [
                "rusqlite",
                "query_row",
                ".execute(",
                ".prepare(",
                "catalog::",
                "self.tenants.join",
            ]
            .iter()
            .any(|word| code.contains(word))
        })
        .map(|path| {
            path.strip_prefix(&src)
                .expect("under src")
                .display()
                .to_string()
        })
        .collect();
    assert!(
        speaking.is_empty(),
        "SQL or tenant directories outside the backend: {speaking:?}"
    );
}
