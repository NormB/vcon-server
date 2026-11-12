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
    std::fs::set_permissions(
        &master,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    )
    .expect("chmod 0600");
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
        [serde_json::json!({ "id": id, "name": "acme", "retention": "18m", "metadata": "none" })]
    );
    assert_eq!(
        s.ok(&["tenant", "set-metadata", &id, "counts"]),
        [serde_json::json!({ "id": id, "metadata": "counts" })],
        "the list shows the default level"
    );
    assert!(
        s.refused(&["tenant", "set-metadata", &id, "most"])
            .contains("not a metadata level"),
        "a level that is not one is refused"
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
    assert!(
        token.starts_with(&format!("vcs_{id}_")),
        "the token starts with vcs_ and the key's id {id}"
    );
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

/// `key create` for a tenant key that reads, with these further arguments.
fn key_create<'a>(tenant: &'a str, more: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "key", "create", "--tenant", tenant, "--scope", "read", "--label", "dated",
    ];
    args.extend_from_slice(more);
    args
}

fn rfc3339(value: &Value) -> time::OffsetDateTime {
    time::OffsetDateTime::parse(
        value.as_str().expect("a time as text"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("RFC 3339")
}

/// The second a key was made in. A key's expiry is a whole second: its
/// life is counted from here.
fn made_in(value: &Value) -> time::OffsetDateTime {
    rfc3339(value)
        .replace_nanosecond(0)
        .expect("zero is in range")
}

#[test]
fn key_create_takes_the_days_until_the_key_expires_or_no_expiry() {
    let s = setup();
    let tenant = id_of(&s.ok(&["tenant", "create", "acme"]));
    let default = s.ok(&key_create(&tenant, &[]));
    let week = s.ok(&key_create(&tenant, &["--expires-days", "7"]));
    let never = s.ok(&key_create(&tenant, &["--no-expiry"]));
    let listed = s.ok(&["key", "list"]);
    assert_eq!(listed.len(), 3, "three keys");
    assert_eq!(
        rfc3339(&default[0]["expires_at"]) - made_in(&listed[0]["created_at"]),
        time::Duration::days(90),
        "with neither argument a key expires in 90 days"
    );
    assert_eq!(
        rfc3339(&week[0]["expires_at"]) - made_in(&listed[1]["created_at"]),
        time::Duration::days(7),
        "--expires-days 7"
    );
    assert!(never[0]["expires_at"].is_null(), "--no-expiry prints null");
    assert!(listed[2]["expires_at"].is_null(), "and the list shows null");
    assert!(
        listed.iter().all(|key| key["last_used_at"].is_null()),
        "no key was used"
    );
}

#[test]
fn key_create_refuses_days_outside_1_to_365_and_says_the_range() {
    let s = setup();
    let tenant = id_of(&s.ok(&["tenant", "create", "acme"]));
    for days in ["0", "366"] {
        let why = s.refused(&key_create(&tenant, &["--expires-days", days]));
        assert!(why.contains("1 to 365 days"), "{days} days: {why}");
    }
    assert_eq!(s.ok(&["key", "list"]).len(), 0, "no key was made");
}

#[test]
fn key_create_with_days_and_no_expiry_or_days_that_are_no_number_exits_2() {
    for more in [
        vec!["--expires-days", "7", "--no-expiry"],
        vec!["--expires-days", "-1"],
        vec!["--expires-days", "soon"],
        vec!["--expires-days"],
    ] {
        let mut args = vec!["key", "create", "--operator", "--label", "ops"];
        args.extend_from_slice(&more);
        args.extend_from_slice(&["--config", "/nonexistent"]);
        let why = usage_error(&args);
        assert!(
            why.contains("--expires-days")
                || why.contains("--no-expiry")
                || why.contains("unexpected argument"),
            "{more:?}: {why}"
        );
    }
}

#[test]
fn key_create_refuses_no_expiry_where_the_configuration_says_so() {
    let s = setup();
    let tenant = id_of(&s.ok(&["tenant", "create", "acme"]));
    let mut text = std::fs::read_to_string(&s.config).expect("the configuration");
    text.push_str("\n[keys]\nno_expiry = false\n");
    std::fs::write(&s.config, text).expect("written");
    let why = s.refused(&key_create(&tenant, &["--no-expiry"]));
    assert!(why.contains("keys.no_expiry"), "names the key: {why}");
    let why = s.refused(&[
        "key",
        "create",
        "--operator",
        "--label",
        "ops",
        "--no-expiry",
    ]);
    assert!(
        why.contains("keys.no_expiry"),
        "for an operator key too: {why}"
    );
    assert_eq!(
        s.ok(&key_create(&tenant, &["--expires-days", "365"])).len(),
        1,
        "a key for a year is still made"
    );
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
        vec!["export", "parquet", "--tenant", "t", "--config", "c"],
        vec!["export", "parquet", "--to", "d", "--config", "c"],
        vec!["tenant", "set-metadata", "t", "--config", "c"],
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
    s.ok(&["tenant", "set-metadata", &tenant, "full"]);
    let to = s.data.with_file_name("export");
    let exported = s.ok(&[
        "export",
        "parquet",
        "--tenant",
        &tenant,
        "--to",
        to.to_str().expect("a UTF-8 path"),
    ]);
    assert_eq!(exported.len(), 1, "one line");
    assert_eq!(
        exported[0]["metadata"], "full",
        "the export runs at the tenant's level"
    );
    assert_eq!(exported[0]["rows"], 0, "a tenant with no vCon");
    assert!(
        to.join(format!(
            "tenant={tenant}/manifest/00000000000000000001.json"
        ))
        .exists(),
        "the manifest is written"
    );
    drop(server);
}

#[test]
fn rekey_tenant_rotates_a_key_and_exits_0() {
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    assert_eq!(s.ok(&["rekey", "tenant", &tenant]), Vec::<Value>::new());
    let admin = s.ok(&["audit", "export", "--admin"]);
    assert!(
        admin
            .iter()
            .any(|r| r["action"] == "rekey_tenant" && r["status"] == 200)
    );
    assert!(
        s.refused(&["rekey", "tenant", "nope"])
            .contains("no tenant")
    );
    let server = datadir::lock(&s.data, Sharing::Serve).expect("a server");
    assert!(s.refused(&["rekey", "tenant", &tenant]).contains("in use"));
    drop(server);
}

#[test]
fn rekey_master_rotates_and_says_what_comes_next() {
    let s = setup();
    s.ok(&["tenant", "create", "acme"]);
    let new_key = s.data.with_file_name("new.key");
    std::fs::write(&new_key, [0x43; 32]).expect("new key");
    std::fs::set_permissions(
        &new_key,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    )
    .expect("chmod 0600");
    let new_key = new_key.to_str().expect("path");
    let out = s.ok(&["rekey", "master", "--new-master", new_key]);
    assert!(out[0]["master_key_id"].is_string());
    assert!(out[0]["next"].is_string());
    assert!(s.refused(&["tenant", "list"]).contains("master key"));
    usage_error(&["rekey", "master", "--config", "c"]);
}

/// `check` is the one command that prints its report and still exits 1:
/// the report is how it says what is wrong.
#[test]
fn check_exits_0_when_sound_and_1_with_its_report_when_not() {
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let report = s.ok(&["check"]);
    assert_eq!(report.last().expect("summary")["problems"], 0);
    // A temporary file left in the objects directory is reported as a
    // leftover, but is not a problem.
    let objects = s.data.join("tenants").join(&tenant).join("objects");
    let catalog_only = objects.join("00000000000000000000000000000000.tmp");
    std::fs::write(&catalog_only, b"x").expect("a leftover is not a problem");
    let report = s.ok(&["check"]);
    assert_eq!(
        report[0]["leftovers"],
        serde_json::json!(["00000000000000000000000000000000.tmp"]),
        "check reports the leftover temporary file"
    );
    assert_eq!(
        report.last().expect("summary")["problems"],
        0,
        "a leftover is not a problem"
    );
    // A stored version's sealed file goes missing.
    let version = {
        let tenants = vcon_server::tenants::Tenants::open(
            &s.data,
            vcon_server::envelope::Key::load_master(&s.data.with_file_name("master.key"))
                .expect("master"),
            std::sync::Arc::new(vcon_server::fs::OsFs),
            std::sync::Arc::new(vcon_server::clock::SystemClock),
        )
        .expect("opens");
        let id = vcon_server::ids::TenantId::parse(&tenant).expect("id");
        let body = br#"{"uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46", "created_at": "2026-10-25T12:00:00Z", "parties": []}"#;
        let found = vcon::read::inspect(
            body,
            None,
            &vcon::read::Limits::new(1 << 20),
            &Default::default(),
        )
        .expect("a vCon");
        let caller = vcon_server::store::Caller {
            key: vcon_server::ids::KeyId([1; 8]),
        };
        match tenants
            .store(id)
            .expect("store")
            .create(&caller, &found, body)
        {
            Ok(vcon_server::store::Created::New(v)) => v,
            other => panic!("{other:?}"),
        }
    };
    std::fs::remove_file(objects.join(version.to_string())).expect("remove");
    let out = s.run(&["check"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("check found 1 problem"),
        "{}",
        stderr(&out)
    );
    let report: Vec<Value> = String::from_utf8(out.stdout)
        .expect("text")
        .lines()
        .map(|l| serde_json::from_str(l).expect("json"))
        .collect();
    assert_eq!(
        report[0]["damaged"],
        serde_json::json!([version.to_string()])
    );

    let out = s.run(&["tenant", "remove", &tenant, "--force"]);
    assert_eq!(out.status.code(), Some(0));
    let gone = s.ok(&["check"]);
    assert_eq!(
        gone.last().expect("summary")["removed_tenants_awaiting_master_rotation"],
        1
    );
}

/// A tenant whose catalog opens but whose objects the process may not
/// read: `check` says that the objects cannot be read, with the error's
/// kind and the system's error number, as the server's log line does, and
/// not that the catalog cannot be opened. Nothing it prints names an
/// object's key.
#[test]
fn a_tenant_whose_objects_cannot_be_read_is_named_as_that_by_check() {
    use std::os::unix::fs::PermissionsExt;
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let version = stored(&s, &tenant);
    let object = s
        .data
        .join("tenants")
        .join(&tenant)
        .join("objects")
        .join(version.to_string());
    let mode =
        |m| std::fs::set_permissions(&object, std::fs::Permissions::from_mode(m)).expect("chmod");
    mode(0o000);
    let check = s.run(&["check"]);
    mode(0o600);
    assert_eq!(check.status.code(), Some(1), "{}", stderr(&check));
    let said = stderr(&check);
    let printed = String::from_utf8(check.stdout).expect("text");
    let report: Vec<Value> = printed
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert_eq!(
        report[0]["problems"],
        serde_json::json!(["its objects cannot be read: permission denied (os error 13)"]),
        "{printed}"
    );
    assert!(
        !printed.contains(&version.to_string()) && !said.contains(&version.to_string()),
        "the object's key is not printed: {printed}{said}"
    );
    assert_eq!(
        s.ok(&["check"]).last().expect("summary")["problems"],
        0,
        "once readable"
    );
}

/// Stores one vCon in `tenant` through the library, as a running server
/// would, and returns its version.
fn stored(s: &Setup, tenant: &str) -> vcon_server::ids::VersionId {
    let tenants = vcon_server::tenants::Tenants::open(
        &s.data,
        vcon_server::envelope::Key::load_master(&s.data.with_file_name("master.key"))
            .expect("master"),
        std::sync::Arc::new(vcon_server::fs::OsFs),
        std::sync::Arc::new(vcon_server::clock::SystemClock),
    )
    .expect("opens");
    let id = vcon_server::ids::TenantId::parse(tenant).expect("id");
    let body = br#"{"uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46", "created_at": "2026-10-25T12:00:00Z", "parties": []}"#;
    let found = vcon::read::inspect(
        body,
        None,
        &vcon::read::Limits::new(1 << 20),
        &Default::default(),
    )
    .expect("a vCon");
    let caller = vcon_server::store::Caller {
        key: vcon_server::ids::KeyId([1; 8]),
    };
    match tenants
        .store(id)
        .expect("store")
        .create(&caller, &found, body)
    {
        Ok(vcon_server::store::Created::New(v)) => v,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_wrong_or_missing_configuration_exits_1() {
    let s = setup();
    std::fs::write(&s.config, "colour = \"blue\"\n").expect("bad configuration");
    assert!(s.refused(&["tenant", "list"]).contains("colour"));
    std::fs::remove_file(&s.config).expect("remove the configuration");
    let said = s.refused(&["tenant", "list"]);
    assert!(
        said.contains(&s.config.display().to_string()),
        "the missing file is named: {said}"
    );
}

/// A tenant's directory the service account may not write, as a command
/// run as root leaves it: `check` names the path and the system's error,
/// and `serve` refuses with the error, not with a key problem.
#[test]
fn a_tenant_catalog_that_may_not_be_opened_is_named_as_that_by_check_and_serve() {
    use std::os::unix::fs::PermissionsExt;
    let s = setup();
    let tenant = s.ok(&["tenant", "create", "acme"])[0]["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let dir = s.data.join("tenants").join(&tenant);
    let mode =
        |m| std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(m)).expect("chmod");
    mode(0o500);
    let check = s.run(&["check"]);
    let serve = s.run(&["serve"]);
    mode(0o700);
    assert_eq!(check.status.code(), Some(1), "{}", stderr(&check));
    let report: Vec<Value> = String::from_utf8(check.stdout)
        .expect("text")
        .lines()
        .map(|l| serde_json::from_str(l).expect("a JSON line"))
        .collect();
    assert_eq!(
        report[0]["problems"],
        serde_json::json!([format!(
            "its catalog cannot be opened: {}: Permission denied (os error 13)",
            dir.display()
        )])
    );
    assert_eq!(serve.status.code(), Some(1), "{}", stderr(&serve));
    assert_eq!(
        stderr(&serve),
        "vcon-server: 1 tenant catalog(s) cannot be opened: Permission denied (os error 13); vcon-server check names the tenants\n"
    );
    assert_eq!(
        s.ok(&["check"]).last().expect("summary")["problems"],
        0,
        "once allowed"
    );
}

/// A page of the site's documentation, by its file name.
fn docs_page(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/content/docs")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The text under the `## {heading}` line of `page`, up to the next `## `
/// heading.
fn docs_section(page: &str, heading: &str) -> String {
    let start = format!("## {heading}\n");
    let rest = page
        .split_once(&start)
        .unwrap_or_else(|| panic!("no section {heading:?}"))
        .1;
    rest.split("\n## ").next().unwrap_or(rest).to_owned()
}

/// The members of a JSON object, sorted.
fn members(line: &Value) -> Vec<String> {
    let mut names: Vec<String> = line
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

/// The members a `Prints` cell of the command-line page names:
/// `{"id", "name"}` gives `id` and `name`, sorted. A cell with no braces
/// gives none.
fn printed_members(cell: &str) -> Vec<String> {
    let Some((_, rest)) = cell.split_once('{') else {
        return Vec::new();
    };
    let inside = rest.split('}').next().unwrap_or_default();
    let mut names: Vec<String> = inside
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    names.sort();
    names
}

/// The `Prints` cell of the command-line page's row whose command starts
/// with `command`.
fn prints_cell(page: &str, command: &str) -> String {
    let row = page
        .lines()
        .find(|line| line.starts_with(&format!("| `{command}")))
        .unwrap_or_else(|| panic!("the command-line page has no row for `{command}`"));
    row.split(" | ").nth(1).unwrap_or_default().to_owned()
}

#[test]
fn the_page_readers_read_a_section_a_row_and_its_members() {
    let page = "## A\n\ntext a\n\n## B\n\n| Command | Prints | Beside |\n|---|---|---|\n\
                | `x list` | `{\"id\", \"name\"}` per x, by name | yes |\n| `x remove <id>` | nothing | no |\n\n## C\n";
    assert_eq!(
        docs_section(page, "A"),
        "\ntext a\n",
        "a section ends at the next heading"
    );
    assert_eq!(
        printed_members(&prints_cell(page, "x list")),
        ["id", "name"],
        "the members a row says its command prints"
    );
    assert!(
        printed_members(&prints_cell(page, "x remove")).is_empty(),
        "a command that prints nothing names no member"
    );
    assert_eq!(
        members(&serde_json::json!({"b": 1, "a": null})),
        ["a", "b"],
        "an object's members, sorted"
    );
}

/// The command-line page says what each member of `check`'s report means.
/// Found on 0.4.2, when the operations page held the list: it left out
/// `tenant` and `name` of a tenant's line, and `tenants` and
/// `oldest_removal_awaiting_master_rotation` of the summary.
#[test]
fn the_command_line_page_names_every_member_check_prints() {
    let s = setup();
    s.ok(&["tenant", "create", "acme"]);
    let report = s.ok(&["check"]);
    assert_eq!(report.len(), 2, "one line for the tenant, then the summary");
    let section = docs_section(&docs_page("command-line.md"), "Checking");
    let mut missing = Vec::new();
    for line in &report {
        for name in members(line) {
            if !section.contains(&format!("`{name}`")) {
                missing.push(name);
            }
        }
    }
    assert!(
        missing.is_empty(),
        "command-line.md, \"Checking\", does not name {missing:?}"
    );
}

/// The command-line page's `Prints` column names the members of each
/// command's output, and they are the members the command prints.
#[test]
fn the_command_line_page_names_the_members_each_command_prints() {
    let s = setup();
    let page = docs_page("command-line.md");
    let mut wrong = Vec::new();
    let mut hold = |command: &str, printed: &Value| {
        let named = printed_members(&prints_cell(&page, command));
        let actual = members(printed);
        if named != actual {
            wrong.push(format!(
                "`{command}`: the page names {named:?}, it prints {actual:?}"
            ));
        }
    };
    let made = s.ok(&["tenant", "create", "acme"]);
    hold("tenant create", &made[0]);
    let tenant = made[0]["id"].as_str().expect("an id").to_owned();
    hold("tenant list", &s.ok(&["tenant", "list"])[0]);
    hold(
        "tenant set-retention",
        &s.ok(&["tenant", "set-retention", &tenant, "5y"])[0],
    );
    hold(
        "tenant set-metadata",
        &s.ok(&["tenant", "set-metadata", &tenant, "counts"])[0],
    );
    hold(
        "tenant set-role-cap <id> --role",
        &s.ok(&["tenant", "set-role-cap", &tenant, "--role", "reader"])[0],
    );
    hold(
        "tenant set-role-cap <id> --all",
        &s.ok(&["tenant", "set-role-cap", &tenant, "--all"])[0],
    );
    hold(
        "tenant role-cap",
        &s.ok(&["tenant", "role-cap", &tenant])[0],
    );
    let key = s.ok(&[
        "key", "create", "--tenant", &tenant, "--scope", "read", "--label", "a",
    ]);
    hold("key create --tenant", &key[0]);
    hold(
        "key create --operator",
        &s.ok(&["key", "create", "--operator", "--label", "o"])[0],
    );
    hold("key list", &s.ok(&["key", "list"])[0]);
    let target = s.data.with_file_name("backup");
    hold(
        "backup",
        &s.ok(&["backup", target.to_str().expect("path")])[0],
    );
    let new_key = s.data.with_file_name("new.key");
    std::fs::write(&new_key, [0x43; 32]).expect("new key");
    std::fs::set_permissions(
        &new_key,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
    )
    .expect("chmod 0600");
    let rotated = s.ok(&[
        "rekey",
        "master",
        "--new-master",
        new_key.to_str().expect("path"),
    ]);
    hold("rekey master", &rotated[0]);
    assert!(
        wrong.is_empty(),
        "command-line.md has fallen behind: {wrong:#?}"
    );
}

// Users and memberships.

const NOBODY: &str = "00000000000000000000000000000000";

fn id_of(lines: &[Value]) -> String {
    lines[0]["id"].as_str().expect("an id").to_owned()
}

/// A tenant and a local user.
fn tenant_and_user(s: &Setup) -> (String, String) {
    (
        id_of(&s.ok(&["tenant", "create", "acme"])),
        id_of(&s.ok(&["user", "add", "Night shift lead"])),
    )
}

/// The records of `action` in the log `args` name: target and status.
fn logged(s: &Setup, args: &[&str], action: &str) -> Vec<(Value, Value)> {
    let mut export = vec!["audit", "export"];
    export.extend_from_slice(args);
    s.ok(&export)
        .into_iter()
        .filter(|r| r["action"] == action)
        .map(|r| (r["target"].clone(), r["status"].clone()))
        .collect()
}

#[test]
fn users_are_added_listed_and_deactivated() {
    let s = setup();
    let local = id_of(&s.ok(&["user", "add", "Night shift lead"]));
    assert_eq!(local.len(), 32, "a user id is 32 hex digits: {local}");
    let sso = id_of(&s.ok(&[
        "user",
        "add",
        "Auditor",
        "--issuer",
        "https://idp.example.com",
        "--subject",
        "subject-1",
    ]));
    let listed = s.ok(&["user", "list"]);
    assert_eq!(listed.len(), 2, "one line for each user");
    assert_eq!(
        (&listed[0]["id"], &listed[0]["label"], &listed[0]["issuer"]),
        (
            &Value::from(local.as_str()),
            &Value::from("Night shift lead"),
            &Value::Null
        ),
        "the local user's line"
    );
    assert_eq!(
        (&listed[1]["issuer"], &listed[1]["subject"]),
        (
            &Value::from("https://idp.example.com"),
            &Value::from("subject-1")
        ),
        "the other user's identity"
    );
    assert_eq!(
        s.ok(&["user", "deactivate", &sso]),
        Vec::<Value>::new(),
        "deactivate prints nothing"
    );
    let listed = s.ok(&["user", "list"]);
    assert!(
        listed[1]["deactivated_at"].is_string() && listed[0]["deactivated_at"].is_null(),
        "only the deactivated user has a time: {listed:?}"
    );
}

#[test]
fn memberships_are_granted_and_revoked_and_each_prints_the_roles_held() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    let granted = s.ok(&[
        "membership",
        "grant",
        &user,
        "--tenant",
        &tenant,
        "--role",
        "legal",
        "--role",
        "reader",
    ]);
    assert_eq!(
        granted,
        [serde_json::json!({"user": user, "tenant": tenant, "roles": ["reader", "legal"]})],
        "the line a grant prints"
    );
    let operator = s.ok(&["membership", "grant", &user, "--operator"]);
    assert_eq!(
        operator,
        [serde_json::json!({"user": user, "tenant": null, "roles": ["operator"]})],
        "the line an operator grant prints"
    );
    let left = s.ok(&[
        "membership",
        "revoke",
        &user,
        "--tenant",
        &tenant,
        "--role",
        "legal",
    ]);
    assert_eq!(
        left[0]["roles"],
        serde_json::json!(["reader"]),
        "what a revoke of one role leaves"
    );
    let none = s.ok(&["membership", "revoke", &user, "--operator"]);
    assert_eq!(
        none[0]["roles"],
        serde_json::json!([]),
        "what a revoke of the membership leaves"
    );
    let listed = s.ok(&["user", "list"]);
    assert_eq!(
        listed[0]["memberships"]
            .as_array()
            .expect("memberships")
            .iter()
            .map(|m| (m["tenant"].clone(), m["roles"].clone()))
            .collect::<Vec<_>>(),
        [(Value::from(tenant), serde_json::json!(["reader"]))],
        "the one membership left"
    );
}

#[test]
fn user_and_membership_commands_are_in_the_admin_log_and_the_tenants() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    s.ok(&[
        "membership",
        "grant",
        &user,
        "--tenant",
        &tenant,
        "--role",
        "auditor",
    ]);
    s.ok(&["user", "list"]);
    s.ok(&["membership", "revoke", &user, "--tenant", &tenant]);
    s.ok(&["user", "deactivate", &user]);
    let grant = [(
        Value::from(format!("{user}/{tenant}/auditor")),
        Value::from(200),
    )];
    let revoke = [(Value::from(format!("{user}/{tenant}")), Value::from(200))];
    let admin = ["--admin"];
    assert_eq!(
        logged(&s, &admin, "user_add"),
        [(Value::from(user.as_str()), Value::from(201))],
        "user add"
    );
    assert_eq!(
        logged(&s, &admin, "user_list"),
        [(Value::Null, Value::from(200))],
        "user list"
    );
    assert_eq!(logged(&s, &admin, "membership_grant"), grant, "the grant");
    assert_eq!(
        logged(&s, &admin, "membership_revoke"),
        revoke,
        "the revoke"
    );
    assert_eq!(
        logged(&s, &admin, "user_deactivate"),
        [(Value::from(user.as_str()), Value::from(204))],
        "user deactivate"
    );
    let own = ["--tenant", tenant.as_str()];
    assert_eq!(
        logged(&s, &own, "membership_grant"),
        grant,
        "the tenant's copy of the grant"
    );
    assert_eq!(
        logged(&s, &own, "membership_revoke"),
        revoke,
        "the tenant's copy of the revoke"
    );
}

#[test]
fn user_and_membership_commands_run_beside_a_server() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    let server = datadir::lock(&s.data, Sharing::Serve).expect("a server");
    let other = id_of(&s.ok(&["user", "add", "Beside"]));
    s.ok(&[
        "membership",
        "grant",
        &user,
        "--tenant",
        &tenant,
        "--role",
        "reader",
    ]);
    s.ok(&["membership", "revoke", &user, "--tenant", &tenant]);
    s.ok(&["user", "deactivate", &other]);
    assert_eq!(s.ok(&["user", "list"]).len(), 2, "both users");
    drop(server);
}

#[test]
fn each_user_and_membership_refusal_exits_1_and_says_why() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    let gone = id_of(&s.ok(&["user", "add", "Leaver"]));
    s.ok(&["user", "deactivate", &gone]);
    s.ok(&[
        "user",
        "add",
        "Taken",
        "--issuer",
        "https://idp.example.com",
        "--subject",
        "s-1",
    ]);
    let t = tenant.as_str();
    for (args, words) in [
        (vec!["user", "add", " outer space"], "a user label is"),
        (
            vec![
                "user",
                "add",
                "L",
                "--issuer",
                "http://idp.example.com",
                "--subject",
                "s",
            ],
            "https://",
        ),
        (
            vec![
                "user",
                "add",
                "L",
                "--issuer",
                "https://idp.example.com",
                "--subject",
                "s-1",
            ],
            "an active user has this issuer and subject",
        ),
        (vec!["user", "deactivate", NOBODY], "no user has this id"),
        (
            vec!["user", "deactivate", "not-an-id"],
            "no user has this id",
        ),
        (
            vec![
                "membership",
                "grant",
                NOBODY,
                "--tenant",
                t,
                "--role",
                "reader",
            ],
            "no user has this id",
        ),
        (
            vec![
                "membership",
                "grant",
                &user,
                "--tenant",
                NOBODY,
                "--role",
                "reader",
            ],
            "no tenant has this id",
        ),
        (
            vec![
                "membership",
                "grant",
                &user,
                "--tenant",
                "nope",
                "--role",
                "reader",
            ],
            "no tenant has this id",
        ),
        (
            vec![
                "membership",
                "grant",
                &user,
                "--tenant",
                t,
                "--role",
                "owner",
            ],
            "reader, producer, records, legal, auditor and tenant-admin",
        ),
        (
            vec![
                "membership",
                "grant",
                &user,
                "--tenant",
                t,
                "--role",
                "operator",
            ],
            "reader, producer, records, legal, auditor and tenant-admin",
        ),
        (
            vec![
                "membership",
                "grant",
                &gone,
                "--tenant",
                t,
                "--role",
                "reader",
            ],
            "the user is deactivated",
        ),
        (
            vec!["membership", "grant", &gone, "--operator"],
            "the user is deactivated",
        ),
        (
            vec!["membership", "revoke", &user, "--tenant", t],
            "the user has no such membership",
        ),
        (
            vec!["membership", "revoke", &user, "--operator"],
            "the user has no such membership",
        ),
        (
            vec![
                "membership",
                "revoke",
                &user,
                "--tenant",
                t,
                "--role",
                "owner",
            ],
            "reader, producer, records, legal, auditor and tenant-admin",
        ),
    ] {
        let said = s.refused(&args);
        assert!(said.contains(words), "{args:?}: {said}");
    }
    assert_eq!(
        s.ok(&["user", "list"])
            .iter()
            .map(|u| u["memberships"].as_array().map(Vec::len))
            .collect::<Vec<_>>(),
        [Some(0), Some(0), Some(0)],
        "no refusal made a membership"
    );
    // Text that is no id, and a name that is no role's, are refused before
    // the registry is asked, so they leave no record; a real role in the
    // wrong kind of membership is refused by the registry, and recorded.
    assert_eq!(
        logged(&s, &["--admin"], "user_deactivate"),
        [
            (Value::from(gone.as_str()), Value::from(204)),
            (Value::from(NOBODY), Value::from(404)),
        ],
        "the deactivation, and the refusal of an id no user has"
    );
    assert_eq!(
        logged(&s, &["--admin"], "membership_grant")
            .into_iter()
            .filter(|(_, status)| status.as_u64() == Some(422))
            .collect::<Vec<_>>(),
        [(
            Value::from(format!("{user}/{t}/operator")),
            Value::from(422)
        )],
        "the one refusal of roles the registry saw"
    );
    assert_eq!(
        logged(&s, &["--admin"], "membership_revoke")
            .into_iter()
            .filter(|(_, status)| status.as_u64() == Some(422))
            .count(),
        0,
        "a revoke that names no role's name asks the registry nothing"
    );
}

#[test]
fn a_user_or_membership_command_line_that_does_not_parse_exits_2() {
    for args in [
        vec!["user", "add", "--config", "c"],
        vec![
            "user",
            "add",
            "L",
            "--issuer",
            "https://idp.example.com",
            "--config",
            "c",
        ],
        vec!["user", "add", "L", "--subject", "s", "--config", "c"],
        vec!["user", "deactivate", "--config", "c"],
        vec!["user", "list"],
        vec!["user", "remove", "u", "--config", "c"],
        vec![
            "membership",
            "grant",
            "u",
            "--role",
            "reader",
            "--config",
            "c",
        ],
        vec!["membership", "grant", "u", "--tenant", "t", "--config", "c"],
        vec![
            "membership",
            "grant",
            "u",
            "--tenant",
            "t",
            "--operator",
            "--role",
            "reader",
            "--config",
            "c",
        ],
        vec![
            "membership",
            "grant",
            "u",
            "--operator",
            "--role",
            "operator",
            "--config",
            "c",
        ],
        vec!["membership", "grant", "--operator", "--config", "c"],
        vec!["membership", "revoke", "u", "--config", "c"],
        vec![
            "membership",
            "revoke",
            "u",
            "--tenant",
            "t",
            "--operator",
            "--config",
            "c",
        ],
        vec![
            "membership",
            "revoke",
            "u",
            "--operator",
            "--role",
            "operator",
            "--config",
            "c",
        ],
        vec!["membership", "list", "--config", "c"],
    ] {
        usage_error(&args);
    }
}

#[test]
fn a_role_named_twice_is_held_once_and_an_identity_is_free_after_deactivation() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    let granted = s.ok(&[
        "membership",
        "grant",
        &user,
        "--tenant",
        &tenant,
        "--role",
        "reader",
        "--role",
        "reader",
        "--role",
        "tenant-admin",
    ]);
    assert_eq!(
        granted[0]["roles"],
        serde_json::json!(["reader", "tenant-admin"]),
        "each role once, in the fixed order"
    );
    let identity = [
        "--issuer",
        "https://idp.example.com/realms/a",
        "--subject",
        "s | 1",
    ];
    let mut add = vec!["user", "add", "First"];
    add.extend_from_slice(&identity);
    let first = id_of(&s.ok(&add));
    s.ok(&["user", "deactivate", &first]);
    s.ok(&["user", "deactivate", &first]);
    let again = id_of(&s.ok(&add));
    assert_ne!(first, again, "the identity under a new id");
}

#[test]
fn removing_a_tenant_takes_its_memberships_with_it() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    s.ok(&[
        "membership",
        "grant",
        &user,
        "--tenant",
        &tenant,
        "--role",
        "reader",
    ]);
    s.ok(&["tenant", "remove", &tenant]);
    assert_eq!(
        s.ok(&["user", "list"])[0]["memberships"],
        serde_json::json!([]),
        "the user is left with no membership"
    );
    assert!(
        s.refused(&["membership", "revoke", &user, "--tenant", &tenant])
            .contains("the user has no such membership"),
        "and there is none to revoke"
    );
}

// Roles, scopes and a tenant's role cap.

fn grant<'a>(user: &'a str, tenant: &'a str, role: &'a str) -> [&'a str; 7] {
    [
        "membership",
        "grant",
        user,
        "--tenant",
        tenant,
        "--role",
        role,
    ]
}

