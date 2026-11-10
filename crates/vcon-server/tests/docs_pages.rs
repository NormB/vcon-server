//! Facts on documentation pages that a test can read from the code.
//!
//! The S3 page's AWS configuration and policies cannot be run in CI (no
//! AWS account), so they are checked here as text: the TOML is a storage
//! table the server accepts, the IAM policy allows exactly the actions the
//! page asks for on the bucket that table names and nothing else, MinIO's
//! policy is the same policy on its own bucket, and the AWS bucket name is
//! one AWS lets no one make.
//!
//! The audit-log-records page lists every action the code records, and only
//! those.
//! The HTTP API page has a row for every route the API's router serves, and
//! only those, and `openapi.json` describes each of them.
//!
//! The tenants-and-keys page names every route the HTTP API page gives to
//! any valid key.
//!
//! The Development page's how-to on adding a GitHub Action names the check
//! that enforces the pin and the two forms it accepts. It is checked here,
//! not in `scripts/test_check_action_pins.py`, because the hook tests run
//! that script in a repository with no website.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::Value;
use vcon_server::config::{self, ObjectStore, S3Settings};

/// What the server needs on its bucket, as `backend/s3.rs` calls it.
const ACTIONS: [&str; 5] = [
    "s3:DeleteObject",
    "s3:GetBucketVersioning",
    "s3:GetObject",
    "s3:ListBucket",
    "s3:PutObject",
];

/// The actions on the bucket itself; the rest are on its objects.
const BUCKET_ACTIONS: [&str; 2] = ["s3:GetBucketVersioning", "s3:ListBucket"];

/// The prefix AWS reserves for its documentation: no bucket may start
/// with it.
const RESERVED: &str = "amzn-s3-demo-";

/// The keys every configuration needs, before a storage table.
const LEAST: &str = r#"listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"
"#;

/// A file of the repository, by its path from the root.
fn read(path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn page() -> String {
    read("website/content/docs/s3-storage.md")
}

/// The text under the `## {heading}` line, up to the next `## ` heading.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let start = format!("\n## {heading}\n");
    let at = text
        .find(&start)
        .unwrap_or_else(|| panic!("the page has no section {heading:?}"));
    let body = &text[at + start.len()..];
    body.find("\n## ").map_or(body, |end| &body[..end])
}

/// The fenced blocks of `text` in `lang`, each with the line before its
/// fence (where a docs-test marker sits).
fn blocks(text: &str, lang: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut lines = text.lines().peekable();
    let mut before = "";
    while let Some(line) = lines.next() {
        if line.trim_start() == format!("```{lang}") {
            let mut code = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start() == "```" {
                    break;
                }
                code.push_str(inner);
                code.push('\n');
            }
            found.push((before.to_owned(), code));
        }
        before = line;
    }
    found
}

/// The S3 settings of a page's storage table, read as the server reads it:
/// a whole configuration as it is, a storage table after `LEAST`.
fn settings(table: &str) -> S3Settings {
    let whole = table.lines().any(|line| line.starts_with("listen ="));
    let text = if whole {
        table.to_owned()
    } else {
        format!("{LEAST}\n{table}")
    };
    let config = config::parse(&text)
        .unwrap_or_else(|e| panic!("the server refuses the page's table: {e:?}\n{table}"));
    match config.storage.objects {
        ObjectStore::S3(s3) => s3,
        ObjectStore::Local => panic!("the page's table keeps the objects local"),
    }
}

/// What a policy allows: each (action, resource) pair, and whether any
/// statement does anything other than allow.
fn grants(policy: &Value) -> (BTreeSet<(String, String)>, bool) {
    let mut grants = BTreeSet::new();
    let mut other = false;
    let as_list = |v: &Value| -> Vec<String> {
        match v {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => a
                .iter()
                .map(|x| x.as_str().expect("a string").to_owned())
                .collect(),
            _ => panic!("not a string or a list: {v}"),
        }
    };
    assert_eq!(
        policy["Version"], "2012-10-17",
        "the IAM policy language version"
    );
    for statement in policy["Statement"].as_array().expect("statements") {
        other |= statement["Effect"] != "Allow";
        let keys: BTreeSet<&str> = statement
            .as_object()
            .expect("a statement")
            .keys()
            .map(String::as_str)
            .collect();
        other |= keys != BTreeSet::from(["Action", "Effect", "Resource"]);
        for action in as_list(&statement["Action"]) {
            for resource in as_list(&statement["Resource"]) {
                grants.insert((action.clone(), resource));
            }
        }
    }
    (grants, other)
}

