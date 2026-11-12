//! The limits and odd inputs.

use super::super::super::schema::version_refusal;

#[test]
fn a_recorded_version_of_zero_is_older_not_absent() {
    let why = version_refusal(Some(0)).expect("refused");
    assert!(
        !why.contains("no vCon schema") && why.contains("older"),
        "an installed schema is named as older: {why}"
    );
}

#[test]
fn the_highest_version_is_refused_without_overflow() {
    assert!(version_refusal(Some(i32::MAX)).is_some(), "refused");
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::super::connect::Purpose;
    use super::super::super::super::schema::{MIGRATIONS, Migrated, SCHEMA_VERSION};
    use super::super::super::live::value;
    use super::super::live::{Db, TENANT_A, count, in_scope, insert_tenant_row, no_scope};

    #[tokio::test]
    async fn an_empty_list_of_migrations_changes_nothing() {
        let db = Db::fresh().await;
        assert_eq!(
            db.migrate_with(&[]).await.expect("no work"),
            Migrated { from: 0, to: 0 },
            "zero migrations, zero to zero"
        );
        assert_eq!(
            value(&db.admin().await, "SELECT to_regnamespace('vcon') IS NULL").await,
            "t",
            "and no schema was made"
        );
    }

    #[tokio::test]
    async fn two_migrates_at_once_leave_the_schema_correct() {
        let db = Db::fresh().await;
        let (a, b) = tokio::join!(db.migrate_with(MIGRATIONS), db.migrate_with(MIGRATIONS));
        for result in [&a, &b] {
            match result {
                Ok(_) => {}
                Err(e) => assert!(e.contains("another"), "only the lock refuses: {e}"),
            }
        }
        assert!(a.is_ok() || b.is_ok(), "one of them succeeded: {a:?} {b:?}");
        assert_eq!(
            count(
                &db.admin().await,
                "SELECT count(*) FROM vcon.schema_version"
            )
            .await,
            i64::from(SCHEMA_VERSION),
            "one row for each version"
        );
    }

    #[tokio::test]
    async fn a_new_connection_rechecks_the_version_though_an_old_one_stays() {
        let db = Db::migrated().await;
        let open = db.app().await;
        db.admin()
            .await
            .batch_execute(&format!(
                "SET ROLE vcon_owner; INSERT INTO vcon.schema_version (version) VALUES ({}); \
                 RESET ROLE",
                SCHEMA_VERSION + 1
            ))
            .await
            .expect("adds a version");
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_err(),
            "the next new connection is refused"
        );
        no_scope(&open, "SELECT 1")
            .await
            .expect("the open one still answers");
    }

    #[tokio::test]
    async fn migrating_after_a_refusal_lets_the_server_in() {
        let db = Db::fresh().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_err(),
            "refused first"
        );
        db.migrate().await;
        assert!(
            db.connect("vcon_app", Purpose::Serve).await.is_ok(),
            "then accepted"
        );
    }

    #[tokio::test]
    async fn an_empty_version_table_reads_as_no_schema() {
        let db = Db::migrated().await;
        db.admin()
            .await
            .batch_execute("SET ROLE vcon_owner; DELETE FROM vcon.schema_version; RESET ROLE")
            .await
            .expect("empties it");
        let e = db
            .connect("vcon_app", Purpose::Serve)
            .await
            .expect_err("refused")
            .to_string();
        assert!(e.contains("no vCon schema"), "says so: {e}");
    }

    #[tokio::test]
    async fn the_owner_role_is_held_to_row_level_security_as_well() {
        let db = Db::migrated().await;
        let admin = db.admin().await;
        admin
            .batch_execute("SET ROLE vcon_owner")
            .await
            .expect("sets the role");
        let e = no_scope(&admin, "SELECT * FROM vcon.version")
            .await
            .expect_err("refused to the owner too");
        assert!(e.contains("no tenant set"), "FORCE row-level security: {e}");
    }

    #[tokio::test]
    async fn a_cached_plan_does_not_turn_a_missing_tenant_into_no_rows() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_tenant_row("tombstone", TENANT_A))
            .await
            .expect("inserts");
        // Five custom plans, then the generic plan the sixth caches.
        app.batch_execute("PREPARE q AS SELECT tenant FROM vcon.tombstone")
            .await
            .expect("prepares");
        for _ in 0..7 {
            in_scope(&app, TENANT_A, "EXECUTE q").await.expect("runs");
        }
        let e = no_scope(&app, "EXECUTE q").await.expect_err("refused");
        assert!(
            e.contains("no tenant set"),
            "the cached plan still refuses: {e}"
        );
    }

    #[tokio::test]
    async fn the_identity_columns_take_values_without_a_sequence_grant() {
        let db = Db::migrated().await;
        let app = db.app().await;
        in_scope(&app, TENANT_A, &insert_tenant_row("manifest", TENANT_A))
            .await
            .expect("the role needs no privilege on the identity sequence");
        in_scope(&app, TENANT_A, &insert_tenant_row("manifest", TENANT_A))
            .await
            .expect("and takes the next number");
        assert_eq!(
            in_scope(
                &app,
                TENANT_A,
                "SELECT count(DISTINCT seq) FROM vcon.manifest"
            )
            .await
            .map(|rows| rows.iter().find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_owned),
                _ => None,
            }))
            .expect("reads"),
            Some("2".to_owned()),
            "two different numbers"
        );
    }

    #[tokio::test]
    async fn an_interrupted_migrate_is_retried_from_the_version_it_reached() {
        let db = Db::fresh().await;
        let v2 = "CREATE TABLE vcon.extra (a int); SELECT boom";
        assert!(
            db.migrate_with(&[MIGRATIONS[0], v2]).await.is_err(),
            "the first run fails"
        );
        assert_eq!(
            db.migrate_with(&[MIGRATIONS[0]]).await.expect("no work"),
            Migrated { from: 1, to: 1 },
            "the binary at version 1 finds what the failed run committed"
        );
    }
}
