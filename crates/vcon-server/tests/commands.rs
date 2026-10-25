//! The commands other than `serve`, run as their own process: what each
//! prints, and its exit code. 0 is done, 1 is refused or failed (the reason
//! on standard error), 2 is a command line that does not parse.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;
use vcon_server::datadir::{self, Sharing};

struct Setup {
    _dir: tempfile::TempDir,
    config: PathBuf,
    data: PathBuf,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().expect("temp dir");
    let data = dir.path().join("data");
    std::fs::create_dir(&data).expect("data dir");
    let master = dir.path().join("master.key");
    std::fs::write(&master, [0x42; 32]).expect("master key");
    let config = dir.path().join("vcon-server.toml");
    std::fs::write(
        &config,
        format!(
            "listen = \"127.0.0.1:1\"\ndata_dir = {:?}\nmaster_key = {:?}\n\
             [tls]\ncert = \"unused.pem\"\nkey = \"unused.pem\"\n",
            data, master
        ),
    )
    .expect("configuration");
    Setup {
        _dir: dir,
        config,
        data,
    }
}

impl Setup {
    /// Runs `vcon-server <args...> --config <file>`.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .output()
            .expect("the binary runs")
    }

    /// Runs a command that must succeed, and parses each line it prints.
    fn ok(&self, args: &[&str]) -> Vec<Value> {
        let out = self.run(args);
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        assert_eq!(stderr(&out), "", "{args:?} wrote to stderr");
        String::from_utf8(out.stdout)
            .expect("text")
            .lines()
            .map(|l| serde_json::from_str(l).expect("a JSON line"))
            .collect()
    }

    /// Runs a command that must be refused with exit code 1, printing
    /// nothing, and returns what it said.
    fn refused(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
        assert!(out.stdout.is_empty(), "{args:?} printed on refusal");
        let said = stderr(&out);
        assert!(said.starts_with("vcon-server: "), "{said}");
        said
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn usage_error(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .args(args)
        .output()
        .expect("the binary runs");
    assert_eq!(out.status.code(), Some(2), "{args:?}: {}", stderr(&out));
    stderr(&out)
}

#[test]
fn tenants_are_made_listed_changed_and_removed() {
    let s = setup();
    let made = s.ok(&["tenant", "create", "acme", "--retention", "18m"]);
    let id = made[0]["id"].as_str().expect("an id").to_owned();
    assert_eq!(made.len(), 1);
    assert_eq!(
        s.ok(&["tenant", "list"]),
        [serde_json::json!({ "id": id, "name": "acme", "retention": "18m" })]
    );
    assert_eq!(
        s.ok(&["tenant", "set-retention", &id, "default"]),
        [serde_json::json!({ "id": id, "retention": null })]
    );
    assert_eq!(s.ok(&["tenant", "remove", &id]), Vec::<Value>::new());
    assert_eq!(s.ok(&["tenant", "list"]), Vec::<Value>::new());
}

#[test]
fn keys_are_made_for_a_tenant_or_the_operator_listed_and_revoked() {
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let made = s.ok(&[
        "key", "create", "--tenant", &tenant, "--scope", "write", "--scope", "read", "--label",
        "producer",
    ]);
    let token = made[0]["token"].as_str().expect("token");
    let id = made[0]["id"].as_str().expect("id").to_owned();
    assert!(token.starts_with(&format!("vcs_{id}_")), "{token}");
    let operator = s.ok(&["key", "create", "--operator", "--label", "ops"]);
    assert!(operator[0]["token"].as_str().is_some());

    let listed = s.ok(&["key", "list"]);
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0]["scopes"], serde_json::json!(["write", "read"]));
    assert_eq!(listed[1]["scopes"], serde_json::json!(["admin"]));
    assert_eq!(listed[1]["tenant"], Value::Null);
    assert_eq!(s.ok(&["key", "list", "--tenant", &tenant]).len(), 1);

    assert_eq!(s.ok(&["key", "revoke", &id]), Vec::<Value>::new());
    assert!(s.ok(&["key", "list"])[0]["revoked_at"].is_string());
}

#[test]
fn audit_export_writes_the_admin_log_or_a_tenants() {
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    s.ok(&[
        "key", "create", "--tenant", &tenant, "--scope", "read", "--label", "l",
    ]);
    let admin = s.ok(&["audit", "export", "--admin"]);
    let actions: Vec<&str> = admin
        .iter()
        .map(|r| r["action"].as_str().expect("action"))
        .collect();
    assert_eq!(actions, ["tenant_create", "key_create"]);
    let first = admin[0]["seq"].as_i64().expect("seq").to_string();
    assert_eq!(
        s.ok(&["audit", "export", "--admin", "--since", &first]),
        admin[1..]
    );
    let own = s.ok(&["audit", "export", "--tenant", &tenant]);
    assert_eq!(own.len(), 1);
    assert_eq!(own[0]["action"], "key_create");
}

#[test]
fn each_refusal_exits_1_and_says_why() {
    let s = setup();
    let unknown = "00000000000000000000000000000000";
    for (args, words) in [
        (vec!["tenant", "set-retention", unknown, "7y"], "no tenant"),
        (
            vec!["tenant", "create", "acme", "--retention", "7w"],
            "\"7w\"",
        ),
        (vec!["key", "revoke", "0123456789abcdef"], "no key"),
        (
            vec![
                "key", "create", "--tenant", unknown, "--scope", "read", "--label", "l",
            ],
            "no tenant",
        ),
        (vec!["audit", "export", "--tenant", "nope"], "no tenant"),
    ] {
        let said = s.refused(&args);
        assert!(said.contains(words), "{args:?}: {said}");
    }
    s.ok(&["tenant", "create", "acme"]);
    assert!(s.refused(&["tenant", "create", "acme"]).contains("name"));
}

#[test]
fn a_command_line_that_does_not_parse_exits_2() {
    for args in [
        vec!["key", "create", "--label", "l", "--config", "c"],
        vec![
            "key",
            "create",
            "--operator",
            "--tenant",
            "t",
            "--label",
            "l",
            "--config",
            "c",
        ],
        vec![
            "key",
            "create",
            "--operator",
            "--scope",
            "read",
            "--label",
            "l",
            "--config",
            "c",
        ],
        vec!["tenant", "remove", "t", "--override-holds", "--config", "c"],
        vec!["audit", "export", "--config", "c"],
        vec![
            "audit", "export", "--admin", "--tenant", "t", "--config", "c",
        ],
        vec!["tenant", "list"],
    ] {
        usage_error(&args);
    }
}

/// The commands that run alone are refused while a server holds the data
/// directory; the others run beside it.
#[test]
fn beside_a_server_only_the_beside_commands_run() {
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let server = datadir::lock(&s.data, Sharing::Serve).expect("a server");
    assert!(s.refused(&["tenant", "create", "beta"]).contains("in use"));
    assert!(s.refused(&["tenant", "remove", &tenant]).contains("in use"));
    s.ok(&["tenant", "list"]);
    s.ok(&["key", "create", "--operator", "--label", "ops"]);
    s.ok(&["audit", "export", "--admin"]);
    drop(server);
}

#[test]
fn a_wrong_or_missing_configuration_exits_1() {
    let s = setup();
    std::fs::write(&s.config, "colour = \"blue\"\n").expect("bad configuration");
    assert!(s.refused(&["tenant", "list"]).contains("colour"));
}