/// Exactly the grants the server needs on `bucket`.
fn needed(bucket: &str) -> BTreeSet<(String, String)> {
    ACTIONS
        .iter()
        .map(|action| {
            let resource = if BUCKET_ACTIONS.contains(action) {
                format!("arn:aws:s3:::{bucket}")
            } else {
                format!("arn:aws:s3:::{bucket}/*")
            };
            ((*action).to_owned(), resource)
        })
        .collect()
}

fn policy_in(code: &str) -> Value {
    serde_json::from_str(code).unwrap_or_else(|e| panic!("the policy is not JSON: {e}\n{code}"))
}

#[test]
fn the_aws_table_is_one_the_server_takes_for_aws() {
    let text = page();
    let aws = section(&text, "AWS S3");
    let tables = blocks(aws, "toml");
    assert_eq!(tables.len(), 1, "one storage table in the AWS section");
    let s3 = settings(&tables[0].1);
    assert_eq!(s3.endpoint, None, "AWS S3 is the region's own endpoint");
    assert!(!s3.path_style, "AWS S3 names the bucket in the host name");
    assert_eq!(
        s3.access_key_id, None,
        "the AWS table takes its keys from the credential chain"
    );
    assert!(
        s3.bucket.starts_with(RESERVED),
        "the AWS bucket {:?} could be someone's real bucket; use a name starting {RESERVED}",
        s3.bucket
    );
}

#[test]
fn the_aws_policy_allows_what_the_server_needs_on_its_bucket_alone() {
    let text = page();
    let aws = section(&text, "AWS S3");
    let bucket = settings(&blocks(aws, "toml")[0].1).bucket;
    let policies = blocks(aws, "json");
    assert_eq!(policies.len(), 1, "one IAM policy in the AWS section");
    let (grants, other) = grants(&policy_in(&policies[0].1));
    assert!(!other, "every statement only allows, with no condition");
    assert_eq!(grants, needed(&bucket), "the policy's grants on {bucket}");
}

#[test]
fn the_minio_policy_is_the_same_on_the_bucket_the_server_uses() {
    let text = page();
    let minio = section(&text, "Set up MinIO (on the storage host)");
    let saved: Vec<_> = blocks(minio, "json")
        .into_iter()
        .filter(|(marker, _)| marker.contains("file vcon-server-policy.json"))
        .collect();
    assert_eq!(saved.len(), 1, "MinIO's policy is saved as a file");
    let table = section(&text, "Add the storage table");
    let bucket = settings(&blocks(table, "toml")[0].1).bucket;
    let (grants, other) = grants(&policy_in(&saved[0].1));
    assert!(!other, "every statement only allows, with no condition");
    assert_eq!(grants, needed(&bucket), "MinIO's grants on {bucket}");
}

#[test]
fn the_page_asks_for_the_actions_the_policies_grant() {
    let text = page();
    let before = section(&text, "Before you start");
    for action in ACTIONS {
        assert!(
            before.contains(&format!("`{action}`")),
            "before you start does not name {action}"
        );
    }
}

#[test]
fn the_readers_find_sections_blocks_and_grants() {
    let text = "\n## A\n\n<!-- m -->\n```json\n{}\n```\n\n## B\n```json\n[]\n```\n";
    assert_eq!(
        blocks(section(text, "A"), "json"),
        [("<!-- m -->".to_owned(), "{}\n".to_owned())],
        "a block and the line before it, in its own section only"
    );
    let deny = policy_in(
        r#"{"Version":"2012-10-17","Statement":[{"Effect":"Deny","Action":"s3:GetObject","Resource":"arn:aws:s3:::b/*"}]}"#,
    );
    assert!(grants(&deny).1, "a Deny is not only an allow");
    let condition = policy_in(
        r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":"s3:GetObject","Resource":"*","Condition":{}}]}"#,
    );
    assert!(grants(&condition).1, "a condition is noticed");
    assert_eq!(needed("b").len(), 5, "five grants");
}

