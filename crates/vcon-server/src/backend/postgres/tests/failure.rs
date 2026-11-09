//! What must be refused, and how the refusal reads.

use super::super::connect::tls_config;
use super::super::secrets::Secrets;
use super::{closed_port, connect, file, no_tls_server, pki, settings, why};

/// Settings with a CA and a password file in `dir`, the password `text`
/// written with `mode`.
fn with_password(dir: &std::path::Path, text: &[u8], mode: u32) -> crate::config::PostgresSettings {
    let ca = file(dir, "ca.pem", pki().ca.as_bytes(), 0o644);
    let password = file(dir, "pg.password", text, mode);
    settings("db1.example.com", 5432, "vcon_app", &ca, Some(&password))
}

/// The refusal `Secrets::read` gives `s`.
fn unread(s: &crate::config::PostgresSettings) -> String {
    Secrets::read(s).expect_err("refused").0
}

#[test]
fn a_missing_password_file_is_named() {
    let dir = tempfile::tempdir().expect("dir");
    let mut s = with_password(dir.path(), b"p\n", 0o600);
    s.password_file = Some(dir.path().join("absent.password"));
    let message = unread(&s);
    assert!(
        message.contains("catalog.postgres.password_file") && message.contains("absent.password"),
        "names the key and the file: {message}"
    );
}

#[test]
fn a_password_file_other_users_can_read_is_refused_without_its_text() {
    let dir = tempfile::tempdir().expect("dir");
    for mode in [0o644, 0o604, 0o601, 0o602] {
        let s = with_password(dir.path(), b"s3cret-example\n", mode);
        let message = unread(&s);
        assert!(
            message.contains("catalog.postgres.password_file")
                && message.contains(&format!("{mode:04o}"))
                && message.contains("0600"),
            "{mode:o}: names the key, the mode and the fix: {message}"
        );
        assert!(!message.contains("s3cret"), "never the secret: {message}");
    }
}

#[test]
fn a_client_key_other_users_can_read_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let made = pki();
    let mut s = with_password(dir.path(), b"p\n", 0o600);
    s.client_cert_file = Some(file(dir.path(), "c.pem", made.cert.as_bytes(), 0o644));
    s.client_key_file = Some(file(dir.path(), "c.key", made.key.as_bytes(), 0o644));
    let message = unread(&s);
    assert!(
        message.contains("catalog.postgres.client_key_file") && message.contains("0644"),
        "names the key and the mode: {message}"
    );
}

#[test]
fn a_secret_that_is_a_directory_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let mut s = with_password(dir.path(), b"p\n", 0o600);
    let sub = dir.path().join("secrets");
    std::fs::create_dir(&sub).expect("dir");
    std::fs::set_permissions(&sub, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .expect("mode");
    s.password_file = Some(sub);
    let message = unread(&s);
    assert!(message.contains("not a regular file"), "{message}");
}

#[test]
fn an_empty_or_two_line_password_file_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    for (text, why) in [
        (&b""[..], "the file is empty"),
        (b"\n", "the file is empty"),
        (b"one\ntwo\n", "more than one line"),
    ] {
        let message = unread(&with_password(dir.path(), text, 0o600));
        assert!(
            message.contains("catalog.postgres.password_file") && message.contains(why),
            "{text:?}: {message}"
        );
    }
}

#[test]
fn a_ca_file_without_a_certificate_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let mut s = with_password(dir.path(), b"p\n", 0o600);
    s.ca_file = Some(file(dir.path(), "empty.pem", b"", 0o644));
    let message = unread(&s);
    assert!(
        message.contains("catalog.postgres.ca_file") && message.contains("no certificate"),
        "{message}"
    );
}

#[test]
fn a_client_key_that_does_not_belong_to_its_certificate_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let (one, other) = (pki(), pki());
    let mut s = with_password(dir.path(), b"p\n", 0o600);
    s.client_cert_file = Some(file(dir.path(), "c.pem", one.cert.as_bytes(), 0o644));
    s.client_key_file = Some(file(dir.path(), "c.key", other.key.as_bytes(), 0o600));
    let secrets = Secrets::read(&s).expect("both files read");
    let message = tls_config(&secrets).expect_err("refused").0;
    assert!(
        message.contains("catalog.postgres.client_key_file")
            && message.contains("catalog.postgres.client_cert_file"),
        "names both keys: {message}"
    );
}

