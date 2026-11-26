//! The live-test deadline (tests/support/live_deadline.rs) in the live
//! helpers' settings: `live_settings` without a server, against small
//! servers on the loopback address, and `as_user`, `with_certificate` and
//! the schema tests' `Db` on the servers `scripts/pg-test-server.sh`
//! starts. The server's own default, which those helpers once took, is
//! left as it is.

use std::time::{Duration, Instant};

use super::super::pool::Pool;
use super::{
    closed_port, connect, connector, file, live_settings, no_tls_server, pki, settings,
    silent_server, why,
};
use crate::config::{POSTGRES_CONNECT_TIMEOUT, POSTGRES_LOCK_TIMEOUT, POSTGRES_STATEMENT_TIMEOUT};
use crate::live_deadline::LIVE_DEADLINE;

/// A directory laid out as `pg-test-server.sh` writes it: the CA, and the
/// password `p`.
fn server_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("dir");
    file(dir.path(), "ca.pem", pki().ca.as_bytes(), 0o644);
    file(dir.path(), "password", b"p\n", 0o644);
    dir
}

/// One connection with `live_settings` for `port`, given `deadline`, and
/// how long it took to be refused.
async fn refused_within(port: u16, deadline: Duration) -> (String, Duration) {
    let dir = server_dir();
    let (_tmp, mut s) = live_settings(dir.path(), port, "vcon_app", deadline);
    s.hosts = vec!["127.0.0.1".to_owned()];
    let started = Instant::now();
    let message = why(connect(&s).await.expect_err("refused"));
    (message, started.elapsed())
}

mod success {
    use super::*;

    #[test]
    fn the_live_settings_give_each_connection_the_deadline_passed() {
        let dir = server_dir();
        let (_tmp, s) = live_settings(dir.path(), 5432, "vcon_app", Duration::from_secs(7));
        assert_eq!(
            s.connect_timeout,
            Duration::from_secs(7),
            "the deadline passed, not the server's default"
        );
    }

    #[test]
    fn the_live_settings_give_each_connection_the_live_deadline() {
        let dir = server_dir();
        let (_tmp, s) = live_settings(dir.path(), 5432, "vcon_app", LIVE_DEADLINE);
        assert_eq!(s.connect_timeout, LIVE_DEADLINE, "the shared live deadline");
    }

    #[test]
    fn the_live_settings_log_in_to_localhost_at_the_port_as_the_user() {
        let dir = server_dir();
        let (_tmp, s) = live_settings(dir.path(), 55432, "vcon_migrate", LIVE_DEADLINE);
        assert_eq!(s.hosts, vec!["localhost".to_owned()], "the host");
        assert_eq!(s.port, 55432, "the port");
        assert_eq!(s.user, "vcon_migrate", "the role");
        assert_eq!(s.dbname, "vcon", "the server's own database");
    }

    #[test]
    fn the_live_settings_trust_the_servers_ca() {
        let dir = server_dir();
        let (_tmp, s) = live_settings(dir.path(), 5432, "vcon_app", LIVE_DEADLINE);
        assert_eq!(
            s.ca_file,
            Some(dir.path().join("ca.pem")),
            "the servers' CA"
        );
    }

    #[test]
    fn the_live_settings_hold_a_private_copy_of_the_password() {
        use std::os::unix::fs::PermissionsExt;

        let dir = server_dir();
        let (tmp, s) = live_settings(dir.path(), 5432, "vcon_app", LIVE_DEADLINE);
        let copy = s.password_file.clone().expect("a password file");
        assert!(
            copy.starts_with(tmp.path()),
            "in the settings' own directory"
        );
        assert_eq!(std::fs::read(&copy).expect("reads"), b"p\n", "the password");
        let mode = std::fs::metadata(&copy).expect("stat").permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "mode 0600, which the server requires: {mode:o}"
        );
    }

    #[test]
    fn a_connector_is_made_from_the_live_settings() {
        let dir = server_dir();
        let (_tmp, s) = live_settings(dir.path(), 5432, "vcon_app", LIVE_DEADLINE);
        let _made = connector(&s);
    }
}

mod failure {
    use super::*;

