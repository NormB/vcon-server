//! The live-test deadline of tests/support/live_deadline.rs, and the
//! helpers that use it, without a live server: the configuration tables
//! the live integration tests write for the binary, read back by the
//! server's own parser, and the PostgreSQL helper of tests/pg/mod.rs
//! against small servers on the loopback address that never answer, hang
//! up, or refuse TLS.
//!
//! By kind: what must work (`success`), what must be refused (`failure`),
//! and the limits and odd inputs (`edge`).

#[path = "support/live_deadline.rs"]
mod live_deadline;
#[cfg(feature = "postgres")]
#[allow(
    dead_code,
    reason = "this crate drives only pg::Server::query; the live crates use the rest"
)]
mod pg;

use std::path::{Path, PathBuf};
use std::time::Duration;

use live_deadline::{LIVE_DEADLINE, missed, postgres_tables, s3_tables, whole_seconds};
use vcon_server::config::{
    CatalogStore, Config, ObjectStore, POSTGRES_CONNECT_TIMEOUT, PostgresSettings, S3_TIMEOUT,
    S3Settings,
};

/// The least a server configuration holds, before its storage tables.
const LEAST: &str = "listen = \"127.0.0.1:0\"\ndata_dir = \"/srv/vcon\"\nmaster_key = \
                     \"/etc/vcon-server/master.key\"\n\n[tls]\ncert = \
                     \"/etc/vcon-server/cert.pem\"\nkey = \"/etc/vcon-server/key.pem\"\n";

/// The configuration a live integration test writes: `LEAST`, the S3
/// tables and the PostgreSQL tables, both with `deadline`, read by the
/// server's own parser.
fn written(deadline: Duration) -> Config {
    written_with("/tmp/ca.pem", "/tmp/pg.password", deadline)
}

/// `written`, with the CA at `ca_file` and the password at
/// `password_file`.
fn written_with(ca_file: &str, password_file: &str, deadline: Duration) -> Config {
    let text = format!(
        "{LEAST}{}{}",
        s3_tables("http://127.0.0.1:9000", "vcon-test", deadline),
        postgres_tables(
            "55432",
            "vcon_t_1_ab",
            "vcon_app",
            Path::new(ca_file),
            Path::new(password_file),
            deadline
        )
    );
    vcon_server::config::parse(&text).unwrap_or_else(|e| panic!("{text}\nis refused: {e}"))
}

/// The `[catalog.postgres]` settings of `config`.
fn postgres(config: &Config) -> &PostgresSettings {
    match &config.catalog {
        CatalogStore::Postgres(settings) => settings,
        CatalogStore::Local => panic!("the catalogs are in PostgreSQL: {config:?}"),
    }
}

/// The `[storage.s3]` settings of `config`.
fn s3(config: &Config) -> &S3Settings {
    match &config.storage.objects {
        ObjectStore::S3(settings) => settings,
        ObjectStore::Local => panic!("the objects are in S3: {config:?}"),
    }
}

/// The test runner's deadline for one test, in seconds, from
/// .config/nextest.toml: `terminate-after` periods of `period`.
fn runner_deadline() -> u64 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.config/nextest.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let line = text
        .lines()
        .find(|l| l.starts_with("slow-timeout = "))
        .expect("the runner's deadline is set");
    let number_after = |key: &str| -> u64 {
        let rest = line.split(key).nth(1).expect("the key is there");
        rest.trim_start_matches([' ', '=', '"'])
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or_else(|e| panic!("{key} in {line}: {e}"))
    };
    number_after("period") * number_after("terminate-after")
}

mod success {
    use super::*;

    #[test]
    fn a_whole_number_of_seconds_is_kept_as_it_is() {
        assert_eq!(whole_seconds(Duration::from_secs(30)), 30, "30 s is 30");
        assert_eq!(whole_seconds(Duration::from_secs(7)), 7, "7 s is 7");
    }

    #[test]
    fn the_postgres_tables_give_the_server_the_live_deadline() {
        assert_eq!(
            postgres(&written(LIVE_DEADLINE)).connect_timeout,
            LIVE_DEADLINE,
            "the binary waits the live deadline for each connection"
        );
    }

