//! The certificate the getting-started page makes is one the terminal UI
//! accepts. `openssl req -x509` marks a certificate as a certificate
//! authority unless told otherwise, and the UI's TLS library refuses such
//! a certificate from a server, so the page's command must say `CA:FALSE`
//! and the page must say why.

use std::path::Path;
use std::process::Command;

fn page() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../website/content/docs/getting-started.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The page's command that makes the self-signed certificate: from the
/// line that starts it to the line that names its output, joined as the
/// shell joins them.
fn certificate_command(page: &str) -> String {
    let lines: Vec<&str> = page
        .lines()
        .skip_while(|line| !line.starts_with("openssl req -x509 "))
        .collect();
    let last = lines
        .iter()
        .position(|line| !line.ends_with('\\'))
        .expect("the command ends");
    lines[..=last]
        .iter()
        .map(|line| line.trim_end_matches('\\').trim())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_pages_command_marks_the_certificate_as_not_a_certificate_authority() {
    let command = certificate_command(&page());
    assert!(
        command.contains("-addext basicConstraints=critical,CA:FALSE"),
        "the page's openssl command holds the option that makes it a server's certificate"
    );
}

#[test]
fn the_certificate_the_pages_command_makes_is_not_a_certificate_authority() {
    let dir = tempfile::tempdir().expect("temp dir");
    let made = Command::new("sh")
        .arg("-c")
        .arg(certificate_command(&page()))
        .current_dir(dir.path())
        .output()
        .expect("sh runs");
    assert!(made.status.success(), "the page's command ends 0");
    let read = Command::new("openssl")
        .args(["x509", "-in", "fullchain.pem", "-noout", "-ext"])
        .arg("basicConstraints")
        .current_dir(dir.path())
        .output()
        .expect("openssl runs");
    let said = String::from_utf8_lossy(&read.stdout);
    assert!(
        said.contains("CA:FALSE") && !said.contains("CA:TRUE"),
        "what openssl made: {said}"
    );
}

#[test]
fn the_page_says_that_the_terminal_ui_refuses_a_certificate_authoritys_certificate() {
    let page = page().replace('\n', " ");
    assert!(
        page.contains(
            "`vcon-server mcp` and `vcon-server tui`, which are built on rustls, refuse such a certificate from a server"
        ),
        "the reason for the options names both commands that refuse it"
    );
}

#[test]
fn a_command_of_one_line_is_read_whole() {
    assert_eq!(
        certificate_command("text\nopenssl req -x509 -out a.pem\nnext\n"),
        "openssl req -x509 -out a.pem",
        "the line after it is not part of it"
    );
}