    #[tokio::test]
    async fn a_server_that_never_answers_is_given_up_at_a_one_second_deadline_by_name() {
        let addr = silent_server().await;
        let (message, _took) = refused_within(addr.port(), Duration::from_secs(1)).await;
        assert!(
            message.contains("catalog.postgres.connect_timeout (1 s)"),
            "names the one-second deadline: {message}"
        );
    }

    #[tokio::test]
    async fn a_server_that_never_answers_is_given_up_at_a_two_second_deadline_by_name() {
        let addr = silent_server().await;
        let (message, took) = refused_within(addr.port(), Duration::from_secs(2)).await;
        assert!(
            message.contains("catalog.postgres.connect_timeout (2 s)"),
            "names the two-second deadline: {message}"
        );
        assert!(
            took >= Duration::from_secs(2) && took < Duration::from_secs(6),
            "given up at the deadline passed: {took:?}"
        );
    }

    #[tokio::test]
    async fn a_one_second_deadline_is_not_given_up_before_it_passes() {
        let addr = silent_server().await;
        let (_message, took) = refused_within(addr.port(), Duration::from_secs(1)).await;
        assert!(
            took >= Duration::from_secs(1),
            "waited the whole deadline: {took:?}"
        );
    }

    #[tokio::test]
    async fn a_closed_port_is_refused_long_before_the_live_deadline() {
        let port = closed_port("127.0.0.1").await;
        let (message, took) = refused_within(port, LIVE_DEADLINE).await;
        assert!(
            !message.contains("connect_timeout"),
            "refused, not timed out: {message}"
        );
        assert!(took < Duration::from_secs(5), "at once: {took:?}");
    }

    #[tokio::test]
    async fn a_server_without_tls_is_refused_long_before_the_live_deadline() {
        let addr = no_tls_server().await;
        let (message, took) = refused_within(addr.port(), LIVE_DEADLINE).await;
        assert!(
            message.contains("server does not support TLS"),
            "refused for its TLS: {message}"
        );
        assert!(took < Duration::from_secs(5), "at once: {took:?}");
    }

    #[tokio::test]
    async fn a_pool_from_the_live_settings_waits_the_deadline_for_a_server_that_never_answers() {
        let addr = silent_server().await;
        let dir = server_dir();
        let (_tmp, mut s) =
            live_settings(dir.path(), addr.port(), "vcon_app", Duration::from_secs(1));
        s.hosts = vec!["127.0.0.1".to_owned()];
        let pool = Pool::new(connector(&s), 1, s.connect_timeout);
        let started = Instant::now();
        let refused = pool.get().await.expect_err("no answer");
        let took = started.elapsed();
        assert!(
            refused.to_string().contains("(1 s)"),
            "names the one-second deadline: {refused}"
        );
        assert!(
            took < Duration::from_secs(5),
            "given up at the deadline: {took:?}"
        );
    }

    #[test]
    #[should_panic(expected = "the test password")]
    fn a_server_directory_without_the_password_is_named() {
        let dir = tempfile::tempdir().expect("dir");
        let _settings = live_settings(dir.path(), 5432, "vcon_app", LIVE_DEADLINE);
    }
}

mod edge {
    use super::*;

    #[test]
    fn the_servers_connect_timeout_default_is_unchanged() {
        assert_eq!(
            POSTGRES_CONNECT_TIMEOUT,
            Duration::from_secs(5),
            "production fails fast at 5 s"
        );
    }

    #[test]
    fn the_settings_for_a_fake_server_keep_the_servers_default() {
        // The tests against small servers on the loopback address time
        // the server's own default; only the live helpers take the live
        // deadline.
        let dir = server_dir();
        let s = settings(
            "127.0.0.1",
            5432,
            "vcon_app",
            &dir.path().join("ca.pem"),
            None,
        );
        assert_eq!(s.connect_timeout, POSTGRES_CONNECT_TIMEOUT, "the default");
    }

    #[test]
    fn the_live_deadline_is_longer_than_the_servers_default() {
        assert!(
            LIVE_DEADLINE > POSTGRES_CONNECT_TIMEOUT,
            "a loaded host gets more than {POSTGRES_CONNECT_TIMEOUT:?}: {LIVE_DEADLINE:?}"
        );
    }

