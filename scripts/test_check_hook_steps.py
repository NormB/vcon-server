"""Tests scripts/check_hook_steps.py: every script a workflow runs is run by
a git hook too, or is listed in CI_ONLY with the reason it runs only in CI.

Run with: python3 -I scripts/test_check_hook_steps.py
"""

import importlib.util
import io
import contextlib
import pathlib
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_hook_steps", HERE / "check_hook_steps.py")
chs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chs)


def workflow(*runs):
    """A parsed workflow with one job whose steps run the texts given."""
    return {"jobs": {"scan": {"steps": [{"uses": "actions/checkout@0123"}] + [{"run": run} for run in runs]}}}


def action(*runs):
    """A parsed composite action whose steps run the texts given."""
    return {"runs": {"using": "composite", "steps": [{"run": run, "shell": "bash"} for run in runs]}}


HOOK = "#!/usr/bin/env bash\npython3 -I scripts/test_a.py\npython3 -I scripts/check_a.py --walk\nscripts/test-b.sh\n"


def found(workflows, hooks=None, ci_only=None):
    return chs.problems(workflows, {"pre-commit": HOOK} if hooks is None else hooks, {} if ci_only is None else ci_only)


class Units(unittest.TestCase):
    """units(): the scripts a shell text runs, each with its subcommand."""

    # Success: the forms the hooks and the workflows use.

    def test_a_python_script(self):
        self.assertEqual(chs.units("python3 -I scripts/test_a.py"), ["scripts/test_a.py"])

    def test_a_shell_script(self):
        self.assertEqual(chs.units("scripts/test-b.sh"), ["scripts/test-b.sh"])

    def test_a_subcommand_is_part_of_the_unit(self):
        self.assertEqual(chs.units("python3 -I scripts/docs_test.py lint"), ["scripts/docs_test.py lint"])

    def test_an_option_is_not_part_of_the_unit(self):
        self.assertEqual(chs.units("python3 -I scripts/check_style.py --walk"), ["scripts/check_style.py"])

    def test_two_subcommands_of_one_script_are_two_units(self):
        text = "python3 -I scripts/clippy_features.py run --quiet\npython3 -I scripts/clippy_features.py check\n"
        self.assertEqual(chs.units(text), ["scripts/clippy_features.py run", "scripts/clippy_features.py check"])

    def test_a_script_run_through_bash(self):
        self.assertEqual(chs.units("bash scripts/test-check-package.sh"), ["scripts/test-check-package.sh"])

    def test_a_script_named_from_a_subdirectory(self):
        self.assertEqual(chs.units("(cd website && ../scripts/test-site.sh)"), ["scripts/test-site.sh"])

    def test_a_quoted_path_with_a_variable_before_it(self):
        self.assertEqual(chs.units('bash "$GITHUB_WORKSPACE/scripts/nextest.sh" install'), ["scripts/nextest.sh install"])

    def test_a_script_in_a_pipe(self):
        self.assertEqual(chs.units("if ! scan_input | python3 -I scripts/scan_forbidden.py; then"), ["scripts/scan_forbidden.py"])

    def test_scripts_in_the_order_they_appear_each_once(self):
        text = "scripts/test-b.sh\npython3 -I scripts/test_a.py\nscripts/test-b.sh\n"
        self.assertEqual(chs.units(text), ["scripts/test-b.sh", "scripts/test_a.py"])

    def test_two_scripts_on_one_line(self):
        self.assertEqual(chs.units("scripts/test-b.sh && python3 -I scripts/test_a.py"), ["scripts/test-b.sh", "scripts/test_a.py"])

    def test_an_argument_with_a_variable_is_not_a_subcommand(self):
        self.assertEqual(chs.units('python3 -I scripts/check_required_checks.py "$RUNNER_TEMP/rules.json"'), ["scripts/check_required_checks.py"])

    def test_a_subcommand_followed_by_arguments(self):
        self.assertEqual(chs.units('scripts/fuzz.sh run "$TARGET" 60'), ["scripts/fuzz.sh run"])

    # Edge.

    def test_no_script_in_an_empty_text(self):
        self.assertEqual(chs.units(""), [])

    def test_a_comment_line_runs_nothing(self):
        self.assertEqual(chs.units("# scripts/test-b.sh checks the hooks\n  # python3 -I scripts/test_a.py\n"), [])

    def test_a_comment_after_a_command_runs_nothing(self):
        self.assertEqual(chs.units("scripts/test-b.sh # then scripts/test-c.sh by hand"), ["scripts/test-b.sh"])

    def test_the_command_after_the_wrapper_is_not_its_subcommand(self):
        self.assertEqual(chs.units("scripts/retry.sh rustup toolchain install"), ["scripts/retry.sh"])

    def test_a_script_the_wrapper_runs_is_a_unit_too(self):
        self.assertEqual(chs.units("scripts/retry.sh scripts/fetch-pinned.sh --url u"), ["scripts/retry.sh", "scripts/fetch-pinned.sh"])

    def test_a_script_given_as_an_argument_is_a_unit_too(self):
        self.assertEqual(
            chs.units("scripts/test-hook-isolation.sh scripts/test-hooks.sh scripts/test-pre-push.sh"),
            ["scripts/test-hook-isolation.sh", "scripts/test-hooks.sh", "scripts/test-pre-push.sh"],
        )

    def test_a_file_that_is_no_script_is_not_a_unit(self):
        self.assertEqual(chs.units("pip install -r scripts/icons-requirements.txt; cat scripts/docs-test-pins.txt"), [])

    def test_a_capitalized_argument_is_not_a_subcommand(self):
        self.assertEqual(chs.units("scripts/wait-for-s3.sh RustFS"), ["scripts/wait-for-s3.sh"])

    def test_a_path_argument_is_not_a_subcommand(self):
        self.assertEqual(chs.units("scripts/run-clients.sh target/debug/vcon-server"), ["scripts/run-clients.sh"])

    def test_a_number_is_not_a_subcommand(self):
        self.assertEqual(chs.units("scripts/tool.sh 60"), ["scripts/tool.sh"])

    def test_a_line_continued_with_a_backslash_is_one_line(self):
        self.assertEqual(chs.units("python3 -I scripts/docs_test.py \\\n  lint\n"), ["scripts/docs_test.py lint"])

    def test_a_script_of_another_directory_is_not_a_unit(self):
        self.assertEqual(chs.units("python3 tools/scripts2/x.py; bash other-scripts/run.sh; bash myscripts/run.sh"), [])

    def test_a_subcommand_ends_at_a_shell_operator(self):
        self.assertEqual(chs.units("scripts/fuzz.sh list| wc -l; scripts/nextest.sh check&& true"), ["scripts/fuzz.sh list", "scripts/nextest.sh check"])

    def test_a_hash_inside_a_word_starts_no_comment(self):
        self.assertEqual(chs.units('echo "${#names[@]}"; scripts/test-b.sh'), ["scripts/test-b.sh"])


