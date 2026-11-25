//! Schema version 15: the registry records its tenant creates under way,
//! in the table `pending_tenant`, so recovery keeps the space of a create
//! that has not yet added its tenant's row (store/pending.rs). Each live
//! test of the migration starts from a database at version 14.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 14;

/// This version's migration, or a panic that says there is none.
fn text() -> &'static str {
    assert!(
        MIGRATIONS.len() > THIS,
        "there are {} migrations",
        MIGRATIONS.len()
    );
    MIGRATIONS[THIS]
}

/// The migration's statements as one line, every run of white space one
/// space.
fn one_line() -> String {
    text()
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join(" ")
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
fn version_15_makes_the_pending_tenant_table_once() {
    assert_eq!(
        text().matches("CREATE TABLE vcon.pending_tenant (").count(),
        1,
        "the fifteenth makes pending_tenant once"
    );
}

#[test]
fn the_binary_s_version_is_at_least_15() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn a_pending_tenant_row_is_an_id_and_the_time_its_create_began() {
    assert!(
        one_line().contains(
            "CREATE TABLE vcon.pending_tenant ( id bytea PRIMARY KEY, started_at bigint NOT NULL );"
        ),
        "the columns and the key: {}",
        one_line()
    );
}

#[test]
fn the_pending_tenant_table_has_the_policy_of_the_registry_s_rows() {
    let line = one_line();
    for statement in [
        "ALTER TABLE vcon.pending_tenant ENABLE ROW LEVEL SECURITY;",
        "ALTER TABLE vcon.pending_tenant FORCE ROW LEVEL SECURITY;",
        "CREATE POLICY registry_rows ON vcon.pending_tenant USING (vcon.in_registry() = (id IS \
         NOT NULL)) WITH CHECK (vcon.in_registry());",
        "GRANT SELECT, INSERT, DELETE ON vcon.pending_tenant TO vcon_app;",
    ] {
        assert!(line.contains(statement), "{statement}: {line}");
    }
}

#[test]
fn the_migration_says_why_the_pending_tenant_table_starts_empty() {
    assert!(
        comment().contains(
            "The table starts empty: migrate holds the database's lock, which a serving server \
             holds too, so no create is under way while it runs."
        ),
        "the reason, in the migration's comment: {}",
        comment()
    );
}

// What must be refused.

#[test]
fn a_version_14_database_is_refused_with_the_command_that_upgrades_it() {
    let refusal = version_refusal(Some(14)).expect("refused");
    assert!(refusal.contains("vcon-server migrate"), "{refusal}");
}

#[test]
fn the_application_role_is_granted_no_update_on_pending_tenant() {
    assert!(
        !one_line().contains("UPDATE ON vcon.pending_tenant") && !one_line().contains("TRUNCATE"),
        "a create begins once: {}",
        one_line()
    );
}

/// Live, against the servers of scripts/pg-test-server.sh.
#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::schema::{MIGRATIONS, Migrated};
    use super::super::live::{Db, TENANT_A, in_scope, no_scope};
    use super::super::v8::on_a_server::{one, owner, refused};
    use super::THIS;

    /// The creating rows the registry reads: each id and its time.
    const ROWS: &str = "SELECT coalesce(string_agg(encode(id, 'hex') || ':' || started_at, ',' \
                        ORDER BY id), '') FROM vcon.pending_tenant";

    fn pending(id: &str, at: i64) -> String {
        format!("INSERT INTO vcon.pending_tenant (id, started_at) VALUES ('\\x{id}', {at})")
    }

    async fn run(client: &tokio_postgres::Client, scope: &str, sql: &str) {
        in_scope(client, scope, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    #[tokio::test]
    async fn a_version_14_database_is_brought_to_version_15() {
        let db = Db::fresh().await;
        db.migrate_with(&MIGRATIONS[..THIS])
            .await
            .expect("version 14");
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..=THIS])
                .await
                .expect("version 15"),
            Migrated { from: 14, to: 15 },
            "the fifteenth migration alone"
        );
        assert_eq!(one(&db.app().await, "registry", ROWS).await, "", "empty");
    }

    #[tokio::test]
    async fn the_application_role_adds_reads_and_deletes_creating_rows() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, "registry", &pending("01", 7)).await;
        run(&app, "registry", &pending("02", -7)).await;
        assert_eq!(one(&app, "registry", ROWS).await, "01:7,02:-7", "both rows");
        run(
            &app,
            "registry",
            "DELETE FROM vcon.pending_tenant WHERE id = '\\x01'",
        )
        .await;
        assert_eq!(one(&app, "registry", ROWS).await, "02:-7", "one is left");
    }

    #[tokio::test]
    async fn the_application_role_cannot_change_a_creating_row() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, "registry", &pending("01", 7)).await;
        for sql in [
            "UPDATE vcon.pending_tenant SET started_at = 8",
            "TRUNCATE vcon.pending_tenant",
        ] {
            assert_eq!(
                refused(&app, "registry", sql).await,
                "permission denied for table pending_tenant",
                "{sql}"
            );
        }
    }

    #[tokio::test]
    async fn a_delete_with_no_scope_is_refused_and_keeps_the_rows() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, "registry", &pending("01", 7)).await;
        assert_eq!(
            no_scope(&app, "DELETE FROM vcon.pending_tenant").await,
            Err("no tenant set for this transaction".to_owned()),
            "a statement outside the registry's transactions"
        );
        assert_eq!(one(&app, "registry", ROWS).await, "01:7", "the row is kept");
    }

    #[tokio::test]
    async fn a_tenant_s_transaction_reads_and_adds_no_creating_row() {
        let db = Db::migrated().await;
        run(&owner(&db).await, "registry", &pending("01", 7)).await;
        let app = db.app().await;
        for sql in [
            "SELECT count(*) FROM vcon.pending_tenant".to_owned(),
            pending("02", 1),
            "DELETE FROM vcon.pending_tenant".to_owned(),
        ] {
            assert_eq!(
                refused(&app, TENANT_A, &sql).await,
                "this transaction is a tenant's, not the registry's",
                "{sql}"
            );
        }
        assert_eq!(
            one(&app, "registry", ROWS).await,
            "01:7",
            "the registry's row is kept"
        );
    }
}