#[test]
fn a_client_certificate_file_without_a_certificate_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let made = pki();
    let mut s = with_password(dir.path(), b"p\n", 0o600);
    s.client_cert_file = Some(file(dir.path(), "c.pem", made.key.as_bytes(), 0o644));
    s.client_key_file = Some(file(dir.path(), "c.key", made.key.as_bytes(), 0o600));
    let message = unread(&s);
    assert!(
        message.contains("catalog.postgres.client_cert_file") && message.contains("no certificate"),
        "{message}"
    );
}

#[tokio::test]
async fn a_refused_connection_names_its_host_and_port() {
    let dir = tempfile::tempdir().expect("dir");
    let port = closed_port("127.0.0.1").await;
    let mut s = with_password(dir.path(), b"p\n", 0o600);
    s.hosts = vec!["127.0.0.1".to_owned()];
    s.port = port;
    let refused = connect(&s).await.expect_err("refused");
    let message = refused.to_string();
    assert!(
        message.contains(&format!("127.0.0.1:{port}")) && message.contains("error connecting"),
        "names where and what: {message}"
    );
}

#[tokio::test]
async fn a_server_that_does_not_offer_tls_is_refused() {
    let dir = tempfile::tempdir().expect("dir");
    let addr = no_tls_server().await;
    let mut s = with_password(dir.path(), b"p\n", 0o600);
    s.hosts = vec!["127.0.0.1".to_owned()];
    s.port = addr.port();
    let message = why(connect(&s).await.expect_err("refused"));
    assert!(
        message.contains("server does not support TLS") && message.contains("ssl on"),
        "says so and what to change: {message}"
    );
}

/// Against the containers of `scripts/pg-test-server.sh`.
#[cfg(feature = "backend-tests")]
mod live {
    use super::super::super::connect::Target;
    use super::super::live::{self, Login, as_user, value, with_certificate};
    use super::super::{connector_to, pool};
    use super::*;

    /// The refusal a connection with `s` gets from the one host it names.
    async fn refused(s: &crate::config::PostgresSettings) -> String {
        let message = why(connect(s).await.expect_err("refused"));
        assert!(
            !message.contains(&live::password()),
            "never the password: {message}"
        );
        message
    }

