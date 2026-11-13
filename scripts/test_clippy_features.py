"""Tests scripts/clippy_features.py: the commit hook and CI lint the same
feature sets, the rows of .github/clippy-feature-sets.txt, each with
warnings denied; a feature set linted anywhere else, a row CI never runs,
and a hook that leaves the list out are each refused.

Run with: python3 -I scripts/test_clippy_features.py
"""

import importlib.util
import io
import pathlib
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("clippy_features", HERE / "clippy_features.py")
clippy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(clippy)

LIST = (
    "# The feature sets.\n"
    "\n"
    "backend: -p vcon-server --all-targets --locked --features backend-tests\n"
    "backend: -p vcon-server --all-targets --locked --no-default-features\n"
    "postgres: -p vcon-server --all-targets --locked --features postgres\n"
)
HOOK = (
    "cargo clippy --workspace --all-targets --locked --quiet -- -D warnings\n"
    "python3 -I scripts/clippy_features.py run --quiet\n"
    "  cargo clippy --manifest-path fuzz/Cargo.toml --all-targets --locked --quiet -- -D warnings\n"
)
CI = (
    "jobs:\n"
    "  rust:\n"
    "    steps:\n"
    "      - run: |\n"
    "          cargo clippy --workspace --all-targets --locked -- -D warnings\n"
    "  backend:\n"
    "    steps:\n"
    "      - run: python3 -I scripts/clippy_features.py run backend\n"
    "  postgres:\n"
    "    steps:\n"
    "      - run: |\n"
    "          python3 -I scripts/clippy_features.py run postgres\n"
)
BACKEND_TESTS = ["-p", "vcon-server", "--all-targets", "--locked", "--features", "backend-tests"]
NO_DEFAULT = ["-p", "vcon-server", "--all-targets", "--locked", "--no-default-features"]
POSTGRES = ["-p", "vcon-server", "--all-targets", "--locked", "--features", "postgres"]


def problems(rows=LIST, hook=HOOK, **workflows):
    return clippy.problems(rows, hook, workflows or {"ci.yml": CI})


def refusal(text):
    """The message parse refuses text with."""
    try:
        clippy.parse(text)
    except clippy.ListError as error:
        return str(error)
    return None


class Recorder:
    """Stands in for the command runner: records each command, answers codes."""

    def __init__(self, *codes):
        self.codes = list(codes)
        self.commands = []
        self.directories = []

    def __call__(self, command, cwd=None):
        self.commands.append(command)
        self.directories.append(cwd)
        return self.codes.pop(0) if self.codes else 0


