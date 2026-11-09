//! What must work.

use std::time::Duration;

use super::super::connect::{Target, session_options, tls_config};
use super::super::secrets::Secrets;
use super::{file, pki, settings};

#[test]
fn a_password_file_is_read_once_without_its_line_ending() {
    let dir = tempfile::tempdir().expect("dir");
    let made = pki();
    let ca = file(dir.path(), "ca.pem", made.ca.as_bytes(), 0o644);
    let password = file(dir.path(), "pg.password", b"s3cret-example\n", 0o600);
    let s = settings("db1.example.com", 5432, "vcon_app", &ca, Some(&password));
    let secrets = Secrets::read(&s).expect("reads");
    assert_eq!(
        secrets.password.as_deref().map(String::as_str),
        Some("s3cret-example"),
        "one line, its line ending left out"
    );
    std::fs::remove_file(&password).expect("removes");
    assert_eq!(
        secrets.password.as_deref().map(String::as_str),
        Some("s3cret-example"),
        "held in memory once read: the file is not needed again"
    );
}

#[test]
fn a_ca_file_and_a_client_certificate_make_the_tls_settings() {
    let dir = tempfile::tempdir().expect("dir");
    let made = pki();
    let mut s = settings(
        "db1.example.com",
        5432,
        "vcon_cert",
        &file(dir.path(), "ca.pem", made.ca.as_bytes(), 0o644),
        None,
    );
    s.client_cert_file = Some(file(dir.path(), "c.pem", made.cert.as_bytes(), 0o644));
    s.client_key_file = Some(file(dir.path(), "c.key", made.key.as_bytes(), 0o600));
    let secrets = Secrets::read(&s).expect("reads");
    assert_eq!(secrets.roots.len(), 1, "the CA is the one trust anchor");
    let (chain, _key) = secrets.client.as_ref().expect("a client certificate");
    assert_eq!(chain.len(), 1, "the client certificate");
    let config = tls_config(&secrets).expect("the TLS settings");
    assert!(
        config.client_auth_cert_resolver.has_certs(),
        "the client certificate is offered when the server asks"
    );
}

#[test]
fn the_system_authorities_are_trusted_without_a_ca_file() {
    let dir = tempfile::tempdir().expect("dir");
    let password = file(dir.path(), "pg.password", b"p\n", 0o600);
    let mut s = settings(
        "db1.example.com",
        5432,
        "vcon_app",
        dir.path(),
        Some(&password),
    );
    s.ca_file = None;
    let secrets = Secrets::read(&s).expect("reads the system's authorities");
    assert!(
        secrets.roots.len() > 1,
        "the system's authorities, not one file: {}",
        secrets.roots.len()
    );
}

#[test]
fn the_session_options_set_both_timeouts_in_milliseconds() {
    assert_eq!(
        session_options(Duration::from_secs(30), Duration::from_secs(10)),
        "-c statement_timeout=30000 -c lock_timeout=10000",
        "PostgreSQL reads both in milliseconds"
    );
}

#[test]
fn a_target_names_its_host_and_port_and_brackets_an_ipv6_address() {
    let named = |host: &str| {
        Target {
            host: host.to_owned(),
            addr: None,
            port: 5432,
        }
        .to_string()
    };
    assert_eq!(named("db1.example.com"), "db1.example.com:5432", "a name");
    assert_eq!(named("192.0.2.10"), "192.0.2.10:5432", "an IPv4 address");
    assert_eq!(
        named("2001:db8::10"),
        "[2001:db8::10]:5432",
        "an IPv6 address"
    );
}

/// Against the containers of `scripts/pg-test-server.sh`.
#[cfg(feature = "backend-tests")]
mod live {
    use super::super::live::{self, Login, as_user, value, with_certificate};
    use super::super::{connect, connector_to, pool};
    use super::*;

    #[tokio::test]
    async fn a_tls_connection_logs_in_with_scram_and_channel_binding() {
        let (_tmp, s) = as_user("vcon_app");
        let c = connect(&s).await.expect("connects");
        assert!(
            c.server.version.starts_with(&format!("{}.", live::major())),
            "the server's version: {}",
            c.server.version
        );
        assert_eq!(c.server.user, "vcon_app", "the role logged in as");
        assert_eq!(
            value(
                &c.client,
                "SELECT ssl::text FROM pg_stat_ssl WHERE pid = pg_backend_pid()"
            )
            .await,
            "true",
            "the server sees TLS on this connection"
        );
        assert_eq!(
            value(
                &c.client,
                "SELECT application_name FROM pg_stat_activity WHERE pid = pg_backend_pid()"
            )
            .await,
            "vcon-server",
            "the server's activity view names the program"
        );
    }