#[test]
fn a_role_cap_is_shown_set_and_removed() {
    let s = setup();
    let (tenant, _) = tenant_and_user(&s);
    let none = [serde_json::json!({ "id": tenant, "role_cap": null })];
    assert_eq!(s.ok(&["tenant", "role-cap", &tenant]), none, "a new tenant");
    let capped = [serde_json::json!({ "id": tenant, "role_cap": ["reader", "auditor"] })];
    assert_eq!(
        s.ok(&[
            "tenant",
            "set-role-cap",
            &tenant,
            "--role",
            "auditor",
            "--role",
            "reader",
        ]),
        capped,
        "the roles in the fixed order, however they were named"
    );
    assert_eq!(s.ok(&["tenant", "role-cap", &tenant]), capped, "read back");
    assert_eq!(
        s.ok(&["tenant", "set-role-cap", &tenant, "--all"]),
        none,
        "removed"
    );
    assert_eq!(s.ok(&["tenant", "role-cap", &tenant]), none, "read back");
}

#[test]
fn user_list_shows_each_memberships_scopes_under_the_cap() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    s.ok(&grant(&user, &tenant, "reader"));
    s.ok(&grant(&user, &tenant, "tenant-admin"));
    s.ok(&["membership", "grant", &user, "--operator"]);
    let scopes = |s: &Setup| -> Vec<Value> {
        s.ok(&["user", "list"])[0]["memberships"]
            .as_array()
            .expect("memberships")
            .iter()
            .map(|m| m["scopes"].clone())
            .collect()
    };
    assert_eq!(
        scopes(&s),
        [
            serde_json::json!(["read", "tenant_admin"]),
            serde_json::json!(["admin"])
        ],
        "each role's scope"
    );
    s.ok(&["tenant", "set-role-cap", &tenant, "--role", "reader"]);
    assert_eq!(
        scopes(&s),
        [serde_json::json!(["read"]), serde_json::json!(["admin"])],
        "the cap takes tenant_admin and leaves the operator membership"
    );
    assert_eq!(
        s.ok(&["user", "list"])[0]["memberships"][0]["roles"],
        serde_json::json!(["reader", "tenant-admin"]),
        "the roles granted are kept"
    );
}