// The audit-log-records page lists every action the code records, and only
// those.

/// The names `Action::as_str` writes in the log, read from its source: each
/// `Action::<Name> => "<name>"` arm.
fn actions_in_code(source: &str) -> BTreeSet<String> {
    let body = source
        .split("pub fn as_str(self) -> &'static str {")
        .nth(1)
        .expect("Action::as_str in audit.rs");
    let body = &body[..body.find("\n    }\n").expect("the end of as_str")];
    body.lines()
        .filter_map(|line| line.trim().strip_prefix("Action::"))
        .filter_map(|arm| arm.split_once("=> \""))
        .map(|(_, rest)| rest.trim_end_matches("\",").to_owned())
        .collect()
}

/// The actions the page's table names: the first cell of each row of the
/// "Every action" section, in backticks.
fn actions_on_page(page: &str) -> BTreeSet<String> {
    section(page, "Every action")
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once('`'))
        .map(|(name, _)| name.to_owned())
        .collect()
}

#[test]
fn the_audit_page_lists_every_action_the_code_records() {
    let code = actions_in_code(&read("crates/vcon-server/src/audit.rs"));
    assert!(code.len() >= 28, "the reader found the actions: {code:?}");
    let page = actions_on_page(&read("website/content/docs/audit-log-records.md"));
    let missing: Vec<_> = code.difference(&page).collect();
    let extra: Vec<_> = page.difference(&code).collect();
    assert!(
        missing.is_empty(),
        "audit-log-records.md does not list {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "audit-log-records.md lists {extra:?}, which the code never records"
    );
}

#[test]
fn the_action_readers_read_arms_and_rows() {
    let source = "impl Action {\n    pub fn as_str(self) -> &'static str {\n        match self {\n            Action::Create => \"create\",\n            Action::KeyList => \"key_list\",\n        }\n    }\n}\nAction::Other => \"no\",\n";
    assert_eq!(
        actions_in_code(source),
        BTreeSet::from(["create".to_owned(), "key_list".to_owned()]),
        "the arms of as_str only"
    );
    let page = "\n## Every action\n\n| Action | Records |\n|---|---|\n| `create` | x |\n\n## Next\n| `no` | y |\n";
    assert_eq!(
        actions_on_page(page),
        BTreeSet::from(["create".to_owned()]),
        "the rows of the section only"
    );
}

// Every route of the API's router is a row of the HTTP API page, and every
// row is a route; `openapi.json` describes each.

/// The `(METHOD, path)` pairs `api::router` routes, read from its source:
/// each `.route("<path>", ...)` call before the first `.merge(`, with each
/// method function named in its arguments. The console and MCP routers are
/// merged after, and have pages of their own.
fn routes_in_code(source: &str) -> BTreeSet<(String, String)> {
    let body = source
        .split("pub fn router(app: Arc<App>) -> Router {")
        .nth(1)
        .expect("api::router in api.rs");
    let body = &body[..body.find(".merge(").expect("the merges after the routes")];
    let mut routes = BTreeSet::new();
    for call in body.split(".route(").skip(1) {
        let path = call
            .split('"')
            .nth(1)
            .expect("a route's path, in quotes")
            .to_owned();
        let after = &call[call.find(',').expect("the path, then the methods")..];
        for method in ["get", "post", "put", "patch", "delete"] {
            let named = after.match_indices(&format!("{method}(")).any(|(at, _)| {
                at == 0 || !after[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_')
            });
            if named {
                routes.insert((method.to_ascii_uppercase(), path.clone()));
            }
        }
    }
    routes
}

/// The `(METHOD, path)` pairs the HTTP API page's tables have a row for:
/// each row whose first cell is `` `METHOD /path` ``.
fn routes_on_page(page: &str) -> BTreeSet<(String, String)> {
    page.lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once('`'))
        .filter_map(|(cell, _)| cell.split_once(' '))
        .filter(|(method, path)| {
            method.chars().all(|c| c.is_ascii_uppercase()) && path.starts_with('/')
        })
        .map(|(method, path)| (method.to_owned(), path.to_owned()))
        .collect()
}