    #[test]
    fn the_postgres_tables_give_the_server_any_deadline_asked_for() {
        assert_eq!(
            postgres(&written(Duration::from_secs(7))).connect_timeout,
            Duration::from_secs(7),
            "the deadline is the one passed, not a fixed value"
        );
    }

    #[test]
    fn the_postgres_tables_name_the_primary_the_database_and_the_role() {
        let config = written(LIVE_DEADLINE);
        let pg = postgres(&config);
        assert_eq!(pg.hosts, vec!["localhost".to_owned()], "the primary's host");
        assert_eq!(pg.port, 55432, "the primary's port");
        assert_eq!(pg.dbname, "vcon_t_1_ab", "the test's database");
        assert_eq!(pg.user, "vcon_app", "the login role");
    }

    #[test]
    fn the_postgres_tables_name_the_ca_and_the_password_file() {
        let config = written(LIVE_DEADLINE);
        let pg = postgres(&config);
        assert_eq!(pg.ca_file, Some(PathBuf::from("/tmp/ca.pem")), "the CA");
        assert_eq!(
            pg.password_file,
            Some(PathBuf::from("/tmp/pg.password")),
            "the password file"
        );
    }

    #[test]
    fn the_s3_tables_give_each_request_the_live_deadline() {
        assert_eq!(
            s3(&written(LIVE_DEADLINE)).timeout,
            LIVE_DEADLINE,
            "the binary waits the live deadline for each S3 request"
        );
    }

    #[test]
    fn the_s3_tables_give_each_request_any_deadline_asked_for() {
        // The server's S3 default is 30 s, the live deadline's value too:
        // only a deadline that differs from both shows the table carries
        // the one passed.
        assert_eq!(
            s3(&written(Duration::from_secs(7))).timeout,
            Duration::from_secs(7),
            "the deadline is the one passed, not the server's default"
        );
    }

    #[test]
    fn the_s3_tables_name_the_service_and_the_bucket() {
        let config = written(LIVE_DEADLINE);
        let s3 = s3(&config);
        assert_eq!(
            s3.endpoint.as_deref(),
            Some("http://127.0.0.1:9000"),
            "the endpoint"
        );
        assert_eq!(s3.bucket, "vcon-test", "the bucket");
        assert!(
            s3.path_style && s3.allow_http,
            "path-style over plain HTTP: {s3:?}"
        );
    }

    #[test]
    fn a_missed_deadline_names_who_waited_and_how_long() {
        let said = missed("vcon_tester at localhost:5432", Duration::from_secs(30));
        assert!(
            said.contains("vcon_tester at localhost:5432") && said.contains("30s"),
            "names the caller and the deadline: {said}"
        );
    }

    #[test]
    fn a_missed_deadline_names_the_shared_value() {
        let said = missed("vcon_app", LIVE_DEADLINE);
        assert!(
            said.contains("live-test deadline"),
            "says which rule it was: {said}"
        );
    }

    #[test]
    fn the_live_deadline_is_longer_than_the_servers_connect_timeout() {
        assert!(
            LIVE_DEADLINE > POSTGRES_CONNECT_TIMEOUT,
            "a loaded host gets more than the production {POSTGRES_CONNECT_TIMEOUT:?}: \
             {LIVE_DEADLINE:?}"
        );
    }

    #[test]
    fn the_live_deadline_is_no_shorter_than_the_servers_s3_timeout() {
        assert!(
            LIVE_DEADLINE >= S3_TIMEOUT,
            "a live S3 request gets at least the production {S3_TIMEOUT:?}: {LIVE_DEADLINE:?}"
        );
    }

    #[test]
    fn the_live_deadline_is_twice_the_old_integration_helper_value_or_more() {
        // tests/pg/mod.rs gave each connection 10 s before this rule.
        assert!(
            LIVE_DEADLINE >= Duration::from_secs(20),
            "at least twice the 10 s the integration helper had: {LIVE_DEADLINE:?}"
        );
    }
}