#[test]
fn a_grant_outside_the_cap_exits_1_and_says_why() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    s.ok(&["tenant", "set-role-cap", &tenant, "--role", "reader"]);
    assert!(
        s.refused(&grant(&user, &tenant, "legal"))
            .contains("outside the tenant's role cap"),
        "the refusal names the cap"
    );
    assert_eq!(
        s.ok(&grant(&user, &tenant, "reader"))[0]["roles"],
        serde_json::json!(["reader"]),
        "a role in the cap is granted"
    );
}

#[test]
fn each_role_cap_refusal_exits_1_and_says_why() {
    let s = setup();
    let (tenant, _) = tenant_and_user(&s);
    let t = tenant.as_str();
    for (args, said) in [
        (
            vec!["tenant", "set-role-cap", t, "--role", "operator"],
            "a role cap is one or more of reader, producer, records, legal, auditor and tenant-admin",
        ),
        (
            vec!["tenant", "set-role-cap", t, "--role", "owner"],
            "a role cap is one or more of",
        ),
        (
            vec!["tenant", "set-role-cap", t, "--role", "tenant_admin"],
            "a role cap is one or more of",
        ),
        (
            vec!["tenant", "set-role-cap", NOBODY, "--role", "reader"],
            "no tenant has this id",
        ),
        (
            vec!["tenant", "set-role-cap", NOBODY, "--all"],
            "no tenant has this id",
        ),
        (
            vec!["tenant", "set-role-cap", "nope", "--all"],
            "no tenant has this id",
        ),
        (vec!["tenant", "role-cap", NOBODY], "no tenant has this id"),
        (vec!["tenant", "role-cap", "nope"], "no tenant has this id"),
    ] {
        let message = s.refused(&args);
        assert!(message.contains(said), "{args:?}: {message}");
    }
    assert_eq!(
        s.ok(&["tenant", "role-cap", t]),
        [serde_json::json!({ "id": t, "role_cap": null })],
        "no refused command set a cap"
    );
}

