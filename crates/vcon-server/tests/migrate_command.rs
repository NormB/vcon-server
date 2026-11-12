//! `vcon-server migrate` as a process, where no PostgreSQL server is
//! needed: the command line, the configuration it reads, and each refusal
//! that comes before a schema is touched. The migrations themselves run
//! against a real server in `tests/serve.rs` and in the library's
//! `backend::postgres` tests (cargo feature `backend-tests`).
//!
//! A test that needs a server's address has a listener of its own on the
//! loopback address, held open for the whole test, which answers a
//! PostgreSQL client's TLS request with "no TLS" or closes the connection.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// What every refusal of the storage starts with.
const STORAGE: &str = "vcon-server: the storage cannot be used: ";

/// The lines every configuration here starts with. Nothing reads the data
/// directory, the master key or the certificate: `migrate` needs none.
fn least(dir: &Path) -> String {
    format!(
        "listen = \"127.0.0.1:1\"\ndata_dir = {:?}\nmaster_key = {:?}\n\
         [tls]\ncert = \"unused.pem\"\nkey = \"unused.pem\"\n",
        dir.join("data"),
        dir.join("master.key")
    )
}

/// Writes `text` as the configuration in `dir` and gives its path.
fn config(dir: &Path, text: &str) -> PathBuf {
    let path = dir.join("vcon-server.toml");
    std::fs::write(&path, text).expect("the configuration is written");
    path
}

/// Runs `vcon-server <args...>`.
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .args(args)
        .output()
        .expect("the binary runs")
}

/// Runs `vcon-server migrate --config <config>`.
fn migrate(config: &Path) -> Output {
    run(&[
        "migrate",
        "--config",
        config.to_str().expect("a UTF-8 path"),
    ])
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// What a run that must be refused said: exit code 1, nothing printed, and
/// text that starts with the program's name.
fn refused(out: &Output) -> String {
    let said = stderr(out);
    assert_eq!(out.status.code(), Some(1), "exit code 1: {said}");
    assert!(out.stdout.is_empty(), "a refusal prints nothing: {said}");
    assert!(
        said.starts_with("vcon-server: "),
        "names the program: {said}"
    );
    said
}

/// What a command line that does not parse said: exit code 2.
fn usage_error(args: &[&str]) -> String {
    let out = run(args);
    let said = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "{args:?}: exit code 2: {said}");
    assert!(out.stdout.is_empty(), "{args:?}: nothing printed: {said}");
    said
}

#[test]
fn migrate_with_no_catalog_table_is_refused_as_a_local_catalog() {
    let dir = tempfile::tempdir().expect("a directory");
    let said = refused(&migrate(&config(dir.path(), &least(dir.path()))));
    assert_eq!(
        said.trim_end(),
        format!("{STORAGE}migrate is for a PostgreSQL catalog: catalog.backend is \"local\""),
        "the whole line"
    );
}

#[test]
fn migrate_with_the_local_backend_named_is_refused_the_same_way() {
    let dir = tempfile::tempdir().expect("a directory");
    let text = format!("{}\n[catalog]\nbackend = \"local\"\n", least(dir.path()));
    let said = refused(&migrate(&config(dir.path(), &text)));
    assert!(
        said.starts_with(STORAGE) && said.contains("catalog.backend is \"local\""),
        "names the setting: {said}"
    );
}

#[test]
fn migrate_with_a_missing_configuration_names_the_file() {
    let dir = tempfile::tempdir().expect("a directory");
    let missing = dir.path().join("none.toml");
    let said = refused(&migrate(&missing));
    assert!(
        said.starts_with("vcon-server: configuration: ")
            && said.contains(&missing.display().to_string()),
        "names the file: {said}"
    );
}

#[test]
fn migrate_with_a_configuration_that_does_not_parse_names_the_key() {
    let dir = tempfile::tempdir().expect("a directory");
    let said = refused(&migrate(&config(dir.path(), "colour = \"blue\"\n")));
    assert!(
        said.starts_with("vcon-server: configuration: ") && said.contains("colour"),
        "names the key: {said}"
    );
}

#[test]
fn migrate_with_an_unknown_backend_is_refused_by_the_configuration() {
    let dir = tempfile::tempdir().expect("a directory");
    let text = format!("{}\n[catalog]\nbackend = \"mysql\"\n", least(dir.path()));
    let said = refused(&migrate(&config(dir.path(), &text)));
    assert!(
        said.starts_with("vcon-server: configuration: ") && said.contains("\"mysql\""),
        "names the value: {said}"
    );
}

