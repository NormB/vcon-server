//! Row-level security under a cached plan. A prepared statement is planned
//! with a tenant set, the server keeps a generic plan for it, and the plan
//! is then run with no tenant, or with another.
//!
//! What holds for every plan: a statement with no tenant never returns or
//! changes a row, and it is refused as soon as it examines one, since the
//! policy's function raises for each row. What does not hold: a refusal
//! from a statement that examines no row. A cached plan that scans a table
//! with no rows, or whose index condition finds none, ends with nothing
//! and no error, because the server never evaluates the policy. The
//! refusal of a query on an empty table that the other tests show comes
//! from planning, which a cached plan does not repeat. So the code that
//! runs a statement sets the tenant; the policy is the second check.

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use tokio_postgres::{Client, SimpleQueryMessage};

    use super::super::live::{
        Db, REGISTRY_TABLES, TENANT_A, TENANT_B, TENANT_TABLES, db_message, in_scope,
        insert_registry_row, insert_tenant_row,
    };

    /// The runs that make the server cache a generic plan: five custom
    /// plans, then the generic plan the sixth run keeps.
    const RUNS: usize = 7;

    /// The scope a table's rows are read in: a tenant's own, or the
    /// registry's marker.
    fn scope_of(table: &str) -> &'static str {
        if REGISTRY_TABLES.contains(&table) {
            "registry"
        } else {
            TENANT_A
        }
    }

    fn all_tables() -> impl Iterator<Item = &'static str> {
        TENANT_TABLES.iter().chain(REGISTRY_TABLES).copied()
    }

    /// The tables the application role may delete from: every one but the
    /// two audit logs and, since schema version 11, the keys of a
    /// tenant's log.
    fn deletable() -> impl Iterator<Item = &'static str> {
        all_tables().filter(|t| !matches!(*t, "audit" | "audit_key" | "admin_audit"))
    }

    /// The statement that puts one row in `table`, a tenant's or the
    /// registry's.
    fn insert_row(table: &str, tenant: &str) -> String {
        if REGISTRY_TABLES.contains(&table) {
            insert_registry_row(table)
        } else {
            insert_tenant_row(table, tenant)
        }
    }

    /// Prepares `statement` as `name` and runs it [`RUNS`] times in
    /// `scope`, each in a transaction that is rolled back, so the plan is
    /// cached and the tables are left as they were.
    async fn cached(app: &Client, name: &str, statement: &str, scope: &str) {
        app.batch_execute(&format!("PREPARE {name} AS {statement}"))
            .await
            .unwrap_or_else(|e| panic!("{name}: prepares: {e}"));
        for run in 0..RUNS {
            app.batch_execute(&format!(
                "BEGIN; SET LOCAL vcon.tenant = '{scope}'; EXECUTE {name}; ROLLBACK"
            ))
            .await
            .unwrap_or_else(|e| panic!("{name}: run {run} in scope {scope}: {e}"));
        }
    }

    /// The number of rows `statement` returns with no tenant set, or the
    /// server's message.
    async fn rows_with_no_tenant(app: &Client, statement: &str) -> Result<usize, String> {
        app.simple_query(statement)
            .await
            .map(|messages| rows_of(&messages))
            .map_err(|e| db_message(&e))
    }

    fn rows_of(messages: &[SimpleQueryMessage]) -> usize {
        messages
            .iter()
            .filter(|m| matches!(m, SimpleQueryMessage::Row(_)))
            .count()
    }

    /// The number of rows of `table` that `scope` sees.
    async fn rows_in(app: &Client, scope: &str, table: &str) -> usize {
        in_scope(app, scope, &format!("SELECT * FROM vcon.{table}"))
            .await
            .map(|messages| rows_of(&messages))
            .unwrap_or_else(|e| panic!("{table}: read in scope {scope}: {e}"))
    }

    fn is_no_tenant(result: &Result<usize, String>) -> bool {
        result
            .as_ref()
            .err()
            .is_some_and(|e| e.contains("no tenant set"))
    }

    /// A migrated database with one row in every table, and the
    /// application role's connection to it.
    async fn with_a_row_in_every_table() -> (Db, Client) {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in all_tables() {
            in_scope(&app, scope_of(table), &insert_row(table, TENANT_A))
                .await
                .unwrap_or_else(|e| panic!("{table}: inserts: {e}"));
        }
        (db, app)
    }

    #[tokio::test]
    async fn a_cached_select_on_every_table_with_a_row_refuses_a_missing_tenant() {
        let (_db, app) = with_a_row_in_every_table().await;
        for table in all_tables() {
            let name = format!("q_{table}");
            cached(
                &app,
                &name,
                &format!("SELECT * FROM vcon.{table}"),
                scope_of(table),
            )
            .await;
            let result = rows_with_no_tenant(&app, &format!("EXECUTE {name}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
        }
    }

    #[tokio::test]
    async fn a_forced_generic_select_on_every_table_with_a_row_refuses_a_missing_tenant() {
        let (_db, app) = with_a_row_in_every_table().await;
        app.batch_execute("SET plan_cache_mode = force_generic_plan")
            .await
            .expect("the setting is taken");
        for table in all_tables() {
            let name = format!("q_{table}");
            cached(
                &app,
                &name,
                &format!("SELECT * FROM vcon.{table}"),
                scope_of(table),
            )
            .await;
            let result = rows_with_no_tenant(&app, &format!("EXECUTE {name}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
        }
    }

    #[tokio::test]
    async fn a_cached_select_refuses_a_missing_tenant_for_another_tenants_row() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in TENANT_TABLES {
            in_scope(&app, TENANT_B, &insert_tenant_row(table, TENANT_B))
                .await
                .unwrap_or_else(|e| panic!("{table}: inserts: {e}"));
            let name = format!("q_{table}");
            // Planned and cached by a tenant that has no row of its own.
            cached(
                &app,
                &name,
                &format!("SELECT * FROM vcon.{table}"),
                TENANT_A,
            )
            .await;
            let result = rows_with_no_tenant(&app, &format!("EXECUTE {name}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
        }
    }

    // The limit the module's first lines state: with no row to examine the
    // policy is not evaluated, so the answer is the refusal or nothing,
    // whichever plan the server kept. It is never a row.
    #[tokio::test]
    async fn a_cached_select_on_every_empty_table_gives_a_missing_tenant_no_row() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in all_tables() {
            let name = format!("q_{table}");
            cached(
                &app,
                &name,
                &format!("SELECT * FROM vcon.{table}"),
                scope_of(table),
            )
            .await;
            let result = rows_with_no_tenant(&app, &format!("EXECUTE {name}")).await;
            assert!(
                is_no_tenant(&result) || result == Ok(0),
                "{table}: refused, or nothing: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_cached_delete_with_no_tenant_is_refused_and_keeps_the_row() {
        let (_db, app) = with_a_row_in_every_table().await;
        for table in deletable() {
            let name = format!("d_{table}");
            cached(
                &app,
                &name,
                &format!("DELETE FROM vcon.{table}"),
                scope_of(table),
            )
            .await;
            let result = rows_with_no_tenant(&app, &format!("EXECUTE {name}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
            assert_eq!(
                rows_in(&app, scope_of(table), table).await,
                1,
                "{table}: the row is still there"
            );
        }
    }

    #[tokio::test]
    async fn a_cached_count_update_with_no_tenant_is_refused_in_both_audit_logs() {
        let (_db, app) = with_a_row_in_every_table().await;
        for table in ["audit", "admin_audit"] {
            let name = format!("u_{table}");
            cached(
                &app,
                &name,
                &format!("UPDATE vcon.{table} SET count = count + 1"),
                scope_of(table),
            )
            .await;
            let result = rows_with_no_tenant(&app, &format!("EXECUTE {name}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
            let counts = in_scope(
                &app,
                scope_of(table),
                &format!("SELECT count FROM vcon.{table}"),
            )
            .await
            .expect("reads the count");
            let count = counts.iter().find_map(|m| match m {
                SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
                _ => None,
            });
            assert_eq!(
                count.as_deref(),
                Some("1"),
                "{table}: the count is as it was inserted"
            );
        }
    }

    #[tokio::test]
    async fn a_cached_insert_into_every_empty_table_refuses_a_missing_tenant() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for table in all_tables() {
            let name = format!("i_{table}");
            cached(&app, &name, &insert_row(table, TENANT_A), scope_of(table)).await;
            let result = rows_with_no_tenant(&app, &format!("EXECUTE {name}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
            assert_eq!(
                rows_in(&app, scope_of(table), table).await,
                0,
                "{table}: nothing was inserted"
            );
        }
    }

    #[tokio::test]
    async fn a_cached_select_refuses_a_setting_a_transaction_left_empty() {
        let (_db, app) = with_a_row_in_every_table().await;
        for table in ["tombstone", "tenant"] {
            let name = format!("q_{table}");
            cached(
                &app,
                &name,
                &format!("SELECT * FROM vcon.{table}"),
                scope_of(table),
            )
            .await;
            // The setting exists on this connection now, and is empty, as
            // on a pooled connection a transaction used and gave back.
            let left = app
                .simple_query("SHOW vcon.tenant")
                .await
                .expect("the setting exists after a transaction set it");
            assert_eq!(rows_of(&left), 1, "{table}: the setting is shown");
            let result = rows_with_no_tenant(&app, &format!("EXECUTE {name}")).await;
            assert!(is_no_tenant(&result), "{table}: {result:?}");
        }
    }

    // A generic plan holds the call of the policy's function, not the
    // tenant it returned while the plan was made.
    #[tokio::test]
    async fn a_plan_cached_by_one_tenant_gives_another_tenant_only_its_own_rows() {
        let (_db, app) = with_a_row_in_every_table().await;
        for table in TENANT_TABLES {
            let name = format!("q_{table}");
            cached(
                &app,
                &name,
                &format!("SELECT * FROM vcon.{table}"),
                TENANT_A,
            )
            .await;
            let other = in_scope(&app, TENANT_B, &format!("EXECUTE {name}"))
                .await
                .unwrap_or_else(|e| panic!("{table}: the other tenant runs it: {e}"));
            assert_eq!(
                rows_of(&other),
                0,
                "{table}: none of the first tenant's rows"
            );
            in_scope(&app, TENANT_B, &insert_tenant_row(table, TENANT_B))
                .await
                .unwrap_or_else(|e| panic!("{table}: the other tenant inserts: {e}"));
            let other = in_scope(&app, TENANT_B, &format!("EXECUTE {name}"))
                .await
                .unwrap_or_else(|e| panic!("{table}: the other tenant runs it again: {e}"));
            assert_eq!(rows_of(&other), 1, "{table}: its own row and no other");
            let first = in_scope(&app, TENANT_A, &format!("EXECUTE {name}"))
                .await
                .unwrap_or_else(|e| panic!("{table}: the first tenant runs it: {e}"));
            assert_eq!(rows_of(&first), 1, "{table}: the first tenant's row only");
        }
    }

    #[tokio::test]
    async fn a_cached_delete_by_one_tenant_leaves_another_tenants_rows() {
        let (_db, app) = with_a_row_in_every_table().await;
        for table in TENANT_TABLES
            .iter()
            .filter(|t| !matches!(**t, "audit" | "audit_key"))
        {
            let name = format!("d_{table}");
            cached(&app, &name, &format!("DELETE FROM vcon.{table}"), TENANT_B).await;
            in_scope(&app, TENANT_B, &format!("EXECUTE {name}"))
                .await
                .unwrap_or_else(|e| panic!("{table}: the other tenant deletes: {e}"));
            assert_eq!(
                rows_in(&app, TENANT_A, table).await,
                1,
                "{table}: the first tenant's row is still there"
            );
        }
    }

    #[tokio::test]
    async fn a_plan_cached_in_one_scope_refuses_the_other_kind_of_scope() {
        let (_db, app) = with_a_row_in_every_table().await;
        cached(&app, "q_tenant", "SELECT * FROM vcon.tenant", "registry").await;
        let e = in_scope(&app, TENANT_A, "EXECUTE q_tenant")
            .await
            .expect_err("a tenant's transaction is refused the registry's rows");
        assert!(
            e.contains("a tenant's, not the registry's"),
            "says why: {e}"
        );
        cached(&app, "q_vcon", "SELECT * FROM vcon.vcon", TENANT_A).await;
        let e = in_scope(&app, "registry", "EXECUTE q_vcon")
            .await
            .expect_err("the registry's transaction is refused a tenant's rows");
        assert!(
            e.contains("the registry's, not a tenant's"),
            "says why: {e}"
        );
    }
}
