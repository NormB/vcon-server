//! Schema version 12: the database names the catalog key the registry's
//! columns are sealed under, in the one row of the table `registry_key`,
//! and the registry's audit log has a key of its own, as each tenant's log
//! has since version 11: the table `admin_audit_key` holds it sealed under
//! the registry's keys, and a record of `admin_audit` names the key that
//! sealed its target. Each live test of the migration starts from a
//! database at version 11.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 11;

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

/// The migration's statements as one line, every run of white space one
/// space.
fn one_line() -> String {
    statements(text())
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
fn version_12_is_the_twelfth_migration_and_makes_the_two_tables() {
    for table in ["registry_key", "admin_audit_key"] {
        assert_eq!(
            text()
                .matches(&format!("CREATE TABLE vcon.{table} ("))
                .count(),
            1,
            "the twelfth makes {table} once"
        );
    }
    assert_eq!(
        text().matches("CREATE TABLE").count(),
        2,
        "and no other table"
    );
}

#[test]
fn the_binary_s_version_is_at_least_12() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn version_12_adds_the_column_that_names_a_registry_record_s_key() {
    assert_eq!(
        statements(text())
            .iter()
            .filter(|line| **line == "ALTER TABLE vcon.admin_audit ADD COLUMN key_id bytea;")
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
            "No deployed database has a row of the registry: a server of the preview stops \
             after it connects. So no record is converted"
        ),
        "the reason, in the migration's comment: {comment}"
    );
}

#[test]
fn the_how_to_says_what_version_12_changes() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    assert!(
        page.contains(
            "Version 12 makes the table `registry_key`, whose one row names the catalog key \
             the registry's columns are sealed under, and gives the registry's audit log a \
             key of its own"
        ),
        "the paragraph on version 12"
    );
}

#[test]
fn the_changelog_says_what_version_12_changes() {
    let text = flat("../../CHANGELOG.md");
    let unreleased = text
        .split(" ## [")
        .find(|section| section.starts_with("Unreleased]"))
        .expect("an Unreleased section");
    assert!(
        unreleased.contains("Version 12 names the catalog key the registry is sealed under"),
        "version 12, in the entry of the schema"
    );
}

#[test]
fn the_status_page_names_version_12() {
    let page = flat("../../website/content/docs/status.md");
    assert!(
        page.contains("Version 12 adds the table `registry_key`"),
        "in the entry of what is on main only"
    );
}

// What must be refused.

#[test]
fn a_version_11_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(11)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 11, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn the_application_role_is_granted_no_delete_of_either_table() {
    let grants: Vec<&str> = statements(text())
        .into_iter()
        .filter(|line| line.starts_with("GRANT "))
        .collect();
    assert_eq!(
        grants,
        [
            "GRANT SELECT, INSERT ON vcon.registry_key, vcon.admin_audit_key TO vcon_app;",
            "GRANT UPDATE (catalog_key_id) ON vcon.registry_key TO vcon_app;",
            "GRANT UPDATE (wrapped) ON vcon.admin_audit_key TO vcon_app;"
        ],
        "the id is read, written once and moved by a rotation; a key is read, made and \
         sealed again; neither is deleted"
    );
}

#[test]
fn version_12_grants_nothing_on_the_logs() {
    for line in statements(text()) {
        if line.contains("GRANT") || line.contains("REVOKE") {
            assert!(
                !line.contains("vcon.audit ") && !line.contains("vcon.admin_audit "),
                "a privilege on a log: {line}"
            );
        }
    }
}

