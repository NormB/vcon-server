//! The rule `vcon-server tui` applies before it starts: standard input and
//! standard output must both be a terminal. The rule is a function of the
//! two facts, so each combination is tested without a terminal.
#![cfg(feature = "tui")]

use vcon_server::tui::require_terminal;

#[test]
fn a_terminal_on_both_sides_is_accepted() {
    assert_eq!(
        require_terminal(true, true),
        Ok(()),
        "keys in, the screen out"
    );
}

#[test]
fn a_redirected_standard_input_is_named() {
    let refused = require_terminal(false, true).expect_err("no keys can be read");
    assert!(
        refused.starts_with("a terminal is required: ")
            && refused.contains("standard input is not one"),
        "{refused}"
    );
    assert!(
        !refused.contains("standard output"),
        "only what is wrong: {refused}"
    );
}

#[test]
fn a_redirected_standard_output_is_named() {
    let refused = require_terminal(true, false).expect_err("no screen can be drawn");
    assert!(
        refused.starts_with("a terminal is required: ")
            && refused.contains("standard output is not one"),
        "{refused}"
    );
    assert!(
        !refused.contains("standard input"),
        "only what is wrong: {refused}"
    );
}

#[test]
fn both_redirected_are_both_named() {
    let refused = require_terminal(false, false).expect_err("neither is a terminal");
    assert!(
        refused.contains("standard input and standard output are not one"),
        "{refused}"
    );
}
