"""Tests scripts/hook_fixture.py: which script tests the commit hook runs,
which of them the hook tests' fixture replaces, what a stand-in does when
the hook runs it, and how the calls it records are checked.

Run with: python3 -I scripts/test_hook_fixture.py
"""

import contextlib
import importlib.util
import io
import os
import pathlib
import stat
import subprocess
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("hook_fixture", HERE / "hook_fixture.py")
hook_fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hook_fixture)

KEPT = "scripts/test_check_repo_links.py"
HOOK = "\n".join(
    [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        "# python3 -I scripts/test_in_a_comment.py",
        "python3 -I scripts/test_alpha.py",
        "python3 -I scripts/check_alpha.py",
        "scripts/test-beta.sh",
        "  python3 -I scripts/test_check_repo_links.py",
        "scripts/rustfmt.sh --check",
        "scripts/nextest.sh run --workspace",
        "python3 -I scripts/test_gamma.py",
        "",
    ]
)
ALL = ["scripts/test_alpha.py", "scripts/test-beta.sh", KEPT, "scripts/test_gamma.py"]
STUBBED = ["scripts/test_alpha.py", "scripts/test-beta.sh", "scripts/test_gamma.py"]


def fixture(directory, hook=HOOK, files=ALL):
    """A tree with the hook and one passing-or-failing file per script test."""
    root = pathlib.Path(directory)
    (root / ".githooks").mkdir()
    (root / "scripts").mkdir()
    (root / ".githooks" / "pre-commit").write_text(hook, encoding="utf-8")
    for name in files:
        path = root / name
        # The real test fails here, so a run that passes ran the stand-in.
        path.write_text("#!/usr/bin/env bash\nexit 7\n" if name.endswith(".sh") else "raise SystemExit(7)\n", encoding="utf-8")
        if name.endswith(".sh"):
            path.chmod(0o755)
    return root


def run_stub(root, name, calls):
    """Runs a script test of the tree the way the hook does."""
    command = ["python3", "-I", name] if name.endswith(".py") else [name]
    env = {k: v for k, v in os.environ.items() if k != hook_fixture.CALLS}
    if calls is not None:
        env[hook_fixture.CALLS] = str(calls)
    return subprocess.run(command, cwd=root, env=env, capture_output=True, text=True, timeout=60, check=False)


def run_main(*argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = hook_fixture.main(list(argv))
    return code, out.getvalue(), err.getvalue()


class ScriptTests(unittest.TestCase):
    def test_python_and_shell_tests_are_found_in_the_hooks_order(self):
        self.assertEqual(hook_fixture.script_tests(HOOK), ALL)

    def test_a_check_that_is_not_a_test_is_left_out(self):
        self.assertNotIn("scripts/check_alpha.py", hook_fixture.script_tests(HOOK))

    def test_a_script_with_arguments_that_is_not_a_test_is_left_out(self):
        names = hook_fixture.script_tests(HOOK)
        self.assertFalse([n for n in names if "rustfmt" in n or "nextest" in n], names)

    def test_a_comment_naming_a_test_is_left_out(self):
        self.assertNotIn("scripts/test_in_a_comment.py", hook_fixture.script_tests(HOOK))

    def test_an_indented_line_is_read(self):
        self.assertIn(KEPT, hook_fixture.script_tests(HOOK))

    def test_a_shell_test_whose_name_holds_several_hyphens_is_found(self):
        self.assertEqual(hook_fixture.script_tests("scripts/test-scan-commit-messages.sh\n"), ["scripts/test-scan-commit-messages.sh"])

    def test_a_python_test_whose_name_holds_digits_is_found(self):
        self.assertEqual(hook_fixture.script_tests("python3 -I scripts/test_wait_for_s3.py\n"), ["scripts/test_wait_for_s3.py"])

    def test_a_hook_without_a_final_newline_is_read(self):
        self.assertEqual(hook_fixture.script_tests("python3 -I scripts/test_a.py"), ["scripts/test_a.py"])

    def test_a_hook_with_windows_line_ends_is_read(self):
        self.assertEqual(hook_fixture.script_tests("python3 -I scripts/test_a.py\r\nscripts/test-b.sh\r\n"), ["scripts/test_a.py", "scripts/test-b.sh"])

    def test_a_hook_that_runs_no_script_test_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, "runs no script test"):
            hook_fixture.script_tests("#!/usr/bin/env bash\ncargo clippy\n")

    def test_an_empty_hook_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, "runs no script test"):
            hook_fixture.script_tests("")

    def test_a_python_test_run_without_isolation_is_refused_by_line(self):
        with self.assertRaisesRegex(hook_fixture.HookError, r"line 2: .*python3 scripts/test_a\.py"):
            hook_fixture.script_tests("scripts/test-b.sh\npython3 scripts/test_a.py\n")

    def test_a_shell_test_run_through_bash_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, "line 1: "):
            hook_fixture.script_tests("bash scripts/test-b.sh\n")

    def test_a_test_run_with_arguments_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, "line 1: "):
            hook_fixture.script_tests("python3 -I scripts/test_a.py -v\n")

    def test_a_test_run_after_another_command_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, "line 1: "):
            hook_fixture.script_tests("true && scripts/test-b.sh\n")

    def test_a_test_run_twice_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, r"line 3: .*scripts/test_a\.py.*twice"):
            hook_fixture.script_tests("python3 -I scripts/test_a.py\nscripts/test-b.sh\npython3 -I scripts/test_a.py\n")

    def test_a_test_named_after_a_code_comment_on_the_line_is_refused(self):
        # The line still runs something; the reader does not guess what.
        with self.assertRaisesRegex(hook_fixture.HookError, "line 1: "):
            hook_fixture.script_tests("scripts/test-b.sh # the retry rule\n")


