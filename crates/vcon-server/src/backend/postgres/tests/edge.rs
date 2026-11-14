//! The limits, and inputs at the edge of what is accepted.

use std::time::{Duration, Instant};

use super::super::connect::{Target, session_options};
use super::super::secrets::Secrets;
use super::{closed_port, connect, connector_to, file, pki, settings, silent_server, why};

#[test]
fn a_secret_file_its_group_may_read_is_accepted() {
    let dir = tempfile::tempdir().expect("dir");
    let ca = file(dir.path(), "ca.pem", pki().ca.as_bytes(), 0o644);
    let password = file(dir.path(), "pg.password", b"p\n", 0o640);
    let s = settings("db1.example.com", 5432, "vcon_app", &ca, Some(&password));
    assert!(
        Secrets::read(&s).is_ok(),
        "0640, as for a file owned by root and readable by the service's group"
    );
}

#[test]
fn a_unicode_password_is_read_as_written() {
    let dir = tempfile::tempdir().expect("dir");
    let ca = file(dir.path(), "ca.pem", pki().ca.as_bytes(), 0o644);
    let password = file(
        dir.path(),
        "pg.password",
        "pässwörd-数据\r\n".as_bytes(),
        0o600,
    );
    let s = settings("db1.example.com", 5432, "vcon_app", &ca, Some(&password));
    let secrets = Secrets::read(&s).expect("reads");
    assert_eq!(
        secrets.password.as_deref().map(String::as_str),
        Some("pässwörd-数据"),
        "UTF-8, a CRLF line ending left out"
    );
}

#[test]
fn a_password_file_that_is_not_utf8_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let ca = file(dir.path(), "ca.pem", pki().ca.as_bytes(), 0o644);
    let password = file(dir.path(), "pg.password", b"\xff\xfe\n", 0o600);
    let s = settings("db1.example.com", 5432, "vcon_app", &ca, Some(&password));
    let message = Secrets::read(&s).expect_err("refused").0;
    assert!(message.contains("not UTF-8"), "{message}");
}

/// PostgreSQL holds both timeouts as milliseconds in a 32-bit integer.
#[test]
fn the_longest_timeouts_fit_postgresql_in_milliseconds() {
    let longest = Duration::from_secs(crate::config::POSTGRES_TIMEOUT_LIMIT);
    let options = session_options(longest, longest);
    assert_eq!(
        options, "-c statement_timeout=2147483000 -c lock_timeout=2147483000",
        "the limit in milliseconds"
    );
    assert!(
        2_147_483_000_u64 <= u64::try_from(i32::MAX).expect("positive"),
        "within PostgreSQL's integer"
    );
}

#[tokio::test]
async fn a_server_that_never_answers_is_given_up_at_the_connect_timeout() {
    let dir = tempfile::tempdir().expect("dir");
    let ca = file(dir.path(), "ca.pem", pki().ca.as_bytes(), 0o644);
    let password = file(dir.path(), "pg.password", b"p\n", 0o600);
    let addr = silent_server().await;
    let mut s = settings("127.0.0.1", addr.port(), "vcon_app", &ca, Some(&password));
    s.connect_timeout = Duration::from_secs(1);
    let started = Instant::now();
    let message = why(connect(&s).await.expect_err("refused"));
    let took = started.elapsed();
    assert!(
        message.contains("catalog.postgres.connect_timeout (1 s)"),
        "{message}"
    );
    assert!(
        took >= Duration::from_secs(1) && took < Duration::from_secs(3),
        "given up at the timeout: {took:?}"
    );
}

#[tokio::test]
async fn a_refused_connection_on_the_ipv6_loopback_is_named_in_brackets() {
    let dir = tempfile::tempdir().expect("dir");
    let ca = file(dir.path(), "ca.pem", pki().ca.as_bytes(), 0o644);
    let password = file(dir.path(), "pg.password", b"p\n", 0o600);
    let port = closed_port("::1").await;
    let s = settings("::1", port, "vcon_app", &ca, Some(&password));
    let refused = connect(&s).await.expect_err("refused");
    assert_eq!(
        refused.0[0].0,
        format!("[::1]:{port}"),
        "the host as an address, in brackets"
    );
}

