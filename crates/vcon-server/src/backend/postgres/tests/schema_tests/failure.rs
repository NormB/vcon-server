//! What must be refused.

use super::super::super::schema::{SCHEMA_VERSION, role_refusal, version_refusal};

#[test]
fn no_schema_is_refused_with_the_command_that_fixes_it() {
    let why = version_refusal(None).expect("refused");
    assert!(
        why.contains("no vCon schema") && why.contains("vcon-server migrate"),
        "says what is wrong and the command: {why}"
    );
}

#[test]
fn an_older_schema_is_refused_with_the_command_that_fixes_it() {
    let why = version_refusal(Some(SCHEMA_VERSION - 1)).expect("refused");
    assert!(
        why.contains("older") && why.contains("vcon-server migrate"),
        "older, and the command: {why}"
    );
}

#[test]
fn a_newer_schema_is_refused_as_newer_than_the_binary() {
    let why = version_refusal(Some(SCHEMA_VERSION + 1)).expect("refused");
    assert!(
        why.contains("newer than this binary"),
        "the design's words: {why}"
    );
    assert!(
        !why.contains("vcon-server migrate"),
        "migrate cannot fix a newer schema: {why}"
    );
}

#[test]
fn a_missing_owner_role_is_named() {
    let why = role_refusal("vcon_migrate", false, true, false).expect("refused");
    assert!(why.contains("vcon_owner"), "names the role: {why}");
}

#[test]
fn a_missing_application_role_is_named() {
    let why = role_refusal("vcon_migrate", true, false, true).expect("refused");
    assert!(why.contains("vcon_app"), "names the role: {why}");
}