/// The `(METHOD, path)` pairs `openapi.json` describes.
fn routes_described(description: &Value) -> BTreeSet<(String, String)> {
    let mut routes = BTreeSet::new();
    for (path, item) in description["paths"].as_object().expect("paths") {
        for method in ["get", "post", "put", "patch", "delete"] {
            if item.get(method).is_some() {
                routes.insert((method.to_ascii_uppercase(), path.clone()));
            }
        }
    }
    routes
}

#[test]
fn every_route_is_on_the_http_api_page_and_every_row_is_a_route() {
    let code = routes_in_code(&read("crates/vcon-server/src/api.rs"));
    assert!(code.len() >= 21, "the reader found the routes: {code:?}");
    let page = routes_on_page(&read("website/content/docs/http-api.md"));
    let missing: Vec<_> = code.difference(&page).collect();
    let extra: Vec<_> = page.difference(&code).collect();
    assert!(missing.is_empty(), "http-api.md has no row for {missing:?}");
    assert!(
        extra.is_empty(),
        "http-api.md has rows for {extra:?}, which no route serves"
    );
    let described = routes_described(&policy_in(&read("crates/vcon-server/openapi.json")));
    let undescribed: Vec<_> = code.difference(&described).collect();
    assert!(
        undescribed.is_empty(),
        "openapi.json does not describe {undescribed:?}"
    );
    let others: Vec<_> = described
        .difference(&code)
        .filter(|(method, path)| {
            !(path == "/console"
                || path.starts_with("/console/")
                || (method == "POST" && path == "/mcp"))
        })
        .collect();
    assert!(
        others.is_empty(),
        "openapi.json describes {others:?}, which no route serves"
    );
}

#[test]
fn the_route_readers_read_calls_and_rows() {
    let source = "pub fn router(app: Arc<App>) -> Router {\n    Router::new()\n        .route(\"/a\", post(create).get(list))\n        .route(\n            \"/a/{id}\",\n            patch(change).delete(remove),\n        )\n        .route(\"/b\", axum::routing::delete(revoke))\n        .route(\"/c\", get(forget_me))\n        .merge(other())\n        .route(\"/d\", get(x))\n}\n";
    let pair = |m: &str, p: &str| (m.to_owned(), p.to_owned());
    assert_eq!(
        routes_in_code(source),
        BTreeSet::from([
            pair("DELETE", "/a/{id}"),
            pair("DELETE", "/b"),
            pair("GET", "/a"),
            pair("GET", "/c"),
            pair("PATCH", "/a/{id}"),
            pair("POST", "/a"),
        ]),
        "every method of every route before the merge, and a handler's name is not a method"
    );
    let page = "| `GET /a` | x |\n| `limit` | y |\n| `GET` | z |\n| `vcon-server check` | w |\n";
    assert_eq!(
        routes_on_page(page),
        BTreeSet::from([pair("GET", "/a")]),
        "route rows only"
    );
}

#[test]
fn the_development_page_says_how_to_add_an_action() {
    let page = read("website/content/docs/development.md");
    let how_to = section(&page, "Adding a GitHub Action");
    for says in [
        "`scripts/check_action_pins.py`",
        "owner/repo@<40-hex commit> # v<version>",
        "./.github/actions/",
    ] {
        assert!(
            how_to.contains(says),
            "the how-to on adding an action says {says}"
        );
    }
}

/// The routes the HTTP API page's tables give to any valid key: the first
/// cell of each row whose second cell is `any valid key`.
fn routes_for_any_key(page: &str) -> Vec<String> {
    page.lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once("` | "))
        .filter(|(_, rest)| rest.starts_with("any valid key |"))
        .map(|(route, _)| route.to_owned())
        .collect()
}

