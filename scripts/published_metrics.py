#!/usr/bin/env python3
"""Derive the home page's numbers from CI's own runs of one commit.

The one rule for each published number:

- automated tests: CI's Tests step runs the tests with cargo-nextest
  (scripts/nextest.sh) and the doc tests with `cargo test --doc`, which
  nextest does not run. Every nextest `Summary` line (one per run: the
  workspace, the fuzz package) adds its `passed` figure, and every line
  that starts with `test result` (one per doc-test binary) adds its
  `passed` figure. A summary that reports anything but passed and skipped
  tests (a failure, a test ended at its deadline, a run stopped early), a
  doc-test binary that reports anything but `ok.`, an output with no
  nextest summary and a total of zero are refused, so the number always
  describes a complete passing run.
- line coverage: the totals of `cargo llvm-cov report --json
  --summary-only` from CI's coverage job: lines covered, lines counted, and
  the percent to one decimal, computed from those two integers.

ci.yml runs this on its own output, so a run it cannot count fails CI;
pages.yml runs it on the artifacts of CI's run of the commit it deploys and
writes the data file website/templates/index.html reads. Nothing commits a
number.

Usage:
    published_metrics.py [--tests OUTPUT] [--coverage JSON]
                         [--commit SHA --run URL --write PATH]

Checks and prints the numbers of the runs given (at least one). With --write, writes them as TOML, with the commit and
the CI run they came from. Exits 1, writing nothing, when a run is refused.
"""

import argparse
import json
import pathlib
import re
import sys

ANSI = re.compile(r"\x1b\[[0-9;]*m")
FIELDS = re.compile(r"[ ;]+")
SHA = re.compile(r"[0-9a-f]{40}")
# nextest's last line, its status word right-aligned in 12 columns:
# "     Summary [   4.006s] 136 tests run: 136 passed (1 slow), 5 skipped".
SUMMARY_START = "     Summary ["
SUMMARY = re.compile(r"^     Summary \[[^\]]*\] (\S+) tests? run: (.*)$")
SUMMARY_COUNT = re.compile(r"^(\d+) ([a-z ]+?)(?: \(\d+ [a-z]+\))?$")


class Refused(Exception):
    """The input does not describe a complete, passing run."""


def summary_passed(line: str) -> int:
    """The passed figure of one nextest `Summary` line, refused unless every
    test that ran passed."""
    m = SUMMARY.match(line)
    ran = m.group(1) if m else ""
    if not ran.isdigit():
        raise Refused(f"unreadable summary line: {line!r}")
    counts = {}
    for part in m.group(2).split(", "):
        c = SUMMARY_COUNT.match(part.strip())
        if not c:
            raise Refused(f"unreadable summary line: {line!r}")
        counts[c.group(2)] = int(c.group(1))
    others = {k: v for k, v in counts.items() if k not in ("passed", "skipped") and v}
    if others:
        what = ", ".join(f"{v} {k}" for k, v in others.items())
        raise Refused(f"the run reported {what}: {line!r}")
    if counts.get("passed") != int(ran):
        raise Refused(f"unreadable summary line: {line!r}")
    return int(ran)


def count_tests(text: str) -> int:
    """Sum the passed figure of every nextest `Summary` line and every doc
    tests' `test result` line in `text`."""
    total = 0
    summaries = 0
    for raw in text.splitlines():
        line = ANSI.sub("", raw)
        if line.startswith(SUMMARY_START):
            total += summary_passed(line)
            summaries += 1
            continue
        if not line.startswith("test result"):
            continue
        fields = FIELDS.split(line)
        if len(fields) < 4 or not fields[3].isdigit() or fields[4:5] != ["passed"]:
            raise Refused(f"unreadable result line: {line!r}")
        if fields[2] != "ok.":
            raise Refused(f"a doc-test binary reported {fields[2]!r}: {line!r}")
        total += int(fields[3])
    if summaries == 0:
        raise Refused("no nextest `Summary` line, so this is not a completed test run")
    if total == 0:
        raise Refused(f"{summaries} summary line(s) report 0 tests passed in total")
    return total


def line_coverage(summary) -> tuple:
    """(covered, counted, percent to one decimal) from an llvm-cov summary."""
    if summary.get("type") != "llvm.coverage.json.export":
        raise Refused("not an llvm.coverage.json.export summary")
    data = summary.get("data")
    if not isinstance(data, list) or len(data) != 1:
        raise Refused("the summary does not hold exactly one set of totals")
    lines = data[0].get("totals", {}).get("lines", {})
    counted, covered = lines.get("count"), lines.get("covered")
    if not isinstance(counted, int) or not isinstance(covered, int):
        raise Refused("the totals hold no line counts")
    if counted == 0:
        raise Refused("no lines were counted")
    if covered > counted:
        raise Refused("more lines covered than counted")
    # From the integers, not llvm's float, so one rule rounds every run.
    tenths = (covered * 1000 * 2 + counted) // (counted * 2)
    return covered, counted, f"{tenths // 10}.{tenths % 10}"


def write(path: pathlib.Path, tests: int, coverage, commit: str, run: str) -> None:
    """Writes the data file the home page reads."""
    if not SHA.fullmatch(commit):
        raise Refused(f"the commit is not a full sha: {commit!r}")
    covered, counted, percent = coverage
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f"automated_tests = {tests}\n"
        f"lines_covered = {covered}\n"
        f"lines_counted = {counted}\n"
        f'line_coverage = "{percent}"\n'
        f'commit = "{commit}"\n'
        f'run = "{run}"\n'
    )


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--tests", help="captured output of the Tests step (nextest and doc tests)")
    ap.add_argument("--coverage", help="`cargo llvm-cov report --json --summary-only` output")
    ap.add_argument("--commit", help="the commit both runs tested")
    ap.add_argument("--run", help="the URL of the CI run")
    ap.add_argument("--write", metavar="PATH", help="write the data file here")
    args = ap.parse_args(argv)
    try:
        if args.tests is None and args.coverage is None:
            raise Refused("give --tests, --coverage or both")
        if args.write and (args.tests is None or args.coverage is None):
            raise Refused("--write needs both --tests and --coverage")
        if args.tests is not None:
            tests = count_tests(pathlib.Path(args.tests).read_text(errors="replace"))
            print(f"automated tests: {tests}")
        if args.coverage is not None:
            try:
                summary = json.loads(pathlib.Path(args.coverage).read_text())
            except json.JSONDecodeError as e:
                raise Refused(f"the coverage summary is not JSON: {e}")
            coverage = line_coverage(summary)
            print(f"lines covered: {coverage[0]} of {coverage[1]} ({coverage[2]}%)")
        if args.write:
            if not args.commit or not args.run:
                raise Refused("--write needs --commit and --run")
            write(pathlib.Path(args.write), tests, coverage, args.commit, args.run)
    except Refused as e:
        print(f"published_metrics: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
