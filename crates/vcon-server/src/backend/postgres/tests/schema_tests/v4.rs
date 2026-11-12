//! Schema version 4: the settings a tenant's administrator sets, the
//! registry's table `tenant_setting`. Each live test starts from a
//! database at version 3, as a build of `main` before the settings left
//! one.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The statements of a migration, besides its comments, each on one line.
fn statements(migration: &str) -> Vec<String> {
    let text: Vec<&str> = migration
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("--") && !line.is_empty())
        .collect();
    text.join(" ")
        .split_inclusive(';')
        .map(|statement| statement.trim().to_owned())
        .collect()
}

#[test]
fn version_4_is_the_fourth_migration() {
    assert!(
        MIGRATIONS[3].contains("CREATE TABLE vcon.tenant_setting ("),
        "the fourth makes the table"
    );
}

#[test]
fn a_version_3_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(3)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 3, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn version_4_changes_no_row_and_drops_nothing() {
    // The owner role is held to row-level security too, so an UPDATE or
    // DELETE in a migration would raise. A statement begins its line.
    for statement in ["UPDATE ", "DELETE ", "DROP ", "TRUNCATE ", "INSERT "] {
        assert!(
            !MIGRATIONS[3]
                .lines()
                .any(|line| line.trim_start().to_uppercase().starts_with(statement)),
            "no {statement}in version 4"
        );
    }
}

#[test]
fn version_4_is_the_table_its_row_level_security_and_its_grant() {
    assert_eq!(
        statements(MIGRATIONS[3]),
        [
            "CREATE TABLE vcon.tenant_setting ( tenant bytea NOT NULL, name text NOT NULL, \
             value_sealed bytea NOT NULL, PRIMARY KEY (tenant, name) );",
            "ALTER TABLE vcon.tenant_setting ENABLE ROW LEVEL SECURITY;",
            "ALTER TABLE vcon.tenant_setting FORCE ROW LEVEL SECURITY;",
            "CREATE POLICY registry_rows ON vcon.tenant_setting \
             USING (vcon.in_registry() = (tenant IS NOT NULL)) WITH CHECK (vcon.in_registry());",
            "GRANT SELECT, INSERT, UPDATE, DELETE ON vcon.tenant_setting TO vcon_app;",
        ],
        "the whole migration, besides its comments"
    );
}

#[test]
fn a_database_newer_than_this_binary_s_version_is_refused_as_newer() {
    let newer = SCHEMA_VERSION + 1;
    let why = version_refusal(Some(newer)).expect("refused");
    assert!(
        why.contains(&format!("version {newer}")) && why.contains("upgrade this server"),
        "a newer database: {why}"
    );
}

