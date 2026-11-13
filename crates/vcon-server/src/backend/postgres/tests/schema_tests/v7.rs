//! Schema version 7: the order versions were stored in (`seq` of the
//! version table), and the application role's leave to vacuum the tables a
//! purge deletes from. Each live test starts from a database at version
//! 6, as a build of `main` before the catalog's statements left one.

use super::super::super::schema::{MIGRATIONS, SCHEMA_VERSION, version_refusal};

/// The position of this version's migration.
const THIS: usize = 6;

/// The statement that adds the column.
const ADD: &str = "ALTER TABLE vcon.version ADD COLUMN seq bigint GENERATED ALWAYS AS IDENTITY;";

#[test]
fn version_7_is_the_seventh_migration() {
    assert!(
        MIGRATIONS.len() > THIS,
        "there are {} migrations",
        MIGRATIONS.len()
    );
    assert!(
        MIGRATIONS[THIS].lines().any(|line| line == ADD),
        "the seventh adds seq, numbered by the server"
    );
}

#[test]
fn version_7_grants_the_vacuum_privilege_only_where_the_server_has_it() {
    let text = MIGRATIONS[THIS];
    assert!(
        text.contains("IF current_setting('server_version_num')::integer >= 170000 THEN"),
        "PostgreSQL 16 has no MAINTAIN privilege"
    );
    assert!(
        text.contains("'GRANT MAINTAIN ON vcon.version, vcon.vcon, vcon.tombstone TO vcon_app'"),
        "the three tables a purge deletes from, and no other"
    );
}

#[test]
fn the_binary_s_version_is_at_least_7() {
    assert!(
        usize::try_from(SCHEMA_VERSION).expect("positive") > THIS,
        "this binary's version: {SCHEMA_VERSION}"
    );
}