class RunTexts(unittest.TestCase):
    """run_texts(): the run steps of a parsed workflow or composite action."""

    def test_the_run_steps_of_each_job(self):
        document = {"jobs": {"a": {"steps": [{"run": "one"}]}, "b": {"steps": [{"uses": "x@1"}, {"run": "two"}]}}}
        self.assertEqual(chs.run_texts(document), ["one", "two"])

    def test_the_run_steps_of_a_composite_action(self):
        self.assertEqual(chs.run_texts(action("one", "two")), ["one", "two"])

    def test_an_empty_file_has_none(self):
        self.assertEqual(chs.run_texts(None), [])

    def test_a_job_without_steps_has_none(self):
        self.assertEqual(chs.run_texts({"jobs": {"a": {"uses": "./.github/workflows/x.yml"}, "b": None}}), [])

    def test_a_document_that_is_not_a_mapping_has_none(self):
        self.assertEqual(chs.run_texts(["jobs"]), [])

    def test_a_run_that_is_not_text_is_read_as_text(self):
        self.assertEqual(chs.run_texts({"jobs": {"a": {"steps": [{"run": 7}]}}}), ["7"])


class Problems(unittest.TestCase):
    """problems(): what the workflows run that no hook runs."""

    # Success.

    def test_a_script_test_the_hook_runs(self):
        self.assertEqual(found({"ci.yml": workflow("python3 -I scripts/test_a.py")}), [])

    def test_a_check_the_hook_runs_with_other_options(self):
        self.assertEqual(found({"ci.yml": workflow("python3 -I scripts/check_a.py")}), [])

    def test_a_shell_test_the_hook_runs(self):
        self.assertEqual(found({"ci.yml": workflow("scripts/test-b.sh")}), [])

    def test_a_script_listed_with_its_reason(self):
        self.assertEqual(found({"ci.yml": workflow("scripts/check-site.sh")}, ci_only={"scripts/check-site.sh": "needs zola"}), [])

    def test_a_subcommand_listed_with_its_reason(self):
        self.assertEqual(found({"ci.yml": workflow("scripts/nextest.sh install")}, ci_only={"scripts/nextest.sh install": "installs the runner"}), [])

    def test_a_workflow_that_runs_no_script(self):
        self.assertEqual(found({"ci.yml": workflow("cargo deny check", "echo done")}), [])

    def test_no_workflow_at_all(self):
        self.assertEqual(found({}), [])

    def test_a_script_another_hook_runs(self):
        hooks = {"pre-commit": HOOK, "pre-push": '"$root/scripts/scan-commit-messages.sh" "$sha"\n'}
        self.assertEqual(found({"ci.yml": workflow("scripts/scan-commit-messages.sh")}, hooks=hooks), [])

    def test_a_script_a_composite_action_runs_and_the_list_holds(self):
        self.assertEqual(found({"actions/nextest/action.yml": action('bash "$W/scripts/nextest.sh" pin')}, ci_only={"scripts/nextest.sh pin": "prints the pin"}), [])

    def test_several_steps_all_covered(self):
        wf = workflow("set -euo pipefail\npython3 -I scripts/test_a.py\npython3 -I scripts/check_a.py\n", "scripts/test-b.sh")
        self.assertEqual(found({"ci.yml": wf}), [])

    def test_a_hook_may_run_what_no_workflow_runs(self):
        self.assertEqual(found({"ci.yml": workflow("scripts/test-b.sh")}), [])

    def test_the_same_script_in_two_workflows_both_covered(self):
        self.assertEqual(found({"ci.yml": workflow("scripts/test-b.sh"), "release.yml": workflow("scripts/test-b.sh")}), [])

    # Failure.

    def test_a_script_test_no_hook_runs(self):
        self.assertEqual(
            found({"ci.yml": workflow("python3 -I scripts/test_docs_test.py")}),
            ["ci.yml: runs `scripts/test_docs_test.py`, which no hook runs: add it to .githooks/pre-commit, or to CI_ONLY in scripts/check_hook_steps.py with the reason it runs only in CI"],
        )

    def test_a_check_no_hook_runs(self):
        self.assertEqual(len(found({"ci.yml": workflow("python3 -I scripts/check_glibc.py --docs .")})), 1)

    def test_a_subcommand_no_hook_runs_of_a_script_a_hook_runs(self):
        hooks = {"pre-commit": "python3 -I scripts/clippy_features.py run --quiet\n"}
        problems = found({"ci.yml": workflow("python3 -I scripts/clippy_features.py check")}, hooks=hooks)
        self.assertEqual(len(problems), 1)
        self.assertIn("runs `scripts/clippy_features.py check`, which no hook runs", problems[0])

    def test_a_script_the_hook_names_only_in_a_comment(self):
        hooks = {"pre-commit": "# scripts/test-c.sh runs in CI\nscripts/test-b.sh\n"}
        self.assertEqual(len(found({"ci.yml": workflow("scripts/test-c.sh")}, hooks=hooks)), 1)

    def test_each_uncovered_script_is_named(self):
        problems = found({"ci.yml": workflow("scripts/test-c.sh\nscripts/test-d.sh")})
        self.assertEqual([p.split("`")[1] for p in problems], ["scripts/test-c.sh", "scripts/test-d.sh"])

    def test_an_uncovered_script_names_each_file_that_runs_it(self):
        problems = found({"ci.yml": workflow("scripts/test-c.sh"), "release.yml": workflow("scripts/test-c.sh")})
        self.assertEqual(len(problems), 1)
        self.assertTrue(problems[0].startswith("ci.yml, release.yml: runs `scripts/test-c.sh`"), problems[0])

    def test_an_uncovered_script_in_a_composite_action(self):
        problems = found({"actions/x/action.yml": action("scripts/test-c.sh")})
        self.assertEqual(len(problems), 1)
        self.assertTrue(problems[0].startswith("actions/x/action.yml: runs `scripts/test-c.sh`"), problems[0])

    def test_a_listed_script_a_hook_runs(self):
        self.assertEqual(
            found({"ci.yml": workflow("scripts/test-b.sh")}, ci_only={"scripts/test-b.sh": "slow"}),
            ["CI_ONLY lists `scripts/test-b.sh`, which .githooks/pre-commit runs: remove it from the list"],
        )

    def test_a_listed_script_no_workflow_runs(self):
        self.assertEqual(
            found({"ci.yml": workflow("scripts/test-b.sh")}, ci_only={"scripts/gone.sh": "was slow"}),
            ["CI_ONLY lists `scripts/gone.sh`, which no workflow runs: remove it from the list"],
        )

    def test_a_listed_script_with_no_reason(self):
        self.assertEqual(
            found({"ci.yml": workflow("scripts/check-site.sh")}, ci_only={"scripts/check-site.sh": "  "}),
            ["CI_ONLY gives no reason for `scripts/check-site.sh`"],
        )

    def test_a_listed_script_does_not_cover_its_subcommand(self):
        problems = found({"ci.yml": workflow("scripts/nextest.sh install")}, ci_only={"scripts/nextest.sh": "the runner"})
        self.assertEqual(len(problems), 2)
        self.assertIn("runs `scripts/nextest.sh install`, which no hook runs", problems[0])
        self.assertIn("CI_ONLY lists `scripts/nextest.sh`, which no workflow runs", problems[1])

    def test_no_hook_at_all(self):
        self.assertEqual(len(found({"ci.yml": workflow("scripts/test-b.sh")}, hooks={})), 1)

    def test_an_uncovered_script_in_a_second_job(self):
        wf = {"jobs": {"a": {"steps": [{"run": "scripts/test-b.sh"}]}, "b": {"steps": [{"run": "scripts/test-c.sh"}]}}}
        self.assertEqual(len(found({"ci.yml": wf})), 1)

    # Edge.

    def test_a_script_named_only_in_a_workflow_comment_line_of_a_step(self):
        self.assertEqual(found({"ci.yml": workflow("# scripts/test-c.sh is run by the next job\nscripts/test-b.sh")}), [])

    def test_a_script_named_in_a_step_name_is_not_run(self):
        wf = {"jobs": {"a": {"steps": [{"name": "what scripts/test-c.sh checks", "run": "scripts/test-b.sh"}]}}}
        self.assertEqual(found({"ci.yml": wf}), [])

    def test_a_script_named_in_an_environment_value_is_not_run(self):
        wf = {"jobs": {"a": {"steps": [{"env": {"KEY": "scripts/docs-test-pins.txt scripts/test-c.sh"}, "run": "scripts/test-b.sh"}]}}}
        self.assertEqual(found({"ci.yml": wf}), [])

    def test_problems_are_sorted_by_script(self):
        problems = found({"ci.yml": workflow("scripts/test-z.sh\nscripts/test-c.sh")})
        self.assertEqual([p.split("`")[1] for p in problems], ["scripts/test-c.sh", "scripts/test-z.sh"])

    def test_the_wrapper_itself_must_be_covered(self):
        problems = found({"ci.yml": workflow("scripts/retry.sh rustup toolchain install")})
        self.assertEqual(len(problems), 1)
        self.assertIn("`scripts/retry.sh`", problems[0])

    def test_an_empty_workflow_file(self):
        self.assertEqual(found({"ci.yml": None}), [])

    def test_a_listed_script_run_by_a_second_hook_names_that_hook(self):
        hooks = {"pre-commit": HOOK, "pre-push": "scripts/scan-commit-messages.sh\n"}
        problems = found({"ci.yml": workflow("scripts/scan-commit-messages.sh")}, hooks=hooks, ci_only={"scripts/scan-commit-messages.sh": "x"})
        self.assertEqual(problems, ["CI_ONLY lists `scripts/scan-commit-messages.sh`, which .githooks/pre-push runs: remove it from the list"])


