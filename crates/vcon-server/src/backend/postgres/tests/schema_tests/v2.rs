//! Schema version 2: the registry's users and memberships, and audit
//! logs that hold no status. Each test starts from a database at version
//! 1, as the first preview release left one.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

#[test]
fn version_2_is_the_second_migration() {
    assert!(MIGRATIONS.get(1).is_some(), "the second migration");
    assert!(
        MIGRATIONS[1].contains("CREATE TABLE vcon.user_account")
            && MIGRATIONS[1].contains("CREATE TABLE vcon.membership"),
        "the second makes the two tables"
    );
}

#[test]
fn a_version_1_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(1)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 1, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn version_2_changes_no_row_of_version_1() {
    // The owner role is held to row-level security too, so an UPDATE or
    // DELETE in a migration would raise; and no release wrote a row.
    // A statement begins its line; a GRANT names UPDATE and DELETE within one.
    for statement in ["UPDATE ", "DELETE ", "DROP TABLE ", "TRUNCATE "] {
        assert!(
            !MIGRATIONS[1]
                .lines()
                .any(|line| line.trim_start().to_uppercase().starts_with(statement)),
            "no {statement}in version 2"
        );
    }
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::super::live::value;
    use super::super::live::{
        Db, TENANT_A, TENANT_B, count, in_scope, insert_registry_row, insert_tenant_row,
    };

    /// The tables version 1 made, which a row can be put in before the
    /// upgrade.
    const V1_TENANT: &[&str] = &[
        "audit",
        "key_use",
        "manifest",
        "parquet_file",
        "purge_pending",
        "tenant_catalog",
        "tenant_lock",
        "tombstone",
        "vcon",
        "version",
    ];
    const V1_REGISTRY: &[&str] = &["admin_audit", "api_key", "master_key_use", "tenant"];

    /// A database at version 1.
    async fn at_version_1() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&[MIGRATIONS[0]]).await.expect("version 1"),
            Migrated { from: 0, to: 1 },
            "the first migration alone"
        );
        db
    }

    /// The number of rows of `table` a transaction in `scope` sees.
    async fn rows(client: &tokio_postgres::Client, scope: &str, table: &str) -> usize {
        in_scope(client, scope, &format!("SELECT 1 FROM vcon.{table}"))
            .await
            .unwrap_or_else(|e| panic!("{table}: {e}"))
            .iter()
            .filter(|m| matches!(m, tokio_postgres::SimpleQueryMessage::Row(_)))
            .count()
    }

    async fn registry(db: &Db, statement: &str) -> Result<(), String> {
        in_scope(&db.app().await, "registry", statement)
            .await
            .map(|_| ())
    }

    const USER: &str = "INSERT INTO vcon.user_account \
        (id, label_sealed, identity_mac, identity_sealed, created_at, deactivated_at)";
    const MEMBER: &str = "INSERT INTO vcon.membership (user_id, tenant, roles_sealed, granted_at)";

    // What must work.

    #[tokio::test]
    async fn a_version_1_database_is_brought_to_version_2_with_its_rows() {
        let db = at_version_1().await;
        // The application role wrote the rows, as a server would have.
        {
            let admin = db.admin().await;
            for table in V1_TENANT {
                in_scope(&admin, TENANT_A, &insert_tenant_row(table, TENANT_A))
                    .await
                    .unwrap_or_else(|e| panic!("{table}: {e}"));
            }
            for table in V1_REGISTRY {
                in_scope(&admin, "registry", &insert_registry_row(table))
                    .await
                    .unwrap_or_else(|e| panic!("{table}: {e}"));
            }
        }
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 1,
                to: SCHEMA_VERSION
            },
            "every migration after the first runs"
        );
        let app = db.app().await;
        for table in V1_TENANT {
            assert_eq!(rows(&app, TENANT_A, table).await, 1, "{table} kept its row");
        }
        for table in V1_REGISTRY {
            assert_eq!(
                rows(&app, "registry", table).await,
                1,
                "{table} kept its row"
            );
        }
        assert_eq!(
            value(
                &db.admin().await,
                "SELECT max(version) FROM vcon.schema_version"
            )
            .await,
            SCHEMA_VERSION.to_string(),
            "the version recorded"
        );
    }

    #[tokio::test]
    async fn the_two_tables_are_made_by_version_2_and_not_before() {
        let db = at_version_1().await;
        let there = |table: &'static str| {
            let db = &db;
            async move {
                value(
                    &db.admin().await,
                    &format!("SELECT to_regclass('vcon.{table}') IS NOT NULL"),
                )
                .await
            }
        };
        assert_eq!(there("user_account").await, "f", "not at version 1");
        assert_eq!(there("membership").await, "f", "not at version 1");
        db.migrate().await;
        assert_eq!(there("user_account").await, "t", "at version 2");
        assert_eq!(there("membership").await, "t", "at version 2");
    }

    #[tokio::test]
    async fn both_logs_take_a_record_with_no_status_at_version_2() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(
            &app,
            TENANT_A,
            &format!(
                "INSERT INTO vcon.audit (tenant, at, actor, action, status, count) \
                 VALUES ('\\x{TENANT_A}', 't', '', 'listener_full', NULL, 1)"
            ),
        )
        .await
        .expect("the tenant's log");
        in_scope(
            &app,
            "registry",
            "INSERT INTO vcon.admin_audit (at, actor, action, status, count) \
             VALUES ('t', '', 'listener_full', NULL, 1)",
        )
        .await
        .expect("the admin log");
        assert_eq!(
            rows(&app, TENANT_A, "audit").await,
            1,
            "the tenant's record"
        );
        assert_eq!(
            rows(&app, "registry", "admin_audit").await,
            1,
            "the admin record"
        );
    }

    #[tokio::test]
    async fn users_of_both_kinds_and_their_memberships_are_kept() {
        let db = Db::migrated().await;
        registry(
            &db,
            &format!(
                "{USER} VALUES ('\\x01', '\\xaa', NULL, NULL, 1, NULL); \
                 {USER} VALUES ('\\x02', '\\xbb', '\\xcc', '\\xdd', 2, NULL); \
                 {MEMBER} VALUES ('\\x01', NULL, '\\xee', 3); \
                 {MEMBER} VALUES ('\\x01', '\\x{TENANT_A}', '\\xee', 4); \
                 {MEMBER} VALUES ('\\x01', '\\x{TENANT_B}', '\\xee', 5); \
                 {MEMBER} VALUES ('\\x02', '\\x{TENANT_A}', '\\xee', 6)"
            ),
        )
        .await
        .expect("a local user, a user with an identity, and four memberships");
        let app = db.app().await;
        assert_eq!(rows(&app, "registry", "user_account").await, 2, "the users");
        assert_eq!(
            rows(&app, "registry", "membership").await,
            4,
            "the memberships"
        );
    }

    #[tokio::test]
    async fn a_serving_connection_is_accepted_once_version_2_is_there() {
        let db = at_version_1().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_err(),
            "refused at version 1"
        );
        db.migrate().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_ok(),
            "accepted at this binary's version"
        );
    }

    // What must be refused.

    #[tokio::test]
    async fn the_logs_of_version_1_refuse_a_record_with_no_status() {
        let db = at_version_1().await;
        let refused = in_scope(
            &db.admin().await,
            "registry",
            "INSERT INTO vcon.admin_audit (at, actor, action, status, count) \
             VALUES ('t', '', 'listener_full', NULL, 1)",
        )
        .await
        .expect_err("refused");
        assert!(refused.contains("null value"), "by the column: {refused}");
    }

    #[tokio::test]
    async fn two_active_users_cannot_share_an_identity() {
        let db = Db::migrated().await;
        registry(
            &db,
            &format!("{USER} VALUES ('\\x01', '\\xaa', '\\xcc', '\\xdd', 1, NULL)"),
        )
        .await
        .expect("the first");
        let refused = registry(
            &db,
            &format!("{USER} VALUES ('\\x02', '\\xaa', '\\xcc', '\\xdd', 2, NULL)"),
        )
        .await
        .expect_err("refused");
        assert!(
            refused.contains("user_account_identity"),
            "by the index: {refused}"
        );
    }

    #[tokio::test]
    async fn an_identity_is_its_mac_and_its_sealed_value_or_neither() {
        let db = Db::migrated().await;
        for (mac, sealed) in [("'\\xcc'", "NULL"), ("NULL", "'\\xdd'")] {
            let refused = registry(
                &db,
                &format!("{USER} VALUES ('\\x01', '\\xaa', {mac}, {sealed}, 1, NULL)"),
            )
            .await
            .expect_err("refused");
            assert!(
                refused.contains("check constraint"),
                "{mac}, {sealed}: {refused}"
            );
        }
    }

    #[tokio::test]
    async fn a_user_has_one_membership_in_a_tenant_and_one_as_operator() {
        let db = Db::migrated().await;
        registry(
            &db,
            &format!(
                "{MEMBER} VALUES ('\\x01', NULL, '\\xee', 1); \
                 {MEMBER} VALUES ('\\x01', '\\x{TENANT_A}', '\\xee', 2)"
            ),
        )
        .await
        .expect("one of each");
        for (tenant, index) in [
            ("NULL".to_owned(), "membership_as_operator"),
            (format!("'\\x{TENANT_A}'"), "membership_in_tenant"),
        ] {
            let refused = registry(
                &db,
                &format!("{MEMBER} VALUES ('\\x01', {tenant}, '\\xee', 3)"),
            )
            .await
            .expect_err("refused");
            assert!(refused.contains(index), "by {index}: {refused}");
        }
    }

    #[tokio::test]
    async fn a_tenants_transaction_reads_and_writes_no_user_or_membership() {
        let db = Db::migrated().await;
        registry(
            &db,
            &format!(
                "{USER} VALUES ('\\x01', '\\xaa', NULL, NULL, 1, NULL); \
                 {MEMBER} VALUES ('\\x01', '\\x{TENANT_A}', '\\xee', 2)"
            ),
        )
        .await
        .expect("rows");
        let app = db.app().await;
        for table in ["user_account", "membership"] {
            assert!(
                in_scope(&app, TENANT_A, &format!("SELECT 1 FROM vcon.{table}"))
                    .await
                    .is_err(),
                "{table} is refused to the tenant the membership is in"
            );
        }
        assert!(
            in_scope(
                &app,
                TENANT_A,
                &format!("{MEMBER} VALUES ('\\x02', '\\x{TENANT_A}', '\\xee', 3)")
            )
            .await
            .is_err(),
            "a tenant cannot give itself a member"
        );
    }

    #[tokio::test]
    async fn a_version_2_that_fails_leaves_version_1_whole() {
        let db = at_version_1().await;
        db.admin()
            .await
            .batch_execute(
                "SET ROLE vcon_owner; CREATE TABLE vcon.membership (other int); RESET ROLE",
            )
            .await
            .expect("a table in the way");
        let refused = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(
            refused.contains("version 2"),
            "names the version: {refused}"
        );
        let admin = db.admin().await;
        assert_eq!(
            value(&admin, "SELECT max(version) FROM vcon.schema_version").await,
            "1",
            "still version 1"
        );
        assert_eq!(
            value(&admin, "SELECT to_regclass('vcon.user_account') IS NULL").await,
            "t",
            "the table made before the failure is gone"
        );
        assert_eq!(
            value(
                &admin,
                "SELECT is_nullable FROM information_schema.columns WHERE table_schema = 'vcon' \
                 AND table_name = 'admin_audit' AND column_name = 'status'"
            )
            .await,
            "NO",
            "the log is as version 1 made it"
        );
    }

    // Limits and odd inputs.

    #[tokio::test]
    async fn a_deactivated_users_identity_is_free_for_a_new_user() {
        let db = Db::migrated().await;
        registry(
            &db,
            &format!(
                "{USER} VALUES ('\\x01', '\\xaa', '\\xcc', '\\xdd', 1, 5); \
                 {USER} VALUES ('\\x02', '\\xaa', '\\xcc', '\\xdd', 6, NULL)"
            ),
        )
        .await
        .expect("the identity under a new user");
    }

    #[tokio::test]
    async fn local_users_never_collide_on_an_identity() {
        let db = Db::migrated().await;
        registry(
            &db,
            &format!(
                "{USER} VALUES ('\\x01', '\\xaa', NULL, NULL, 1, NULL); \
                 {USER} VALUES ('\\x02', '\\xaa', NULL, NULL, 2, NULL)"
            ),
        )
        .await
        .expect("two local users");
    }

    #[tokio::test]
    async fn migrating_twice_from_version_1_does_the_work_once() {
        let db = at_version_1().await;
        db.migrate().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "nothing left to do"
        );
        assert_eq!(
            count(
                &db.admin().await,
                "SELECT count(*) FROM vcon.schema_version"
            )
            .await,
            i64::from(SCHEMA_VERSION),
            "one row for each version"
        );
    }

    #[tokio::test]
    async fn the_application_role_cannot_change_the_new_tables_shape() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for statement in [
            "DROP TABLE vcon.user_account",
            "ALTER TABLE vcon.membership ADD COLUMN extra int",
            "ALTER TABLE vcon.user_account DISABLE ROW LEVEL SECURITY",
            "DROP POLICY registry_rows ON vcon.membership",
        ] {
            assert!(
                in_scope(&app, "registry", statement).await.is_err(),
                "{statement} is refused"
            );
        }
    }
}
