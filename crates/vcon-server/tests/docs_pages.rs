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

#[path = "support/audit_page.rs"]
mod audit_page;

/// The actions the page's table names: the first cell of each row of the
/// "Every action" section, in backticks (support/audit_page.rs, which
/// tests/audit_contract.rs reads the page with too).
fn actions_on_page(page: &str) -> BTreeSet<String> {
    audit_page::actions(page)
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

/// The HTTP API page's masked view section.
fn masked_section() -> String {
    let page = read("website/content/docs/http-api.md");
    let start = page
        .find("\n## Masked view\n")
        .expect("a masked view section");
    let rest = &page[start + 1..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |at| at + 3);
    rest[..end].to_owned()
}

// The HTTP API page's example of the masked view is the answer the code
// gives for the first of the cases the rule is tested against.
#[test]
fn the_masked_view_example_is_what_the_code_answers() {
    let section = masked_section();
    let start = section.find("```json\n").expect("a JSON example") + "```json\n".len();
    let end = start + section[start..].find("```").expect("the example's end");
    let shown: Value = serde_json::from_str(&section[start..end]).expect("the example is JSON");
    let cases: Vec<Value> =
        serde_json::from_str(&read("crates/vcon-server/tests/data/masked-cases.json"))
            .expect("the cases");
    let body = cases[0]["body"].as_str().expect("a body");
    let made = serde_json::to_value(vcon_server::masked::summary(body)).expect("JSON");
    assert_eq!(shown, made, "the page's example");
}

// The page's table names every field of the answer, and only those.
#[test]
fn the_masked_view_table_names_every_field_of_the_answer() {
    let section = masked_section();
    let named: BTreeSet<String> = section
        .lines()
        .filter(|line| line.starts_with("| `"))
        .flat_map(|line| {
            let cell = line.split('|').nth(1).unwrap_or_default();
            cell.split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    let made = serde_json::to_value(vcon_server::masked::summary("{}")).expect("JSON");
    let fields: BTreeSet<String> = made
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(named, fields, "the table's fields");
}

// ---- The catalog's refusal of a schema version, as the pages quote it. ----
//
// A binary that meets a catalog with a newer schema version than its own
// stops with `CatalogError::UnknownSchema`'s text, which names the
// catalog's version. A page that tells a reader what that refusal says
// names the version this build writes (an older binary reading this
// build's data) or the one past it (this build reading newer data). Only
// a `## From A to B` section of a page, the record of one upgrade, names
// an older version. On 2026-11-25 three troubleshooting entries still
// quoted version 9 after 0.4.17 moved the catalogs to 10.

/// One quote of the refusal on a page.
#[derive(Debug, PartialEq)]
struct RefusalQuote {
    /// The `## ` heading of the section that holds it, "" before the first.
    section: String,
    /// The version the quote names.
    version: i64,
    /// Whether the words after the number are the code's words.
    exact: bool,
}

/// The refusal's words after the version number, from the code.
fn refusal_tail() -> String {
    let text = vcon_server::catalog::CatalogError::UnknownSchema(0).to_string();
    let lead = "schema version 0";
    assert!(
        text.starts_with(lead),
        "the refusal starts with the version: {text}"
    );
    text[lead.len()..].to_owned()
}

/// Every quote of the refusal in `page`: the words `schema version N`
/// followed by ` is not`, whatever the line breaks between them.
fn refusal_quotes(page: &str, tail: &str) -> Vec<RefusalQuote> {
    const LEAD: &str = "schema version ";
    let mut found = Vec::new();
    let mut section = String::new();
    let mut body: Vec<String> = Vec::new();
    let mut flush = |section: &str, body: &mut Vec<String>| {
        // The section's words, one space between each, so a quote reads
        // the same across a line break.
        let text = body
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        body.clear();
        let mut rest = text.as_str();
        while let Some(at) = rest.find(LEAD) {
            let after = &rest[at + LEAD.len()..];
            let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            rest = &after[digits..];
            if digits == 0 || !rest.starts_with(" is not") {
                continue;
            }
            found.push(RefusalQuote {
                section: section.to_owned(),
                version: after[..digits].parse().expect("digits"),
                exact: rest.starts_with(tail),
            });
        }
    };
    for line in page.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            flush(&section, &mut body);
            section = heading.trim().to_owned();
        } else {
            body.push(line.to_owned());
        }
    }
    flush(&section, &mut body);
    found
}

/// What is wrong with the quotes of `file`, for a build at schema
/// version `build`.
fn stale_refusals(file: &str, page: &str, tail: &str, build: i64) -> Vec<String> {
    refusal_quotes(page, tail)
        .into_iter()
        .filter_map(|q| {
            let record = q.section.starts_with("From ");
            if !q.exact {
                Some(format!(
                    "{file}, section {:?}: schema version {}: the words after the number are not the code's {tail:?}",
                    q.section, q.version
                ))
            } else if record && q.version > build {
                Some(format!(
                    "{file}, section {:?}: schema version {}: an upgrade record names a version past this build's {build}",
                    q.section, q.version
                ))
            } else if !record && q.version != build && q.version != build + 1 {
                Some(format!(
                    "{file}, section {:?}: schema version {}: name {build} (an older binary on this build's data) or {} (this build on newer data)",
                    q.section,
                    q.version,
                    build + 1
                ))
            } else {
                None
            }
        })
        .collect()
}

/// Every Markdown page of the site, and the READMEs, by path from the root.
fn quoted_pages() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut pages = Vec::new();
    let mut dirs = vec![root.join("website/content")];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|x| x == "md") {
                let name = path
                    .strip_prefix(&root)
                    .expect("under the root")
                    .to_string_lossy()
                    .into_owned();
                pages.push((name.clone(), read(&name)));
            }
        }
    }
    for name in [
        "README.md",
        "crates/vcon-server/README.md",
        "crates/vcon/README.md",
    ] {
        pages.push((name.to_owned(), read(name)));
    }
    pages.sort();
    pages
}

