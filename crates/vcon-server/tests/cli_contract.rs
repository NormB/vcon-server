//! The command line's part of the contract (website/content/docs/stability.md):
//! contract/cli.json holds every command, each flag and argument it takes,
//! whether it is required, the value it takes, its default and its possible
//! values, all read from the binary's `--help`, and the exit codes
//! command-line.md lists. `scripts/contract_diff.py --kind cli` compares two
//! of these files.
//!
//! The snapshot is written, never edited by hand: when the binary's command
//! line changes, [`REGENERATE`] writes it again, and the change is reviewed
//! in the diff. The prose of `--help` is not in it.
//!
//! Each exit code the page lists is also the code of one command run here,
//! and each code run here is on the page.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Command;

#[path = "support/cli_help.rs"]
mod cli_help;

mod contract_snapshot {
    include!("support/contract_snapshot.rs");
}

use cli_help::{Help, Opt, Positional, commands, help, leaves, parse_help};

/// The variable that makes [`the_snapshot_is_what_help_shows`] write the
/// snapshot, named as `VCON_SERVER_WRITE_SCREENS` is for the TUI's screens.
const WRITE: &str = "VCON_SERVER_WRITE_CONTRACT";

/// The one command that writes contract/cli.json again.
const REGENERATE: &str =
    "VCON_SERVER_WRITE_CONTRACT=1 cargo test -p vcon-server --test cli_contract";

/// The snapshot, from the repository root.
const SNAPSHOT: &str = "contract/cli.json";

/// One command for each exit code command-line.md lists, and the code it
/// exits with. A missing configuration file is refused (1); a command
/// without its required `--config` does not parse (2).
const RUNS: &[(&str, &[&str])] = &[
    ("0", &["--version"]),
    ("1", &["check", "--config", "no-such-config.toml"]),
    ("2", &["serve"]),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn command_line_page() -> String {
    std::fs::read_to_string(root().join("website/content/docs/command-line.md"))
        .expect("command-line.md")
}

/// The exit codes of the page's "Output and exit codes" section, each with
/// its meaning: the text after `**N:**` up to its first `.` or `,`.
fn exit_codes(page: &str) -> BTreeMap<String, String> {
    let Some((_, section)) = page.split_once("\n## Output and exit codes\n").or_else(|| {
        page.strip_prefix("## Output and exit codes\n")
            .map(|rest| ("", rest))
    }) else {
        return BTreeMap::new();
    };
    let section = section.split("\n## ").next().unwrap_or(section);
    // Each bullet with its wrapped lines joined.
    let mut bullets: Vec<String> = Vec::new();
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("- ") {
            bullets.push(rest.to_owned());
        } else if let (Some(last), Some(rest)) = (bullets.last_mut(), line.strip_prefix("  ")) {
            last.push(' ');
            last.push_str(rest.trim());
        }
    }
    bullets
        .iter()
        .filter_map(|b| {
            let (code, rest) = b.strip_prefix("**")?.split_once(":**")?;
            code.parse::<u8>().ok()?;
            let meaning = rest.trim().split(['.', ',']).next().unwrap_or("").trim();
            Some((code.to_owned(), meaning.to_owned()))
        })
        .collect()
}

/// One flag as the snapshot holds it.
fn flag_json(o: &Opt) -> serde_json::Value {
    serde_json::json!({
        "short": o.short,
        "value": o.value,
        "required": o.required,
        "repeated": o.repeated,
        "default": o.default,
        "values": o.values,
    })
}

/// One argument as the snapshot holds it.
fn argument_json(a: &Positional) -> serde_json::Value {
    serde_json::json!({
        "name": a.name,
        "required": a.required,
        "repeated": a.repeated,
        "default": a.default,
        "values": a.values,
    })
}

