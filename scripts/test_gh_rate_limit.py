"""Tests scripts/gh_rate_limit.py: a GitHub API call that failed because the
token's hourly budget is spent waits for the budget's reset, when the reset
is near enough, and is then made once more; any other failure is left to the
caller.

Run with: python3 -I scripts/test_gh_rate_limit.py
"""

import importlib.util
import io
import json
import pathlib
import re
import subprocess
import sys
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("gh_rate_limit", HERE / "gh_rate_limit.py")
rl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rl)

NOW = 1_795_000_000
# The message gh printed on main at f02d554 (2026-11-19 01:59:16Z), shortened
# after the request id.
LIMITED = (
    "gh: API rate limit exceeded for installation. If you reach out to GitHub Support for help, "
    "please include the request ID A5A1:2FC5B2:EFD185:2F8732B:6AFE57F4 and timestamp 2026-11-19 01:59:16 UTC."
)


def budget_json(remaining, reset=NOW + 120, limit=5000):
    return json.dumps({"resources": {"core": {"limit": limit, "remaining": remaining, "reset": reset, "used": limit - remaining}}})


def done(cmd, rc=0, out="", err=""):
    return subprocess.CompletedProcess(cmd, rc, out, err)


class Gh:
    """A fake subprocess.run for gh: `gh api rate_limit` is answered from
    `budgets` in turn (the last one again), every other command from
    `answers` in turn (the last one again). Each command is recorded."""

    def __init__(self, answers, budgets=()):
        self.answers = list(answers)
        self.budgets = list(budgets)
        self.calls = []

    def __call__(self, cmd, **kw):
        self.calls.append(list(cmd))
        if list(cmd[:3]) == ["gh", "api", "rate_limit"]:
            if not self.budgets:
                return done(cmd, 1, err="gh: no budget answer in this test")
            item = self.budgets.pop(0) if len(self.budgets) > 1 else self.budgets[0]
        else:
            item = self.answers.pop(0) if len(self.answers) > 1 else self.answers[0]
        if isinstance(item, BaseException):
            raise item
        rc, out, err = item
        return done(cmd, rc, out, err)

    def made(self):
        return [c for c in self.calls if c[:3] != ["gh", "api", "rate_limit"]]


class Clock:
    def __init__(self, now=NOW):
        self.now = float(now)
        self.slept = []

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.slept.append(seconds)
        self.now += seconds


def call(gh, clock=None, max_wait=900, margin=5):
    clock = clock or Clock()
    err = io.StringIO()
    result = rl.run_gh(["gh", "api", "repos/o/r/rules/branches/main"], run=gh, clock=clock, sleep=clock.sleep, max_wait=max_wait, margin=margin, err=err)
    return result, clock, err.getvalue()


