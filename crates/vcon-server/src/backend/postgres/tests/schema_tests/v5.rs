//! Schema version 5: the sessions of the users who signed in, the
//! registry's table `session`. Each live test starts from a database at
//! version 4, as a build of `main` before sessions left one. No version
//! number after 5 is written here: what this binary makes is read from
//! `SCHEMA_VERSION`.

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
fn the_fifth_migration_makes_the_session_table() {
    assert!(
        MIGRATIONS[4].contains("CREATE TABLE vcon.session ("),
        "the fifth makes the table"
    );
}

#[test]
fn a_version_4_database_is_refused_with_the_command_that_upgrades_it() {
    let why = version_refusal(Some(4)).expect("refused");
    assert!(
        why.contains(&format!(
            "version 4, older than this binary's {SCHEMA_VERSION}"
        )) && why.contains("vcon-server migrate"),
        "the versions and the command: {why}"
    );
}

#[test]
fn version_5_changes_no_row_and_drops_nothing() {
    // The owner role is held to row-level security too, so an UPDATE or
    // DELETE in a migration would raise. A statement begins its line.
    for statement in ["UPDATE ", "DELETE ", "DROP ", "TRUNCATE ", "INSERT "] {
        assert!(
            !MIGRATIONS[4]
                .lines()
                .any(|line| line.trim_start().to_uppercase().starts_with(statement)),
            "no {statement}in version 5"
        );
    }
}

#[test]
fn version_5_is_the_table_its_indexes_its_row_level_security_and_its_grant() {
    assert_eq!(
        statements(MIGRATIONS[4]),
        [
            "CREATE TABLE vcon.session ( \
             id_hash bytea PRIMARY KEY CHECK (octet_length(id_hash) = 32), \
             user_id bytea NOT NULL, tenant bytea, scopes_sealed bytea NOT NULL, \
             created_at bigint NOT NULL, last_seen_at bigint NOT NULL, \
             idle_expires_at bigint NOT NULL, expires_at bigint NOT NULL, \
             idle_seconds bigint NOT NULL, amr_sealed bytea NOT NULL, \
             csrf_hash bytea NOT NULL CHECK (octet_length(csrf_hash) = 32) );",
            "CREATE INDEX session_of_user ON vcon.session (user_id);",
            "CREATE INDEX session_in_tenant ON vcon.session (tenant);",
            "ALTER TABLE vcon.session ENABLE ROW LEVEL SECURITY;",
            "ALTER TABLE vcon.session FORCE ROW LEVEL SECURITY;",
            "CREATE POLICY registry_rows ON vcon.session \
             USING (vcon.in_registry() = (id_hash IS NOT NULL)) WITH CHECK (vcon.in_registry());",
            "GRANT SELECT, INSERT, UPDATE, DELETE ON vcon.session TO vcon_app;",
        ],
        "the whole migration, besides its comments"
    );
}