mod failure {
    use super::*;

    #[test]
    fn a_configuration_without_the_postgres_deadline_falls_back_to_the_fail_fast_default() {
        // What a live test that writes its own table gets: the reason
        // every live configuration goes through `postgres_tables`.
        let text = format!(
            "{LEAST}{}\n[catalog]\nbackend = \"postgres\"\n\n[catalog.postgres]\nhosts = \
             [\"localhost\"]\ndbname = \"vcon\"\nuser = \"vcon_app\"\npassword_file = \
             \"/tmp/pg.password\"\n",
            s3_tables("http://127.0.0.1:9000", "vcon-test", LIVE_DEADLINE)
        );
        let config = vcon_server::config::parse(&text).expect("read");
        assert_eq!(
            postgres(&config).connect_timeout,
            Duration::from_secs(5),
            "the production default is unchanged, and is not the live deadline"
        );
        assert_ne!(
            postgres(&config).connect_timeout,
            LIVE_DEADLINE,
            "a table of its own misses the live deadline"
        );
    }

    #[test]
    fn a_configuration_without_the_s3_deadline_falls_back_to_the_production_default() {
        let text = format!(
            "{LEAST}\n[storage]\nobjects = \"s3\"\n\n[storage.s3]\nendpoint = \
             \"http://127.0.0.1:9000\"\nregion = \"us-east-1\"\nbucket = \"b\"\npath_style = \
             true\nallow_http = true\n"
        );
        let config = vcon_server::config::parse(&text).expect("read");
        assert_eq!(
            s3(&config).timeout,
            Duration::from_secs(30),
            "the production default is unchanged"
        );
    }

    #[test]
    fn the_server_still_refuses_a_zero_connect_timeout_written_by_hand() {
        let text = format!(
            "{LEAST}{}\n[catalog]\nbackend = \"postgres\"\n\n[catalog.postgres]\nhosts = \
             [\"localhost\"]\ndbname = \"vcon\"\nuser = \"vcon_app\"\npassword_file = \
             \"/tmp/pg.password\"\nconnect_timeout = 0\n",
            s3_tables("http://127.0.0.1:9000", "vcon-test", LIVE_DEADLINE)
        );
        let refused = vcon_server::config::parse(&text).expect_err("zero is refused");
        assert!(
            refused.0.contains("catalog.postgres.connect_timeout"),
            "names the key: {}",
            refused.0
        );
    }

    #[cfg(feature = "postgres")]
    mod on_the_loopback {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::time::Instant;

        use super::super::pg::Server;
        use super::*;

        /// A CA's certificate, as PEM, in `dir`.
        fn ca_file(dir: &Path) -> PathBuf {
            let key = rcgen::KeyPair::generate().expect("a key");
            let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            let ca = params.self_signed(&key).expect("the CA");
            let path = dir.join("ca.pem");
            std::fs::write(&path, ca.pem()).expect("writes the CA");
            path
        }

        /// A password drawn for this test: no password is written in the
        /// source.
        fn drawn() -> String {
            let mut random = [0u8; 8];
            std::fs::File::open("/dev/urandom")
                .expect("urandom")
                .read_exact(&mut random)
                .expect("8 random bytes");
            random.iter().map(|b| format!("{b:02x}")).collect()
        }

        /// A server on 127.0.0.1 that takes one connection and runs `each`
        /// on it, with the CA file `dir` holds.
        struct Serving {
            server: Server,
            thread: std::thread::JoinHandle<()>,
        }

        impl Serving {
            fn new(dir: &Path, each: fn(TcpStream)) -> Serving {
                let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
                let port = listener.local_addr().expect("an address").port();
                let thread = std::thread::spawn(move || {
                    if let Ok((socket, _)) = listener.accept() {
                        each(socket);
                    }
                });
                Serving {
                    server: Server {
                        port,
                        ca_file: ca_file(dir),
                        password: drawn(),
                    },
                    thread,
                }
            }

