//! Schema version 10: a registry transaction reads the lock rows of every
//! tenant, and writes none. A server lists with it the tenants that have a
//! catalog, to remove one a cut-off create or removal left with no row in
//! the registry. A tenant's transaction reads and writes its own lock row
//! and no other, as before. Each live test of the migration starts from a
//! database at version 9.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 9;

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
fn version_10_is_the_tenth_migration_and_replaces_the_policy_of_the_lock_rows() {
    assert_eq!(
        text()
            .matches("DROP POLICY tenant_rows ON vcon.tenant_lock;")
            .count(),
        1,
        "the tenth takes the one policy of version 1 off the lock rows"
    );
}

#[test]
fn the_binary_s_version_is_at_least_10() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn a_read_asks_for_the_registry_s_marker_before_it_asks_for_a_tenant() {
    let lines = statements(text());
    let at = lines
        .iter()
        .position(|line| {
            line.starts_with("CREATE POLICY lock_reads ON vcon.tenant_lock FOR SELECT")
        })
        .expect("the policy of a read");
    assert_eq!(
        lines[at + 1..at + 3],
        [
            "    USING (CASE WHEN current_setting('vcon.tenant', true) = 'registry' THEN true",
            "                ELSE tenant = vcon.tenant() END);"
        ],
        "a CASE gives the order: vcon.tenant() raises in a registry transaction"
    );
}

#[test]
fn each_kind_of_write_has_a_policy_that_asks_for_the_tenant() {
    let text = statements(text()).join("\n");
    for (name, kind, rule) in [
        (
            "lock_inserts",
            "INSERT",
            "WITH CHECK (tenant = vcon.tenant())",
        ),
        (
            "lock_updates",
            "UPDATE",
            "USING (tenant = vcon.tenant()) WITH CHECK (tenant = vcon.tenant())",
        ),
        ("lock_deletes", "DELETE", "USING (tenant = vcon.tenant())"),
    ] {
        let policy = format!("CREATE POLICY {name} ON vcon.tenant_lock FOR {kind}\n    {rule};");
        assert_eq!(text.matches(&policy).count(), 1, "{policy}");
    }
}

/// A page of the site, or the CHANGELOG, as text with its line breaks
/// made spaces.
fn flat(path: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_how_to_says_what_version_10_changes() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    assert!(
        page.contains(
            "Version 10 changes who reads the table `tenant_lock`, which holds one row for \
             each tenant that has a catalog. A transaction of the registry now reads every \
             row of it, and writes none"
        ),
        "the paragraph on version 10"
    );
}

#[test]
fn the_how_to_says_a_removed_tenant_s_audit_records_are_kept() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    assert!(
        page.contains(
            "A tenant's audit records are never deleted: the statements that remove a \
             tenant delete its rows from every table but `audit` and `audit_key`"
        ),
        "the rule, in the how-to"
    );
}

#[test]
fn the_how_to_says_version_10_under_schema_versions_and_that_the_log_is_kept() {
    // The changelog links the how-to for this; its entry does not walk
    // through the versions.
    let page = flat("../../website/content/docs/postgres-catalog.md");
    let (before, versions) = page
        .split_once(" ### Schema versions ")
        .expect("the how-to has the section Schema versions");
    assert!(
        versions.contains("Version 10 changes who reads the table `tenant_lock`"),
        "version 10, in the section of the schema's versions"
    );
    assert!(
        before.contains("A tenant that is removed keeps its audit records."),
        "the rule, where the page says what the database holds"
    );
}

// What must be refused.

#[test]
fn a_version_9_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(9)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 9, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn no_role_is_granted_anything_by_version_10() {
    assert!(
        !statements(text())
            .iter()
            .any(|line| line.contains("GRANT") || line.contains("REVOKE")),
        "the privileges stay as version 1 gave them"
    );
}

#[test]
fn the_registry_s_marker_is_in_no_policy_of_a_write() {
    let text = statements(text()).join("\n");
    assert_eq!(
        text.matches("'registry'").count(),
        1,
        "the marker is read once, by the policy of a read"
    );
}

// The edges.