class Success(unittest.TestCase):
    def test_a_call_that_works_is_made_once_and_reads_no_budget(self):
        gh = Gh([(0, "[]", "")])
        result, clock, err = call(gh)
        self.assertEqual(result.returncode, 0, "the call worked")
        self.assertEqual(len(gh.calls), 1, "no budget read for a call that worked")
        self.assertEqual(clock.slept, [], "no wait")

    def test_a_spent_budget_is_waited_for_then_the_call_is_made_again(self):
        gh = Gh([(1, "", LIMITED), (0, "{}", "")], [(0, budget_json(0, NOW + 120), "")])
        result, clock, err = call(gh)
        self.assertEqual(result.returncode, 0, f"the second call worked: {err}")
        self.assertEqual(clock.slept, [125], "waits until the reset plus the margin")
        self.assertEqual(len(gh.made()), 2, "the call is made again once")

    def test_the_wait_is_announced_with_the_reset_time(self):
        gh = Gh([(1, "", LIMITED), (0, "{}", "")], [(0, budget_json(0, NOW + 120), "")])
        _, _, err = call(gh)
        self.assertIn("0 of 5000 requests left", err, err)
        self.assertIn("2026-11-18T", err, "the reset is printed as a UTC time")
        self.assertIn("waiting 125 s", err, err)

    def test_the_answer_of_the_second_call_is_returned(self):
        gh = Gh([(1, "", LIMITED), (0, "second", "")], [(0, budget_json(0), "")])
        result, _, _ = call(gh)
        self.assertEqual(result.stdout, "second", "the caller reads the second answer")

    def test_a_secondary_rate_limit_with_budget_left_is_waited_a_minute(self):
        gh = Gh([(1, "", "gh: You have exceeded a secondary rate limit. Please wait a few minutes."), (0, "{}", "")], [(0, budget_json(4000), "")])
        result, clock, err = call(gh)
        self.assertEqual(result.returncode, 0, err)
        self.assertEqual(clock.slept, [rl.SECONDARY_WAIT], "a secondary limit carries no reset; one fixed wait")

    def test_plan_left_when_budget_remains(self):
        self.assertEqual(rl.plan(rl.Budget(5000, 1, NOW + 10), NOW, 900, 5), ("left", 0), "one request left")

    def test_plan_wait_until_reset_plus_margin(self):
        self.assertEqual(rl.plan(rl.Budget(5000, 0, NOW + 300), NOW, 900, 5), ("wait", 305), "reset in 300 s")

    def test_plan_waits_at_exactly_the_most_allowed(self):
        self.assertEqual(rl.plan(rl.Budget(5000, 0, NOW + 895), NOW, 900, 5), ("wait", 900), "900 s is allowed")

    def test_budget_is_read_from_the_core_resource(self):
        self.assertEqual(rl.core_budget(json.loads(budget_json(7, 123, 1000))), rl.Budget(1000, 7, 123), "core values")

    def test_rate_limited_matches_the_installation_message(self):
        self.assertTrue(rl.rate_limited(LIMITED), "the message from main's run")

    def test_rate_limited_matches_a_user_token_message(self):
        self.assertTrue(rl.rate_limited("API rate limit exceeded for user ID 1."), "user tokens say 'for user ID'")

    def test_cli_wait_with_budget_left_exits_0_without_sleeping(self):
        gh = Gh([], [(0, budget_json(10), "")])
        clock = Clock()
        out = io.StringIO()
        rc = rl.main(["wait"], run=gh, clock=clock, sleep=clock.sleep, err=out)
        self.assertEqual(rc, 0, out.getvalue())
        self.assertEqual(clock.slept, [], "no wait with budget left")

    def test_cli_wait_on_a_spent_budget_sleeps_and_exits_3(self):
        gh = Gh([], [(0, budget_json(0, NOW + 60), "")])
        clock = Clock()
        out = io.StringIO()
        rc = rl.main(["wait", "--max", "900", "--margin", "0"], run=gh, clock=clock, sleep=clock.sleep, err=out)
        self.assertEqual(rc, 3, out.getvalue())
        self.assertEqual(clock.slept, [60], "slept to the reset")


