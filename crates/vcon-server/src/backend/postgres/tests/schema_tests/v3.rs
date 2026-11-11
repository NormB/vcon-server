//! Schema version 3: a tenant's role cap, the column `role_cap_sealed` of
//! the registry's tenant table. Each live test starts from a database at
//! version 2, as a build of `main` before the role cap left one.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

#[test]
fn version_3_is_the_third_migration_and_this_binarys() {
    assert_eq!(MIGRATIONS.len(), 3, "the migrations");
    assert_eq!(SCHEMA_VERSION, 3, "this binary's version");
    assert!(
        MIGRATIONS[2].contains("ALTER TABLE vcon.tenant ADD COLUMN role_cap_sealed bytea;"),
        "the third adds the column, which takes NULL"
    );
}

#[test]
fn a_version_2_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(2)).expect("refused");
    assert!(
        why.contains("version 2, older than this binary's 3")
            && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn version_3_changes_no_row_and_makes_no_table() {
    // The owner role is held to row-level security too, so an UPDATE or
    // DELETE in a migration would raise. A statement begins its line.
    for statement in [
        "UPDATE ",
        "DELETE ",
        "DROP ",
        "TRUNCATE ",
        "CREATE ",
        "GRANT ",
    ] {
        assert!(
            !MIGRATIONS[2]
                .lines()
                .any(|line| line.trim_start().to_uppercase().starts_with(statement)),
            "no {statement}in version 3"
        );
    }
}

#[test]
fn version_3_is_one_statement() {
    let statements: Vec<&str> = MIGRATIONS[2]
        .lines()
        .filter(|line| !line.trim_start().starts_with("--") && !line.trim().is_empty())
        .collect();
    assert_eq!(
        statements,
        ["ALTER TABLE vcon.tenant ADD COLUMN role_cap_sealed bytea;"],
        "the whole migration, besides its comments"
    );
}

#[test]
fn a_database_newer_than_version_3_is_refused_as_newer() {
    let why = version_refusal(Some(4)).expect("refused");
    assert!(
        why.contains("version 4") && why.contains("upgrade this server"),
        "a newer database: {why}"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{MIGRATIONS, Migrated};
    use super::super::super::live::value;
    use super::super::live::{Db, TENANT_A, count, in_scope, insert_registry_row};

    /// A database at version 2.
    async fn at_version_2() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..2]).await.expect("version 2"),
            Migrated { from: 0, to: 2 },
            "the first two migrations alone"
        );
        db
    }

    async fn registry(db: &Db, statement: &str) -> Result<(), String> {
        in_scope(&db.app().await, "registry", statement)
            .await
            .map(|_| ())
    }

    /// The one value a registry transaction of the application role reads.
    async fn read(db: &Db, sql: &str) -> Option<String> {
        in_scope(&db.app().await, "registry", sql)
            .await
            .expect("reads")
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => Some(r.get(0).map(str::to_owned)),
                _ => None,
            })
            .expect("a row")
    }

    const COLUMN: &str = "SELECT count(*) FROM information_schema.columns WHERE \
        table_schema = 'vcon' AND table_name = 'tenant' AND column_name = 'role_cap_sealed'";

    // What must work.

    #[tokio::test]
    async fn a_version_2_database_is_brought_to_version_3_with_its_tenants() {
        let db = at_version_2().await;
        in_scope(
            &db.admin().await,
            "registry",
            &insert_registry_row("tenant"),
        )
        .await
        .expect("a tenant, written before the upgrade");
        assert_eq!(
            db.migrate().await,
            Migrated { from: 2, to: 3 },
            "only the third migration runs"
        );
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.tenant")
                .await
                .as_deref(),
            Some("1"),
            "the tenant kept its row"
        );
        assert_eq!(
            read(&db, "SELECT role_cap_sealed FROM vcon.tenant").await,
            None,
            "and has no cap: NULL"
        );
    }

    #[tokio::test]
    async fn the_column_is_made_by_version_3_and_not_before() {
        let db = at_version_2().await;
        assert_eq!(count(&db.admin().await, COLUMN).await, 0, "at version 2");
        db.migrate().await;
        assert_eq!(count(&db.admin().await, COLUMN).await, 1, "at version 3");
    }

    #[tokio::test]
    async fn the_application_role_sets_reads_and_removes_a_cap() {
        let db = Db::migrated().await;
        registry(&db, &insert_registry_row("tenant"))
            .await
            .expect("a tenant");
        registry(&db, "UPDATE vcon.tenant SET role_cap_sealed = '\\xc0ffee'")
            .await
            .expect("the cap is set");
        assert_eq!(
            read(
                &db,
                "SELECT encode(role_cap_sealed, 'hex') FROM vcon.tenant"
            )
            .await
            .as_deref(),
            Some("c0ffee"),
            "read back"
        );
        registry(&db, "UPDATE vcon.tenant SET role_cap_sealed = NULL")
            .await
            .expect("the cap is removed");
        assert_eq!(
            read(&db, "SELECT role_cap_sealed FROM vcon.tenant").await,
            None,
            "NULL again"
        );
    }

    #[tokio::test]
    async fn a_serving_connection_is_accepted_once_version_3_is_there() {
        let db = at_version_2().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_err(),
            "refused at version 2"
        );
        db.migrate().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_ok(),
            "accepted at version 3"
        );
    }

    #[tokio::test]
    async fn a_new_database_reaches_version_3_with_the_column() {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate().await,
            Migrated { from: 0, to: 3 },
            "all three migrations"
        );
        assert_eq!(count(&db.admin().await, COLUMN).await, 1, "the column");
    }

    // What must be refused.

    #[tokio::test]
    async fn a_version_3_that_fails_leaves_version_2_whole() {
        let db = at_version_2().await;
        db.admin()
            .await
            .batch_execute(
                "SET ROLE vcon_owner; ALTER TABLE vcon.tenant ADD COLUMN role_cap_sealed int; \
                 RESET ROLE",
            )
            .await
            .expect("a column in the way");
        let refused = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(
            refused.contains("version 3"),
            "names the version: {refused}"
        );
        assert_eq!(
            value(
                &db.admin().await,
                "SELECT max(version) FROM vcon.schema_version"
            )
            .await,
            "2",
            "still version 2"
        );
    }

    #[tokio::test]
    async fn a_tenants_transaction_reads_and_writes_no_cap() {
        let db = Db::migrated().await;
        registry(&db, &insert_registry_row("tenant"))
            .await
            .expect("a tenant");
        let app = db.app().await;
        assert!(
            in_scope(&app, TENANT_A, "SELECT role_cap_sealed FROM vcon.tenant")
                .await
                .is_err(),
            "a tenant's transaction is refused the registry's table"
        );
        assert!(
            in_scope(
                &app,
                TENANT_A,
                "UPDATE vcon.tenant SET role_cap_sealed = NULL"
            )
            .await
            .is_err(),
            "a tenant cannot remove its own cap"
        );
    }

    #[tokio::test]
    async fn the_application_role_cannot_drop_or_retype_the_column() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for statement in [
            "ALTER TABLE vcon.tenant DROP COLUMN role_cap_sealed",
            "ALTER TABLE vcon.tenant ALTER COLUMN role_cap_sealed TYPE text",
            "ALTER TABLE vcon.tenant ALTER COLUMN role_cap_sealed SET NOT NULL",
        ] {
            assert!(
                in_scope(&app, "registry", statement).await.is_err(),
                "{statement} is refused"
            );
        }
    }

    #[tokio::test]
    async fn the_column_takes_bytes_and_not_text_that_is_no_bytea() {
        let db = Db::migrated().await;
        registry(&db, &insert_registry_row("tenant"))
            .await
            .expect("a tenant");
        assert!(
            registry(&db, "UPDATE vcon.tenant SET role_cap_sealed = 7")
                .await
                .is_err(),
            "a number is refused: the cap is kept sealed, as bytes"
        );
    }

    // Limits and odd inputs.

    #[tokio::test]
    async fn migrating_twice_from_version_2_does_the_work_once() {
        let db = at_version_2().await;
        db.migrate().await;
        assert_eq!(
            db.migrate().await,
            Migrated { from: 3, to: 3 },
            "nothing left to do"
        );
        assert_eq!(
            count(
                &db.admin().await,
                "SELECT count(*) FROM vcon.schema_version"
            )
            .await,
            3,
            "one row for each version"
        );
    }

    #[tokio::test]
    async fn a_version_2_database_with_no_tenant_gains_the_column() {
        let db = at_version_2().await;
        db.migrate().await;
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.tenant")
                .await
                .as_deref(),
            Some("0"),
            "no tenant"
        );
        assert_eq!(count(&db.admin().await, COLUMN).await, 1, "the column");
    }

    #[tokio::test]
    async fn a_tenant_made_after_the_upgrade_has_no_cap_unless_given_one() {
        let db = at_version_2().await;
        db.migrate().await;
        registry(&db, &insert_registry_row("tenant"))
            .await
            .expect("a tenant, written as version 2 wrote one");
        assert_eq!(
            read(&db, "SELECT role_cap_sealed FROM vcon.tenant").await,
            None,
            "the column has no default but NULL"
        );
    }

    #[tokio::test]
    async fn an_empty_sealed_value_is_not_no_cap() {
        let db = Db::migrated().await;
        registry(&db, &insert_registry_row("tenant"))
            .await
            .expect("a tenant");
        registry(&db, "UPDATE vcon.tenant SET role_cap_sealed = '\\x'")
            .await
            .expect("empty bytes are stored");
        assert_eq!(
            read(
                &db,
                "SELECT (role_cap_sealed IS NOT NULL)::text FROM vcon.tenant"
            )
            .await
            .as_deref(),
            Some("true"),
            "empty is a value; only NULL is no cap"
        );
    }
}
