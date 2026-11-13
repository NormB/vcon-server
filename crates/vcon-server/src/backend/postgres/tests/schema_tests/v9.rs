//! Schema version 9: the count of a counted audit record takes any larger
//! number in one statement, where version 8 took one more. Nothing else
//! of a record changes, and no record is deleted, for the application
//! role and for the owner. Each live test of the migration starts from a
//! database at version 8.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 8;

/// The rule of version 9, as the migration writes it.
const RULE: &str = "    IF OLD.counted AND NEW.count > OLD.count";

/// This version's migration, or a panic that says there is none.
fn text() -> &'static str {
    assert!(
        MIGRATIONS.len() > THIS,
        "there are {} migrations",
        MIGRATIONS.len()
    );
    MIGRATIONS[THIS]
}

/// The lines of `text` that are statements, not comments.
fn statements(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect()
}

// What must work.

#[test]
fn version_9_is_the_ninth_migration_and_replaces_the_rule() {
    assert_eq!(
        text()
            .matches("CREATE OR REPLACE FUNCTION vcon.audit_append_only() RETURNS trigger")
            .count(),
        1,
        "the ninth replaces the function the triggers of version 8 run"
    );
}

#[test]
fn the_binary_s_version_is_at_least_9() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn the_rule_takes_a_larger_count_of_a_counted_record() {
    assert_eq!(
        statements(text())
            .iter()
            .filter(|line| **line == RULE)
            .count(),
        1,
        "the rule, once"
    );
}

#[test]
fn the_rule_still_compares_the_whole_row_but_its_count() {
    assert!(
        statements(text())
            .contains(&"        AND to_jsonb(NEW) - 'count' = to_jsonb(OLD) - 'count' THEN"),
        "a column a later version adds is held without a change here"
    );
}

#[test]
fn version_9_keeps_the_words_for_a_delete_and_for_emptying_a_log() {
    for words in [
        "RAISE EXCEPTION 'an audit log is not emptied'",
        "RAISE EXCEPTION 'an audit record is not deleted'",
    ] {
        assert_eq!(
            text().matches(words).count(),
            1,
            "{words}: as version 8 wrote it"
        );
        assert_eq!(
            MIGRATIONS[THIS - 1].matches(words).count(),
            1,
            "{words}: in version 8"
        );
    }
}

#[test]
fn every_refusal_of_version_9_has_the_privilege_error_code() {
    assert_eq!(
        text().matches("RAISE EXCEPTION ").count(),
        3,
        "emptied, deleted, changed"
    );
    assert_eq!(
        text()
            .matches("USING ERRCODE = 'insufficient_privilege';")
            .count(),
        3,
        "each with the code version 8 gave it"
    );
}

// What must be refused.

#[test]
fn a_version_8_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(8)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 8, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn the_rule_of_version_8_is_not_in_version_9() {
    assert!(
        !statements(text())
            .iter()
            .any(|line| line.contains("OLD.count + 1")),
        "one more is no longer the only change"
    );
    assert!(
        !text().contains("but for one more in the count"),
        "and the message does not say it is"
    );
}

#[test]
fn no_role_is_granted_anything_by_version_9() {
    for word in ["GRANT ", "REVOKE ", "SECURITY DEFINER", "OWNER TO"] {
        assert!(
            !statements(text()).iter().any(|line| line.contains(word)),
            "no {word}in version 9: the function keeps its owner and its privileges"
        );
    }
}

// The edges.

#[test]
fn version_9_changes_no_row_by_a_statement_and_makes_no_table_or_trigger() {
    // The owner role is held to row-level security too, so an UPDATE or
    // DELETE in a migration would raise. A statement begins its line.
    for statement in [
        "UPDATE ",
        "DELETE ",
        "DROP ",
        "TRUNCATE ",
        "INSERT ",
        "CREATE TABLE ",
        "ALTER TABLE ",
        "CREATE TRIGGER ",
        "CREATE FUNCTION ",
    ] {
        assert!(
            !text()
                .lines()
                .any(|line| line.trim_start().to_uppercase().starts_with(statement)),
            "no {statement}in version 9"
        );
    }
}

