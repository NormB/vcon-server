"""Tests scripts/codeql_pr_gate.py: an Analyze job of codeql.yml passes only
when code scanning processed the SARIF it uploaded and, on a pull request,
the pull request's merge commit has no open CodeQL alert of the job's
category that main does not have.

Run with: python3 -I scripts/test_codeql_pr_gate.py
"""

import importlib.util
import io
import json
import pathlib
import subprocess
import sys
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("codeql_pr_gate", HERE / "codeql_pr_gate.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)

REPO = "NormB/vcon-server"
SARIF = "f08077dc-e322-11f1-8a25-93b32890ae38"
JS = "/language:javascript-typescript"
RUST = "/language:rust"
PR_REF = "refs%2Fpull%2F7%2Fmerge"
MAIN_REF = "refs%2Fheads%2Fmain"


def alert(number, category=JS, rule="js/incomplete-sanitization", path="console-tests/tests/format.spec.js", line=210):
    return {
        "number": number,
        "rule": {"id": rule, "severity": "warning"},
        "html_url": f"https://github.com/{REPO}/security/code-scanning/{number}",
        "most_recent_instance": {"category": category, "location": {"path": path, "start_line": line}},
    }


class FakeApi:
    """Answers each API path from `answers` (a list is answered in turn, the
    last one again) and records every path asked."""

    def __init__(self, answers):
        self.answers = answers
        self.asked = []

    def __call__(self, path):
        self.asked.append(path)
        for prefix, answer in self.answers.items():
            if path.startswith(prefix):
                if isinstance(answer, Exception):
                    raise answer
                if isinstance(answer, list) and answer and isinstance(answer[0], tuple):
                    return answer.pop(0)[0] if len(answer) > 1 else answer[0][0]
                return answer
        raise gate.GateError(f"no answer for {path}")


class Clock:
    def __init__(self):
        self.now = 0.0
        self.slept = []

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.slept.append(seconds)
        self.now += seconds


def status(value, errors=None):
    return {"processing_status": value, "errors": errors, "analyses_url": "x"}


def sarif_path(sarif=SARIF):
    return f"repos/{REPO}/code-scanning/sarifs/{sarif}"


def alerts_path(ref):
    return f"repos/{REPO}/code-scanning/alerts?ref={ref}&state=open&tool_name=CodeQL&per_page=100&page="


def run(argv, answers, env=None):
    """main with `answers`: (code, stdout, stderr, api, clock)."""
    api, clock, out, err = FakeApi(answers), Clock(), io.StringIO(), io.StringIO()
    env = {"GITHUB_REPOSITORY": REPO} if env is None else env
    code = gate.main(argv, env, api, clock, clock.sleep, out, err)
    return code, out.getvalue(), err.getvalue(), api, clock


def pr_args(pr="7", category=JS, wait="900"):
    return ["check", "--sarif-id", SARIF, "--category", category, "--pr", pr, "--base", "main", "--wait", wait]


def complete_with(pr_alerts, main_alerts):
    return {
        sarif_path(): status("complete"),
        alerts_path(PR_REF): pr_alerts,
        alerts_path(MAIN_REF): main_alerts,
    }


class Processed(unittest.TestCase):
    # Success.
    def test_a_complete_sarif_passes(self):
        code, out, err, _, _ = run(pr_args(pr=""), {sarif_path(): status("complete")})
        self.assertEqual(code, 0, err)
        self.assertIn(f"code scanning processed SARIF {SARIF}", out)

    def test_a_pending_sarif_is_asked_again_until_complete(self):
        answers = {sarif_path(): [(status("pending"),), (status("pending"),), (status("complete"),)]}
        code, _, err, api, _ = run(pr_args(pr=""), answers)
        self.assertEqual(code, 0, err)
        self.assertEqual(api.asked.count(sarif_path()), 3, api.asked)

    def test_the_wait_between_asks_doubles_up_to_a_minute(self):
        answers = {sarif_path(): [(status("pending"),)] * 7 + [(status("complete"),)]}
        _, _, _, _, clock = run(pr_args(pr=""), answers)
        self.assertEqual(clock.slept, [5, 10, 20, 40, 60, 60, 60])

    def test_a_push_to_main_checks_only_the_processing(self):
        code, _, err, api, _ = run(pr_args(pr=""), {sarif_path(): status("complete")})
        self.assertEqual(code, 0, err)
        self.assertEqual(api.asked, [sarif_path()], "no alert is asked for without a pull request")

    # Failure.
    def test_a_failed_sarif_is_refused_with_its_errors(self):
        code, _, err, _, _ = run(pr_args(pr=""), {sarif_path(): status("failed", ["invalid SARIF"])})
        self.assertEqual(code, 1)
        self.assertIn(f"code scanning failed to process SARIF {SARIF}: invalid SARIF", err)

    def test_a_sarif_still_pending_at_the_deadline_is_refused(self):
        code, _, err, _, clock = run(pr_args(pr="", wait="300"), {sarif_path(): status("pending")})
        self.assertEqual(code, 1)
        self.assertIn(f"SARIF {SARIF} was not processed in 300 s", err)
        self.assertGreaterEqual(clock.now, 300)

    def test_the_deadline_is_not_passed_by_the_last_wait(self):
        _, _, _, _, clock = run(pr_args(pr="", wait="30"), {sarif_path(): status("pending")})
        self.assertEqual(clock.now, 30, clock.slept)

    def test_no_sarif_id_is_refused(self):
        argv = ["check", "--sarif-id", "", "--category", JS, "--pr", "", "--base", "main"]
        code, _, err, api, _ = run(argv, {})
        self.assertEqual(code, 1)
        self.assertIn("the analysis uploaded no SARIF", err)
        self.assertEqual(api.asked, [], "nothing to ask")

    def test_an_unknown_status_is_refused(self):
        code, _, err, _, _ = run(pr_args(pr=""), {sarif_path(): status("weird")})
        self.assertEqual(code, 1)
        self.assertIn("unknown processing status 'weird'", err)

    def test_an_answer_with_no_status_is_refused(self):
        code, _, err, _, _ = run(pr_args(pr=""), {sarif_path(): {"errors": None}})
        self.assertEqual(code, 1)
        self.assertIn("unknown processing status None", err)

    def test_an_api_error_answers_2(self):
        code, _, err, _, _ = run(pr_args(pr=""), {sarif_path(): gate.GateError("HTTP 403")})
        self.assertEqual(code, 2)
        self.assertIn("codeql gate: cannot read code scanning: HTTP 403", err)

    def test_a_failed_sarif_is_not_followed_by_the_alerts(self):
        _, _, _, api, _ = run(pr_args(), {sarif_path(): status("failed", ["x"])})
        self.assertEqual(api.asked, [sarif_path()])

    def test_a_failed_sarif_with_no_errors_says_none_were_given(self):
        _, _, err, _, _ = run(pr_args(pr=""), {sarif_path(): status("failed")})
        self.assertIn("no errors given", err)

    # Edge.
    def test_a_wait_of_0_asks_once(self):
        code, _, _, api, clock = run(pr_args(pr="", wait="0"), {sarif_path(): status("pending")})
        self.assertEqual(code, 1)
        self.assertEqual((api.asked.count(sarif_path()), clock.slept), (1, []))

    def test_complete_after_a_pending_is_one_wait(self):
        _, _, _, _, clock = run(pr_args(pr=""), {sarif_path(): [(status("pending"),), (status("complete"),)]})
        self.assertEqual(clock.slept, [5])

    def test_the_sarif_id_is_quoted_in_the_path(self):
        argv = ["check", "--sarif-id", "a/b", "--category", JS, "--pr", "", "--base", "main"]
        _, _, _, api, _ = run(argv, {f"repos/{REPO}/code-scanning/sarifs/a%2Fb": status("complete")})
        self.assertEqual(api.asked, [f"repos/{REPO}/code-scanning/sarifs/a%2Fb"])


class NewAlerts(unittest.TestCase):
    # Success.
    def test_no_alert_on_the_pull_request_passes(self):
        code, out, err, _, _ = run(pr_args(), complete_with([], []))
        self.assertEqual(code, 0, err)
        self.assertIn(f"pull request 7 adds no CodeQL alert in {JS}", out)

    def test_an_alert_main_has_too_passes(self):
        code, _, err, _, _ = run(pr_args(), complete_with([alert(5)], [alert(5)]))
        self.assertEqual(code, 0, err)

    def test_an_alert_of_another_category_is_left_to_its_own_job(self):
        code, _, err, _, _ = run(pr_args(), complete_with([alert(9, category=RUST)], []))
        self.assertEqual(code, 0, err)

    def test_the_pull_request_and_main_are_both_asked(self):
        _, _, _, api, _ = run(pr_args(), complete_with([], []))
        self.assertEqual(api.asked, [sarif_path(), alerts_path(PR_REF) + "1", alerts_path(MAIN_REF) + "1"])

    def test_main_is_asked_by_the_base_branch_given(self):
        argv = pr_args()
        argv[argv.index("--base") + 1] = "release"
        answers = {sarif_path(): status("complete"), alerts_path(PR_REF): [], alerts_path("refs%2Fheads%2Frelease"): []}
        code, _, err, _, _ = run(argv, answers)
        self.assertEqual(code, 0, err)

    def test_a_full_page_asks_the_next(self):
        page1 = [alert(n) for n in range(1, 101)]
        answers = {sarif_path(): status("complete"), alerts_path(PR_REF) + "1": page1, alerts_path(PR_REF) + "2": [],
                   alerts_path(MAIN_REF) + "1": page1, alerts_path(MAIN_REF) + "2": []}
        code, _, err, api, _ = run(pr_args(), answers)
        self.assertEqual(code, 0, err)
        self.assertIn(alerts_path(PR_REF) + "2", api.asked)

    # Failure.
    def test_an_alert_main_does_not_have_is_refused(self):
        code, _, err, _, _ = run(pr_args(), complete_with([alert(77)], []))
        self.assertEqual(code, 1)
        self.assertIn(f"pull request 7 adds 1 CodeQL alert in {JS}", err)

    def test_the_refusal_names_the_rule_the_place_and_the_link(self):
        _, _, err, _, _ = run(pr_args(), complete_with([alert(77)], []))
        self.assertIn(
            f"#77 js/incomplete-sanitization (warning) console-tests/tests/format.spec.js:210 https://github.com/{REPO}/security/code-scanning/77",
            err,
        )

    def test_each_new_alert_is_named_in_number_order(self):
        _, _, err, _, _ = run(pr_args(), complete_with([alert(80), alert(78), alert(5)], [alert(5)]))
        lines = [l for l in err.splitlines() if l.startswith("  #")]
        self.assertEqual([l.split()[0] for l in lines], ["#78", "#80"])
        self.assertIn("adds 2 CodeQL alerts", err)

    def test_an_alert_on_a_later_page_is_refused(self):
        page1 = [alert(n) for n in range(1, 101)]
        answers = {sarif_path(): status("complete"), alerts_path(PR_REF) + "1": page1, alerts_path(PR_REF) + "2": [alert(500)],
                   alerts_path(MAIN_REF) + "1": page1, alerts_path(MAIN_REF) + "2": []}
        code, _, err, _, _ = run(pr_args(), answers)
        self.assertEqual(code, 1)
        self.assertIn("#500", err)

    def test_an_alert_main_has_only_on_a_later_page_passes(self):
        page1 = [alert(n) for n in range(1, 101)]
        answers = {sarif_path(): status("complete"), alerts_path(PR_REF) + "1": [alert(150)],
                   alerts_path(MAIN_REF) + "1": page1, alerts_path(MAIN_REF) + "2": [alert(150)]}
        code, _, err, _, _ = run(pr_args(), answers)
        self.assertEqual(code, 0, err)

    def test_an_api_error_on_the_alerts_answers_2(self):
        answers = {sarif_path(): status("complete"), alerts_path(PR_REF): gate.GateError("HTTP 404")}
        code, _, err, _, _ = run(pr_args(), answers)
        self.assertEqual(code, 2)
        self.assertIn("HTTP 404", err)

    def test_an_answer_that_is_not_a_list_answers_2(self):
        answers = {sarif_path(): status("complete"), alerts_path(PR_REF): {"message": "no analysis found"}}
        code, _, err, _, _ = run(pr_args(), answers)
        self.assertEqual(code, 2)
        self.assertIn("not a list of alerts", err)

    def test_the_rust_job_refuses_a_rust_alert(self):
        code, _, err, _, _ = run(pr_args(category=RUST), complete_with([alert(81, category=RUST, rule="rust/cleartext-logging")], []))
        self.assertEqual(code, 1)
        self.assertIn("rust/cleartext-logging", err)

    # Edge.
    def test_an_alert_with_no_instance_counts_for_every_job(self):
        bare = {"number": 90, "rule": {"id": "js/x"}, "html_url": "u"}
        code, _, err, _, _ = run(pr_args(), complete_with([bare], []))
        self.assertEqual(code, 1, "no category: it cannot be left to another job")
        self.assertIn("#90 js/x", err)

    def test_an_alert_with_no_location_is_named_without_one(self):
        bare = {"number": 91, "rule": {"id": "js/y", "severity": "error"}, "html_url": "u", "most_recent_instance": {"category": JS}}
        _, _, err, _, _ = run(pr_args(), complete_with([bare], []))
        self.assertIn("#91 js/y (error) (no location) u", err)

    def test_an_alert_that_is_not_a_mapping_answers_2(self):
        code, _, err, _, _ = run(pr_args(), complete_with(["x"], []))
        self.assertEqual(code, 2)
        self.assertIn("not a list of alerts", err)

    def test_the_merge_ref_of_the_pull_request_is_quoted(self):
        _, _, _, api, _ = run(pr_args(pr="12"), {sarif_path(): status("complete"), alerts_path("refs%2Fpull%2F12%2Fmerge"): [],
                                                   alerts_path(MAIN_REF): []})
        self.assertIn(alerts_path("refs%2Fpull%2F12%2Fmerge") + "1", api.asked)

    def test_a_single_alert_is_named_in_the_singular(self):
        _, _, err, _, _ = run(pr_args(), complete_with([alert(77)], []))
        self.assertIn("adds 1 CodeQL alert in", err)
        self.assertNotIn("alerts in", err)

    def test_a_category_that_only_begins_like_the_jobs_is_another(self):
        code, _, err, _, _ = run(pr_args(category=RUST), complete_with([alert(95, category=RUST + "-other")], []))
        self.assertEqual(code, 0, err)

    def test_an_alert_main_has_in_another_category_is_matched_by_number(self):
        code, _, err, _, _ = run(pr_args(), complete_with([alert(96)], [alert(96, category=RUST)]))
        self.assertEqual(code, 0, err)

    def test_the_deadline_is_met_after_several_waits(self):
        code, _, err, _, clock = run(pr_args(pr="", wait="35"), {sarif_path(): status("pending")})
        self.assertEqual((code, clock.slept), (1, [5, 10, 20]))
        self.assertIn("not processed in 35 s", err)

    def test_new_alerts_is_pure(self):
        self.assertEqual([a["number"] for a in gate.new_alerts([alert(3), alert(1), alert(2, category=RUST)], [alert(3)], JS)], [1])


class Usage(unittest.TestCase):
    def test_no_repository_answers_2(self):
        code, _, err, _, _ = run(pr_args(), {}, env={})
        self.assertEqual(code, 2)
        self.assertIn("GITHUB_REPOSITORY", err)

    def test_an_unknown_command_answers_2(self):
        code, _, _, _, _ = run(["nope"], {})
        self.assertEqual(code, 2)

    def test_no_category_answers_2(self):
        code, _, _, _, _ = run(["check", "--sarif-id", SARIF], {})
        self.assertEqual(code, 2)

    def test_a_wait_that_is_not_a_number_answers_2(self):
        code, _, _, _, _ = run(pr_args(wait="soon"), {})
        self.assertEqual(code, 2)

    def test_a_negative_wait_answers_2(self):
        code, _, err, _, _ = run(pr_args(wait="-1"), {})
        self.assertEqual(code, 2)
        self.assertIn("--wait", err)

    def test_a_pull_request_that_is_not_a_number_answers_2(self):
        code, _, err, _, _ = run(pr_args(pr="seven"), {})
        self.assertEqual(code, 2)
        self.assertIn("--pr", err)

    def test_the_default_wait_is_fifteen_minutes(self):
        argv = ["check", "--sarif-id", SARIF, "--category", JS]
        _, _, _, _, clock = run(argv, {sarif_path(): status("pending")})
        self.assertEqual(clock.now, 900)


class GhApi(unittest.TestCase):
    def test_gh_api_parses_the_json_answer(self):
        def fake(cmd, **kw):
            self.assertEqual(cmd, ["gh", "api", "repos/x/y"])
            return subprocess.CompletedProcess(cmd, 0, json.dumps({"a": 1}), "")

        self.assertEqual(gate.gh_api("repos/x/y", run=fake), {"a": 1})

    def test_gh_api_failure_is_a_gate_error_with_its_message(self):
        fake = lambda cmd, **kw: subprocess.CompletedProcess(cmd, 1, "", "HTTP 403: Resource not accessible\n")  # noqa: E731
        with self.assertRaises(gate.GateError) as caught:
            gate.gh_api("repos/x/y", run=fake)
        self.assertIn("HTTP 403", str(caught.exception))

    def test_gh_api_answer_that_is_not_json_is_a_gate_error(self):
        fake = lambda cmd, **kw: subprocess.CompletedProcess(cmd, 0, "<html>", "")  # noqa: E731
        with self.assertRaises(gate.GateError):
            gate.gh_api("repos/x/y", run=fake)

    def test_gh_api_that_does_not_answer_in_time_is_a_gate_error(self):
        def fake(cmd, **kw):
            raise subprocess.TimeoutExpired(cmd, kw.get("timeout"))

        with self.assertRaises(gate.GateError) as caught:
            gate.gh_api("repos/x/y", run=fake)
        self.assertIn("did not answer", str(caught.exception))

    def test_gh_api_has_a_deadline(self):
        seen = {}

        def fake(cmd, **kw):
            seen.update(kw)
            return subprocess.CompletedProcess(cmd, 0, "[]", "")

        gate.gh_api("repos/x/y", run=fake)
        self.assertEqual(seen.get("timeout"), 60)

    LIMITED = "gh: API rate limit exceeded for installation. If you reach out to GitHub Support for help\n"

    def budget(self, remaining, reset):
        return json.dumps({"resources": {"core": {"limit": 5000, "remaining": remaining, "reset": reset}}})

    def test_a_spent_budget_is_waited_for_and_the_call_made_again(self):
        calls, slept = [], []

        def fake(cmd, **kw):
            calls.append(cmd)
            if cmd == ["gh", "api", "rate_limit"]:
                return subprocess.CompletedProcess(cmd, 0, self.budget(0, 1_000_100), "")
            if len(calls) == 1:
                return subprocess.CompletedProcess(cmd, 1, "", self.LIMITED)
            return subprocess.CompletedProcess(cmd, 0, json.dumps({"a": 2}), "")

        answer = gate.gh_api("repos/x/y", run=fake, clock=lambda: 1_000_000, sleep=slept.append, err=io.StringIO())
        self.assertEqual(answer, {"a": 2}, "the answer after the reset")
        self.assertEqual(slept, [105], "waits to the reset plus 5 s")

    def test_a_reset_further_off_than_the_gate_waits_is_a_gate_error(self):
        def fake(cmd, **kw):
            if cmd == ["gh", "api", "rate_limit"]:
                return subprocess.CompletedProcess(cmd, 0, self.budget(0, 1_003_000), "")
            return subprocess.CompletedProcess(cmd, 1, "", self.LIMITED)

        slept = []
        with self.assertRaises(gate.GateError) as caught:
            gate.gh_api("repos/x/y", run=fake, clock=lambda: 1_000_000, sleep=slept.append, err=io.StringIO())
        self.assertIn("rate limit exceeded", str(caught.exception), "the gh message is kept")
        self.assertEqual(slept, [], "no wait past the most the gate waits")

    def test_the_gate_waits_at_most_the_rate_limit_wait(self):
        self.assertEqual(gate.RATE_LIMIT_WAIT, 900, "15 min, inside the job's deadline")


class Workflow(unittest.TestCase):
    TEXT = (HERE.parent / ".github" / "workflows" / "codeql.yml").read_text(encoding="utf-8")

    def test_every_analyze_job_runs_the_gate_after_the_analysis(self):
        self.assertIn("id: analyze", self.TEXT)
        self.assertLess(self.TEXT.index("codeql-action/analyze@"), self.TEXT.index("python3 -I scripts/codeql_pr_gate.py check"))

    def test_the_gate_is_given_the_uploaded_sarif_and_the_category(self):
        self.assertIn("SARIF_ID: ${{ steps.analyze.outputs.sarif-id }}", self.TEXT)
        self.assertIn('CATEGORY: "/language:${{ matrix.language }}"', self.TEXT)
        self.assertIn('category: "/language:${{ matrix.language }}"', self.TEXT, "the same category as the upload")

    def test_the_gate_reads_its_inputs_from_the_environment_not_the_expression(self):
        run_line = next(l for l in self.TEXT.splitlines() if "scripts/codeql_pr_gate.py" in l and "python3" in l)
        self.assertNotIn("${{", run_line, "no expression is expanded into the shell")

    def test_the_gate_is_given_the_pull_request_and_its_base(self):
        self.assertIn("PR: ${{ github.event.pull_request.number }}", self.TEXT)
        self.assertIn("BASE: ${{ github.base_ref }}", self.TEXT)

    def test_the_job_may_read_code_scanning(self):
        self.assertIn("security-events: write", self.TEXT)

    def test_the_job_deadline_holds_the_analysis_and_the_wait(self):
        self.assertRegex(self.TEXT, r"timeout-minutes: 60\n", "60 min: the analysis, 15 min of waiting and 15 min for a spent API budget")
        self.assertRegex(self.TEXT, r"--wait 900(?![0-9])", "15 min")


if __name__ == "__main__":
    unittest.main()