#[test]
fn the_any_key_reader_takes_the_rows_with_that_scope() {
    let page = "| Route | Scope | Answers |\n|---|---|---|\n| `GET /key` | any valid key | x |\n\
                | `GET /vcons` | `read` | y |\n| `GET /health` | any valid key | z |\n";
    assert_eq!(
        routes_for_any_key(page),
        ["GET /key", "GET /health"],
        "the rows whose scope is any valid key, in order"
    );
    assert!(
        routes_for_any_key("| `GET /vcons` | `read` | any valid key may not |\n").is_empty(),
        "the words in another cell do not count"
    );
}

/// The tenants-and-keys page says what each scope allows and then what any
/// valid key may ask. Found on 0.4.2: it named `GET /health` and
/// `GET /openapi.json` and left out `GET /key`, added in 0.3.0.
#[test]
fn the_keys_page_names_every_route_any_valid_key_may_ask() {
    let routes = routes_for_any_key(&read("website/content/docs/http-api.md"));
    assert!(
        routes.len() >= 3,
        "the reader found the routes for any valid key: {routes:?}"
    );
    let keys_page = read("website/content/docs/tenants-and-keys.md");
    let sentence = keys_page
        .split("\n\n")
        .find(|paragraph| paragraph.starts_with("Any valid key may ask"))
        .expect("the tenants-and-keys page says what any valid key may ask");
    let missing: Vec<&String> = routes
        .iter()
        .filter(|route| !sentence.contains(&format!("`{route}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "the tenants-and-keys page does not say any valid key may ask {missing:?}: {sentence}"
    );
}

/// The source files the Development page's layout tree names under
/// `crates/vcon-server/src/`: each `name.rs` on a line of the tree's `src/`
/// part.
fn sources_in_tree(page: &str) -> BTreeSet<String> {
    let tree = page
        .split("```text\n")
        .nth(1)
        .and_then(|rest| rest.split("\n```").next())
        .expect("the layout tree, the page's first text block");
    let src = tree
        .split_once("    └── src/\n")
        .expect("the tree's src/ part")
        .1;
    let mut names = BTreeSet::new();
    for line in src.lines().take_while(|line| line.starts_with("        ")) {
        let entry = line.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
        let listed = entry.split("  ").next().unwrap_or_default();
        for name in listed.split(", ") {
            if let Some(file) = name.trim().split('/').next().filter(|f| f.ends_with(".rs")) {
                names.insert(file.to_owned());
            }
        }
    }
    names
}

#[test]
fn the_tree_reader_takes_each_file_of_the_src_part() {
    let page = "Intro.\n\n```text\ncrates/\n└── vcon-server/\n    ├── console/   files\n    └── src/\n        ├── main.rs        the command line\n        ├── fs.rs, clock.rs  behind interfaces\n        └── mcp.rs         the server; mcp/stdio.rs, the command\nfuzz/               targets\n```\n";
    assert_eq!(
        sources_in_tree(page),
        BTreeSet::from(["clock.rs", "fs.rs", "main.rs", "mcp.rs"].map(str::to_owned)),
        "each file a line lists, and nothing after the src/ part"
    );
}

/// The Development page's layout tree is where a new contributor looks for
/// a module. Found on 0.4.2: it left out seven of the 25 files, among them
/// `commands.rs`, `backend.rs` and `tui.rs`.
#[test]
fn the_development_page_tree_names_every_source_file() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let files: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".rs"))
        .collect();
    assert!(
        files.contains("main.rs"),
        "the walk reached the sources: {files:?}"
    );
    let named = sources_in_tree(&read("website/content/docs/development.md"));
    let missing: Vec<&String> = files.difference(&named).collect();
    assert!(
        missing.is_empty(),
        "the layout tree does not name {missing:?}"
    );
    let gone: Vec<&String> = named.difference(&files).collect();
    assert!(
        gone.is_empty(),
        "the layout tree names files that are not there: {gone:?}"
    );
}