#[test]
fn migrate_with_a_postgres_catalog_and_local_objects_is_refused_by_the_configuration() {
    let dir = tempfile::tempdir().expect("a directory");
    let text = format!(
        "{}\n[catalog]\nbackend = \"postgres\"\n\n[catalog.postgres]\n\
         hosts = [\"db1.example.com\"]\ndbname = \"vcon\"\nuser = \"vcon_migrate\"\n",
        least(dir.path())
    );
    let said = refused(&migrate(&config(dir.path(), &text)));
    assert!(
        said.starts_with("vcon-server: configuration: ") && said.contains("storage.objects"),
        "names the setting a PostgreSQL catalog needs: {said}"
    );
}

#[test]
fn migrate_with_a_postgres_backend_and_no_postgres_table_is_refused_by_the_configuration() {
    let dir = tempfile::tempdir().expect("a directory");
    let text = format!("{}\n[catalog]\nbackend = \"postgres\"\n", least(dir.path()));
    let said = refused(&migrate(&config(dir.path(), &text)));
    assert!(
        said.starts_with("vcon-server: configuration: ") && said.contains("catalog.postgres"),
        "names the missing table: {said}"
    );
}

#[test]
fn migrate_without_a_configuration_argument_exits_2() {
    let said = usage_error(&["migrate"]);
    assert!(said.contains("--config"), "names the argument: {said}");
}

#[test]
fn migrate_with_another_argument_exits_2() {
    let said = usage_error(&["migrate", "--config", "a.toml", "now"]);
    assert!(said.contains("now"), "names the argument: {said}");
}

#[test]
fn migrate_with_an_unknown_option_exits_2() {
    let said = usage_error(&["migrate", "--config", "a.toml", "--to", "3"]);
    assert!(said.contains("--to"), "names the option: {said}");
}

#[test]
fn migrate_with_the_configuration_named_twice_exits_2() {
    let said = usage_error(&["migrate", "--config", "a.toml", "--config", "b.toml"]);
    assert!(said.contains("--config"), "names the option: {said}");
}

#[test]
fn migrate_help_says_what_it_does_and_exits_0() {
    let out = run(&["migrate", "--help"]);
    let printed = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "help exits 0: {}", stderr(&out));
    assert!(
        printed.contains("PostgreSQL") && printed.contains("vcon_owner"),
        "names the catalog and the role: {printed}"
    );
    assert!(
        printed.contains("--config"),
        "names the argument: {printed}"
    );
}

#[test]
fn the_programs_help_lists_migrate() {
    let out = run(&["--help"]);
    let printed = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "help exits 0: {}", stderr(&out));
    assert!(
        printed
            .lines()
            .any(|l| l.trim_start().starts_with("migrate ")),
        "a line for the command: {printed}"
    );
}

#[test]
fn migrate_reads_no_data_directory_and_no_master_key_and_makes_neither() {
    let dir = tempfile::tempdir().expect("a directory");
    let said = refused(&migrate(&config(dir.path(), &least(dir.path()))));
    assert!(
        said.contains("migrate is for a PostgreSQL catalog"),
        "the catalog's refusal, not a data directory's or a key's: {said}"
    );
    assert!(
        !dir.path().join("data").exists(),
        "no data directory was made"
    );
    assert!(
        !dir.path().join("master.key").exists(),
        "no master key was made"
    );
}

#[test]
fn migrate_does_not_wait_for_or_refuse_the_data_directorys_lock() {
    let dir = tempfile::tempdir().expect("a directory");
    let data = dir.path().join("data");
    std::fs::create_dir(&data).expect("the data directory");
    let _server = vcon_server::datadir::lock(&data, vcon_server::datadir::Sharing::Serve)
        .expect("a server's lock");
    let said = refused(&migrate(&config(dir.path(), &least(dir.path()))));
    assert!(
        said.contains("migrate is for a PostgreSQL catalog") && !said.contains("in use"),
        "the catalog's refusal while a server holds the directory: {said}"
    );
}

#[test]
fn migrate_leaves_the_configuration_file_as_it_was() {
    let dir = tempfile::tempdir().expect("a directory");
    let text = least(dir.path());
    let path = config(dir.path(), &text);
    refused(&migrate(&path));
    assert_eq!(
        std::fs::read_to_string(&path).expect("the file is read"),
        text,
        "the command reads the file and no more"
    );
}