class Stubbed(unittest.TestCase):
    def test_every_test_but_the_kept_one_is_stubbed(self):
        self.assertEqual(hook_fixture.stubbed(HOOK), STUBBED)

    def test_the_kept_test_is_the_one_a_case_exercises(self):
        self.assertEqual(sorted(hook_fixture.KEPT), [KEPT])

    def test_each_kept_test_says_why(self):
        self.assertTrue(all(why.strip() for why in hook_fixture.KEPT.values()), hook_fixture.KEPT)

    def test_a_hook_that_no_longer_runs_a_kept_test_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, r"no longer runs scripts/test_check_repo_links\.py"):
            hook_fixture.stubbed("python3 -I scripts/test_alpha.py\n")

    def test_a_hook_that_runs_only_the_kept_test_stubs_nothing(self):
        self.assertEqual(hook_fixture.stubbed("python3 -I scripts/test_check_repo_links.py\n"), [])


class RealHook(unittest.TestCase):
    """The hook of this tree, so a change to it is read here first."""

    def setUp(self):
        self.root = HERE.parent
        self.hook = (self.root / hook_fixture.HOOK).read_text(encoding="utf-8")

    def test_the_real_hook_is_read_without_a_refusal(self):
        self.assertGreater(len(hook_fixture.script_tests(self.hook)), 10)

    def test_every_script_test_the_real_hook_runs_is_a_file(self):
        missing = [n for n in hook_fixture.script_tests(self.hook) if not (self.root / n).is_file()]
        self.assertEqual(missing, [])

    def test_every_mention_of_a_script_test_in_the_real_hook_is_read(self):
        lines = [line for line in self.hook.splitlines() if not line.lstrip().startswith("#")]
        mentions = sum(line.count("scripts/test_") + line.count("scripts/test-") for line in lines)
        self.assertEqual(mentions, len(hook_fixture.script_tests(self.hook)))

    def test_the_real_hook_runs_this_test(self):
        self.assertIn("scripts/test_hook_fixture.py", hook_fixture.script_tests(self.hook))

    def test_the_real_hook_keeps_the_test_a_case_exercises(self):
        self.assertNotIn(KEPT, hook_fixture.stubbed(self.hook))

    def test_the_hook_test_names_the_kept_tests_refusal(self):
        # The case that needs the kept test is still in scripts/test-hooks.sh.
        text = (self.root / "scripts" / "test-hooks.sh").read_text(encoding="utf-8")
        self.assertIn("test_every_path_bestpractices_links_to_exists", text)
        kept = (self.root / KEPT).read_text(encoding="utf-8")
        self.assertIn("def test_every_path_bestpractices_links_to_exists", kept)

    def test_the_hook_tests_stub_after_the_first_commit_and_check_the_calls(self):
        text = (self.root / "scripts" / "test-hooks.sh").read_text(encoding="utf-8")
        first = text.index('git commit -q -m "Start the fixture"')
        stub = text.index("scripts/hook_fixture.py\" stub")
        check = text.index("scripts/hook_fixture.py\" check")
        case = text.index('expect accept "clean change"')
        self.assertLess(first, stub, "the real script tests run once, in the first commit")
        self.assertLess(stub, case, "the cases run against the stand-ins")
        self.assertLess(case, check, "the calls are checked after a case the hook accepts")