#[test]
fn a_role_cap_command_line_that_does_not_parse_exits_2() {
    for args in [
        vec!["tenant", "set-role-cap", "t", "--config", "c"],
        vec!["tenant", "set-role-cap", "--all", "--config", "c"],
        vec![
            "tenant",
            "set-role-cap",
            "t",
            "--all",
            "--role",
            "reader",
            "--config",
            "c",
        ],
        vec!["tenant", "set-role-cap", "t", "--role", "--config", "c"],
        vec!["tenant", "set-role-cap", "t", "--all"],
        vec!["tenant", "role-cap", "--config", "c"],
        vec![
            "tenant", "role-cap", "t", "--role", "reader", "--config", "c",
        ],
        vec!["tenant", "role-cap", "t"],
    ] {
        let said = usage_error(&args);
        assert!(said.starts_with("error:"), "{args:?}: {said}");
    }
}

#[test]
fn role_cap_commands_are_in_the_admin_log_and_a_change_in_the_tenants() {
    let s = setup();
    let (tenant, _) = tenant_and_user(&s);
    s.ok(&["tenant", "set-role-cap", &tenant, "--role", "legal"]);
    s.ok(&["tenant", "role-cap", &tenant]);
    s.ok(&["tenant", "set-role-cap", &tenant, "--all"]);
    s.refused(&["tenant", "set-role-cap", &tenant, "--role", "operator"]);
    let set = [
        (
            serde_json::json!(format!("{tenant}/legal")),
            serde_json::json!(200),
        ),
        (
            serde_json::json!(format!("{tenant}/all")),
            serde_json::json!(200),
        ),
    ];
    let mut all = set.to_vec();
    all.push((
        serde_json::json!(format!("{tenant}/operator")),
        serde_json::json!(422),
    ));
    assert_eq!(
        logged(&s, &["--admin"], "role_cap_set"),
        all,
        "the admin log, with the refused cap"
    );
    assert_eq!(
        logged(&s, &["--tenant", &tenant], "role_cap_set"),
        set,
        "the tenant's log"
    );
    assert_eq!(
        logged(&s, &["--admin"], "role_cap_read"),
        [(serde_json::json!(tenant), serde_json::json!(200))],
        "the read, in the admin log"
    );
    assert_eq!(
        logged(&s, &["--tenant", &tenant], "role_cap_read"),
        [],
        "and not in the tenant's"
    );
}

