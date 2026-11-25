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
        self.deadlines = []

    def __call__(self, path, deadline=None):
        self.asked.append(path)
        self.deadlines.append(deadline)
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


EMPTY = "unexpected end of JSON input\n"
NOT_FOUND = "gh: Not Found (HTTP 404)\n"
FORBIDDEN = "gh: Resource not accessible by integration (HTTP 403)\n"


class Gh:
    """A fake `run` for gh_api: answers each call from `script` in turn, the
    last one again. Each item is (exit, stdout, stderr), or an exception to
    raise. It records every command it was given."""

    def __init__(self, *script):
        self.script = list(script)
        self.calls = []

    def __call__(self, cmd, **kw):
        self.calls.append(cmd)
        item = self.script.pop(0) if len(self.script) > 1 else self.script[0]
        if isinstance(item, BaseException):
            raise item
        code, out, err = item
        return subprocess.CompletedProcess(cmd, code, out, err)


def ok(answer):
    return (0, json.dumps(answer), "")


def timeout():
    return subprocess.TimeoutExpired(["gh"], 60)


def ask(gh, deadline, clock=None):
    """gh_api through `gh` on a clock at 0 with `deadline`: (answer or the
    GateError, clock, stderr)."""
    clock, err = clock or Clock(), io.StringIO()
    try:
        answer = gate.gh_api("repos/x/y", run=gh, clock=clock, sleep=clock.sleep, err=err, deadline=deadline)
    except gate.GateError as error:
        answer = error
    return answer, clock, err.getvalue()


def run_gh(argv, gh):
    """main with gh_api over the fake `gh`: (code, stdout, stderr, clock)."""
    clock, out, err = Clock(), io.StringIO(), io.StringIO()

    def api(path, deadline=None):
        return gate.gh_api(path, run=gh, clock=clock, sleep=clock.sleep, err=err, deadline=deadline)

    code = gate.main(argv, {"GITHUB_REPOSITORY": REPO}, api, clock, clock.sleep, out, err)
    return code, out.getvalue(), err.getvalue(), clock


class RetrySuccess(unittest.TestCase):
    """A transient read is asked again while the --wait budget lasts."""

    def test_an_empty_answer_is_asked_again(self):
        answer, clock, _ = ask(Gh((0, "", ""), ok({"a": 1})), deadline=900)
        self.assertEqual(answer, {"a": 1}, "the second answer")
        self.assertEqual(clock.slept, [5], "one wait of 5 s")

    def test_a_gh_failure_on_an_empty_body_is_asked_again(self):
        answer, _, _ = ask(Gh((1, "", EMPTY), ok([])), deadline=900)
        self.assertEqual(answer, [], "the #282 failure is retried")

    def test_a_read_that_timed_out_is_asked_again(self):
        answer, _, _ = ask(Gh(timeout(), ok({"b": 2})), deadline=900)
        self.assertEqual(answer, {"b": 2}, "the answer after the timeout")

    def test_a_502_is_asked_again(self):
        answer, _, _ = ask(Gh((1, "", "gh: Bad Gateway (HTTP 502)\n"), ok([1])), deadline=900)
        self.assertEqual(answer, [1], "a server error is transient")

    def test_a_500_is_asked_again(self):
        answer, _, _ = ask(Gh((1, "", "HTTP 500: Internal Server Error\n"), ok([2])), deadline=900)
        self.assertEqual(answer, [2], "the other form of gh's message")

    def test_an_answer_that_is_not_json_is_asked_again(self):
        answer, _, _ = ask(Gh((0, "<html>unicorn</html>", ""), ok({"c": 3})), deadline=900)
        self.assertEqual(answer, {"c": 3}, "an HTML error page is transient")

    def test_a_429_without_the_rate_limit_text_is_asked_again(self):
        answer, _, _ = ask(Gh((1, "", "gh: Too Many Requests (HTTP 429)\n"), ok([3])), deadline=900)
        self.assertEqual(answer, [3], "429 is no refusal")

    def test_the_waits_double_from_5_s(self):
        _, clock, _ = ask(Gh((0, "", ""), (0, "", ""), (0, "", ""), ok([])), deadline=900)
        self.assertEqual(clock.slept, [5, 10, 20], "5, 10, 20")

    def test_the_waits_stop_doubling_at_a_minute(self):
        script = [(0, "", "")] * 7 + [ok([])]
        _, clock, _ = ask(Gh(*script), deadline=900)
        self.assertEqual(clock.slept, [5, 10, 20, 40, 60, 60, 60], "at most 60 s each")

    def test_each_retry_is_said_with_its_error_and_its_wait(self):
        _, _, said = ask(Gh((1, "", EMPTY), ok([])), deadline=900)
        self.assertIn("unexpected end of JSON input", said, "the error")
        self.assertIn("5 s", said, "the wait")

    def test_the_gate_passes_after_an_empty_alert_listing(self):
        gh = Gh(ok(status("complete")), (1, "", EMPTY), ok([]), ok([]))
        code, out, err, _ = run_gh(pr_args(), gh)
        self.assertEqual(code, 0, err)
        self.assertIn("adds no CodeQL alert", out)

    def test_the_gate_passes_after_a_sarif_read_that_timed_out(self):
        gh = Gh(timeout(), ok(status("complete")))
        code, _, err, _ = run_gh(["check", "--sarif-id", SARIF, "--category", JS], gh)
        self.assertEqual(code, 0, err)

    def test_main_gives_every_read_the_wait_budget(self):
        code, _, _, api, _ = run(pr_args(wait="300"), complete_with([], []))
        self.assertEqual(code, 0)
        self.assertEqual(set(api.deadlines), {300}, "one deadline, --wait seconds from the start")

    def test_the_same_command_is_asked_each_time(self):
        gh = Gh((0, "", ""), ok([]))
        ask(gh, deadline=900)
        self.assertEqual(gh.calls, [["gh", "api", "repos/x/y"]] * 2, "the same read again")