    #[tokio::test]
    async fn a_client_certificate_alone_logs_in() {
        let (_tmp, s) = with_certificate("vcon_cert", Login::CertificateAlone);
        let c = connect(&s).await.expect("connects with the certificate");
        assert_eq!(c.server.user, "vcon_cert", "pg_hba.conf's cert line");
        assert_eq!(
            value(
                &c.client,
                "SELECT client_dn FROM pg_stat_ssl WHERE pid = pg_backend_pid()"
            )
            .await,
            "/CN=vcon_cert",
            "the server read the client certificate"
        );
    }

    #[tokio::test]
    async fn a_client_certificate_and_a_password_log_in_together() {
        let (_tmp, s) = with_certificate("vcon_both", Login::CertificateAndSecret);
        let c = connect(&s).await.expect("connects");
        assert_eq!(
            c.server.user, "vcon_both",
            "scram-sha-256 with clientcert=verify-full"
        );
    }

    #[tokio::test]
    async fn the_pool_hands_the_same_connection_to_the_next_caller() {
        let (_tmp, s) = as_user("vcon_app");
        let pool = pool(&s);
        let first = pool.get().await.expect("a connection");
        let pid = first.server().pid;
        assert_eq!(pool.idle(), 0, "none waits while it is leased");
        drop(first);
        assert_eq!(pool.idle(), 1, "it went back to the pool");
        let second = pool.get().await.expect("a connection");
        assert_eq!(second.server().pid, pid, "the same server process: reused");
        assert_eq!(
            value(second.client(), "SELECT pg_backend_pid()::text").await,
            pid.to_string(),
            "and it still answers"
        );
    }

    #[tokio::test]
    async fn the_pool_opens_another_connection_while_one_is_leased() {
        let (_tmp, s) = as_user("vcon_app");
        let pool = pool(&s);
        let first = pool.get().await.expect("a connection");
        let second = pool.get().await.expect("another");
        assert_ne!(
            first.server().pid,
            second.server().pid,
            "two callers, two connections"
        );
        drop((first, second));
        assert_eq!(pool.idle(), 2, "both kept");
    }

    /// The catalog's traits are synchronous and run on `spawn_blocking`
    /// threads; there, `Handle::block_on` runs the pool's async calls.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_lease_on_a_blocking_thread_runs_its_query_through_the_runtime_handle() {
        let (_tmp, s) = as_user("vcon_app");
        let pool = pool(&s);
        let handle = tokio::runtime::Handle::current();
        let (pid, answered) = tokio::task::spawn_blocking(move || {
            let lease = pool.get_blocking(&handle).expect("a lease");
            let answered = handle.block_on(value(lease.client(), "SELECT pg_backend_pid()::text"));
            (lease.server().pid, answered)
        })
        .await
        .expect("the blocking thread ends");
        assert_eq!(
            answered,
            pid.to_string(),
            "the query ran on the leased connection"
        );
    }

    #[tokio::test]
    async fn the_pool_reconnects_from_memory_once_the_secret_files_are_gone() {
        let (tmp, s) = with_certificate("vcon_both", Login::CertificateAndSecret);
        let pool = pool(&s);
        let first = pool.get().await.expect("a connection");
        std::fs::remove_file(tmp.path().join("pg.password")).expect("removes the password");
        std::fs::remove_file(tmp.path().join("client.key")).expect("removes the key");
        let second = pool
            .get()
            .await
            .expect("a new connection without the files");
        assert_ne!(
            first.server().pid,
            second.server().pid,
            "a second connection was made"
        );
    }

    #[tokio::test]
    async fn a_standby_named_first_is_passed_over_for_the_primary() {
        let (_tmp, s) = as_user("vcon_app");
        let target = |port| Target {
            host: "localhost".to_owned(),
            addr: None,
            port,
        };
        let c = connector_to(vec![target(live::standby()), target(live::primary())], &s)
            .connect()
            .await
            .expect("the primary takes it");
        assert_eq!(
            c.server.target,
            format!("localhost:{}", live::primary()),
            "the standby was skipped"
        );
    }

    #[tokio::test]
    async fn the_preview_names_the_server_it_reached_and_refuses_to_go_on() {
        let db = super::super::schema_tests::live::Db::migrated().await;
        let (_tmp, s) = db.settings("vcon_app");
        let refused = super::super::super::preview(&s).await;
        assert!(
            refused
                .0
                .contains(&format!("connected to PostgreSQL {}.", live::major()))
                && refused.0.contains("as vcon_app over TLS")
                && refused.0.contains("preview")
                && refused.0.contains("catalog.backend = \"local\""),
            "names what it reached and what to do: {refused}"
        );
        assert!(!refused.0.contains(&live::password()), "never the password");
    }
}
