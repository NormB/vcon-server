//! Schema version 13: the database holds the claim of the bucket prefix,
//! in the one row of the table `storage_claim`. With the catalogs in a
//! data directory that id is in the file `storage.claim`
//! (backend/s3/claim.rs); with the catalogs in PostgreSQL every server on
//! the database reads the same row. Each live test of the migration
//! starts from a database at version 12.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 12;

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

/// Texts that are, and are not, a claim id, each with whether it is one:
/// the server (`parse_id`) and the database (the table's CHECK) must
/// agree on every one.
const IDS: &[(&str, bool)] = &[
    ("0123456789abcdef0123456789abcdef", true),
    ("00000000000000000000000000000000", true),
    ("ffffffffffffffffffffffffffffffff", true),
    ("0123456789ABCDEF0123456789ABCDEF", false),
    ("0123456789abcdef0123456789abcde", false),
    ("0123456789abcdef0123456789abcdef0", false),
    ("0123456789abcdef0123456789abcdeg", false),
    ("0123456789abcdef 123456789abcdef", false),
    (" 123456789abcdef0123456789abcdef", false),
    ("", false),
];

// What must work.

#[test]
fn version_13_is_the_thirteenth_migration_and_makes_the_one_table() {
    assert_eq!(
        text().matches("CREATE TABLE vcon.storage_claim (").count(),
        1,
        "the thirteenth makes storage_claim once"
    );
    assert_eq!(
        text().matches("CREATE TABLE").count(),
        1,
        "and no other table"
    );
}

#[test]
fn the_binary_s_version_is_at_least_13() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn the_application_role_may_read_the_claim_and_make_it() {
    assert_eq!(
        statements(text())
            .iter()
            .filter(|line| **line == "GRANT SELECT, INSERT ON vcon.storage_claim TO vcon_app;")
            .count(),
        1,
        "the one grant of the migration"
    );
    assert_eq!(one_line().matches("GRANT").count(), 1, "and no other grant");
}

#[test]
fn the_migration_says_why_the_table_starts_empty() {
    let comment = comment();
    assert!(
        comment.contains(
            "No deployed database has a claim: a server of the preview stops after it \
             connects. So the table starts empty"
        ),
        "the reason, in the migration's comment: {comment}"
    );
}

#[test]
fn the_how_to_says_what_version_13_changes() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    assert!(
        page.contains(
            "Version 13 makes the table `storage_claim`, whose one row holds the claim of \
             the bucket prefix"
        ),
        "the paragraph on version 13"
    );
    assert!(
        page.contains("schema version 6 to schema version 13"),
        "the line migrate prints on a database of the first preview"
    );
}

#[test]
fn the_how_to_says_what_version_13_changes_under_schema_versions() {
    let page = flat("../../website/content/docs/postgres-catalog.md");
    let (before, versions) = page
        .split_once(" ### Schema versions ")
        .expect("the how-to has the section Schema versions");
    assert!(
        versions.contains("Version 13 makes the table `storage_claim`, whose one row holds the"),
        "version 13, in the section of the schema's versions"
    );
    assert!(
        before.contains("schema version 6 to schema version 13"),
        "the upgrade step names this version"
    );
    // The changelog's entry tells an upgrader to run migrate and links the
    // how-to; it does not walk through the versions.
    let changelog = flat("../../CHANGELOG.md");
    let unreleased = changelog
        .split(" ## [")
        .find(|section| section.starts_with("Unreleased]"))
        .expect("an Unreleased section");
    assert!(
        !unreleased.contains("Version 13 ") && !unreleased.contains("`storage_claim`"),
        "the changelog leaves the schema's versions to the how-to"
    );
}

#[test]
fn the_status_page_names_version_13() {
    let page = flat("../../website/content/docs/status.md");
    assert!(
        page.contains("**Schema version 13 of the PostgreSQL preview.**"),
        "the entry of what is on main only"
    );
}

#[test]
fn the_s3_page_says_where_the_claim_is_kept_for_each_kind_of_catalog() {
    let page = flat("../../website/content/docs/s3-storage.md");
    assert!(
        page.contains("in the row of the table `storage_claim`"),
        "the claim of catalogs in PostgreSQL"
    );
    assert!(
        page.contains("`storage.claim`"),
        "the claim of catalogs in a data directory"
    );
}

