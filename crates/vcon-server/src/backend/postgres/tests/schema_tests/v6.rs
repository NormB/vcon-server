//! Schema version 6: an access key's expiry and last-used time, the
//! columns `expires_at` and `last_used_at` of the registry's key table.
//! Each live test starts from a database at version 5, as a build of
//! `main` before key expiry left one.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 5;

#[test]
fn version_6_is_the_sixth_migration() {
    assert!(
        MIGRATIONS.len() > THIS,
        "there are {} migrations",
        MIGRATIONS.len()
    );
    for column in ["expires_at", "last_used_at"] {
        assert!(
            MIGRATIONS[THIS].contains(&format!(
                "ALTER TABLE vcon.api_key ADD COLUMN {column} bigint;"
            )),
            "the sixth adds {column}, which takes NULL"
        );
    }
}

#[test]
fn a_version_5_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(5)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 5, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn version_6_changes_no_row_and_makes_no_table() {
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
            !MIGRATIONS[THIS]
                .lines()
                .any(|line| line.trim_start().to_uppercase().starts_with(statement)),
            "no {statement}in version 6"
        );
    }
}

#[test]
fn version_6_is_two_statements_with_no_default() {
    let statements: Vec<&str> = MIGRATIONS[THIS]
        .lines()
        .filter(|line| !line.trim_start().starts_with("--") && !line.trim().is_empty())
        .collect();
    assert_eq!(
        statements,
        [
            "ALTER TABLE vcon.api_key ADD COLUMN expires_at bigint;",
            "ALTER TABLE vcon.api_key ADD COLUMN last_used_at bigint;"
        ],
        "the whole migration, besides its comments: no default, so a key there is has no expiry"
    );
}