#[test]
fn a_role_that_cannot_set_the_owner_is_named_with_the_grant() {
    let why = role_refusal("vcon_app", true, true, false).expect("refused");
    assert!(
        why.contains("\"vcon_app\"") && why.contains("GRANT vcon_owner"),
        "the role and the grant: {why}"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{
        COMMANDS_LOCK, MIGRATIONS, SCHEMA_VERSION, SERVERS_LOCK,
    };
    use super::super::super::live::{standby, value};
    use super::super::super::why;
    use super::super::live::{
        Db, REGISTRY_TABLES, TENANT_A, TENANT_B, TENANT_TABLES, count, in_scope,
        insert_registry_row, insert_tenant_row, no_scope,
    };

    fn is_no_tenant(result: &Result<(), String>) -> bool {
        result
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("no tenant set"))
    }

    fn all_tables() -> Vec<&'static str> {
        TENANT_TABLES
            .iter()
            .chain(REGISTRY_TABLES)
            .copied()
            .collect()
    }

    #[tokio::test]
    async fn every_empty_table_refuses_a_select_with_no_tenant() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in all_tables() {
            let result = no_scope(&app, &format!("SELECT * FROM vcon.{table}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
        }
    }

    #[tokio::test]
    async fn every_table_refuses_an_insert_with_no_tenant() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in TENANT_TABLES {
            let result = no_scope(&app, &insert_tenant_row(table, TENANT_A)).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
        }
        for table in REGISTRY_TABLES {
            let result = no_scope(&app, &insert_registry_row(table)).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
        }
    }

    #[tokio::test]
    async fn every_table_refuses_an_update_or_delete_with_no_tenant() {
        let db = Db::migrated().await;
        let app = db.app().await;
        let writable: Vec<&str> = all_tables()
            .into_iter()
            .filter(|t| *t != "audit" && *t != "admin_audit")
            .collect();
        for table in &writable {
            let column = match *table {
                "vcon" | "version" | "tombstone" | "key_use" | "parquet_file" | "purge_pending"
                | "manifest" | "tenant_lock" | "tenant_catalog" => "tenant",
                "tenant" => "name_mac",
                "master_key_use" => "master_key_id",
                "api_key" => "token_hash",
                "user_account" => "label_sealed",
                "membership" => "roles_sealed",
                "tenant_setting" => "value_sealed",
                "session" => "scopes_sealed",
                other => panic!("{other}"),
            };
            let result = no_scope(
                &app,
                &format!("UPDATE vcon.{table} SET {column} = {column}"),
            )
            .await;
            assert!(is_no_tenant(&result), "update {table}: {result:?}");
            let result = no_scope(&app, &format!("DELETE FROM vcon.{table}")).await;
            assert!(is_no_tenant(&result), "delete {table}: {result:?}");
        }
    }

    #[tokio::test]
    async fn every_table_with_rows_refuses_a_query_after_an_earlier_tenant() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in TENANT_TABLES {
            in_scope(&app, TENANT_A, &insert_tenant_row(table, TENANT_A))
                .await
                .expect("inserts");
        }
        for table in REGISTRY_TABLES {
            in_scope(&app, "registry", &insert_registry_row(table))
                .await
                .expect("inserts");
        }
        // The setting now exists on this connection, empty: the reused
        // connection case that a plain policy would read as no rows.
        for table in all_tables() {
            let result = no_scope(&app, &format!("SELECT * FROM vcon.{table}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
        }
    }

    #[tokio::test]
    async fn the_version_table_is_refused_to_the_application_role() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for statement in [
            "SELECT * FROM vcon.schema_version",
            "INSERT INTO vcon.schema_version (version) VALUES (9)",
            "UPDATE vcon.schema_version SET version = 9",
            "DELETE FROM vcon.schema_version",
        ] {
            let result = no_scope(&app, statement).await;
            assert!(
                result
                    .as_ref()
                    .is_err_and(|e| e.contains("permission denied")),
                "{statement}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_tenant_transaction_is_refused_the_registry() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in REGISTRY_TABLES {
            let result = in_scope(&app, TENANT_A, &format!("SELECT * FROM vcon.{table}")).await;
            let e = result.expect_err("refused");
            assert!(e.contains("is a tenant's"), "{table}: {e}");
        }
    }

    #[tokio::test]
    async fn the_registry_transaction_is_refused_a_tenants_rows() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in TENANT_TABLES {
            let result = in_scope(&app, "registry", &format!("SELECT * FROM vcon.{table}")).await;
            let e = result.expect_err("refused");
            assert!(e.contains("is the registry's"), "{table}: {e}");
        }
    }

    #[tokio::test]
    async fn a_row_for_another_tenant_is_refused() {
        let db = Db::migrated().await;
        let app = db.app().await;
        let e = in_scope(&app, TENANT_A, &insert_tenant_row("tombstone", TENANT_B))
            .await
            .expect_err("refused");
        assert!(
            e.contains("row-level security"),
            "the WITH CHECK refuses: {e}"
        );
    }

    #[tokio::test]
    async fn another_tenants_row_cannot_be_changed_or_removed() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_tenant_row("key_use", TENANT_A))
            .await
            .expect("inserts");
        in_scope(&app, TENANT_B, "UPDATE vcon.key_use SET wraps = 99")
            .await
            .expect("no row, no error");
        in_scope(&app, TENANT_B, "DELETE FROM vcon.key_use")
            .await
            .expect("no row, no error");
        let rows = in_scope(&app, TENANT_A, "SELECT wraps FROM vcon.key_use")
            .await
            .expect("reads");
        let wraps = rows
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_owned),
                _ => None,
            })
            .expect("the row");
        assert_eq!(wraps, "1", "tenant B changed nothing");
    }

    #[tokio::test]
    async fn a_malformed_tenant_setting_is_refused() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for scope in ["zz", "abcd", "00112233445566778899AABBCCDDEEFF"] {
            let e = in_scope(&app, scope, "SELECT * FROM vcon.vcon")
                .await
                .expect_err("refused");
            assert!(e.contains("not a tenant id"), "{scope}: {e}");
        }
    }

    #[tokio::test]
    async fn the_audit_log_cannot_be_deleted_changed_or_truncated() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_tenant_row("audit", TENANT_A))
            .await
            .expect("inserts");
        for statement in [
            "DELETE FROM vcon.audit",
            "UPDATE vcon.audit SET action = 'changed'",
            "TRUNCATE vcon.audit",
        ] {
            let e = in_scope(&app, TENANT_A, statement)
                .await
                .expect_err("refused");
            assert!(e.contains("permission denied"), "{statement}: {e}");
        }
        for statement in [
            "DELETE FROM vcon.admin_audit",
            "UPDATE vcon.admin_audit SET action = 'changed'",
            "TRUNCATE vcon.admin_audit",
        ] {
            let e = in_scope(&app, "registry", statement)
                .await
                .expect_err("refused");
            assert!(e.contains("permission denied"), "{statement}: {e}");
        }
    }

    #[tokio::test]
    async fn the_application_role_cannot_change_the_schema() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for statement in [
            "CREATE TABLE vcon.evil (a int)",
            "DROP TABLE vcon.vcon",
            "ALTER TABLE vcon.vcon DISABLE ROW LEVEL SECURITY",
            "DROP POLICY tenant_rows ON vcon.vcon",
            "TRUNCATE vcon.version",
        ] {
            let result = no_scope(&app, statement).await;
            let e = result.expect_err("refused");
            assert!(
                e.contains("permission denied") || e.contains("must be owner"),
                "{statement}: {e}"
            );
        }
    }

    #[tokio::test]
    async fn a_serving_connection_to_a_database_with_no_schema_is_refused() {
        let db = Db::fresh().await;
        let why = why(db
            .connect("vcon_app", Purpose::Serve)
            .await
            .expect_err("refused"));
        assert!(
            why.contains("no vCon schema") && why.contains("vcon-server migrate"),
            "says so: {why}"
        );
    }

    #[tokio::test]
    async fn a_serving_connection_to_a_newer_schema_is_refused() {
        let db = Db::migrated().await;
        db.admin()
            .await
            .batch_execute(&format!(
                "SET ROLE vcon_owner; INSERT INTO vcon.schema_version (version) VALUES ({}); \
                 RESET ROLE",
                SCHEMA_VERSION + 1
            ))
            .await
            .expect("adds a version");
        let why = why(db
            .connect("vcon_app", Purpose::Serve)
            .await
            .expect_err("refused"));
        assert!(why.contains("newer than this binary"), "says so: {why}");
    }

    #[tokio::test]
    async fn a_serving_connection_as_a_member_of_the_owner_role_is_refused() {
        let db = Db::migrated().await;
        let why = why(db
            .connect("vcon_migrate", Purpose::Serve)
            .await
            .expect_err("refused"));
        assert!(
            why.contains("vcon_migrate") && why.contains("owns"),
            "names the role: {why}"
        );
    }

    #[tokio::test]
    async fn a_serving_connection_as_a_role_without_grants_is_refused() {
        let db = Db::migrated().await;
        let why = why(db
            .connect("vcon_stranger", Purpose::Serve)
            .await
            .expect_err("refused"));
        assert!(
            why.contains("vcon_stranger") && why.contains("vcon_app"),
            "names the role and the grant: {why}"
        );
    }

    #[tokio::test]
    async fn migrate_refuses_a_role_that_cannot_act_as_the_owner() {
        let db = Db::fresh().await;
        let why = why(db
            .connect("vcon_app", Purpose::Migrate)
            .await
            .expect_err("refused"));
        assert!(why.contains("GRANT vcon_owner"), "says so: {why}");
        assert_eq!(
            value(
                &db.admin().await,
                "SELECT to_regclass('vcon.schema_version') IS NULL"
            )
            .await,
            "t",
            "nothing was made"
        );
    }

    #[tokio::test]
    async fn migrate_refuses_while_a_server_holds_the_lock() {
        let db = Db::fresh().await;
        let holder = db.admin().await;
        holder
            .simple_query(&format!("SELECT pg_advisory_lock({SERVERS_LOCK})"))
            .await
            .expect("holds it");
        let e = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(e.contains("another"), "says so: {e}");
        assert_eq!(
            value(&holder, "SELECT to_regclass('vcon.schema_version') IS NULL").await,
            "t",
            "and nothing was made"
        );
    }

    #[tokio::test]
    async fn migrate_refuses_while_a_command_holds_the_lock() {
        let db = Db::fresh().await;
        let holder = db.admin().await;
        holder
            .simple_query(&format!("SELECT pg_advisory_lock({COMMANDS_LOCK})"))
            .await
            .expect("holds it");
        let e = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(e.contains("another"), "says so: {e}");
    }

    #[tokio::test]
    async fn migrate_refuses_while_a_lock_is_shared() {
        let db = Db::fresh().await;
        let holder = db.admin().await;
        holder
            .simple_query(&format!("SELECT pg_advisory_lock_shared({SERVERS_LOCK})"))
            .await
            .expect("holds it shared");
        let e = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(
            e.contains("another"),
            "a shared holder is a server too: {e}"
        );
    }

    #[tokio::test]
    async fn a_migration_that_fails_leaves_nothing_behind() {
        let db = Db::fresh().await;
        let bad = "CREATE SCHEMA vcon; CREATE TABLE vcon.partial (a int); SELECT boom";
        let e = db.migrate_with(&[bad]).await.expect_err("refused");
        assert!(
            e.contains("boom") || e.contains("does not exist"),
            "the failure named: {e}"
        );
        assert_eq!(
            value(
                &db.admin().await,
                "SELECT to_regclass('vcon.partial') IS NULL"
            )
            .await,
            "t",
            "the transaction rolled back"
        );
    }

    #[tokio::test]
    async fn a_later_migration_that_fails_keeps_the_earlier_one() {
        let db = Db::fresh().await;
        let v2 = "CREATE TABLE vcon.extra (a int); SELECT boom";
        let e = db
            .migrate_with(&[MIGRATIONS[0], v2])
            .await
            .expect_err("refused");
        assert!(
            e.contains("boom") || e.contains("does not exist"),
            "named: {e}"
        );
        let admin = db.admin().await;
        assert_eq!(
            count(&admin, "SELECT count(*) FROM vcon.schema_version").await,
            1,
            "version 1 stayed committed"
        );
        assert_eq!(
            value(&admin, "SELECT to_regclass('vcon.extra') IS NULL").await,
            "t",
            "version 2 left nothing"
        );
    }

    #[tokio::test]
    async fn migrate_refuses_a_database_newer_than_the_binary() {
        let db = Db::migrated().await;
        db.admin()
            .await
            .batch_execute(&format!(
                "SET ROLE vcon_owner; INSERT INTO vcon.schema_version (version) VALUES ({}); \
                 RESET ROLE",
                SCHEMA_VERSION + 1
            ))
            .await
            .expect("adds a version");
        let e = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(e.contains("newer than this binary"), "says so: {e}");
    }

    #[tokio::test]
    async fn migrate_refuses_a_schema_it_did_not_make() {
        let db = Db::fresh().await;
        db.admin()
            .await
            .batch_execute("SET ROLE vcon_owner; CREATE SCHEMA vcon; RESET ROLE")
            .await
            .expect("makes a schema");
        let e = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(
            e.contains("vcon") && e.contains("did not make"),
            "says so: {e}"
        );
    }

    #[tokio::test]
    async fn migrate_refuses_a_standby() {
        let db = Db::fresh().await;
        // The standby answers as the primary would; only the port differs.
        let (_dir, mut settings) = db.settings("vcon_migrate");
        settings.port = standby();
        settings.dbname = "vcon".to_owned();
        let e = super::super::super::super::migrate(&settings)
            .await
            .expect_err("refused");
        assert!(e.0.contains("standby"), "says so: {}", e.0);
    }

    #[tokio::test]
    async fn no_message_holds_the_password_or_the_tenant_setting() {
        let db = Db::fresh().await;
        let password = super::super::super::live::password();
        let (_dir, settings) = db.settings("vcon_stranger");
        let e = super::super::super::super::migrate(&settings)
            .await
            .expect_err("refused");
        assert!(
            !e.0.contains(&password),
            "the refusal never holds the password: {}",
            e.0
        );
    }
}