#[tokio::test]
async fn each_host_is_tried_in_order_and_each_refusal_named() {
    let dir = tempfile::tempdir().expect("dir");
    let ca = file(dir.path(), "ca.pem", pki().ca.as_bytes(), 0o644);
    let password = file(dir.path(), "pg.password", b"p\n", 0o600);
    let s = settings("127.0.0.1", 5432, "vcon_app", &ca, Some(&password));
    let (four, six) = (closed_port("127.0.0.1").await, closed_port("::1").await);
    let targets = vec![
        Target {
            host: "127.0.0.1".to_owned(),
            addr: None,
            port: four,
        },
        Target {
            host: "::1".to_owned(),
            addr: None,
            port: six,
        },
    ];
    let refused = connector_to(targets, &s)
        .connect()
        .await
        .expect_err("refused");
    let names: Vec<&str> = refused.0.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        names,
        [format!("127.0.0.1:{four}"), format!("[::1]:{six}")],
        "both, in order"
    );
}

/// Against the containers of `scripts/pg-test-server.sh`.
#[cfg(feature = "backend-tests")]
mod live {
    use super::super::live::{self, as_user, value, within_five_seconds};
    use super::super::pool;
    use super::*;

    #[tokio::test]
    async fn a_connection_over_ipv6_is_made() {
        let (_tmp, mut s) = as_user("vcon_app");
        s.hosts = vec!["::1".to_owned()];
        s.port = live::primary6();
        let c = connect(&s).await.expect("connects over ::1");
        // Docker's port proxy carries [::1] to the container over IPv4, so
        // the server cannot see the family; the client side shows it: the
        // address connected to, and the certificate checked against it.
        assert_eq!(
            c.server.target,
            format!("[::1]:{}", live::primary6()),
            "connected to the IPv6 loopback, whose address the certificate holds"
        );
        assert_eq!(
            value(&c.client, "SELECT 'answered'").await,
            "answered",
            "and the connection answers"
        );
    }

    #[tokio::test]
    async fn unicode_role_and_database_names_log_in() {
        let (_tmp, mut s) = as_user("vcon_ünï");
        s.dbname = "vcon_数据".to_owned();
        let c = connect(&s).await.expect("connects");
        assert_eq!(
            value(
                &c.client,
                "SELECT current_user || ' ' || current_database()"
            )
            .await,
            "vcon_ünï vcon_数据",
            "the names as written"
        );
    }

    #[tokio::test]
    async fn a_caller_is_refused_when_every_connection_stays_in_use() {
        let (_tmp, mut s) = as_user("vcon_app");
        s.pool = 1;
        s.connect_timeout = Duration::from_secs(1);
        let pool = pool(&s);
        let _held = pool.get().await.expect("the one connection");
        let started = Instant::now();
        let refused = pool.get().await.expect_err("none free");
        let took = started.elapsed();
        assert!(
            refused
                .to_string()
                .contains("all 1 connections (catalog.postgres.pool)"),
            "{refused}"
        );
        assert!(
            took >= Duration::from_secs(1) && took < Duration::from_secs(3),
            "it waited connect_timeout: {took:?}"
        );
    }

