//! Schema version 16: a tenant's row records the key-encryption key a key
//! backend wrapped its tenant key under, and the registry's key row the
//! registry root's `key-wrap` blob. Each live test of the migration starts
//! from a database at version 15.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 15;

/// This version's migration, or a panic that says there is none.
fn text() -> &'static str {
    assert!(
        MIGRATIONS.len() > THIS,
        "there are {} migrations",
        MIGRATIONS.len()
    );
    MIGRATIONS[THIS]
}

/// The migration's statements as one line, every run of white space one
/// space.
fn one_line() -> String {
    text()
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
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
fn version_16_adds_the_four_columns_of_a_tenant_s_kek() {
    assert!(
        one_line().contains(
            "ALTER TABLE vcon.tenant ADD COLUMN wrap_ref text, ADD COLUMN wrapped_kek bytea, \
             ADD COLUMN pending_wrap_ref text, ADD COLUMN pending_wrapped_kek bytea,"
        ),
        "the columns, nullable, in one statement: {}",
        one_line()
    );
}

#[test]
fn version_16_keeps_each_pair_whole() {
    let line = one_line();
    for check in [
        "ADD CONSTRAINT kek_whole CHECK ((wrap_ref IS NULL) = (wrapped_kek IS NULL))",
        "ADD CONSTRAINT pending_kek_whole CHECK ((pending_wrap_ref IS NULL) = \
         (pending_wrapped_kek IS NULL))",
    ] {
        assert!(line.contains(check), "{check}: {line}");
    }
}

#[test]
fn version_16_adds_the_registry_root_to_the_key_row() {
    assert!(
        one_line().contains(
            "ALTER TABLE vcon.registry_key ADD COLUMN wrapped_root bytea, \
             ADD COLUMN pending_wrapped_root bytea;"
        ),
        "the root's blob and the pending one: {}",
        one_line()
    );
}

#[test]
fn the_application_role_may_write_the_root_s_columns_only() {
    let line = one_line();
    assert!(
        line.contains(
            "GRANT UPDATE (wrapped_root, pending_wrapped_root) ON vcon.registry_key TO vcon_app;"
        ),
        "the one grant: {line}"
    );
    assert_eq!(line.matches("GRANT").count(), 1, "and no other grant");
}

#[test]
fn the_binary_s_version_is_at_least_16() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn the_migration_says_every_row_stays_under_the_master_key() {
    assert!(
        comment().contains(
            "Every row there is keeps NULL in each new column: its keys are under the master key"
        ),
        "why no row is rewritten: {}",
        comment()
    );
}

#[test]
fn the_how_to_says_what_version_16_changes_under_schema_versions() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    let (before, versions) = page
        .split_once(" ### Schema versions ")
        .expect("the how-to has the section Schema versions");
    assert!(
        versions.contains("Version 16 adds the columns of a key backend"),
        "version 16, in the section of the schema's versions"
    );
    assert!(
        before.contains("schema version 6 to schema version 16"),
        "the upgrade step names this version"
    );
}

#[test]
fn the_status_page_names_version_16() {
    let page = flat("../../website/content/docs/status.md");
    assert!(
        page.contains(
            "Version 16 adds the columns of a key backend to `tenant` and `registry_key`"
        ),
        "in the entry of the PostgreSQL preview's schema version"
    );
}

// What must be refused.

#[test]
fn a_version_15_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(15)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 15, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn version_16_makes_no_table_and_drops_nothing() {
    let line = one_line();
    for word in [
        "CREATE", "DROP", "FUNCTION", "TRIGGER", "POLICY", "DELETE", "INSERT",
    ] {
        assert!(
            !line.contains(word),
            "{word} is not in the migration: {line}"
        );
    }
}

// The edges.

