"""Tests scripts/check_required_checks.py: every check the main branch's
ruleset requires is a job that reports on every pull request, and every job
that runs on every pull request is required or gated by a required job.

Run with: python3 -I scripts/test_check_required_checks.py
"""

import contextlib
import importlib.util
import io
import json
import pathlib
import sys
import tempfile
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
            "analyze": job("Analyze (${{ matrix.language }})", strategy={"matrix": {"language": ["actions", "rust"]}},
                           steps=[{"uses": "github/codeql-action/init@0000000000000000000000000000000000000000"},
                                  {"uses": "github/codeql-action/analyze@0000000000000000000000000000000000000000",
                                   "with": {"category": "/language:${{ matrix.language }}"}}]),
        }},
        "osv-scanner.yml": {True: {"pull_request": None, "workflow_dispatch": None}, "jobs": {"scan": job("OSV (all lockfiles)")}},
        "sanitizers.yml": {True: {"pull_request": {"paths": ["crates/**"]}}, "jobs": {"asan": job("AddressSanitizer (vcon-server)")}},
    }
    out.update(changes)
    return out


REQUIRED = ["fmt, clippy, tests", "CI success", "Analyze (actions)", "Analyze (rust)", "OSV (all lockfiles)", "CodeQL"]
# Ruleset 25842923 ("main") from 2026-11-18, when it began to require `DCO`
# and stopped requiring the CLA assistant's `license/cla`.
LIVE = ["fmt, clippy, tests", "coverage", "no AI attribution or personal data", "website",
        "logo files match their source", "DCO", "console in a browser",
        "client examples against a live server", "git hooks", "CI success", "Analyze (actions)",
        "Analyze (javascript-typescript)", "Analyze (rust)", "OSV (all lockfiles)", "CodeQL"]
NO_CLA = "the ruleset requires 'license/cla', which no workflow job reports"
UNSCANNED = "the ruleset requires 'CodeQL', which code scanning reports only on a pull request whose CodeQL analysis is uploaded, and no workflow that runs on every pull request uploads one"


class Required(unittest.TestCase):
    def test_every_required_check_reports_and_every_every_pr_job_is_required(self):
        self.assertEqual(check.problems(workflows(), REQUIRED), [])

    def test_a_job_on_every_pr_that_gates_nothing_is_named(self):
        found = check.problems(workflows(), [r for r in REQUIRED if r != "OSV (all lockfiles)"])
        self.assertEqual(found, ["osv-scanner.yml: 'OSV (all lockfiles)' runs on every pull request, but no required check needs it"])

    def test_each_matrix_name_is_a_check_of_its_own(self):
        found = check.problems(workflows(), [r for r in REQUIRED if r != "Analyze (rust)"])
        self.assertEqual(found, ["codeql.yml: 'Analyze (rust)' runs on every pull request, but no required check needs it"])

    def test_each_matrix_include_entry_is_a_check_of_its_own(self):
        codeql = workflows()["codeql.yml"]
        codeql["jobs"]["analyze"]["strategy"] = {"matrix": {"include": [
            {"language": "actions", "bundle": "actions"}, {"language": "rust", "bundle": "rust"}]}}
        self.assertEqual(check.problems(workflows(**{"codeql.yml": codeql}), REQUIRED), [],
                         "Analyze (actions) and Analyze (rust) come from the include entries")

    def test_a_matrix_include_entry_without_the_name_s_key_is_built_at_run_time(self):
        codeql = workflows()["codeql.yml"]
        codeql["jobs"]["analyze"]["strategy"] = {"matrix": {"include": [{"language": "rust"}, {"bundle": "actions"}]}}
        found = check.problems(workflows(**{"codeql.yml": codeql}), REQUIRED)
        self.assertIn("the ruleset requires 'Analyze (rust)', which no workflow job reports", found,
                      "an entry that leaves the key out cannot be named, so no name is assumed")

    def test_a_matrix_of_lists_and_include_is_built_at_run_time(self):
        codeql = workflows()["codeql.yml"]
        codeql["jobs"]["analyze"]["strategy"] = {"matrix": {"language": ["actions", "rust"], "include": [{"language": "go"}]}}
        found = check.problems(workflows(**{"codeql.yml": codeql}), REQUIRED)
        self.assertIn("the ruleset requires 'Analyze (rust)', which no workflow job reports", found,
                      "include on top of lists can add or extend combinations; not read here")
        self.assertIsNone(check.names("analyze", codeql["jobs"]["analyze"]),
                          "no name is read from either part, not the include entry's alone")

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
        self.assertEqual(len(found), 3, found)
        self.assertTrue(all("not on every pull request" in f for f in found[:2]), found)
        self.assertEqual(found[2], UNSCANNED)

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