#[test]
fn the_function_of_version_9_has_a_fixed_search_path() {
    assert_eq!(
        text()
            .matches("SET search_path = pg_catalog, pg_temp")
            .count(),
        1,
        "the function reads no name from the caller's search path"
    );
}

#[test]
fn the_function_of_version_9_is_the_one_of_version_8_but_for_the_rule_and_its_words() {
    /// The function's lines in `text`, from its language to its end.
    fn body(text: &str) -> Vec<&str> {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines
            .iter()
            .position(|line| line.contains("FUNCTION vcon.audit_append_only() RETURNS trigger"))
            .expect("the function");
        let end = lines[start..]
            .iter()
            .position(|line| *line == "$$;")
            .expect("its end");
        lines[start + 1..=start + end].to_vec()
    }
    let old = body(MIGRATIONS[THIS - 1]);
    let new = body(text());
    assert_eq!(old.len(), new.len(), "line for line");
    let different: Vec<(&str, &str)> = old
        .iter()
        .zip(&new)
        .filter(|(a, b)| a != b)
        .map(|(a, b)| (*a, *b))
        .collect();
    assert_eq!(
        different,
        [
            ("    IF OLD.counted AND NEW.count = OLD.count + 1", RULE),
            (
                "    RAISE EXCEPTION 'an audit record is not changed, but for one more in the \
                 count of a counted record'",
                "    RAISE EXCEPTION 'an audit record is not changed, but for a larger count of a \
                 counted record'"
            ),
        ],
        "two lines and no other"
    );
}

// The pages that show this version.

/// The text of the docs page `name`.
fn page(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/content/docs")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_how_to_shows_the_upgrade_to_this_binary_s_version() {
    let page = page("postgres-catalog.md");
    let line = format!(
        "{{\"from\":6,\"message\":\"schema version 6 to schema version {SCHEMA_VERSION}\",\
         \"migrated\":true,\"to\":{SCHEMA_VERSION}}}"
    );
    assert!(
        page.lines().any(|l| l == line),
        "the line migrate prints after an upgrade from 0.4.5: {line}"
    );
    assert!(
        page.contains(&format!(
            "A build from `main` makes version {SCHEMA_VERSION},"
        )),
        "and the version in words"
    );
}

#[test]
fn the_how_to_names_no_older_version_as_this_binary_s() {
    let page = page("postgres-catalog.md");
    let ours = format!("this binary's {SCHEMA_VERSION}: run vcon-server migrate");
    let named: Vec<&str> = page.matches("this binary's ").collect();
    assert_eq!(
        page.matches(&ours).count(),
        named.len(),
        "every refusal the page shows names version {SCHEMA_VERSION}"
    );
    assert!(named.len() >= 2, "in the upgrade section and in the errors");
    let newer = format!(
        "(version {}, this binary {SCHEMA_VERSION})",
        SCHEMA_VERSION + 1
    );
    assert_eq!(
        page.matches(&newer).count(),
        page.matches(", this binary ").count(),
        "and the refusal of a newer database names the next one"
    );
}

#[test]
fn the_how_to_shows_the_words_of_the_rule_and_says_a_count_only_grows() {
    let page = page("postgres-catalog.md");
    assert!(
        page.lines().any(|l| l
            == "ERROR:  an audit record is not changed, but for a larger count of a counted record"),
        "the server's words, as psql prints them"
    );
    assert!(
        !page.contains("but for one more in the count"),
        "and not the words of version 8"
    );
    assert!(
        page.contains("The count only grows"),
        "the rule in the page's words"
    );
}

#[test]
fn the_status_page_names_this_binary_s_version() {
    let page = page("status.md");
    assert!(
        page.contains(&format!(
            "**Schema version {SCHEMA_VERSION} of the PostgreSQL preview.**"
        )) && page.contains(&format!("leaves the database at version {SCHEMA_VERSION};")),
        "the entry's title and its first sentence"
    );
}