#[test]
fn version_16_changes_no_other_table() {
    let line = one_line();
    assert_eq!(
        line.matches("vcon.").count(),
        line.matches("vcon.tenant ").count() + line.matches("vcon.registry_key ").count(),
        "every name is the tenant table's or the key row's: {line}"
    );
    assert_eq!(
        line.matches("ALTER TABLE").count(),
        2,
        "one statement for each table"
    );
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::schema::{MIGRATIONS, Migrated};
    use super::super::live::{Db, in_scope};
    use super::super::v8::on_a_server::{one, owner, refused};
    use super::THIS;

    /// The scope of a registry transaction.
    const REGISTRY: &str = "registry";

    /// A tenant row's four new columns, in hex or NULL, joined by `|`.
    const KEK: &str = "SELECT coalesce(wrap_ref, 'NULL') || '|' || \
         coalesce(encode(wrapped_kek, 'hex'), 'NULL') || '|' || \
         coalesce(pending_wrap_ref, 'NULL') || '|' || \
         coalesce(encode(pending_wrapped_kek, 'hex'), 'NULL') FROM vcon.tenant";

    /// The key row's two new columns, the same way.
    const ROOT: &str = "SELECT coalesce(encode(wrapped_root, 'hex'), 'NULL') || '|' || \
         coalesce(encode(pending_wrapped_root, 'hex'), 'NULL') FROM vcon.registry_key";

    /// A tenant row with its file key and no backend key.
    const TENANT: &str = "INSERT INTO vcon.tenant (id, name_mac, name_sealed, created_at, \
         tenant_key_id, wrapped_tenant_key, key_since) \
         VALUES ('\\xa1', '\\xb1', '\\xc1', 1, '\\xd1', '\\xbeef', 1)";

    /// The key row.
    const KEY_ROW: &str = "INSERT INTO vcon.registry_key (catalog_key_id) VALUES ('\\xe1')";

    async fn run(client: &tokio_postgres::Client, sql: &str) {
        in_scope(client, REGISTRY, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A database at version 15.
    async fn at_version_15() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 15"),
            Migrated { from: 0, to: 15 },
            "the first fifteen migrations alone"
        );
        db
    }

    // What must work.

    #[tokio::test]
    async fn a_version_15_database_is_brought_to_version_16() {
        let db = at_version_15().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..=THIS])
                .await
                .expect("version 16"),
            Migrated { from: 15, to: 16 },
            "the sixteenth migration alone"
        );
    }

    #[tokio::test]
    async fn the_rows_of_version_15_are_under_the_master_key_after_it() {
        let db = at_version_15().await;
        let owner = owner(&db).await;
        run(&owner, TENANT).await;
        run(&owner, KEY_ROW).await;
        db.migrate().await;
        let app = db.app().await;
        assert_eq!(
            one(&app, REGISTRY, KEK).await,
            "NULL|NULL|NULL|NULL",
            "the tenant has no backend key"
        );
        assert_eq!(one(&app, REGISTRY, ROOT).await, "NULL|NULL", "nor the root");
        assert_eq!(
            one(
                &app,
                REGISTRY,
                "SELECT encode(wrapped_tenant_key, 'hex') FROM vcon.tenant"
            )
            .await,
            "beef",
            "its key is as it was"
        );
    }

    #[tokio::test]
    async fn the_application_role_writes_a_tenant_s_kek_and_the_root() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT).await;
        run(&app, KEY_ROW).await;
        run(
            &app,
            "UPDATE vcon.tenant SET wrap_ref = 'pkcs11:k', wrapped_kek = '\\x0106', \
             pending_wrap_ref = 'pkcs11:n', pending_wrapped_kek = '\\x0107'",
        )
        .await;
        run(
            &app,
            "UPDATE vcon.registry_key SET wrapped_root = '\\x0108', pending_wrapped_root = '\\x0109'",
        )
        .await;
        assert_eq!(
            one(&app, REGISTRY, KEK).await,
            "pkcs11:k|0106|pkcs11:n|0107",
            "both pairs"
        );
        assert_eq!(one(&app, REGISTRY, ROOT).await, "0108|0109", "the root's");
    }

    // What must be refused.

    #[tokio::test]
    async fn half_a_kek_is_refused() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT).await;
        for half in [
            "wrap_ref = 'pkcs11:k'",
            "wrapped_kek = '\\x0106'",
            "pending_wrap_ref = 'pkcs11:k'",
            "pending_wrapped_kek = '\\x0106'",
        ] {
            let why = refused(&app, REGISTRY, &format!("UPDATE vcon.tenant SET {half}")).await;
            assert!(why.contains("kek_whole"), "{half}: {why}");
        }
    }

    #[tokio::test]
    async fn the_application_role_may_not_update_another_column_of_the_key_row() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, KEY_ROW).await;
        let why = refused(
            &app,
            REGISTRY,
            "UPDATE vcon.registry_key SET one = false, wrapped_root = '\\x01'",
        )
        .await;
        assert!(
            why.contains("permission denied"),
            "only the granted columns: {why}"
        );
    }

    #[tokio::test]
    async fn a_tenant_s_transaction_does_not_read_the_root() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, KEY_ROW).await;
        let why = refused(&app, super::super::live::TENANT_A, ROOT).await;
        assert!(
            why.contains("this transaction is a tenant's, not the registry's"),
            "a tenant's transaction is refused the registry's row: {why}"
        );
    }

    // The edges.

    #[tokio::test]
    async fn migrate_on_a_database_at_this_binary_s_version_keeps_the_columns() {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, TENANT).await;
        run(
            &app,
            "UPDATE vcon.tenant SET wrap_ref = 'pkcs11:k', wrapped_kek = '\\x0106'",
        )
        .await;
        db.migrate().await;
        assert_eq!(
            one(&app, REGISTRY, KEK).await,
            "pkcs11:k|0106|NULL|NULL",
            "kept"
        );
    }
}
