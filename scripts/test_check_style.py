"""Tests scripts/check_style.py: the parts of the Rust style rules that
rustfmt and clippy do not check.

Run with: python3 -I scripts/test_check_style.py
"""

import importlib.util
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_style", HERE / "check_style.py")
check_style = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_style)


def rules(text, nonleaf=False):
    """The (line, rule) of each finding in `text`."""
    return [(f.line, f.rule) for f in check_style.findings(text, nonleaf=nonleaf)]


class HeldTransactions(unittest.TestCase):
    """A catalog or registry transaction holds its connection's lock; one
    made in a `match`, `if let` or `while let` scrutinee, or a `for` loop's
    iterator, lives until the block ends, so a call in the block that takes
    the lock again waits on itself."""

    def test_a_for_loop_over_a_snapshot_temporary_is_refused(self):
        text = "fn f() {\n    for seq in self.catalog.snapshot()?.unpublished()? {\n        g();\n    }\n}\n"
        self.assertEqual(rules(text), [(2, "held-transaction")])

    def test_a_match_on_an_immediate_temporary_is_refused(self):
        text = "fn f() {\n    match self.registry.immediate()?.tenant(id)? {\n        _ => {}\n    }\n}\n"
        self.assertEqual(rules(text), [(2, "held-transaction")])

    def test_if_let_and_while_let_on_a_temporary_are_refused(self):
        text = (
            "fn f() {\n    if let Some(r) = self.catalog.snapshot()?.row(v)? {\n        g(r);\n    }\n"
            "    while let Some(r) = self.catalog.immediate()?.next()? {\n        g(r);\n    }\n}\n"
        )
        self.assertEqual(rules(text), [(2, "held-transaction"), (5, "held-transaction")])

    def test_a_transaction_bound_first_is_allowed(self):
        text = (
            "fn f() {\n    let seqs = self.catalog.snapshot()?.unpublished()?;\n"
            "    for seq in seqs {\n        g(seq);\n    }\n"
            "    match tx.version_row(v)? {\n        _ => {}\n    }\n"
            "    if self.catalog.snapshot()?.has(v)? {\n        g(v);\n    }\n}\n"
        )
        self.assertEqual(rules(text), [])

    def test_a_snapshot_inside_the_block_is_allowed(self):
        text = "fn f() {\n    for v in vs {\n        let r = self.catalog.snapshot()?.row(v)?;\n    }\n}\n"
        self.assertEqual(rules(text), [])


