//! The line `vcon-server migrate` prints, made from what the migration
//! did. These tests need no server.

use super::super::super::schema::{Migrated, SCHEMA_VERSION};

/// The how-to page, read from the website's sources.
fn how_to() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/content/docs/postgres-catalog.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn a_first_migration_prints_both_versions_and_that_it_migrated() {
    let line = Migrated {
        from: 0,
        to: SCHEMA_VERSION,
    }
    .line();
    assert_eq!(
        line,
        serde_json::json!({
            "migrated": true,
            "from": 0,
            "to": SCHEMA_VERSION,
            "message": format!("schema version 0 to schema version {SCHEMA_VERSION}"),
        }),
        "the four members of a first run"
    );
}

#[test]
fn an_upgrade_prints_the_version_found_and_the_version_left() {
    let line = Migrated { from: 3, to: 5 }.line();
    assert_eq!(line["from"], 3, "the version found");
    assert_eq!(line["to"], 5, "the version left");
    assert_eq!(line["migrated"], true, "the schema changed");
    assert_eq!(
        line["message"], "schema version 3 to schema version 5",
        "the message names both"
    );
}

#[test]
fn a_schema_already_at_the_version_prints_that_nothing_migrated() {
    let line = Migrated {
        from: SCHEMA_VERSION,
        to: SCHEMA_VERSION,
    }
    .line();
    assert_eq!(line["migrated"], false, "nothing was applied");
    assert_eq!(line["from"], line["to"], "the version is the same");
    assert_eq!(
        line["message"],
        format!("already at schema version {SCHEMA_VERSION}"),
        "the message says so"
    );
}

#[test]
fn the_message_member_is_the_message_of_the_migration() {
    for done in [
        Migrated { from: 0, to: 1 },
        Migrated { from: 2, to: 2 },
        Migrated { from: 1, to: 9 },
    ] {
        assert_eq!(
            done.line()["message"],
            done.message(),
            "{done:?}: one text, printed and tested"
        );
    }
}

#[test]
fn the_line_has_four_members_and_no_other() {
    let line = Migrated { from: 1, to: 2 }.line();
    let mut names: Vec<&str> = line
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["from", "message", "migrated", "to"],
        "the members the how-to shows"
    );
}

#[test]
fn the_printed_line_is_the_how_tos_with_this_binarys_version() {
    let shown = how_to()
        .lines()
        .find(|l| l.starts_with("{\"from\":0,"))
        .expect("the how-to shows the first run's line")
        .replace("<version>", &SCHEMA_VERSION.to_string());
    let printed = Migrated {
        from: 0,
        to: SCHEMA_VERSION,
    }
    .line()
    .to_string();
    assert_eq!(printed, shown, "the how-to's line is what migrate prints");
}

#[test]
fn the_how_tos_second_run_reads_the_members_the_line_has() {
    let page = how_to();
    assert!(
        page.contains("jq -c '{migrated, same: (.from == .to)}'"),
        "the how-to reads migrated, from and to"
    );
    let line = Migrated { from: 4, to: 4 }.line();
    assert_eq!(
        serde_json::json!({"migrated": line["migrated"], "same": line["from"] == line["to"]})
            .to_string(),
        "{\"migrated\":false,\"same\":true}",
        "what the how-to's second run prints"
    );
    assert!(
        page.contains("{\"migrated\":false,\"same\":true}"),
        "and the page shows it"
    );
}

#[test]
fn an_empty_database_with_no_migrations_prints_version_zero_unchanged() {
    let line = Migrated { from: 0, to: 0 }.line();
    assert_eq!(line["migrated"], false, "nothing to apply");
    assert_eq!(
        line["message"], "already at schema version 0",
        "version 0 is no schema"
    );
}

#[test]
fn the_highest_versions_print_without_overflow() {
    let line = Migrated {
        from: i32::MAX - 1,
        to: i32::MAX,
    }
    .line();
    assert_eq!(line["from"], i32::MAX - 1, "the version found");
    assert_eq!(line["to"], i32::MAX, "the version left");
    assert_eq!(line["migrated"], true, "one version was applied");
}

#[test]
fn the_versions_are_numbers_not_text() {
    let line = Migrated { from: 1, to: 2 }.line();
    assert!(
        line["from"].is_i64() && line["to"].is_i64(),
        "numbers: {line}"
    );
    assert!(line["migrated"].is_boolean(), "a boolean: {line}");
    assert!(line["message"].is_string(), "text: {line}");
}