/// A configuration with the objects in a bucket and the catalogs in
/// PostgreSQL at `port` of each of `hosts`, logging in with `password`.
fn postgres_config(dir: &Path, hosts: &[&str], port: u16, password: &Path) -> PathBuf {
    let hosts: Vec<String> = hosts.iter().map(|h| format!("{h:?}")).collect();
    let text = format!(
        "{}\n[storage]\nobjects = \"s3\"\n\n[storage.s3]\nregion = \"us-east-1\"\n\
         bucket = \"vcon\"\n\n[catalog]\nbackend = \"postgres\"\n\n[catalog.postgres]\n\
         hosts = [{}]\nport = {port}\ndbname = \"vcon\"\nuser = \"vcon_migrate\"\n\
         password_file = {password:?}\n",
        least(dir),
        hosts.join(", ")
    );
    config(dir, &text)
}

/// A file a secret is read from, with `mode`.
fn secret_file(dir: &Path, mode: u32) -> PathBuf {
    let path = dir.join("pg.password");
    std::fs::write(&path, "not-a-real-one\n").expect("the file is written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("the mode");
    path
}

#[cfg(not(feature = "postgres"))]
#[test]
fn migrate_in_a_build_without_postgres_names_the_cargo_feature() {
    let dir = tempfile::tempdir().expect("a directory");
    let password = secret_file(dir.path(), 0o600);
    let path = postgres_config(dir.path(), &["db1.example.com"], 5432, &password);
    let said = refused(&migrate(&path));
    assert!(
        said.starts_with(STORAGE)
            && said.contains("catalog.backend is \"postgres\"")
            && said.contains("cargo feature postgres"),
        "names the setting and the feature: {said}"
    );
}

#[cfg(feature = "postgres")]
mod with_postgres {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// What the listener does with a connection.
    #[derive(Clone, Copy)]
    enum Answer {
        /// Reads the client's TLS request and answers that it has no TLS.
        NoTls,
        /// Closes the connection.
        Close,
    }

    /// A listener on the loopback address, open until it is dropped with
    /// the test, that counts the connections it took.
    struct Listener {
        port: u16,
        taken: Arc<AtomicUsize>,
    }

    impl Listener {
        fn start(answer: Answer) -> Listener {
            let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
            let port = listener.local_addr().expect("an address").port();
            let taken = Arc::new(AtomicUsize::new(0));
            let count = Arc::clone(&taken);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { break };
                    count.fetch_add(1, Ordering::SeqCst);
                    answer_one(stream, answer);
                }
            });
            Listener { port, taken }
        }

        fn taken(&self) -> usize {
            self.taken.load(Ordering::SeqCst)
        }
    }

    fn answer_one(mut stream: TcpStream, answer: Answer) {
        if matches!(answer, Answer::NoTls) {
            // The TLS request is eight bytes; "N" says the server has none.
            let mut request = [0_u8; 8];
            if stream.read_exact(&mut request).is_ok() {
                let _ = stream.write_all(b"N");
            }
        }
    }

    fn server_line(port: u16) -> String {
        format!("{STORAGE}catalog.postgres: no server could be used: 127.0.0.1:{port}: ")
    }

    #[test]
    fn migrate_reaches_the_server_the_configuration_names_and_refuses_one_without_tls() {
        let dir = tempfile::tempdir().expect("a directory");
        let server = Listener::start(Answer::NoTls);
        let password = secret_file(dir.path(), 0o600);
        let path = postgres_config(dir.path(), &["127.0.0.1"], server.port, &password);
        let said = refused(&migrate(&path));
        assert!(
            said.starts_with(&server_line(server.port)),
            "names the key and the server: {said}"
        );
        assert!(
            said.contains("turn ssl on in the server's postgresql.conf"),
            "says what to change: {said}"
        );
        assert_eq!(said.lines().count(), 1, "one line: {said}");
        assert_eq!(server.taken(), 1, "one connection, and no retry");
    }

    #[test]
    fn migrate_refuses_a_server_that_closes_the_connection_and_names_it() {
        let dir = tempfile::tempdir().expect("a directory");
        let server = Listener::start(Answer::Close);
        let password = secret_file(dir.path(), 0o600);
        let path = postgres_config(dir.path(), &["127.0.0.1"], server.port, &password);
        let said = refused(&migrate(&path));
        assert!(
            said.starts_with(&server_line(server.port)),
            "names the key and the server: {said}"
        );
        assert_eq!(server.taken(), 1, "one connection, and no retry");
    }

    #[test]
    fn migrate_tries_each_host_and_names_each_refusal() {
        let dir = tempfile::tempdir().expect("a directory");
        let server = Listener::start(Answer::NoTls);
        let password = secret_file(dir.path(), 0o600);
        let path = postgres_config(
            dir.path(),
            &["127.0.0.1", "localhost"],
            server.port,
            &password,
        );
        let said = refused(&migrate(&path));
        assert!(
            said.starts_with(&server_line(server.port))
                && said.contains(&format!("; localhost:{}: ", server.port)),
            "both hosts, in the order given: {said}"
        );
    }

    #[test]
    fn migrate_reads_the_password_file_before_it_connects_and_names_a_missing_one() {
        let dir = tempfile::tempdir().expect("a directory");
        let server = Listener::start(Answer::NoTls);
        let missing = dir.path().join("pg.password");
        let path = postgres_config(dir.path(), &["127.0.0.1"], server.port, &missing);
        let said = refused(&migrate(&path));
        assert!(
            said.starts_with(&format!("{STORAGE}catalog.postgres.password_file "))
                && said.contains(&missing.display().to_string()),
            "names the key and the file: {said}"
        );
        assert_eq!(server.taken(), 0, "no connection without the secret");
    }

    #[test]
    fn migrate_refuses_a_password_file_other_users_may_read() {
        let dir = tempfile::tempdir().expect("a directory");
        let server = Listener::start(Answer::NoTls);
        let password = secret_file(dir.path(), 0o644);
        let path = postgres_config(dir.path(), &["127.0.0.1"], server.port, &password);
        let said = refused(&migrate(&path));
        assert!(
            said.starts_with(&format!("{STORAGE}catalog.postgres.password_file "))
                && said.contains("mode 0644")
                && said.contains("chmod 600"),
            "names the mode and the command: {said}"
        );
        assert_eq!(server.taken(), 0, "no connection with such a file");
    }

    #[test]
    fn migrate_refuses_an_empty_password_file() {
        let dir = tempfile::tempdir().expect("a directory");
        let server = Listener::start(Answer::NoTls);
        let password = dir.path().join("pg.password");
        std::fs::write(&password, "").expect("the file is written");
        std::fs::set_permissions(&password, std::fs::Permissions::from_mode(0o600))
            .expect("the mode");
        let path = postgres_config(dir.path(), &["127.0.0.1"], server.port, &password);
        let said = refused(&migrate(&path));
        assert!(
            said.starts_with(&format!("{STORAGE}catalog.postgres.password_file ")),
            "names the key: {said}"
        );
        assert_eq!(server.taken(), 0, "no connection with no password");
    }

    #[test]
    fn migrate_does_not_print_the_password_in_a_refusal() {
        let dir = tempfile::tempdir().expect("a directory");
        let server = Listener::start(Answer::NoTls);
        let password = secret_file(dir.path(), 0o600);
        let path = postgres_config(dir.path(), &["127.0.0.1"], server.port, &password);
        let out = migrate(&path);
        let said = refused(&out);
        let held = std::fs::read_to_string(&password).expect("the file is read");
        assert!(
            !said.contains(held.trim_end()),
            "the refusal holds no part of the secret file"
        );
    }

    #[test]
    fn migrate_reaches_the_server_while_a_server_holds_the_data_directory() {
        let dir = tempfile::tempdir().expect("a directory");
        let data = dir.path().join("data");
        std::fs::create_dir(&data).expect("the data directory");
        let _held = vcon_server::datadir::lock(&data, vcon_server::datadir::Sharing::Serve)
            .expect("a server's lock");
        let server = Listener::start(Answer::NoTls);
        let password = secret_file(dir.path(), 0o600);
        let path = postgres_config(dir.path(), &["127.0.0.1"], server.port, &password);
        let said = refused(&migrate(&path));
        assert!(
            said.starts_with(&server_line(server.port)) && !said.contains("in use"),
            "the database's locks are the ones migrate takes: {said}"
        );
        assert_eq!(server.taken(), 1, "the server was reached");
    }
}