class GlobImports(unittest.TestCase):
    def test_a_glob_import_at_file_level_is_refused(self):
        self.assertEqual(rules("use std::io::*;\n"), [(1, "glob-import")])

    def test_a_glob_inside_braces_is_refused(self):
        self.assertEqual(rules("use std::io::{self, *};\n"), [(1, "glob-import")])

    def test_a_prelude_is_refused_even_in_a_test_module(self):
        text = "#[cfg(test)]\nmod tests {\n    use proptest::prelude::*;\n}\n"
        self.assertEqual(rules(text), [(3, "glob-import")])

    def test_a_prelude_submodule_of_a_non_leaf_module_is_refused(self):
        found = check_style.findings("mod prelude;\npub use prelude::*;\n", nonleaf=True)
        self.assertEqual([(f.line, f.rule) for f in found], [(2, "glob-import")])
        self.assertIn("prelude", found[0].message, "the message names the prelude")

    def test_use_super_glob_in_an_inline_module_is_allowed(self):
        text = "#[cfg(test)]\nmod tests {\n    use super::*;\n}\n"
        self.assertEqual(rules(text), [])

    def test_use_super_glob_at_file_level_is_refused(self):
        # A file such as src/mcp/tests.rs is a module of its own, not inline.
        self.assertEqual(rules("use super::*;\n"), [(1, "glob-import")])

    def test_use_super_super_glob_is_refused(self):
        text = "mod a {\n    mod b {\n        use super::super::*;\n    }\n}\n"
        self.assertEqual(rules(text), [(3, "glob-import")])

    def test_use_super_glob_in_a_function_is_refused(self):
        text = "fn f() {\n    use super::*;\n}\n"
        self.assertEqual(rules(text), [(2, "glob-import")])

    def test_an_enum_glob_inside_a_function_is_allowed(self):
        text = "fn name(r: &Rejection) -> &str {\n    use Rejection::*;\n    match r { TooLarge => \"x\" }\n}\n"
        self.assertEqual(rules(text), [])

    def test_an_enum_glob_inside_a_method_is_allowed(self):
        text = "impl Form {\n    fn f(self) {\n        use crate::Form::*;\n    }\n}\n"
        self.assertEqual(rules(text), [])

    def test_an_enum_glob_at_file_level_is_refused(self):
        self.assertEqual(rules("use crate::Form::*;\n"), [(1, "glob-import")])

    def test_a_module_glob_inside_a_function_is_refused(self):
        text = "fn f() {\n    use std::io::*;\n}\n"
        self.assertEqual(rules(text), [(2, "glob-import")])

    def test_pub_use_of_a_submodule_glob_in_a_non_leaf_module_is_allowed(self):
        text = "mod parts;\npub use parts::*;\n"
        self.assertEqual(rules(text, nonleaf=True), [])
        self.assertEqual(rules("mod parts;\npub use self::parts::*;\n", nonleaf=True), [])

    def test_pub_use_glob_in_a_leaf_module_is_refused(self):
        self.assertEqual(rules("mod parts;\npub use parts::*;\n"), [(2, "glob-import")])

    def test_pub_use_glob_of_a_module_not_declared_here_is_refused(self):
        self.assertEqual(rules("pub use serde_json::*;\n", nonleaf=True), [(1, "glob-import")])

    def test_private_use_of_a_submodule_glob_is_refused(self):
        self.assertEqual(rules("mod parts;\nuse parts::*;\n", nonleaf=True), [(2, "glob-import")])

    def test_text_in_comments_and_strings_is_not_code(self):
        text = (
            "// use std::io::*;\n"
            "/* use std::io::*; /* nested */ use a::*; */\n"
            "//! use vcon::read::Rejection::*;\n"
            "const A: &str = \"use std::io::*;\";\n"
            "const B: &str = r#\"use std::io::*; \"# ;\n"
            "const C: char = '{';\n"
            "fn f<'a>(x: &'a str) -> &'a str { x }\n"
            "const D: u8 = b'}';\n"
        )
        self.assertEqual(rules(text), [])

    def test_braces_in_strings_do_not_change_the_nesting(self):
        text = "mod tests {\n    const S: &str = \"}\";\n    use super::*;\n}\n"
        self.assertEqual(rules(text), [])

    def test_use_in_an_impl_trait_bound_is_not_an_import(self):
        text = "fn f<'a>(x: &'a u8) -> impl Sized + use<'a> {\n    let p: *const u8 = x;\n}\nmod m {\n    use super::*;\n}\n"
        self.assertEqual(rules(text), [])

    def test_a_pub_crate_glob_is_refused(self):
        self.assertEqual(rules("pub(crate) use std::io::*;\n"), [(1, "glob-import")])

    def test_the_line_is_the_line_of_the_use(self):
        text = "\n\nuse std::{\n    io::*,\n};\n"
        self.assertEqual(rules(text), [(3, "glob-import")])


class RustfmtSkip(unittest.TestCase):
    def test_a_skip_with_no_comment_above_is_refused(self):
        self.assertEqual(rules("#[rustfmt::skip]\nconst T: [u8; 2] = [1, 2];\n"), [(1, "rustfmt-skip")])

    def test_a_skip_with_a_reason_above_is_allowed(self):
        text = "// Aligned by hand: one row per byte.\n#[rustfmt::skip]\nconst T: [u8; 2] = [1, 2];\n"
        self.assertEqual(rules(text), [])

    def test_an_inner_skip_needs_a_reason_too(self):
        self.assertEqual(rules("#![rustfmt::skip]\n"), [(1, "rustfmt-skip")])

    def test_a_skip_in_a_comment_is_not_code(self):
        self.assertEqual(rules("// #[rustfmt::skip]\n"), [])


class NonLeaf(unittest.TestCase):
    def test_root_and_mod_files_are_non_leaf(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "src" / "store").mkdir(parents=True)
            for name in ["lib.rs", "main.rs", "store.rs", "leaf.rs", "store/mod.rs"]:
                (root / "src" / name).write_text("")
            self.assertTrue(check_style.is_nonleaf(root / "src" / "lib.rs"))
            self.assertTrue(check_style.is_nonleaf(root / "src" / "main.rs"))
            self.assertTrue(check_style.is_nonleaf(root / "src" / "store.rs"))
            self.assertTrue(check_style.is_nonleaf(root / "src" / "store" / "mod.rs"))
            self.assertFalse(check_style.is_nonleaf(root / "src" / "leaf.rs"))


def clean_env():
    """os.environ without git's GIT_* variables, so a git command here acts
    on its own scratch repository even when a hook runs these tests."""
    return {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}