class Repository(unittest.TestCase):
    """repository_problems() and main(), on a tree written for the test and
    on this repository."""

    def tree(self, hook, ci, extra=None):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        (root / ".githooks").mkdir()
        (root / ".github/workflows").mkdir(parents=True)
        (root / ".githooks/pre-commit").write_text(hook, encoding="utf-8")
        (root / ".github/workflows/ci.yml").write_text(ci, encoding="utf-8")
        for path, text in (extra or {}).items():
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(text, encoding="utf-8")
        return root

    CI = "jobs:\n  scan:\n    steps:\n      - run: |\n          set -euo pipefail\n          python3 -I scripts/test_a.py\n"

    def run_main(self, root, ci_only):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = chs.main(root, ci_only)
        return code, out.getvalue(), err.getvalue()

    def test_this_repository_keeps_the_rule(self):
        self.assertEqual(chs.repository_problems(HERE.parent, chs.CI_ONLY), [])

    def test_every_listed_reason_is_a_sentence_of_some_length(self):
        for unit, reason in chs.CI_ONLY.items():
            self.assertGreaterEqual(len(reason.split()), 3, unit)

    def test_this_repository_runs_the_check_in_the_hook_and_in_ci(self):
        hook = (HERE.parent / ".githooks/pre-commit").read_text(encoding="utf-8")
        ci = (HERE.parent / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        for text in (hook, ci):
            self.assertIn("scripts/check_hook_steps.py", chs.units(text))
            self.assertIn("scripts/test_check_hook_steps.py", chs.units(text))

    def test_a_tree_that_keeps_the_rule(self):
        root = self.tree(HOOK, self.CI)
        self.assertEqual(self.run_main(root, {}), (0, "check_hook_steps: ok (1 scripts run in workflows: 1 in a hook, 0 only in CI)\n", ""))

    def test_a_tree_whose_workflow_runs_what_no_hook_runs(self):
        root = self.tree("scripts/test-b.sh\n", self.CI)
        code, out, err = self.run_main(root, {})
        self.assertEqual((code, out), (1, ""))
        self.assertTrue(err.startswith("check_hook_steps: .github/workflows/ci.yml: runs `scripts/test_a.py`, which no hook runs"), err)

    def test_a_second_workflow_is_read(self):
        root = self.tree(HOOK, self.CI, {".github/workflows/release.yml": "jobs:\n  a:\n    steps:\n      - run: scripts/test-c.sh\n"})
        code, _, err = self.run_main(root, {})
        self.assertEqual(code, 1)
        self.assertIn(".github/workflows/release.yml: runs `scripts/test-c.sh`", err)

    def test_a_composite_action_is_read(self):
        root = self.tree(HOOK, self.CI, {".github/actions/x/action.yml": "runs:\n  using: composite\n  steps:\n    - run: scripts/test-c.sh\n      shell: bash\n"})
        code, _, err = self.run_main(root, {})
        self.assertEqual(code, 1)
        self.assertIn(".github/actions/x/action.yml: runs `scripts/test-c.sh`", err)

    def test_every_hook_file_is_read(self):
        root = self.tree(HOOK, self.CI.replace("scripts/test_a.py", "scripts/test_c.py"), {".githooks/pre-push": "python3 -I scripts/test_c.py\n"})
        self.assertEqual(self.run_main(root, {})[0], 0)

    def test_the_list_counts_in_the_summary(self):
        root = self.tree(HOOK, self.CI.replace("python3 -I scripts/test_a.py", "python3 -I scripts/test_a.py\n          scripts/check-site.sh"))
        code, out, _ = self.run_main(root, {"scripts/check-site.sh": "needs the site's tools"})
        self.assertEqual((code, out), (0, "check_hook_steps: ok (2 scripts run in workflows: 1 in a hook, 1 only in CI)\n"))

    def test_a_tree_with_no_hook_directory(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        (root / ".github/workflows").mkdir(parents=True)
        (root / ".github/workflows/ci.yml").write_text(self.CI, encoding="utf-8")
        code, _, err = self.run_main(root, {})
        self.assertEqual(code, 1)
        self.assertIn("runs `scripts/test_a.py`, which no hook runs", err)

    def test_a_tree_with_no_workflow(self):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        code, _, err = self.run_main(root, {})
        self.assertEqual((code, err), (1, "check_hook_steps: .github/workflows holds no workflow\n"))

    def test_a_workflow_that_is_not_yaml(self):
        root = self.tree(HOOK, "jobs: [unclosed\n")
        code, _, err = self.run_main(root, {})
        self.assertEqual(code, 1)
        self.assertTrue(err.startswith("check_hook_steps: .github/workflows/ci.yml is not YAML"), err)

    def test_a_hook_added_later_is_read(self):
        root = self.tree("scripts/test-b.sh\n", self.CI, {".githooks/commit-msg": "python3 -I scripts/test_a.py\n"})
        self.assertEqual(self.run_main(root, {})[0], 0)

    def test_a_copy_of_a_hook_under_another_name_is_no_hook(self):
        # git runs a hook by its name, which holds no dot: a saved copy
        # (pre-commit.orig) or a sample runs nothing.
        for name in ("pre-commit.orig", "pre-commit.sample", "pre-commit~", ".pre-commit.swp"):
            with self.subTest(name=name):
                root = self.tree("scripts/test-b.sh\n", self.CI, {f".githooks/{name}": "python3 -I scripts/test_a.py\n"})
                code, _, err = self.run_main(root, {})
                self.assertEqual(code, 1)
                self.assertIn("runs `scripts/test_a.py`, which no hook runs", err)

    def test_a_directory_in_the_hook_directory_is_no_hook(self):
        root = self.tree("scripts/test-b.sh\n", self.CI, {".githooks/lib/commit-msg": "python3 -I scripts/test_a.py\n"})
        self.assertEqual(self.run_main(root, {})[0], 1)


if __name__ == "__main__":
    unittest.main()