    #[test]
    fn only_the_connect_timeout_differs_from_the_servers_defaults() {
        let dir = server_dir();
        let (_tmp, s) = live_settings(dir.path(), 5432, "vcon_app", LIVE_DEADLINE);
        assert_eq!(
            s.statement_timeout, POSTGRES_STATEMENT_TIMEOUT,
            "statements"
        );
        assert_eq!(s.lock_timeout, POSTGRES_LOCK_TIMEOUT, "locks");
        assert_eq!(s.pool, crate::config::POSTGRES_POOL, "the pool");
    }

    #[test]
    fn a_sub_second_deadline_is_kept_as_given() {
        let dir = server_dir();
        let (_tmp, s) = live_settings(dir.path(), 5432, "vcon_app", Duration::from_millis(1500));
        assert_eq!(
            s.connect_timeout,
            Duration::from_millis(1500),
            "not rounded: the settings take any duration"
        );
    }

    #[test]
    fn two_live_settings_have_directories_of_their_own() {
        let dir = server_dir();
        let (a, sa) = live_settings(dir.path(), 5432, "vcon_app", LIVE_DEADLINE);
        let (b, sb) = live_settings(dir.path(), 5432, "vcon_app", LIVE_DEADLINE);
        assert_ne!(a.path(), b.path(), "two directories");
        assert_ne!(sa.password_file, sb.password_file, "two copies");
    }

    #[tokio::test]
    async fn the_one_millisecond_deadline_is_missed_at_once() {
        let addr = silent_server().await;
        let (message, took) = refused_within(addr.port(), Duration::from_millis(1)).await;
        assert!(
            message.contains("catalog.postgres.connect_timeout"),
            "timed out by the deadline: {message}"
        );
        assert!(took < Duration::from_secs(2), "at once: {took:?}");
    }
}

/// On the servers `scripts/pg-test-server.sh` starts.
#[cfg(feature = "backend-tests")]
mod on_a_server {
    use super::super::super::connect::Purpose;
    use super::super::live::{Login, as_user, with_certificate};
    use super::super::pool;
    use super::super::schema_tests::live::Db;
    use super::*;

    #[test]
    fn the_application_role_is_given_the_live_deadline() {
        let (_tmp, s) = as_user("vcon_app");
        assert_eq!(
            s.connect_timeout, LIVE_DEADLINE,
            "as_user reads the shared value"
        );
    }

    #[test]
    fn the_tester_is_given_the_live_deadline() {
        let (_tmp, s) = as_user("vcon_tester");
        assert_eq!(
            s.connect_timeout, LIVE_DEADLINE,
            "as_user reads the shared value"
        );
    }

    #[test]
    fn a_certificate_login_is_given_the_live_deadline() {
        let (_tmp, s) = with_certificate("vcon_cert", Login::CertificateAlone);
        assert_eq!(
            s.connect_timeout, LIVE_DEADLINE,
            "with_certificate reads the shared value"
        );
    }

    #[test]
    fn a_certificate_and_password_login_is_given_the_live_deadline() {
        let (_tmp, s) = with_certificate("vcon_cert", Login::CertificateAndSecret);
        assert_eq!(
            s.connect_timeout, LIVE_DEADLINE,
            "with_certificate reads the shared value"
        );
    }

    #[tokio::test]
    async fn a_test_database_is_reached_with_the_live_deadline() {
        let db = Db::fresh().await;
        let (_tmp, s) = db.settings("vcon_app");
        assert_eq!(
            s.connect_timeout, LIVE_DEADLINE,
            "Db::settings reads the shared value"
        );
        let connected = db
            .connect("vcon_tester", Purpose::Connect)
            .await
            .expect("the tester connects");
        drop(connected);
    }

    #[tokio::test]
    async fn the_primary_is_reached_within_the_live_deadline() {
        let (_tmp, s) = as_user("vcon_app");
        let started = Instant::now();
        let connected = connect(&s).await.expect("connects");
        assert!(
            started.elapsed() < LIVE_DEADLINE,
            "within the deadline: {:?}",
            started.elapsed()
        );
        drop(connected);
    }

    #[tokio::test]
    async fn a_pool_of_the_live_settings_waits_the_live_deadline_for_a_free_connection() {
        let (_tmp, s) = as_user("vcon_app");
        let pool = pool(&s);
        let lease = pool.get().await.expect("a connection");
        drop(lease);
        assert_eq!(
            s.connect_timeout, LIVE_DEADLINE,
            "the pool's wait is the deadline"
        );
    }
}