class AssertMessages(unittest.TestCase):
    def missing(self, text):
        return [f.line for f in check_style.findings(text) if f.rule == "assert-message"]

    def test_each_assertion_needs_a_message(self):
        text = (
            "fn f() {\n"
            "    assert!(x);\n"
            "    assert_eq!(a, b);\n"
            "    assert_ne!(a, b);\n"
            "    debug_assert!(x);\n"
            "    debug_assert_eq!(a, b);\n"
            "    debug_assert_ne!(a, b);\n"
            "    prop_assert!(x);\n"
            "    prop_assert_eq!(a, b);\n"
            "    prop_assert_ne!(a, b);\n"
            "}\n"
        )
        self.assertEqual(self.missing(text), [2, 3, 4, 5, 6, 7, 8, 9, 10])

    def test_an_assertion_with_a_message_passes(self):
        text = (
            "fn f() {\n"
            "    assert!(x, \"x holds after open\");\n"
            "    assert_eq!(a, b, \"read back {a}\");\n"
            "    assert_ne!(a, b, \"{}\", why);\n"
            "    prop_assert_eq!(a, b, \"why\");\n"
            "}\n"
        )
        self.assertEqual(self.missing(text), [])

    def test_commas_inside_nested_brackets_are_not_a_message(self):
        text = (
            "fn f() {\n"
            "    assert!(matches!(r, Err(E::A | E::B)));\n"
            "    assert_eq!(f(a, b), [1, 2]);\n"
            "    assert_eq!(v, vec![(1, 2), (3, 4)]);\n"
            "    assert!(m.iter().all(|(k, v)| k == v));\n"
            "    assert_eq!(\n        a,\n        b,\n    );\n"
            "}\n"
        )
        self.assertEqual(self.missing(text), [2, 3, 4, 5, 6])

    def test_any_bracket_works(self):
        self.assertEqual(self.missing("fn f() { assert![x]; assert_eq!{a, b}; }\n"), [1, 1])

    def test_a_name_that_is_not_a_macro_call_is_not_an_assertion(self):
        text = "fn assert(x: bool) {}\nfn f() { assert(true); let assert_eq = 1; }\n// assert!(x);\n"
        self.assertEqual(self.missing(text), [])


