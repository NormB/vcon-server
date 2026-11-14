//! Schema version 11: each tenant's audit log has a key of its own. The
//! table `audit_key` holds it sealed under the tenant's catalog key, and a
//! record names the key that sealed its target. A rotation of the tenant
//! key seals the audit key again and changes no record. Each live test of
//! the migration starts from a database at version 10.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 10;

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
fn version_11_is_the_eleventh_migration_and_makes_the_table_of_audit_keys() {
    assert_eq!(
        text().matches("CREATE TABLE vcon.audit_key (").count(),
        1,
        "the eleventh makes the one table"
    );
}

#[test]
fn the_binary_s_version_is_at_least_11() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn version_11_adds_the_column_that_names_a_record_s_key() {
    assert_eq!(
        statements(text())
            .iter()
            .filter(|line| **line == "ALTER TABLE vcon.audit ADD COLUMN key_id bytea;")
            .count(),
        1,
        "one column, with no default and no NOT NULL: no row is rewritten"
    );
}

#[test]
fn the_migration_says_why_no_record_is_converted() {
    let comment = comment();
    assert!(
        comment.contains(
            "No deployed database has an audit record: a server of the preview stops after \
             it connects. So no record is converted"
        ),
        "the reason, in the migration's comment: {comment}"
    );
}

#[test]
fn the_how_to_says_what_version_11_changes() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    assert!(
        page.contains(
            "Version 11 gives each tenant's audit log a key of its own. The table \
             `audit_key` holds that key, sealed under the tenant's catalog key, and each \
             record in `audit` names the key that sealed its target"
        ),
        "the paragraph on version 11"
    );
}

#[test]
fn the_how_to_says_the_audit_key_does_not_rotate_with_the_tenant_key() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    assert!(
        page.contains(
            "The audit key does not rotate with the tenant key: a rotation seals the same \
             audit key under the new catalog key and changes no audit record"
        ),
        "the rule, in the how-to"
    );
}

#[test]
fn the_how_to_says_what_version_11_changes_under_schema_versions() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    let (_, versions) = page
        .split_once(" ### Schema versions ")
        .expect("the how-to has the section Schema versions");
    assert!(
        versions.contains("Version 11 gives each tenant's audit log a key of its own"),
        "version 11, in the section of the schema's versions"
    );
    assert!(
        !versions.contains("is not written yet"),
        "the re-seal is written: the section does not say it is not"
    );
}

// What must be refused.

#[test]
fn a_version_10_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(10)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 10, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn the_application_role_is_granted_no_delete_of_an_audit_key() {
    let grants: Vec<&str> = statements(text())
        .into_iter()
        .filter(|line| line.starts_with("GRANT "))
        .collect();
    assert_eq!(
        grants,
        [
            "GRANT SELECT, INSERT ON vcon.audit_key TO vcon_app;",
            "GRANT UPDATE (wrapped) ON vcon.audit_key TO vcon_app;"
        ],
        "a key is read, made and sealed again, and never deleted"
    );
}

#[test]
fn version_11_grants_nothing_on_the_logs() {
    for line in statements(text()) {
        if line.contains("GRANT") || line.contains("REVOKE") {
            assert!(
                !line.contains("vcon.audit ") && !line.contains("admin_audit"),
                "a privilege on a log: {line}"
            );
        }
    }
}

#[test]
fn version_11_replaces_no_function_and_no_trigger_and_makes_no_sequence() {
    for line in statements(text()) {
        for word in [
            "FUNCTION", "TRIGGER", "DROP ", "DISABLE", "IDENTITY", "serial",
        ] {
            assert!(
                !line.contains(word),
                "the rule that a record is not changed stays as version 9 made it: {line}"
            );
        }
    }
}

// The edges.

#[test]
fn version_11_changes_no_row() {
    for line in statements(text()) {
        let line = line.trim_start();
        for word in ["INSERT ", "UPDATE ", "DELETE ", "TRUNCATE "] {
            assert!(!line.starts_with(word), "{word}in version 11: {line}");
        }
    }
}

