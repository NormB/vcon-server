"""Tests scripts/hook_tests.py: which pushes must run the hook tests that
CI's "git hooks" job runs, read from the files those tests copy.

Run with: python3 -I scripts/test_hook_tests.py
"""

import importlib.util
import pathlib
import subprocess
import sys
import unittest
import unittest.mock

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("hook_tests", HERE / "hook_tests.py")
ht = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ht)

SCRIPT = """#!/usr/bin/env bash
src="$(git rev-parse --show-toplevel)"
cp -R "$src/.githooks" "$src/scripts" "$src/.gitignore" .
cp "$src/website/content/_index.md" website/content/
sed 's/a/b/' "$src/.github/sets.txt" > .github/sets.txt
for script in "$@"; do cp "$src/$script" "$case_dir/repo/$script"; done
# cp "$src/commented-out.txt" .
echo "$srcdir/not-this.txt"
"""


class ReadPaths(unittest.TestCase):
    def test_each_path_after_src_is_read(self):
        self.assertEqual(
            ht.read_paths(SCRIPT),
            [".githooks", ".github/sets.txt", ".gitignore", "scripts", "website/content/_index.md"],
        )

    def test_a_path_named_by_a_variable_is_not_a_path(self):
        self.assertNotIn("$script", ht.read_paths(SCRIPT))

    def test_a_comment_is_not_read(self):
        self.assertNotIn("commented-out.txt", ht.read_paths(SCRIPT))

    def test_another_variable_that_begins_with_src_is_not_src(self):
        self.assertNotIn("not-this.txt", ht.read_paths(SCRIPT))
        self.assertNotIn("dir/not-this.txt", ht.read_paths(SCRIPT))

    def test_a_path_named_twice_is_listed_once(self):
        self.assertEqual(ht.read_paths('cp "$src/a" .\ncp "$src/a" b\n'), ["a"])

    def test_a_script_that_copies_nothing_reads_nothing(self):
        self.assertEqual(ht.read_paths("#!/bin/sh\necho hello\n"), [])

    def test_a_path_that_climbs_out_of_the_tree_is_refused(self):
        with self.assertRaises(ht.HookTestsError):
            ht.read_paths('cp "$src/../elsewhere" .\n')

    def test_the_real_hook_tests_read_the_hooks_and_the_scripts(self):
        found = ht.tree_read_paths(HERE.parent)
        for path in [".githooks", "scripts", ".github", "CHANGELOG.md", "website/content/docs", ".githooks/pre-push"]:
            self.assertIn(path, found, path)

    def test_the_tests_that_are_run_are_the_ones_read(self):
        self.assertEqual(ht.TESTS, ("scripts/test-hooks.sh", "scripts/test-pre-push.sh"))
        for name in ht.TESTS:
            self.assertTrue((HERE.parent / name).is_file(), name)


class RealTree(unittest.TestCase):
    READ = ht.tree_read_paths(HERE.parent)

    def test_a_docs_page_is_read_by_the_hook_tests(self):
        self.assertEqual(ht.needed(["website/content/docs/install.md"], self.READ), ["website/content/docs/install.md"])

    def test_the_changelog_and_the_licence_files_are_read(self):
        for path in ["CHANGELOG.md", "LICENSE-MIT", "LICENSE-APACHE"]:
            self.assertEqual(ht.needed([path], self.READ), [path], path)

    def test_the_server_code_and_the_readme_are_not_read(self):
        self.assertEqual(ht.needed(["crates/vcon-server/src/main.rs", "README.md", "Cargo.lock"], self.READ), [])

    def test_the_rule_itself_and_its_test_are_read(self):
        self.assertEqual(ht.needed(["scripts/hook_tests.py", "scripts/test_hook_tests.py"], self.READ), ["scripts/hook_tests.py", "scripts/test_hook_tests.py"])


class Shapes(unittest.TestCase):
    def test_a_trailing_slash_names_the_same_directory(self):
        self.assertEqual(ht.read_paths('cp -R "$src/website/content/docs/" .\n'), ["website/content/docs"])

    def test_an_absolute_path_is_refused(self):
        with self.assertRaises(ht.HookTestsError):
            ht.read_paths('cp "$src//etc/passwd" .\n')

    def test_a_path_with_a_space_is_one_path(self):
        self.assertEqual(ht.read_paths('cp "$src/my file.txt" .\n'), ["my file.txt"])

    def test_a_tree_with_one_of_the_two_hook_tests_is_refused(self):
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            (pathlib.Path(d) / "scripts").mkdir()
            (pathlib.Path(d) / "scripts/test-hooks.sh").write_text('cp "$src/a" .\n', encoding="utf-8")
            with self.assertRaises(ht.HookTestsError) as caught:
                ht.tree_read_paths(d)
        self.assertIn("scripts/test-pre-push.sh", str(caught.exception))