class CredentialsInMessages(unittest.TestCase):
    """A failed assertion's message, a panic's text and a debug print reach
    the test log, CI's log among them. None of them may show a key's token,
    a secret, a cookie or a session id, so none names a variable that holds
    one."""

    def shown(self, text):
        return [line for line, rule in rules(text) if rule == "credential-in-message"]

    # Refused.

    def test_a_token_in_an_assert_message_is_refused(self):
        self.assertEqual(self.shown('fn f() {\n    assert!(ok, "{token}");\n}\n'), [2])

    def test_a_token_shown_with_debug_is_refused(self):
        self.assertEqual(self.shown('fn f() {\n    assert_eq!(a, 401, "{token:?}");\n}\n'), [2])

    def test_a_token_in_an_assert_ne_message_is_refused(self):
        self.assertEqual(self.shown('fn f() { assert_ne!(a, b, "both are {token:#?}"); }\n'), [1])

    def test_a_secret_in_a_message_is_refused(self):
        self.assertEqual(self.shown('fn f() { assert!(!log.contains(secret), "{secret} is in the log"); }\n'), [1])

    def test_a_cookie_in_a_message_is_refused(self):
        self.assertEqual(self.shown('fn f() { assert!(set, "no cookie: {cookie}"); }\n'), [1])

    def test_a_session_id_in_a_message_is_refused(self):
        text = 'fn f() {\n    assert!(live, "{session_id:?} has ended");\n    assert!(live, "{}", id_of_session);\n    assert!(live, "{sid}");\n}\n'
        self.assertEqual(self.shown(text), [2, 3, 4])

    def test_a_session_that_is_not_an_id_passes(self):
        # tests/tui_terminal.rs calls its pseudo-terminal a session.
        self.assertEqual(self.shown('fn f() { assert!(live, "the screen: {}", session.shown()); }\n'), [])

    def test_a_longer_name_that_holds_the_word_is_refused(self):
        text = 'fn f() {\n    assert!(a, "{new_token}");\n    assert!(b, "{refresh_tokens}");\n    assert!(c, "{api_secret:?}");\n}\n'
        self.assertEqual(self.shown(text), [2, 3, 4])

    def test_a_field_of_a_value_is_refused_by_the_fields_name(self):
        self.assertEqual(self.shown('fn f() { assert!(a, "{}", made.token); }\n'), [1])

    def test_a_token_as_an_argument_after_the_message_is_refused(self):
        self.assertEqual(self.shown('fn f() {\n    assert!(ok, "made {}", token);\n}\n'), [2])

    def test_a_named_argument_that_is_a_token_is_refused(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "made {t}", t = token); }\n'), [1])

    def test_a_token_in_a_panic_is_refused(self):
        self.assertEqual(self.shown('fn f() {\n    panic!("not parsed: {token}");\n}\n'), [2])

    def test_a_token_in_a_panic_inside_a_closure_inside_an_assertion_is_refused(self):
        text = 'fn f() {\n    assert!(\n        g().unwrap_or_else(|e| panic!("{e}: {bearer}")),\n        "parsed"\n    );\n}\n'
        self.assertEqual(self.shown(text), [3])

    def test_a_token_in_a_debug_print_is_refused(self):
        text = 'fn f() {\n    println!("{token}");\n    eprintln!("got {}", cookie);\n    dbg!(secret);\n    print!("{password}");\n    eprint!("{passphrase}");\n}\n'
        self.assertEqual(self.shown(text), [2, 3, 4, 5, 6])

    def test_dbg_of_an_expression_that_names_a_token_is_refused(self):
        self.assertEqual(self.shown('fn f() { let n = dbg!(a, token.len()); }\n'), [1])

    def test_a_token_in_a_macro_inside_an_argument_after_the_message_is_refused(self):
        text = 'fn f() {\n    assert!(a, "{}", format!("Bearer {token}"));\n    assert!(b, "{:?}", vec![cookie]);\n}\n'
        self.assertEqual(self.shown(text), [2, 3])

    def test_a_message_over_several_lines_is_named_by_the_line_of_the_message(self):
        text = 'fn f() {\n    assert_eq!(\n        a,\n        b,\n        "the answer for {token}"\n    );\n}\n'
        self.assertEqual(self.shown(text), [5])

    def test_the_debug_and_prop_forms_are_read_too(self):
        text = 'fn f() {\n    debug_assert!(a, "{token}");\n    prop_assert_eq!(a, b, "{token}");\n    unreachable!("{secret}");\n}\n'
        self.assertEqual(self.shown(text), [2, 3, 4])

    def test_upper_case_names_are_read_as_their_words(self):
        self.assertEqual(self.shown('fn f() { assert!(a, "{TOKEN} {}", SESSION_COOKIE); }\n'), [1, 1])

    # Allowed.

    def test_a_message_that_says_the_word_and_shows_no_value_passes(self):
        text = 'fn f() {\n    assert!(ok, "the token starts with the key id");\n    panic!("no session cookie was set");\n}\n'
        self.assertEqual(self.shown(text), [])

    def test_a_token_in_the_condition_passes(self):
        text = 'fn f() {\n    assert!(token.starts_with("vcs_"), "the prefix");\n    assert_eq!(parse(&token), Ok(id), "case {n}");\n}\n'
        self.assertEqual(self.shown(text), [])

    def test_a_string_built_from_a_token_in_the_condition_passes(self):
        text = 'fn f() {\n    assert!(auth(&format!("Bearer {token}")).is_ok(), "the key is taken");\n}\n'
        self.assertEqual(self.shown(text), [])

    def test_a_format_that_is_not_a_message_passes(self):
        text = 'fn f() {\n    let header = format!("Bearer {token}");\n    write!(out, "{cookie}")?;\n}\n'
        self.assertEqual(self.shown(text), [])

    def test_other_names_pass(self):
        text = 'fn f() {\n    assert!(a, "{key}: {error}");\n    assert!(b, "{tokenized} {secretary} {sessions} {id}");\n    println!("{checked} checked");\n}\n'
        self.assertEqual(self.shown(text), [])

    def test_escaped_braces_are_not_a_placeholder(self):
        self.assertEqual(self.shown('fn f() { assert!(a, "write {{token}} for the token"); }\n'), [])

    def test_a_comment_is_not_read(self):
        text = 'fn f() {\n    // assert!(a, "{token}");\n    /* panic!("{secret}"); */\n}\n'
        self.assertEqual(self.shown(text), [])

    def test_a_macro_shown_inside_a_string_is_not_read(self):
        self.assertEqual(self.shown('const T: &str = "assert!(a, \\"{token}\\");";\n'), [])

    def test_a_name_that_is_not_a_macro_call_is_not_read(self):
        self.assertEqual(self.shown('fn panic(token: &str) {}\nfn f() { panic(token); }\n'), [])

    def test_a_raw_string_message_is_read(self):
        self.assertEqual(self.shown('fn f() { assert!(a, r#"the "token" {token}"#); }\n'), [1])

    def test_a_character_literal_does_not_end_the_reading(self):
        text = "fn f() {\n    assert!(c != ')', \"{token}\");\n    assert!(d != '\"', \"fine\");\n}\n"
        self.assertEqual(self.shown(text), [2])

    def test_a_lifetime_is_not_a_character_literal(self):
        text = "fn f<'a>(token: &'a str) {\n    assert!(g::<&'a str>(token), \"{token}\");\n}\n"
        self.assertEqual(self.shown(text), [2])

    def test_one_finding_for_each_argument_that_shows_one(self):
        self.assertEqual(self.shown('fn f() { assert!(a, "{token} {secret}", cookie); }\n'), [1, 1])

    def test_the_finding_says_what_to_do(self):
        found = [f for f in check_style.findings('fn f() { assert!(a, "{token}"); }\n') if f.rule == "credential-in-message"]
        self.assertEqual(len(found), 1, "one finding")
        self.assertIn("`token`", found[0].message)
        self.assertIn("assert!", found[0].message)