// What must be refused.

#[test]
fn a_version_12_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(12)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 12, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn the_application_role_is_granted_no_update_and_no_delete() {
    let line = one_line();
    for word in ["UPDATE", "DELETE", "TRUNCATE", "ALL"] {
        assert!(
            !line.split([' ', ',', '(', ')', ';']).any(|w| w == word),
            "{word} is not in the migration: {line}"
        );
    }
}

#[test]
fn version_13_replaces_no_function_and_no_trigger_and_makes_no_sequence() {
    let line = one_line();
    for word in ["FUNCTION", "TRIGGER", "SEQUENCE", "IDENTITY", "DROP"] {
        assert!(!line.contains(word), "{word} is not in the migration");
    }
}

#[test]
fn the_claim_table_takes_one_row_only() {
    assert!(
        one_line().contains(
            "CREATE TABLE vcon.storage_claim ( one boolean PRIMARY KEY DEFAULT true CHECK (one),"
        ),
        "a key column that holds one value: {}",
        one_line()
    );
}

#[test]
fn the_table_checks_that_an_id_is_32_lowercase_hexadecimal_digits() {
    assert!(
        one_line().contains("claim_id text NOT NULL CHECK (claim_id ~ '^[0-9a-f]{32}$')"),
        "the column and its check: {}",
        one_line()
    );
}

// The edges.

#[test]
fn version_13_changes_no_other_table() {
    let line = one_line();
    assert_eq!(
        line.matches("vcon.").count(),
        line.matches("vcon.storage_claim").count() + line.matches("vcon.in_registry()").count(),
        "every name of the schema is the new table's or the registry's marker: {line}"
    );
    assert_eq!(
        line.matches("ALTER TABLE").count(),
        2,
        "the two statements of row level security"
    );
}

#[test]
fn the_table_has_row_level_security_forced_on_its_owner() {
    for kind in ["ENABLE", "FORCE"] {
        assert_eq!(
            statements(text())
                .iter()
                .filter(|line| **line
                    == format!("ALTER TABLE vcon.storage_claim {kind} ROW LEVEL SECURITY;"))
                .count(),
            1,
            "{kind}"
        );
    }
}

#[test]
fn the_table_has_the_policy_of_the_registry_s_rows() {
    assert!(
        one_line().contains(
            "CREATE POLICY registry_rows ON vcon.storage_claim USING (vcon.in_registry() = \
             (claim_id IS NOT NULL)) WITH CHECK (vcon.in_registry());"
        ),
        "read and written in a transaction of the registry only: {}",
        one_line()
    );
}