            /// `server.query` as `vcon_tester` with `deadline`, how long
            /// it took, and the server's thread ended: a connection of its
            /// own wakes the server if the helper gave up before it
            /// connected, and closes at once.
            fn query(self, deadline: Duration) -> (Result<Vec<String>, String>, Duration) {
                let started = Instant::now();
                let answered = self
                    .server
                    .query("vcon_tester", "vcon", "SELECT 1", deadline);
                let took = started.elapsed();
                drop(TcpStream::connect(("127.0.0.1", self.server.port)));
                self.thread.join().expect("the server's thread ends");
                (answered, took)
            }
        }

        /// Takes the connection and never answers.
        fn silent(socket: TcpStream) {
            let mut socket = socket;
            let mut rest = Vec::new();
            let _read = socket.read_to_end(&mut rest);
        }

        /// Answers PostgreSQL's SSLRequest with `N`, as a server with ssl
        /// off does, and then waits.
        fn without_tls(socket: TcpStream) {
            let mut socket = socket;
            let mut request = [0u8; 8];
            if socket.read_exact(&mut request).is_ok() {
                let _sent = socket.write_all(b"N");
            }
            let mut rest = Vec::new();
            let _read = socket.read_to_end(&mut rest);
        }

        /// Closes each connection at once.
        fn hanging_up(socket: TcpStream) {
            drop(socket);
        }

        #[test]
        fn a_server_that_never_answers_is_given_up_at_the_deadline_by_name() {
            let dir = tempfile::tempdir().expect("dir");
            let (answered, _took) =
                Serving::new(dir.path(), silent).query(Duration::from_millis(300));
            let said = answered.expect_err("no answer");
            assert!(
                said.contains("live-test deadline") && said.contains("300ms"),
                "names the deadline it missed: {said}"
            );
        }

        #[test]
        fn a_server_that_never_answers_is_given_up_no_sooner_than_the_deadline() {
            let dir = tempfile::tempdir().expect("dir");
            let (answered, took) =
                Serving::new(dir.path(), silent).query(Duration::from_millis(300));
            assert!(answered.is_err(), "no answer: {answered:?}");
            assert!(
                took >= Duration::from_millis(300),
                "waited the whole deadline: {took:?}"
            );
        }

        #[test]
        fn a_server_that_never_answers_is_given_up_soon_after_the_deadline() {
            let dir = tempfile::tempdir().expect("dir");
            let (answered, took) =
                Serving::new(dir.path(), silent).query(Duration::from_millis(300));
            assert!(answered.is_err(), "no answer: {answered:?}");
            assert!(
                took < Duration::from_secs(5),
                "the deadline, not a fixed timeout of the helper's own, ended it: {took:?}"
            );
        }

        #[test]
        fn a_one_millisecond_deadline_is_named_as_given() {
            let dir = tempfile::tempdir().expect("dir");
            let (answered, _took) =
                Serving::new(dir.path(), silent).query(Duration::from_millis(1));
            let said = answered.expect_err("no answer");
            assert!(said.contains("(1ms"), "names the 1 ms deadline: {said}");
        }

        #[test]
        fn the_role_and_the_port_are_named_in_a_missed_deadline() {
            let dir = tempfile::tempdir().expect("dir");
            let serving = Serving::new(dir.path(), silent);
            let port = serving.server.port;
            let (answered, _took) = serving.query(Duration::from_millis(100));
            let said = answered.expect_err("no answer");
            assert!(
                said.contains("vcon_tester") && said.contains(&format!("localhost:{port}")),
                "names who waited, and for which server: {said}"
            );
        }

        #[test]
        fn a_server_without_tls_is_refused_before_the_deadline() {
            let dir = tempfile::tempdir().expect("dir");
            let (answered, took) =
                Serving::new(dir.path(), without_tls).query(Duration::from_secs(20));
            let said = answered.expect_err("refused");
            assert!(
                said.contains("vcon_tester cannot connect") && !said.contains("live-test deadline"),
                "refused for its TLS, not timed out: {said}"
            );
            assert!(took < Duration::from_secs(10), "at once: {took:?}");
        }