#[test]
fn the_migration_keeps_no_session_id_and_no_token() {
    let columns = statements(MIGRATIONS[4]).remove(0);
    for kept in ["id_hash", "csrf_hash"] {
        assert!(columns.contains(kept), "{kept}");
    }
    for never in [" id bytea", "token", "secret", "cookie"] {
        assert!(!columns.contains(never), "no column for {never:?}");
    }
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::super::live::value;
    use super::super::live::{Db, TENANT_A, count, in_scope};

    /// A database at version 4.
    async fn at_version_4() -> Db {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&MIGRATIONS[..4]).await.expect("version 4"),
            Migrated { from: 0, to: 4 },
            "the first four migrations alone"
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
        table_schema = 'vcon' AND table_name = 'session'";

    const COLUMNS: &str = "id_hash, user_id, tenant, scopes_sealed, created_at, last_seen_at, \
        idle_expires_at, expires_at, idle_seconds, amr_sealed, csrf_hash";

    /// An INSERT of one session whose id's hash is 32 bytes of `byte`, of
    /// the user `user`, in `tenant` (SQL text: a bytea literal or NULL).
    fn session(byte: &str, user: &str, tenant: &str) -> String {
        format!(
            "INSERT INTO vcon.session ({COLUMNS}) VALUES (decode(repeat('{byte}', 32), 'hex'), \
             '\\x{user}', {tenant}, '\\x5c09e5', 100, 100, 1000, 5000, 900, '\\xa312', \
             decode(repeat('ee', 32), 'hex'))"
        )
    }

    async fn live(db: &Db) -> Option<String> {
        read(
            db,
            "SELECT coalesce(string_agg(encode(substr(id_hash, 1, 1), 'hex'), ',' \
             ORDER BY id_hash), '') FROM vcon.session",
        )
        .await
    }

    // What must work.

    #[tokio::test]
    async fn a_version_4_database_is_brought_to_this_version_with_its_rows() {
        let db = at_version_4().await;
        in_scope(
            &db.admin().await,
            "registry",
            "INSERT INTO vcon.tenant_setting (tenant, name, value_sealed) \
             VALUES ('\\x01', 'session.idle_seconds', '\\xc0ffee')",
        )
        .await
        .expect("a setting a version-4 build kept");
        assert_eq!(
            db.migrate().await,
            Migrated {
                from: 4,
                to: SCHEMA_VERSION
            },
            "the migrations after the fourth run"
        );
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.tenant_setting")
                .await
                .as_deref(),
            Some("1"),
            "the setting is kept"
        );
        assert_eq!(
            read(&db, "SELECT count(*) FROM vcon.session")
                .await
                .as_deref(),
            Some("0"),
            "no one is signed in"
        );
    }

    #[tokio::test]
    async fn the_table_is_made_by_version_5_and_not_before() {
        let db = at_version_4().await;
        assert_eq!(count(&db.admin().await, TABLE).await, 0, "at version 4");
        db.migrate().await;
        assert_eq!(count(&db.admin().await, TABLE).await, 1, "after version 5");
    }

    #[tokio::test]
    async fn the_application_role_starts_sees_and_ends_a_session() {
        let db = Db::migrated().await;
        registry(&db, &session("a1", "51", "'\\x01'"))
            .await
            .expect("a tenant's session");
        registry(&db, &session("b2", "52", "NULL"))
            .await
            .expect("an operator's session");
        registry(
            &db,
            "UPDATE vcon.session SET last_seen_at = 400, idle_expires_at = 1300 \
             WHERE id_hash = decode(repeat('a1', 32), 'hex')",
        )
        .await
        .expect("seen again");
        assert_eq!(
            read(
                &db,
                "SELECT last_seen_at || '/' || idle_expires_at || '/' || expires_at \
                 FROM vcon.session WHERE id_hash = decode(repeat('a1', 32), 'hex')"
            )
            .await
            .as_deref(),
            Some("400/1300/5000"),
            "the two times moved and the end did not"
        );
        registry(
            &db,
            "DELETE FROM vcon.session WHERE id_hash = decode(repeat('a1', 32), 'hex')",
        )
        .await
        .expect("ended");
        assert_eq!(live(&db).await.as_deref(), Some("b2"), "the other stays");
    }

    #[tokio::test]
    async fn sessions_are_removed_by_user_by_tenant_and_by_their_limits() {
        let db = Db::migrated().await;
        for (byte, user, tenant) in [
            ("a1", "51", "'\\x01'"),
            ("a2", "51", "'\\x02'"),
            ("a3", "52", "'\\x01'"),
            ("a4", "52", "'\\x02'"),
            ("a5", "52", "NULL"),
        ] {
            registry(&db, &session(byte, user, tenant))
                .await
                .expect("a session");
        }
        registry(&db, "DELETE FROM vcon.session WHERE user_id = '\\x51'")
            .await
            .expect("a user's");
        assert_eq!(live(&db).await.as_deref(), Some("a3,a4,a5"), "by user");
        registry(&db, "DELETE FROM vcon.session WHERE tenant = '\\x01'")
            .await
            .expect("a tenant's");
        assert_eq!(live(&db).await.as_deref(), Some("a4,a5"), "by tenant");
        registry(
            &db,
            "UPDATE vcon.session SET expires_at = 50 WHERE tenant IS NULL",
        )
        .await
        .expect("one past its end");
        registry(
            &db,
            "DELETE FROM vcon.session WHERE idle_expires_at <= 60 OR expires_at <= 60",
        )
        .await
        .expect("the ended ones");
        assert_eq!(live(&db).await.as_deref(), Some("a4"), "by a limit");
    }

    #[tokio::test]
    async fn a_serving_connection_is_accepted_once_this_version_is_there() {
        let db = at_version_4().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_err(),
            "refused at version 4"
        );
        db.migrate().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_ok(),
            "accepted at this binary's version"
        );
    }

    #[tokio::test]
    async fn a_new_database_reaches_this_version_with_the_table() {
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

    #[tokio::test]
    async fn the_table_has_its_two_indexes() {
        let db = Db::migrated().await;
        assert_eq!(
            value(
                &db.admin().await,
                "SELECT string_agg(indexname, ',' ORDER BY indexname) FROM pg_indexes \
                 WHERE schemaname = 'vcon' AND tablename = 'session'"
            )
            .await,
            "session_in_tenant,session_of_user,session_pkey",
            "by tenant, by user, and the key"
        );
    }

    // What must be refused.

    #[tokio::test]
    async fn a_version_5_that_fails_leaves_version_4_whole() {
        let db = at_version_4().await;
        db.admin()
            .await
            .batch_execute("SET ROLE vcon_owner; CREATE TABLE vcon.session (other int); RESET ROLE")
            .await
            .expect("a table in the way");
        let refused = db.migrate_with(MIGRATIONS).await.expect_err("refused");
        assert!(
            refused.contains("version 5"),
            "names the version: {refused}"
        );
        assert_eq!(
            value(
                &db.admin().await,
                "SELECT max(version) FROM vcon.schema_version"
            )
            .await,
            "4",
            "still version 4"
        );
    }

    #[tokio::test]
    async fn a_tenants_transaction_reads_and_writes_no_session() {
        let db = Db::migrated().await;
        registry(&db, &session("a1", "51", &format!("'\\x{}'", "01")))
            .await
            .expect("a session");
        let app = db.app().await;
        let seen = in_scope(&app, TENANT_A, "SELECT count(*) FROM vcon.session").await;
        let rows = seen.map(|messages| {
            messages.iter().find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_owned),
                _ => None,
            })
        });
        assert!(
            !matches!(rows.as_ref().map(Option::as_deref), Ok(Some(n)) if n != "0"),
            "a tenant's transaction sees no session: {rows:?}"
        );
        assert!(
            in_scope(&app, TENANT_A, &session("a2", "51", "'\\x01'"))
                .await
                .is_err(),
            "and starts none"
        );
        let _ = in_scope(&app, TENANT_A, "DELETE FROM vcon.session").await;
        assert_eq!(live(&db).await.as_deref(), Some("a1"), "and ends none");
    }

    #[tokio::test]
    async fn the_application_role_cannot_drop_or_change_the_table() {
        let db = Db::migrated().await;
        for statement in [
            "DROP TABLE vcon.session",
            "ALTER TABLE vcon.session DISABLE ROW LEVEL SECURITY",
            "ALTER TABLE vcon.session ADD COLUMN extra int",
            "TRUNCATE vcon.session",
        ] {
            assert!(
                registry(&db, statement).await.is_err(),
                "the application role is refused: {statement}"
            );
        }
    }

    #[tokio::test]
    async fn a_second_session_with_one_id_hash_is_refused() {
        let db = Db::migrated().await;
        registry(&db, &session("a1", "51", "NULL"))
            .await
            .expect("the first row");
        let second = registry(&db, &session("a1", "52", "'\\x01'"))
            .await
            .expect_err("the second row");
        assert!(
            second.contains("duplicate key"),
            "one row for an id: {second}"
        );
    }

    #[tokio::test]
    async fn a_row_that_lacks_a_part_is_refused() {
        let db = Db::migrated().await;
        let whole = session("a1", "51", "NULL");
        for (part, null) in [
            ("decode(repeat('a1', 32), 'hex')", "NULL"),
            ("'\\x51'", "NULL::bytea"),
            ("'\\x5c09e5'", "NULL"),
            ("'\\xa312'", "NULL"),
            ("decode(repeat('ee', 32), 'hex')", "NULL"),
            ("100, 100, 1000, 5000, 900", "NULL, 100, 1000, 5000, 900"),
            ("100, 100, 1000, 5000, 900", "100, NULL, 1000, 5000, 900"),
            ("100, 100, 1000, 5000, 900", "100, 100, NULL, 5000, 900"),
            ("100, 100, 1000, 5000, 900", "100, 100, 1000, NULL, 900"),
            ("100, 100, 1000, 5000, 900", "100, 100, 1000, 5000, NULL"),
        ] {
            assert!(whole.contains(part), "the row has {part}");
            let lacking = whole.replacen(part, null, 1);
            assert!(
                registry(&db, &lacking).await.is_err(),
                "a row with a NULL is refused: {lacking}"
            );
        }
        registry(&db, &whole).await.expect("the whole row is taken");
    }

    #[tokio::test]
    async fn a_hash_that_is_not_32_bytes_is_refused() {
        let db = Db::migrated().await;
        let whole = session("a1", "51", "NULL");
        for (part, other) in [
            ("repeat('a1', 32)", "repeat('a1', 31)"),
            ("repeat('a1', 32)", "repeat('a1', 33)"),
            ("repeat('ee', 32)", "repeat('ee', 31)"),
            ("repeat('ee', 32)", "repeat('ee', 33)"),
        ] {
            let wrong = whole.replacen(part, other, 1);
            let refused = registry(&db, &wrong).await.expect_err("refused");
            assert!(refused.contains("check constraint"), "{other}: {refused}");
        }
    }

    // Limits and odd inputs.

    #[tokio::test]
    async fn migrating_twice_from_version_4_does_the_work_once() {
        let db = at_version_4().await;
        let now = SCHEMA_VERSION;
        assert_eq!(
            db.migrate().await,
            Migrated { from: 4, to: now },
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
    async fn one_user_may_have_many_sessions_in_one_tenant() {
        let db = Db::migrated().await;
        for byte in ["a1", "a2", "a3"] {
            registry(&db, &session(byte, "51", "'\\x01'"))
                .await
                .expect("a session");
        }
        assert_eq!(
            live(&db).await.as_deref(),
            Some("a1,a2,a3"),
            "three browsers"
        );
    }

    #[tokio::test]
    async fn an_operator_s_session_has_no_tenant_and_is_not_removed_with_any() {
        let db = Db::migrated().await;
        registry(&db, &session("a1", "51", "NULL"))
            .await
            .expect("an operator's session");
        registry(&db, "DELETE FROM vcon.session WHERE tenant = '\\x01'")
            .await
            .expect("a tenant's sessions");
        assert_eq!(live(&db).await.as_deref(), Some("a1"), "still there");
    }

    #[tokio::test]
    async fn removing_the_sessions_of_a_user_that_has_none_removes_nothing() {
        let db = Db::migrated().await;
        registry(&db, &session("a1", "51", "NULL"))
            .await
            .expect("a session");
        registry(&db, "DELETE FROM vcon.session WHERE user_id = '\\x52'")
            .await
            .expect("no error");
        assert_eq!(live(&db).await.as_deref(), Some("a1"), "the one session");
    }
}