#[test]
fn version_12_replaces_no_function_and_no_trigger_and_makes_no_sequence() {
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

#[test]
fn the_registry_s_key_table_takes_one_row_only() {
    assert!(
        one_line().contains(
            "CREATE TABLE vcon.registry_key ( one boolean PRIMARY KEY DEFAULT true CHECK (one), \
             catalog_key_id bytea NOT NULL );"
        ),
        "a primary key that has one value: {}",
        one_line()
    );
}

// The edges.

#[test]
fn version_12_changes_no_row() {
    for line in statements(text()) {
        let line = line.trim_start();
        for word in ["INSERT ", "UPDATE ", "DELETE ", "TRUNCATE "] {
            assert!(!line.starts_with(word), "{word}in version 12: {line}");
        }
    }
}

#[test]
fn version_12_changes_no_table_of_a_tenant() {
    for line in statements(text()) {
        for table in ["vcon.audit ", "vcon.audit_key ", "vcon.tenant_catalog"] {
            assert!(!line.contains(table), "{table}in version 12: {line}");
        }
    }
}

#[test]
fn both_tables_have_row_level_security_forced_on_their_owner() {
    let lines = statements(text());
    for table in ["registry_key", "admin_audit_key"] {
        for how in ["ENABLE", "FORCE"] {
            let line = format!("ALTER TABLE vcon.{table} {how} ROW LEVEL SECURITY;");
            assert_eq!(
                lines.iter().filter(|l| **l == line).count(),
                1,
                "once: {line}"
            );
        }
    }
}

#[test]
fn both_tables_have_the_policy_of_the_registry_s_rows() {
    let text = one_line();
    for (table, column) in [
        ("registry_key", "catalog_key_id"),
        ("admin_audit_key", "key_id"),
    ] {
        assert!(
            text.contains(&format!(
                "CREATE POLICY registry_rows ON vcon.{table} USING (vcon.in_registry() = \
                 ({column} IS NOT NULL)) WITH CHECK (vcon.in_registry());"
            )),
            "{table}: the policy version 1 gives the registry's tables: {text}"
        );
    }
}

#[test]
fn the_constraint_on_a_record_s_key_does_not_read_the_records_that_are_there() {
    assert!(
        one_line().contains(
            "ADD CONSTRAINT admin_audit_sealed_by FOREIGN KEY (key_id) REFERENCES \
             vcon.admin_audit_key (key_id) NOT VALID;"
        ),
        "a plain ADD CONSTRAINT reads the log as the owner with no marker set, and is refused"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use tokio_postgres::Client;

    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::live::{Db, TENANT_A, in_scope};
    use super::super::v8::on_a_server::{NOT_CHANGED, one, owner, refused};
    use super::THIS;

    /// The scope of a registry transaction.
    const REGISTRY: &str = "registry";

    /// What a tenant's transaction is told when it asks for the registry.
    const A_TENANT_S: &str = "this transaction is a tenant's, not the registry's";

    /// What a transaction with no marker is told.
    const NO_TENANT: &str = "no tenant set for this transaction";

    /// The id the database names, in hex, or nothing.
    const KEY_ID: &str = "SELECT coalesce(string_agg(encode(catalog_key_id, 'hex'), ','), '') FROM vcon.registry_key";

    /// Every audit key of the registry's log, oldest first, as one text.
    const KEYS: &str = "SELECT coalesce(string_agg(encode(key_id, 'hex') || ':' || \
         encode(wrapped, 'hex'), ',' ORDER BY ordinal), '') FROM vcon.admin_audit_key";

    /// Every record of the registry's log, in order, as one text.
    const RECORDS: &str =
        "SELECT coalesce(string_agg(a::text, ';' ORDER BY seq), '') FROM vcon.admin_audit a";

    /// A database at version 11.
    async fn at_version_11() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 11"),
            Migrated { from: 0, to: 11 },
            "the first eleven migrations alone"
        );
        db
    }

    /// The statement that makes the audit key `id`, wrapped as `wrapped`,
    /// both in hex, after the keys that are there.
    fn key(id: &str, wrapped: &str) -> String {
        format!(
            "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) \
             SELECT '\\x{id}', coalesce(max(ordinal), 0) + 1, '\\x{wrapped}' \
             FROM vcon.admin_audit_key"
        )
    }

    /// The statement that makes a record of the registry's log whose
    /// target the key `id` sealed; `counted` marks a counted record.
    fn record(id: Option<&str>, counted: bool) -> String {
        let id = id.map_or("NULL".to_owned(), |id| format!("'\\x{id}'"));
        format!(
            "INSERT INTO vcon.admin_audit \
                 (at, actor, action, target_mac, target_sealed, status, count, counted, key_id) \
             VALUES ('t', 'a', 'b', '\\x01', '\\x02', 1, 1, {counted}, {id})"
        )
    }

    /// A record as version 11 holds it: there is no column for a key.
    fn old_record(counted: bool) -> String {
        format!(
            "INSERT INTO vcon.admin_audit \
                 (at, actor, action, target_mac, target_sealed, status, count, counted) \
             VALUES ('t', 'a', 'b', '\\x01', '\\x02', 1, 1, {counted})"
        )
    }

    async fn run(client: &Client, scope: &str, sql: &str) {
        in_scope(client, scope, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A migrated database that names the key `c1` and whose log has the
    /// audit key `aa`, and the application role's connection.
    async fn with_keys() -> (Db, Client) {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(
            &app,
            REGISTRY,
            "INSERT INTO vcon.registry_key (catalog_key_id) VALUES ('\\xc1')",
        )
        .await;
        run(&app, REGISTRY, &key("aa", "01")).await;
        (db, app)
    }

    // What must work.

    #[tokio::test]
    async fn a_version_11_database_is_brought_to_version_12() {
        let db = at_version_11().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..=THIS])
                .await
                .expect("version 12"),
            Migrated { from: 11, to: 12 },
            "the twelfth migration alone"
        );
    }

    #[tokio::test]
    async fn a_migrated_database_names_no_key_and_has_no_audit_key() {
        let db = Db::migrated().await;
        let app = db.app().await;
        assert_eq!(one(&app, REGISTRY, KEY_ID).await, "", "no id yet");
        assert_eq!(one(&app, REGISTRY, KEYS).await, "", "no audit key yet");
    }

    #[tokio::test]
    async fn the_registry_writes_and_reads_the_id_of_its_key() {
        let (_db, app) = with_keys().await;
        assert_eq!(one(&app, REGISTRY, KEY_ID).await, "c1", "the id written");
    }

    #[tokio::test]
    async fn the_registry_moves_the_id_to_another_key() {
        let (_db, app) = with_keys().await;
        run(
            &app,
            REGISTRY,
            "UPDATE vcon.registry_key SET catalog_key_id = '\\xc2' WHERE catalog_key_id = '\\xc1'",
        )
        .await;
        assert_eq!(
            one(&app, REGISTRY, KEY_ID).await,
            "c2",
            "a rotation's write"
        );
    }

    #[tokio::test]
    async fn the_registry_makes_reads_and_seals_again_the_key_of_its_log() {
        let (_db, app) = with_keys().await;
        assert_eq!(one(&app, REGISTRY, KEYS).await, "aa:01", "the key made");
        run(
            &app,
            REGISTRY,
            "UPDATE vcon.admin_audit_key SET wrapped = '\\x09'",
        )
        .await;
        assert_eq!(one(&app, REGISTRY, KEYS).await, "aa:09", "sealed again");
    }

    #[tokio::test]
    async fn a_record_names_the_key_that_sealed_it() {
        let (_db, app) = with_keys().await;
        run(&app, REGISTRY, &record(Some("aa"), false)).await;
        assert_eq!(
            one(
                &app,
                REGISTRY,
                "SELECT encode(key_id, 'hex') FROM vcon.admin_audit"
            )
            .await,
            "aa",
            "the id of the key"
        );
    }

    #[tokio::test]
    async fn a_record_with_no_key_is_still_written() {
        let (_db, app) = with_keys().await;
        run(&app, REGISTRY, &record(None, false)).await;
        assert_eq!(
            one(
                &app,
                REGISTRY,
                "SELECT key_id IS NULL FROM vcon.admin_audit"
            )
            .await,
            "t",
            "the column takes no value"
        );
    }

    #[tokio::test]
    async fn a_counted_record_that_names_a_key_takes_a_larger_count() {
        let (_db, app) = with_keys().await;
        run(&app, REGISTRY, &record(Some("aa"), true)).await;
        run(
            &app,
            REGISTRY,
            "UPDATE vcon.admin_audit SET count = count + 4",
        )
        .await;
        assert_eq!(
            one(&app, REGISTRY, "SELECT count FROM vcon.admin_audit").await,
            "5",
            "the rule of version 9 reads the new column as it reads the others"
        );
    }

    #[tokio::test]
    async fn the_records_of_version_11_are_kept_as_they_were_with_no_key() {
        let db = at_version_11().await;
        // A server is refused at version 11, so the owner writes the rows.
        let owner = owner(&db).await;
        run(&owner, REGISTRY, &old_record(false)).await;
        run(&owner, REGISTRY, &old_record(true)).await;
        let before = one(&owner, REGISTRY, RECORDS).await;
        db.migrate().await;
        let app = db.app().await;
        let after = one(&app, REGISTRY, RECORDS).await;
        assert_eq!(
            after.replace(",)", ")"),
            before,
            "each record is the same, with one empty column more at its end"
        );
        assert_eq!(
            one(
                &app,
                REGISTRY,
                "SELECT count(*) FROM vcon.admin_audit WHERE key_id IS NULL"
            )
            .await,
            "2",
            "no record of version 11 names a key"
        );
    }

    #[tokio::test]
    async fn a_counted_record_of_version_11_still_takes_a_larger_count() {
        let db = at_version_11().await;
        run(&owner(&db).await, REGISTRY, &old_record(true)).await;
        db.migrate().await;
        let app = db.app().await;
        run(
            &app,
            REGISTRY,
            "UPDATE vcon.admin_audit SET count = count + 1",
        )
        .await;
        assert_eq!(
            one(&app, REGISTRY, "SELECT count FROM vcon.admin_audit").await,
            "2",
            "a record made before the column is counted on"
        );
    }

    #[tokio::test]
    async fn each_table_has_the_one_policy_of_the_registry_s_rows() {
        let db = Db::migrated().await;
        for table in ["registry_key", "admin_audit_key"] {
            assert_eq!(
                one(
                    &db.app().await,
                    REGISTRY,
                    &format!(
                        "SELECT string_agg(policyname || ' ' || cmd, ',' ORDER BY policyname) \
                         FROM pg_policies WHERE schemaname = 'vcon' AND tablename = '{table}'"
                    )
                )
                .await,
                "registry_rows ALL",
                "{table}: one policy for every kind of statement"
            );
        }
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
        assert_eq!(one(&app, REGISTRY, KEY_ID).await, "c1", "the id is kept");
        assert_eq!(one(&app, REGISTRY, KEYS).await, "aa:01", "the key is kept");
    }

    // What must be refused.

    #[tokio::test]
    async fn the_database_names_one_key_only() {
        let (_db, app) = with_keys().await;
        let why = refused(
            &app,
            REGISTRY,
            "INSERT INTO vcon.registry_key (catalog_key_id) VALUES ('\\xc2')",
        )
        .await;
        assert!(
            why.contains("duplicate key value violates unique constraint"),
            "a second row: {why}"
        );
        let why = refused(
            &app,
            REGISTRY,
            "INSERT INTO vcon.registry_key (one, catalog_key_id) VALUES (false, '\\xc2')",
        )
        .await;
        assert!(
            why.contains("violates check constraint"),
            "a row under the other value of the key column: {why}"
        );
        assert_eq!(one(&app, REGISTRY, KEY_ID).await, "c1", "the one id");
    }

    #[tokio::test]
    async fn the_application_role_cannot_delete_the_id_or_a_key() {
        let (_db, app) = with_keys().await;
        for table in ["registry_key", "admin_audit_key"] {
            assert_eq!(
                refused(&app, REGISTRY, &format!("DELETE FROM vcon.{table}")).await,
                format!("permission denied for table {table}"),
                "DELETE"
            );
            assert_eq!(
                refused(&app, REGISTRY, &format!("TRUNCATE vcon.{table}")).await,
                format!("permission denied for table {table}"),
                "TRUNCATE"
            );
        }
        assert_eq!(one(&app, REGISTRY, KEY_ID).await, "c1", "the id is there");
        assert_eq!(one(&app, REGISTRY, KEYS).await, "aa:01", "the key is there");
    }

    #[tokio::test]
    async fn the_application_role_changes_no_column_of_a_key_but_its_sealed_copy() {
        let (_db, app) = with_keys().await;
        for set in [
            "key_id = '\\xbb'",
            "ordinal = 9",
            "wrapped = '\\x09', key_id = '\\xbb'",
        ] {
            assert_eq!(
                refused(
                    &app,
                    REGISTRY,
                    &format!("UPDATE vcon.admin_audit_key SET {set}")
                )
                .await,
                "permission denied for table admin_audit_key",
                "SET {set}"
            );
        }
        assert_eq!(
            refused(&app, REGISTRY, "UPDATE vcon.registry_key SET one = false").await,
            "permission denied for table registry_key",
            "the id's row stays the one row"
        );
        assert_eq!(
            one(&app, REGISTRY, KEYS).await,
            "aa:01",
            "the key is as made"
        );
    }

    #[tokio::test]
    async fn a_tenant_s_transaction_reads_neither_table() {
        let (_db, app) = with_keys().await;
        for sql in [KEY_ID, KEYS] {
            assert_eq!(
                refused(&app, TENANT_A, sql).await,
                A_TENANT_S,
                "a tenant asks: {sql}"
            );
        }
    }

    #[tokio::test]
    async fn a_tenant_s_transaction_writes_neither_table() {
        let (_db, app) = with_keys().await;
        for sql in [
            "INSERT INTO vcon.registry_key (catalog_key_id) VALUES ('\\xc2')".to_owned(),
            "UPDATE vcon.registry_key SET catalog_key_id = '\\xc2'".to_owned(),
            key("bb", "02"),
            "UPDATE vcon.admin_audit_key SET wrapped = '\\x09'".to_owned(),
        ] {
            assert_eq!(
                refused(&app, TENANT_A, &sql).await,
                A_TENANT_S,
                "a tenant writes: {sql}"
            );
        }
        assert_eq!(one(&app, REGISTRY, KEY_ID).await, "c1", "the id is as made");
        assert_eq!(
            one(&app, REGISTRY, KEYS).await,
            "aa:01",
            "the key is as made"
        );
    }

    #[tokio::test]
    async fn a_transaction_with_no_marker_reads_neither_table() {
        let (_db, app) = with_keys().await;
        for sql in [KEY_ID, KEYS] {
            let why = super::super::live::no_scope(&app, sql)
                .await
                .expect_err("refused");
            assert_eq!(why, NO_TENANT, "no marker: {sql}");
        }
    }

    #[tokio::test]
    async fn a_record_cannot_name_a_key_the_log_does_not_have() {
        let (_db, app) = with_keys().await;
        let why = refused(&app, REGISTRY, &record(Some("bb"), false)).await;
        assert!(
            why.contains("violates foreign key constraint \"admin_audit_sealed_by\""),
            "a record that names no key of the log: {why}"
        );
        assert_eq!(one(&app, REGISTRY, RECORDS).await, "", "nothing is written");
    }

    #[tokio::test]
    async fn a_key_that_a_record_names_is_not_deleted_by_the_owner() {
        let (db, app) = with_keys().await;
        run(&app, REGISTRY, &record(Some("aa"), false)).await;
        let why = refused(
            &owner(&db).await,
            REGISTRY,
            "DELETE FROM vcon.admin_audit_key",
        )
        .await;
        assert!(
            why.contains("violates foreign key constraint \"admin_audit_sealed_by\""),
            "the record would name a key that is gone: {why}"
        );
        assert_eq!(one(&app, REGISTRY, KEYS).await, "aa:01", "the key is there");
    }

    #[tokio::test]
    async fn the_key_a_record_names_is_not_changed() {
        let (db, app) = with_keys().await;
        run(&app, REGISTRY, &key("bb", "02")).await;
        run(&app, REGISTRY, &record(Some("aa"), true)).await;
        for client in [&app, &owner(&db).await] {
            let why = refused(
                client,
                REGISTRY,
                "UPDATE vcon.admin_audit SET key_id = '\\xbb'",
            )
            .await;
            assert!(
                why == NOT_CHANGED || why == "permission denied for table admin_audit",
                "the record keeps the key it names: {why}"
            );
        }
        assert_eq!(
            one(
                &app,
                REGISTRY,
                "SELECT encode(key_id, 'hex') FROM vcon.admin_audit"
            )
            .await,
            "aa",
            "as written"
        );
    }

    #[tokio::test]
    async fn two_keys_cannot_share_an_id_or_an_ordinal() {
        let (_db, app) = with_keys().await;
        for sql in [
            "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) VALUES ('\\xaa', 2, '\\x03')",
            "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) VALUES ('\\xbb', 1, '\\x03')",
        ] {
            let why = refused(&app, REGISTRY, sql).await;
            assert!(
                why.contains("duplicate key value violates unique constraint"),
                "{sql}: {why}"
            );
        }
    }

    #[tokio::test]
    async fn a_failed_migration_leaves_version_11_whole() {
        let db = at_version_11().await;
        let broken = format!("{}\nSELECT 1 / 0;", MIGRATIONS[THIS]);
        let mut with: Vec<&str> = MIGRATIONS[..THIS].to_vec();
        with.push(&broken);
        let why = db.migrate_with(&with).await.expect_err("refused");
        assert!(why.contains("division by zero"), "the cause: {why}");
        assert_eq!(
            one(
                &owner(&db).await,
                REGISTRY,
                "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'vcon' \
                 AND table_name IN ('registry_key', 'admin_audit_key')"
            )
            .await,
            "0",
            "neither table is left behind"
        );
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..=THIS]).await.expect("again"),
            Migrated { from: 11, to: 12 },
            "the database was still at version 11"
        );
    }

    // The edges.

    #[tokio::test]
    async fn an_ordinal_is_positive() {
        let (_db, app) = with_keys().await;
        for ordinal in [0, -1] {
            let why = refused(
                &app,
                REGISTRY,
                &format!(
                    "INSERT INTO vcon.admin_audit_key (key_id, ordinal, wrapped) \
                     VALUES ('\\xbb', {ordinal}, '\\x03')"
                ),
            )
            .await;
            assert!(
                why.contains("violates check constraint"),
                "ordinal {ordinal}: {why}"
            );
        }
    }

    #[tokio::test]
    async fn a_second_key_comes_after_the_first() {
        let (_db, app) = with_keys().await;
        run(&app, REGISTRY, &key("bb", "02")).await;
        assert_eq!(
            one(&app, REGISTRY, KEYS).await,
            "aa:01,bb:02",
            "oldest first"
        );
        run(&app, REGISTRY, &record(Some("aa"), false)).await;
        run(&app, REGISTRY, &record(Some("bb"), false)).await;
        assert_eq!(
            one(
                &app,
                REGISTRY,
                "SELECT string_agg(encode(key_id, 'hex'), ',' ORDER BY seq) FROM vcon.admin_audit"
            )
            .await,
            "aa,bb",
            "a record under each key"
        );
    }

    #[tokio::test]
    async fn a_key_no_record_names_is_deleted_by_the_owner_only() {
        let (db, app) = with_keys().await;
        run(
            &owner(&db).await,
            REGISTRY,
            "DELETE FROM vcon.admin_audit_key",
        )
        .await;
        assert_eq!(one(&app, REGISTRY, KEYS).await, "", "the owner's delete");
    }

    #[tokio::test]
    async fn the_owner_too_is_under_the_policy_of_each_table() {
        let (db, _app) = with_keys().await;
        let owner = owner(&db).await;
        for sql in [KEY_ID, KEYS] {
            assert_eq!(
                refused(&owner, TENANT_A, sql).await,
                A_TENANT_S,
                "forced on the owner: {sql}"
            );
        }
    }

    #[tokio::test]
    async fn migrating_twice_does_the_work_once() {
        let db = at_version_11().await;
        db.migrate().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "the second run finds this binary's version"
        );
    }

    #[tokio::test]
    async fn the_column_of_a_record_s_key_has_no_default() {
        let db = Db::migrated().await;
        assert_eq!(
            one(
                &db.app().await,
                REGISTRY,
                "SELECT is_nullable || ' ' || coalesce(column_default, 'none') \
                 FROM information_schema.columns WHERE table_schema = 'vcon' \
                 AND table_name = 'admin_audit' AND column_name = 'key_id'"
            )
            .await,
            "YES none",
            "no value for the records that are there"
        );
    }
}
