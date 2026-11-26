#!/usr/bin/env python3
"""Fails an Analyze job of .github/workflows/codeql.yml that code scanning
did not judge: its SARIF not processed, or, on a pull request, an alert the
pull request adds.

    codeql_pr_gate.py check --sarif-id ID --category CATEGORY
                            [--pr N --base BRANCH] [--wait SECONDS]

On #224 (2026-11-16) the JavaScript job uploaded its SARIF and waited
about 2.5 minutes for code scanning to process it; the analyze action then
wrote "Timed out waiting for analysis to finish processing. Continuing."
and the job passed. Code scanning never held that analysis for the pull
request, so its `CodeQL` check concluded neutral ("1 configuration not
found"), which a required check accepts, and alert 77 opened on main after
the merge.

THE RULE: the job passes when code scanning reports the SARIF `complete`
within --wait seconds (asked again after 5, 10, 20, 40 and then every 60
seconds) and, when --pr names a pull request, no open CodeQL alert of
--category on the pull request's merge commit (refs/pull/N/merge) is
missing from the open alerts of --base. A `failed` status, an unknown
status, no SARIF id and a SARIF still pending at the deadline are refused.
An alert with no category is counted for every job. An empty --pr (a push
or a scheduled run) checks only the processing.

Every read of code scanning shares the same --wait budget. A read that
failed in a way that asking again may fix (no answer in time, a gh failure
that is no 4xx refusal, an empty or non-JSON body) is asked again after 5,
10, 20, 40 and then every 60 seconds until that budget is spent. A 4xx
refusal (scripts/gh_rate_limit.py's `refused`) and a spent API budget are
final at once.

Exit 0 when it passes, 1 when it refuses (each reason on stderr), 2 when
code scanning cannot be read or the arguments are wrong. GITHUB_REPOSITORY
names the repository; GH_TOKEN is read by gh.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import time
import urllib.parse

# No bytecode beside the scripts for the module loaded by path
# (scripts/test_no_bytecode.py).
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("gh_rate_limit", pathlib.Path(__file__).resolve().parent / "gh_rate_limit.py")
gh_rate_limit = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gh_rate_limit)

PAGE = 100
FIRST_WAIT, LONGEST_WAIT = 5, 60
DEFAULT_WAIT = 900
GH_DEADLINE = 60
RATE_LIMIT_WAIT = 900


class GateError(Exception):
    """Code scanning cannot be read; the message says why. `transient` is
    True for a failure that asking again may pass."""

    def __init__(self, message, transient=False):
        super().__init__(message)
        self.transient = transient


def _read(path, run, clock, sleep, err):
    """One `gh api <path>`: its JSON answer, or a GateError. A spent API
    budget is waited for (at most RATE_LIMIT_WAIT seconds) by
    scripts/gh_rate_limit.py's rule and is not transient after it; a real
    refusal (gh_rate_limit.refused, a 4xx) is not transient either."""
    try:
        done = gh_rate_limit.run_gh(
            ["gh", "api", path], run=run, clock=clock, sleep=sleep, max_wait=RATE_LIMIT_WAIT, err=err,
            capture_output=True, text=True, timeout=GH_DEADLINE,
        )
    except subprocess.TimeoutExpired as error:
        raise GateError(f"gh api {path} did not answer in {GH_DEADLINE} s", transient=True) from error
    if done.returncode != 0:
        said = f"{done.stderr or ''}\n{done.stdout or ''}"
        final = gh_rate_limit.refused(said) or gh_rate_limit.rate_limited(said)
        raise GateError((done.stderr or "").strip() or f"gh api {path} exited {done.returncode}", transient=not final)
    try:
        return json.loads(done.stdout)
    except json.JSONDecodeError as error:
        raise GateError(f"gh api {path} answered no JSON", transient=True) from error


def gh_api(path: str, run=subprocess.run, clock=time.time, sleep=time.sleep, err=sys.stderr, deadline=None):
    """The JSON answer of `gh api <path>`. A transient failure (no answer in
    GH_DEADLINE seconds, a gh failure that is no refusal, an empty or
    non-JSON body) is asked again after 5, 10, 20, 40 and then every 60
    seconds while clock() is before `deadline`, the --wait budget; on
    #282 (2026-11-25) one alert listing came back empty and the job failed.
    With no deadline, or once it is passed, the last error is raised."""
    asked, pause = 0, FIRST_WAIT
    while True:
        asked += 1
        try:
            return _read(path, run, clock, sleep, err)
        except GateError as error:
            left = None if deadline is None else deadline - clock()
            if not error.transient or left is None:
                raise
            if left <= 0:
                raise GateError(f"{error} (asked {asked} times; the --wait budget is spent)") from error
            seconds = min(pause, left)
            print(f"codeql gate: {error}; asking again in {seconds:g} s", file=err)
            sleep(seconds)
            pause = min(pause * 2, LONGEST_WAIT)


def processed(repo, sarif_id, api, clock, sleep, wait) -> str | None:
    """None when code scanning processed the SARIF, else the reason it did not."""
    if not sarif_id:
        return "the analysis uploaded no SARIF (the analyze step gave no sarif-id)"
    path = f"repos/{repo}/code-scanning/sarifs/{urllib.parse.quote(sarif_id, safe='')}"
    deadline = clock() + wait
    pause = FIRST_WAIT
    while True:
        answer = api(path)
        value = answer.get("processing_status") if isinstance(answer, dict) else None
        if value == "complete":
            return None
        if value == "failed":
            errors = "; ".join(str(e) for e in answer.get("errors") or []) or "no errors given"
            return f"code scanning failed to process SARIF {sarif_id}: {errors}"
        if value != "pending":
            return f"code scanning gave SARIF {sarif_id} an unknown processing status {value!r}"
        left = deadline - clock()
        if left <= 0:
            return f"SARIF {sarif_id} was not processed in {wait} s; code scanning holds no result for this job"
        sleep(min(pause, left))
        pause = min(pause * 2, LONGEST_WAIT)


def open_alerts(repo, ref, api) -> list[dict]:
    """Every open CodeQL alert on `ref`, page by page."""
    found = []
    quoted = urllib.parse.quote(ref, safe="")
    page = 1
    while True:
        answer = api(f"repos/{repo}/code-scanning/alerts?ref={quoted}&state=open&tool_name=CodeQL&per_page={PAGE}&page={page}")
        if not isinstance(answer, list) or not all(isinstance(a, dict) for a in answer):
            raise GateError(f"code scanning answered for {ref} with something that is not a list of alerts")
        found += answer
        if len(answer) < PAGE:
            return found
        page += 1


def new_alerts(pr_alerts, base_alerts, category) -> list[dict]:
    """The alerts of `pr_alerts` in `category` (or with none) that
    `base_alerts` does not hold, by number."""
    base = {a.get("number") for a in base_alerts}
    out = []
    for a in pr_alerts:
        mine = (a.get("most_recent_instance") or {}).get("category")
        if mine not in (None, category) or a.get("number") in base:
            continue
        out.append(a)
    return sorted(out, key=lambda a: a.get("number") or 0)


def describe(a) -> str:
    instance = a.get("most_recent_instance") or {}
    location = instance.get("location") or {}
    place = f"{location['path']}:{location.get('start_line', '?')}" if location.get("path") else "(no location)"
    rule = a.get("rule") or {}
    severity = f" ({rule['severity']})" if rule.get("severity") else ""
    return f"#{a.get('number')} {rule.get('id', '?')}{severity} {place} {a.get('html_url', '')}".rstrip()


def main(argv, env, api, clock, sleep, out, err) -> int:
    parser = argparse.ArgumentParser(prog="codeql_pr_gate.py", description="Fails an Analyze job code scanning did not judge.")
    sub = parser.add_subparsers(dest="command")
    check = sub.add_parser("check")
    check.add_argument("--sarif-id", required=True)
    check.add_argument("--category", required=True)
    check.add_argument("--pr", default="")
    check.add_argument("--base", default="main")
    check.add_argument("--wait", type=int, default=DEFAULT_WAIT)
    try:
        args = parser.parse_args(argv)
    except SystemExit:
        return 2
    if args.command != "check":
        print("codeql gate: usage: codeql_pr_gate.py check --sarif-id ID --category CATEGORY [--pr N --base BRANCH] [--wait SECONDS]", file=err)
        return 2
    if args.wait < 0:
        print("codeql gate: --wait is a number of seconds, 0 or more", file=err)
        return 2
    if args.pr and not args.pr.isdigit():
        print(f"codeql gate: --pr is a pull request number, not {args.pr!r}", file=err)
        return 2
    repo = env.get("GITHUB_REPOSITORY")
    if not repo:
        print("codeql gate: GITHUB_REPOSITORY names no repository", file=err)
        return 2
    # One budget for every read: the processing wait and each retry of a
    # transient read end at the same deadline.
    deadline = clock() + args.wait

    def read(path):
        return api(path, deadline=deadline)

    try:
        reason = processed(repo, args.sarif_id, read, clock, sleep, args.wait)
        if reason:
            print(f"codeql gate: refused: {reason}", file=err)
            return 1
        print(f"codeql gate: code scanning processed SARIF {args.sarif_id}", file=out)
        if not args.pr:
            return 0
        added = new_alerts(
            open_alerts(repo, f"refs/pull/{args.pr}/merge", read), open_alerts(repo, f"refs/heads/{args.base}", read), args.category
        )
    except GateError as error:
        print(f"codeql gate: cannot read code scanning: {error}", file=err)
        return 2
    if not added:
        print(f"codeql gate: pull request {args.pr} adds no CodeQL alert in {args.category}", file=out)
        return 0
    noun = "alert" if len(added) == 1 else "alerts"
    print(f"codeql gate: refused: pull request {args.pr} adds {len(added)} CodeQL {noun} in {args.category} that {args.base} does not have:", file=err)
    for a in added:
        print(f"  {describe(a)}", file=err)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:], os.environ, gh_api, time.monotonic, time.sleep, sys.stdout, sys.stderr))
