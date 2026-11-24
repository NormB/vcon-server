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

/// Whether `path` is a test module of its own: named `<name>_tests.rs`
/// and declared by `parent` only under `#[cfg(test)]`, so no build but a
/// test's compiles it.
fn test_module(parent: &str, path: &Path) -> bool {
    let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    name.ends_with("_tests") && parent.contains(&format!("#[cfg(test)]\nmod {name};"))
}

#[test]
fn a_file_is_a_test_module_only_when_its_parent_declares_it_so() {
    let path = Path::new("src/store/group_tests.rs");
    assert!(
        test_module("mod a;\n#[cfg(test)]\nmod group_tests;\n", path),
        "a file declared under #[cfg(test)] is a test module"
    );
    assert!(
        !test_module("mod a;\nmod group_tests;\n", path),
        "a file declared without #[cfg(test)] is built into the server"
    );
    assert!(
        !test_module(
            "#[cfg(test)]\nmod group;\n",
            Path::new("src/store/group.rs")
        ),
        "a file whose name does not end in _tests is not one"
    );
}

#[test]
fn only_the_local_backend_speaks_sqlite_for_the_store() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let parent = std::fs::read_to_string(src.join("store.rs")).expect("reads store.rs");
    let mut store = vec![src.join("store.rs")];
    store.extend(
        sources(&src.join("store"))
            .into_iter()
            .filter(|path| !test_module(&parent, path)),
    );
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
    let parent = std::fs::read_to_string(src.join("tenants.rs")).expect("reads tenants.rs");
    let mut registry = vec![src.join("tenants.rs")];
    registry.extend(
        sources(&src.join("tenants"))
            .into_iter()
            .filter(|path| !test_module(&parent, path)),
    );
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