class Parse(unittest.TestCase):
    def test_rows_are_read_in_order_with_their_groups(self):
        self.assertEqual(
            clippy.parse(LIST),
            [("backend", BACKEND_TESTS), ("backend", NO_DEFAULT), ("postgres", POSTGRES)],
        )

    def test_comments_and_blank_lines_are_left_out(self):
        self.assertEqual(clippy.parse("# a\n\n   \n  # b\nx: --all-targets --locked --features a\n"), [("x", ["--all-targets", "--locked", "--features", "a"])])

    def test_spaces_around_a_row_and_between_arguments_do_not_matter(self):
        self.assertEqual(clippy.parse("  x:   --all-targets\t--locked   --features  a  \n"), [("x", ["--all-targets", "--locked", "--features", "a"])])

    def test_a_list_without_a_final_newline_is_read(self):
        self.assertEqual(len(clippy.parse(LIST.rstrip("\n"))), 3)

    def test_windows_line_ends_are_read(self):
        self.assertEqual(clippy.parse(LIST.replace("\n", "\r\n")), clippy.parse(LIST))

    def test_a_feature_list_with_commas_is_one_argument(self):
        rows = clippy.parse("x: --all-targets --locked --features postgres,backend-tests\n")
        self.assertEqual(rows[0][1][-1], "postgres,backend-tests")

    def test_features_written_with_an_equals_sign_are_read(self):
        self.assertEqual(clippy.parse("x: --all-targets --locked --features=a\n")[0][1][-1], "--features=a")

    def test_a_group_name_may_hold_digits_and_hyphens(self):
        self.assertEqual(clippy.parse("pg-16: --all-targets --locked --features a\n")[0][0], "pg-16")

    def test_an_empty_list_is_refused(self):
        self.assertIn("no row", refusal(""))

    def test_a_list_of_comments_only_is_refused(self):
        self.assertIn("no row", refusal("# nothing\n\n"))

    def test_a_row_without_a_group_is_refused_with_its_line(self):
        self.assertIn("line 2", refusal("# a\n-p vcon-server --all-targets --locked --features a\n"))

    def test_a_group_name_with_other_characters_is_refused(self):
        for name in ("Backend", "a b", "1a", "a_b", ""):
            with self.subTest(name=name):
                self.assertIn("group", refusal(f"{name}: --all-targets --locked --features a\n") or "")

    def test_a_row_with_no_arguments_is_refused(self):
        self.assertIn("no arguments", refusal("x:\n"))

    def test_a_row_that_holds_the_lint_arguments_is_refused(self):
        self.assertIn("`--`", refusal("x: --all-targets --locked --features a -- -D warnings\n"))

    def test_a_row_that_holds_quiet_is_refused(self):
        self.assertIn("--quiet", refusal("x: --all-targets --locked --features a --quiet\n"))
        self.assertIn("--quiet", refusal("x: --all-targets --locked --features a -q\n"))

    def test_a_row_that_leaves_out_the_test_targets_is_refused(self):
        self.assertIn("--all-targets", refusal("x: --locked --features a\n"))

    def test_a_row_that_leaves_out_the_lock_file_is_refused(self):
        self.assertIn("--locked", refusal("x: --all-targets --features a\n"))

    def test_a_row_of_the_default_features_is_refused(self):
        self.assertIn("feature set", refusal("x: --workspace --all-targets --locked\n"))

    def test_features_without_a_value_are_refused(self):
        self.assertIn("--features", refusal("x: --all-targets --locked --features\n"))
        self.assertIn("--features", refusal("x: --all-targets --locked --features --no-default-features\n"))
        self.assertIn("--features", refusal("x: --all-targets --locked --features=\n"))

    def test_a_shell_character_in_a_row_is_refused(self):
        for bad in ("$(id)", "a;b", "`id`", "a|b", "'a'", '"a"', "a&b", ">out", "a\\b"):
            with self.subTest(bad=bad):
                self.assertIn("character", refusal(f"x: --all-targets --locked --features {bad}\n") or "")

    def test_the_same_arguments_twice_are_refused_even_in_two_groups(self):
        text = "x: --all-targets --locked --features a\ny: --all-targets --locked --features a\n"
        self.assertIn("line 2", refusal(text))
        self.assertIn("twice", refusal(text))

    def test_every_refusal_names_the_list(self):
        self.assertIn(".github/clippy-feature-sets.txt", refusal("x:\n"))
        self.assertIn(".github/clippy-feature-sets.txt", refusal(""))

    def test_no_default_features_with_features_is_one_feature_set(self):
        rows = clippy.parse("x: --all-targets --locked --no-default-features --features a\n")
        self.assertEqual(rows[0][1], ["--all-targets", "--locked", "--no-default-features", "--features", "a"])

    def test_all_features_is_a_feature_set(self):
        self.assertEqual(len(clippy.parse("x: --all-targets --locked --all-features\n")), 1)


