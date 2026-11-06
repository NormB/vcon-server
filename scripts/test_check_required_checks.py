"""Tests scripts/check_required_checks.py: every check the main branch's
ruleset requires is a job that reports on every pull request, and every job
that runs on every pull request is required or gated by a required job.

Run with: python3 -I scripts/test_check_required_checks.py
"""

import importlib.util
import pathlib
import sys
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_required_checks", HERE / "check_required_checks.py")
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def job(name, **more):
    return {"name": name, "runs-on": "ubuntu-latest", "steps": [{"run": "true"}], **more}


def workflows(**changes):
    """ci.yml, codeql.yml and osv-scanner.yml as they are shaped, and
    sanitizers.yml, filtered by path. PyYAML reads the key `on` as True."""
    out = {
        "ci.yml": {True: {"push": {"branches": ["main"]}, "pull_request": None}, "jobs": {
            "rust": job("fmt, clippy, tests"),
            "fuzz-targets": job("list the fuzz targets"),
            "fuzz": job("fuzz ${{ matrix.target }}", needs="fuzz-targets",
                        strategy={"matrix": {"target": "${{ fromJSON(needs.fuzz-targets.outputs.targets) }}"}}),
            "ci-success": job("CI success", needs=["rust", "fuzz-targets", "fuzz"]),
        }},
        "codeql.yml": {True: {"pull_request": {"branches": ["main"]}}, "jobs": {
            "analyze": job("Analyze (${{ matrix.language }})", strategy={"matrix": {"language": ["actions", "rust"]}}),
        }},
        "osv-scanner.yml": {True: {"pull_request": None, "workflow_dispatch": None}, "jobs": {"scan": job("OSV (all lockfiles)")}},
        "sanitizers.yml": {True: {"pull_request": {"paths": ["crates/**"]}}, "jobs": {"asan": job("AddressSanitizer (vcon-server)")}},
    }
    out.update(changes)
    return out


REQUIRED = ["fmt, clippy, tests", "CI success", "license/cla", "Analyze (actions)", "Analyze (rust)", "OSV (all lockfiles)"]


class Required(unittest.TestCase):
    def test_every_required_check_reports_and_every_every_pr_job_is_required(self):
        self.assertEqual(check.problems(workflows(), REQUIRED), [])

    def test_a_job_on_every_pr_that_gates_nothing_is_named(self):
        found = check.problems(workflows(), [r for r in REQUIRED if r != "OSV (all lockfiles)"])
        self.assertEqual(found, ["osv-scanner.yml: 'OSV (all lockfiles)' runs on every pull request, but no required check needs it"])

    def test_each_matrix_name_is_a_check_of_its_own(self):
        found = check.problems(workflows(), [r for r in REQUIRED if r != "Analyze (rust)"])
        self.assertEqual(found, ["codeql.yml: 'Analyze (rust)' runs on every pull request, but no required check needs it"])

    def test_a_job_a_required_job_needs_is_gated(self):
        # fuzz and fuzz-targets are not required by name; CI success needs them.
        self.assertEqual(check.problems(workflows(), REQUIRED), [])

    def test_a_job_needed_only_through_another_is_gated(self):
        ci = workflows()["ci.yml"]
        ci["jobs"]["ci-success"]["needs"] = ["rust", "fuzz"]
        self.assertEqual(check.problems(workflows(**{"ci.yml": ci}), REQUIRED), [])

    def test_a_required_check_from_a_path_filtered_workflow_is_refused(self):
        found = check.problems(workflows(), REQUIRED + ["AddressSanitizer (vcon-server)"])
        self.assertEqual(len(found), 1)
        self.assertIn("not on every pull request", found[0])

    def test_a_required_check_no_job_has_is_refused(self):
        found = check.problems(workflows(), REQUIRED + ["Analyze (python)"])
        self.assertEqual(found, ["the ruleset requires 'Analyze (python)', which no workflow job reports"])

    def test_a_required_check_of_a_matrix_built_at_run_time_is_refused(self):
        found = check.problems(workflows(), REQUIRED + ["fuzz parse_vcon"])
        self.assertEqual(found, ["the ruleset requires 'fuzz parse_vcon', which no workflow job reports"])

    def test_a_matrix_built_at_run_time_that_nothing_needs_is_named_once(self):
        ci = workflows()["ci.yml"]
        ci["jobs"]["ci-success"]["needs"] = ["rust", "fuzz-targets"]
        found = check.problems(workflows(**{"ci.yml": ci}), REQUIRED)
        self.assertEqual(found, ["ci.yml: 'fuzz ${{ matrix.target }}' runs on every pull request, but no required check needs it"])

    def test_a_workflow_on_pull_requests_to_other_branches_only_is_not_every_pr(self):
        codeql = workflows()["codeql.yml"]
        codeql[True]["pull_request"]["branches"] = ["release/*"]
        found = check.problems(workflows(**{"codeql.yml": codeql}), REQUIRED)
        self.assertTrue(all("not on every pull request" in f for f in found) and len(found) == 2, found)

    def test_paths_ignore_is_a_filter_too(self):
        osv = workflows()["osv-scanner.yml"]
        osv[True]["pull_request"] = {"paths-ignore": ["docs/**"]}
        found = check.problems(workflows(**{"osv-scanner.yml": osv}), REQUIRED)
        self.assertEqual(len(found), 1)
        self.assertIn("not on every pull request", found[0])

    def test_the_on_key_spelled_out_is_read_too(self):
        osv = workflows()["osv-scanner.yml"]
        osv["on"] = osv.pop(True)
        self.assertEqual(check.problems(workflows(**{"osv-scanner.yml": osv}), REQUIRED), [])

    def test_a_workflow_with_no_pull_request_trigger_needs_nothing(self):
        extra = {True: {"schedule": [{"cron": "1 1 * * 1"}]}, "jobs": {"s": job("Scorecard")}}
        self.assertEqual(check.problems(workflows(**{"scorecard.yml": extra}), REQUIRED), [])

    def test_a_trigger_given_as_a_list_is_read(self):
        osv = workflows()["osv-scanner.yml"]
        osv[True] = ["push", "pull_request"]
        self.assertEqual(check.problems(workflows(**{"osv-scanner.yml": osv}), REQUIRED), [])


class Contexts(unittest.TestCase):
    def test_the_contexts_come_from_the_branch_rules(self):
        rules = [
            {"type": "deletion"},
            {"type": "required_status_checks", "parameters": {"required_status_checks": [{"context": "CI success", "integration_id": 15368}, {"context": "license/cla"}]}},
        ]
        self.assertEqual(check.required_contexts(rules), ["CI success", "license/cla"])

    def test_a_ruleset_object_is_read_too(self):
        ruleset = {"id": 25842923, "rules": [{"type": "required_status_checks", "parameters": {"required_status_checks": [{"context": "git hooks"}]}}]}
        self.assertEqual(check.required_contexts(ruleset), ["git hooks"])

    def test_rules_with_no_required_checks_are_refused(self):
        with self.assertRaises(ValueError):
            check.required_contexts([{"type": "deletion"}])


class Repository(unittest.TestCase):
    def test_this_repository_s_workflows_parse(self):
        names = check.every_pr_jobs(check.read_workflows(HERE.parent))
        self.assertIn("CI success", {n for _, _, ns in names for n in (ns or [])})


if __name__ == "__main__":
    unittest.main()