class Failure(unittest.TestCase):
    def test_a_failure_that_is_not_the_rate_limit_is_returned_at_once(self):
        gh = Gh([(1, "", "gh: Not Found (HTTP 404)")], [(0, budget_json(0), "")])
        result, clock, _ = call(gh)
        self.assertEqual(result.returncode, 1, "the failure is the caller's")
        self.assertEqual(len(gh.calls), 1, "no budget read and no second call")
        self.assertEqual(clock.slept, [], "no wait")

    def test_a_reset_further_off_than_the_most_allowed_is_not_waited_for(self):
        gh = Gh([(1, "", LIMITED)], [(0, budget_json(0, NOW + 3000), "")])
        result, clock, err = call(gh)
        self.assertEqual(result.returncode, 1, "the rate-limited failure is returned")
        self.assertEqual(clock.slept, [], "no wait past --max")
        self.assertIn("more than the 900 s", err, err)

    def test_a_second_rate_limited_answer_is_returned_not_waited_again(self):
        gh = Gh([(1, "", LIMITED), (1, "", LIMITED)], [(0, budget_json(0, NOW + 10), "")])
        result, clock, _ = call(gh)
        self.assertEqual(result.returncode, 1, "still limited after one wait")
        self.assertEqual(len(gh.made()), 2, "made twice, not more")
        self.assertEqual(len(clock.slept), 1, "waited once")

    def test_an_unreadable_budget_returns_the_failure(self):
        gh = Gh([(1, "", LIMITED)], [(1, "", "gh: connection reset")])
        result, clock, err = call(gh)
        self.assertEqual(result.returncode, 1, "the failure is returned")
        self.assertEqual(clock.slept, [], "no wait without a reset time")
        self.assertIn("cannot read the rate limit", err, err)
        self.assertEqual(len(gh.made()), 1, "not made again")

    def test_a_budget_answer_that_is_not_json_is_unreadable(self):
        gh = Gh([(1, "", LIMITED)], [(0, "<html>", "")])
        result, clock, err = call(gh)
        self.assertEqual(clock.slept, [], "no wait")
        self.assertIn("cannot read the rate limit", err, err)

    def test_core_budget_refuses_an_answer_without_core(self):
        with self.assertRaises(ValueError, msg="no resources.core"):
            rl.core_budget({"resources": {}})

    def test_core_budget_refuses_a_remaining_that_is_not_a_number(self):
        with self.assertRaises(ValueError, msg="remaining is text"):
            rl.core_budget({"resources": {"core": {"limit": 5000, "remaining": "0", "reset": NOW}}})

    def test_plan_too_far(self):
        self.assertEqual(rl.plan(rl.Budget(5000, 0, NOW + 896), NOW, 900, 5), ("too-far", 901), "901 s is over")

    def test_rate_limited_does_not_match_other_errors(self):
        for text in ("gh: Not Found (HTTP 404)", "gh: Bad credentials (HTTP 401)", "rate", ""):
            self.assertFalse(rl.rate_limited(text), text)

    def test_cli_wait_on_a_far_reset_exits_1(self):
        gh = Gh([], [(0, budget_json(0, NOW + 5000), "")])
        clock = Clock()
        out = io.StringIO()
        rc = rl.main(["wait", "--max", "60"], run=gh, clock=clock, sleep=clock.sleep, err=out)
        self.assertEqual(rc, 1, out.getvalue())
        self.assertEqual(clock.slept, [], "no wait")

    def test_cli_wait_without_a_budget_answer_exits_2(self):
        gh = Gh([], [(1, "", "gh: offline")])
        clock = Clock()
        out = io.StringIO()
        self.assertEqual(rl.main(["wait"], run=gh, clock=clock, sleep=clock.sleep, err=out), 2, out.getvalue())

    def test_cli_refuses_a_negative_max(self):
        out = io.StringIO()
        clock = Clock()
        gh = Gh([], [(0, budget_json(10), "")])
        self.assertEqual(rl.main(["wait", "--max", "-1"], run=gh, clock=clock, sleep=clock.sleep, err=out), 2, out.getvalue())
        self.assertEqual(gh.calls, [], "refused before any budget read")

    def test_cli_refuses_an_unknown_command(self):
        out = io.StringIO()
        clock = Clock()
        self.assertEqual(rl.main(["sleep"], run=Gh([]), clock=clock, sleep=clock.sleep, err=out), 2, out.getvalue())


