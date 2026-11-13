"""Tests scripts/check_runner_images.py: no workflow and no composite
action names a floating runner image (a label that ends in -latest).

Run with: python3 -I scripts/test_check_runner_images.py
"""

import contextlib
import importlib.util
import io
import pathlib
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_runner_images", HERE / "check_runner_images.py")
cri = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cri)


def job(runs_on="ubuntu-24.04", **more):
    return {"name": "a job", "runs-on": runs_on, "steps": [{"run": "true"}], **more}


def workflow(**jobs):
    return {"name": "CI", "on": {"push": None}, "jobs": jobs}


def found(document, name="ci.yml"):
    return cri.problems({name: document})


def line(where, label, name="ci.yml"):
    return f"{name}: {where}: `{label}` is a floating runner image: name a fixed one (for example ubuntu-24.04)"


class Floating(unittest.TestCase):
    """floating(): the floating labels in one text."""

    def test_success_a_fixed_ubuntu_image(self):
        self.assertEqual(cri.floating("ubuntu-24.04"), [])

    def test_success_a_fixed_arm_image(self):
        self.assertEqual(cri.floating("ubuntu-24.04-arm"), [])

    def test_success_a_fixed_macos_image(self):
        self.assertEqual(cri.floating("macos-15"), [])

    def test_success_a_fixed_windows_image(self):
        self.assertEqual(cri.floating("windows-2025"), [])

    def test_success_a_self_hosted_label(self):
        self.assertEqual(cri.floating("self-hosted"), [])

    def test_success_an_expression_between_fixed_images(self):
        self.assertEqual(cri.floating("${{ matrix.target == 'aarch64-unknown-linux-gnu' && 'ubuntu-24.04-arm' || 'ubuntu-24.04' }}"), [])

    def test_success_an_expression_that_names_the_matrix(self):
        self.assertEqual(cri.floating("${{ matrix.runner }}"), [])

    def test_failure_ubuntu_latest(self):
        self.assertEqual(cri.floating("ubuntu-latest"), ["ubuntu-latest"])

    def test_failure_macos_latest(self):
        self.assertEqual(cri.floating("macos-latest"), ["macos-latest"])

    def test_failure_windows_latest(self):
        self.assertEqual(cri.floating("windows-latest"), ["windows-latest"])

    def test_failure_a_floating_label_with_a_size(self):
        self.assertEqual(cri.floating("macos-latest-xlarge"), ["macos-latest-xlarge"])

    def test_failure_a_floating_arm_label(self):
        self.assertEqual(cri.floating("ubuntu-latest-arm"), ["ubuntu-latest-arm"])

    def test_failure_a_floating_label_in_an_expression(self):
        self.assertEqual(
            cri.floating("${{ matrix.target == 'aarch64-unknown-linux-gnu' && 'ubuntu-24.04-arm' || 'ubuntu-latest' }}"),
            ["ubuntu-latest"],
        )

    def test_failure_two_floating_labels_in_one_text(self):
        self.assertEqual(cri.floating("${{ matrix.mac && 'macos-latest' || 'ubuntu-latest' }}"), ["macos-latest", "ubuntu-latest"])

    def test_edge_an_empty_text(self):
        self.assertEqual(cri.floating(""), [])

    def test_edge_the_word_latest_alone_is_no_runner_label(self):
        self.assertEqual(cri.floating("latest"), [])

    def test_edge_an_image_tag_latest_is_not_a_runner_label(self):
        # A container image is pinned by digest, which another check holds.
        self.assertEqual(cri.floating("postgres:latest"), [])

    def test_edge_a_longer_word_that_holds_latest(self):
        self.assertEqual(cri.floating("ubuntu-latestish"), [])
        self.assertEqual(cri.floating("the-latest_release"), [])

    def test_edge_upper_case_is_the_same_label(self):
        # GitHub compares runner labels without regard to case.
        self.assertEqual(cri.floating("Ubuntu-Latest"), ["Ubuntu-Latest"])

    def test_edge_a_label_between_quotes_or_brackets(self):
        self.assertEqual(cri.floating("['ubuntu-latest']"), ["ubuntu-latest"])
        self.assertEqual(cri.floating('"ubuntu-latest"'), ["ubuntu-latest"])

    def test_edge_a_path_that_ends_in_latest(self):
        self.assertEqual(cri.floating("releases/download/v1-latest/x.zip"), [])
        self.assertEqual(cri.floating("images/ubuntu-latest"), [])
        self.assertEqual(cri.floating("ubuntu-latest/readme"), [])


