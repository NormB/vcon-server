"""Tests scripts/ci_success.py: the "CI success" job of
.github/workflows/ci.yml passes only when every other job of the workflow
succeeded, and the workflow keeps every job under it.

Run with: python3 -I scripts/test_ci_success.py
"""

import importlib.util
import pathlib
import sys
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("ci_success", HERE / "ci_success.py")
ci_success = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_success)

# The jobs of ci.yml on 2026-11-05.
JOBS = ["rust", "coverage", "fuzz-targets", "fuzz", "clients", "scan", "hooks", "messages", "deny", "site", "brand", "console", "docs-binary", "docs"]


def needs(**results):
    """`needs` as toJSON(needs) gives it: every job succeeded, except the
    results given (a job id's dashes written as underscores)."""
    out = {job: {"result": "success", "outputs": {}} for job in JOBS}
    for job, result in results.items():
        out[job.replace("_", "-")]["result"] = result
    return out


def aggregator(**changes):
    job = {
        "name": "CI success",
        "if": "always()",
        "needs": list(JOBS),
        "runs-on": "ubuntu-latest",
        "steps": [
            {"uses": "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"},
            {"name": "Every job above succeeded", "env": {"NEEDS": "${{ toJSON(needs) }}"}, "run": "python3 -I scripts/ci_success.py verdict"},
        ],
    }
    job.update(changes)
    return job


def workflow(**changes):
    jobs = {job: {"name": job, "runs-on": "ubuntu-latest", "steps": [{"run": "true"}]} for job in JOBS}
    jobs["fuzz"]["needs"] = "fuzz-targets"
    jobs["docs"]["needs"] = "docs-binary"
    jobs["ci-success"] = aggregator(**changes)
    return {"name": "CI", "jobs": jobs}


class Verdict(unittest.TestCase):
    def test_every_job_succeeded_passes(self):
        self.assertEqual(ci_success.failures(needs()), [])

    def test_a_failed_how_to_scenario_fails(self):
        self.assertEqual(ci_success.failures(needs(docs="failure")), ["docs: failure"])

    def test_a_failed_fuzz_target_fails(self):
        self.assertEqual(ci_success.failures(needs(fuzz="failure")), ["fuzz: failure"])

    def test_a_skipped_job_fails(self):
        # No job of ci.yml has a condition of its own (see Workflow below),
        # so a skip only ever means a job it needs did not succeed.
        self.assertEqual(ci_success.failures(needs(docs="skipped")), ["docs: skipped"])

    def test_a_cancelled_job_fails(self):
        self.assertEqual(ci_success.failures(needs(fuzz="cancelled")), ["fuzz: cancelled"])

    def test_the_build_failing_reports_it_and_the_scenarios_it_stopped(self):
        self.assertEqual(
            ci_success.failures(needs(docs_binary="failure", docs="skipped")),
            ["docs: skipped", "docs-binary: failure"],
        )

    def test_a_job_with_no_result_fails(self):
        given = needs()
        del given["deny"]["result"]
        self.assertEqual(ci_success.failures(given), ["deny: no result"])

    def test_no_jobs_at_all_fails(self):
        self.assertEqual(ci_success.failures({}), ["no job results: the job needs nothing"])


class Workflow(unittest.TestCase):
    def test_the_aggregator_as_written_passes(self):
        self.assertEqual(ci_success.workflow_problems(workflow()), [])

    def test_a_job_left_out_of_needs_is_found(self):
        self.assertEqual(
            ci_success.workflow_problems(workflow(needs=[j for j in JOBS if j not in ("docs", "fuzz")])),
            ["ci-success does not need: docs, fuzz"],
        )

    def test_a_job_named_in_needs_that_does_not_exist_is_found(self):
        self.assertEqual(ci_success.workflow_problems(workflow(needs=JOBS + ["gone"])), ["ci-success needs jobs ci.yml lacks: gone"])

    def test_a_single_need_written_as_a_string_is_read(self):
        wf = workflow(needs="rust")
        self.assertIn("ci-success does not need: " + ", ".join(sorted(j for j in JOBS if j != "rust")), ci_success.workflow_problems(wf))

    def test_without_always_a_failure_would_skip_the_aggregator_and_a_skip_passes(self):
        self.assertEqual(ci_success.workflow_problems(workflow(**{"if": "success()"})), ["ci-success must run with if: always(), not success()"])
        self.assertEqual(ci_success.workflow_problems(workflow(**{"if": "${{ !cancelled() }}"})), ["ci-success must run with if: always(), not ${{ !cancelled() }}"])

    def test_always_inside_an_expression_is_the_same(self):
        self.assertEqual(ci_success.workflow_problems(workflow(**{"if": "${{ always() }}"})), [])

    def test_a_missing_condition_is_found(self):
        wf = workflow()
        del wf["jobs"]["ci-success"]["if"]
        self.assertEqual(ci_success.workflow_problems(wf), ["ci-success must run with if: always(), not (none)"])

    def test_the_check_name_the_ruleset_requires_is_kept(self):
        self.assertEqual(ci_success.workflow_problems(workflow(name="CI ok")), ["ci-success must be named 'CI success', not 'CI ok'"])

    def test_the_verdict_must_be_given_the_results(self):
        steps = [{"run": "python3 -I scripts/ci_success.py verdict"}]
        self.assertEqual(ci_success.workflow_problems(workflow(steps=steps)), ["ci-success has no step running 'scripts/ci_success.py verdict' with NEEDS: ${{ toJSON(needs) }}"])

    def test_a_missing_aggregator_is_found(self):
        wf = workflow()
        del wf["jobs"]["ci-success"]
        self.assertEqual(ci_success.workflow_problems(wf), ["ci.yml has no ci-success job"])

    def test_a_job_with_its_own_condition_must_be_decided_here(self):
        # The verdict fails a skipped job. A job that may skip by its own
        # rule would then fail every run it skips, so it is refused until
        # the verdict is given a rule for it.
        wf = workflow()
        wf["jobs"]["coverage"]["if"] = "github.event_name == 'push'"
        self.assertEqual(ci_success.workflow_problems(wf), ["coverage has a condition of its own (if: github.event_name == 'push'); the verdict fails a skipped job"])


if __name__ == "__main__":
    unittest.main()