#[test]
fn the_binary_s_version_is_at_least_6() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::super::live::value;
    use super::super::live::{Db, TENANT_A, count, in_scope, insert_registry_row};
    use super::THIS;

    /// A database at version 5.
    async fn at_version_5() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 5"),
            Migrated { from: 0, to: 5 },
            "the first five migrations alone"
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

    const COLUMNS: &str = "SELECT count(*) FROM information_schema.columns WHERE \
        table_schema = 'vcon' AND table_name = 'api_key' AND column_name IN \
        ('expires_at', 'last_used_at') AND data_type = 'bigint' AND is_nullable = 'YES'";

    /// The update a server makes for an accepted request: only forward.
    const TOUCH: &str = "UPDATE vcon.api_key SET last_used_at = 600 \
        WHERE id = '\\x01' AND (last_used_at IS NULL OR last_used_at < 600)";

    // What must work.

    #[tokio::test]
    async fn a_version_5_database_is_brought_to_version_6_with_its_keys() {
        let db = at_version_5().await;
        in_scope(
            &db.admin().await,
            "registry",
            &insert_registry_row("api_key"),
        )
        .await
        .expect("a key, written before the upgrade");
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 5,
                to: SCHEMA_VERSION
            },
            "every migration after the fifth runs"
        );
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.api_key")
                .await
                .as_deref(),
            Some("1"),
            "the key kept its row"
        );
        assert_eq!(
            read(&db, "SELECT expires_at FROM vcon.api_key").await,
            None,
            "and has no expiry: NULL"
        );
        assert_eq!(
            read(&db, "SELECT last_used_at FROM vcon.api_key").await,
            None,
            "and no last-used time: NULL"
        );
    }

    #[tokio::test]
    async fn the_columns_are_made_by_version_6_and_not_before() {
        let db = at_version_5().await;
        assert_eq!(count(&db.admin().await, COLUMNS).await, 0, "at version 5");
        db.migrate().await;
        assert_eq!(count(&db.admin().await, COLUMNS).await, 2, "at version 6");
    }

    #[tokio::test]
    async fn the_application_role_writes_and_reads_an_expiry() {
        let db = at_version_5().await;
        db.migrate().await;
        registry(
            &db,
            "INSERT INTO vcon.api_key (id, token_hash, scopes_sealed, label_sealed, created_at, \
             expires_at) VALUES ('\\x01', '\\x02', '\\x03', '\\x04', 1, 9000)",
        )
        .await
        .expect("a key with an expiry");
        assert_eq!(
            read(&db, "SELECT expires_at FROM vcon.api_key")
                .await
                .as_deref(),
            Some("9000"),
            "read back"
        );
    }

    #[tokio::test]
    async fn the_application_role_moves_a_last_used_time_forward() {
        let db = at_version_5().await;
        db.migrate().await;
        registry(&db, &insert_registry_row("api_key"))
            .await
            .expect("a key");
        registry(&db, TOUCH).await.expect("the first use");
        assert_eq!(
            read(&db, "SELECT last_used_at FROM vcon.api_key")
                .await
                .as_deref(),
            Some("600"),
            "written"
        );
    }

    #[tokio::test]
    async fn a_serving_connection_is_accepted_once_version_6_is_there() {
        let db = at_version_5().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_err(),
            "refused at version 5"
        );
        db.migrate().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_ok(),
            "accepted at this binary's version"
        );
    }

    #[tokio::test]
    async fn a_new_database_has_the_columns() {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 0,
                to: SCHEMA_VERSION
            },
            "every migration"
        );
        assert_eq!(count(&db.admin().await, COLUMNS).await, 2, "both columns");
    }

    // What must be refused.

    #[tokio::test]
    async fn a_version_6_that_fails_leaves_version_5_whole() {
        let db = at_version_5().await;
        db.admin()
            .await
            .batch_execute(
                "SET ROLE vcon_owner; ALTER TABLE vcon.api_key ADD COLUMN last_used_at int; \
                 RESET ROLE",
            )
            .await
            .expect("a column in the way of the second statement");
        let refused = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(
            refused.contains("version 6"),
            "names the version: {refused}"
        );
        assert_eq!(
            value(
                &db.admin().await,
                "SELECT max(version) FROM vcon.schema_version"
            )
            .await,
            "5",
            "still version 5"
        );
        assert_eq!(
            count(
                &db.admin().await,
                "SELECT count(*) FROM information_schema.columns WHERE table_schema = 'vcon' \
                 AND table_name = 'api_key' AND column_name = 'expires_at'"
            )
            .await,
            0,
            "the first statement's column is not kept either"
        );
    }

    #[tokio::test]
    async fn a_tenants_transaction_reads_and_writes_no_key_time() {
        let db = at_version_5().await;
        db.migrate().await;
        registry(&db, &insert_registry_row("api_key"))
            .await
            .expect("a key");
        let tenant = db.app().await;
        // The registry's tables raise for a tenant's transaction, as
        // version 1 made them.
        for statement in [
            "SELECT expires_at, last_used_at FROM vcon.api_key",
            "UPDATE vcon.api_key SET expires_at = 1, last_used_at = 1",
        ] {
            assert!(
                in_scope(&tenant, TENANT_A, statement).await.is_err(),
                "refused in a tenant's transaction: {statement}"
            );
        }
        assert_eq!(
            read(&db, "SELECT expires_at FROM vcon.api_key").await,
            None,
            "the key's expiry is as it was"
        );
    }

    #[tokio::test]
    async fn the_application_role_cannot_drop_or_retype_the_columns() {
        let db = at_version_5().await;
        db.migrate().await;
        for statement in [
            "ALTER TABLE vcon.api_key DROP COLUMN expires_at",
            "ALTER TABLE vcon.api_key DROP COLUMN last_used_at",
            "ALTER TABLE vcon.api_key ALTER COLUMN expires_at TYPE text",
        ] {
            assert!(
                registry(&db, statement).await.is_err(),
                "refused: {statement}"
            );
        }
    }

    #[tokio::test]
    async fn the_columns_take_no_text() {
        let db = at_version_5().await;
        db.migrate().await;
        registry(&db, &insert_registry_row("api_key"))
            .await
            .expect("a key");
        for column in ["expires_at", "last_used_at"] {
            assert!(
                registry(
                    &db,
                    &format!("UPDATE vcon.api_key SET {column} = 'tomorrow'")
                )
                .await
                .is_err(),
                "{column} is a number"
            );
        }
    }

    // The edges.

    #[tokio::test]
    async fn migrating_twice_from_version_5_does_the_work_once() {
        let db = at_version_5().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 5,
                to: SCHEMA_VERSION
            },
            "the first"
        );
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "the second finds nothing to do"
        );
        assert_eq!(count(&db.admin().await, COLUMNS).await, 2, "two columns");
    }

    #[tokio::test]
    async fn a_version_5_database_with_no_key_gains_the_columns() {
        let db = at_version_5().await;
        db.migrate().await;
        assert_eq!(count(&db.admin().await, COLUMNS).await, 2, "two columns");
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.api_key")
                .await
                .as_deref(),
            Some("0"),
            "and no key"
        );
    }

    #[tokio::test]
    async fn a_last_used_time_does_not_move_back_or_repeat() {
        let db = at_version_5().await;
        db.migrate().await;
        registry(&db, &insert_registry_row("api_key"))
            .await
            .expect("a key");
        registry(
            &db,
            "UPDATE vcon.api_key SET last_used_at = 1200 WHERE id = '\\x01'",
        )
        .await
        .expect("a later use, written first");
        registry(&db, TOUCH)
            .await
            .expect("an earlier use changes no row");
        assert_eq!(
            read(&db, "SELECT last_used_at FROM vcon.api_key")
                .await
                .as_deref(),
            Some("1200"),
            "the later time is kept"
        );
    }

    #[tokio::test]
    async fn a_key_made_after_the_upgrade_has_no_expiry_unless_given_one() {
        let db = at_version_5().await;
        db.migrate().await;
        registry(&db, &insert_registry_row("api_key"))
            .await
            .expect("a key with no expiry named");
        assert_eq!(
            read(&db, "SELECT expires_at FROM vcon.api_key").await,
            None,
            "the column has no default: the server names every expiry it gives"
        );
    }
}