class UuidsInMessages(unittest.TestCase):
    """The uuid of a vCon is a value the catalogs seal. A failed assertion's
    message, a panic's text, a debug print and a log line reach a log, and
    code scanning refuses a uuid written there (rust/cleartext-logging), so
    none names a variable that holds one."""

    def shown(self, text):
        return [line for line, rule in rules(text) if rule == "uuid-in-message"]

    # Refused.

    def test_a_uuid_in_an_assert_message_is_refused(self):
        self.assertEqual(self.shown('fn f() {\n    assert!(ok, "{uuid}, stored under an earlier key");\n}\n'), [2])

    def test_a_uuid_shown_with_debug_is_refused(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "{uuid:?} reads"); }\n'), [1])

    def test_a_uuid_in_an_assert_eq_message_is_refused(self):
        self.assertEqual(self.shown('fn f() { assert_eq!(a, b, "{uuid} has one version"); }\n'), [1])

    def test_a_uuid_as_an_argument_after_the_message_is_refused(self):
        self.assertEqual(self.shown('fn f() {\n    assert!(ok, "{} reads", uuid);\n}\n'), [2])

    def test_a_named_argument_that_is_a_uuid_is_refused(self):
        text = 'fn f() {\n    assert!(ok, "{u} reads", u = uuid);\n    assert!(ok, "{uuid} reads", uuid = found.id);\n}\n'
        self.assertEqual(self.shown(text), [2, 3])

    def test_a_field_named_uuid_is_refused(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "{}", found.uuid); }\n'), [1])

    def test_a_longer_name_that_ends_in_the_word_is_refused(self):
        text = 'fn f() {\n    assert!(a, "{gzip_uuid}");\n    assert!(b, "{vcon_uuids:?}");\n}\n'
        self.assertEqual(self.shown(text), [2, 3])

    def test_a_uuid_in_a_panic_is_refused(self):
        self.assertEqual(self.shown('fn f() { panic!("{uuid} is not stored"); }\n'), [1])

    def test_a_uuid_in_a_print_is_refused(self):
        text = 'fn f() {\n    println!("stored {uuid}");\n    eprintln!("{uuid}");\n    dbg!(uuid);\n}\n'
        self.assertEqual(self.shown(text), [2, 3, 4])

    def test_a_uuid_in_a_log_line_is_refused(self):
        text = (
            'fn f() {\n    tracing::info!("stored {uuid}");\n    warn!(%uuid, "not packed");\n'
            '    tracing::error!(uuid = %found.id, "lost");\n    debug!("{}", uuid);\n    trace!(?uuid);\n}\n'
        )
        self.assertEqual(self.shown(text), [2, 3, 4, 5, 6])

    def test_a_uuid_in_an_unreachable_is_refused(self):
        self.assertEqual(self.shown('fn f() { unreachable!("{uuid} has no version"); }\n'), [1])

    def test_a_message_over_several_lines_is_found_at_its_argument(self):
        text = 'fn f() {\n    assert!(\n        ok,\n        "{uuid} reads"\n    );\n}\n'
        self.assertEqual(self.shown(text), [4])

    def test_the_finding_says_what_to_do(self):
        found = [f for f in check_style.findings('fn f() { assert!(a, "{uuid}"); }\n') if f.rule == "uuid-in-message"]
        self.assertEqual(len(found), 1)
        self.assertIn("`assert!` shows `uuid`", found[0].message)
        self.assertIn("its place in the test", found[0].message)

    # Passes.

    def test_a_message_with_no_uuid_passes(self):
        self.assertEqual(self.shown('fn f() { assert!(reads(&store, uuid), "vCon {}, stored under an earlier key", at + 1); }\n'), [])

    def test_a_uuid_that_is_compared_and_not_shown_passes(self):
        self.assertEqual(self.shown('fn f() { assert_eq!(found.uuid, uuid, "the same vCon"); }\n'), [])

    def test_a_count_of_uuids_passes(self):
        text = 'fn f() {\n    assert!(ok, "{uuid_count} vCons");\n    assert!(ok, "{}", uuids_len);\n}\n'
        self.assertEqual(self.shown(text), [])

    def test_a_mac_of_a_uuid_passes(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "{uuid_mac:?}"); }\n'), [])

    def test_a_format_that_builds_a_path_passes(self):
        self.assertEqual(self.shown('fn f() { let path = format!("/vcons/{uuid}"); call(&path); }\n'), [])

    def test_a_write_to_a_response_passes(self):
        self.assertEqual(self.shown('fn f() { writeln!(out, "{uuid}")?; }\n'), [])

    # Edges.

    def test_a_doc_comment_that_names_the_placeholder_passes(self):
        text = '/// The message is `"{uuid} reads"`: assert!(ok, "{uuid}").\nfn f() {}\n// println!("{uuid}");\n'
        self.assertEqual(self.shown(text), [])

    def test_a_sql_string_outside_a_message_passes(self):
        text = 'fn f() { tx.execute("SELECT uuid_sealed FROM vcon.vcon WHERE uuid = $1", &[&uuid]); }\n'
        self.assertEqual(self.shown(text), [])

    def test_the_word_in_a_message_s_text_passes(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "the uuid is not found by its {mac:?}"); }\n'), [])

    def test_an_escaped_brace_is_no_placeholder(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "the path is /vcons/{{uuid}}"); }\n'), [])

    def test_the_type_is_not_a_value(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "{}", size_of::<Uuid>()); }\n'), [])

    def test_a_format_inside_a_message_argument_is_read(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "{}", format!("{uuid} reads")); }\n'), [1])

    def test_a_placeholder_and_its_named_argument_on_two_lines_are_two_findings(self):
        text = 'fn f() {\n    assert!(\n        ok,\n        "{u} and {uuid}",\n        u = uuid\n    );\n}\n'
        self.assertEqual(self.shown(text), [4, 5])

    def test_one_finding_for_an_argument_that_shows_it_twice(self):
        self.assertEqual(self.shown('fn f() { assert!(ok, "{uuid} and {uuid:?}"); }\n'), [1])

    def test_a_file_outside_the_crates_is_not_read_for_this_rule(self):
        found = check_style.findings('fn f() { println!("stored {uuid}"); }\n', uuids=False)
        self.assertEqual([f.rule for f in found], [])

    def test_the_rule_reads_the_crates_and_not_the_example_clients(self):
        self.assertTrue(check_style.reads_uuids("crates/vcon-server/src/store.rs"))
        self.assertTrue(check_style.reads_uuids("./crates/vcon/tests/a.rs"))
        self.assertFalse(check_style.reads_uuids("clients/rust/src/main.rs"))
        self.assertFalse(check_style.reads_uuids("fuzz/fuzz_targets/crates.rs"))

    def test_a_credential_and_a_uuid_in_one_message_are_two_findings(self):
        found = rules('fn f() { assert!(ok, "{token} {uuid}"); }\n')
        self.assertEqual(sorted(found), [(1, "credential-in-message"), (1, "uuid-in-message")])


