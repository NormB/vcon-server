#!/usr/bin/env python3
"""Waits out a spent GitHub API budget instead of failing on it.

    gh_rate_limit.py wait [--max SECONDS] [--margin SECONDS]

A workflow's token has one hourly budget for the whole repository, shared
by every job of every run. On 2026-11-19 (main at f02d554) a ClusterFuzzLite
job of the pull request before it spent that budget, and CodeQL,
OSV-Scanner, Scorecard and CI's required-checks step on main then failed on
"API rate limit exceeded for installation" for a cause outside the change.
CI's step gave up after five tries over about two minutes; the budget came
back at 02:03:26Z, five minutes later.

THE RULE, in one place for the shell (scripts/retry.sh calls `wait` after a
failed `gh`) and for Python (run_gh): a call that failed with a rate-limit
message reads the budget (`gh api rate_limit`, which costs none of it). With
no request left, it waits until the reset plus --margin seconds, if that is
at most --max seconds away, and the call is made once more. With requests
left (a secondary limit, or a reset that came while the call failed) it
waits SECONDARY_WAIT seconds and makes the call once more. Any other
failure, a reset further off than --max, or a budget it cannot read is the
caller's failure, unchanged.

`wait` exits 0 when requests are left, 3 after it waited for the reset, 1
when the reset is further off than --max, and 2 when the budget cannot be
read or the arguments are wrong. Each line it prints goes to stderr.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from typing import NamedTuple

DEFAULT_MAX = 900
DEFAULT_MARGIN = 5
SECONDARY_WAIT = 60
GH_DEADLINE = 60
_LIMITED = re.compile(r"api rate limit exceeded|secondary rate limit", re.IGNORECASE)
_STATUS = re.compile(r"\bHTTP (4\d\d)\b")


class Budget(NamedTuple):
    limit: int
    remaining: int
    reset: int


def rate_limited(text: str) -> bool:
    """Whether gh's output names a spent rate limit."""
    return bool(_LIMITED.search(text or ""))


def refused(text: str) -> bool:
    """Whether gh's output is a real refusal that asking again cannot
    change: an HTTP 4xx status (`(HTTP 404)` or `HTTP 404:`), other than 429
    and a spent rate limit, which `run_gh` waits for. Every other failure (a
    5xx, a dropped connection, an empty or cut body) may pass when asked
    again; the caller decides whether it asks."""
    text = text or ""
    status = _STATUS.search(text)
    return bool(status) and status.group(1) != "429" and not rate_limited(text)


def _count(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"the rate limit's {name} is not a number: {value!r}")
    return int(value)


def core_budget(answer) -> Budget:
    """The REST budget (resources.core) of a `GET /rate_limit` answer."""
    core = (answer.get("resources") or {}).get("core") if isinstance(answer, dict) else None
    if not isinstance(core, dict):
        raise ValueError("the rate limit answer has no resources.core")
    return Budget(_count(core.get("limit"), "limit"), _count(core.get("remaining"), "remaining"), _count(core.get("reset"), "reset"))


def plan(budget: Budget, now: float, max_wait: int, margin: int) -> tuple[str, int]:
    """("left", 0) with requests left; else ("wait", s) or ("too-far", s),
    s being the seconds to the reset plus the margin."""
    if budget.remaining > 0:
        return "left", 0
    seconds = max(0, int(budget.reset - now)) + margin
    return ("wait" if seconds <= max_wait else "too-far"), seconds


def _when(epoch: int) -> str:
    return datetime.fromtimestamp(epoch, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def read_budget(run) -> Budget:
    """The budget, read with `gh api rate_limit`; ValueError if it cannot be."""
    try:
        done = run(["gh", "api", "rate_limit"], capture_output=True, text=True, timeout=GH_DEADLINE)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(str(error)) from error
    if done.returncode != 0:
        raise ValueError((done.stderr or "").strip() or f"gh exited {done.returncode}")
    try:
        return core_budget(json.loads(done.stdout))
    except json.JSONDecodeError as error:
        raise ValueError("the answer is not JSON") from error


def wait_for_reset(run, clock, sleep, max_wait: int, margin: int, err) -> int:
    """Reads the budget and waits for its reset when none is left. 0: left;
    3: waited; 1: too far off; 2: unreadable."""
    try:
        budget = read_budget(run)
    except ValueError as error:
        print(f"gh-rate-limit: cannot read the rate limit: {error}", file=err)
        return 2
    verdict, seconds = plan(budget, clock(), max_wait, margin)
    line = f"gh-rate-limit: {budget.remaining} of {budget.limit} requests left; the budget resets at {_when(budget.reset)}"
    if verdict == "left":
        print(line, file=err)
        return 0
    if verdict == "too-far":
        print(f"{line}, {seconds} s from now, more than the {max_wait} s this call may wait", file=err)
        return 1
    print(f"{line}; waiting {seconds} s", file=err)
    sleep(seconds)
    return 3


def run_gh(cmd, *, run=subprocess.run, clock=time.time, sleep=time.sleep, max_wait=DEFAULT_MAX, margin=DEFAULT_MARGIN, err=sys.stderr, **kw):
    """Runs a gh command (run(cmd, **kw)); a rate-limited failure is made
    once more after the wait the rule gives. Returns the last answer."""
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    done = run(cmd, **kw)
    if done.returncode == 0 or not rate_limited(f"{done.stderr or ''}\n{done.stdout or ''}") or max_wait <= 0:
        return done
    waited = wait_for_reset(run, clock, sleep, max_wait, margin, err)
    if waited == 0:
        seconds = min(SECONDARY_WAIT, max_wait)
        print(f"gh-rate-limit: rate-limited with requests left; waiting {seconds} s", file=err)
        sleep(seconds)
    elif waited != 3:
        return done
    return run(cmd, **kw)


def main(argv, run=subprocess.run, clock=time.time, sleep=time.sleep, err=sys.stderr) -> int:
    parser = argparse.ArgumentParser(prog="gh_rate_limit.py", add_help=False)
    parser.add_argument("command", nargs="?")
    parser.add_argument("--max", type=int, default=DEFAULT_MAX)
    parser.add_argument("--margin", type=int, default=DEFAULT_MARGIN)
    try:
        args = parser.parse_args(argv)
    except SystemExit:
        args = None
    if args is None or args.command != "wait" or args.max < 0 or args.margin < 0:
        print("gh-rate-limit: usage: gh_rate_limit.py wait [--max SECONDS] [--margin SECONDS] (both 0 or more)", file=err)
        return 2
    return wait_for_reset(run, clock, sleep, args.max, args.margin, err)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