class CodeScanning(unittest.TestCase):
    """"CodeQL" is the code-scanning check-run (app github-advanced-security),
    not a job: it reports on a pull request once a CodeQL analysis of it is
    uploaded, and fails on a new alert in code the pull request changes."""

    def test_codeql_is_reported_when_a_workflow_on_every_pr_uploads_an_analysis(self):
        self.assertEqual(check.problems(workflows(), REQUIRED), [])

    def test_codeql_from_a_path_filtered_workflow_is_refused(self):
        codeql = workflows()["codeql.yml"]
        codeql[True]["pull_request"]["paths"] = ["crates/**"]
        found = [f for f in check.problems(workflows(**{"codeql.yml": codeql}), REQUIRED) if "'CodeQL'" in f]
        self.assertEqual(found, [UNSCANNED])

    def test_codeql_with_the_upload_turned_off_is_refused(self):
        codeql = workflows()["codeql.yml"]
        codeql["jobs"]["analyze"]["steps"][1]["with"]["upload"] = "never"
        found = check.problems(workflows(**{"codeql.yml": codeql}), REQUIRED)
        self.assertEqual(found, [UNSCANNED])

    def test_codeql_with_no_codeql_workflow_is_refused(self):
        wfs = workflows()
        wfs["codeql.yml"]["jobs"]["analyze"]["steps"] = [{"run": "true"}]
        found = check.problems(wfs, REQUIRED)
        self.assertEqual(found, [UNSCANNED])

    def test_an_uploaded_analysis_whose_check_is_not_required_is_named(self):
        # On #131 the rust analysis found 6 new alerts and "Analyze (rust)"
        # still passed: only the CodeQL check-run fails on them.
        found = check.problems(workflows(), [r for r in REQUIRED if r != "CodeQL"])
        self.assertEqual(found, ["a workflow that runs on every pull request uploads a CodeQL analysis, but the ruleset does not require 'CodeQL', the check that fails on a new alert"])

    def test_upload_values_that_still_upload(self):
        for value in (None, "always", "failure-only", True):
            step = {"uses": "github/codeql-action/analyze@v4"}
            if value is not None:
                step["with"] = {"upload": value}
            self.assertTrue(check.uploads_codeql({"jobs": {"a": {"steps": [step]}}}), value)
        for value in ("never", False, "false"):
            step = {"uses": "github/codeql-action/analyze@v4", "with": {"upload": value}}
            self.assertFalse(check.uploads_codeql({"jobs": {"a": {"steps": [step]}}}), value)


class Contexts(unittest.TestCase):
    def test_the_contexts_come_from_the_branch_rules(self):
        rules = [
            {"type": "deletion"},
            {"type": "required_status_checks", "parameters": {"required_status_checks": [{"context": "CI success", "integration_id": 15368}, {"context": "DCO"}]}},
        ]
        self.assertEqual(check.required_contexts(rules), ["CI success", "DCO"])

    def test_a_ruleset_object_is_read_too(self):
        ruleset = {"id": 25842923, "rules": [{"type": "required_status_checks", "parameters": {"required_status_checks": [{"context": "git hooks"}]}}]}
        self.assertEqual(check.required_contexts(ruleset), ["git hooks"])

    def test_rules_with_no_required_checks_are_refused(self):
        with self.assertRaises(ValueError):
            check.required_contexts([{"type": "deletion"}])


def dco(**more):
    """dco.yml as it is shaped: one job, on every pull request."""
    return {"dco.yml": {True: {"pull_request": None}, "jobs": {"dco": job("DCO", **more)}}}


def contract(**more):
    """contract.yml as it is shaped: one job, on every pull request."""
    return {"contract.yml": {True: {"pull_request": None}, "jobs": {"contract": job("Contract", **more)}}}


ADVISORY_REQUIRED = "the ruleset requires 'DCO', which ADVISORY in scripts/check_required_checks.py still lists: remove it from ADVISORY"
ADVISORY_STALE = "ADVISORY lists 'DCO', which no job that runs on every pull request reports"
DCO_UNNEEDED = "dco.yml: 'DCO' runs on every pull request, but no required check needs it"