class Commands(unittest.TestCase):
    def setUp(self):
        self.rows = clippy.parse(LIST)

    def test_every_row_runs_with_warnings_denied_when_no_group_is_named(self):
        self.assertEqual(
            clippy.commands(self.rows, [], quiet=False),
            [
                ["cargo", "clippy", *BACKEND_TESTS, "--", "-D", "warnings"],
                ["cargo", "clippy", *NO_DEFAULT, "--", "-D", "warnings"],
                ["cargo", "clippy", *POSTGRES, "--", "-D", "warnings"],
            ],
        )

    def test_a_named_group_runs_its_rows_only(self):
        self.assertEqual(clippy.commands(self.rows, ["postgres"], quiet=False), [["cargo", "clippy", *POSTGRES, "--", "-D", "warnings"]])

    def test_two_named_groups_run_in_the_list_s_order(self):
        got = clippy.commands(self.rows, ["postgres", "backend"], quiet=False)
        self.assertEqual([c[-4] for c in got], ["backend-tests", "--no-default-features", "postgres"])

    def test_a_group_named_twice_runs_once(self):
        self.assertEqual(len(clippy.commands(self.rows, ["backend", "backend"], quiet=False)), 2)

    def test_quiet_goes_before_the_lint_arguments(self):
        got = clippy.commands(self.rows, ["postgres"], quiet=True)
        self.assertEqual(got, [["cargo", "clippy", *POSTGRES, "--quiet", "--", "-D", "warnings"]])

    def test_a_group_the_list_does_not_hold_is_refused(self):
        with self.assertRaises(clippy.ListError) as raised:
            clippy.commands(self.rows, ["backend", "sqlite"], quiet=False)
        self.assertIn("sqlite", str(raised.exception))
        self.assertIn("backend, postgres", str(raised.exception))

    def test_every_command_denies_warnings(self):
        for command in clippy.commands(self.rows, [], quiet=True):
            self.assertEqual(command[-3:], ["--", "-D", "warnings"])


class Run(unittest.TestCase):
    def setUp(self):
        self.rows = clippy.parse(LIST)
        self.err = io.StringIO()

    def test_each_command_runs_in_the_repository(self):
        runner = Recorder()
        self.assertEqual(clippy.run(self.rows, [], False, runner, "/a/root", self.err), 0)
        self.assertEqual(runner.commands, clippy.commands(self.rows, [], quiet=False))
        self.assertEqual(runner.directories, ["/a/root"] * 3)

    def test_the_first_failure_ends_the_run_with_its_code(self):
        runner = Recorder(0, 101, 0)
        self.assertEqual(clippy.run(self.rows, [], False, runner, ".", self.err), 101)
        self.assertEqual(len(runner.commands), 2)

    def test_a_failure_names_the_command_to_run_again(self):
        clippy.run(self.rows, ["postgres"], True, Recorder(101), ".", self.err)
        self.assertIn("cargo clippy -p vcon-server --all-targets --locked --features postgres --quiet -- -D warnings", self.err.getvalue())
        self.assertIn("failed", self.err.getvalue())

    def test_a_run_that_passes_says_nothing_when_quiet(self):
        clippy.run(self.rows, [], True, Recorder(), ".", self.err)
        self.assertEqual(self.err.getvalue(), "")

    def test_a_run_names_each_command_when_not_quiet(self):
        clippy.run(self.rows, [], False, Recorder(), ".", self.err)
        self.assertEqual(self.err.getvalue().count("cargo clippy"), 3)

    def test_a_command_ended_by_a_signal_fails_the_run(self):
        self.assertNotEqual(clippy.run(self.rows, [], False, Recorder(-9), ".", self.err), 0)
        self.assertGreater(clippy.run(self.rows, [], False, Recorder(-9), ".", self.err), 0)

    def test_an_unknown_group_runs_nothing(self):
        runner = Recorder()
        with self.assertRaises(clippy.ListError):
            clippy.run(self.rows, ["sqlite"], False, runner, ".", self.err)
        self.assertEqual(runner.commands, [])