class Edge(unittest.TestCase):
    def test_a_reset_already_past_waits_only_the_margin(self):
        self.assertEqual(rl.plan(rl.Budget(5000, 0, NOW - 30), NOW, 900, 5), ("wait", 5), "the clock passed the reset")

    def test_a_zero_margin_and_a_reset_now_waits_nothing_but_still_retries(self):
        gh = Gh([(1, "", LIMITED), (0, "{}", "")], [(0, budget_json(0, NOW), "")])
        result, clock, err = call(gh, margin=0)
        self.assertEqual(result.returncode, 0, err)
        self.assertEqual(clock.slept, [0], "a zero wait")
        self.assertEqual(len(gh.made()), 2, "made again")

    def test_max_zero_never_waits(self):
        gh = Gh([(1, "", LIMITED)], [(0, budget_json(0, NOW - 60), "")])
        result, clock, _ = call(gh, max_wait=0, margin=0)
        self.assertEqual(clock.slept, [], "--max 0 turns the wait off")
        self.assertEqual(result.returncode, 1, "failure returned")
        self.assertEqual(len(gh.calls), 1, "no budget read and no second call, even with the reset past")

    def test_rate_limited_is_case_blind(self):
        self.assertTrue(rl.rate_limited("api RATE LIMIT exceeded"), "upper and lower case")

    def test_a_rate_limit_message_on_stdout_counts(self):
        gh = Gh([(1, '{"message":"API rate limit exceeded for installation."}', ""), (0, "{}", "")], [(0, budget_json(0, NOW + 1), "")])
        result, _, err = call(gh)
        self.assertEqual(result.returncode, 0, f"gh api prints the JSON body on stdout: {err}")

    def test_a_rate_limited_answer_with_budget_left_waits_the_fixed_minute(self):
        gh = Gh([(1, "", LIMITED), (0, "{}", "")], [(0, budget_json(3), "")])
        result, clock, err = call(gh)
        self.assertEqual(clock.slept, [rl.SECONDARY_WAIT], f"the budget refilled or another limit answered: {err}")

    def test_a_timeout_of_the_first_call_is_not_caught(self):
        gh = Gh([subprocess.TimeoutExpired("gh", 60)])
        with self.assertRaises(subprocess.TimeoutExpired, msg="the caller handles its own deadline"):
            call(gh)

    def test_budget_reset_as_float_is_accepted(self):
        self.assertEqual(rl.core_budget({"resources": {"core": {"limit": 60, "remaining": 0, "reset": 12.0}}}), rl.Budget(60, 0, 12), "a float reset is rounded")

    def test_booleans_are_not_numbers(self):
        with self.assertRaises(ValueError, msg="True is not a count"):
            rl.core_budget({"resources": {"core": {"limit": 60, "remaining": True, "reset": 1}}})

    def test_the_extra_run_arguments_reach_both_calls(self):
        seen = []

        def run(cmd, **kw):
            seen.append(kw)
            if cmd[:3] == ["gh", "api", "rate_limit"]:
                return done(cmd, 0, budget_json(0, NOW + 1))
            return done(cmd, 1 if len(seen) == 1 else 0, "", LIMITED if len(seen) == 1 else "")

        clock = Clock()
        rl.run_gh(["gh", "api", "x"], run=run, clock=clock, sleep=clock.sleep, max_wait=900, margin=0, err=io.StringIO(), timeout=61)
        made = [kw for kw in seen if kw.get("timeout") == 61]
        self.assertEqual(len(made), 2, f"the caller's timeout is passed to both calls: {seen}")

    def test_cli_default_max_is_900(self):
        gh = Gh([], [(0, budget_json(0, NOW + 895), "")])
        clock = Clock()
        out = io.StringIO()
        self.assertEqual(rl.main(["wait"], run=gh, clock=clock, sleep=clock.sleep, err=out), 3, out.getvalue())
        self.assertEqual(clock.slept, [900], "895 s plus the default 5 s margin")

    def test_cli_with_no_arguments_is_a_usage_error(self):
        out = io.StringIO()
        clock = Clock()
        self.assertEqual(rl.main([], run=Gh([]), clock=clock, sleep=clock.sleep, err=out), 2, out.getvalue())
        self.assertIn("usage", out.getvalue(), "names the usage")


ROOT = HERE.parent
WORKFLOWS = sorted((ROOT / ".github" / "workflows").glob("*.yml"))
# A gh command that reads GitHub's API.
GH_READ = re.compile(r"\bgh (api|run list|run view|attestation verify|release view|release download)\b")
# Reads left out of the rule, each with its reason.
EXEMPT = {
    ("release.yml", 'if gh attestation verify "$changed"'): "a changed archive must fail to verify; a retry asks again for the refusal it expects",
}
# A step that waits for a spent budget runs one of these.
WAITERS = ("scripts/retry.sh gh ", "scripts/codeql_pr_gate.py", "scripts/advertised_release.py", "scripts/docs_test.py run")
MIN_TIMEOUT = 20  # minutes: the 15 min wait (DEFAULT_MAX) and the job's own work


def code_lines(text):
    """(number, line) of each line that is not a YAML comment."""
    return [(n, line) for n, line in enumerate(text.splitlines(), 1) if not line.lstrip().startswith("#")]


def jobs(text):
    """{job id: its text} of a workflow, read from the two-space keys under `jobs:`."""
    found, name, body, inside = {}, None, [], False
    for line in text.splitlines():
        if line.startswith("jobs:"):
            inside = True
            continue
        if not inside:
            continue
        key = re.match(r"^  ([A-Za-z0-9_-]+):\s*$", line)
        if key:
            if name:
                found[name] = "\n".join(body)
            name, body = key.group(1), []
        elif name:
            body.append(line)
    if name:
        found[name] = "\n".join(body)
    return found


def unwrapped_reads(name, text):
    """`file:line` of each gh read in `text` that does not go through scripts/retry.sh."""
    bad = []
    for n, line in code_lines(text):
        if not GH_READ.search(line) or "scripts/retry.sh gh " in line:
            continue
        if any(f == name and part in line for f, part in EXEMPT):
            continue
        bad.append(f"{name}:{n}")
    return bad


def short_waiting_jobs(name, text):
    """`file:job` of each job with a waiting step and a deadline under MIN_TIMEOUT."""
    bad = []
    for job, body in jobs(text).items():
        if not any(w in body for w in WAITERS):
            continue
        deadline = re.search(r"^    timeout-minutes: (\d+)\s*$", body, re.M)
        if not deadline or int(deadline.group(1)) < MIN_TIMEOUT:
            bad.append(f"{name}:{job}")
    return bad


