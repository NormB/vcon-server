//! Schema version 14: a tenant's catalog records its writes under way, in
//! the table `pending_write`, so recovery keeps the object of a write
//! that has not yet added its version's row (store/pending.rs). Each live
//! test of the migration starts from a database at version 13.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 13;

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

/// The migration's statements as one line, every run of white space one
/// space.
fn one_line() -> String {
    statements(text())
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A file of the repository as one line, every run of white space one
/// space.
fn flat(path: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The migration's comment as one line, without the comment marks.
fn comment() -> String {
    text()
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("--"))
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

// What must work.

#[test]
fn version_14_is_the_fourteenth_migration_and_makes_the_one_table() {
    assert_eq!(
        text().matches("CREATE TABLE vcon.pending_write (").count(),
        1,
        "the fourteenth makes pending_write once"
    );
    assert_eq!(
        text().matches("CREATE TABLE").count(),
        1,
        "and no other table"
    );
}

#[test]
fn the_binary_s_version_is_at_least_14() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn the_application_role_may_read_add_and_delete_pending_rows() {
    assert_eq!(
        statements(text())
            .iter()
            .filter(
                |line| **line == "GRANT SELECT, INSERT, DELETE ON vcon.pending_write TO vcon_app;"
            )
            .count(),
        1,
        "the one grant of the migration"
    );
    assert_eq!(one_line().matches("GRANT").count(), 1, "and no other grant");
}

#[test]
fn a_row_is_a_tenant_s_version_and_the_time_its_write_began() {
    assert!(
        one_line().contains(
            "CREATE TABLE vcon.pending_write ( tenant bytea NOT NULL, version bytea NOT NULL, \
             started_at bigint NOT NULL, PRIMARY KEY (tenant, version) );"
        ),
        "the columns and the key: {}",
        one_line()
    );
}

#[test]
fn the_table_has_the_policy_of_a_tenant_s_rows() {
    assert!(
        one_line().contains(
            "CREATE POLICY tenant_rows ON vcon.pending_write USING (tenant = vcon.tenant()) \
             WITH CHECK (tenant = vcon.tenant());"
        ),
        "read and written in its tenant's transactions only: {}",
        one_line()
    );
}

#[test]
fn the_migration_says_why_the_table_starts_empty() {
    let comment = comment();
    assert!(
        comment.contains(
            "The table starts empty: migrate holds the database's lock, which a serving server \
             holds too, so no write is under way while it runs."
        ),
        "the reason, in the migration's comment: {comment}"
    );
}

#[test]
fn the_how_to_says_what_version_14_changes_under_schema_versions() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    let (_, versions) = page
        .split_once(" ### Schema versions ")
        .expect("the how-to has the section Schema versions");
    assert!(
        versions.contains("Version 14 makes the table `pending_write`"),
        "version 14, in the section of the schema's versions"
    );
    // The line migrate prints names the binary's version, 15 from schema
    // version 15 on (schema_tests::v9).
}

#[test]
fn the_status_page_names_version_14() {
    let page = flat("../../website/content/docs/status.md");
    assert!(
        page.contains("Version 14 adds the table `pending_write`"),
        "in the entry of the PostgreSQL preview's schema version"
    );
}

// What must be refused.

#[test]
fn a_version_13_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(13)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 13, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn the_application_role_is_granted_no_update_and_no_truncate() {
    let line = one_line();
    for word in ["UPDATE", "TRUNCATE", "ALL"] {
        assert!(
            !line.split([' ', ',', '(', ')', ';']).any(|w| w == word),
            "{word} is not in the migration: {line}"
        );
    }
}

#[test]
fn version_14_replaces_no_function_and_no_trigger_and_makes_no_sequence() {
    let line = one_line();
    for word in ["FUNCTION", "TRIGGER", "SEQUENCE", "IDENTITY", "DROP"] {
        assert!(!line.contains(word), "{word} is not in the migration");
    }
}

// The edges.

#[test]
fn version_14_changes_no_other_table() {
    let line = one_line();
    assert_eq!(
        line.matches("vcon.").count(),
        line.matches("vcon.pending_write").count()
            + line.matches("vcon.tenant()").count()
            + line.matches("vcon.tenant_lock ").count(),
        "every name of the schema is the new table's, the tenant's marker or the lock row's: {line}"
    );
    assert_eq!(
        line.matches("vcon.tenant_lock ").count(),
        1,
        "the lock row's table is named once, in its one change: {line}"
    );
    assert_eq!(
        line.matches("ALTER TABLE").count(),
        3,
        "the two statements of row level security and the lock row's column"
    );
}

#[test]
fn the_lock_row_gains_one_removing_column_false_by_default() {
    assert_eq!(
        one_line()
            .matches(
                "ALTER TABLE vcon.tenant_lock ADD COLUMN removing boolean NOT NULL DEFAULT false;"
            )
            .count(),
        1,
        "one column, never null, false for every row there is"
    );
}

#[test]
fn the_migration_says_why_the_lock_row_is_marked() {
    let comment = comment();
    for said in [
        "answer that the version is damaged",
        "sets `removing` on the tenant's lock row",
        "it waits for every transaction that holds the row",
        "A removal cut off leaves the row marked",
    ] {
        assert!(comment.contains(said), "the comment says: {said}");
    }
}

#[test]
fn a_write_begins_only_on_an_unmarked_lock_row() {
    use super::super::super::catalog::{Begin, begin_text};
    let text = begin_text(Begin::Write, crate::ids::TenantId([0xab; 16]));
    assert!(
        text.contains("AND NOT removing FOR UPDATE"),
        "the begin takes the lock row only when it is not marked: {text}"
    );
}

#[test]
fn the_table_has_row_level_security_forced_on_its_owner() {
    for kind in ["ENABLE", "FORCE"] {
        assert_eq!(
            statements(text())
                .iter()
                .filter(|line| **line
                    == format!("ALTER TABLE vcon.pending_write {kind} ROW LEVEL SECURITY;"))
                .count(),
            1,
            "{kind}"
        );
    }
}

#[test]
fn the_migration_keeps_the_version_id_and_the_time_in_the_clear() {
    assert!(
        comment().contains("The version id is random and in the bucket's object names, and the time is a time: both are in the clear."),
        "why no column is sealed: {}",
        comment()
    );
    assert!(!one_line().contains("_sealed"), "no sealed column");
    assert!(!one_line().contains("_mac"), "no keyed column");
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::schema::{MIGRATIONS, Migrated};
    use super::super::live::{Db, TENANT_A, TENANT_B, in_scope, no_scope};
    use super::super::v8::on_a_server::{one, owner, refused};
    use super::THIS;

    /// What a transaction with no marker is told.
    const NO_TENANT: &str = "no tenant set for this transaction";

    /// The pending rows a scope reads: each version and its time, in hex
    /// and digits, by version.
    const ROWS: &str = "SELECT coalesce(string_agg(encode(version, 'hex') || ':' || started_at, ',' \
                        ORDER BY version), '') FROM vcon.pending_write";

    /// The statement that records a write of version `v` (hex) begun at
    /// `at`, for `tenant` (hex).
    fn pending(tenant: &str, v: &str, at: i64) -> String {
        format!(
            "INSERT INTO vcon.pending_write (tenant, version, started_at) \
             VALUES ('\\x{tenant}', '\\x{v}', {at})"
        )
    }

    async fn run(client: &tokio_postgres::Client, scope: &str, sql: &str) {
        in_scope(client, scope, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A database at version 13.
    async fn at_version_13() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 13"),
            Migrated { from: 0, to: 13 },
            "the first thirteen migrations alone"
        );
        db
    }

    /// The statement that makes the lock row of `tenant` (hex).
    fn lock_row(tenant: &str) -> String {
        format!("INSERT INTO vcon.tenant_lock (tenant) VALUES ('\\x{tenant}')")
    }

    /// The mark of the scope's lock row, as text.
    const MARK: &str = "SELECT removing::text FROM vcon.tenant_lock";

    // What must work.

    #[tokio::test]
    async fn a_lock_row_of_version_13_is_kept_and_not_marked() {
        let db = at_version_13().await;
        run(&owner(&db).await, TENANT_A, &lock_row(TENANT_A)).await;
        db.migrate().await;
        assert_eq!(
            one(&db.app().await, TENANT_A, MARK).await,
            "false",
            "the row is kept, and no tenant is being removed"
        );
    }

    #[tokio::test]
    async fn a_new_lock_row_is_not_marked() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &lock_row(TENANT_A)).await;
        assert_eq!(one(&app, TENANT_A, MARK).await, "false", "by default");
    }

    #[tokio::test]
    async fn the_application_role_marks_its_tenant_s_lock_row() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &lock_row(TENANT_A)).await;
        run(
            &app,
            TENANT_A,
            "UPDATE vcon.tenant_lock SET removing = true",
        )
        .await;
        assert_eq!(one(&app, TENANT_A, MARK).await, "true", "marked");
    }

    #[tokio::test]
    async fn a_version_13_database_is_brought_to_version_14() {
        let db = at_version_13().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..=THIS])
                .await
                .expect("version 14"),
            Migrated { from: 13, to: 14 },
            "the fourteenth migration alone"
        );
    }

    #[tokio::test]
    async fn a_migrated_database_holds_no_pending_row() {
        let db = Db::migrated().await;
        assert_eq!(one(&db.app().await, TENANT_A, ROWS).await, "", "empty");
    }

    #[tokio::test]
    async fn the_application_role_adds_reads_and_deletes_a_tenant_s_rows() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &pending(TENANT_A, "01", 7)).await;
        run(&app, TENANT_A, &pending(TENANT_A, "02", -7)).await;
        assert_eq!(
            one(&app, TENANT_A, ROWS).await,
            "01:7,02:-7",
            "both rows, with their times"
        );
        run(
            &app,
            TENANT_A,
            "DELETE FROM vcon.pending_write WHERE version = '\\x01'",
        )
        .await;
        assert_eq!(one(&app, TENANT_A, ROWS).await, "02:-7", "one is left");
    }

    #[tokio::test]
    async fn the_rows_of_version_13_are_as_they_were() {
        let db = at_version_13().await;
        run(
            &owner(&db).await,
            TENANT_A,
            &format!(
                "INSERT INTO vcon.key_use (tenant, tenant_key_id, wraps) \
                 VALUES ('\\x{TENANT_A}', '\\xc1', 3)"
            ),
        )
        .await;
        db.migrate().await;
        assert_eq!(
            one(
                &db.app().await,
                TENANT_A,
                "SELECT wraps FROM vcon.key_use WHERE tenant_key_id = '\\xc1'"
            )
            .await,
            "3",
            "a row of another table"
        );
    }

    // What must be refused.

    #[tokio::test]
    async fn the_application_role_cannot_change_a_pending_row() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &pending(TENANT_A, "01", 7)).await;
        for sql in [
            "UPDATE vcon.pending_write SET started_at = 8",
            "TRUNCATE vcon.pending_write",
        ] {
            assert_eq!(
                refused(&app, TENANT_A, sql).await,
                "permission denied for table pending_write",
                "{sql}"
            );
        }
        assert_eq!(
            one(&app, TENANT_A, ROWS).await,
            "01:7",
            "the row is as made"
        );
    }

    #[tokio::test]
    async fn a_second_row_for_one_version_is_refused() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &pending(TENANT_A, "01", 7)).await;
        let why = refused(&app, TENANT_A, &pending(TENANT_A, "01", 8)).await;
        assert!(why.contains("duplicate key"), "the key refuses it: {why}");
    }

    #[tokio::test]
    async fn a_tenant_cannot_mark_another_tenant_s_lock_row() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &lock_row(TENANT_A)).await;
        run(
            &app,
            TENANT_B,
            "UPDATE vcon.tenant_lock SET removing = true",
        )
        .await;
        assert_eq!(
            one(&app, TENANT_A, MARK).await,
            "false",
            "A's row is not marked"
        );
    }

    #[tokio::test]
    async fn a_lock_row_cannot_be_marked_null() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &lock_row(TENANT_A)).await;
        assert!(
            in_scope(
                &app,
                TENANT_A,
                "UPDATE vcon.tenant_lock SET removing = NULL"
            )
            .await
            .is_err(),
            "the mark is true or false"
        );
        assert_eq!(one(&app, TENANT_A, MARK).await, "false", "and stays false");
    }

    #[tokio::test]
    async fn a_tenant_neither_reads_nor_deletes_another_tenant_s_rows() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &pending(TENANT_A, "01", 7)).await;
        assert_eq!(one(&app, TENANT_B, ROWS).await, "", "B reads none of A's");
        run(&app, TENANT_B, "DELETE FROM vcon.pending_write").await;
        assert_eq!(one(&app, TENANT_A, ROWS).await, "01:7", "A's row is kept");
    }

    #[tokio::test]
    async fn a_tenant_cannot_add_another_tenant_s_row() {
        let db = Db::migrated().await;
        let app = db.app().await;
        let why = refused(&app, TENANT_B, &pending(TENANT_A, "01", 7)).await;
        assert!(
            why.contains("row-level security"),
            "the policy refuses it: {why}"
        );
    }

    #[tokio::test]
    async fn a_statement_with_no_tenant_is_refused() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for sql in [
            "SELECT * FROM vcon.pending_write".to_owned(),
            "DELETE FROM vcon.pending_write".to_owned(),
            pending(TENANT_A, "01", 7),
        ] {
            let why = no_scope(&app, &sql).await.expect_err("refused");
            assert!(why.contains(NO_TENANT), "{sql}: {why}");
        }
    }

    // The edges.

    #[tokio::test]
    async fn a_time_at_either_end_of_the_range_is_kept() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &pending(TENANT_A, "01", i64::MIN)).await;
        run(&app, TENANT_A, &pending(TENANT_A, "02", i64::MAX)).await;
        assert_eq!(
            one(&app, TENANT_A, ROWS).await,
            format!("01:{},02:{}", i64::MIN, i64::MAX),
            "both ends of the range"
        );
    }

    #[tokio::test]
    async fn two_tenants_may_each_have_a_row_for_one_version_id() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &pending(TENANT_A, "01", 1)).await;
        run(&app, TENANT_B, &pending(TENANT_B, "01", 2)).await;
        assert_eq!(one(&app, TENANT_A, ROWS).await, "01:1", "A's");
        assert_eq!(one(&app, TENANT_B, ROWS).await, "01:2", "B's");
    }

    #[tokio::test]
    async fn migrate_on_a_database_at_this_binary_s_version_keeps_the_rows() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &pending(TENANT_A, "01", 7)).await;
        db.migrate().await;
        assert_eq!(one(&app, TENANT_A, ROWS).await, "01:7", "kept");
    }
}