class StubText(unittest.TestCase):
    def test_a_python_stub_is_python_and_names_its_test(self):
        text = hook_fixture.stub_text("scripts/test_alpha.py")
        compile(text, "stub", "exec")
        self.assertIn("scripts/test_alpha.py", text)

    def test_a_shell_stub_starts_with_a_bash_line(self):
        text = hook_fixture.stub_text("scripts/test-beta.sh")
        self.assertTrue(text.startswith("#!/usr/bin/env bash\n"), text)
        self.assertIn("scripts/test-beta.sh", text)

    def test_a_stub_ends_with_a_newline(self):
        for name in ("scripts/test_alpha.py", "scripts/test-beta.sh"):
            self.assertTrue(hook_fixture.stub_text(name).endswith("\n"), name)

    def test_a_stub_says_what_it_stands_in_for(self):
        for name in ("scripts/test_alpha.py", "scripts/test-beta.sh"):
            self.assertIn("scripts/hook_fixture.py", hook_fixture.stub_text(name), name)

    def test_a_path_of_another_kind_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, r"scripts/test_alpha\.rb"):
            hook_fixture.stub_text("scripts/test_alpha.rb")

    def test_a_path_holding_a_quote_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, "not a script test's path"):
            hook_fixture.stub_text("scripts/test_a'; rm -rf x; '.sh")

    def test_a_path_outside_scripts_is_refused(self):
        with self.assertRaisesRegex(hook_fixture.HookError, "not a script test's path"):
            hook_fixture.stub_text("../test_alpha.py")


