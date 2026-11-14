//! The claim of the bucket prefix that a database keeps (schema version
//! 13, `backend/postgres/claim.rs`): the row a server reads and makes,
//! and a start on a real bucket with the claim in a database. They need a
//! PostgreSQL server and an S3-compatible service.

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use std::sync::Arc;

    use super::super::super::catalog::Database;
    use super::super::super::claim::PgClaim;
    use super::super::catalog_tests::on_a_server::database;
    use super::super::schema_tests::live::{Db, in_scope};
    use crate::backend::Objects;
    use crate::backend::s3::claim::{self, Home};
    use crate::backend::s3::remote::{bucket, secret_file};
    use crate::backend::s3::{S3, place, place_for};
    use crate::config::{S3Settings, StorageError};
    use crate::ids::{TenantId, VersionId};

    /// Two claim ids.
    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "fedcba9876543210fedcba9876543210";

    /// Runs `call` on a runtime of its own, for a test's own connections.
    fn wait<T>(call: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(call)
    }

    /// The claims in the table, as the application role reads them in a
    /// transaction of the registry, joined by commas.
    fn rows(db: &Db) -> String {
        wait(async {
            in_scope(
                &db.app().await,
                "registry",
                "SELECT coalesce(string_agg(claim_id, ','), '') FROM vcon.storage_claim",
            )
            .await
            .expect("reads the table")
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(r) => r.get(0).map(str::to_owned),
                _ => None,
            })
            .expect("one row")
        })
    }

    /// Runs `sql` as the schema's owner.
    fn as_owner(db: &Db, sql: &str) {
        wait(async {
            let owner = db.admin().await;
            owner
                .batch_execute("SET ROLE vcon_owner")
                .await
                .expect("the owner role");
            owner
                .batch_execute(sql)
                .await
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        });
    }

    /// A migrated database and its claim's home.
    fn home() -> (Db, Arc<Database>, PgClaim) {
        let (db, database) = database();
        let home = PgClaim(database.clone());
        (db, database, home)
    }

    /// A new bucket with the secret in a file, as a start reads it.
    fn bucket_with_secret() -> (Arc<S3>, S3Settings, (tempfile::TempDir, std::path::PathBuf)) {
        let secret = secret_file();
        let (s3, mut settings) = bucket(false);
        settings.secret_access_key_file = Some(secret.1.clone());
        (s3, settings, secret)
    }

    // What must work.

    #[test]
    fn a_database_is_named_a_database_in_a_refusal() {
        let (_db, _database, home) = home();
        assert_eq!(home.noun(), "database", "as a refusal names it");
    }

    #[test]
    fn a_migrated_database_keeps_no_claim() {
        let (_db, _database, home) = home();
        assert_eq!(home.read(), Ok(None), "no row yet");
    }

    #[test]
    fn a_claim_kept_is_the_row_and_is_read_back() {
        let (db, _database, home) = home();
        assert_eq!(home.keep(A), Ok(A.to_owned()), "the id given is kept");
        assert_eq!(rows(&db), A, "in the table's one row");
        assert_eq!(home.read(), Ok(Some(A.to_owned())), "and read back");
    }

    #[test]
    fn the_next_server_on_the_database_reads_the_same_claim() {
        let (db, database, home) = home();
        home.keep(A).expect("keeps");
        database.give_up_lock();
        drop((home, database));
        let (_dir, settings) = db.settings("vcon_app");
        let next = PgClaim(Database::connect(&settings).expect("the next server connects"));
        assert_eq!(next.read(), Ok(Some(A.to_owned())), "the claim kept");
    }

    #[test]
    fn a_database_claims_an_empty_prefix_and_starts_on_it_again() {
        let (db, _database, home) = home();
        let (s3, settings, _secret) = bucket_with_secret();
        place_for(&settings, false, &home).expect("the database claims the prefix");
        let id = home
            .read()
            .expect("reads")
            .expect("the database keeps its claim");
        assert_eq!(rows(&db), id, "in the table's one row");
        assert_eq!(
            s3.read_claim().expect("reads"),
            Some(id.clone()),
            "the bucket holds the same claim"
        );
        place_for(&settings, false, &home).expect("the claiming database starts again");
        assert_eq!(home.read(), Ok(Some(id)), "with the claim it made");
    }

    // What must be refused.

    #[test]
    fn a_text_that_is_not_a_claim_id_is_not_kept_and_not_sent() {
        let (db, database, home) = home();
        let before = database.round_trips();
        for text in ["", "not a claim", &A.to_uppercase(), &format!("{A}\n")] {
            let StorageError(why) = home.keep(text).expect_err("refused");
            assert!(
                why.contains("catalog.postgres") && why.contains("not a claim id"),
                "{text:?}: {why}"
            );
        }
        assert_eq!(database.round_trips(), before, "nothing was sent");
        assert_eq!(rows(&db), "", "no row was made");
    }

    #[test]
    fn a_claim_that_cannot_be_read_is_an_error_and_not_no_claim() {
        let (db, _database, home) = home();
        home.keep(A).expect("keeps");
        as_owner(&db, "REVOKE SELECT ON vcon.storage_claim FROM vcon_app");
        let StorageError(why) = home.read().expect_err("refused");
        assert!(
            why.contains("catalog.postgres")
                && why.contains("the claim of the bucket prefix cannot be read"),
            "the table and the step: {why}"
        );
    }

    #[test]
    fn a_claim_that_cannot_be_written_is_an_error_and_no_claim_is_given() {
        let (db, _database, home) = home();
        as_owner(&db, "REVOKE INSERT ON vcon.storage_claim FROM vcon_app");
        let StorageError(why) = home.keep(A).expect_err("refused");
        assert!(
            why.contains("catalog.postgres")
                && why.contains("the claim of the bucket prefix cannot be written"),
            "the table and the step: {why}"
        );
        assert_eq!(home.read(), Ok(None), "no claim is kept");
    }

    #[test]
    fn a_row_that_holds_no_claim_id_is_refused() {
        let (db, _database, home) = home();
        home.keep(A).expect("keeps");
        // The table's check refuses such a row; without the check, the
        // server's own rule still does.
        as_owner(
            &db,
            "ALTER TABLE vcon.storage_claim DROP CONSTRAINT storage_claim_claim_id_check; \
             SET vcon.tenant = 'registry'; \
             UPDATE vcon.storage_claim SET claim_id = 'not a claim'",
        );
        assert_eq!(rows(&db), "not a claim", "the row a hand changed");
        let StorageError(why) = home.read().expect_err("refused");
        assert!(
            why.contains("catalog.postgres") && why.contains("does not hold a claim id"),
            "says what the row holds: {why}"
        );
        let StorageError(why) = home.keep(B).expect_err("refused");
        assert!(
            why.contains("does not hold a claim id"),
            "a keep reads the same row: {why}"
        );
    }

    #[test]
    fn a_second_database_on_one_prefix_is_refused_and_keeps_no_claim() {
        let (_first_db, _first_database, first) = home();
        let (second_db, _second_database, second) = home();
        let (s3, settings, _secret) = bucket_with_secret();
        place_for(&settings, false, &first).expect("the first database claims the prefix");
        let claimed = s3.read_claim().expect("reads");
        let refused = place_for(&settings, false, &second).expect_err("another database");
        assert!(
            refused.0.contains("another database") && refused.0.contains(&settings.bucket),
            "the refusal says why and names the bucket: {refused}"
        );
        assert_eq!(second.read(), Ok(None), "a refused database claims nothing");
        assert_eq!(rows(&second_db), "", "its table has no row");
        assert_eq!(
            s3.read_claim().expect("reads"),
            claimed,
            "the bucket's claim is the first database's"
        );
    }

    #[test]
    fn a_database_is_refused_the_prefix_of_a_data_directory() {
        let (db, _database, home) = home();
        let (s3, settings, _secret) = bucket_with_secret();
        let data = tempfile::tempdir().expect("dir");
        place(&settings, false, data.path()).expect("the data directory claims the prefix");
        let refused = place_for(&settings, false, &home).expect_err("another's prefix");
        assert!(
            refused.0.contains("belongs to another database"),
            "a database is told of other catalogs in its own words: {refused}"
        );
        assert_eq!(rows(&db), "", "the database claims nothing");
        assert_eq!(
            s3.read_claim().expect("reads"),
            claim::read_file(data.path()).expect("reads"),
            "the bucket's claim is the data directory's"
        );
    }

    #[test]
    fn a_data_directory_is_refused_the_prefix_of_a_database() {
        let (_db, _database, home) = home();
        let (_s3, settings, _secret) = bucket_with_secret();
        place_for(&settings, false, &home).expect("the database claims the prefix");
        let data = tempfile::tempdir().expect("dir");
        let refused = place(&settings, false, data.path()).expect_err("another's prefix");
        assert!(
            refused.0.contains("belongs to another data directory"),
            "the data directory's refusal, as before: {refused}"
        );
        assert_eq!(
            claim::read_file(data.path()),
            Ok(None),
            "the data directory claims nothing"
        );
    }

    #[test]
    fn a_prefix_holding_objects_and_no_claim_is_refused_to_a_database_and_kept() {
        let (db, _database, home) = home();
        let (s3, settings, _secret) = bucket_with_secret();
        let tenant = TenantId::generate();
        s3.tenant(tenant)
            .put(VersionId::generate(), b"another server's object")
            .expect("put");
        let refused = place_for(&settings, false, &home).expect_err("unclaimed objects");
        assert!(
            refused.0.contains("holds tenant objects but no claim;"),
            "{refused}"
        );
        assert_eq!(
            s3.tenant(tenant).list().expect("list").len(),
            1,
            "nothing is removed"
        );
        assert_eq!(s3.read_claim().expect("reads"), None, "nothing is claimed");
        assert_eq!(rows(&db), "", "in the database either");
    }

    // The edges.

    #[test]
    fn a_second_claim_keeps_the_first_and_gives_it() {
        let (db, _database, home) = home();
        assert_eq!(home.keep(A), Ok(A.to_owned()), "the first");
        assert_eq!(home.keep(B), Ok(A.to_owned()), "the claim that is kept");
        assert_eq!(rows(&db), A, "one row, the first");
    }

    #[test]
    fn claims_made_at_once_keep_one_id_and_every_one_is_given_it() {
        let (db, database, _home) = home();
        let ids: Vec<String> = (0..4).map(|_| claim::new_id()).collect();
        let kept: Vec<String> = std::thread::scope(|scope| {
            let threads: Vec<_> = ids
                .iter()
                .map(|id| {
                    let home = PgClaim(database.clone());
                    scope.spawn(move || home.keep(id).expect("keeps"))
                })
                .collect();
            threads
                .into_iter()
                .map(|t| t.join().expect("the thread ends"))
                .collect()
        });
        let row = rows(&db);
        assert!(ids.contains(&row), "the row is one of the ids given: {row}");
        for given in &kept {
            assert_eq!(given, &row, "every claimant is given the row's id");
        }
    }

    #[test]
    fn a_read_and_a_keep_give_their_connection_back() {
        let (_db, database, home) = home();
        home.read().expect("reads");
        let idle = database.idle();
        assert!(idle >= 1, "the connection that read waits");
        home.keep(A).expect("keeps");
        home.read().expect("reads");
        home.keep(B).expect("keeps");
        assert!(
            home.keep("not a claim").is_err(),
            "a refused keep takes none"
        );
        assert_eq!(database.idle(), idle, "none is held and none is added");
    }

    #[test]
    fn a_claim_refused_once_is_kept_when_the_cause_is_gone() {
        let (db, database, home) = home();
        as_owner(&db, "REVOKE INSERT ON vcon.storage_claim FROM vcon_app");
        assert!(home.keep(A).is_err(), "refused while the right is gone");
        let idle = database.idle();
        as_owner(&db, "GRANT INSERT ON vcon.storage_claim TO vcon_app");
        assert_eq!(home.keep(B), Ok(B.to_owned()), "the claim asked for now");
        assert_eq!(
            database.idle(),
            idle,
            "the refused keep left its connection usable"
        );
    }

    #[test]
    fn a_database_s_claim_is_written_again_to_its_emptied_prefix() {
        let (_db, _database, home) = home();
        let (s3, settings, _secret) = bucket_with_secret();
        place_for(&settings, false, &home).expect("claims");
        let id = home.read().expect("reads").expect("a claim");
        s3.remove_named(claim::OBJECT)
            .expect("the bucket is emptied");
        assert_eq!(s3.read_claim().expect("reads"), None, "no claim object");
        place_for(&settings, false, &home).expect("the claim is finished");
        assert_eq!(
            s3.read_claim().expect("reads"),
            Some(id.clone()),
            "the database's claim"
        );
        assert_eq!(home.read(), Ok(Some(id)), "which is as it was");
    }

    #[test]
    fn a_database_takes_an_empty_second_prefix_with_the_claim_it_has() {
        let (db, _database, home) = home();
        let (s3, settings, secret) = bucket_with_secret();
        place_for(&settings, false, &home).expect("claims");
        let id = s3.read_claim().expect("reads").expect("a claim");
        let mut other = settings.clone();
        other.prefix = "other/".to_owned();
        place_for(&other, false, &home).expect("a second prefix is used");
        assert_eq!(rows(&db), id, "the database has one claim");
        let key = std::fs::read_to_string(&secret.1).expect("the secret file");
        let second = S3::connect(&other, Some(key.trim_end()), None).expect("a client");
        assert_eq!(
            second.read_claim().expect("reads"),
            Some(id),
            "written under the second prefix too"
        );
    }
}
