"""Tests scripts/check_test_runner.py: every test run in the commit hook and
in the workflows has the per-test deadline, because each runs through
cargo-nextest at the pinned version with .config/nextest.toml; the only
`cargo test` runs left are the doc tests, which nextest does not run, and
Miri's.

Run with: python3 -I scripts/test_check_test_runner.py
"""

import importlib.util
import pathlib
import sys
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_test_runner", HERE / "check_test_runner.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

CONFIG = '[profile.default]\nslow-timeout = { period = "60s", terminate-after = 3 }\n'
INSTALL = {"name": "Install cargo-nextest", "run": "scripts/nextest.sh install"}


def job(*runs, install=True):
    steps = [{"uses": "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"}]
    if install:
        steps.append(dict(INSTALL))
    steps += [{"name": f"step {i}", "run": run} for i, run in enumerate(runs)]
    return {"name": "a job", "runs-on": "ubuntu-latest", "steps": steps}


def problems(config=CONFIG, hook="scripts/nextest.sh run --workspace --locked\ncargo test --workspace --doc --locked\n", **jobs):
    workflows = {"ci.yml": {"jobs": jobs or {"rust": job("scripts/nextest.sh run --workspace --locked\ncargo test --workspace --locked --doc")}}}
    return check.problems(config, hook, workflows)


class Deadline(unittest.TestCase):
    def test_the_rule_holds_for_nextest_runs_and_doc_tests(self):
        self.assertEqual(problems(), [])

    def test_a_config_without_the_deadline_is_refused(self):
        self.assertTrue(any("slow-timeout" in p for p in problems(config="[profile.default]\n")))

    def test_another_deadline_is_refused(self):
        other = CONFIG.replace("terminate-after = 3", "terminate-after = 30")
        self.assertTrue(any("slow-timeout" in p for p in problems(config=other)))

    def test_a_config_that_is_not_toml_is_refused(self):
        self.assertTrue(any("nextest.toml" in p for p in problems(config="[profile.default\n")))


class CargoTest(unittest.TestCase):
    def test_cargo_test_in_the_hook_is_refused(self):
        found = problems(hook="cargo test --workspace --locked --quiet\n")
        self.assertTrue(any("pre-commit" in p and "cargo test" in p for p in found), found)

    def test_cargo_test_in_a_workflow_is_refused(self):
        found = problems(backends=job("cargo test -p vcon-server --lib backend::", install=False))
        self.assertTrue(any("backends" in p and "cargo test" in p for p in found), found)

    def test_cargo_test_with_a_toolchain_is_refused(self):
        found = problems(asan=job('cargo "+$NIGHTLY" test -Zbuild-std --lib', install=False))
        self.assertTrue(any("asan" in p for p in found), found)

    def test_cargo_test_held_in_a_bash_array_is_refused(self):
        found = problems(asan=job('test=(cargo "+$NIGHTLY" test -Zbuild-std\n  --locked -p vcon-server)\n"${test[@]}" -- --skip mlock_', install=False))
        self.assertTrue(any("asan" in p for p in found), found)

    def test_doc_tests_may_run_under_cargo_test(self):
        self.assertEqual(problems(hook="scripts/nextest.sh run --workspace\ncargo test --workspace --locked --quiet --doc\n"), [])

    def test_miri_may_run_under_cargo_test(self):
        self.assertEqual(problems(miri=job("cargo \"+$NIGHTLY\" miri test --locked -p vcon-server --lib", install=False)), [])

    def test_cargo_llvm_cov_without_nextest_is_refused(self):
        found = problems(coverage=job("cargo llvm-cov --workspace --locked --lcov --output-path lcov.info"))
        self.assertTrue(any("coverage" in p and "llvm-cov" in p for p in found), found)

    def test_cargo_llvm_cov_nextest_and_report_are_allowed(self):
        cov = job("scripts/nextest.sh check\ncargo llvm-cov nextest --workspace --locked --lcov --output-path lcov.info\ncargo llvm-cov report --summary-only")
        self.assertEqual(problems(coverage=cov), [])

    def test_a_comment_naming_cargo_test_is_not_a_run(self):
        self.assertEqual(problems(hook="# cargo test has no deadline\nscripts/nextest.sh run --workspace\ncargo test --workspace --doc\n"), [])


class Nextest(unittest.TestCase):
    def test_nextest_called_directly_is_refused(self):
        found = problems(rust=job("scripts/nextest.sh run --workspace\ncargo test --workspace --doc\ncargo nextest run --lib"))
        self.assertEqual(found, ["ci.yml job rust: cargo nextest called directly; use scripts/nextest.sh (pinned version, the repository's config): cargo nextest run --lib"])

    def test_a_job_that_runs_nextest_without_installing_it_is_refused(self):
        found = problems(rust=job("scripts/nextest.sh run --workspace", install=False))
        self.assertTrue(any("rust" in p and "install" in p for p in found), found)

    def test_llvm_cov_nextest_needs_the_version_check(self):
        found = problems(coverage=job("cargo llvm-cov nextest --workspace"))
        self.assertTrue(any("coverage" in p and "check" in p for p in found), found)

    def test_the_hook_must_run_the_workspace_under_nextest(self):
        found = problems(hook="cargo test --doc\n")
        self.assertTrue(any("pre-commit" in p and "nextest" in p for p in found), found)

    def test_the_hook_must_run_the_doc_tests(self):
        found = problems(hook="scripts/nextest.sh run --workspace\n")
        self.assertTrue(any("pre-commit" in p and "--doc" in p for p in found), found)

    def test_the_rust_job_must_run_the_doc_tests(self):
        found = problems(rust=job("scripts/nextest.sh run --workspace"))
        self.assertTrue(any("rust" in p and "--doc" in p for p in found), found)


class Repository(unittest.TestCase):
    def test_this_repository_keeps_the_rule(self):
        root = HERE.parent
        self.assertEqual(check.repository_problems(root), [])


if __name__ == "__main__":
    unittest.main()