    #[tokio::test]
    async fn a_connection_returned_within_the_wait_goes_to_the_waiter() {
        let (_tmp, mut s) = as_user("vcon_app");
        s.pool = 1;
        s.connect_timeout = Duration::from_secs(5);
        let pool = pool(&s);
        let held = pool.get().await.expect("the one connection");
        let pid = held.server().pid;
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            drop(held);
        });
        let next = pool.get().await.expect("the returned connection");
        assert_eq!(next.server().pid, pid, "the same connection, not a second");
    }

    #[tokio::test]
    async fn the_longest_timeouts_are_taken_by_the_server() {
        let (_tmp, mut s) = as_user("vcon_app");
        let longest = Duration::from_secs(crate::config::POSTGRES_TIMEOUT_LIMIT);
        s.statement_timeout = longest;
        s.lock_timeout = longest;
        let c = connect(&s).await.expect("connects");
        for setting in ["statement_timeout", "lock_timeout"] {
            assert_eq!(
                value(
                    &c.client,
                    &format!("SELECT setting FROM pg_settings WHERE name = '{setting}'")
                )
                .await,
                "2147483000",
                "{setting} in milliseconds"
            );
        }
    }

    #[tokio::test]
    async fn a_statement_past_the_statement_timeout_is_cancelled() {
        let (_tmp, mut s) = as_user("vcon_app");
        s.statement_timeout = Duration::from_secs(1);
        let c = connect(&s).await.expect("connects");
        let e = c
            .client
            .simple_query("SELECT pg_sleep(3)")
            .await
            .expect_err("cancelled");
        assert_eq!(
            e.code(),
            Some(&tokio_postgres::error::SqlState::QUERY_CANCELED),
            "57014: {e}"
        );
        assert_eq!(
            value(&c.client, "SELECT 'still here'").await,
            "still here",
            "the connection stays usable"
        );
    }

    #[tokio::test]
    async fn a_connection_the_server_closed_is_not_handed_out_again() {
        let (_tmp, s) = as_user("vcon_app");
        let pool = pool(&s);
        let lease = pool.get().await.expect("a connection");
        let pid = lease.server().pid;
        let client_closed = {
            let other = connect(&s).await.expect("a second connection");
            drop(lease);
            assert_eq!(pool.idle(), 1, "kept");
            value(
                &other.client,
                &format!("SELECT pg_terminate_backend({pid})::text"),
            )
            .await
        };
        assert_eq!(
            client_closed, "true",
            "the server ended the kept connection"
        );
        assert!(
            within_five_seconds(|| pool.idle_closed() == 1).await,
            "the kept connection sees its socket closed"
        );
        let next = pool.get().await.expect("a connection");
        assert_ne!(
            next.server().pid,
            pid,
            "a new connection, not the closed one"
        );
        assert_eq!(
            value(next.client(), "SELECT 'open'").await,
            "open",
            "and it answers"
        );
        drop(next);
        assert_eq!(
            pool.idle(),
            1,
            "the closed one was let go, the new one kept"
        );
    }
}

/// A configuration with the bucket and the two catalog tables the
/// PostgreSQL guide leaves, `catalog` standing for the `[catalog]` part.
fn configuration(catalog: &str) -> String {
    format!(
        "listen = \"0.0.0.0:8443\"\ndata_dir = \"/var/lib/vcon-server\"\n\
         master_key = \"/etc/vcon-server/master.key\"\n\n[tls]\ncert = \"c.pem\"\nkey = \"k.pem\"\n\n\
         [storage]\nobjects = \"s3\"\n\n[storage.s3]\nendpoint = \"https://objects.example.com:9000\"\n\
         region = \"us-east-1\"\nbucket = \"vcon\"\naccess_key_id = \"k\"\n\
         secret_access_key_file = \"/etc/vcon-server/s3.secret\"\n{catalog}"
    )
}

const POSTGRES_TABLE: &str = "\n[catalog.postgres]\nhosts = [\"db1.example.com\"]\ndbname = \"vcon\"\n\
                              user = \"vcon_app\"\npassword_file = \"/etc/vcon-server/pg.password\"\n";

/// The guide's way back to the local catalog is to remove both tables.
/// Found on 2026-11-10, running the PostgreSQL guide: a message said to
/// set `catalog.backend = "local"`, and a configuration with that and the
/// `[catalog.postgres]` table still in it is refused.
#[test]
fn the_guide_s_configuration_reads_and_so_does_one_with_both_tables_removed() {
    let with_tables = configuration(&format!(
        "\n[catalog]\nbackend = \"postgres\"\n{POSTGRES_TABLE}"
    ));
    assert!(
        crate::config::parse(&with_tables).is_ok(),
        "the guide's configuration reads"
    );
    assert!(
        crate::config::parse(&configuration("")).is_ok(),
        "following the advice leaves a configuration that reads"
    );
}

/// Why the advice is to remove both tables: `backend = "local"` beside a
/// `[catalog.postgres]` table is refused, so that a table left behind is
/// never silently unused.
#[test]
fn a_local_backend_beside_a_postgres_table_is_refused() {
    let left = configuration(&format!(
        "\n[catalog]\nbackend = \"local\"\n{POSTGRES_TABLE}"
    ));
    let refused = crate::config::parse(&left).err().map(|e| e.to_string());
    assert_eq!(
        refused.as_deref(),
        Some("catalog.postgres is set but catalog.backend is not \"postgres\""),
        "the change the old message advised"
    );
    let alone = configuration("\n[catalog]\nbackend = \"local\"\n");
    assert!(
        crate::config::parse(&alone).is_ok(),
        "the [catalog] table alone, set to local, reads"
    );
}