class RetryFailure(unittest.TestCase):
    """A real refusal is not asked again, and a spent budget fails closed."""

    def test_a_404_is_not_asked_again(self):
        gh = Gh((1, "", NOT_FOUND), ok([]))
        answer, clock, _ = ask(gh, deadline=900)
        self.assertIsInstance(answer, gate.GateError)
        self.assertEqual((len(gh.calls), clock.slept), (1, []), "one ask, no wait")

    def test_a_403_is_not_asked_again(self):
        gh = Gh((1, "", FORBIDDEN), ok([]))
        answer, _, _ = ask(gh, deadline=900)
        self.assertIsInstance(answer, gate.GateError)
        self.assertEqual(len(gh.calls), 1, "one ask")

    def test_a_401_is_not_asked_again(self):
        gh = Gh((1, "", "gh: Bad credentials (HTTP 401)\n"), ok([]))
        ask(gh, deadline=900)
        self.assertEqual(len(gh.calls), 1, "one ask")

    def test_a_422_is_not_asked_again(self):
        gh = Gh((1, "", "HTTP 422: Validation Failed\n"), ok([]))
        ask(gh, deadline=900)
        self.assertEqual(len(gh.calls), 1, "one ask")

    def test_a_refusal_keeps_its_message(self):
        answer, _, _ = ask(Gh((1, "", NOT_FOUND)), deadline=900)
        self.assertIn("HTTP 404", str(answer), "gh's message")

    def test_a_read_still_failing_at_the_deadline_is_a_gate_error(self):
        answer, clock, _ = ask(Gh((1, "", EMPTY)), deadline=100)
        self.assertIsInstance(answer, gate.GateError)
        self.assertEqual(clock.now, 100, "it waited the whole budget")

    def test_the_last_error_is_named(self):
        answer, _, _ = ask(Gh(timeout(), (0, "<html>", "")), deadline=30)
        self.assertIn("no JSON", str(answer), "the last error, not the first")
        self.assertNotIn("did not answer", str(answer), "the first error is not the one named")

    def test_the_error_says_the_budget_is_spent(self):
        answer, _, _ = ask(Gh((1, "", EMPTY)), deadline=30)
        self.assertIn("--wait budget is spent", str(answer), "the budget")
        self.assertIn("asked 4 times", str(answer), "how often it asked: at 0, 5, 15 and 30 s")

    def test_the_gate_answers_2_when_the_alerts_cannot_be_read_in_time(self):
        gh = Gh(ok(status("complete")), (1, "", EMPTY))
        code, _, err, _ = run_gh(pr_args(wait="60"), gh)
        self.assertEqual(code, 2, "fails closed")
        self.assertIn("cannot read code scanning", err)

    def test_the_gate_answers_2_at_once_on_an_unknown_sarif(self):
        gh = Gh((1, "", NOT_FOUND))
        code, _, _, clock = run_gh(pr_args(), gh)
        self.assertEqual((code, clock.slept), (2, []), "a 404 for the SARIF is final")

    def test_no_deadline_asks_once(self):
        gh = Gh((1, "", EMPTY), ok([]))
        answer, _, _ = ask(gh, deadline=None)
        self.assertIsInstance(answer, gate.GateError)
        self.assertEqual(len(gh.calls), 1, "a caller with no budget gets the first error")

    def test_a_deadline_already_passed_asks_once(self):
        clock = Clock()
        clock.now = 50
        gh = Gh((1, "", EMPTY), ok([]))
        ask(gh, deadline=40, clock=clock)
        self.assertEqual((len(gh.calls), clock.slept), (1, []), "no time left")

    def test_a_refusal_is_not_transient_and_an_empty_body_is(self):
        refused, _, _ = ask(Gh((1, "", NOT_FOUND)), deadline=None)
        empty, _, _ = ask(Gh((1, "", EMPTY)), deadline=None)
        self.assertEqual((refused.transient, empty.transient), (False, True), "the error says which it is")