class Install(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = fixture(self.directory.name)
        self.calls = self.root / "calls"

    def test_install_returns_the_stubbed_tests(self):
        self.assertEqual(hook_fixture.install(self.root), STUBBED)

    def test_each_stubbed_file_becomes_its_stub(self):
        hook_fixture.install(self.root)
        for name in STUBBED:
            self.assertEqual((self.root / name).read_text(encoding="utf-8"), hook_fixture.stub_text(name), name)

    def test_the_kept_test_is_left_as_it_was(self):
        before = (self.root / KEPT).read_bytes()
        hook_fixture.install(self.root)
        self.assertEqual((self.root / KEPT).read_bytes(), before)

    def test_the_hook_is_left_as_it_was(self):
        hook_fixture.install(self.root)
        self.assertEqual((self.root / hook_fixture.HOOK).read_text(encoding="utf-8"), HOOK)

    def test_a_shell_stub_keeps_the_executable_bit(self):
        hook_fixture.install(self.root)
        mode = (self.root / "scripts/test-beta.sh").stat().st_mode
        self.assertTrue(mode & stat.S_IXUSR, oct(mode))

    def test_a_python_stub_run_as_the_hook_runs_it_passes_and_records_its_name(self):
        hook_fixture.install(self.root)
        done = run_stub(self.root, "scripts/test_alpha.py", self.calls)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(self.calls.read_text(encoding="utf-8"), "scripts/test_alpha.py\n")

    def test_a_shell_stub_run_as_the_hook_runs_it_passes_and_records_its_name(self):
        hook_fixture.install(self.root)
        done = run_stub(self.root, "scripts/test-beta.sh", self.calls)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(self.calls.read_text(encoding="utf-8"), "scripts/test-beta.sh\n")

    def test_calls_are_appended_in_the_order_run(self):
        hook_fixture.install(self.root)
        for name in ("scripts/test_gamma.py", "scripts/test-beta.sh", "scripts/test_gamma.py"):
            run_stub(self.root, name, self.calls)
        self.assertEqual(
            self.calls.read_text(encoding="utf-8").splitlines(),
            ["scripts/test_gamma.py", "scripts/test-beta.sh", "scripts/test_gamma.py"],
        )

    def test_a_stub_prints_nothing_when_it_passes(self):
        hook_fixture.install(self.root)
        for name in STUBBED:
            done = run_stub(self.root, name, self.calls)
            self.assertEqual((done.stdout, done.stderr), ("", ""), name)

    def test_a_python_stub_fails_when_no_calls_file_is_named(self):
        hook_fixture.install(self.root)
        done = run_stub(self.root, "scripts/test_alpha.py", None)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn(hook_fixture.CALLS, done.stderr)

    def test_a_shell_stub_fails_when_no_calls_file_is_named(self):
        hook_fixture.install(self.root)
        done = run_stub(self.root, "scripts/test-beta.sh", None)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn(hook_fixture.CALLS, done.stderr)

    def test_a_stub_fails_when_the_calls_file_cannot_be_written(self):
        hook_fixture.install(self.root)
        for name in ("scripts/test_alpha.py", "scripts/test-beta.sh"):
            done = run_stub(self.root, name, self.root / "no-such-directory" / "calls")
            self.assertNotEqual(done.returncode, 0, name)

    def test_before_install_the_fixtures_tests_fail(self):
        # What makes the passing runs above evidence that a stand-in ran.
        self.assertEqual(run_stub(self.root, "scripts/test_alpha.py", self.calls).returncode, 7)
        self.assertFalse(self.calls.exists())

    def test_installing_twice_gives_the_same_files(self):
        hook_fixture.install(self.root)
        once = {name: (self.root / name).read_bytes() for name in ALL}
        self.assertEqual(hook_fixture.install(self.root), STUBBED)
        self.assertEqual({name: (self.root / name).read_bytes() for name in ALL}, once)

    def test_a_missing_script_test_is_refused_and_nothing_is_written(self):
        (self.root / "scripts/test_gamma.py").unlink()
        before = (self.root / "scripts/test_alpha.py").read_bytes()
        with self.assertRaisesRegex(hook_fixture.HookError, r"scripts/test_gamma\.py"):
            hook_fixture.install(self.root)
        self.assertEqual((self.root / "scripts/test_alpha.py").read_bytes(), before)

    def test_a_tree_without_the_hook_is_refused(self):
        (self.root / hook_fixture.HOOK).unlink()
        with self.assertRaisesRegex(hook_fixture.HookError, r"\.githooks/pre-commit"):
            hook_fixture.install(self.root)

    def test_a_script_test_that_is_a_symbolic_link_is_refused(self):
        # Writing through it would change a file outside the fixture.
        outside = pathlib.Path(self.directory.name) / "outside.py"
        outside.write_text("real\n", encoding="utf-8")
        (self.root / "scripts/test_alpha.py").unlink()
        (self.root / "scripts/test_alpha.py").symlink_to(outside)
        with self.assertRaisesRegex(hook_fixture.HookError, "symbolic link"):
            hook_fixture.install(self.root)
        self.assertEqual(outside.read_text(encoding="utf-8"), "real\n")


class Findings(unittest.TestCase):
    def calls(self, *names):
        return "".join(name + "\n" for name in names)

    def test_each_name_called_once_is_fine(self):
        self.assertEqual(hook_fixture.findings(STUBBED, self.calls(*STUBBED), 1), [])

    def test_each_name_called_twice_is_fine_for_two_runs(self):
        self.assertEqual(hook_fixture.findings(STUBBED, self.calls(*STUBBED, *STUBBED), 2), [])

    def test_no_calls_is_fine_for_no_runs(self):
        self.assertEqual(hook_fixture.findings(STUBBED, "", 0), [])

    def test_a_name_never_called_is_found(self):
        found = hook_fixture.findings(STUBBED, self.calls(*STUBBED[:2]), 1)
        self.assertEqual(found, ["scripts/test_gamma.py: the hook ran it 0 times, not 1"])

    def test_a_name_called_too_often_is_found(self):
        found = hook_fixture.findings(STUBBED, self.calls(*STUBBED, STUBBED[0]), 1)
        self.assertEqual(found, ["scripts/test_alpha.py: the hook ran it 2 times, not 1"])

    def test_an_empty_calls_file_finds_every_name(self):
        self.assertEqual(len(hook_fixture.findings(STUBBED, "", 1)), len(STUBBED))

    def test_a_call_by_a_name_that_is_not_stubbed_is_found(self):
        found = hook_fixture.findings(STUBBED, self.calls(*STUBBED, "scripts/test_other.py"), 1)
        self.assertEqual(found, ["scripts/test_other.py: recorded a call, and is not a stubbed script test"])

    def test_blank_lines_in_the_calls_are_not_calls(self):
        self.assertEqual(hook_fixture.findings(STUBBED, "\n" + self.calls(*STUBBED) + "\n", 1), [])

    def test_findings_come_in_the_hooks_order(self):
        found = hook_fixture.findings(STUBBED, "", 1)
        self.assertEqual([f.split(":")[0] for f in found], STUBBED)

    def test_no_names_and_no_calls_is_fine(self):
        self.assertEqual(hook_fixture.findings([], "", 3), [])


class Main(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = fixture(self.directory.name)
        self.calls = self.root / "calls"

    def test_stub_prints_each_stubbed_test_and_returns_zero(self):
        code, out, err = run_main("stub", str(self.root))
        self.assertEqual((code, out.splitlines(), err), (0, STUBBED, ""))

    def test_stub_installs_the_stubs(self):
        run_main("stub", str(self.root))
        self.assertEqual(run_stub(self.root, "scripts/test_alpha.py", self.calls).returncode, 0)

    def test_check_passes_when_each_stub_ran_once(self):
        self.calls.write_text("".join(n + "\n" for n in STUBBED), encoding="utf-8")
        code, out, err = run_main("check", str(self.root), str(self.calls), "1")
        self.assertEqual((code, err), (0, ""))
        self.assertIn("3 script tests", out)

    def test_check_fails_and_names_a_stub_that_did_not_run(self):
        self.calls.write_text("scripts/test_alpha.py\n", encoding="utf-8")
        code, _, err = run_main("check", str(self.root), str(self.calls), "1")
        self.assertEqual(code, 1)
        self.assertIn("scripts/test-beta.sh: the hook ran it 0 times, not 1", err)
        self.assertIn("scripts/test_gamma.py: the hook ran it 0 times, not 1", err)

    def test_check_treats_a_missing_calls_file_as_no_calls(self):
        code, _, err = run_main("check", str(self.root), str(self.calls), "1")
        self.assertEqual(code, 1)
        self.assertEqual(len(err.strip().splitlines()), len(STUBBED), err)

    def test_check_passes_with_no_calls_file_for_no_runs(self):
        code, _, err = run_main("check", str(self.root), str(self.calls), "0")
        self.assertEqual((code, err), (0, ""))

    def test_check_refuses_a_count_that_is_not_a_number(self):
        code, _, err = run_main("check", str(self.root), str(self.calls), "once")
        self.assertEqual(code, 2)
        self.assertIn("usage:", err)

    def test_check_refuses_a_negative_count(self):
        code, _, err = run_main("check", str(self.root), str(self.calls), "-1")
        self.assertEqual(code, 2)
        self.assertIn("usage:", err)

    def test_no_arguments_prints_the_usage(self):
        code, _, err = run_main()
        self.assertEqual(code, 2)
        self.assertIn("usage:", err)

    def test_an_unknown_command_prints_the_usage(self):
        code, _, err = run_main("unstub", str(self.root))
        self.assertEqual(code, 2)
        self.assertIn("usage:", err)

    def test_stub_with_an_extra_argument_prints_the_usage_and_writes_nothing(self):
        before = (self.root / "scripts/test_alpha.py").read_bytes()
        code, _, err = run_main("stub", str(self.root), "extra")
        self.assertEqual(code, 2)
        self.assertIn("usage:", err)
        self.assertEqual((self.root / "scripts/test_alpha.py").read_bytes(), before)

    def test_a_hook_this_reader_refuses_fails_with_its_reason(self):
        (self.root / hook_fixture.HOOK).write_text("bash scripts/test-beta.sh\n", encoding="utf-8")
        code, _, err = run_main("stub", str(self.root))
        self.assertEqual(code, 1)
        self.assertIn("hook_fixture: ", err)
        self.assertIn("line 1: ", err)

    def test_a_tree_that_is_not_a_fixture_fails_with_its_reason(self):
        code, _, err = run_main("stub", str(self.root / "nowhere"))
        self.assertEqual(code, 1)
        self.assertIn(".githooks/pre-commit", err)

    def test_run_as_a_script_it_stubs_the_tree(self):
        done = subprocess.run(
            ["python3", "-I", str(HERE / "hook_fixture.py"), "stub", str(self.root)],
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
        self.assertEqual((done.returncode, done.stdout.splitlines()), (0, STUBBED), done.stderr)


if __name__ == "__main__":
    unittest.main()