        #[test]
        fn a_server_that_hangs_up_is_refused_before_the_deadline() {
            let dir = tempfile::tempdir().expect("dir");
            let (answered, took) =
                Serving::new(dir.path(), hanging_up).query(Duration::from_secs(20));
            let said = answered.expect_err("refused");
            assert!(
                said.contains("vcon_tester cannot connect") && !said.contains("live-test deadline"),
                "refused for the hang-up, not timed out: {said}"
            );
            assert!(took < Duration::from_secs(10), "at once: {took:?}");
        }

        #[test]
        fn a_ca_file_that_is_missing_is_named() {
            let dir = tempfile::tempdir().expect("dir");
            let mut serving = Serving::new(dir.path(), silent);
            serving.server.ca_file = dir.path().join("absent.pem");
            let (answered, took) = serving.query(Duration::from_secs(20));
            let said = answered.expect_err("refused");
            assert!(said.contains("the test CA"), "names the CA: {said}");
            assert!(took < Duration::from_secs(10), "before any wait: {took:?}");
        }

        #[test]
        fn a_deadline_of_zero_is_missed_at_once_and_named() {
            let dir = tempfile::tempdir().expect("dir");
            let (answered, took) = Serving::new(dir.path(), silent).query(Duration::ZERO);
            let said = answered.expect_err("no answer");
            assert!(said.contains("(0ns"), "names the zero deadline: {said}");
            assert!(took < Duration::from_secs(5), "at once: {took:?}");
        }

        #[test]
        fn the_deadline_covers_the_whole_login_not_only_the_tcp_connect() {
            // The TCP connection is taken at once: only a deadline over
            // the TLS handshake and the login ends this wait.
            let dir = tempfile::tempdir().expect("dir");
            let (answered, took) =
                Serving::new(dir.path(), silent).query(Duration::from_millis(500));
            let said = answered.expect_err("no answer");
            assert!(
                said.contains("live-test deadline") && took < Duration::from_secs(5),
                "the login's wait is bounded by the deadline: {said} after {took:?}"
            );
        }
    }
}

mod edge {
    use super::*;

    #[test]
    fn one_millisecond_is_one_second_in_a_configuration() {
        assert_eq!(whole_seconds(Duration::from_millis(1)), 1, "rounded up");
    }

    #[test]
    fn zero_is_one_second_in_a_configuration_the_least_the_server_takes() {
        assert_eq!(whole_seconds(Duration::ZERO), 1, "never zero");
    }

    #[test]
    fn a_second_and_a_half_is_two_seconds_in_a_configuration() {
        assert_eq!(whole_seconds(Duration::from_millis(1500)), 2, "rounded up");
    }

    #[test]
    fn just_under_a_second_is_one_second_in_a_configuration() {
        assert_eq!(whole_seconds(Duration::from_millis(999)), 1, "rounded up");
    }

    #[test]
    fn a_nanosecond_past_a_whole_second_rounds_up() {
        assert_eq!(
            whole_seconds(Duration::new(59, 1)),
            60,
            "the server never waits less than asked"
        );
    }

    #[test]
    fn the_longest_duration_does_not_overflow() {
        assert_eq!(whole_seconds(Duration::MAX), u64::MAX, "saturates");
    }

    #[test]
    fn a_zero_deadline_still_makes_a_configuration_the_server_reads() {
        assert_eq!(
            postgres(&written(Duration::ZERO)).connect_timeout,
            Duration::from_secs(1),
            "one second, not a refused zero"
        );
        assert_eq!(
            s3(&written(Duration::ZERO)).timeout,
            Duration::from_secs(1),
            "one second, not a refused zero"
        );
    }

    #[test]
    fn a_fractional_deadline_is_rounded_up_in_the_configuration() {
        assert_eq!(
            postgres(&written(Duration::from_millis(1500))).connect_timeout,
            Duration::from_secs(2),
            "never shorter than asked"
        );
    }