#[test]
fn the_server_reads_as_a_claim_id_what_the_table_s_check_describes() {
    for (text, is_one) in IDS {
        assert_eq!(
            crate::backend::s3::claim::parse_id(text).is_some(),
            *is_one,
            "{text:?}"
        );
    }
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use tokio_postgres::Client;

    use super::super::super::super::catalog::Database;
    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION, SERVERS_LOCK};
    use super::super::live::{Db, TENANT_A, in_scope};
    use super::super::v8::on_a_server::{one, owner, refused};
    use super::{IDS, THIS};
    use crate::config::StorageError;

    /// The scope of a registry transaction.
    const REGISTRY: &str = "registry";

    /// What a tenant's transaction is told when it asks for the registry.
    const A_TENANT_S: &str = "this transaction is a tenant's, not the registry's";

    /// What a transaction with no marker is told.
    const NO_TENANT: &str = "no tenant set for this transaction";

    /// The claim the database holds, or nothing.
    const CLAIM: &str = "SELECT coalesce(string_agg(claim_id, ','), '') FROM vcon.storage_claim";

    /// Two claim ids.
    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "fedcba9876543210fedcba9876543210";

    /// The statement that makes `id` the database's claim.
    fn claim(id: &str) -> String {
        format!("INSERT INTO vcon.storage_claim (claim_id) VALUES ('{id}')")
    }

    /// A database at version 12.
    async fn at_version_12() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 12"),
            Migrated { from: 0, to: 12 },
            "the first twelve migrations alone"
        );
        db
    }

    async fn run(client: &Client, scope: &str, sql: &str) {
        in_scope(client, scope, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A migrated database that holds the claim [`A`], and the
    /// application role's connection.
    async fn with_claim() -> (Db, Client) {
        let db = Db::migrated().await;
        let app = db.app().await;
        run(&app, REGISTRY, &claim(A)).await;
        (db, app)
    }

    /// Waits for `call` on a runtime of its own, for a test that also
    /// connects as a server does, which must not be on a runtime's thread.
    fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    // What must work.

    #[tokio::test]
    async fn a_version_12_database_is_brought_to_version_13() {
        let db = at_version_12().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..=THIS])
                .await
                .expect("version 13"),
            Migrated { from: 12, to: 13 },
            "the thirteenth migration alone"
        );
    }

    #[tokio::test]
    async fn a_migrated_database_holds_no_claim() {
        let db = Db::migrated().await;
        assert_eq!(one(&db.app().await, REGISTRY, CLAIM).await, "", "no row");
    }

    #[tokio::test]
    async fn the_registry_writes_and_reads_the_claim() {
        let (_db, app) = with_claim().await;
        assert_eq!(one(&app, REGISTRY, CLAIM).await, A, "the id written");
    }

    #[tokio::test]
    async fn the_table_has_the_one_policy_of_the_registry_s_rows() {
        let db = Db::migrated().await;
        assert_eq!(
            one(
                &db.app().await,
                REGISTRY,
                "SELECT string_agg(policyname || ' ' || cmd, ',' ORDER BY policyname) \
                 FROM pg_policies WHERE schemaname = 'vcon' AND tablename = 'storage_claim'"
            )
            .await,
            "registry_rows ALL",
            "one policy for every kind of statement"
        );
    }

    #[tokio::test]
    async fn migrate_on_a_database_at_this_binary_s_version_keeps_the_claim() {
        let (db, app) = with_claim().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "nothing to apply"
        );
        assert_eq!(one(&app, REGISTRY, CLAIM).await, A, "the claim is kept");
    }

    #[tokio::test]
    async fn a_claim_made_where_there_is_one_changes_nothing_and_is_not_an_error() {
        let (_db, app) = with_claim().await;
        // The statement a server makes its claim with: a server that
        // starts at the same moment may have made the row first.
        run(
            &app,
            REGISTRY,
            &format!("{} ON CONFLICT DO NOTHING", claim(B)),
        )
        .await;
        assert_eq!(one(&app, REGISTRY, CLAIM).await, A, "the first claim");
    }

    #[tokio::test]
    async fn the_rows_of_version_12_are_as_they_were() {
        const KEY: &str = "SELECT coalesce(string_agg(encode(catalog_key_id, 'hex'), ','), '') \
                           FROM vcon.registry_key";
        let db = at_version_12().await;
        // A server of this binary does not connect to version 12, so the
        // owner writes the row.
        run(
            &owner(&db).await,
            REGISTRY,
            "INSERT INTO vcon.registry_key (catalog_key_id) VALUES ('\\xc1')",
        )
        .await;
        db.migrate().await;
        assert_eq!(
            one(&db.app().await, REGISTRY, KEY).await,
            "c1",
            "a row of another table"
        );
    }

    // What must be refused.

    #[tokio::test]
    async fn the_database_holds_one_claim_only() {
        let (_db, app) = with_claim().await;
        let why = refused(&app, REGISTRY, &claim(B)).await;
        assert!(
            why.contains("duplicate key value violates unique constraint"),
            "a second row: {why}"
        );
        let why = refused(
            &app,
            REGISTRY,
            &format!("INSERT INTO vcon.storage_claim (one, claim_id) VALUES (false, '{B}')"),
        )
        .await;
        assert!(
            why.contains("violates check constraint"),
            "a row under the other value of the key column: {why}"
        );
        assert_eq!(one(&app, REGISTRY, CLAIM).await, A, "the one claim");
    }

    #[tokio::test]
    async fn a_text_that_is_not_a_claim_id_is_refused() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for (text, is_one) in IDS.iter().filter(|(_, is_one)| !is_one) {
            let why = refused(&app, REGISTRY, &claim(text)).await;
            assert!(
                why.contains("violates check constraint"),
                "{text:?} ({is_one}): {why}"
            );
        }
        let why = refused(
            &app,
            REGISTRY,
            &format!("INSERT INTO vcon.storage_claim (claim_id) VALUES (E'{A}\\n')"),
        )
        .await;
        assert!(
            why.contains("violates check constraint"),
            "an id with its line ending: {why}"
        );
        assert_eq!(one(&app, REGISTRY, CLAIM).await, "", "no row was made");
    }

    #[tokio::test]
    async fn a_claim_with_no_id_is_refused() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for sql in [
            "INSERT INTO vcon.storage_claim (claim_id) VALUES (NULL)",
            "INSERT INTO vcon.storage_claim DEFAULT VALUES",
        ] {
            let why = refused(&app, REGISTRY, sql).await;
            assert!(why.contains("violates not-null constraint"), "{sql}: {why}");
        }
        assert_eq!(one(&app, REGISTRY, CLAIM).await, "", "no row was made");
    }

    #[tokio::test]
    async fn the_application_role_cannot_change_the_claim() {
        let (_db, app) = with_claim().await;
        for sql in [
            format!("UPDATE vcon.storage_claim SET claim_id = '{B}'"),
            "UPDATE vcon.storage_claim SET one = true".to_owned(),
            format!(
                "{} ON CONFLICT (one) DO UPDATE SET claim_id = EXCLUDED.claim_id",
                claim(B)
            ),
        ] {
            assert_eq!(
                refused(&app, REGISTRY, &sql).await,
                "permission denied for table storage_claim",
                "{sql}"
            );
        }
        assert_eq!(one(&app, REGISTRY, CLAIM).await, A, "the claim is as made");
    }

    #[tokio::test]
    async fn the_application_role_cannot_delete_the_claim() {
        let (_db, app) = with_claim().await;
        for sql in [
            "DELETE FROM vcon.storage_claim",
            "TRUNCATE vcon.storage_claim",
        ] {
            assert_eq!(
                refused(&app, REGISTRY, sql).await,
                "permission denied for table storage_claim",
                "{sql}"
            );
        }
        assert_eq!(one(&app, REGISTRY, CLAIM).await, A, "the claim is there");
    }

    #[tokio::test]
    async fn a_tenant_s_transaction_reads_no_claim() {
        let (_db, app) = with_claim().await;
        assert_eq!(
            refused(&app, TENANT_A, CLAIM).await,
            A_TENANT_S,
            "a tenant asks"
        );
    }

    #[tokio::test]
    async fn a_tenant_s_transaction_makes_no_claim() {
        let db = Db::migrated().await;
        let app = db.app().await;
        assert_eq!(
            refused(&app, TENANT_A, &claim(A)).await,
            A_TENANT_S,
            "a tenant writes"
        );
        assert_eq!(one(&app, REGISTRY, CLAIM).await, "", "no row was made");
    }

    #[tokio::test]
    async fn a_transaction_with_no_marker_reads_and_makes_no_claim() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for sql in [CLAIM.to_owned(), claim(A)] {
            let why = super::super::live::no_scope(&app, &sql)
                .await
                .expect_err("refused");
            assert_eq!(why, NO_TENANT, "no marker: {sql}");
        }
        assert_eq!(one(&app, REGISTRY, CLAIM).await, "", "no row was made");
    }

    #[tokio::test]
    async fn a_failed_migration_leaves_version_12_whole() {
        let db = at_version_12().await;
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
                 AND table_name = 'storage_claim'"
            )
            .await,
            "0",
            "the table is not left behind"
        );
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..=THIS]).await.expect("again"),
            Migrated { from: 12, to: 13 },
            "the database was still at version 12"
        );
    }

    #[tokio::test]
    async fn a_table_of_that_name_made_by_hand_refuses_the_migration() {
        let db = at_version_12().await;
        owner(&db)
            .await
            .batch_execute("CREATE TABLE vcon.storage_claim (note text)")
            .await
            .expect("a table the migration did not make");
        let why = db
            .migrate_with(&MIGRATIONS[..=THIS])
            .await
            .expect_err("refused");
        assert!(
            why.starts_with("version 13: ") && why.contains("already exists"),
            "the version and the cause: {why}"
        );
        assert_eq!(
            one(
                &owner(&db).await,
                REGISTRY,
                "SELECT max(version) FROM vcon.schema_version"
            )
            .await,
            "12",
            "the database is still at version 12"
        );
    }

    #[tokio::test]
    async fn the_migration_is_refused_while_a_server_holds_the_database_s_lock() {
        let db = at_version_12().await;
        let holder = db.admin().await;
        holder
            .batch_execute(&format!("SELECT pg_advisory_lock_shared({SERVERS_LOCK})"))
            .await
            .expect("holds the lock a server holds");
        let why = db
            .migrate_with(&MIGRATIONS[..=THIS])
            .await
            .expect_err("refused");
        assert!(
            why.contains("another server or command holds the database's locks"),
            "says so: {why}"
        );
        drop(holder);
        assert_eq!(
            one(
                &owner(&db).await,
                REGISTRY,
                "SELECT max(version) FROM vcon.schema_version"
            )
            .await,
            "12",
            "nothing was applied"
        );
    }

    #[test]
    fn a_server_of_this_binary_refuses_a_database_at_version_12() {
        let db = wait(at_version_12());
        let (_dir, settings) = db.settings("vcon_app");
        let StorageError(why) = Database::connect(&settings).expect_err("refused");
        assert!(
            why.contains(&format!(
                "version 12, older than this binary's {SCHEMA_VERSION}"
            )) && why.contains("vcon-server migrate"),
            "the versions and the command: {why}"
        );
    }

    #[tokio::test]
    async fn a_database_a_newer_binary_migrated_is_not_migrated() {
        let db = Db::migrated().await;
        let mut with: Vec<&str> = MIGRATIONS.to_vec();
        with.push("SELECT 1;");
        db.migrate_with(&with).await.expect("a later version");
        let why = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(
            why.contains(&format!(
                "version {}, this binary {SCHEMA_VERSION}",
                SCHEMA_VERSION + 1
            )) && why.contains("upgrade this server"),
            "the versions and what to do: {why}"
        );
    }

    // The edges.

    #[tokio::test]
    async fn the_owner_too_is_under_the_policy_of_the_table() {
        let (db, _app) = with_claim().await;
        assert_eq!(
            refused(&owner(&db).await, TENANT_A, CLAIM).await,
            A_TENANT_S,
            "forced on the owner"
        );
    }

    #[tokio::test]
    async fn the_owner_alone_deletes_the_claim() {
        let (db, app) = with_claim().await;
        run(
            &owner(&db).await,
            REGISTRY,
            "DELETE FROM vcon.storage_claim",
        )
        .await;
        assert_eq!(one(&app, REGISTRY, CLAIM).await, "", "the owner's delete");
        run(&app, REGISTRY, &claim(B)).await;
        assert_eq!(one(&app, REGISTRY, CLAIM).await, B, "a claim is made again");
    }

    #[tokio::test]
    async fn migrating_twice_does_the_work_once() {
        let db = at_version_12().await;
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
    async fn the_database_takes_as_a_claim_id_what_the_server_reads_as_one() {
        let db = Db::migrated().await;
        let app = db.app().await;
        let owner = owner(&db).await;
        for (text, is_one) in IDS {
            let made = in_scope(&app, REGISTRY, &claim(text)).await.is_ok();
            assert_eq!(made, *is_one, "{text:?}");
            assert_eq!(
                crate::backend::s3::claim::parse_id(text).is_some(),
                made,
                "the server and the database agree on {text:?}"
            );
            run(&owner, REGISTRY, "DELETE FROM vcon.storage_claim").await;
        }
    }

    #[tokio::test]
    async fn the_column_of_the_id_has_no_default() {
        let db = Db::migrated().await;
        assert_eq!(
            one(
                &db.app().await,
                REGISTRY,
                "SELECT data_type || ' ' || is_nullable || ' ' || coalesce(column_default, 'none') \
                 FROM information_schema.columns WHERE table_schema = 'vcon' \
                 AND table_name = 'storage_claim' AND column_name = 'claim_id'"
            )
            .await,
            "text NO none",
            "an id is always given"
        );
    }

    #[tokio::test]
    async fn the_application_role_holds_two_rights_on_the_table() {
        let db = Db::migrated().await;
        assert_eq!(
            one(
                &db.app().await,
                REGISTRY,
                "SELECT string_agg(privilege_type, ',' ORDER BY privilege_type) \
                 FROM information_schema.table_privileges WHERE table_schema = 'vcon' \
                 AND table_name = 'storage_claim' AND grantee = 'vcon_app'"
            )
            .await,
            "INSERT,SELECT",
            "to read the claim and to make it"
        );
    }
}