class Problems(unittest.TestCase):
    def test_the_rule_holds(self):
        self.assertEqual(problems(), [])

    def test_one_step_may_run_every_row(self):
        ci = "      - run: python3 -I scripts/clippy_features.py run\n"
        self.assertEqual(problems(**{"ci.yml": ci}), [])

    def test_two_workflows_may_share_the_groups(self):
        a = "      - run: python3 -I scripts/clippy_features.py run backend\n"
        b = "      - run: python3 -I scripts/clippy_features.py run postgres\n"
        self.assertEqual(problems(**{"a.yml": a, "b.yml": b}), [])

    def test_one_step_may_run_two_groups(self):
        ci = "          python3 -I scripts/clippy_features.py run backend postgres\n"
        self.assertEqual(problems(**{"ci.yml": ci}), [])

    def test_a_workflow_may_run_quietly(self):
        ci = "          python3 -I scripts/clippy_features.py run --quiet\n"
        self.assertEqual(problems(**{"ci.yml": ci}), [])

    def test_a_comment_that_names_a_feature_set_is_no_command(self):
        hook = HOOK + "# cargo clippy --features postgres -- -D warnings\n"
        ci = CI + "        # cargo clippy -p vcon-server --features postgres\n"
        self.assertEqual(problems(hook=hook, **{"ci.yml": ci}), [])

    def test_the_default_and_fuzz_lints_stay_where_they_are(self):
        ci = CI + "          cargo clippy --manifest-path fuzz/Cargo.toml --all-targets --locked -- -D warnings\n"
        self.assertEqual(problems(**{"ci.yml": ci}), [])

    def test_a_feature_set_built_and_not_linted_is_no_problem(self):
        ci = CI + "          cargo build --locked -p vcon-server --features postgres\n          scripts/nextest.sh run -p vcon-server --features postgres,backend-tests\n"
        self.assertEqual(problems(**{"ci.yml": ci}), [])

    def test_the_check_step_is_no_run(self):
        ci = CI + "          python3 -I scripts/clippy_features.py check\n          python3 -I scripts/test_clippy_features.py\n"
        self.assertEqual(problems(**{"ci.yml": ci}), [])

    def test_a_lint_argument_is_no_feature_selection(self):
        ci = CI + "          cargo clippy --workspace --all-targets --locked -- -F clippy::dbg_macro -D warnings\n"
        hook = HOOK + "cargo clippy --workspace --locked --quiet -- -F warnings\n"
        self.assertEqual(problems(hook=hook, **{"ci.yml": ci}), [])

    def test_a_second_hook_run_in_another_form_is_refused_with_its_line(self):
        hook = HOOK + "python3 -I scripts/clippy_features.py run backend || true\n"
        self.assertEqual(problems(hook=hook), [".githooks/pre-commit line 4: runs scripts/clippy_features.py in a form this reader does not know: python3 -I scripts/clippy_features.py run backend || true"])

    def test_a_hook_that_leaves_the_list_out_is_refused(self):
        hook = "cargo clippy --workspace --all-targets --locked --quiet -- -D warnings\n"
        self.assertEqual(problems(hook=hook), [".githooks/pre-commit: never runs `python3 -I scripts/clippy_features.py run --quiet`, which lints every row of .github/clippy-feature-sets.txt"])

    def test_a_hook_that_runs_one_group_only_is_refused(self):
        hook = HOOK.replace("run --quiet", "run --quiet backend")
        found = problems(hook=hook)
        self.assertTrue(any("pre-commit" in p and "every row" in p for p in found), found)

    def test_a_hook_whose_run_is_commented_out_is_refused(self):
        hook = HOOK.replace("python3 -I scripts/clippy_features.py", "# python3 -I scripts/clippy_features.py")
        self.assertTrue(any("never runs" in p for p in problems(hook=hook)))

    def test_a_hook_that_ignores_the_result_is_refused(self):
        hook = HOOK.replace("run --quiet\n", "run --quiet || true\n")
        self.assertTrue(any("pre-commit" in p for p in problems(hook=hook)))

    def test_a_hook_that_lints_a_feature_set_itself_is_refused_with_its_line(self):
        hook = HOOK + "cargo clippy -p vcon-server --all-targets --locked --features postgres -- -D warnings\n"
        self.assertEqual(problems(hook=hook), [".githooks/pre-commit line 4: lints a feature set outside .github/clippy-feature-sets.txt: cargo clippy -p vcon-server --all-targets --locked --features postgres -- -D warnings"])

    def test_a_workflow_that_lints_a_feature_set_itself_is_refused(self):
        for flags in ("--features backend-tests", "--no-default-features", "--all-features", "-F postgres", "--features=postgres", "-Fpostgres"):
            with self.subTest(flags=flags):
                ci = CI + f"          cargo clippy -p vcon-server --all-targets --locked {flags} -- -D warnings\n"
                found = problems(**{"ci.yml": ci})
                self.assertEqual(len(found), 1, found)
                self.assertIn(".github/workflows/ci.yml line 13: lints a feature set outside", found[0])

    def test_a_toolchain_argument_does_not_hide_a_lint(self):
        ci = CI + "          cargo +nightly clippy --features postgres -- -D warnings\n"
        self.assertTrue(any("line 13" in p for p in problems(**{"ci.yml": ci})))

    def test_a_group_no_workflow_runs_is_refused(self):
        ci = CI.replace("run postgres", "run backend")
        self.assertEqual(problems(**{"ci.yml": ci}), ["no workflow runs the group `postgres` of .github/clippy-feature-sets.txt: add a step `python3 -I scripts/clippy_features.py run postgres`"])

    def test_a_workflow_that_names_an_unknown_group_is_refused(self):
        ci = CI + "          python3 -I scripts/clippy_features.py run sqlite\n"
        self.assertEqual(problems(**{"ci.yml": ci}), [".github/workflows/ci.yml line 13: runs the group `sqlite`, which .github/clippy-feature-sets.txt does not hold"])

    def test_a_workflow_run_in_a_form_this_reader_does_not_know_is_refused(self):
        for line in (
            "python3 scripts/clippy_features.py run backend",
            "python3 -I scripts/clippy_features.py run backend || true",
            "python3 -I scripts/clippy_features.py lint backend",
            "python3 -I scripts/clippy_features.py run $GROUP",
            "python3 -I scripts/clippy_features.py",
        ):
            with self.subTest(line=line):
                found = problems(**{"ci.yml": CI + f"          {line}\n"})
                self.assertTrue(any("line 13" in p and "form" in p for p in found), found)

    def test_no_workflow_at_all_is_refused_for_each_group(self):
        found = clippy.problems(LIST, HOOK, {})
        self.assertEqual(len(found), 2, found)

    def test_a_list_that_does_not_parse_is_the_one_problem(self):
        found = problems(rows="x:\n")
        self.assertEqual(len(found), 1, found)
        self.assertIn("no arguments", found[0])

    def test_a_step_with_a_name_on_the_run_line_is_read(self):
        ci = CI.replace("- run: python3 -I scripts/clippy_features.py run backend", "- run: python3 -I scripts/clippy_features.py run sqlite")
        self.assertTrue(any("sqlite" in p and "line 8" in p for p in problems(**{"ci.yml": ci})))

    def test_each_problem_is_reported_not_only_the_first(self):
        hook = "cargo clippy --features a -- -D warnings\n"
        ci = "          cargo clippy --no-default-features -- -D warnings\n"
        self.assertEqual(len(problems(hook=hook, **{"ci.yml": ci})), 5)

    def test_workflows_are_reported_in_name_order(self):
        bad = "          cargo clippy --all-features -- -D warnings\n"
        found = problems(**{"b.yml": bad, "a.yml": bad, "ci.yml": CI})
        self.assertEqual([p.split(" line")[0] for p in found], [".github/workflows/a.yml", ".github/workflows/b.yml"])