class CredentialNamedPaths(unittest.TestCase):
    """A path is not a credential. A parameter or field of a path type
    named `password` makes code scanning read every file name passed to it
    as a hard-coded password (three alerts on 2026-11-12, for
    `postgres_config(..., password: &Path)` given `dir.join("pg.password")`).
    The name says what the value is: the file."""

    def named(self, text):
        return [line for line, rule in rules(text) if rule == "credential-path"]

    # Refused.

    def test_a_path_parameter_named_password_is_refused(self):
        self.assertEqual(self.named("fn config(dir: &Path, password: &Path) -> PathBuf {\n}\n"), [1])

    def test_a_pathbuf_parameter_named_secret_is_refused(self):
        self.assertEqual(self.named("fn f(\n    secret: PathBuf,\n) {}\n"), [2])

    def test_a_qualified_path_type_is_refused(self):
        self.assertEqual(self.named("fn f(token: &std::path::Path) {}\n"), [1])

    def test_a_mutable_reference_and_a_lifetime_are_read_through(self):
        self.assertEqual(self.named("fn f<'a>(passphrase: &'a mut PathBuf) {}\n"), [1])

    def test_a_field_named_for_the_credential_is_refused(self):
        self.assertEqual(self.named("struct S {\n    pub password: PathBuf,\n}\n"), [2])

    def test_an_optional_path_is_refused(self):
        self.assertEqual(self.named("struct S { cookie: Option<PathBuf> }\n"), [1])

    def test_a_let_with_a_path_type_is_refused(self):
        self.assertEqual(self.named("fn f() { let password: PathBuf = dir.join(name); }\n"), [1])

    def test_each_such_parameter_is_a_finding(self):
        self.assertEqual(self.named("fn f(password: &Path, secret: &Path) {}\n"), [1, 1])

    def test_a_longer_name_that_holds_the_word_is_refused(self):
        self.assertEqual(self.named("fn f(db_password: &Path, session_id: PathBuf) {}\n"), [1, 1])

    # Accepted.

    def test_a_name_that_says_file_is_accepted(self):
        self.assertEqual(self.named("fn f(password_file: &Path, secret_path: PathBuf, token_dir: &Path) {}\n"), [])

    def test_a_credential_that_is_text_is_not_this_rules(self):
        self.assertEqual(self.named("fn f(password: &str, token: String, secret: &[u8]) {}\n"), [])

    def test_a_path_with_another_name_is_accepted(self):
        self.assertEqual(self.named("fn f(dir: &Path, key: &Path, config: PathBuf) {}\n"), [])

    def test_a_struct_literal_field_is_not_a_declaration(self):
        self.assertEqual(self.named("fn f() { let s = S { password: path.clone(), other: PathBuf::new() }; }\n"), [])

    def test_a_type_after_the_next_comma_is_not_the_names(self):
        self.assertEqual(self.named("fn f(password: &str, dir: &Path) {}\n"), [])

    def test_the_return_type_is_not_the_last_parameters(self):
        self.assertEqual(self.named("fn f(dir: &Path, password: &str) -> PathBuf {\n}\n"), [])

    def test_a_comment_and_a_string_are_not_read(self):
        self.assertEqual(self.named('// fn f(password: &Path)\nfn g() { let s = "password: &Path"; }\n'), [])

    # The limits.

    def test_a_generic_that_holds_a_path_deeper_is_refused(self):
        self.assertEqual(self.named("fn f(secret: Option<&Path>) {}\n"), [1])

    def test_a_path_type_of_another_name_is_not_read(self):
        self.assertEqual(self.named("fn f(password: Utf8PathBuf, token: PathLike) {}\n"), [])

    def test_the_finding_says_what_to_do(self):
        found = [f for f in check_style.findings("fn f(password: &Path) {}\n") if f.rule == "credential-path"]
        self.assertEqual(len(found), 1, "one finding")
        self.assertIn("`password`", found[0].message)
        self.assertIn("password_file", found[0].message)