#[test]
fn version_11_names_the_tenants_logs_only_and_leaves_the_registry_s() {
    assert!(
        !statements(text())
            .iter()
            .any(|line| line.contains("admin_audit")),
        "the registry's log keeps the registry's keys in this version"
    );
}

#[test]
fn the_table_of_audit_keys_has_row_level_security_forced_on_its_owner() {
    let lines = statements(text());
    for line in [
        "ALTER TABLE vcon.audit_key ENABLE ROW LEVEL SECURITY;",
        "ALTER TABLE vcon.audit_key FORCE ROW LEVEL SECURITY;",
    ] {
        assert_eq!(
            lines.iter().filter(|l| **l == line).count(),
            1,
            "once: {line}"
        );
    }
}

#[test]
fn the_constraint_on_a_record_s_key_does_not_read_the_records_that_are_there() {
    let text = statements(text()).join(" ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        text.contains(
            "ADD CONSTRAINT audit_sealed_by FOREIGN KEY (tenant, key_id) REFERENCES \
             vcon.audit_key (tenant, key_id) NOT VALID;"
        ),
        "a plain ADD CONSTRAINT reads the log as the owner with no tenant set, and is refused"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use tokio_postgres::Client;

    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::live::{Db, TENANT_A, TENANT_B, in_scope};
    use super::super::v8::on_a_server::{NOT_CHANGED, one, owner, refused};
    use super::THIS;

    /// What a registry transaction is told when it asks for a tenant.
    const THE_REGISTRY_S: &str = "this transaction is the registry's, not a tenant's";

    /// What a transaction with no tenant is told.
    const NO_TENANT: &str = "no tenant set for this transaction";

    /// Every audit key a transaction sees, oldest first, as one text.
    const KEYS: &str = "SELECT coalesce(string_agg(encode(key_id, 'hex') || ':' || \
         encode(wrapped, 'hex'), ',' ORDER BY ordinal), '') FROM vcon.audit_key";

    /// Every audit record a transaction sees, in order, as one text.
    const RECORDS: &str =
        "SELECT coalesce(string_agg(a::text, ';' ORDER BY seq), '') FROM vcon.audit a";

    /// A database at version 10.
    async fn at_version_10() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 10"),
            Migrated { from: 0, to: 10 },
            "the first ten migrations alone"
        );
        db
    }

    /// The statement that makes the audit key `id` of `tenant`, wrapped
    /// as `wrapped`, both in hex, after the keys the transaction sees.
    fn key(tenant: &str, id: &str, wrapped: &str) -> String {
        format!(
            "INSERT INTO vcon.audit_key (tenant, key_id, ordinal, wrapped) \
             SELECT '\\x{tenant}', '\\x{id}', coalesce(max(ordinal), 0) + 1, '\\x{wrapped}' \
             FROM vcon.audit_key"
        )
    }

    /// The statement that makes the audit key `id` of the first tenant
    /// with the ordinal `ordinal`.
    fn key_at(id: &str, ordinal: i64) -> String {
        format!(
            "INSERT INTO vcon.audit_key (tenant, key_id, ordinal, wrapped) \
             VALUES ('\\x{TENANT_A}', '\\x{id}', {ordinal}, '\\x03')"
        )
    }

    /// The statement that makes an audit record of `tenant` whose target
    /// the key `id` sealed; `counted` marks a counted record.
    fn record(tenant: &str, id: Option<&str>, counted: bool) -> String {
        let id = id.map_or("NULL".to_owned(), |id| format!("'\\x{id}'"));
        format!(
            "INSERT INTO vcon.audit \
                 (tenant, at, actor, action, target_mac, target_sealed, status, count, counted, \
                  key_id) \
             VALUES ('\\x{tenant}', 't', 'a', 'b', '\\x01', '\\x02', 1, 1, {counted}, {id})"
        )
    }

    /// A record as version 10 holds it: there is no column for a key.
    fn old_record(tenant: &str, counted: bool) -> String {
        format!(
            "INSERT INTO vcon.audit \
                 (tenant, at, actor, action, target_mac, target_sealed, status, count, counted) \
             VALUES ('\\x{tenant}', 't', 'a', 'b', '\\x01', '\\x02', 1, 1, {counted})"
        )
    }

    async fn run(client: &Client, scope: &str, sql: &str) {
        in_scope(client, scope, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A migrated database where each tenant has the audit key `aa`, and
    /// the application role's connection.
    async fn with_keys() -> (Db, Client) {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT_A, &key(TENANT_A, "aa", "01")).await;
        run(&app, TENANT_B, &key(TENANT_B, "aa", "02")).await;
        (db, app)
    }

    // What must work.

    #[tokio::test]
    async fn a_version_10_database_is_brought_to_version_11() {
        let db = at_version_10().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..=THIS])
                .await
                .expect("version 11"),
            Migrated { from: 10, to: 11 },
            "the eleventh migration alone"
        );
    }

    #[tokio::test]
    async fn a_tenant_makes_and_reads_its_audit_key() {
        let (_db, app) = with_keys().await;
        assert_eq!(one(&app, TENANT_A, KEYS).await, "aa:01", "its own key");
        assert_eq!(one(&app, TENANT_B, KEYS).await, "aa:02", "the other's own");
    }

    #[tokio::test]
    async fn a_tenant_seals_its_audit_key_again() {
        let (_db, app) = with_keys().await;
        run(
            &app,
            TENANT_A,
            "UPDATE vcon.audit_key SET wrapped = '\\x09'",
        )
        .await;
        assert_eq!(one(&app, TENANT_A, KEYS).await, "aa:09", "sealed again");
        assert_eq!(
            one(&app, TENANT_B, KEYS).await,
            "aa:02",
            "the other tenant's key is as it was"
        );
    }

    #[tokio::test]
    async fn a_record_names_the_key_that_sealed_it() {
        let (_db, app) = with_keys().await;
        run(&app, TENANT_A, &record(TENANT_A, Some("aa"), false)).await;
        assert_eq!(
            one(
                &app,
                TENANT_A,
                "SELECT encode(key_id, 'hex') FROM vcon.audit"
            )
            .await,
            "aa",
            "the id of the key"
        );
    }

    #[tokio::test]
    async fn a_record_with_no_key_is_still_written() {
        let (_db, app) = with_keys().await;
        run(&app, TENANT_A, &record(TENANT_A, None, false)).await;
        assert_eq!(
            one(&app, TENANT_A, "SELECT key_id IS NULL FROM vcon.audit").await,
            "t",
            "the column takes no value"
        );
    }

    #[tokio::test]
    async fn a_counted_record_that_names_a_key_takes_a_larger_count() {
        let (_db, app) = with_keys().await;
        run(&app, TENANT_A, &record(TENANT_A, Some("aa"), true)).await;
        run(&app, TENANT_A, "UPDATE vcon.audit SET count = count + 4").await;
        assert_eq!(
            one(&app, TENANT_A, "SELECT count FROM vcon.audit").await,
            "5",
            "the rule of version 9 reads the new column as it reads the others"
        );
    }

    #[tokio::test]
    async fn the_records_of_version_10_are_kept_as_they_were_with_no_key() {
        let db = at_version_10().await;
        // A server is refused at version 10, so the owner writes the rows.
        let owner = owner(&db).await;
        run(&owner, TENANT_A, &old_record(TENANT_A, false)).await;
        run(&owner, TENANT_A, &old_record(TENANT_A, true)).await;
        let before = one(&owner, TENANT_A, RECORDS).await;
        db.migrate().await;
        let app = db.app().await;
        let after = one(&app, TENANT_A, RECORDS).await;
        assert_eq!(
            after.replace(",)", ")"),
            before,
            "each record is the same, with one empty column more at its end"
        );
        assert_eq!(
            one(
                &app,
                TENANT_A,
                "SELECT count(*) FROM vcon.audit WHERE key_id IS NULL"
            )
            .await,
            "2",
            "no record of version 10 names a key"
        );
    }

    #[tokio::test]
    async fn a_counted_record_of_version_10_still_takes_a_larger_count() {
        let db = at_version_10().await;
        run(&owner(&db).await, TENANT_A, &old_record(TENANT_A, true)).await;
        db.migrate().await;
        let app = db.app().await;
        run(&app, TENANT_A, "UPDATE vcon.audit SET count = count + 1").await;
        assert_eq!(
            one(&app, TENANT_A, "SELECT count FROM vcon.audit").await,
            "2",
            "a record made before the column is counted on"
        );
    }

    #[tokio::test]
    async fn the_table_has_the_one_policy_of_a_tenant_s_rows() {
        let db = Db::migrated().await;
        assert_eq!(
            one(
                &db.app().await,
                "registry",
                "SELECT string_agg(policyname || ' ' || cmd, ',' ORDER BY policyname) \
                 FROM pg_policies WHERE schemaname = 'vcon' AND tablename = 'audit_key'"
            )
            .await,
            "tenant_rows ALL",
            "one policy for every kind of statement"
        );
    }

    #[tokio::test]
    async fn migrate_on_a_database_at_this_binary_s_version_changes_nothing() {
        let (db, app) = with_keys().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "nothing to apply"
        );
        assert_eq!(one(&app, TENANT_A, KEYS).await, "aa:01", "the key is kept");
    }

    // What must be refused.

    #[tokio::test]
    async fn the_application_role_cannot_delete_an_audit_key() {
        let (_db, app) = with_keys().await;
        assert_eq!(
            refused(&app, TENANT_A, "DELETE FROM vcon.audit_key").await,
            "permission denied for table audit_key",
            "an old key is kept, so that its records can be read"
        );
        assert_eq!(one(&app, TENANT_A, KEYS).await, "aa:01", "the key is there");
    }

    #[tokio::test]
    async fn the_application_role_cannot_empty_the_table() {
        let (_db, app) = with_keys().await;
        assert_eq!(
            refused(&app, TENANT_A, "TRUNCATE vcon.audit_key").await,
            "permission denied for table audit_key",
            "no privilege to empty it"
        );
    }

    #[tokio::test]
    async fn the_application_role_changes_no_column_of_a_key_but_its_sealed_copy() {
        let (_db, app) = with_keys().await;
        for set in [
            "key_id = '\\xbb'",
            "ordinal = 9",
            &format!("tenant = '\\x{TENANT_B}'"),
            "wrapped = '\\x09', key_id = '\\xbb'",
        ] {
            assert_eq!(
                refused(&app, TENANT_A, &format!("UPDATE vcon.audit_key SET {set}")).await,
                "permission denied for table audit_key",
                "SET {set}"
            );
        }
        assert_eq!(
            one(&app, TENANT_A, KEYS).await,
            "aa:01",
            "the key is as made"
        );
    }

    #[tokio::test]
    async fn a_tenant_does_not_see_or_change_another_tenant_s_key() {
        let (_db, app) = with_keys().await;
        run(
            &app,
            TENANT_A,
            &format!("UPDATE vcon.audit_key SET wrapped = '\\x09' WHERE tenant = '\\x{TENANT_B}'"),
        )
        .await;
        assert_eq!(
            one(&app, TENANT_B, KEYS).await,
            "aa:02",
            "the statement found no row of the other tenant"
        );
        assert_eq!(
            one(
                &app,
                TENANT_A,
                &format!("SELECT count(*) FROM vcon.audit_key WHERE tenant = '\\x{TENANT_B}'")
            )
            .await,
            "0",
            "and reads none"
        );
    }

    #[tokio::test]
    async fn a_tenant_cannot_make_a_key_for_another_tenant() {
        let (_db, app) = with_keys().await;
        assert_eq!(
            refused(&app, TENANT_A, &key(TENANT_B, "bb", "03")).await,
            "new row violates row-level security policy for table \"audit_key\"",
            "the policy checks the row's tenant"
        );
    }

    #[tokio::test]
    async fn a_registry_transaction_reads_no_audit_key() {
        let (db, app) = with_keys().await;
        for client in [&app, &owner(&db).await] {
            assert_eq!(
                refused(client, "registry", KEYS).await,
                THE_REGISTRY_S,
                "the registry has no tenant's key"
            );
        }
    }

    #[tokio::test]
    async fn a_transaction_with_no_tenant_reads_no_audit_key() {
        let (_db, app) = with_keys().await;
        let refusal = app
            .simple_query(KEYS)
            .await
            .expect_err("refused with no tenant");
        assert_eq!(
            refusal.as_db_error().expect("the server's").message(),
            NO_TENANT,
            "no tenant, no row"
        );
    }

    #[tokio::test]
    async fn a_record_cannot_name_a_key_that_does_not_exist() {
        let (_db, app) = with_keys().await;
        let why = refused(&app, TENANT_A, &record(TENANT_A, Some("cc"), false)).await;
        assert!(
            why.contains("violates foreign key constraint \"audit_sealed_by\""),
            "the key must be the tenant's: {why}"
        );
    }

    #[tokio::test]
    async fn a_record_cannot_name_a_key_only_another_tenant_has() {
        let (_db, app) = with_keys().await;
        run(&app, TENANT_B, &key(TENANT_B, "bb", "03")).await;
        let why = refused(&app, TENANT_A, &record(TENANT_A, Some("bb"), false)).await;
        assert!(
            why.contains("violates foreign key constraint \"audit_sealed_by\""),
            "the tenant and the id together: {why}"
        );
    }

    #[tokio::test]
    async fn the_key_a_record_names_is_never_changed() {
        let (db, app) = with_keys().await;
        run(&app, TENANT_A, &key(TENANT_A, "bb", "03")).await;
        run(&app, TENANT_A, &record(TENANT_A, Some("aa"), true)).await;
        assert_eq!(
            refused(&app, TENANT_A, "UPDATE vcon.audit SET key_id = '\\xbb'").await,
            "permission denied for table audit",
            "the application role updates a count and nothing else"
        );
        for set in [
            "key_id = '\\xbb'",
            "key_id = '\\xbb', count = count + 1",
            "key_id = NULL",
        ] {
            assert_eq!(
                refused(
                    &owner(&db).await,
                    TENANT_A,
                    &format!("UPDATE vcon.audit SET {set}")
                )
                .await,
                NOT_CHANGED,
                "the owner too: SET {set}"
            );
        }
    }

    #[tokio::test]
    async fn the_owner_cannot_delete_a_key_that_sealed_a_record() {
        let (db, app) = with_keys().await;
        run(&app, TENANT_A, &record(TENANT_A, Some("aa"), false)).await;
        let why = refused(&owner(&db).await, TENANT_A, "DELETE FROM vcon.audit_key").await;
        assert!(
            why.contains("violates foreign key constraint \"audit_sealed_by\""),
            "the record keeps its key: {why}"
        );
    }

    #[tokio::test]
    async fn a_tenant_cannot_have_two_keys_with_one_id() {
        let (_db, app) = with_keys().await;
        let why = refused(&app, TENANT_A, &key(TENANT_A, "aa", "03")).await;
        assert!(
            why.contains("duplicate key value violates unique constraint"),
            "the id is the key's name within its tenant: {why}"
        );
    }

    #[tokio::test]
    async fn a_key_needs_its_sealed_copy() {
        let (_db, app) = with_keys().await;
        let why = refused(
            &app,
            TENANT_A,
            &format!(
                "INSERT INTO vcon.audit_key (tenant, key_id, ordinal) \
                 VALUES ('\\x{TENANT_A}', '\\xbb', 2)"
            ),
        )
        .await;
        assert!(
            why.contains("null value in column \"wrapped\""),
            "a key that is not stored cannot open a record: {why}"
        );
    }

    #[tokio::test]
    async fn a_tenant_cannot_have_two_keys_with_one_ordinal() {
        let (_db, app) = with_keys().await;
        let why = refused(&app, TENANT_A, &key_at("bb", 1)).await;
        assert!(
            why.contains("duplicate key value violates unique constraint"),
            "one key is the newest: {why}"
        );
    }

    #[tokio::test]
    async fn a_key_s_ordinal_is_one_or_more() {
        let (_db, app) = with_keys().await;
        for ordinal in [0, -1] {
            let why = refused(&app, TENANT_A, &key_at("bb", ordinal)).await;
            assert!(
                why.contains("violates check constraint"),
                "an ordinal of {ordinal}: {why}"
            );
        }
    }

    // The edges.

    #[tokio::test]
    async fn a_key_s_ordinal_need_not_follow_the_one_before_it() {
        let (_db, app) = with_keys().await;
        run(&app, TENANT_A, &key_at("05", 7)).await;
        run(&app, TENANT_A, &key_at("bb", 3)).await;
        assert_eq!(
            one(&app, TENANT_A, KEYS).await,
            "aa:01,bb:03,05:03",
            "the keys in the order of their ordinals"
        );
    }

    #[tokio::test]
    async fn a_tenant_s_keys_are_read_oldest_first() {
        let (_db, app) = with_keys().await;
        // Ids that sort the other way, so that the order is the making's.
        run(&app, TENANT_A, &key(TENANT_A, "05", "03")).await;
        run(&app, TENANT_A, &key(TENANT_A, "ff", "04")).await;
        assert_eq!(
            one(&app, TENANT_A, KEYS).await,
            "aa:01,05:03,ff:04",
            "the newest is the last"
        );
    }

    #[tokio::test]
    async fn two_tenants_may_have_keys_with_one_id() {
        let (_db, app) = with_keys().await;
        run(&app, TENANT_A, &record(TENANT_A, Some("aa"), false)).await;
        run(&app, TENANT_B, &record(TENANT_B, Some("aa"), false)).await;
        for tenant in [TENANT_A, TENANT_B] {
            assert_eq!(
                one(&app, tenant, "SELECT count(*) FROM vcon.audit").await,
                "1",
                "each tenant's record names its own key"
            );
        }
    }

    #[tokio::test]
    async fn records_under_two_keys_and_under_none_are_in_one_log() {
        let (_db, app) = with_keys().await;
        run(&app, TENANT_A, &key(TENANT_A, "bb", "03")).await;
        for id in [Some("aa"), Some("bb"), None] {
            run(&app, TENANT_A, &record(TENANT_A, id, false)).await;
        }
        assert_eq!(
            one(
                &app,
                TENANT_A,
                "SELECT string_agg(coalesce(encode(key_id, 'hex'), 'none'), ',' ORDER BY seq) \
                 FROM vcon.audit"
            )
            .await,
            "aa,bb,none",
            "each record keeps the key it was sealed under"
        );
    }

    #[tokio::test]
    async fn sealing_a_key_again_changes_no_record_that_names_it() {
        let (_db, app) = with_keys().await;
        run(&app, TENANT_A, &record(TENANT_A, Some("aa"), false)).await;
        let before = one(&app, TENANT_A, RECORDS).await;
        run(
            &app,
            TENANT_A,
            "UPDATE vcon.audit_key SET wrapped = '\\x09'",
        )
        .await;
        assert_eq!(
            one(&app, TENANT_A, RECORDS).await,
            before,
            "the record is as it was written"
        );
    }

    #[tokio::test]
    async fn the_table_s_columns_are_the_four_of_this_version() {
        let db = Db::migrated().await;
        assert_eq!(
            one(
                &db.app().await,
                "registry",
                "SELECT string_agg(column_name || ' ' || data_type || ' ' || is_nullable, ',' \
                 ORDER BY ordinal_position) FROM information_schema.columns \
                 WHERE table_schema = 'vcon' AND table_name = 'audit_key'"
            )
            .await,
            "tenant bytea NO,key_id bytea NO,ordinal bigint NO,wrapped bytea NO",
            "the tenant, the key's id, its order and its sealed copy"
        );
    }

    #[tokio::test]
    async fn the_column_of_a_record_s_key_is_the_last_and_takes_no_value() {
        let db = Db::migrated().await;
        assert_eq!(
            one(
                &db.app().await,
                "registry",
                "SELECT column_name || ' ' || data_type || ' ' || is_nullable || ' ' || \
                 coalesce(column_default, 'none') FROM information_schema.columns \
                 WHERE table_schema = 'vcon' AND table_name = 'audit' \
                 ORDER BY ordinal_position DESC LIMIT 1"
            )
            .await,
            "key_id bytea YES none",
            "a column a record of version 10 has no value in"
        );
    }
}