class Main(unittest.TestCase):
    def tree(self, rows=LIST, hook=HOOK, ci=CI):
        holder = tempfile.TemporaryDirectory()
        self.addCleanup(holder.cleanup)
        root = pathlib.Path(holder.name)
        (root / ".github/workflows").mkdir(parents=True)
        (root / ".githooks").mkdir()
        if rows is not None:
            (root / ".github/clippy-feature-sets.txt").write_text(rows, encoding="utf-8")
        if hook is not None:
            (root / ".githooks/pre-commit").write_text(hook, encoding="utf-8")
        if ci is not None:
            (root / ".github/workflows/ci.yml").write_text(ci, encoding="utf-8")
        return root

    def main(self, *argv, root=None, runner=None):
        out, err = io.StringIO(), io.StringIO()
        runner = runner or Recorder()
        code = clippy.main(list(argv), root or self.tree(), runner, out, err)
        return code, out.getvalue(), err.getvalue(), runner

    def test_run_lints_every_row(self):
        code, _, _, runner = self.main("run")
        self.assertEqual((code, len(runner.commands)), (0, 3))

    def test_run_quiet_passes_quiet_to_cargo(self):
        code, _, err, runner = self.main("run", "--quiet")
        self.assertEqual(code, 0)
        self.assertTrue(all("--quiet" in c for c in runner.commands))
        self.assertEqual(err, "")

    def test_run_with_a_group_lints_its_rows(self):
        _, _, _, runner = self.main("run", "backend")
        self.assertEqual(len(runner.commands), 2)

    def test_run_runs_cargo_in_the_repository(self):
        root = self.tree()
        _, _, _, runner = self.main("run", root=root)
        self.assertEqual(runner.directories, [root] * 3)

    def test_run_answers_the_failing_command_s_code(self):
        code, _, err, _ = self.main("run", runner=Recorder(101))
        self.assertEqual(code, 101)
        self.assertIn("failed", err)

    def test_run_with_an_unknown_group_exits_2_and_runs_nothing(self):
        code, _, err, runner = self.main("run", "sqlite")
        self.assertEqual((code, runner.commands), (2, []))
        self.assertIn("sqlite", err)

    def test_run_without_the_list_exits_2(self):
        code, _, err, runner = self.main("run", root=self.tree(rows=None))
        self.assertEqual((code, runner.commands), (2, []))
        self.assertIn("clippy-feature-sets.txt", err)

    def test_run_with_a_list_that_does_not_parse_exits_2(self):
        code, _, err, runner = self.main("run", root=self.tree(rows="x:\n"))
        self.assertEqual((code, runner.commands), (2, []))
        self.assertIn("no arguments", err)

    def test_run_with_an_unknown_option_is_a_usage_error(self):
        code, _, err, runner = self.main("run", "--verbose")
        self.assertEqual((code, runner.commands), (2, []))
        self.assertIn("usage", err)

    def test_no_command_and_an_unknown_command_are_usage_errors(self):
        for argv in ((), ("lint",), ("check", "more")):
            with self.subTest(argv=argv):
                code, _, err, _ = self.main(*argv)
                self.assertEqual(code, 2)
                self.assertIn("usage", err)

    def test_check_passes_and_says_what_it_read(self):
        code, out, err, runner = self.main("check")
        self.assertEqual((code, err, runner.commands), (0, "", []))
        self.assertIn("3 feature sets", out)

    def test_check_names_each_problem_and_exits_1(self):
        code, _, err, _ = self.main("check", root=self.tree(hook="true\n"))
        self.assertEqual(code, 1)
        self.assertIn("never runs", err)

    def test_check_without_the_hook_exits_1(self):
        code, _, err, _ = self.main("check", root=self.tree(hook=None))
        self.assertEqual(code, 1)
        self.assertIn(".githooks/pre-commit", err)

    def test_check_reads_every_workflow(self):
        root = self.tree(ci=CI.replace("run postgres", "run backend"))
        (root / ".github/workflows/other.yml").write_text("      - run: python3 -I scripts/clippy_features.py run postgres\n", encoding="utf-8")
        self.assertEqual(self.main("check", root=root)[0], 0)

    def test_check_without_a_workflow_exits_1(self):
        code, _, err, _ = self.main("check", root=self.tree(ci=None))
        self.assertEqual(code, 1)
        self.assertIn("no workflow runs the group", err)


class Repository(unittest.TestCase):
    def test_this_repository_keeps_the_rule(self):
        self.assertEqual(clippy.repository_problems(HERE.parent), [])

    def test_the_list_holds_the_feature_sets_ci_linted_before_it(self):
        rows = clippy.parse((HERE.parent / clippy.LIST).read_text(encoding="utf-8"))
        selections = sorted(" ".join(args[args.index("--locked") + 1 :]) for _, args in rows)
        self.assertEqual(
            selections,
            ["--features backend-tests", "--features postgres", "--features postgres,backend-tests", "--no-default-features"],
        )


if __name__ == "__main__":
    unittest.main()