/// contract/cli.json's text for `commands` (each command's words after
/// `vcon-server` and its `--help`) and `codes`. Every object's members are
/// in name order, so one binary has one text.
fn render(commands: &[(Vec<String>, Help)], codes: &BTreeMap<String, String>) -> String {
    let mut all = BTreeMap::new();
    for (path, h) in commands {
        let name = std::iter::once("vcon-server")
            .chain(path.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        let flags: BTreeMap<&str, serde_json::Value> = h
            .options
            .iter()
            .map(|o| (o.key.as_str(), flag_json(o)))
            .collect();
        let arguments: Vec<serde_json::Value> = h.arguments.iter().map(argument_json).collect();
        all.insert(
            name,
            serde_json::json!({ "arguments": arguments, "flags": flags }),
        );
    }
    let snapshot = serde_json::json!({ "commands": all, "exit_codes": codes });
    let mut text = serde_json::to_string_pretty(&snapshot).expect("JSON");
    text.push('\n');
    text
}

/// The message of a snapshot that is not what the binary shows: the first
/// line that differs, and the command that writes it again.
fn stale(file: &str, now: &str) -> String {
    contract_snapshot::stale(SNAPSHOT, "the binary's --help shows", REGENERATE, file, now)
}

/// Each code the page lists that [`RUNS`] does not run, and each code it
/// runs that the page does not list.
fn unmatched(page: &BTreeMap<String, String>, runs: &[(&str, &[&str])]) -> Vec<String> {
    let mut found: Vec<String> = runs
        .iter()
        .filter(|(code, _)| !page.contains_key(*code))
        .map(|(code, _)| format!("exit {code} is run here but command-line.md does not list it"))
        .collect();
    found.extend(
        page.keys()
            .filter(|code| !runs.iter().any(|(c, _)| c == code))
            .map(|code| {
                format!("exit {code} is on command-line.md but no command here exits with it")
            }),
    );
    found.sort();
    found
}

/// The snapshot of this build.
fn snapshot_now() -> String {
    let mut cache = HashMap::new();
    let found: Vec<(Vec<String>, Help)> = commands(Vec::new(), &mut cache)
        .into_iter()
        .map(|path| {
            let h = help(&path, &mut cache).unwrap_or_else(|e| panic!("{e}"));
            (path, h)
        })
        .collect();
    render(&found, &exit_codes(&command_line_page()))
}

#[test]
fn the_snapshot_is_what_help_shows() {
    let now = snapshot_now();
    let path = root().join(SNAPSHOT);
    if std::env::var_os(WRITE).is_some() {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("contract/");
        std::fs::write(&path, &now).expect("the snapshot is written");
        return;
    }
    let file = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(file == now, "{}", stale(&file, &now));
}

#[test]
fn each_exit_code_on_the_page_is_the_code_of_its_command() {
    let page = exit_codes(&command_line_page());
    assert_eq!(
        unmatched(&page, RUNS),
        Vec::<String>::new(),
        "command-line.md lists exactly the codes this test runs"
    );
    let dir = tempfile::tempdir().expect("a directory");
    for (code, args) in RUNS {
        let out = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
            .args(*args)
            .current_dir(dir.path())
            .output()
            .expect("the binary runs");
        assert_eq!(
            out.status.code().map(|c| c.to_string()).as_deref(),
            Some(*code),
            "`vcon-server {}` exits {code}, as command-line.md says ({}): {}",
            args.join(" "),
            page.get(*code).map_or("not listed", String::as_str),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn a_refused_command_says_why_on_standard_error_and_prints_nothing() {
    let dir = tempfile::tempdir().expect("a directory");
    let out = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .args(RUNS[1].1)
        .current_dir(dir.path())
        .output()
        .expect("the binary runs");
    assert!(
        out.stdout.is_empty(),
        "exit 1 prints nothing on standard output, as command-line.md says"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("vcon-server: "),
        "exit 1 says why as `vcon-server: <reason>`: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn an_unknown_flag_does_not_parse() {
    let out = Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .args(["check", "--config", "c.toml", "--colour"])
        .output()
        .expect("the binary runs");
    assert_eq!(
        out.status.code(),
        Some(2),
        "a flag the command lacks exits 2"
    );
}

#[test]
fn the_snapshot_holds_the_exit_codes_of_the_page() {
    // The text this build writes, not the file: the file is that text
    // (the_snapshot_is_what_help_shows), and a run that writes the file
    // must not read it at the same time.
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot_now()).expect("JSON");
    let codes: BTreeMap<String, String> =
        serde_json::from_value(snapshot["exit_codes"].clone()).expect("an object of strings");
    assert_eq!(
        codes,
        exit_codes(&command_line_page()),
        "the snapshot's exit codes are the page's"
    );
}

#[test]
fn the_page_lists_0_1_and_2() {
    let codes = exit_codes(&command_line_page());
    assert_eq!(
        codes
            .iter()
            .map(|(c, m)| (c.as_str(), m.as_str()))
            .collect::<Vec<_>>(),
        [
            ("0", "done"),
            ("1", "refused or failed"),
            ("2", "the command line does not parse")
        ],
        "the three codes and their meanings, read from command-line.md"
    );
}

#[test]
fn every_command_is_walked_root_first_without_help() {
    let mut cache = HashMap::new();
    let all = commands(Vec::new(), &mut cache);
    assert_eq!(all.first(), Some(&Vec::new()), "the root comes first");
    assert!(
        !all.iter().any(|p| p.last().is_some_and(|w| w == "help")),
        "clap's `help` subcommand is not a command of the contract: {all:?}"
    );
    for leaf in leaves(Vec::new(), &mut cache) {
        assert!(all.contains(&leaf), "{leaf:?} is walked");
    }
    for group in [["tenant"], ["key"], ["audit"]] {
        let group: Vec<String> = group.iter().map(|w| (*w).to_owned()).collect();
        assert!(
            all.contains(&group),
            "the group {group:?} is walked as well as its commands"
        );
    }
}

#[test]
fn rendering_twice_gives_the_same_text() {
    let help = parse_help(TENANT_REMOVE);
    let codes = exit_codes(PAGE);
    let one = render(
        &[(vec!["tenant".into(), "remove".into()], help.clone())],
        &codes,
    );
    let two = render(&[(vec!["tenant".into(), "remove".into()], help)], &codes);
    assert_eq!(one, two, "the snapshot has one text for one binary");
    assert!(
        one.ends_with("}\n"),
        "the snapshot ends in a newline: {one:?}"
    );
}

#[test]
fn the_rendered_snapshot_is_the_shape_contract_diff_reads() {
    let text = render(
        &[
            (Vec::new(), parse_help(ROOT)),
            (
                vec!["tenant".into(), "remove".into()],
                parse_help(TENANT_REMOVE),
            ),
        ],
        &exit_codes(PAGE),
    );
    let v: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let remove = &v["commands"]["vcon-server tenant remove"];
    assert_eq!(
        remove["arguments"],
        serde_json::json!([{"name": "ID", "required": true, "repeated": false, "default": null, "values": null}]),
        "an argument has its five members"
    );
    assert_eq!(
        remove["flags"]["--config"],
        serde_json::json!({"short": null, "value": "<CONFIG>", "required": true, "repeated": false,
                           "default": null, "values": null}),
        "a flag has its six members"
    );
    assert_eq!(
        v["commands"]["vcon-server"]["flags"]["--version"]["short"], "-V",
        "the root is `vcon-server`"
    );
    assert_eq!(
        v["exit_codes"]["2"], "the command line does not parse",
        "exit codes by code"
    );
}

#[test]
fn a_flags_default_and_values_are_in_the_rendered_snapshot() {
    let help = parse_help(
        "Usage: vcon-server x [OPTIONS]\n\nOptions:\n      --view <VIEW>  V [default: full] [possible values: full, masked]\n",
    );
    let v: serde_json::Value =
        serde_json::from_str(&render(&[(vec!["x".into()], help)], &BTreeMap::new())).expect("JSON");
    let view = &v["commands"]["vcon-server x"]["flags"]["--view"];
    assert_eq!(view["default"], "full", "the default is held: {view}");
    assert_eq!(
        view["values"],
        serde_json::json!(["full", "masked"]),
        "the values are held: {view}"
    );
}

#[test]
fn a_stale_snapshot_names_the_command_that_writes_it() {
    let message = stale("{\n  \"a\": 1\n}\n", "{\n  \"a\": 2\n}\n");
    assert!(
        message.contains(REGENERATE),
        "the one command is named: {message}"
    );
    assert!(message.contains(SNAPSHOT), "the file is named: {message}");
    assert!(
        message.contains("line 2") && message.contains("\"a\": 1") && message.contains("\"a\": 2"),
        "the first line that differs is shown, both sides: {message}"
    );
}

#[test]
fn a_missing_snapshot_names_the_command_that_writes_it() {
    let message = stale("", "{\n}\n");
    assert!(
        message.contains(REGENERATE),
        "the one command is named: {message}"
    );
    assert!(
        message.contains("line 1"),
        "the first line differs: {message}"
    );
}

#[test]
fn a_snapshot_with_a_line_too_many_is_stale_at_that_line() {
    let message = stale("{\n}\nx\n", "{\n}\n");
    assert!(
        message.contains("line 3"),
        "the extra line is named: {message}"
    );
}

#[test]
fn a_code_listed_but_not_run_or_run_but_not_listed_is_named() {
    let page: BTreeMap<String, String> = [("0", "done"), ("3", "busy")]
        .into_iter()
        .map(|(c, m)| (c.to_owned(), m.to_owned()))
        .collect();
    let found = unmatched(&page, &[("0", &["--version"]), ("2", &["serve"])]);
    assert_eq!(
        found,
        [
            "exit 2 is run here but command-line.md does not list it",
            "exit 3 is on command-line.md but no command here exits with it"
        ],
        "both directions are named"
    );
}

#[test]
fn a_page_without_the_section_lists_no_code() {
    assert!(
        exit_codes("# Commands\n\n- **0:** done.\n").is_empty(),
        "codes are read from the Output and exit codes section only"
    );
}

#[test]
fn a_meaning_wrapped_over_two_lines_is_read_whole() {
    let page = "## Output and exit codes\n\n- **1:** refused\n  or failed. More.\n- **2:** bad, such as x.\n\n## Next\n\n- **9:** not here.\n";
    let codes = exit_codes(page);
    assert_eq!(
        codes.get("1").map(String::as_str),
        Some("refused or failed"),
        "{codes:?}"
    );
    assert_eq!(
        codes.get("2").map(String::as_str),
        Some("bad"),
        "a comma ends the meaning: {codes:?}"
    );
    assert!(
        !codes.contains_key("9"),
        "the next section is not read: {codes:?}"
    );
}

// Planted `--help` texts, in clap's two layouts.

const ROOT: &str = "A vCon server\n\nUsage: vcon-server <COMMAND>\n\nCommands:\n  serve   Starts the server\n  tenant  Tenants\n  help    Print this message\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n";

const TENANT_REMOVE: &str = "Removes a tenant\n\nUsage: vcon-server tenant remove [OPTIONS] --config <CONFIG> <ID>\n\nArguments:\n  <ID>  The tenant's id\n\nOptions:\n      --config <CONFIG>  The configuration file\n      --force            Remove its vCons too\n      --override-holds   Remove its vCons even under a legal hold\n  -h, --help             Print help\n";

const PAGE: &str = "## Output and exit codes\n\n- **0:** done.\n- **1:** refused or failed. Nothing.\n- **2:** the command line does not parse, such as a missing `--config`.\n";

fn opt<'a>(h: &'a Help, key: &str) -> &'a Opt {
    h.options
        .iter()
        .find(|o| o.key == key)
        .unwrap_or_else(|| panic!("{key} is read: {:?}", h.options))
}

#[test]
fn a_required_flag_is_outside_the_brackets_of_the_usage_line() {
    let h = parse_help(TENANT_REMOVE);
    assert!(
        opt(&h, "--config").required,
        "--config is outside [OPTIONS]"
    );
    assert!(
        !opt(&h, "--force").required,
        "--force is one of the [OPTIONS]"
    );
    assert_eq!(
        opt(&h, "--config").value.as_deref(),
        Some("<CONFIG>"),
        "its value is read"
    );
    assert_eq!(opt(&h, "--force").value, None, "a switch takes no value");
}

#[test]
fn short_and_long_names_are_read() {
    let h = parse_help(ROOT);
    assert_eq!(
        opt(&h, "--version").short.as_deref(),
        Some("-V"),
        "-V, --version"
    );
    assert_eq!(opt(&h, "--help").short.as_deref(), Some("-h"), "-h, --help");
    assert_eq!(
        h.subcommands,
        ["serve", "tenant", "help"],
        "the commands are read"
    );
}

#[test]
fn arguments_are_read_in_order() {
    let h = parse_help(TENANT_REMOVE);
    assert_eq!(
        h.arguments,
        [Positional {
            name: "ID".into(),
            required: true,
            ..Positional::default()
        }],
        "<ID> is one required argument"
    );
}

#[test]
fn a_default_and_possible_values_are_read() {
    let text = "Usage: vcon-server x [OPTIONS]\n\nOptions:\n      --days <DAYS>  Days [default: 90]\n      --view <VIEW>  The view [default: full] [possible values: full, masked]\n";
    let h = parse_help(text);
    assert_eq!(
        opt(&h, "--days").default.as_deref(),
        Some("90"),
        "[default: 90]"
    );
    assert_eq!(opt(&h, "--days").values, None, "no list of values");
    assert_eq!(
        opt(&h, "--view").values,
        Some(vec!["full".to_owned(), "masked".to_owned()]),
        "[possible values: full, masked]"
    );
    assert_eq!(
        opt(&h, "--view").default.as_deref(),
        Some("full"),
        "a default beside the values"
    );
}

#[test]
fn nested_brackets_on_the_usage_line_make_no_flag_required() {
    let text = "Usage: vcon-server x --config <C> [--force [--override-holds]]\n\nOptions:\n      --config <C>      C\n      --force           F\n      --override-holds  O\n";
    let h = parse_help(text);
    assert!(opt(&h, "--config").required, "outside the brackets");
    assert!(!opt(&h, "--force").required, "inside one pair");
    assert!(!opt(&h, "--override-holds").required, "inside two pairs");
}

#[test]
fn a_flag_in_the_middle_of_a_bracket_pair_is_not_required() {
    let text = "Usage: vcon-server x [--since <S> --admin <A>] --config <C>\n\nOptions:\n      --since <S>   S\n      --admin <A>   A\n      --config <C>  C\n";
    let h = parse_help(text);
    assert!(
        !opt(&h, "--admin").required,
        "--admin is inside the pair that --since opens"
    );
    assert!(
        opt(&h, "--config").required,
        "--config is after the pair closes"
    );
}

#[test]
fn optional_and_repeated_arguments_are_read() {
    let text = "Usage: vcon-server x [NAME] <FILES>...\n\nArguments:\n  [NAME]      A name\n  <FILES>...  Files\n";
    let h = parse_help(text);
    assert_eq!(
        h.arguments,
        [
            Positional {
                name: "NAME".into(),
                ..Positional::default()
            },
            Positional {
                name: "FILES".into(),
                required: true,
                repeated: true,
                ..Positional::default()
            }
        ],
        "[NAME] may be left out; <FILES>... is required and repeated"
    );
}

#[test]
fn an_arguments_default_and_possible_values_are_read() {
    let text = "Usage: vcon-server x [METADATA]\n\nArguments:\n  [METADATA]  What the files hold [default: none] [possible values: none, full]\n";
    let h = parse_help(text);
    assert_eq!(
        h.arguments,
        [Positional {
            name: "METADATA".into(),
            default: Some("none".into()),
            values: Some(vec!["none".into(), "full".into()]),
            ..Positional::default()
        }],
        "an argument's [default: ] and [possible values: ]"
    );
}

#[test]
fn an_optional_or_repeated_value_is_read() {
    let text = "Usage: vcon-server x [OPTIONS]\n\nOptions:\n      --color [<WHEN>]  When\n      --tag <TAG>...    Tags\n";
    let h = parse_help(text);
    assert_eq!(
        opt(&h, "--color").value.as_deref(),
        Some("[<WHEN>]"),
        "a value that may be left out"
    );
    assert!(!opt(&h, "--color").repeated, "one value");
    assert!(
        opt(&h, "--tag").repeated,
        "<TAG>... may be given more than once"
    );
}

#[test]
fn the_long_layout_reads_a_default_on_a_later_line() {
    let text = "Usage: vcon-server x [OPTIONS]\n\nOptions:\n      --days <DAYS>\n          The days until it expires.\n\n          [default: 90]\n\n  -h, --help\n          Print help (see a summary with '-h')\n";
    let h = parse_help(text);
    assert_eq!(
        opt(&h, "--days").default.as_deref(),
        Some("90"),
        "the default two lines down"
    );
    assert_eq!(
        opt(&h, "--help").default,
        None,
        "the next entry starts afresh"
    );
}

#[test]
fn a_list_of_possible_values_is_read() {
    let text = "Usage: vcon-server x [OPTIONS]\n\nOptions:\n      --metadata <METADATA>\n          What the files hold\n\n          Possible values:\n          - none:   No clear columns\n          - counts: Counts\n          - full\n\n  -h, --help\n          Print help\n";
    let h = parse_help(text);
    assert_eq!(
        opt(&h, "--metadata").values,
        Some(vec![
            "none".to_owned(),
            "counts".to_owned(),
            "full".to_owned()
        ]),
        "each `- value` line, with or without its text"
    );
    assert_eq!(
        opt(&h, "--help").values,
        None,
        "the list ends at the next entry"
    );
}

#[test]
fn a_flag_with_no_long_name_is_keyed_by_its_short_name() {
    let text =
        "Usage: vcon-server x [OPTIONS]\n\nOptions:\n  -q          Quiet\n  -n <COUNT>  Count\n";
    let h = parse_help(text);
    assert_eq!(opt(&h, "-q").short.as_deref(), Some("-q"), "-q alone");
    assert_eq!(
        opt(&h, "-n").value.as_deref(),
        Some("<COUNT>"),
        "-n with a value"
    );
}

#[test]
fn flags_under_any_heading_are_read() {
    let text = "Usage: vcon-server x [OPTIONS]\n\nStorage:\n      --to <TO>  Where\n\nOptions:\n  -h, --help  Print help\n";
    let h = parse_help(text);
    assert_eq!(
        opt(&h, "--to").value.as_deref(),
        Some("<TO>"),
        "a heading of clap's next_help_heading"
    );
    assert_eq!(h.options.len(), 2, "both entries: {:?}", h.options);
}

#[test]
fn description_text_is_not_read_as_a_flag() {
    let text = "Usage: vcon-server x [OPTIONS]\n\nOptions:\n      --expires-days <DAYS>  Without this and without --no-expiry it is 90 days\n      --no-expiry            Never\n";
    let h = parse_help(text);
    assert_eq!(
        h.options.iter().map(|o| o.key.as_str()).collect::<Vec<_>>(),
        ["--expires-days", "--no-expiry"],
        "a flag named in a description is not an entry"
    );
    assert!(
        h.flags.contains(&"--no-expiry".to_owned()),
        "the documented-commands reader still finds it"
    );
}