#[test]
fn role_cap_commands_run_beside_a_server() {
    let s = setup();
    let (tenant, user) = tenant_and_user(&s);
    let server = datadir::lock(&s.data, Sharing::Serve).expect("a server");
    s.ok(&["tenant", "set-role-cap", &tenant, "--role", "reader"]);
    assert_eq!(
        s.ok(&["tenant", "role-cap", &tenant])[0]["role_cap"],
        serde_json::json!(["reader"]),
        "set and read beside a server"
    );
    assert!(
        s.refused(&grant(&user, &tenant, "records"))
            .contains("outside the tenant's role cap"),
        "and it holds at once"
    );
    drop(server);
}

#[test]
fn a_tenant_key_may_hold_tenant_admin_and_the_roles_spelling_is_refused() {
    let s = setup();
    let (tenant, _) = tenant_and_user(&s);
    s.ok(&[
        "key",
        "create",
        "--tenant",
        &tenant,
        "--scope",
        "tenant_admin",
        "--scope",
        "read",
        "--label",
        "admin",
    ]);
    assert_eq!(
        s.ok(&["key", "list"])[0]["scopes"],
        serde_json::json!(["read", "tenant_admin"]),
        "the scopes the key holds"
    );
    assert!(
        s.refused(&[
            "key",
            "create",
            "--tenant",
            &tenant,
            "--scope",
            "tenant-admin",
            "--label",
            "x",
        ])
        .contains("tenant_admin"),
        "the role's spelling is refused, and the message names the scope"
    );
}