class Ratchet(unittest.TestCase):
    def verdict(self, counts, baseline):
        return check_style.ratchet(counts, baseline)

    def test_a_file_at_its_baseline_passes(self):
        self.assertEqual(self.verdict({"a.rs": 3}, {"a.rs": 3}), [])

    def test_a_file_above_its_baseline_is_refused(self):
        self.assertEqual(len(self.verdict({"a.rs": 4}, {"a.rs": 3})), 1)

    def test_a_new_file_must_have_none(self):
        problems = self.verdict({"a.rs": 0, "new.rs": 1}, {})
        self.assertEqual(len(problems), 1)
        self.assertIn("new.rs", problems[0])

    def test_a_file_below_its_baseline_must_lower_it(self):
        # The baseline only shrinks: a fixed site is written down at once.
        problems = self.verdict({"a.rs": 1}, {"a.rs": 3})
        self.assertEqual(len(problems), 1)
        self.assertIn("--write-baseline", problems[0])

    def test_an_entry_for_a_file_not_in_the_tree_is_named_as_stale_once(self):
        # A file that was removed, or a tree that is not this repository (the
        # hook tests' fixture), is not a file whose count fell.
        problems = self.verdict({"a.rs": 0}, {"gone.rs": 2})
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("gone.rs", problems[0])
        self.assertIn("not in the tree", problems[0])
        self.assertNotIn("0 assertion", problems[0], "no count is reported for a file that is not there")

    def test_the_written_baseline_drops_an_entry_for_a_file_not_in_the_tree(self):
        self.assertEqual(check_style.next_baseline({"a.rs": 1}, {"a.rs": 1, "gone.rs": 2}), {"a.rs": 1})

    def test_a_baseline_entry_for_a_file_that_is_gone_or_clean_must_go(self):
        self.assertEqual(len(self.verdict({}, {"gone.rs": 2})), 1)
        self.assertEqual(len(self.verdict({"a.rs": 0}, {"a.rs": 1})), 1)

    def test_the_written_baseline_never_rises(self):
        self.assertEqual(
            check_style.next_baseline({"a.rs": 5, "b.rs": 1, "c.rs": 0, "new.rs": 2}, {"a.rs": 3, "b.rs": 2, "c.rs": 1}),
            {"a.rs": 3, "b.rs": 1},
        )

    def test_the_baseline_file_round_trips(self):
        text = check_style.format_baseline({"b.rs": 1, "a.rs": 12})
        self.assertTrue(text.startswith("#"), "the file says what it is")
        self.assertEqual(check_style.parse_baseline(text), {"a.rs": 12, "b.rs": 1})


class LintTables(unittest.TestCase):
    WORKSPACE = {"rust": {"missing_docs": "deny"}, "clippy": {"unwrap_used": "deny", "missing_errors_doc": "deny"}}

    def problems(self, lints, omit=()):
        return check_style.lint_table_problems(self.WORKSPACE, "crate", lints, omit)

    def test_a_member_that_takes_the_workspace_lints_passes(self):
        self.assertEqual(self.problems({"workspace": True}), [])

    def test_a_member_with_no_lints_is_refused(self):
        self.assertEqual(len(self.problems({})), 3, "each missing lint is named")

    def test_an_own_table_must_hold_every_workspace_lint_at_its_level(self):
        own = {"rust": {"missing_docs": "deny", "unsafe_code": "forbid"}, "clippy": {"unwrap_used": "deny", "missing_errors_doc": "deny"}}
        self.assertEqual(self.problems(own), [], "an own table may add lints")
        weaker = {"rust": {"missing_docs": "warn"}, "clippy": {"unwrap_used": "deny", "missing_errors_doc": "deny"}}
        found = self.problems(weaker)
        self.assertEqual(len(found), 1, found)
        self.assertIn("missing_docs", found[0])

    def test_a_lint_given_as_a_table_is_read_by_its_level(self):
        own = {"rust": {"missing_docs": {"level": "deny", "priority": 1}}, "clippy": {"unwrap_used": "deny", "missing_errors_doc": "deny"}}
        self.assertEqual(self.problems(own), [])

    def test_named_lints_may_be_left_out(self):
        own = {"clippy": {"unwrap_used": "deny"}}
        self.assertEqual(self.problems(own, omit={"missing_docs", "missing_errors_doc"}), [])

    def test_the_repository_tables_agree(self):
        root = HERE.parent
        self.assertEqual(check_style.lint_tables(root), [], "every member holds the workspace's lints")