class Advisory(unittest.TestCase):
    """A job named in ADVISORY reports on every pull request without being
    required. The tests pass their own list; main passes ADVISORY, which
    holds `Contract` (contract.yml); `DCO` left it on 2026-11-18."""

    def test_an_advisory_job_may_run_on_every_pr_unrequired(self):
        self.assertEqual(check.problems(workflows(**dco()), REQUIRED, {"DCO"}), [])

    def test_without_the_list_the_same_job_is_refused(self):
        self.assertEqual(check.problems(workflows(**dco()), REQUIRED), [DCO_UNNEEDED])

    def test_an_unlisted_job_beside_it_is_still_refused(self):
        wfs = workflows(**dco())
        wfs["dco.yml"]["jobs"]["other"] = job("Other")
        found = check.problems(wfs, REQUIRED, {"DCO"})
        self.assertEqual(found, ["dco.yml: 'Other' runs on every pull request, but no required check needs it"])

    def test_an_advisory_check_the_ruleset_requires_is_refused(self):
        self.assertEqual(check.problems(workflows(**dco()), REQUIRED + ["DCO"], {"DCO"}), [ADVISORY_REQUIRED])

    def test_an_advisory_name_no_job_reports_is_refused(self):
        self.assertEqual(check.problems(workflows(), REQUIRED, {"DCO"}), [ADVISORY_STALE])

    def test_an_advisory_job_of_a_path_filtered_workflow_is_refused(self):
        wfs = workflows(**dco())
        wfs["dco.yml"][True]["pull_request"] = {"paths": ["scripts/**"]}
        self.assertEqual(check.problems(wfs, REQUIRED, {"DCO"}), [ADVISORY_STALE])

    def test_an_advisory_job_a_required_job_needs_is_refused(self):
        ci = workflows()["ci.yml"]
        ci["jobs"]["dco"] = job("DCO")
        ci["jobs"]["ci-success"]["needs"] = ["rust", "fuzz-targets", "fuzz", "dco"]
        found = check.problems(workflows(**{"ci.yml": ci}), REQUIRED, {"DCO"})
        self.assertEqual(found, ["ADVISORY lists 'DCO', but a required check needs it, so a red result blocks the merge: remove it from ADVISORY"])

    def test_advisory_names_compare_exactly(self):
        found = check.problems(workflows(**dco()), REQUIRED, {"dco"})
        self.assertEqual(found, ["ADVISORY lists 'dco', which no job that runs on every pull request reports", DCO_UNNEEDED])

    def test_one_stale_name_of_two_is_named(self):
        found = check.problems(workflows(**dco()), REQUIRED, {"DCO", "Contract"})
        self.assertEqual(found, ["ADVISORY lists 'Contract', which no job that runs on every pull request reports"])

    def test_each_matrix_name_of_an_advisory_job_is_listed_on_its_own(self):
        more = {"strategy": {"matrix": {"part": ["a", "b"]}}}
        wfs = workflows(**{"dco.yml": {True: {"pull_request": None}, "jobs": {"dco": job("DCO (${{ matrix.part }})", **more)}}})
        self.assertEqual(check.problems(wfs, REQUIRED, {"DCO (a)", "DCO (b)"}), [])
        self.assertEqual(check.problems(wfs, REQUIRED, {"DCO (a)"}),
                         ["dco.yml: 'DCO (b)' runs on every pull request, but no required check needs it"])

    def test_the_list_holds_contract_and_is_kept(self):
        # Decision (ix): the advisory jobs are named here, and a name leaves
        # when the ruleset requires it. `DCO` left on 2026-11-18; `Contract`
        # (contract.yml) is advisory until 1.0.
        self.assertEqual(check.ADVISORY, frozenset({"Contract"}))
        self.assertIsInstance(check.ADVISORY, frozenset)

    def test_the_list_allows_no_unrequired_job_but_contract(self):
        found = check.problems(workflows(**dco(), **contract()), REQUIRED, check.ADVISORY)
        self.assertEqual(found, [DCO_UNNEEDED])

    def test_the_list_reports_no_stale_name_beside_the_contract_job(self):
        self.assertEqual(check.problems(workflows(**contract()), REQUIRED, check.ADVISORY), [])

    def test_the_list_without_a_contract_job_names_it_stale(self):
        found = check.problems(workflows(), REQUIRED, check.ADVISORY)
        self.assertEqual(found, ["ADVISORY lists 'Contract', which no job that runs on every pull request reports"])

    def test_a_required_contract_while_listed_is_refused(self):
        found = check.problems(workflows(**contract()), REQUIRED + ["Contract"], check.ADVISORY)
        self.assertEqual(found, ["the ruleset requires 'Contract', which ADVISORY in scripts/check_required_checks.py "
                                 "still lists: remove it from ADVISORY"])

    def test_a_path_filtered_contract_job_is_stale(self):
        wfs = workflows(**contract())
        wfs["contract.yml"][True]["pull_request"] = {"paths": ["contract/**"]}
        found = check.problems(wfs, REQUIRED, check.ADVISORY)
        self.assertEqual(found, ["ADVISORY lists 'Contract', which no job that runs on every pull request reports"])

    def test_the_required_dco_job_passes_with_the_list(self):
        found = check.problems(workflows(**dco(), **contract()), REQUIRED + ["DCO"], check.ADVISORY)
        self.assertEqual(found, [])


