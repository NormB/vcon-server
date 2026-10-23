use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_vcon-server"))
        .args(args)
        .output()
        .expect("the binary runs")
}

#[test]
fn version_flag_prints_name_and_version() {
    let out = run(&["--version"]);
    assert!(out.status.success(), "exit status {:?}", out.status);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        concat!("vcon-server ", env!("CARGO_PKG_VERSION"), "\n")
    );
}

#[test]
fn unknown_arguments_are_refused_with_usage() {
    let out = run(&["--no-such-flag"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: vcon-server --version"));
}
