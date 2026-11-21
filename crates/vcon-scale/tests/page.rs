//! The scale page (website/content/docs/scale.md) shows exactly the result
//! files committed in website/static/scale/, in file-name order, and links
//! each one.

use std::path::{Path, PathBuf};

use serde_json::Value;
use vcon_scale::report;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn results() -> Vec<(String, Value)> {
    let dir = root().join("website/static/scale");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let text = std::fs::read_to_string(dir.join(&name))
                .unwrap_or_else(|e| panic!("read {name}: {e}"));
            let json = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, json)
        })
        .collect()
}

fn page() -> String {
    let path = root().join("website/content/docs/scale.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn the_page_table_is_the_committed_results() {
    let found = results();
    assert!(
        !found.is_empty(),
        "website/static/scale holds at least one result"
    );
    let jsons: Vec<Value> = found.iter().map(|(_, json)| json.clone()).collect();
    let table = report::page_table(&jsons).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        page().contains(&table),
        "scale.md must hold this table, made from the result files:\n{table}"
    );
}

#[test]
fn the_page_links_every_result_file() {
    let page = page();
    for (name, _) in results() {
        let link = format!("(https://vcon-server.com/scale/{name})");
        assert!(page.contains(&link), "scale.md links {name} as {link}");
    }
}

#[test]
fn each_result_file_is_named_for_its_label() {
    for (name, json) in results() {
        let label = json["label"].as_str().unwrap_or_default();
        assert_eq!(
            name,
            format!("{label}.json"),
            "the file is named for the run"
        );
    }
}
