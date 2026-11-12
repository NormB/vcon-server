//! A live test's database is its own, and is dropped when the test ends,
//! pass or fail. A server that tests ran on for hours once held thousands
//! of such databases: PostgreSQL keeps statistics for each in shared
//! memory, the container's `/dev/shm` filled, and every new connection
//! failed with `could not resize shared memory segment`. A test that is
//! killed drops nothing, so each new database first removes the ones old
//! enough that no running test can own them.

use super::names::{STALE_AFTER, is_stale, name_at};

const NOW: u64 = 1_800_000_000;

#[test]
fn a_name_holds_the_time_it_was_made_and_its_random_part() {
    assert_eq!(
        name_at(0x6b49_d200, &[0xab, 0xcd]),
        "vcon_t_6b49d200_abcd",
        "the prefix, the seconds in hexadecimal, the random bytes"
    );
}

#[test]
fn a_database_just_made_is_not_stale() {
    assert!(!is_stale(&name_at(NOW, &[1]), NOW), "made this second");
    assert!(
        !is_stale(&name_at(NOW - 60, &[1]), NOW),
        "a test a minute old may still run"
    );
}

#[test]
fn a_database_older_than_the_limit_is_stale() {
    assert!(
        is_stale(&name_at(NOW - STALE_AFTER - 1, &[1]), NOW),
        "one second past the limit"
    );
    assert!(is_stale(&name_at(1, &[1]), NOW), "from long ago");
}

#[test]
fn a_database_exactly_at_the_limit_is_kept() {
    assert!(
        !is_stale(&name_at(NOW - STALE_AFTER, &[1]), NOW),
        "the limit itself is not past it"
    );
}

#[test]
fn a_database_from_a_clock_ahead_of_this_one_is_kept() {
    assert!(
        !is_stale(&name_at(NOW + 3600, &[1]), NOW),
        "a time in the future is not an age"
    );
}

#[test]
fn a_database_that_is_not_a_tests_is_never_stale() {
    for name in [
        "vcon",
        "postgres",
        "template1",
        "vcon_t",
        "vcon_t_",
        "vcon_test_1_ab",
        "other_t_1_ab",
    ] {
        assert!(!is_stale(name, NOW), "{name} is not swept");
    }
}

#[test]
fn a_test_name_whose_time_does_not_parse_is_never_stale() {
    for name in [
        "vcon_t_zz_ab",
        "vcon_t__ab",
        "vcon_t_0123456789abcdef",
        "vcon_t_ffffffffffffffffff_ab",
        "vcon_t_-1_ab",
    ] {
        assert!(!is_stale(name, NOW), "{name} is left alone");
    }
}

#[test]
fn the_limit_is_longer_than_any_test_may_run() {
    // .config/nextest.toml ends a test well inside this.
    assert!(STALE_AFTER >= 900, "at least a quarter of an hour");
}

#[cfg(feature = "backend-tests")]
mod on_a_server {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::super::super::super::connect::Purpose;
    use super::super::super::live::{as_user, value};
    use super::super::live::{Db, connect};
    use super::super::names::{STALE_AFTER, name_at};

    /// The tester's connection to the server's own database.
    async fn tester() -> tokio_postgres::Client {
        let (_dir, settings) = as_user("vcon_tester");
        connect(&settings, Purpose::Connect)
            .await
            .expect("the tester connects")
            .client
    }

    async fn exists(client: &tokio_postgres::Client, name: &str) -> bool {
        value(
            client,
            &format!("SELECT count(*) FROM pg_database WHERE datname = '{name}'"),
        )
        .await
            == "1"
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_secs()
    }

    #[tokio::test]
    async fn a_tests_database_is_there_while_the_test_runs_and_gone_when_it_ends() {
        let tester = tester().await;
        let db = Db::migrated().await;
        let name = db.name.clone();
        assert!(
            exists(&tester, &name).await,
            "there while the test holds it"
        );
        drop(db);
        assert!(
            !exists(&tester, &name).await,
            "dropped with the test's guard"
        );
    }

    #[test]
    fn a_tests_database_is_gone_when_the_test_panics() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let named = std::sync::Mutex::new(String::new());
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let db = runtime.block_on(Db::fresh());
            named.lock().expect("the name").clone_from(&db.name);
            assert!(
                db.name.is_empty(),
                "a test that fails while it holds a database"
            );
        }));
        assert!(failed.is_err(), "the test inside failed");
        let name = named.lock().expect("the name").clone();
        assert!(
            name.starts_with("vcon_t_"),
            "it had made a database: {name}"
        );
        let gone = runtime.block_on(async { !exists(&tester().await, &name).await });
        assert!(gone, "the guard dropped it while the panic unwound");
    }

    #[tokio::test]
    async fn a_database_with_open_connections_is_dropped_all_the_same() {
        let tester = tester().await;
        let db = Db::migrated().await;
        let name = db.name.clone();
        let held = db.app().await;
        drop(db);
        assert!(
            !exists(&tester, &name).await,
            "dropped though a connection was open"
        );
        assert!(
            held.simple_query("SELECT 1").await.is_err(),
            "the connection that was open was ended by the drop"
        );
    }

    #[tokio::test]
    async fn a_new_database_sweeps_one_a_killed_test_left_and_keeps_a_running_tests() {
        let tester = tester().await;
        let random: [u8; 6] = aws_lc_rs::rand::generate(&aws_lc_rs::rand::SystemRandom::new())
            .expect("random")
            .expose();
        let left = name_at(now() - STALE_AFTER - 60, &random);
        let running = name_at(now(), &random);
        for name in [&left, &running] {
            tester
                .batch_execute(&format!("CREATE DATABASE {name} OWNER vcon_owner"))
                .await
                .expect("a database made around the guard");
        }
        let db = Db::fresh().await;
        assert!(
            !exists(&tester, &left).await,
            "the one older than the limit was swept"
        );
        assert!(
            exists(&tester, &running).await,
            "the one another test could be running on was kept"
        );
        tester
            .batch_execute(&format!("DROP DATABASE {running} WITH (FORCE)"))
            .await
            .expect("this test removes what it made");
        drop(db);
    }

    #[tokio::test]
    async fn a_run_of_databases_leaves_none_behind() {
        let tester = tester().await;
        let mut names = Vec::new();
        for _ in 0..4 {
            let db = Db::fresh().await;
            names.push(db.name.clone());
        }
        let listed = names
            .iter()
            .map(|n| format!("'{n}'"))
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(
            value(
                &tester,
                &format!("SELECT count(*) FROM pg_database WHERE datname IN ({listed})")
            )
            .await,
            "0",
            "each was dropped when its guard went out of scope"
        );
        assert_eq!(
            names
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4,
            "and each had a name of its own"
        );
    }
}