def step_of(text, uses):
    """The text of the step that `uses:` the action `uses`, up to the next step."""
    start = text.index(uses)
    begin = text.rfind("\n      - ", 0, start)
    end = text.find("\n      - ", start)
    return text[begin : end if end != -1 else len(text)]


class Workflows(unittest.TestCase):
    """The rule reaches every workflow, and the ClusterFuzzLite build step
    that spent the budget on 2026-11-19 looks up no artifact."""

    CFL = (ROOT / ".github" / "workflows" / "clusterfuzzlite.yml").read_text(encoding="utf-8")

    def test_every_gh_read_in_a_workflow_goes_through_retry(self):
        bad = [b for w in WORKFLOWS for b in unwrapped_reads(w.name, w.read_text(encoding="utf-8"))]
        self.assertEqual(bad, [], "each gh read waits out a spent budget through scripts/retry.sh")

    def test_the_rule_finds_the_reads_it_holds(self):
        found = sum(len(GH_READ.findall(line)) for w in WORKFLOWS for _, line in code_lines(w.read_text(encoding="utf-8")))
        self.assertGreaterEqual(found, 8, "a reading that lost the gh lines cannot pass")

    def test_an_unwrapped_read_is_found(self):
        text = "jobs:\n  a:\n    steps:\n      - run: gh api repos/o/r/rules/branches/main\n"
        self.assertEqual(unwrapped_reads("ci.yml", text), ["ci.yml:4"], "a bare gh api")

    def test_a_wrapped_read_and_a_comment_pass(self):
        text = "      # gh api in a comment\n      - run: scripts/retry.sh gh api repos/o/r\n"
        self.assertEqual(unwrapped_reads("ci.yml", text), [], "a comment and a wrapped call")

    def test_the_exemption_holds_only_in_its_file(self):
        text = '          if gh attestation verify "$changed" --repo r; then\n'
        self.assertEqual(unwrapped_reads("release.yml", text), [], "exempt in release.yml")
        self.assertEqual(unwrapped_reads("ci.yml", text), ["ci.yml:1"], "not elsewhere")

    def test_every_exempt_line_is_still_in_its_workflow(self):
        for (name, part), why in EXEMPT.items():
            self.assertIn(part, (ROOT / ".github" / "workflows" / name).read_text(encoding="utf-8"), f"stale exemption: {why}")

    def test_every_waiting_job_has_room_for_the_wait(self):
        bad = [b for w in WORKFLOWS for b in short_waiting_jobs(w.name, w.read_text(encoding="utf-8"))]
        self.assertEqual(bad, [], f"a job that may wait 15 min for the budget has timeout-minutes of {MIN_TIMEOUT} or more")

    def test_a_short_waiting_job_is_found(self):
        text = "jobs:\n  scan:\n    timeout-minutes: 10\n    steps:\n      - run: scripts/retry.sh gh api x\n  other:\n    timeout-minutes: 5\n"
        self.assertEqual(short_waiting_jobs("ci.yml", text), ["ci.yml:scan"], "only the job that waits")

    def test_a_waiting_job_with_no_deadline_is_found(self):
        text = "jobs:\n  decide:\n    steps:\n      - run: python3 -I scripts/advertised_release.py decide\n"
        self.assertEqual(short_waiting_jobs("pages.yml", text), ["pages.yml:decide"], "no timeout-minutes")

    def test_the_clusterfuzzlite_build_looks_up_no_artifact(self):
        step = step_of(self.CFL, "google/clusterfuzzlite/actions/build_fuzzers@")
        self.assertRegex(step, r"\n\s+keep-unaffected-fuzz-targets: true\n", "no coverage lookup through every artifact")
        self.assertNotIn("github-token", step, "the build step needs no API token")

    def test_the_clusterfuzzlite_build_uploads_no_build(self):
        step = step_of(self.CFL, "google/clusterfuzzlite/actions/build_fuzzers@")
        self.assertNotRegex(step, r"upload-build: true", "no build artifact")

    def test_the_hook_and_ci_run_these_tests(self):
        for path in (".githooks/pre-commit", ".github/workflows/ci.yml"):
            self.assertIn("python3 -I scripts/test_gh_rate_limit.py", (ROOT / path).read_text(encoding="utf-8"), path)


if __name__ == "__main__":
    unittest.main()
