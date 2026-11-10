//! `vcon-server tui` as a process with no terminal: it needs one, says so
//! in words a reader can act on, and reads no secret and draws nothing
//! before it finds out.
#![cfg(feature = "tui")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A client file in `dir` naming a key file there, which exists (mode
/// 0600) only if `with_key` says so.
fn client_file(dir: &Path, with_key: bool) -> PathBuf {
    let key_file = dir.join("tui.token");
    if with_key {
        std::fs::write(&key_file, "vcs_0011223344556677_secret\n").expect("write the key file");
        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    let config = dir.join("tui.toml");
    std::fs::write(
        &config,
        format!(
            "url = \"https://vcon.example.com:8443\"\nkey_file = \"{}\"\n",
            key_file.display()
        ),
    )
    .expect("write the client file");
    config
}

/// `vcon-server tui --config <config>` with standard input from
/// `/dev/null` and standard output and error as pipes: no terminal.
fn tui(config: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .arg("tui")
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .output()
        .expect("vcon-server runs")
}

fn said(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn without_a_terminal_it_says_that_a_terminal_is_required() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&client_file(dir.path(), true));
    assert_eq!(out.status.code(), Some(1), "it fails: {}", said(&out));
    assert!(
        said(&out).starts_with("vcon-server: a terminal is required: "),
        "what is missing, first: {}",
        said(&out)
    );
}

#[test]
fn the_message_has_no_operating_system_error_text() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&client_file(dir.path(), true));
    let said = said(&out);
    assert!(
        !said.contains("os error") && !said.contains("No such device"),
        "ENXIO from opening /dev/tty tells a reader nothing: {said}"
    );
}

#[test]
fn the_message_says_what_to_do() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&client_file(dir.path(), true));
    let said = said(&out);
    assert!(
        said.contains("run it in a terminal"),
        "the way out is named: {said}"
    );
}

#[test]
fn the_message_names_what_is_not_a_terminal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&client_file(dir.path(), true));
    let said = said(&out);
    assert!(
        said.contains("standard input and standard output are not one"),
        "both were redirected here: {said}"
    );
}

#[test]
fn the_terminal_is_checked_before_the_key_file_is_read() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&client_file(dir.path(), false));
    let said = said(&out);
    assert!(
        said.contains("a terminal is required") && !said.contains("key_file"),
        "no secret is read for a run that cannot start: {said}"
    );
}

#[test]
fn a_missing_client_file_is_still_reported_first() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&dir.path().join("missing.toml"));
    assert_eq!(out.status.code(), Some(1), "it fails: {}", said(&out));
    assert!(
        said(&out).starts_with("vcon-server: client file: "),
        "the client file is read before anything else: {}",
        said(&out)
    );
}

#[test]
fn nothing_is_written_to_a_standard_output_that_is_not_a_terminal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&client_file(dir.path(), true));
    assert!(
        out.stdout.is_empty(),
        "no escape sequence reaches a pipe or a file: {:?}",
        out.stdout
    );
}

#[test]
fn the_message_is_one_line() {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = tui(&client_file(dir.path(), true));
    let said = said(&out);
    assert_eq!(
        said.lines().count(),
        1,
        "one line on standard error, ended by a newline: {said:?}"
    );
    assert!(said.ends_with('\n'), "ended by a newline: {said:?}");
}