const TAIL: &str = " is not one this build knows";

fn quote(section: &str, version: i64, exact: bool) -> RefusalQuote {
    RefusalQuote {
        section: section.to_owned(),
        version,
        exact,
    }
}

// Success.

#[test]
fn the_tail_is_the_codes_text_after_the_number() {
    assert_eq!(refusal_tail(), TAIL, "the refusal's words after the number");
}

#[test]
fn a_quote_on_one_line_is_read() {
    let page = "## When it fails\n\n- `schema version 10 is not one this build knows`: copy.\n";
    assert_eq!(
        refusal_quotes(page, TAIL),
        vec![quote("When it fails", 10, true)],
        "one quote, its section and its version"
    );
}

#[test]
fn a_quote_across_a_line_break_is_read() {
    let page = "## A\n\n`the registry cannot be opened: … schema version 10 is not one this\n  build knows`\n";
    assert_eq!(
        refusal_quotes(page, TAIL),
        vec![quote("A", 10, true)],
        "the words are joined across the break"
    );
}

#[test]
fn two_quotes_in_two_sections_are_both_read() {
    let page = "## A\nschema version 10 is not one this build knows\n## B\nschema version 11 is not one this build knows\n";
    assert_eq!(
        refusal_quotes(page, TAIL),
        vec![quote("A", 10, true), quote("B", 11, true)],
        "each quote with its own section"
    );
}

#[test]
fn a_quote_under_a_subheading_keeps_its_section() {
    let page = "## From 0.4.3 to 0.4.4\n### Schema version 9\nschema version 9 is not one this build knows\n";
    assert_eq!(
        refusal_quotes(page, TAIL),
        vec![quote("From 0.4.3 to 0.4.4", 9, true)],
        "a ### heading does not start a section"
    );
}

#[test]
fn a_quote_naming_this_builds_version_passes() {
    let page = "## When a step fails\nschema version 10 is not one this build knows\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10),
        Vec::<String>::new(),
        "an older binary reading this build's data names 10"
    );
}

#[test]
fn a_quote_naming_the_version_past_this_build_passes() {
    let page = "## Versions\nschema version 11 is not one this build knows\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10),
        Vec::<String>::new(),
        "this build reading newer data names 11"
    );
}

#[test]
fn an_upgrade_record_may_name_an_older_version() {
    let page = "## From 0.3.0 to 0.4.0\nschema version 4 is not one this build knows\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10),
        Vec::<String>::new(),
        "a From section records what that upgrade printed"
    );
}

#[test]
fn an_upgrade_record_may_name_this_builds_version() {
    let page = "## From 0.4.16 to 0.4.17\nschema version 10 is not one this build knows\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10),
        Vec::<String>::new(),
        "the upgrade to this build records the version it writes"
    );
}