class Needed(unittest.TestCase):
    READ = [".githooks", "scripts", "website/content/_index.md"]

    def test_a_change_to_a_hook_needs_them(self):
        self.assertEqual(ht.needed([".githooks/pre-commit"], self.READ), [".githooks/pre-commit"])

    def test_a_change_to_a_script_needs_them(self):
        self.assertEqual(ht.needed(["src/lib.rs", "scripts/scan_forbidden.py"], self.READ), ["scripts/scan_forbidden.py"])

    def test_a_change_to_a_file_read_by_its_name_needs_them(self):
        self.assertEqual(ht.needed(["website/content/_index.md"], self.READ), ["website/content/_index.md"])

    def test_every_path_they_read_is_given_in_order_once(self):
        changed = ["scripts/b.py", "README.md", "scripts/a.py", "scripts/b.py"]
        self.assertEqual(ht.needed(changed, self.READ), ["scripts/b.py", "scripts/a.py"])

    def test_a_change_they_do_not_read_does_not_need_them(self):
        self.assertEqual(ht.needed(["crates/vcon-server/src/lib.rs", "README.md"], self.READ), [])

    def test_a_path_that_only_begins_like_one_they_read_does_not_need_them(self):
        self.assertEqual(ht.needed(["scripts-old/x.py", ".githooks.bak", "website/content/_index.md.orig"], self.READ), [])

    def test_a_page_beside_the_one_they_read_does_not_need_them(self):
        self.assertEqual(ht.needed(["website/content/about.md"], self.READ), [])

    def test_no_change_needs_nothing(self):
        self.assertEqual(ht.needed([], self.READ), [])

    def test_blank_lines_and_spaces_are_not_paths(self):
        self.assertEqual(ht.needed(["", "  ", "scripts/x.sh  "], self.READ), ["scripts/x.sh"])

    def test_a_removed_or_renamed_file_counts_by_its_path(self):
        self.assertEqual(ht.needed(["scripts/gone.py"], self.READ), ["scripts/gone.py"])


class Command(unittest.TestCase):
    def run_needed(self, stdin, root=None):
        args = [sys.executable, "-I", str(HERE / "hook_tests.py"), "needed"]
        if root:
            args += ["--root", str(root)]
        return subprocess.run(args, input=stdin, capture_output=True, text=True, timeout=60, check=False)

    def test_a_push_that_changes_a_hook_answers_0_and_names_it(self):
        out = self.run_needed(".githooks/pre-push\nREADME.md\n")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(out.stdout, "hook tests: needed: the push changes .githooks/pre-push, which they read\n")

    def test_the_count_of_more_paths_is_given(self):
        out = self.run_needed("scripts/a.py\nscripts/b.py\nscripts/c.py\n")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(out.stdout, "hook tests: needed: the push changes scripts/a.py and 2 more files they read\n")

    def test_a_push_they_do_not_read_answers_1_and_says_so(self):
        out = self.run_needed("crates/vcon/src/lib.rs\n")
        self.assertEqual(out.returncode, 1, out.stderr)
        self.assertEqual(out.stdout, "hook tests: not needed: the push changes no file they read\n")

    def test_an_empty_push_answers_1(self):
        self.assertEqual(self.run_needed("").returncode, 1)

    def test_a_tree_without_the_hook_tests_answers_2(self):
        import tempfile

        with tempfile.TemporaryDirectory() as empty:
            out = self.run_needed("scripts/a.py\n", root=empty)
        self.assertEqual(out.returncode, 2)
        self.assertIn("hook_tests: ", out.stderr)

    def test_a_hook_test_that_reads_outside_the_tree_answers_2(self):
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            (pathlib.Path(d) / "scripts").mkdir()
            for name in ht.TESTS:
                (pathlib.Path(d) / name).write_text('cp "$src/../outside" .\n', encoding="utf-8")
            out = self.run_needed("scripts/a.py\n", root=d)
        self.assertEqual(out.returncode, 2)
        self.assertIn("which is not a path of the tree", out.stderr)
        self.assertEqual(out.stdout, "", "no verdict is printed")

    def test_a_root_that_is_a_file_answers_2(self):
        out = self.run_needed("scripts/a.py\n", root=HERE / "hook_tests.py")
        self.assertEqual(out.returncode, 2)
        self.assertIn("cannot read scripts/test-hooks.sh", out.stderr)

    def test_an_unknown_command_answers_2(self):
        out = subprocess.run([sys.executable, "-I", str(HERE / "hook_tests.py"), "nope"], capture_output=True, text=True, timeout=60, check=False)
        self.assertEqual(out.returncode, 2)