class Command(unittest.TestCase):
    def run_in(self, files, untracked=None):
        env = clean_env()
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            subprocess.run(["git", "init", "-q", str(root)], check=True, env=env)
            for name, text in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            subprocess.run(["git", "-C", str(root), "add", "-A"], check=True, env=env)
            for name, text in (untracked or {}).items():
                (root / name).write_text(text)
            return subprocess.run(
                [sys.executable, "-I", str(HERE / "check_style.py")],
                cwd=root,
                capture_output=True,
                text=True,
                env=env,
            )

    def test_the_environment_of_a_git_hook_is_not_used(self):
        # The commit hook runs these tests with GIT_INDEX_FILE naming the
        # real repository's index; a git command in the scratch repository
        # that inherits it writes that index (it once replaced a commit's
        # tree with this test's one file).
        with tempfile.TemporaryDirectory() as d:
            index = pathlib.Path(d) / "index"
            index.write_bytes(b"the caller's index")
            with mock.patch.dict(os.environ, {"GIT_INDEX_FILE": str(index), "GIT_DIR": d}):
                result = self.run_in({"src/lib.rs": "pub fn f() {}\n"})
            self.assertEqual(index.read_bytes(), b"the caller's index", "the caller's index is untouched")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_uuid_in_a_message_fails_under_crates_and_passes_in_a_client(self):
        shown = 'fn f(uuid: u8) {\n    println!("stored {uuid}");\n}\n'
        result = self.run_in({"clients/rust/src/main.rs": shown})
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.run_in({"clients/rust/src/main.rs": shown, "crates/a/src/lib.rs": shown})
        self.assertEqual(result.returncode, 1, "a uuid in a print under crates/ must fail the check")
        self.assertIn("crates/a/src/lib.rs:2: uuid-in-message:", result.stderr)
        self.assertNotIn("clients/", result.stderr)

    def test_a_clean_tree_passes(self):
        result = self.run_in({"src/lib.rs": "mod a;\npub use a::*;\n", "src/a.rs": "pub fn f() {}\n"})
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_each_finding_is_named_by_path_and_line_and_the_command_fails(self):
        result = self.run_in({"src/lib.rs": "mod a;\n", "src/a.rs": "\nuse std::io::*;\n"})
        self.assertEqual(result.returncode, 1, "a glob import must fail the check")
        self.assertIn("src/a.rs:2: glob-import:", result.stderr)

    def test_walk_reads_every_rs_file_below_the_directory_without_git(self):
        # The commit hook runs it in a copy of the index, which is no repository.
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "src").mkdir()
            (root / "src" / "lib.rs").write_text("mod a;\n")
            (root / "src" / "a.rs").write_text("use std::io::*;\n")
            (root / "target" / "debug").mkdir(parents=True)
            (root / "target" / "debug" / "out.rs").write_text("use std::fs::*;\n")
            result = subprocess.run(
                [sys.executable, "-I", str(HERE / "check_style.py"), "--walk"],
                cwd=root,
                capture_output=True,
                text=True,
            )
        self.assertEqual(result.returncode, 1, "a glob import must fail the check")
        self.assertIn("src/a.rs:1: glob-import:", result.stderr)
        self.assertNotIn("target", result.stderr, "build output is not source")

    def test_an_assertion_without_a_message_in_a_new_file_fails_the_command(self):
        result = self.run_in({"src/lib.rs": "fn f() {\n    assert!(true);\n}\n"})
        self.assertEqual(result.returncode, 1, "a new file may not lack messages")
        self.assertIn("src/lib.rs", result.stderr)

    def test_a_file_within_the_baseline_passes_the_command(self):
        result = self.run_in({
            "src/lib.rs": "fn f() {\n    assert!(true);\n}\n",
            "style/assert-message-baseline.txt": "# counts\n1\tsrc/lib.rs\n",
        })
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_files_that_are_not_tracked_are_not_read(self):
        result = self.run_in({"src/lib.rs": "\n"}, untracked={"src/b.rs": "use std::io::*;\n"})
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