#[test]
fn every_page_quotes_the_refusal_with_the_codes_words_and_a_live_version() {
    let tail = refusal_tail();
    let build = vcon_server::catalog::SCHEMA_VERSION;
    let problems: Vec<String> = quoted_pages()
        .iter()
        .flat_map(|(file, page)| stale_refusals(file, page, &tail, build))
        .collect();
    assert!(
        problems.is_empty(),
        "stale refusal quotes:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_pages_that_name_the_refusal_are_all_read() {
    let tail = refusal_tail();
    let quoted: BTreeSet<String> = quoted_pages()
        .into_iter()
        .filter(|(_, page)| !refusal_quotes(page, &tail).is_empty())
        .map(|(file, _)| file)
        .collect();
    for file in [
        "website/content/docs/on-disk-format.md",
        "website/content/docs/roles-and-scopes.md",
        "website/content/docs/tenant-administration.md",
        "website/content/docs/upgrading.md",
        "website/content/docs/users-and-memberships.md",
    ] {
        assert!(
            quoted.contains(file),
            "{file} quotes the refusal: {quoted:?}"
        );
    }
}

// Failure.

#[test]
fn a_quote_one_version_behind_this_build_is_refused() {
    let page = "## When a step fails\nschema version 9 is not one this build knows\n";
    let found = stale_refusals("p.md", page, TAIL, 10);
    assert_eq!(found.len(), 1, "one problem: {found:?}");
    assert!(
        found[0].contains("p.md")
            && found[0].contains("When a step fails")
            && found[0].contains('9'),
        "the problem names the file, the section and the version: {}",
        found[0]
    );
}

#[test]
fn a_quote_two_versions_past_this_build_is_refused() {
    let page = "## Versions\nschema version 12 is not one this build knows\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10).len(),
        1,
        "no release prints 12 for this build's data"
    );
}

#[test]
fn a_quote_in_other_words_is_refused() {
    let page = "## A\nschema version 10 is not known to this build\n";
    let found = stale_refusals("p.md", page, TAIL, 10);
    assert_eq!(found.len(), 1, "one problem: {found:?}");
    assert!(
        found[0].contains("words"),
        "the problem says the words differ: {}",
        found[0]
    );
}

#[test]
fn an_upgrade_record_past_this_build_is_refused() {
    let page = "## From 0.4.16 to 0.4.17\nschema version 11 is not one this build knows\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10).len(),
        1,
        "an upgrade to this build cannot have printed a later version"
    );
}

#[test]
fn a_quote_before_any_heading_is_held_to_this_build() {
    let page = "schema version 9 is not one this build knows\n## From 0.4.3 to 0.4.4\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10).len(),
        1,
        "text before the first section is not an upgrade record"
    );
}

#[test]
fn each_stale_quote_is_reported_once() {
    let page = "## A\nschema version 8 is not one this build knows\nschema version 9 is not one this build knows\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10).len(),
        2,
        "two stale quotes, two problems"
    );
}

// Edge.

#[test]
fn a_version_named_without_the_refusal_is_no_quote() {
    let page = "## A\nThe catalogs move to schema version 10.\n`schema version 4` or `3`\n";
    assert_eq!(
        refusal_quotes(page, TAIL),
        Vec::<RefusalQuote>::new(),
        "only a version followed by ` is not` is a quote"
    );
}

#[test]
fn a_heading_named_schema_version_is_no_quote() {
    let page = "### Schema version 9, and no way back without the copy\n";
    assert_eq!(
        refusal_quotes(page, TAIL),
        Vec::<RefusalQuote>::new(),
        "a capital S heading is not the refusal"
    );
}

#[test]
fn a_long_version_number_is_read_whole() {
    let page = "## A\nschema version 1234 is not one this build knows\n";
    assert_eq!(
        refusal_quotes(page, TAIL),
        vec![quote("A", 1234, true)],
        "every digit"
    );
}

#[test]
fn a_quote_at_the_end_of_the_page_is_read() {
    let page = "## A\nschema version 10 is not one this build knows";
    assert_eq!(
        refusal_quotes(page, TAIL),
        vec![quote("A", 10, true)],
        "no newline after it"
    );
}

#[test]
fn a_from_word_inside_a_heading_is_not_an_upgrade_record() {
    let page = "## Away from the old version\nschema version 4 is not one this build knows\n";
    assert_eq!(
        stale_refusals("p.md", page, TAIL, 10).len(),
        1,
        "only a heading that starts with `From ` records an upgrade"
    );
}

#[test]
fn a_page_with_no_quote_has_no_problem() {
    assert_eq!(
        stale_refusals("p.md", "## A\nNothing here.\n", TAIL, 10),
        Vec::<String>::new(),
        "nothing to check"
    );
}

#[test]
fn the_page_walk_finds_the_site_and_the_readmes() {
    let pages = quoted_pages();
    let names: BTreeSet<&str> = pages.iter().map(|(name, _)| name.as_str()).collect();
    for name in [
        "README.md",
        "website/content/_index.md",
        "website/content/docs/upgrading.md",
        "website/content/problems/internal.md",
    ] {
        assert!(names.contains(name), "the walk reads {name}");
    }
}
