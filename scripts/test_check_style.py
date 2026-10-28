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

    def test_files_that_are_not_tracked_are_not_read(self):
        result = self.run_in({"src/lib.rs": "\n"}, untracked={"src/b.rs": "use std::io::*;\n"})
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