#[test]
fn version_10_names_no_table_but_the_lock_rows() {
    for line in statements(text()) {
        if let Some(at) = line.find(" ON vcon.") {
            assert!(
                line[at..].starts_with(" ON vcon.tenant_lock"),
                "another table: {line}"
            );
        }
    }
}

#[test]
fn version_10_changes_no_row_and_makes_no_table() {
    for line in statements(text()) {
        let line = line.trim_start();
        for word in [
            "INSERT ",
            "UPDATE ",
            "DELETE ",
            "TRUNCATE ",
            "CREATE TABLE",
            "ALTER TABLE",
        ] {
            assert!(!line.starts_with(word), "{word}in version 10: {line}");
        }
    }
}

#[test]
fn version_10_makes_four_policies_and_drops_one() {
    let lines = statements(text());
    let starting = |with: &str| lines.iter().filter(|line| line.starts_with(with)).count();
    assert_eq!(
        starting("CREATE POLICY "),
        4,
        "one for each kind of statement"
    );
    assert_eq!(starting("DROP POLICY "), 1, "the one of version 1");
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use tokio_postgres::Client;

    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::live::{Db, TENANT_A, TENANT_B, in_scope};
    use super::super::v8::on_a_server::{one, owner, refused};
    use super::THIS;

    /// What a registry transaction is told when it asks for a tenant.
    const THE_REGISTRY_S: &str = "this transaction is the registry's, not a tenant's";

    /// What a transaction with no tenant is told.
    const NO_TENANT: &str = "no tenant set for this transaction";

    /// Every lock row a transaction sees, in order, as one text.
    const LIST: &str = "SELECT coalesce(string_agg(encode(tenant, 'hex'), ',' ORDER BY tenant), '') \
         FROM vcon.tenant_lock";

    /// The policies of the lock rows, by name, each with its statement.
    const POLICIES: &str = "SELECT string_agg(policyname || ' ' || cmd, ',' ORDER BY policyname) \
         FROM pg_policies WHERE schemaname = 'vcon' AND tablename = 'tenant_lock'";

    /// A database at version 9.
    async fn at_version_9() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 9"),
            Migrated { from: 0, to: 9 },
            "the first nine migrations alone"
        );
        db
    }

    /// The statement that makes the lock row of `tenant`.
    fn lock_row(tenant: &str) -> String {
        format!("INSERT INTO vcon.tenant_lock (tenant) VALUES ('\\x{tenant}')")
    }

    /// Makes the lock rows of both tenants, each in its own transaction.
    async fn two_tenants(client: &Client) {
        for tenant in [TENANT_A, TENANT_B] {
            in_scope(client, tenant, &lock_row(tenant))
                .await
                .unwrap_or_else(|e| panic!("the lock row of {tenant}: {e}"));
        }
    }

    /// A migrated database with the lock rows of both tenants, and the
    /// application role's connection.
    async fn with_two_tenants() -> (Db, Client) {
        let db = Db::migrated().await;
        let app = db.app().await;
        two_tenants(&app).await;
        (db, app)
    }

    // What must work.

    #[tokio::test]
    async fn a_registry_transaction_lists_the_lock_row_of_every_tenant() {
        let (_db, app) = with_two_tenants().await;
        assert_eq!(
            one(&app, "registry", LIST).await,
            format!("{TENANT_A},{TENANT_B}"),
            "both tenants, to the registry"
        );
    }

    #[tokio::test]
    async fn the_owner_lists_them_in_a_registry_transaction_too() {
        let (db, _app) = with_two_tenants().await;
        assert_eq!(
            one(&owner(&db).await, "registry", LIST).await,
            format!("{TENANT_A},{TENANT_B}"),
            "the policy is one for both roles"
        );
    }

    #[tokio::test]
    async fn a_tenant_transaction_reads_its_own_lock_row() {
        let (_db, app) = with_two_tenants().await;
        for tenant in [TENANT_A, TENANT_B] {
            assert_eq!(one(&app, tenant, LIST).await, tenant, "its own row");
        }
    }

    #[tokio::test]
    async fn a_tenant_write_still_locks_its_row() {
        let (_db, app) = with_two_tenants().await;
        assert_eq!(
            one(
                &app,
                TENANT_A,
                &format!(
                    "SELECT encode(tenant, 'hex') FROM vcon.tenant_lock \
                     WHERE tenant = '\\x{TENANT_A}' FOR UPDATE"
                )
            )
            .await,
            TENANT_A,
            "the row a write of the tenant holds to its end"
        );
    }

    #[tokio::test]
    async fn a_tenant_transaction_deletes_its_own_lock_row() {
        let (_db, app) = with_two_tenants().await;
        in_scope(&app, TENANT_A, "DELETE FROM vcon.tenant_lock")
            .await
            .expect("deletes its row");
        assert_eq!(
            one(&app, "registry", LIST).await,
            TENANT_B,
            "the other tenant's row is there, and the registry sees the removal"
        );
    }

    #[tokio::test]
    async fn a_version_9_database_is_brought_to_version_10_with_its_lock_rows() {
        let db = at_version_9().await;
        // A server is refused at version 9, so the owner writes the rows.
        two_tenants(&owner(&db).await).await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 9,
                to: SCHEMA_VERSION
            },
            "migrate brings it to this binary's version"
        );
        assert_eq!(
            one(&db.app().await, "registry", LIST).await,
            format!("{TENANT_A},{TENANT_B}"),
            "the rows made under version 9"
        );
    }

    #[tokio::test]
    async fn a_new_database_has_the_four_policies_and_not_the_old_one() {
        let db = Db::migrated().await;
        assert_eq!(
            one(&db.app().await, "registry", POLICIES).await,
            "lock_deletes DELETE,lock_inserts INSERT,lock_reads SELECT,lock_updates UPDATE",
            "one policy for each kind of statement"
        );
    }

    #[tokio::test]
    async fn migrating_twice_from_version_9_does_the_work_once() {
        let db = at_version_9().await;
        db.migrate().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "nothing to do the second time"
        );
        assert_eq!(
            one(&db.app().await, "registry", POLICIES).await,
            "lock_deletes DELETE,lock_inserts INSERT,lock_reads SELECT,lock_updates UPDATE",
            "the policies once"
        );
    }

    #[tokio::test]
    async fn the_registry_reads_one_tenant_s_row_by_its_id() {
        let (_db, app) = with_two_tenants().await;
        assert_eq!(
            one(
                &app,
                "registry",
                &format!("SELECT count(*) FROM vcon.tenant_lock WHERE tenant = '\\x{TENANT_B}'")
            )
            .await,
            "1",
            "a condition on the tenant finds the row"
        );
    }

    // What must be refused.

    #[tokio::test]
    async fn a_registry_transaction_makes_no_lock_row() {
        let (_db, app) = with_two_tenants().await;
        let other = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            refused(&app, "registry", &lock_row(other)).await,
            THE_REGISTRY_S,
            "an INSERT asks for a tenant"
        );
        assert_eq!(
            one(&app, "registry", LIST).await,
            format!("{TENANT_A},{TENANT_B}"),
            "no row was made"
        );
    }

    #[tokio::test]
    async fn a_registry_transaction_changes_no_lock_row() {
        let (_db, app) = with_two_tenants().await;
        assert_eq!(
            refused(
                &app,
                "registry",
                &format!("UPDATE vcon.tenant_lock SET tenant = '\\x{TENANT_A}'")
            )
            .await,
            THE_REGISTRY_S,
            "an UPDATE asks for a tenant"
        );
    }

    #[tokio::test]
    async fn a_registry_transaction_deletes_no_lock_row() {
        let (_db, app) = with_two_tenants().await;
        for sql in [
            "DELETE FROM vcon.tenant_lock".to_owned(),
            format!("DELETE FROM vcon.tenant_lock WHERE tenant = '\\x{TENANT_A}'"),
        ] {
            assert_eq!(
                refused(&app, "registry", &sql).await,
                THE_REGISTRY_S,
                "{sql}"
            );
        }
        assert_eq!(
            one(&app, "registry", LIST).await,
            format!("{TENANT_A},{TENANT_B}"),
            "both rows are there"
        );
    }

    #[tokio::test]
    async fn a_registry_transaction_locks_no_row() {
        let (_db, app) = with_two_tenants().await;
        assert_eq!(
            refused(
                &app,
                "registry",
                "SELECT tenant FROM vcon.tenant_lock FOR UPDATE"
            )
            .await,
            THE_REGISTRY_S,
            "a lock on a row is a tenant's"
        );
    }

    #[tokio::test]
    async fn the_owner_writes_no_lock_row_in_a_registry_transaction_either() {
        let (db, _app) = with_two_tenants().await;
        assert_eq!(
            refused(
                &owner(&db).await,
                "registry",
                "DELETE FROM vcon.tenant_lock"
            )
            .await,
            THE_REGISTRY_S,
            "the policies are forced on the owner"
        );
    }

    #[tokio::test]
    async fn a_tenant_transaction_sees_only_its_own_lock_row() {
        let (_db, app) = with_two_tenants().await;
        assert_eq!(
            one(&app, TENANT_A, "SELECT count(*) FROM vcon.tenant_lock").await,
            "1",
            "one row of two"
        );
        assert_eq!(
            one(
                &app,
                TENANT_A,
                &format!("SELECT count(*) FROM vcon.tenant_lock WHERE tenant = '\\x{TENANT_B}'")
            )
            .await,
            "0",
            "the other tenant's row is not found by its id"
        );
    }

    #[tokio::test]
    async fn a_tenant_makes_no_lock_row_for_another_tenant() {
        let db = Db::migrated().await;
        let e = refused(&db.app().await, TENANT_A, &lock_row(TENANT_B)).await;
        assert!(
            e.contains("row-level security policy"),
            "the check of an INSERT: {e}"
        );
    }

    #[tokio::test]
    async fn a_tenant_deletes_no_lock_row_of_another_tenant() {
        let (_db, app) = with_two_tenants().await;
        in_scope(
            &app,
            TENANT_A,
            &format!("DELETE FROM vcon.tenant_lock WHERE tenant = '\\x{TENANT_B}'"),
        )
        .await
        .expect("finds no row");
        assert_eq!(
            one(&app, "registry", LIST).await,
            format!("{TENANT_A},{TENANT_B}"),
            "both rows are there"
        );
    }

    #[tokio::test]
    async fn a_tenant_moves_its_lock_row_to_no_other_tenant() {
        let (_db, app) = with_two_tenants().await;
        let other = "0123456789abcdef0123456789abcdef";
        let e = refused(
            &app,
            TENANT_A,
            &format!("UPDATE vcon.tenant_lock SET tenant = '\\x{other}'"),
        )
        .await;
        assert!(
            e.contains("row-level security policy"),
            "the check of an UPDATE: {e}"
        );
    }

    #[tokio::test]
    async fn a_transaction_with_no_tenant_reads_no_lock_row() {
        let (_db, app) = with_two_tenants().await;
        assert_eq!(
            refused(&app, "", LIST).await,
            NO_TENANT,
            "an empty marker is no registry"
        );
    }

    #[tokio::test]
    async fn a_marker_that_is_almost_the_registry_s_reads_no_lock_row() {
        let (_db, app) = with_two_tenants().await;
        for marker in ["Registry", "registry ", "registr"] {
            assert_eq!(
                refused(&app, marker, LIST).await,
                "vcon.tenant is not a tenant id",
                "{marker:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_database_at_version_9_refuses_the_registry_s_read() {
        let db = at_version_9().await;
        let owner = owner(&db).await;
        two_tenants(&owner).await;
        assert_eq!(
            refused(&owner, "registry", LIST).await,
            THE_REGISTRY_S,
            "the policy of version 1 asks for a tenant"
        );
    }

    #[tokio::test]
    async fn a_version_10_that_fails_leaves_version_9_whole() {
        let db = at_version_9().await;
        let broken = format!("{}\nSELECT boom;", MIGRATIONS[THIS]);
        let mut migrations: Vec<&str> = MIGRATIONS[..THIS].to_vec();
        migrations.push(&broken);
        let e = db.migrate_with(&migrations).await.expect_err("fails");
        assert!(e.starts_with("version 10: "), "names the version: {e}");
        assert_eq!(
            one(&owner(&db).await, "registry", POLICIES).await,
            "tenant_rows ALL",
            "the policy of version 1 was put back with the rest"
        );
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 9,
                to: SCHEMA_VERSION
            },
            "and the real migration then runs from 9"
        );
    }

    #[tokio::test]
    async fn the_registry_still_reads_no_other_table_of_a_tenant() {
        let (_db, app) = with_two_tenants().await;
        for table in ["tenant_catalog", "vcon", "version", "audit", "key_use"] {
            assert_eq!(
                refused(
                    &app,
                    "registry",
                    &format!("SELECT count(*) FROM vcon.{table}")
                )
                .await,
                THE_REGISTRY_S,
                "{table}"
            );
        }
    }

    #[tokio::test]
    async fn the_application_role_cannot_put_the_old_policy_back() {
        let db = Db::migrated().await;
        let e = refused(
            &db.app().await,
            "registry",
            "DROP POLICY lock_reads ON vcon.tenant_lock",
        )
        .await;
        assert!(e.contains("must be owner"), "only the owner: {e}");
    }

    // The edges.

    #[tokio::test]
    async fn the_registry_lists_nothing_when_no_tenant_has_a_lock_row() {
        let db = Db::migrated().await;
        assert_eq!(
            one(&db.app().await, "registry", LIST).await,
            "",
            "no row and no error"
        );
    }

    #[tokio::test]
    async fn with_no_lock_row_a_read_with_no_tenant_ends_with_nothing() {
        let db = Db::migrated().await;
        // The planner does not evaluate the CASE, and no row asks for the
        // tenant: nothing is returned, where the other tables refuse.
        let rows = in_scope(&db.app().await, "", "SELECT tenant FROM vcon.tenant_lock")
            .await
            .expect("no row asks for a tenant");
        assert!(
            !rows
                .iter()
                .any(|m| matches!(m, tokio_postgres::SimpleQueryMessage::Row(_))),
            "no row is returned"
        );
    }

    #[tokio::test]
    async fn a_removed_tenant_is_gone_from_the_registry_s_list() {
        let (_db, app) = with_two_tenants().await;
        for tenant in [TENANT_A, TENANT_B] {
            in_scope(&app, tenant, "DELETE FROM vcon.tenant_lock")
                .await
                .expect("deletes its row");
        }
        assert_eq!(one(&app, "registry", LIST).await, "", "none is left");
    }

    #[tokio::test]
    async fn a_tenant_makes_its_lock_row_once() {
        let (_db, app) = with_two_tenants().await;
        let e = refused(&app, TENANT_A, &lock_row(TENANT_A)).await;
        assert!(e.contains("duplicate key"), "the primary key holds: {e}");
        in_scope(
            &app,
            TENANT_A,
            &format!("{} ON CONFLICT DO NOTHING", lock_row(TENANT_A)),
        )
        .await
        .expect("a create that asks for it");
    }

    #[tokio::test]
    async fn a_registry_read_rolled_back_and_a_tenant_read_after_it_see_their_own() {
        let (_db, app) = with_two_tenants().await;
        // One connection, as a pool gives it out: the marker is the
        // transaction's and does not stay.
        assert_eq!(
            one(&app, "registry", "SELECT count(*) FROM vcon.tenant_lock").await,
            "2",
            "the registry"
        );
        assert_eq!(
            one(&app, TENANT_B, LIST).await,
            TENANT_B,
            "then a tenant, on the same connection"
        );
        assert_eq!(
            refused(&app, "", LIST).await,
            NO_TENANT,
            "then no marker at all"
        );
    }

    #[tokio::test]
    async fn a_tenant_with_no_lock_row_reads_none() {
        let (_db, app) = with_two_tenants().await;
        assert_eq!(
            one(&app, "0123456789abcdef0123456789abcdef", LIST).await,
            "",
            "a tenant id with no row sees no other's"
        );
    }

    #[tokio::test]
    async fn the_owner_as_a_tenant_sees_only_that_tenant_s_row() {
        let (db, _app) = with_two_tenants().await;
        assert_eq!(
            one(&owner(&db).await, TENANT_B, LIST).await,
            TENANT_B,
            "forced on the owner"
        );
    }
}