class RetryEdge(unittest.TestCase):
    def test_the_last_wait_is_cut_to_the_deadline(self):
        _, clock, _ = ask(Gh((0, "", "")), deadline=12)
        self.assertEqual(clock.slept, [5, 7], "5 s, then the 7 s left")

    def test_no_wait_passes_the_deadline(self):
        _, clock, _ = ask(Gh((0, "", "")), deadline=333)
        self.assertEqual(clock.now, 333, "it stops at the deadline exactly")

    def test_a_json_answer_is_not_asked_again(self):
        gh = Gh(ok([]), ok([1]))
        answer, clock, _ = ask(gh, deadline=900)
        self.assertEqual((answer, clock.slept), ([], []), "the first answer")

    def test_a_json_null_is_an_answer(self):
        answer, _, _ = ask(Gh((0, "null", ""), ok([])), deadline=900)
        self.assertIsNone(answer, "valid JSON is returned; the caller judges it")

    def test_a_body_of_blanks_is_asked_again(self):
        answer, _, _ = ask(Gh((0, " \n\t", ""), ok([4])), deadline=900)
        self.assertEqual(answer, [4], "only blanks is an empty body")

    def test_a_refusal_after_a_transient_failure_ends_the_asking(self):
        gh = Gh((0, "", ""), (1, "", NOT_FOUND), ok([]))
        answer, clock, _ = ask(gh, deadline=900)
        self.assertIsInstance(answer, gate.GateError)
        self.assertEqual((len(gh.calls), clock.slept), (2, [5]), "stops at the 404")

    def test_a_refusal_on_stdout_is_read(self):
        gh = Gh((1, '{"message":"Not Found","status":"404"} gh: Not Found (HTTP 404)', ""), ok([]))
        ask(gh, deadline=900)
        self.assertEqual(len(gh.calls), 1, "gh may print the error on stdout")

    def test_a_rate_limit_403_is_left_to_the_rate_limit_rule(self):
        limited = "gh: API rate limit exceeded for installation (HTTP 403)\n"
        budget = json.dumps({"resources": {"core": {"limit": 5000, "remaining": 0, "reset": 10**9}}})
        gh = Gh((1, "", limited), (0, budget, ""), (1, "", limited))
        answer, clock, _ = ask(gh, deadline=900)
        self.assertIn("rate limit", str(answer), "gh's message is kept")
        self.assertEqual(len(gh.calls), 2, "the read and the budget read; no transient retry of a spent budget")
        self.assertEqual(clock.slept, [], "a reset further off than the rule allows is not waited for")

    def test_a_wait_of_0_asks_each_read_once(self):
        gh = Gh((1, "", EMPTY), ok(status("complete")))
        code, _, _, clock = run_gh(pr_args(wait="0"), gh)
        self.assertEqual((code, clock.slept, len(gh.calls)), (2, [], 1), "no budget, no retry")

    def test_the_budget_is_shared_by_every_read(self):
        gh = Gh(ok(status("pending")), ok(status("pending")), ok(status("complete")), (1, "", EMPTY))
        code, _, _, clock = run_gh(pr_args(wait="20"), gh)
        self.assertEqual(code, 2)
        self.assertEqual(clock.now, 20, "the alert reads get only what the processing left")

    def test_a_status_line_without_a_code_is_transient(self):
        answer, _, _ = ask(Gh((1, "", "Post \"https://api.github.com\": EOF\n"), ok([5])), deadline=900)
        self.assertEqual(answer, [5], "a dropped connection is retried")

    def test_a_longer_number_after_http_4_is_no_refusal(self):
        gh = Gh((1, "", "HTTP 4040 something\n"), ok([6]))
        answer, _, _ = ask(gh, deadline=900)
        self.assertEqual(answer, [6], "HTTP 4040 is not a 4xx status")


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