def run_main(contexts):
    """main's exit status and standard error for a ruleset that requires
    `contexts`, against this repository's workflows."""
    rules = [{"type": "required_status_checks", "parameters": {"required_status_checks": [{"context": c} for c in contexts]}}]
    with tempfile.TemporaryDirectory() as tmp:
        path = pathlib.Path(tmp, "rules.json")
        path.write_text(json.dumps(rules), encoding="utf-8")
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = check.main(["check_required_checks.py", str(path)])
    return rc, err.getvalue()


class Retired(unittest.TestCase):
    """The CLA retired on 2026-11-18: the ruleset requires `DCO` and no
    longer requires the CLA assistant's `license/cla`."""

    def test_no_context_is_external(self):
        self.assertEqual(check.EXTERNAL, frozenset())
        self.assertIsInstance(check.EXTERNAL, frozenset)

    def test_a_ruleset_that_still_requires_license_cla_is_refused(self):
        found = check.problems(workflows(**contract()), REQUIRED + ["license/cla"], check.ADVISORY)
        self.assertEqual(found, [NO_CLA])

    def test_license_cla_is_refused_beside_a_required_dco(self):
        found = check.problems(workflows(**dco(), **contract()), REQUIRED + ["DCO", "license/cla"], check.ADVISORY)
        self.assertEqual(found, [NO_CLA])

    def test_the_live_ruleset_passes_against_this_repository(self):
        self.assertEqual(check.problems(check.read_workflows(HERE.parent), LIVE, check.ADVISORY), [])

    def test_the_live_ruleset_requires_dco_and_not_license_cla(self):
        self.assertIn("DCO", LIVE)
        self.assertNotIn("license/cla", LIVE)
        self.assertEqual(len(LIVE), len(set(LIVE)), "a context twice")

    def test_this_repository_s_dco_job_runs_on_every_pr(self):
        real = check.read_workflows(HERE.parent)
        names = {n for file, _, ns in check.every_pr_jobs(real) if file == "dco.yml" for n in (ns or [])}
        self.assertEqual(names, {"DCO"})

    def test_this_repository_s_dco_job_unrequired_is_refused(self):
        found = check.problems(check.read_workflows(HERE.parent), [c for c in LIVE if c != "DCO"], check.ADVISORY)
        self.assertEqual(found, [DCO_UNNEEDED])

    def test_main_passes_the_live_ruleset(self):
        rc, err = run_main(LIVE)
        self.assertEqual(rc, 0, err)

    def test_main_refuses_the_ruleset_from_before_the_switch(self):
        before = [c for c in LIVE if c != "DCO"] + ["license/cla"]
        rc, err = run_main(before)
        self.assertEqual(rc, 1)
        self.assertIn(f"check_required_checks: {NO_CLA}", err)
        self.assertIn(f"check_required_checks: {DCO_UNNEEDED}", err)

    def test_main_refuses_license_cla_named_again_after_the_switch(self):
        rc, err = run_main(LIVE + ["license/cla"])
        self.assertEqual((rc, err), (1, f"check_required_checks: {NO_CLA}\n"))

    def test_the_cla_context_compares_exactly(self):
        found = check.problems(workflows(**contract()), REQUIRED + ["License/CLA"], check.ADVISORY)
        self.assertEqual(found, ["the ruleset requires 'License/CLA', which no workflow job reports"])


class Repository(unittest.TestCase):
    def test_this_repository_s_workflows_parse(self):
        names = check.every_pr_jobs(check.read_workflows(HERE.parent))
        self.assertIn("CI success", {n for _, _, ns in names for n in (ns or [])})

    def test_this_repository_s_codeql_workflow_uploads_on_every_pr(self):
        found = check.problems(check.read_workflows(HERE.parent), ["CodeQL"])
        self.assertEqual([f for f in found if "'CodeQL'" in f], [])


if __name__ == "__main__":
    unittest.main()