class Problems(unittest.TestCase):
    """problems(): each floating image of the parsed files, with its place."""

    # Success.

    def test_success_every_job_on_a_fixed_image(self):
        self.assertEqual(found(workflow(a=job(), b=job("ubuntu-24.04-arm"))), [])

    def test_success_a_matrix_of_fixed_images(self):
        doc = workflow(build=job("${{ matrix.runner }}", strategy={"matrix": {"include": [{"runner": "ubuntu-24.04"}, {"runner": "ubuntu-24.04-arm"}]}}))
        self.assertEqual(found(doc), [])

    def test_success_a_list_of_labels(self):
        self.assertEqual(found(workflow(a=job(["self-hosted", "linux", "arm64"]))), [])

    def test_success_a_runner_group(self):
        self.assertEqual(found(workflow(a=job({"group": "large", "labels": ["ubuntu-24.04"]}))), [])

    def test_success_a_job_that_calls_a_workflow(self):
        self.assertEqual(found(workflow(a={"uses": "./.github/workflows/x.yml", "with": {"runner": "ubuntu-24.04"}})), [])

    def test_success_a_composite_action(self):
        self.assertEqual(found({"runs": {"using": "composite", "steps": [{"run": "true", "shell": "bash"}]}}, "actions/x/action.yml"), [])

    def test_success_no_file(self):
        self.assertEqual(cri.problems({}), [])

    def test_success_a_step_script_may_print_the_word(self):
        doc = workflow(a=job(steps=[{"run": "echo 'moved from ubuntu-latest'"}]))
        self.assertEqual(found(doc), [])

    def test_success_a_name_may_hold_the_word(self):
        doc = workflow(a=job(name="was ubuntu-latest", steps=[{"name": "not on ubuntu-latest", "run": "true"}]))
        self.assertEqual(found(doc), [])

    # Failure.

    def test_failure_a_job_on_ubuntu_latest(self):
        self.assertEqual(found(workflow(build=job("ubuntu-latest"))), [line("jobs.build.runs-on", "ubuntu-latest")])

    def test_failure_a_job_on_macos_latest(self):
        self.assertEqual(found(workflow(build=job("macos-latest"))), [line("jobs.build.runs-on", "macos-latest")])

    def test_failure_a_job_on_windows_latest(self):
        self.assertEqual(found(workflow(build=job("windows-latest"))), [line("jobs.build.runs-on", "windows-latest")])

    def test_failure_a_floating_image_in_a_matrix_row(self):
        doc = workflow(build=job("${{ matrix.runner }}", strategy={"matrix": {"include": [{"runner": "ubuntu-latest"}, {"runner": "ubuntu-24.04-arm"}]}}))
        self.assertEqual(found(doc), [line("jobs.build.strategy.matrix.include[0].runner", "ubuntu-latest")])

    def test_failure_a_floating_image_in_a_matrix_list(self):
        doc = workflow(build=job("${{ matrix.os }}", strategy={"matrix": {"os": ["ubuntu-24.04", "macos-latest"]}}))
        self.assertEqual(found(doc), [line("jobs.build.strategy.matrix.os[1]", "macos-latest")])

    def test_failure_a_floating_image_in_an_expression(self):
        doc = workflow(smoke=job("${{ matrix.arm && 'ubuntu-24.04-arm' || 'ubuntu-latest' }}"))
        self.assertEqual(found(doc), [line("jobs.smoke.runs-on", "ubuntu-latest")])

    def test_failure_a_floating_image_in_a_list_of_labels(self):
        self.assertEqual(found(workflow(a=job(["ubuntu-latest"]))), [line("jobs.a.runs-on[0]", "ubuntu-latest")])

    def test_failure_a_floating_image_in_a_runner_group(self):
        doc = workflow(a=job({"group": "large", "labels": ["ubuntu-latest"]}))
        self.assertEqual(found(doc), [line("jobs.a.runs-on.labels[0]", "ubuntu-latest")])

    def test_failure_a_floating_image_given_to_a_called_workflow(self):
        doc = workflow(a={"uses": "./.github/workflows/x.yml", "with": {"runner": "ubuntu-latest"}})
        self.assertEqual(found(doc), [line("jobs.a.with.runner", "ubuntu-latest")])

    def test_failure_a_floating_default_of_a_workflow_input(self):
        doc = {"on": {"workflow_call": {"inputs": {"runner": {"type": "string", "default": "ubuntu-latest"}}}}, "jobs": {"a": job("${{ inputs.runner }}")}}
        self.assertEqual(found(doc), [line("on.workflow_call.inputs.runner.default", "ubuntu-latest")])

    def test_failure_a_floating_image_in_an_environment_value(self):
        doc = workflow(a=job(env={"RUNNER_IMAGE": "ubuntu-latest"}))
        self.assertEqual(found(doc), [line("jobs.a.env.RUNNER_IMAGE", "ubuntu-latest")])

    def test_failure_each_job_is_named(self):
        doc = workflow(a=job("ubuntu-latest"), b=job(), c=job("ubuntu-latest"))
        self.assertEqual(found(doc), [line("jobs.a.runs-on", "ubuntu-latest"), line("jobs.c.runs-on", "ubuntu-latest")])

    def test_failure_each_file_is_named(self):
        got = cri.problems({"ci.yml": workflow(a=job("ubuntu-latest")), "release.yml": workflow(a=job("ubuntu-latest"))})
        self.assertEqual(got, [line("jobs.a.runs-on", "ubuntu-latest"), line("jobs.a.runs-on", "ubuntu-latest", "release.yml")])

    def test_failure_a_floating_image_given_to_an_action_of_a_composite_action(self):
        doc = {"runs": {"using": "composite", "steps": [{"uses": "x/y@0123", "with": {"os": "macos-latest"}}]}}
        self.assertEqual(found(doc, "actions/x/action.yml"), [line("runs.steps[0].with.os", "macos-latest", "actions/x/action.yml")])

    # Edge.

    def test_edge_an_empty_file(self):
        self.assertEqual(found(None), [])

    def test_edge_a_file_that_is_a_list(self):
        self.assertEqual(found(["ubuntu-latest"]), [line("[0]", "ubuntu-latest")])

    def test_edge_a_job_with_no_runner(self):
        self.assertEqual(found(workflow(a={"steps": [{"run": "true"}]})), [])

    def test_edge_values_that_are_not_text(self):
        doc = workflow(a=job(**{"timeout-minutes": 5, "continue-on-error": True, "env": {"N": None}}))
        self.assertEqual(found(doc), [])

    def test_edge_a_key_that_is_a_floating_label_is_reported(self):
        doc = workflow(a=job(env={"ubuntu-latest": "x"}))
        self.assertEqual(found(doc), [line("jobs.a.env", "ubuntu-latest")])

    def test_edge_yaml_reads_on_as_true(self):
        # PyYAML reads the key `on` as the boolean true.
        doc = {True: {"workflow_call": {"inputs": {"runner": {"default": "ubuntu-latest"}}}}, "jobs": {}}
        self.assertEqual(found(doc), [line("on.workflow_call.inputs.runner.default", "ubuntu-latest")])

    def test_edge_a_run_script_outside_a_step_is_still_a_script(self):
        doc = {"runs": {"using": "composite", "steps": [{"run": "echo ubuntu-latest", "shell": "bash"}]}}
        self.assertEqual(found(doc, "actions/x/action.yml"), [])

    def test_edge_a_description_may_hold_the_word(self):
        doc = {"description": "Runs on ubuntu-latest no longer", "runs": {"using": "composite", "steps": []}}
        self.assertEqual(found(doc, "actions/x/action.yml"), [])

    def test_edge_one_label_twice_in_one_value_is_reported_once(self):
        doc = workflow(a=job("${{ matrix.x && 'ubuntu-latest' || 'ubuntu-latest' }}"))
        self.assertEqual(found(doc), [line("jobs.a.runs-on", "ubuntu-latest")])

    def test_edge_a_job_id_with_a_dot_or_a_space_is_quoted_in_the_place(self):
        doc = {"jobs": {"a b": job("ubuntu-latest")}}
        self.assertEqual(found(doc), [line("jobs.'a b'.runs-on", "ubuntu-latest")])