#[test]
fn a_database_at_this_binary_s_version_is_not_refused() {
    assert_eq!(
        version_refusal(Some(SCHEMA_VERSION)),
        None,
        "this binary's version"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::super::live::value;
    use super::super::live::{Db, TENANT_A, count, in_scope, insert_registry_row};

    /// A database at version 3.
    async fn at_version_3() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..3]).await.expect("version 3"),
            Migrated { from: 0, to: 3 },
            "the first three migrations alone"
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

    const TABLE: &str = "SELECT count(*) FROM information_schema.tables WHERE \
        table_schema = 'vcon' AND table_name = 'tenant_setting'";

    const SETTING: &str = "INSERT INTO vcon.tenant_setting (tenant, name, value_sealed) \
        VALUES ('\\x01', 'masking.legal', '\\xc0ffee')";

    // What must work.

    #[tokio::test]
    async fn a_version_3_database_is_brought_to_version_4_with_its_tenants_and_caps() {
        let db = at_version_3().await;
        let admin = db.admin().await;
        in_scope(&admin, "registry", &insert_registry_row("tenant"))
            .await
            .expect("a tenant, written before the upgrade");
        in_scope(
            &admin,
            "registry",
            "UPDATE vcon.tenant SET role_cap_sealed = '\\xbeef'",
        )
        .await
        .expect("and its cap");
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 3,
                to: SCHEMA_VERSION
            },
            "the migrations after the third run"
        );
        assert_eq!(
            read(
                &db,
                "SELECT encode(role_cap_sealed, 'hex') FROM vcon.tenant"
            )
            .await
            .as_deref(),
            Some("beef"),
            "the tenant kept its row and its cap"
        );
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.tenant_setting")
                .await
                .as_deref(),
            Some("0"),
            "and has set nothing"
        );
    }

    #[tokio::test]
    async fn the_table_is_made_by_version_4_and_not_before() {
        let db = at_version_3().await;
        assert_eq!(count(&db.admin().await, TABLE).await, 0, "at version 3");
        db.migrate().await;
        assert_eq!(count(&db.admin().await, TABLE).await, 1, "at version 4");
    }

    #[tokio::test]
    async fn the_application_role_sets_reads_changes_and_removes_a_setting() {
        let db = Db::migrated().await;
        registry(&db, SETTING).await.expect("the setting is set");
        let stored = "SELECT encode(value_sealed, 'hex') FROM vcon.tenant_setting \
                      WHERE tenant = '\\x01' AND name = 'masking.legal'";
        assert_eq!(
            read(&db, stored).await.as_deref(),
            Some("c0ffee"),
            "read back"
        );
        registry(
            &db,
            "INSERT INTO vcon.tenant_setting (tenant, name, value_sealed) \
             VALUES ('\\x01', 'masking.legal', '\\xdecaf0') \
             ON CONFLICT (tenant, name) DO UPDATE SET value_sealed = excluded.value_sealed",
        )
        .await
        .expect("set again, over the value it had");
        assert_eq!(
            read(&db, stored).await.as_deref(),
            Some("decaf0"),
            "the last value"
        );
        registry(
            &db,
            "DELETE FROM vcon.tenant_setting WHERE tenant = '\\x01'",
        )
        .await
        .expect("removed");
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.tenant_setting")
                .await
                .as_deref(),
            Some("0"),
            "no row is left"
        );
    }

    #[tokio::test]
    async fn a_serving_connection_is_accepted_once_version_4_is_there() {
        let db = at_version_3().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_err(),
            "refused at version 3"
        );
        db.migrate().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_ok(),
            "accepted at version 4"
        );
    }

    #[tokio::test]
    async fn a_new_database_reaches_version_4_with_the_table() {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 0,
                to: SCHEMA_VERSION
            },
            "every migration"
        );
        assert_eq!(count(&db.admin().await, TABLE).await, 1, "the table");
    }

    // What must be refused.

    #[tokio::test]
    async fn a_version_4_that_fails_leaves_version_3_whole() {
        let db = at_version_3().await;
        db.admin()
            .await
            .batch_execute(
                "SET ROLE vcon_owner; CREATE TABLE vcon.tenant_setting (other int); RESET ROLE",
            )
            .await
            .expect("a table in the way");
        let refused = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(
            refused.contains("version 4"),
            "names the version: {refused}"
        );
        assert_eq!(
            value(
                &db.admin().await,
                "SELECT max(version) FROM vcon.schema_version"
            )
            .await,
            "3",
            "still version 3"
        );
    }

    #[tokio::test]
    async fn a_tenants_transaction_reads_and_writes_no_setting() {
        let db = Db::migrated().await;
        registry(&db, SETTING).await.expect("a setting");
        let app = db.app().await;
        assert!(
            in_scope(&app, TENANT_A, "SELECT name FROM vcon.tenant_setting")
                .await
                .is_err(),
            "a tenant's transaction is refused the registry's table"
        );
        // Another name, so the row is refused for its scope and not as a
        // second row of the name above.
        let other = SETTING.replace("masking.legal", "masking.reader");
        assert!(
            in_scope(&app, TENANT_A, &other).await.is_err(),
            "a tenant's transaction sets nothing"
        );
        registry(&db, &other)
            .await
            .expect("the same row is taken from the registry's transaction");
        assert!(
            in_scope(&app, TENANT_A, "DELETE FROM vcon.tenant_setting")
                .await
                .is_err(),
            "and removes nothing"
        );
    }

    #[tokio::test]
    async fn the_application_role_cannot_drop_or_change_the_table() {
        let db = Db::migrated().await;
        for statement in [
            "DROP TABLE vcon.tenant_setting",
            "ALTER TABLE vcon.tenant_setting DISABLE ROW LEVEL SECURITY",
            "ALTER TABLE vcon.tenant_setting ADD COLUMN extra int",
            "TRUNCATE vcon.tenant_setting",
        ] {
            assert!(
                registry(&db, statement).await.is_err(),
                "the application role is refused: {statement}"
            );
        }
    }

    #[tokio::test]
    async fn a_second_row_for_a_tenants_name_is_refused() {
        let db = Db::migrated().await;
        registry(&db, SETTING).await.expect("the first row");
        let second = registry(&db, SETTING).await.expect_err("the second row");
        assert!(
            second.contains("duplicate key"),
            "one row for a tenant's name: {second}"
        );
    }

    #[tokio::test]
    async fn a_row_with_no_tenant_name_or_value_is_refused() {
        let db = Db::migrated().await;
        for row in [
            "(NULL, 'masking.legal', '\\x01')",
            "('\\x01', NULL, '\\x01')",
            "('\\x01', 'masking.legal', NULL)",
        ] {
            let statement = format!(
                "INSERT INTO vcon.tenant_setting (tenant, name, value_sealed) VALUES {row}"
            );
            assert!(
                registry(&db, &statement).await.is_err(),
                "a row with a NULL is refused: {row}"
            );
        }
    }

    #[tokio::test]
    async fn a_value_that_is_no_bytea_is_refused() {
        let db = Db::migrated().await;
        assert!(
            registry(
                &db,
                "INSERT INTO vcon.tenant_setting (tenant, name, value_sealed) \
                 VALUES ('\\x01', 'masking.legal', 'true'::text)"
            )
            .await
            .is_err(),
            "a value is sealed bytes, never plain text"
        );
    }

    // Limits and odd inputs.

    #[tokio::test]
    async fn migrating_twice_from_version_3_does_the_work_once() {
        let db = at_version_3().await;
        let now = SCHEMA_VERSION;
        assert_eq!(
            db.migrate().await,
            Migrated { from: 3, to: now },
            "the first"
        );
        assert_eq!(
            db.migrate().await,
            Migrated { from: now, to: now },
            "the second finds nothing to do"
        );
        assert_eq!(count(&db.admin().await, TABLE).await, 1, "one table");
    }

    #[tokio::test]
    async fn a_version_3_database_with_no_tenant_gains_the_table() {
        let db = at_version_3().await;
        db.migrate().await;
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.tenant_setting")
                .await
                .as_deref(),
            Some("0"),
            "the table is there for the first tenant"
        );
    }

    #[tokio::test]
    async fn one_name_in_two_tenants_is_two_rows() {
        let db = Db::migrated().await;
        registry(&db, SETTING).await.expect("the first tenant's");
        registry(&db, &SETTING.replace("'\\x01'", "'\\x02'"))
            .await
            .expect("the second tenant's");
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.tenant_setting")
                .await
                .as_deref(),
            Some("2"),
            "a row for each tenant"
        );
    }

    #[tokio::test]
    async fn an_empty_sealed_value_is_a_value_not_a_missing_one() {
        let db = Db::migrated().await;
        registry(&db, &SETTING.replace("'\\xc0ffee'", "''"))
            .await
            .expect("empty bytes are not NULL");
        assert_eq!(
            read(
                &db,
                "SELECT octet_length(value_sealed) FROM vcon.tenant_setting"
            )
            .await
            .as_deref(),
            Some("0"),
            "kept as empty bytes; the codec that unseals it refuses it"
        );
    }
}