    #[test]
    fn paths_with_spaces_and_quotes_are_read_back_as_written() {
        let config = written_with(
            "/tmp/a dir/ca \"x\".pem",
            "/tmp/a dir/pg\\pw",
            LIVE_DEADLINE,
        );
        let pg = postgres(&config);
        assert_eq!(
            pg.ca_file,
            Some(PathBuf::from("/tmp/a dir/ca \"x\".pem")),
            "the CA path round-trips"
        );
        assert_eq!(
            pg.password_file,
            Some(PathBuf::from("/tmp/a dir/pg\\pw")),
            "the password path round-trips"
        );
    }

    #[test]
    fn a_missed_zero_deadline_is_named_as_zero() {
        let said = missed("vcon_app", Duration::ZERO);
        assert!(said.contains("0ns"), "names the zero deadline: {said}");
    }

    #[test]
    fn the_live_deadline_leaves_the_test_runner_time_to_report_it() {
        let runner = runner_deadline();
        assert!(
            LIVE_DEADLINE.as_secs() * 2 < runner,
            "two missed connections are reported by name before the runner's {runner} s ends the \
             test: {LIVE_DEADLINE:?}"
        );
    }

    #[test]
    fn the_live_deadline_is_a_whole_number_of_seconds() {
        assert_eq!(
            LIVE_DEADLINE.subsec_nanos(),
            0,
            "a configuration holds it exactly: {LIVE_DEADLINE:?}"
        );
        assert_eq!(
            whole_seconds(LIVE_DEADLINE),
            LIVE_DEADLINE.as_secs(),
            "the configuration's value is the deadline itself"
        );
    }

    /// The library's source file `relative`.
    fn source(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn the_library_includes_the_deadline_only_in_its_test_build() {
        // With #[cfg(test)] on the include, the compiler refuses any
        // reference from code a release build compiles, so nothing of
        // the file reaches a release binary.
        let lib = source("src/lib.rs");
        let at = lib
            .find("pub(crate) mod live_deadline;")
            .expect("src/lib.rs includes the live deadline");
        let before = &lib[..at];
        // The item before ends at a line ending in `;`; a `;` inside an
        // attribute's text does not end a line.
        let item_start = before.rfind(";\n").map_or(0, |i| i + 2);
        let attributes = &before[item_start..];
        assert!(
            attributes.contains("#[cfg(test)]"),
            "the include is for the test build only: {attributes}"
        );
        assert!(
            attributes.contains("#[path = \"../tests/support/live_deadline.rs\"]"),
            "the include is this one file: {attributes}"
        );
    }

    #[test]
    fn the_library_names_the_deadline_only_in_its_test_modules() {
        let allowed = [
            "src/lib.rs",
            "src/backend/postgres/tests.rs",
            "src/backend/postgres/tests/deadline_tests.rs",
            "src/backend/s3.rs",
        ];
        let mut found = Vec::new();
        let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("reads src").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && std::fs::read_to_string(&path)
                        .expect("reads")
                        .contains("live_deadline")
                {
                    found.push(path);
                }
            }
        }
        let base = Path::new(env!("CARGO_MANIFEST_DIR"));
        for path in &found {
            let relative = path.strip_prefix(base).expect("under the crate");
            assert!(
                allowed.iter().any(|a| Path::new(a) == relative),
                "{} names the live deadline: only the test modules listed may",
                relative.display()
            );
        }
        let s3 = source("src/backend/s3.rs");
        let tests_start = s3
            .find("#[cfg(test)]\nmod tests {")
            .expect("s3.rs has its tests");
        let first = s3.find("live_deadline").expect("s3.rs's tests use it");
        assert!(
            first > tests_start,
            "s3.rs names the live deadline only after its test module starts"
        );
    }

    #[test]
    fn the_servers_defaults_are_unchanged() {
        assert_eq!(
            POSTGRES_CONNECT_TIMEOUT,
            Duration::from_secs(5),
            "the production connect timeout"
        );
        assert_eq!(
            S3_TIMEOUT,
            Duration::from_secs(30),
            "the production S3 timeout"
        );
    }
}