class Repository(unittest.TestCase):
    """main(), on a tree written for the test and on this repository."""

    GOOD = "jobs:\n  a:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: 'true'\n"

    def tree(self, files):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory()))
        for path, text in files.items():
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(text, encoding="utf-8")
        return root

    def run_main(self, root):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = cri.main(root)
        return code, out.getvalue(), err.getvalue()

    def test_success_this_repository_names_fixed_images_only(self):
        code, _, err = self.run_main(HERE.parent)
        self.assertEqual((code, err), (0, ""))

    def test_success_this_repository_runs_the_check_in_the_hook_and_in_ci(self):
        for path in (".githooks/pre-commit", ".github/workflows/ci.yml"):
            text = (HERE.parent / path).read_text(encoding="utf-8")
            self.assertIn("python3 -I scripts/check_runner_images.py\n", text, path)
            self.assertIn("python3 -I scripts/test_check_runner_images.py\n", text, path)

    def test_success_a_tree_of_fixed_images(self):
        root = self.tree({".github/workflows/ci.yml": self.GOOD, ".github/workflows/release.yaml": self.GOOD})
        self.assertEqual(self.run_main(root), (0, "check_runner_images: ok (2 files name no floating runner image)\n", ""))

    def test_failure_a_workflow_on_a_floating_image(self):
        root = self.tree({".github/workflows/ci.yml": self.GOOD.replace("ubuntu-24.04", "ubuntu-latest")})
        self.assertEqual(
            self.run_main(root),
            (1, "", "check_runner_images: " + line("jobs.a.runs-on", "ubuntu-latest", ".github/workflows/ci.yml") + "\n"),
        )

    def test_failure_a_second_workflow_is_read(self):
        root = self.tree({".github/workflows/ci.yml": self.GOOD, ".github/workflows/z.yml": self.GOOD.replace("ubuntu-24.04", "macos-latest")})
        code, _, err = self.run_main(root)
        self.assertEqual(code, 1)
        self.assertIn(".github/workflows/z.yml: jobs.a.runs-on: `macos-latest`", err)

    def test_failure_a_composite_action_is_read(self):
        action = "runs:\n  using: composite\n  steps:\n    - uses: x/y@0123\n      with:\n        os: windows-latest\n"
        root = self.tree({".github/workflows/ci.yml": self.GOOD, ".github/actions/x/action.yml": action})
        code, _, err = self.run_main(root)
        self.assertEqual(code, 1)
        self.assertIn(".github/actions/x/action.yml: runs.steps[0].with.os: `windows-latest`", err)

    def test_failure_a_tree_with_no_workflow(self):
        root = self.tree({"README.md": "x\n"})
        self.assertEqual(self.run_main(root), (1, "", "check_runner_images: .github/workflows holds no workflow\n"))

    def test_failure_a_workflow_that_is_not_yaml(self):
        root = self.tree({".github/workflows/ci.yml": "jobs: [unclosed\n"})
        code, _, err = self.run_main(root)
        self.assertEqual(code, 1)
        self.assertTrue(err.startswith("check_runner_images: .github/workflows/ci.yml is not YAML"), err)

    def test_edge_a_comment_may_hold_the_word(self):
        root = self.tree({".github/workflows/ci.yml": "# moved from ubuntu-latest on 2026-11-13\n" + self.GOOD})
        self.assertEqual(self.run_main(root)[0], 0)

    def test_edge_a_quoted_label_is_read(self):
        root = self.tree({".github/workflows/ci.yml": self.GOOD.replace("ubuntu-24.04", "'ubuntu-latest'")})
        self.assertEqual(self.run_main(root)[0], 1)


if __name__ == "__main__":
    unittest.main()
