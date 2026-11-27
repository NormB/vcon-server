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

/// The production files that open a store or a registry as a command
/// beside a server does (`Durability::Beside`: COMMIT syncs, no group
/// sync), by name, sorted.
fn beside_openers(src: &Path) -> Vec<String> {
    let mut found: Vec<String> = sources(src)
        .into_iter()
        .map(|path| path.strip_prefix(src).expect("under src").to_path_buf())
        .filter(|rel| !test_file(rel))
        .filter(|rel| {
            let text = std::fs::read_to_string(src.join(rel)).expect("reads a source file");
            calls_beside(production(&text))
        })
        .map(|rel| rel.display().to_string())
        .collect();
    found.sort();
    found
}

/// Whether `rel`, a path under src, is a test file: `<name>_tests.rs`, or
/// a file in a `tests` directory (backend/postgres/tests).
fn test_file(rel: &Path) -> bool {
    let stem = rel.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    stem.ends_with("_tests")
        || rel
            .parent()
            .is_some_and(|dir| dir.components().any(|c| c.as_os_str() == "tests"))
}

/// Whether `code` opens a store or a registry beside a server.
fn calls_beside(code: &str) -> bool {
    ["Store::open_beside(", "Tenants::open_beside"]
        .iter()
        .any(|call| code.contains(call))
}

#[test]
fn a_file_named_for_its_tests_is_a_test_file() {
    assert!(
        test_file(Path::new("store/sync_tests.rs")),
        "<name>_tests.rs is skipped"
    );
}

#[test]
fn a_file_in_a_tests_directory_is_a_test_file() {
    assert!(
        test_file(Path::new("backend/postgres/tests/read_tests.rs")),
        "a file under a tests directory is skipped"
    );
    assert!(
        test_file(Path::new("backend/postgres/tests/schema.rs")),
        "whatever its name"
    );
}

#[test]
fn a_module_file_named_tests_is_still_read() {
    assert!(
        !test_file(Path::new("backend/postgres/tests.rs")),
        "the file that declares a tests directory is not in it"
    );
}

#[test]
fn a_name_that_only_contains_tests_is_still_read() {
    assert!(
        !test_file(Path::new("contests.rs")),
        "only a name ending in _tests is a test file"
    );
    assert!(
        !test_file(Path::new("tests_old/a.rs")),
        "only a directory named tests"
    );
}

#[test]
fn a_production_file_is_read() {
    assert!(!test_file(Path::new("store.rs")), "store.rs is read");
    assert!(
        !test_file(Path::new("backend/local.rs")),
        "backend/local.rs is read"
    );
}

#[test]
fn a_store_opened_beside_is_a_beside_call() {
    assert!(
        calls_beside("let s = Store::open_beside(dir, id, key, fs, clock)?;"),
        "Store::open_beside"
    );
}

#[test]
fn a_registry_opened_beside_is_a_beside_call() {
    assert!(
        calls_beside("Tenants::open_beside(dir, master, fs, clock)"),
        "Tenants::open_beside"
    );
    assert!(
        calls_beside("let open = Tenants::open_beside_in;"),
        "Tenants::open_beside_in, as a function value too"
    );
}

#[test]
fn a_serving_open_is_not_a_beside_call() {
    assert!(
        !calls_beside("Store::open_as(dir, id, key, fs, clock, durability)?"),
        "open_as with the rule's durability"
    );
    assert!(
        !calls_beside("Tenants::open_in(dir, master, fs, clock, objects, catalogs)"),
        "the server's registry opener"
    );
}

#[test]
fn a_beside_call_in_a_test_module_is_not_counted() {
    let text =
        "fn serve() {}\n\nmod tests {\n    fn t() { Store::open_beside(d, i, k, f, c); }\n}\n";
    assert!(
        !calls_beside(production(text)),
        "the part before `mod tests` is the production code"
    );
}

#[test]
fn the_check_finds_a_serving_file_that_opens_beside() {
    let dir = tempfile::tempdir().expect("a directory");
    let src = dir.path();
    std::fs::create_dir_all(src.join("backend/postgres/tests")).expect("dirs");
    std::fs::write(
        src.join("serving.rs"),
        "fn open() { Store::open_beside(d, i, k, f, c); }\n",
    )
    .expect("write");
    std::fs::write(src.join("serving_tests.rs"), "Store::open_beside(\n").expect("write");
    std::fs::write(
        src.join("backend/postgres/tests/live.rs"),
        "Tenants::open_beside(\n",
    )
    .expect("write");
    std::fs::write(src.join("quiet.rs"), "fn open() { Store::open_as(); }\n").expect("write");
    assert_eq!(
        beside_openers(src),
        ["serving.rs"],
        "the production file that opens beside, and no test file"
    );
}

#[test]
fn the_check_finds_a_serving_file_that_opens_a_registry_beside() {
    let dir = tempfile::tempdir().expect("a directory");
    std::fs::create_dir_all(dir.path().join("serve")).expect("dirs");
    std::fs::write(
        dir.path().join("serve/start.rs"),
        "fn start() { let open = Tenants::open_beside_in; }\n",
    )
    .expect("write");
    assert_eq!(
        beside_openers(dir.path()),
        ["serve/start.rs"],
        "a registry opened beside, by its path under src"
    );
}

#[test]
fn the_check_finds_nothing_in_a_tree_with_no_beside_open() {
    let dir = tempfile::tempdir().expect("a directory");
    std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("write");
    assert_eq!(
        beside_openers(dir.path()),
        Vec::<String>::new(),
        "nothing opens beside"
    );
}

/// Only a command that runs beside a server opens beside one. A serving
/// path that did (backend/local.rs in release 0.4.18, for every tenant on
/// local objects) runs without the group sync of its commits.
#[test]
fn only_a_command_beside_a_server_opens_a_catalog_beside_one() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert_eq!(
        beside_openers(&src),
        ["commands.rs"],
        "a serving path opens its catalogs for group sync, not beside a server"
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