    #[tokio::test]
    async fn a_server_certificate_from_another_authority_is_refused() {
        let (_tmp, mut s) = as_user("vcon_app");
        s.ca_file = Some(live::dir().join("wrong-ca.pem"));
        let message = refused(&s).await;
        assert!(
            message.contains("not signed by an authority in catalog.postgres.ca_file"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_server_certificate_for_another_host_name_is_refused() {
        let (_tmp, s) = as_user("vcon_app");
        let wrong_name = Target {
            host: "db.example.com".to_owned(),
            addr: Some([127, 0, 0, 1].into()),
            port: live::primary(),
        };
        let refused = connector_to(vec![wrong_name], &s)
            .connect()
            .await
            .expect_err("refused");
        let message = why(refused);
        assert!(
            message.contains("does not hold the host name"),
            "reached at the right address under a name its certificate lacks: {message}"
        );
    }

    #[tokio::test]
    async fn a_real_server_with_ssl_off_is_refused() {
        let (_tmp, mut s) = as_user("vcon_app");
        s.port = live::plain();
        let message = refused(&s).await;
        assert!(message.contains("server does not support TLS"), "{message}");
    }

    /// tokio-postgres checks channel binding before it answers an md5
    /// request, so the password, which is right, is never sent.
    #[tokio::test]
    async fn an_md5_login_is_refused_before_the_password_is_sent() {
        let (_tmp, s) = as_user("vcon_md5");
        let message = refused(&s).await;
        assert!(
            message.contains("server did not use channel binding") && message.contains("md5"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_plain_password_login_is_refused_before_the_password_is_sent() {
        let (_tmp, s) = as_user("vcon_plain");
        let message = refused(&s).await;
        assert!(
            message.contains("server did not use channel binding"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_trust_login_is_refused_when_a_password_is_set() {
        let (_tmp, s) = as_user("vcon_trust");
        let message = refused(&s).await;
        assert!(
            message.contains("server did not use channel binding"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_superuser_is_refused() {
        let (_tmp, s) = as_user("vcon_super");
        let message = refused(&s).await;
        assert!(
            message.contains("\"vcon_super\" is a superuser"),
            "names the role and why: {message}"
        );
    }

    #[tokio::test]
    async fn a_role_with_bypassrls_is_refused() {
        let (_tmp, s) = as_user("vcon_bypass");
        let message = refused(&s).await;
        assert!(
            message.contains("\"vcon_bypass\" has BYPASSRLS"),
            "names the role and why: {message}"
        );
    }

    #[tokio::test]
    async fn a_server_that_takes_only_read_only_transactions_is_refused() {
        let (_tmp, s) = as_user("vcon_ro");
        let message = refused(&s).await;
        assert!(message.contains("transaction_read_only is on"), "{message}");
    }

    #[tokio::test]
    async fn a_standby_alone_is_refused() {
        let (_tmp, mut s) = as_user("vcon_app");
        s.port = live::standby();
        let message = refused(&s).await;
        assert!(
            message.contains("standby") && message.contains("pg_is_in_recovery"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_wrong_password_is_refused_without_showing_it() {
        let (tmp, s) = as_user("vcon_app");
        file(tmp.path(), "pg.password", b"not-the-password-5f3a\n", 0o600);
        let message = refused(&s).await;
        assert!(
            message.contains("SQLSTATE 28P01") && !message.contains("not-the-password-5f3a"),
            "the server's refusal, without the password: {message}"
        );
    }

    #[tokio::test]
    async fn a_client_certificate_from_another_authority_is_refused_by_the_server() {
        let (tmp, mut s) = with_certificate("vcon_cert", Login::CertificateAlone);
        let key = std::fs::read(live::dir().join("stranger.key")).expect("the key");
        s.client_key_file = Some(file(tmp.path(), "client.key", &key, 0o600));
        s.client_cert_file = Some(live::dir().join("stranger.crt"));
        let message = refused(&s).await;
        assert!(
            message.contains("UnknownCA")
                && message.contains("the server refused the client certificate")
                && message.contains("catalog.postgres.client_cert_file"),
            "the server's TLS layer refused the certificate, and the message says which: {message}"
        );
    }

    #[tokio::test]
    async fn a_server_that_asks_for_a_password_none_was_given_for_is_refused() {
        let (_tmp, mut s) = with_certificate("vcon_cert", Login::CertificateAlone);
        s.user = "vcon_app".to_owned();
        let message = refused(&s).await;
        assert!(
            message.contains("asked for a password") && message.contains("password_file"),
            "{message}"
        );
    }

    /// A primary that turns standby answers 25006 to a write; the
    /// connection must not go to the next caller.
    #[tokio::test]
    async fn a_read_only_answer_drops_the_connection_from_the_pool() {
        let (_tmp, s) = as_user("vcon_app");
        let pool = pool(&s);
        let mut lease = pool.get().await.expect("a connection");
        let pid = lease.server().pid;
        let _ro = value(
            lease.client(),
            "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY; SELECT 1",
        )
        .await;
        let e = lease
            .client()
            .simple_query("CREATE TEMP TABLE vcon_written (x int)")
            .await
            .expect_err("refused");
        assert_eq!(
            e.code(),
            Some(&tokio_postgres::error::SqlState::READ_ONLY_SQL_TRANSACTION),
            "25006: {e}"
        );
        lease.failed(&e);
        drop(lease);
        assert_eq!(pool.idle(), 0, "not kept");
        let next = pool.get().await.expect("a connection");
        assert_ne!(next.server().pid, pid, "a new connection");
    }
}
