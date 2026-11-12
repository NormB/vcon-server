//! What must work.

use super::super::super::schema::{
    COMMANDS_LOCK, MIGRATIONS, SCHEMA_VERSION, SERVERS_LOCK, role_refusal, version_refusal,
};

#[test]
fn the_schema_version_is_the_number_of_migrations() {
    assert_eq!(
        usize::try_from(SCHEMA_VERSION).expect("positive"),
        MIGRATIONS.len(),
        "the binary's version is its last migration's"
    );
}

#[test]
fn the_binarys_own_version_is_not_refused() {
    assert_eq!(
        version_refusal(Some(SCHEMA_VERSION)),
        None,
        "a connection to this binary's schema is used"
    );
}

#[test]
fn a_role_that_may_act_as_the_owner_is_accepted() {
    assert_eq!(
        role_refusal("vcon_migrate", true, true, true),
        None,
        "both roles exist and SET ROLE is allowed"
    );
}

#[test]
fn the_two_advisory_locks_are_different_numbers() {
    assert_ne!(SERVERS_LOCK, COMMANDS_LOCK, "they lock different things");
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::super::live::value;
    use super::super::live::{
        Db, REGISTRY_TABLES, TENANT_A, TENANT_B, TENANT_TABLES, count, in_scope,
        insert_registry_row, insert_tenant_row,
    };

    #[tokio::test]
    async fn migrate_makes_every_table_and_records_the_version() {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 0,
                to: SCHEMA_VERSION
            },
            "from nothing to this binary's version"
        );
        let admin = db.admin().await;
        for table in TENANT_TABLES
            .iter()
            .chain(REGISTRY_TABLES)
            .chain(["schema_version"].iter())
        {
            assert_eq!(
                value(
                    &admin,
                    &format!("SELECT to_regclass('vcon.{table}') IS NOT NULL")
                )
                .await,
                "t",
                "the table {table} exists"
            );
        }
        assert_eq!(
            count(
                &admin,
                "SELECT count(*) FROM pg_tables WHERE schemaname = 'vcon'"
            )
            .await,
            i64::try_from(TENANT_TABLES.len() + REGISTRY_TABLES.len() + 1).expect("fits"),
            "and no other table does"
        );
        assert_eq!(
            value(&admin, "SELECT max(version) FROM vcon.schema_version").await,
            SCHEMA_VERSION.to_string(),
            "the version is recorded for the migrating role that owns the table"
        );
    }

    #[tokio::test]
    async fn migrate_is_a_no_op_at_this_binarys_version() {
        let db = Db::migrated().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "nothing to do"
        );
        assert_eq!(
            count(
                &db.admin().await,
                "SELECT count(*) FROM vcon.schema_version"
            )
            .await,
            i64::from(SCHEMA_VERSION),
            "one row for each version, and no second row for any"
        );
    }

    #[tokio::test]
    async fn every_object_belongs_to_the_owner_role() {
        let db = Db::migrated().await;
        let admin = db.admin().await;
        assert_eq!(
            count(
                &admin,
                "SELECT count(*) FROM pg_tables WHERE schemaname = 'vcon' AND tableowner <> 'vcon_owner'"
            )
            .await,
            0,
            "every table belongs to vcon_owner, as the role that migrated them"
        );
        assert_eq!(
            value(
                &admin,
                "SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname = 'vcon'"
            )
            .await,
            "vcon_owner",
            "and the schema"
        );
    }

    #[tokio::test]
    async fn the_application_role_connects_once_the_schema_is_this_binarys() {
        let db = Db::migrated().await;
        let connected = db.connect("vcon_app", Purpose::Serve).await;
        assert!(
            connected.is_ok(),
            "the serving checks pass: {:?}",
            connected.err().map(|e| e.to_string())
        );
    }

    #[tokio::test]
    async fn each_table_holds_a_row_of_the_tenant_set() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in TENANT_TABLES {
            in_scope(&app, TENANT_A, &insert_tenant_row(table, TENANT_A))
                .await
                .unwrap_or_else(|e| panic!("{table}: {e}"));
            let rows = in_scope(
                &app,
                TENANT_A,
                &format!("SELECT count(*) FROM vcon.{table}"),
            )
            .await
            .expect("reads");
            let count = rows
                .iter()
                .find_map(|m| match m {
                    tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_owned),
                    _ => None,
                })
                .expect("a count");
            assert_eq!(count, "1", "{table} shows the row just written");
        }
    }

    #[tokio::test]
    async fn a_tenants_rows_are_hidden_from_another_tenant() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_tenant_row("vcon", TENANT_A))
            .await
            .expect("inserts");
        let rows = in_scope(&app, TENANT_B, "SELECT 1 FROM vcon.vcon")
            .await
            .expect("reads");
        assert!(
            !rows
                .iter()
                .any(|m| matches!(m, tokio_postgres::SimpleQueryMessage::Row(_))),
            "tenant B sees none of tenant A's rows"
        );
    }

    #[tokio::test]
    async fn the_registry_transaction_reads_and_writes_the_registry() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in REGISTRY_TABLES {
            in_scope(&app, "registry", &insert_registry_row(table))
                .await
                .unwrap_or_else(|e| panic!("{table}: {e}"));
            let rows = in_scope(&app, "registry", &format!("SELECT 1 FROM vcon.{table}"))
                .await
                .expect("reads");
            assert!(
                rows.iter()
                    .any(|m| matches!(m, tokio_postgres::SimpleQueryMessage::Row(_))),
                "{table} shows its row"
            );
        }
    }

    #[tokio::test]
    async fn the_count_of_an_audit_row_can_be_updated() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_tenant_row("audit", TENANT_A))
            .await
            .expect("inserts");
        in_scope(&app, TENANT_A, "UPDATE vcon.audit SET count = count + 1")
            .await
            .expect("the column privilege on count allows this");
        in_scope(&app, "registry", &insert_registry_row("admin_audit"))
            .await
            .expect("inserts");
        in_scope(
            &app,
            "registry",
            "UPDATE vcon.admin_audit SET count = count + 1",
        )
        .await
        .expect("the admin log's count too");
    }

    #[tokio::test]
    async fn every_table_has_forced_row_level_security_and_one_policy() {
        let db = Db::migrated().await;
        let admin = db.admin().await;
        let tables: Vec<&str> = TENANT_TABLES
            .iter()
            .chain(REGISTRY_TABLES)
            .copied()
            .collect();
        for table in &tables {
            let sql = format!(
                "SELECT c.relrowsecurity AND c.relforcerowsecurity FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = 'vcon' AND c.relname = '{table}'"
            );
            assert_eq!(
                value(&admin, &sql).await,
                "t",
                "{table} has ENABLE and FORCE row-level security"
            );
            assert_eq!(
                count(
                    &admin,
                    &format!(
                        "SELECT count(*) FROM pg_policies WHERE schemaname = 'vcon' AND tablename = '{table}'"
                    )
                )
                .await,
                1,
                "{table} has one policy"
            );
        }
    }

    #[tokio::test]
    async fn a_later_migration_is_applied_after_an_earlier_one() {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&[MIGRATIONS[0]]).await.expect("migrates"),
            Migrated { from: 0, to: 1 },
            "the first"
        );
        let v2 = "CREATE TABLE vcon.extra (tenant bytea PRIMARY KEY)";
        assert_eq!(
            db.migrate_with(&[MIGRATIONS[0], v2])
                .await
                .expect("migrates"),
            Migrated { from: 1, to: 2 },
            "then only the second"
        );
        let admin = db.admin().await;
        assert_eq!(
            value(&admin, "SELECT to_regclass('vcon.extra') IS NOT NULL").await,
            "t",
            "the second migration's table exists"
        );
        assert_eq!(
            count(&admin, "SELECT count(*) FROM vcon.schema_version").await,
            2,
            "each version has its row"
        );
    }

    #[tokio::test]
    async fn both_migrations_of_a_fresh_database_apply_in_one_run() {
        let db = Db::fresh().await;
        let v2 = "CREATE TABLE vcon.extra (tenant bytea PRIMARY KEY)";
        assert_eq!(
            db.migrate_with(&[MIGRATIONS[0], v2])
                .await
                .expect("migrates"),
            Migrated { from: 0, to: 2 },
            "both"
        );
        let admin = db.admin().await;
        assert_eq!(
            count(&admin, "SELECT count(*) FROM vcon.schema_version").await,
            2,
            "both recorded"
        );
    }

    #[tokio::test]
    async fn a_command_reports_what_it_migrated() {
        let db = Db::fresh().await;
        let (_dir, settings) = db.settings("vcon_migrate");
        let report = super::super::super::super::migrate(&settings)
            .await
            .expect("migrates");
        let now = SCHEMA_VERSION;
        assert_eq!(report, Migrated { from: 0, to: now }, "the versions");
        assert_eq!(
            report.message(),
            format!("schema version 0 to schema version {now}"),
            "the line it prints"
        );
        let (_dir, settings) = db.settings("vcon_migrate");
        let again = super::super::super::super::migrate(&settings)
            .await
            .expect("no change");
        assert_eq!(
            again.message(),
            format!("already at schema version {now}"),
            "a run at the same version says so"
        );
    }

    #[tokio::test]
    async fn a_refused_migrate_leaves_the_locks_free_for_the_next() {
        let db = Db::migrated().await;
        let holder = db.admin().await;
        holder
            .simple_query(&format!(
                "SELECT pg_advisory_lock({})",
                super::super::super::super::schema::SERVERS_LOCK
            ))
            .await
            .expect("holds the lock");
        assert!(
            db.migrate_with(MIGRATIONS).await.is_err(),
            "refused while it is held"
        );
        holder
            .simple_query("SELECT pg_advisory_unlock_all()")
            .await
            .expect("releases");
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "works once it is free"
        );
    }
}