class NotUtf8Paths(unittest.TestCase):
    """`git log --name-only` prints a path's bytes as they are when
    core.quotePath is false. A path that is not UTF-8 crashed the rule, and
    the crash exited 1, which pre-push reads as "not needed": the hook
    tests were skipped for a push that needed them."""

    def run_bytes(self, stdin):
        args = [sys.executable, "-I", str(HERE / "hook_tests.py"), "needed"]
        return subprocess.run(args, input=stdin, capture_output=True, timeout=60, check=False)

    def test_a_push_with_a_path_that_is_not_utf8_still_needs_them(self):
        out = self.run_bytes(b"caf\xe9.txt\nscripts/test-hooks.sh\n")
        self.assertNotIn(b"Traceback", out.stderr, out.stderr)
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(out.stdout, b"hook tests: needed: the push changes scripts/test-hooks.sh, which they read\n")

    def test_a_path_that_is_not_utf8_under_a_read_directory_needs_them(self):
        out = self.run_bytes(b"scripts/caf\xe9.py\n")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(out.stdout, b"hook tests: needed: the push changes scripts/caf\\udce9.py, which they read\n")

    def test_only_a_path_that_is_not_utf8_and_not_read_answers_1(self):
        out = self.run_bytes(b"crates/caf\xe9.rs\n")
        self.assertNotIn(b"Traceback", out.stderr, out.stderr)
        self.assertEqual(out.returncode, 1, out.stderr)

    def test_a_path_in_utf8_is_named_as_it_is(self):
        out = self.run_bytes("scripts/café.py\n".encode())
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(out.stdout, "hook tests: needed: the push changes scripts/café.py, which they read\n".encode())

    def test_the_paths_are_read_as_bytes_with_each_bad_byte_kept(self):
        import io

        self.assertEqual(ht.changed_paths(io.BytesIO(b"a\xff\nb\n")), ["a\udcff", "b"])

    def test_a_hook_test_that_is_not_utf8_answers_2_not_1(self):
        # read_text raises UnicodeDecodeError, which is no OSError: before,
        # it was an uncaught exception, exit 1, "not needed".
        import tempfile

        with tempfile.TemporaryDirectory() as d:
            (pathlib.Path(d) / "scripts").mkdir()
            for name in ht.TESTS:
                (pathlib.Path(d) / name).write_bytes(b'cp "$src/caf\xe9" .\n')
            out = subprocess.run(
                [sys.executable, "-I", str(HERE / "hook_tests.py"), "needed", "--root", d],
                input=b"scripts/a.py\n", capture_output=True, timeout=60, check=False,
            )
        self.assertEqual(out.returncode, 2, out.stderr)
        self.assertIn(b"hook_tests: cannot tell: ", out.stderr)

    def test_a_rule_that_fails_answers_2_not_1(self):
        import io

        def broken(_root):
            raise RuntimeError("no such thing")

        err = io.StringIO()
        with unittest.mock.patch.object(ht, "tree_read_paths", broken), unittest.mock.patch("sys.stderr", err):
            self.assertEqual(ht.run(["needed"], io.BytesIO(b"scripts/a.py\n")), 2, "1 would skip the hook tests")
        self.assertIn("hook_tests: cannot tell: no such thing", err.getvalue())


class PushHook(unittest.TestCase):
    HOOK = (HERE.parent / ".githooks" / "pre-push").read_text(encoding="utf-8")

    def code(self):
        return "\n".join(line for line in self.HOOK.splitlines() if not line.strip().startswith("#"))

    def test_the_push_hook_asks_the_rule_with_the_outgoing_files(self):
        self.assertIn('git log --name-only --format= "${range[@]}"', self.code())
        self.assertIn('python3 -I "$root/scripts/hook_tests.py" needed', self.code())

    def test_the_push_hook_runs_each_hook_test_the_rule_names(self):
        for name in ht.TESTS:
            self.assertIn(f'"$root/{name}" </dev/null', self.code(), name)

    def test_a_failed_hook_test_or_an_unreadable_rule_refuses_the_push(self):
        code = self.code()
        self.assertIn("pre-push: refused: the hook tests failed", code)
        self.assertIn("pre-push: refused: cannot tell whether the hook tests are needed", code)

    def test_the_rule_is_asked_once_after_every_ref_is_read(self):
        code = self.code()
        ask = 'python3 -I "$root/scripts/hook_tests.py" needed'
        self.assertEqual(code.count(ask), 1)
        self.assertLess(code.rindex("\ndone"), code.index(ask), "after the loop over the refs")


if __name__ == "__main__":
    unittest.main()