#[test]
fn the_changelog_names_this_binary_s_version_and_the_rule() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../CHANGELOG.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let unreleased = text
        .split("\n## [")
        .find(|section| section.starts_with("Unreleased]"))
        .expect("an Unreleased section");
    assert!(
        unreleased.contains(&format!(
            "**The PostgreSQL preview's schema is at version {SCHEMA_VERSION}.**"
        )) && unreleased.contains(&format!(
            "older than this binary's {SCHEMA_VERSION}: run vcon-server migrate"
        )),
        "the entry's title and the refusal it quotes"
    );
    assert!(
        unreleased.contains("The count only grows"),
        "and the rule of version 9"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use tokio_postgres::Client;

    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::live::{Db, TENANT_A, TENANT_B, in_scope};
    use super::super::v8::on_a_server::{
        NOT_CHANGED, NOT_DELETED, NOT_EMPTIED, count_of, one, owner, refused, rows, scope, triggers,
    };
    use super::THIS;

    /// The two audit logs.
    const LOGS: [&str; 2] = ["audit", "admin_audit"];

    /// The words of version 8, which a database at version 8 still says.
    const ONE_MORE: &str =
        "an audit record is not changed, but for one more in the count of a counted record";

    /// The largest count a record holds.
    const LARGEST: &str = "9223372036854775807";

    /// A database at version 8.
    async fn at_version_8() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 8"),
            Migrated { from: 0, to: 8 },
            "the first eight migrations alone"
        );
        db
    }

    /// An INSERT of one record into the log `table` for `tenant`, with
    /// `count`, and ` RETURNING seq`.
    fn record(table: &str, tenant: &str, count: &str, counted: bool) -> String {
        match table {
            "audit" => format!(
                "INSERT INTO vcon.audit (tenant, at, actor, action, status, count, counted) \
                 VALUES ('\\x{tenant}', 't', 'a', 'b', 1, {count}, {counted}) RETURNING seq"
            ),
            "admin_audit" => format!(
                "INSERT INTO vcon.admin_audit (at, actor, action, status, count, counted) \
                 VALUES ('t', 'a', 'b', 1, {count}, {counted}) RETURNING seq"
            ),
            other => panic!("no record for {other}"),
        }
    }

    /// Stores one record with a count of one in `table`, as the
    /// application role.
    async fn store(db: &Db, table: &str, counted: bool) {
        one(
            &db.app().await,
            scope(table),
            &record(table, TENANT_A, "1", counted),
        )
        .await;
    }

    /// `UPDATE vcon.<table> SET <set>` by `client`, in the table's scope.
    async fn change(client: &Client, table: &str, set: &str) -> Result<(), String> {
        in_scope(
            client,
            scope(table),
            &format!("UPDATE vcon.{table} SET {set}"),
        )
        .await
        .map(|_| ())
    }

    /// Every record of `table` without its count, in the order stored,
    /// read by the owner.
    async fn but_the_count(db: &Db, table: &str) -> String {
        one(
            &owner(db).await,
            scope(table),
            &format!(
                "SELECT coalesce(jsonb_agg(to_jsonb(t) - 'count' - 'key_id' ORDER BY seq), '[]')::text \
                 FROM vcon.{table} t"
            ),
        )
        .await
    }

    /// The counts of `table`'s records, in the order stored, read by the
    /// owner.
    async fn counts(db: &Db, table: &str) -> String {
        one(
            &owner(db).await,
            scope(table),
            &format!(
                "SELECT coalesce(string_agg(count::text, ',' ORDER BY seq), '') FROM vcon.{table}"
            ),
        )
        .await
    }

    /// The text of the append-only function the database holds.
    async fn rule(db: &Db) -> String {
        one(
            &db.admin().await,
            TENANT_A,
            "SELECT prosrc FROM pg_proc WHERE proname = 'audit_append_only' \
             AND pronamespace = 'vcon'::regnamespace",
        )
        .await
    }

    // What must work.

    #[tokio::test]
    async fn the_application_role_adds_many_to_a_counted_record() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            change(&db.app().await, table, "count = count + 5")
                .await
                .expect("five more in one statement");
            assert_eq!(count_of(&db, table).await, "6", "{table}: one and five");
        }
    }

    #[tokio::test]
    async fn the_owner_adds_many_to_a_counted_record_too() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            change(&owner(&db).await, table, "count = count + 41")
                .await
                .expect("the rule is one for both roles");
            assert_eq!(count_of(&db, table).await, "42", "{table}");
        }
    }

    #[tokio::test]
    async fn one_more_is_a_larger_count_as_before() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            for client in [db.app().await, owner(&db).await] {
                change(&client, table, "count = count + 1")
                    .await
                    .expect("one more");
            }
            assert_eq!(count_of(&db, table).await, "3", "{table}: one more, twice");
        }
    }

    #[tokio::test]
    async fn a_count_is_set_to_a_larger_number() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            change(&db.app().await, table, "count = 1000")
                .await
                .expect("a number, not a sum");
            assert_eq!(count_of(&db, table).await, "1000", "{table}");
        }
    }

    #[tokio::test]
    async fn sums_added_one_after_another_add_up() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let app = db.app().await;
            for n in [3, 4, 1, 250] {
                change(&app, table, &format!("count = count + {n}"))
                    .await
                    .expect("each is larger than the last");
            }
            assert_eq!(count_of(&db, table).await, "259", "{table}");
        }
    }

    #[tokio::test]
    async fn an_update_that_writes_the_other_columns_as_they_are_is_a_larger_count() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            change(
                &owner(&db).await,
                table,
                "count = count + 7, at = at, version = NULL, target_mac = NULL, counted = true",
            )
            .await
            .expect("nothing but the count is different");
            assert_eq!(count_of(&db, table).await, "8", "{table}");
        }
    }

    #[tokio::test]
    async fn a_version_8_database_is_brought_to_version_9_with_its_records_as_they_were() {
        let db = at_version_8().await;
        // A server is refused at version 8, so the tester writes the rows.
        let admin = db.admin().await;
        let mut before = Vec::new();
        for table in LOGS {
            for (count, counted) in [("1", true), ("17", true), ("1", false)] {
                one(
                    &admin,
                    scope(table),
                    &record(table, TENANT_A, count, counted),
                )
                .await;
            }
            before.push((but_the_count(&db, table).await, counts(&db, table).await));
        }
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 8,
                to: SCHEMA_VERSION
            },
            "migrate brings it to this binary's version"
        );
        for (table, before) in LOGS.iter().zip(before) {
            assert_eq!(before.1, "1,17,1", "{table}: the counts stored");
            assert_eq!(
                (but_the_count(&db, table).await, counts(&db, table).await),
                before,
                "{table}: every record as it was, in its order"
            );
        }
    }

    #[tokio::test]
    async fn a_record_counted_before_the_migration_takes_many_after_it() {
        let db = at_version_8().await;
        for table in LOGS {
            one(
                &db.admin().await,
                scope(table),
                &record(table, TENANT_A, "17", true),
            )
            .await;
        }
        db.migrate().await;
        for table in LOGS {
            change(&db.app().await, table, "count = count + 83")
                .await
                .expect("the rule of version 9 holds the old record");
            assert_eq!(count_of(&db, table).await, "100", "{table}");
        }
    }

    #[tokio::test]
    async fn a_new_database_has_the_rule() {
        let db = Db::migrated().await;
        let rule = rule(&db).await;
        assert!(
            rule.contains("NEW.count > OLD.count") && !rule.contains("OLD.count + 1"),
            "the function a new database runs: {rule}"
        );
    }

    #[tokio::test]
    async fn migrating_twice_from_version_8_does_the_work_once() {
        let db = at_version_8().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 8,
                to: SCHEMA_VERSION
            },
            "the first run"
        );
        let first = rule(&db).await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "the second finds nothing to do"
        );
        assert_eq!(rule(&db).await, first, "and leaves the function as it is");
    }

    #[tokio::test]
    async fn the_migration_keeps_the_triggers_of_version_8() {
        let db = at_version_8().await;
        assert_eq!(triggers(&db).await, "8", "version 8 made them");
        db.migrate().await;
        assert_eq!(
            triggers(&db).await,
            "8",
            "version 9 replaces the function and makes or drops no trigger"
        );
    }

    #[tokio::test]
    async fn the_function_keeps_its_owner_and_public_cannot_run_it() {
        let db = Db::migrated().await;
        let facts = one(
            &db.admin().await,
            TENANT_A,
            "SELECT pg_get_userbyid(proowner) || ' ' || \
             has_function_privilege('public', oid, 'EXECUTE')::text || ' ' || \
             has_function_privilege('vcon_app', oid, 'EXECUTE')::text || ' ' || \
             prosecdef::text || ' ' || array_to_string(proconfig, ';') \
             FROM pg_proc WHERE proname = 'audit_append_only' \
             AND pronamespace = 'vcon'::regnamespace",
        )
        .await;
        assert_eq!(
            facts, "vcon_owner false false false search_path=pg_catalog, pg_temp",
            "the owner, PUBLIC's and the application role's leave to run it, whether it runs as \
             its owner, and its settings"
        );
    }

    #[tokio::test]
    async fn every_counted_record_an_update_finds_takes_the_sum() {
        let db = Db::migrated().await;
        for table in LOGS {
            for count in ["1", "5", "70"] {
                one(
                    &db.app().await,
                    scope(table),
                    &record(table, TENANT_A, count, true),
                )
                .await;
            }
            change(&db.app().await, table, "count = count + 10")
                .await
                .expect("each is larger than it was");
            assert_eq!(counts(&db, table).await, "11,15,80", "{table}");
        }
    }

    #[tokio::test]
    async fn an_accepted_update_changes_the_count_and_nothing_else_of_the_record() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let before = but_the_count(&db, table).await;
            change(&db.app().await, table, "count = count + 12")
                .await
                .expect("twelve more");
            assert_eq!(
                but_the_count(&db, table).await,
                before,
                "{table}: its number and every other column"
            );
            assert_eq!(count_of(&db, table).await, "13", "{table}");
        }
    }

    // What must be refused.

    #[tokio::test]
    async fn a_database_at_version_8_still_takes_one_more_only() {
        let db = at_version_8().await;
        for table in LOGS {
            let admin = db.admin().await;
            one(&admin, scope(table), &record(table, TENANT_A, "1", true)).await;
            let e = refused(
                &owner(&db).await,
                scope(table),
                &format!("UPDATE vcon.{table} SET count = count + 2"),
            )
            .await;
            assert_eq!(e, ONE_MORE, "{table}: the rule the migration replaces");
        }
    }

    #[tokio::test]
    async fn a_count_is_not_set_to_what_it_is() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            for to in ["count", "count + 0", "1", "count * 1"] {
                for client in [db.app().await, owner(&db).await] {
                    let e = change(&client, table, &format!("count = {to}"))
                        .await
                        .expect_err("not larger");
                    assert_eq!(e, NOT_CHANGED, "{table}: count = {to}");
                }
            }
            assert_eq!(count_of(&db, table).await, "1", "{table}");
        }
    }

    #[tokio::test]
    async fn a_count_is_not_made_smaller() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            change(&db.app().await, table, "count = 50")
                .await
                .expect("fifty");
            for to in [
                "count - 1",
                "49",
                "1",
                "0",
                "-1",
                "count / 2",
                "-9223372036854775808",
            ] {
                for client in [db.app().await, owner(&db).await] {
                    let e = change(&client, table, &format!("count = {to}"))
                        .await
                        .expect_err("smaller");
                    assert_eq!(e, NOT_CHANGED, "{table}: count = {to}");
                }
            }
            assert_eq!(count_of(&db, table).await, "50", "{table}");
        }
    }

    #[tokio::test]
    async fn a_larger_count_with_another_column_changed_is_refused() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let before = but_the_count(&db, table).await;
            let client = owner(&db).await;
            for other in [
                "at = 'later'",
                "actor = 'other'",
                "action = 'other'",
                "target_mac = '\\x01'",
                "target_sealed = '\\x01'",
                "version = 'v'",
                "status = 2",
                "status = NULL",
                "counted = false",
                "seq = DEFAULT",
            ] {
                for count in ["count + 1", "count + 5", "1000"] {
                    let e = change(&client, table, &format!("{other}, count = {count}"))
                        .await
                        .expect_err("another column");
                    assert_eq!(e, NOT_CHANGED, "{table}: {other}, count = {count}");
                }
            }
            assert_eq!(
                count_of(&db, table).await,
                "1",
                "{table}: the count is as stored"
            );
            assert_eq!(
                but_the_count(&db, table).await,
                before,
                "{table}: and the rest"
            );
        }
    }

    #[tokio::test]
    async fn another_column_alone_is_not_changed() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            for other in [
                "at = 'later'",
                "status = 2",
                "counted = false",
                "seq = DEFAULT",
            ] {
                let e = change(&owner(&db).await, table, other)
                    .await
                    .expect_err("no count at all");
                assert_eq!(e, NOT_CHANGED, "{table}: {other}");
            }
        }
    }

    #[tokio::test]
    async fn the_application_role_has_no_leave_to_write_another_column() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let e = change(&db.app().await, table, "actor = 'other', count = count + 5")
                .await
                .expect_err("the grant is of the count alone");
            assert!(
                e.contains(&format!("permission denied for table {table}")),
                "{table}: {e}"
            );
            assert_eq!(count_of(&db, table).await, "1", "{table}");
        }
    }

    #[tokio::test]
    async fn the_count_of_a_record_that_is_not_counted_takes_no_larger_number() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, false).await;
            for to in ["count + 1", "count + 5", "1000"] {
                for client in [db.app().await, owner(&db).await] {
                    let e = change(&client, table, &format!("count = {to}"))
                        .await
                        .expect_err("not counted");
                    assert_eq!(e, NOT_CHANGED, "{table}: count = {to}");
                }
            }
            assert_eq!(count_of(&db, table).await, "1", "{table}");
        }
    }

    #[tokio::test]
    async fn a_record_is_not_moved_to_another_tenant_with_a_larger_count() {
        let db = Db::migrated().await;
        store(&db, "audit", true).await;
        let e = change(
            &owner(&db).await,
            "audit",
            &format!("tenant = '\\x{TENANT_B}', count = count + 5"),
        )
        .await
        .expect_err("the tenant is a column like the others");
        assert_eq!(e, NOT_CHANGED, "the words of the rule");
    }

    #[tokio::test]
    async fn a_count_is_not_set_to_null() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            for client in [db.app().await, owner(&db).await] {
                let e = change(&client, table, "count = NULL")
                    .await
                    .expect_err("no number is not a larger one");
                assert_eq!(e, NOT_CHANGED, "{table}");
            }
            assert_eq!(count_of(&db, table).await, "1", "{table}");
        }
    }

    #[tokio::test]
    async fn no_record_is_deleted_under_version_9() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let sql = format!("DELETE FROM vcon.{table}");
            let e = refused(&owner(&db).await, scope(table), &sql).await;
            assert_eq!(e, NOT_DELETED, "{table}: the trigger holds the owner");
            let e = refused(&db.app().await, scope(table), &sql).await;
            assert!(
                e.contains(&format!("permission denied for table {table}")),
                "{table}: the application role has no leave at all: {e}"
            );
            assert_eq!(rows(&db, table).await, "1", "{table}: the record is there");
        }
    }

    #[tokio::test]
    async fn no_log_is_emptied_under_version_9() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let sql = format!("TRUNCATE vcon.{table}");
            let e = refused(&owner(&db).await, scope(table), &sql).await;
            assert_eq!(e, NOT_EMPTIED, "{table}: the trigger holds the owner");
            let e = refused(&db.app().await, scope(table), &sql).await;
            assert!(
                e.contains(&format!("permission denied for table {table}")),
                "{table}: the application role has no leave at all: {e}"
            );
            assert_eq!(rows(&db, table).await, "1", "{table}: the record is there");
        }
    }

    #[tokio::test]
    async fn a_record_whose_count_grew_is_not_deleted_either() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            change(&db.app().await, table, "count = count + 9")
                .await
                .expect("nine more");
            let e = refused(
                &owner(&db).await,
                scope(table),
                &format!("DELETE FROM vcon.{table} WHERE count = 10"),
            )
            .await;
            assert_eq!(e, NOT_DELETED, "{table}");
            assert_eq!(
                count_of(&db, table).await,
                "10",
                "{table}: there, as counted"
            );
        }
    }

    #[tokio::test]
    async fn a_version_9_that_fails_leaves_version_8_whole() {
        let db = at_version_8().await;
        let old = rule(&db).await;
        let broken = format!("{}\nSELECT boom;", MIGRATIONS[THIS]);
        let mut migrations: Vec<&str> = MIGRATIONS[..THIS].to_vec();
        migrations.push(&broken);
        let e = db.migrate_with(&migrations).await.expect_err("fails");
        assert!(e.starts_with("version 9: "), "names the version: {e}");
        assert_eq!(rule(&db).await, old, "the function was rolled back with it");
        assert!(
            old.contains("OLD.count + 1"),
            "the rule of version 8: {old}"
        );
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 8,
                to: SCHEMA_VERSION
            },
            "and the real migration then runs from 8"
        );
    }

    #[tokio::test]
    async fn the_application_role_cannot_replace_the_rule_of_version_9() {
        let db = Db::migrated().await;
        let e = refused(
            &db.app().await,
            TENANT_A,
            "CREATE OR REPLACE FUNCTION vcon.audit_append_only() RETURNS trigger \
             LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END'",
        )
        .await;
        assert!(e.contains("permission denied for schema vcon"), "{e}");
        let e = refused(&db.app().await, TENANT_A, "SELECT vcon.audit_append_only()").await;
        assert!(
            e.contains("permission denied for function audit_append_only"),
            "{e}"
        );
    }

    #[tokio::test]
    async fn a_refused_count_among_several_records_changes_none() {
        let db = Db::migrated().await;
        for table in LOGS {
            for count in ["1", "5"] {
                one(
                    &db.app().await,
                    scope(table),
                    &record(table, TENANT_A, count, true),
                )
                .await;
            }
            // Three is larger than the first count and smaller than the second.
            let e = change(&db.app().await, table, "count = 3")
                .await
                .expect_err("the second record's count would be smaller");
            assert_eq!(e, NOT_CHANGED, "{table}");
            assert_eq!(counts(&db, table).await, "1,5", "{table}: neither changed");
        }
    }

    // The edges.

    #[tokio::test]
    async fn the_smallest_larger_count_is_taken_and_the_same_number_again_is_not() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let app = db.app().await;
            change(&app, table, "count = 2").await.expect("one to two");
            let e = change(&app, table, "count = 2")
                .await
                .expect_err("two to two");
            assert_eq!(e, NOT_CHANGED, "{table}");
            assert_eq!(count_of(&db, table).await, "2", "{table}");
        }
    }

    #[tokio::test]
    async fn a_count_is_set_to_the_largest_number_and_then_takes_no_more() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let app = db.app().await;
            change(&app, table, &format!("count = {LARGEST}"))
                .await
                .expect("the largest is larger than one");
            let e = change(&app, table, "count = count + 1")
                .await
                .expect_err("no number is larger");
            assert!(e.contains("bigint out of range"), "{table}: {e}");
            assert_eq!(count_of(&db, table).await, LARGEST, "{table}");
        }
    }

    #[tokio::test]
    async fn a_sum_past_the_largest_number_is_an_error_and_changes_nothing() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let e = change(
                &db.app().await,
                table,
                &format!("count = count + {LARGEST}"),
            )
            .await
            .expect_err("one and the largest");
            assert!(e.contains("bigint out of range"), "{table}: {e}");
            assert_eq!(count_of(&db, table).await, "1", "{table}");
        }
    }

    #[tokio::test]
    async fn a_negative_count_takes_a_larger_negative_one() {
        // No rule holds a count above zero; the rule is of the order alone.
        let db = Db::migrated().await;
        for table in LOGS {
            one(
                &db.app().await,
                scope(table),
                &record(table, TENANT_A, "-5", true),
            )
            .await;
            change(&db.app().await, table, "count = -4")
                .await
                .expect("larger");
            let e = change(&db.app().await, table, "count = -5")
                .await
                .expect_err("smaller");
            assert_eq!(e, NOT_CHANGED, "{table}");
            assert_eq!(count_of(&db, table).await, "-4", "{table}");
        }
    }

    #[tokio::test]
    async fn a_tenant_adds_nothing_to_another_tenant_s_count_by_any_sum() {
        let db = Db::migrated().await;
        store(&db, "audit", true).await;
        let found = one(
            &db.app().await,
            TENANT_B,
            "WITH changed AS (UPDATE vcon.audit SET count = count + 5 RETURNING 1) \
             SELECT count(*) FROM changed",
        )
        .await;
        assert_eq!(found, "0", "the other tenant's update finds no record");
        assert_eq!(count_of(&db, "audit").await, "1", "the count is as stored");
    }

    #[tokio::test]
    async fn an_update_that_finds_no_record_is_no_error() {
        let db = Db::migrated().await;
        for table in LOGS {
            for client in [db.app().await, owner(&db).await] {
                change(&client, table, "count = count + 5")
                    .await
                    .expect("a row trigger runs for a row");
            }
            assert_eq!(rows(&db, table).await, "0", "{table}: and stores none");
        }
    }

    #[tokio::test]
    async fn a_larger_count_rolled_back_leaves_the_count() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let app = db.app().await;
            let batch = format!(
                "BEGIN; SET LOCAL vcon.tenant = '{}'; UPDATE vcon.{table} SET count = count + 5; \
                 ROLLBACK",
                scope(table)
            );
            app.batch_execute(&batch).await.expect("runs");
            assert_eq!(count_of(&db, table).await, "1", "{table}");
        }
    }

    #[tokio::test]
    async fn two_connections_add_to_one_record_and_both_sums_are_kept() {
        let db = Db::migrated().await;
        for table in LOGS {
            store(&db, table, true).await;
            let (first, second) = (db.app().await, db.app().await);
            let (a, b) = tokio::join!(
                change(&first, table, "count = count + 3"),
                change(&second, table, "count = count + 4")
            );
            a.expect("the first");
            b.expect("the second waits for the row and reads the new count");
            assert_eq!(
                count_of(&db, table).await,
                "8",
                "{table}: one, three and four"
            );
        }
    }

    #[tokio::test]
    async fn an_accepted_update_keeps_the_record_s_number() {
        let db = Db::migrated().await;
        for table in LOGS {
            let seq = one(
                &db.app().await,
                scope(table),
                &record(table, TENANT_A, "1", true),
            )
            .await;
            let after = one(
                &db.app().await,
                scope(table),
                &format!("UPDATE vcon.{table} SET count = count + 5 RETURNING seq"),
            )
            .await;
            assert_eq!(after, seq, "{table}: the number the server gave it");
        }
    }

    #[tokio::test]
    async fn only_the_counted_records_a_statement_names_are_added_to() {
        let db = Db::migrated().await;
        for table in LOGS {
            for counted in [true, false, true] {
                one(
                    &db.app().await,
                    scope(table),
                    &record(table, TENANT_A, "1", counted),
                )
                .await;
            }
            change(&db.app().await, table, "count = count + 5 WHERE counted")
                .await
                .expect("the counted ones");
            assert_eq!(counts(&db, table).await, "6,1,6", "{table}");
        }
    }

    #[tokio::test]
    async fn the_records_of_two_tenants_are_kept_by_the_migration_each_for_its_tenant() {
        let db = at_version_8().await;
        let admin = db.admin().await;
        one(&admin, TENANT_A, &record("audit", TENANT_A, "3", true)).await;
        one(&admin, TENANT_B, &record("audit", TENANT_B, "8", true)).await;
        db.migrate().await;
        for (tenant, count) in [(TENANT_A, "3"), (TENANT_B, "8")] {
            let found = one(
                &db.app().await,
                tenant,
                "SELECT string_agg(count::text, ',') FROM vcon.audit",
            )
            .await;
            assert_eq!(found, count, "{tenant}: its record and no other");
        }
    }

    #[tokio::test]
    async fn the_rule_reads_the_same_on_this_server_as_in_the_migration() {
        let db = Db::migrated().await;
        let rule = rule(&db).await;
        for line in [
            "IF OLD.counted AND NEW.count > OLD.count",
            "AND to_jsonb(NEW) - 'count' = to_jsonb(OLD) - 'count' THEN",
            "RAISE EXCEPTION 'an audit record is not changed, but for a larger count of a \
             counted record'",
        ] {
            assert_eq!(rule.matches(line).count(), 1, "{line}");
        }
    }
}