#[test]
fn a_version_6_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(6)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 6, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn version_7_changes_no_row_by_a_statement_and_makes_no_table() {
    // The owner role is held to row-level security too, so an UPDATE or
    // DELETE in a migration would raise. A statement begins its line.
    for statement in ["UPDATE ", "DELETE ", "DROP ", "TRUNCATE ", "CREATE "] {
        assert!(
            !MIGRATIONS[THIS]
                .lines()
                .any(|line| line.trim_start().to_uppercase().starts_with(statement)),
            "no {statement}in version 7"
        );
    }
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::super::live::value;
    use super::super::live::{Db, TENANT_A, TENANT_B, db_message, in_scope};
    use super::{ADD, THIS};

    /// A database at version 6.
    async fn at_version_6() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..THIS])
                .await
                .expect("version 6"),
            Migrated { from: 0, to: 6 },
            "the first six migrations alone"
        );
        db
    }

    /// A version row of `tenant` whose id ends in `n`.
    fn insert_version(tenant: &str, n: u8) -> String {
        format!(
            "INSERT INTO vcon.version (tenant, version, uuid_mac, form_sealed, gzip_sealed, \
             size_sealed, stored_at, tenant_key_id, wrapped_key) \
             VALUES ('\\x{tenant}', '\\x{n:02x}', '\\x02', '\\x03', '\\x04', '\\x05', 1, '\\x06', \
             '\\x07')"
        )
    }

    /// The one value `sql` gives the application role in `tenant`'s scope.
    async fn read(db: &Db, tenant: &str, sql: &str) -> Option<String> {
        in_scope(&db.app().await, tenant, sql)
            .await
            .expect("reads")
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => Some(r.get(0).map(str::to_owned)),
                _ => None,
            })
            .expect("a row")
    }

    const COLUMN: &str = "SELECT count(*) FROM information_schema.columns WHERE \
        table_schema = 'vcon' AND table_name = 'version' AND column_name = 'seq' \
        AND data_type = 'bigint' AND is_identity = 'YES' AND identity_generation = 'ALWAYS'";

    // What must work.

    #[tokio::test]
    async fn a_version_6_database_is_brought_to_version_7_with_its_versions_numbered() {
        let db = at_version_6().await;
        // A server is refused at version 6, so the tester writes the rows.
        let app = db.admin().await;
        for n in 1..=3 {
            in_scope(&app, TENANT_A, &insert_version(TENANT_A, n))
                .await
                .expect("a version, stored before the upgrade");
        }
        for n in 1..=2 {
            in_scope(&app, TENANT_B, &insert_version(TENANT_B, n))
                .await
                .expect("another tenant's");
        }
        drop(app);
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 6,
                to: SCHEMA_VERSION
            },
            "every migration after the sixth runs"
        );
        assert_eq!(
            read(
                &db,
                TENANT_A,
                "SELECT count(DISTINCT seq) || ' ' || count(*) FROM vcon.version"
            )
            .await
            .as_deref(),
            Some("3 3"),
            "the three versions kept their rows, each with a number of its own"
        );
        assert_eq!(
            read(
                &db,
                TENANT_B,
                "SELECT count(DISTINCT seq) || ' ' || count(*) FROM vcon.version"
            )
            .await
            .as_deref(),
            Some("2 2"),
            "and the other tenant's two"
        );
    }

    #[tokio::test]
    async fn the_column_is_made_by_version_7_and_not_before() {
        let db = at_version_6().await;
        let admin = db.admin().await;
        assert_eq!(value(&admin, COLUMN).await, "0", "not at version 6");
        db.migrate().await;
        assert_eq!(value(&admin, COLUMN).await, "1", "there at version 7");
    }

    #[tokio::test]
    async fn versions_take_rising_numbers_in_the_order_they_are_stored() {
        let db = Db::migrated().await;
        let app = db.app().await;
        // The ids fall while the numbers rise: the order is the storing's.
        for n in [9, 5, 1] {
            in_scope(&app, TENANT_A, &insert_version(TENANT_A, n))
                .await
                .expect("stores");
        }
        assert_eq!(
            read(
                &db,
                TENANT_A,
                "SELECT string_agg(encode(version, 'hex'), ' ' ORDER BY seq) FROM vcon.version"
            )
            .await
            .as_deref(),
            Some("09 05 01"),
            "by seq, the versions are in the order they were stored"
        );
    }

    #[tokio::test]
    async fn a_serving_connection_is_accepted_once_version_7_is_there() {
        let db = at_version_6().await;
        let e = db
            .connect("vcon_app", Purpose::Serve)
            .await
            .expect_err("refused at version 6")
            .to_string();
        assert!(e.contains("version 6, older"), "says the version: {e}");
        db.migrate().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_ok(),
            "accepted at this binary's version"
        );
    }

    #[tokio::test]
    async fn a_new_database_has_the_column() {
        let db = Db::migrated().await;
        assert_eq!(
            value(&db.admin().await, COLUMN).await,
            "1",
            "every migration from none makes it"
        );
    }

    #[tokio::test]
    async fn the_application_role_may_vacuum_the_purged_tables_from_postgresql_17() {
        let db = Db::migrated().await;
        let admin = db.admin().await;
        let from_17 = value(
            &admin,
            "SELECT current_setting('server_version_num')::int >= 170000",
        )
        .await
            == "t";
        if from_17 {
            assert_eq!(
                value(
                    &admin,
                    "SELECT string_agg(c.relname, ' ' ORDER BY c.relname) FROM pg_class c \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname = 'vcon' AND c.relkind = 'r' \
                       AND has_table_privilege('vcon_app', c.oid, 'MAINTAIN')"
                )
                .await,
                "tombstone vcon version",
                "the three tables a purge deletes from, and no other"
            );
        }
        db.app()
            .await
            .batch_execute("VACUUM vcon.version, vcon.vcon, vcon.tombstone")
            .await
            .expect("VACUUM by the application role is no error on any version");
    }

    // What must be refused.

    #[tokio::test]
    async fn a_version_7_that_fails_leaves_version_6_whole() {
        let db = at_version_6().await;
        let broken = format!("{ADD}\nSELECT boom;");
        let mut migrations: Vec<&str> = MIGRATIONS[..THIS].to_vec();
        migrations.push(&broken);
        let e = db.migrate_with(&migrations).await.expect_err("fails");
        assert!(e.starts_with("version 7: "), "names the version: {e}");
        let admin = db.admin().await;
        assert_eq!(
            value(&admin, COLUMN).await,
            "0",
            "the column was rolled back with it"
        );
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 6,
                to: SCHEMA_VERSION
            },
            "and the real migration then runs from 6"
        );
    }

    #[tokio::test]
    async fn the_application_role_cannot_give_a_version_its_number() {
        let db = Db::migrated().await;
        let statement = insert_version(TENANT_A, 1)
            .replace("wrapped_key) VALUES", "wrapped_key, seq) VALUES")
            .replace("'\\x07')", "'\\x07', 99)");
        assert!(
            statement.contains(", seq) VALUES") && statement.ends_with(", 99)"),
            "{statement}"
        );
        let e = in_scope(&db.app().await, TENANT_A, &statement)
            .await
            .expect_err("refused");
        assert!(
            e.contains("cannot insert a non-DEFAULT value into column \"seq\""),
            "the server numbers the row: {e}"
        );
    }

    #[tokio::test]
    async fn the_application_role_cannot_change_a_versions_number() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_version(TENANT_A, 1))
            .await
            .expect("stores");
        let e = in_scope(&app, TENANT_A, "UPDATE vcon.version SET seq = 1000")
            .await
            .expect_err("refused");
        assert!(
            e.contains("column \"seq\" can only be updated to DEFAULT"),
            "the number is the server's: {e}"
        );
    }

    #[tokio::test]
    async fn the_application_role_cannot_drop_or_retype_the_column() {
        let db = Db::migrated().await;
        let app = db.app().await;
        for statement in [
            "ALTER TABLE vcon.version DROP COLUMN seq",
            "ALTER TABLE vcon.version ALTER COLUMN seq TYPE integer",
            "ALTER TABLE vcon.version ALTER COLUMN seq DROP IDENTITY",
        ] {
            let e = app.batch_execute(statement).await.expect_err("refused");
            assert!(
                db_message(&e).contains("must be owner of table version"),
                "{statement}: {e}"
            );
        }
    }

    #[tokio::test]
    async fn a_tenants_transaction_reads_no_number_of_another_tenant() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_version(TENANT_A, 1))
            .await
            .expect("stores");
        assert_eq!(
            read(&db, TENANT_B, "SELECT count(seq) FROM vcon.version")
                .await
                .as_deref(),
            Some("0"),
            "row-level security covers the new column as the others"
        );
    }

    // The limits and odd inputs.

    #[tokio::test]
    async fn migrating_twice_from_version_6_does_the_work_once() {
        let db = at_version_6().await;
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 6,
                to: SCHEMA_VERSION
            },
            "the first run"
        );
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: SCHEMA_VERSION,
                to: SCHEMA_VERSION
            },
            "the second finds nothing to do"
        );
    }

    #[tokio::test]
    async fn a_version_6_database_with_no_version_gains_the_column() {
        let db = at_version_6().await;
        db.migrate().await;
        assert_eq!(
            read(&db, TENANT_A, "SELECT count(*) FROM vcon.version")
                .await
                .as_deref(),
            Some("0"),
            "no row was made"
        );
        in_scope(&db.app().await, TENANT_A, &insert_version(TENANT_A, 1))
            .await
            .expect("and the first version stored takes a number");
    }

    #[tokio::test]
    async fn the_numbers_of_two_tenants_come_from_one_sequence() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_version(TENANT_A, 1))
            .await
            .expect("stores");
        in_scope(&app, TENANT_B, &insert_version(TENANT_B, 1))
            .await
            .expect("stores");
        in_scope(&app, TENANT_A, &insert_version(TENANT_A, 2))
            .await
            .expect("stores");
        let a = read(
            &db,
            TENANT_A,
            "SELECT max(seq) - min(seq) FROM vcon.version",
        )
        .await;
        assert_eq!(
            a.as_deref(),
            Some("4"),
            "the other tenant's version took the numbers between, two a row since version 8: the \
             order is the database's"
        );
    }

    #[tokio::test]
    async fn a_number_is_not_given_again_after_its_version_is_deleted() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_version(TENANT_A, 1))
            .await
            .expect("stores");
        let first = read(&db, TENANT_A, "SELECT seq FROM vcon.version").await;
        in_scope(&app, TENANT_A, "DELETE FROM vcon.version")
            .await
            .expect("purges");
        in_scope(&app, TENANT_A, &insert_version(TENANT_A, 1))
            .await
            .expect("stores the same id again");
        let second = read(&db, TENANT_A, "SELECT seq FROM vcon.version").await;
        assert!(
            second.as_deref().and_then(|s| s.parse::<i64>().ok())
                > first.as_deref().and_then(|s| s.parse::<i64>().ok()),
            "a later version has a later number: {first:?} then {second:?}"
        );
    }
}
