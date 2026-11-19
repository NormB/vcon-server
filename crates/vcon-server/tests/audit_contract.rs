//! The audit records' part of the contract (website/content/docs/stability.md):
//! contract/audit.json holds the members of an audit record and every
//! action a record can name. `scripts/contract_diff.py --kind audit`
//! compares two of these files.
//!
//! The members are those of a record the audit log writes and reads back
//! (`audit::export`), and the page's "A record" table lists the same ones;
//! the actions are the page's "Every action" table, which tests/docs_pages.rs
//! holds to the code. The snapshot is written, never edited by hand:
//! [`REGENERATE`] writes it again, and the change is reviewed in the diff.

use std::collections::BTreeSet;
use std::path::PathBuf;

use time::OffsetDateTime;
use vcon_server::audit::{self, Action, Actor, Entry, NO_ANSWER};

#[path = "support/audit_page.rs"]
mod audit_page;

mod contract_snapshot {
    include!("support/contract_snapshot.rs");
}

/// The variable that makes [`the_snapshot_is_what_the_log_writes_and_the_page_lists`]
/// write the snapshot, as for contract/cli.json.
const WRITE: &str = "VCON_SERVER_WRITE_CONTRACT";

/// The one command that writes contract/audit.json again.
const REGENERATE: &str =
    "VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --test audit_contract";

/// The snapshot, from the repository root.
const SNAPSHOT: &str = "contract/audit.json";

/// The page that lists the members and the actions.
const PAGE: &str = "website/content/docs/audit-log-records.md";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn page() -> String {
    std::fs::read_to_string(root().join(PAGE)).expect("audit-log-records.md")
}

/// Each record the log reads back after `entries` are recorded.
fn exported(entries: &[Entry]) -> Vec<serde_json::Value> {
    let log = rusqlite::Connection::open_in_memory().expect("an in-memory log");
    log.execute_batch(audit::SCHEMA).expect("the audit table");
    for entry in entries {
        audit::record(&log, entry, OffsetDateTime::UNIX_EPOCH).expect("recorded");
    }
    let mut out = Vec::new();
    audit::export(&log, None, &mut out).expect("exported");
    String::from_utf8(out)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("a JSON record"))
        .collect()
}

/// The members of each record the log reads back after `entries` are
/// recorded, each record's in its order.
fn exported_members(entries: &[Entry]) -> Vec<Vec<String>> {
    exported(entries)
        .iter()
        .map(|record| {
            record
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect()
        })
        .collect()
}

/// A record of a request, and one of something that answers none.
fn two_entries() -> [Entry; 2] {
    [
        Entry {
            actor: Actor::Sweep,
            action: Action::Purge,
            target: Some("01a07da8-c2bb-83e5-b9a2-279e0d16bc46".into()),
            version: None,
            status: 200,
        },
        Entry {
            actor: Actor::Cli,
            action: Action::ListenerFull,
            target: Some("listen".into()),
            version: None,
            status: NO_ANSWER,
        },
    ]
}

/// The members of a record this build writes, sorted.
fn members_now() -> Vec<String> {
    let mut members = exported_members(&two_entries()[..1])
        .pop()
        .expect("one record");
    members.sort();
    members
}

/// The snapshot of `members` and `actions`, as contract_diff.py reads it.
fn render(members: &[String], actions: &BTreeSet<String>) -> String {
    let mut members = members.to_vec();
    members.sort();
    let snapshot = serde_json::json!({ "actions": actions, "members": members });
    let mut text = serde_json::to_string_pretty(&snapshot).expect("JSON");
    text.push('\n');
    text
}

fn stale(file: &str, now: &str) -> String {
    contract_snapshot::stale(
        SNAPSHOT,
        "the audit log writes and audit-log-records.md lists",
        REGENERATE,
        file,
        now,
    )
}

// Success.

#[test]
fn the_snapshot_is_what_the_log_writes_and_the_page_lists() {
    let now = render(&members_now(), &audit_page::actions(&page()));
    let path = root().join(SNAPSHOT);
    if std::env::var_os(WRITE).is_some() {
        std::fs::write(&path, &now).expect("the snapshot is written");
        return;
    }
    let file = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(file == now, "{}", stale(&file, &now));
}

#[test]
fn the_page_lists_the_members_a_record_has() {
    let mut listed = audit_page::members(&page());
    listed.sort();
    assert_eq!(
        listed,
        members_now(),
        "audit-log-records.md's A record table and a written record"
    );
}

#[test]
fn a_record_that_answers_no_request_has_the_same_members() {
    let entries = two_entries();
    let values = exported(&entries);
    assert_eq!(
        values[1]["status"],
        serde_json::Value::Null,
        "the second record answers no request"
    );
    let records = exported_members(&entries);
    assert_eq!(records.len(), 2, "both read back");
    let mut answered = records[0].clone();
    let mut unanswered = records[1].clone();
    answered.sort();
    unanswered.sort();
    assert_eq!(
        answered, unanswered,
        "a null status is a member, not a missing one"
    );
}

#[test]
fn the_page_lists_at_least_the_actions_of_the_two_records() {
    let actions = audit_page::actions(&page());
    for action in [Action::Purge.as_str(), Action::ListenerFull.as_str()] {
        assert!(actions.contains(action), "the page lists {action}");
    }
}

// The readers, on planted pages.

const SAMPLE: &str = "\
intro

## A record

| Field | Holds |
|---|---|
| `seq` | the number |
| `at` | when |

## Every action

| Action | Records | Log |
|---|---|---|
| `read` | a read | the tenant's |
| `create` | a store | the tenant's |

## Next

| `other` | not an action |
";

#[test]
fn the_readers_read_the_members_and_the_actions_of_their_sections() {
    assert_eq!(audit_page::members(SAMPLE), ["seq", "at"], "page order");
    assert_eq!(
        audit_page::actions(SAMPLE),
        BTreeSet::from(["create".to_owned(), "read".to_owned()]),
        "the action rows only"
    );
}

#[test]
fn a_page_without_a_section_has_no_names_from_it() {
    assert!(
        audit_page::members("## Other\n| `x` | y |\n").is_empty(),
        "no A record section"
    );
    assert!(
        audit_page::section(SAMPLE, "Missing").is_none(),
        "no such section"
    );
}

#[test]
fn a_row_without_a_backticked_name_is_not_read() {
    let page = SAMPLE.replace("| `at` | when |", "| at | when |");
    assert_eq!(
        audit_page::members(&page),
        ["seq"],
        "the plain cell is not a member"
    );
}

#[test]
fn the_rendered_snapshot_is_sorted_and_ends_with_a_newline() {
    let text = render(
        &["seq".to_owned(), "at".to_owned()],
        &BTreeSet::from(["read".to_owned(), "create".to_owned()]),
    );
    assert!(text.ends_with("}\n"), "a newline at the end");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(
        value["members"],
        serde_json::json!(["at", "seq"]),
        "members sorted"
    );
    assert_eq!(
        value["actions"],
        serde_json::json!(["create", "read"]),
        "actions sorted"
    );
}

#[test]
fn a_stale_snapshot_names_the_command_that_writes_it() {
    let message = stale("", "{\n");
    assert!(message.contains(REGENERATE), "{message}");
    assert!(
        message.contains(
            "contract/audit.json is not what the audit log writes and audit-log-records.md lists"
        ),
        "{message}"
    );
}
