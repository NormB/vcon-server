#!/usr/bin/env python3
"""The performance regression gate: counts that do not depend on the CPU's
speed or the host's load, per operation, held to scripts/perf-baseline.json.

CI's "perf-gate" job (ci.yml) builds the server in release mode and runs
it under valgrind through scripts/scale-run.sh (SCALE_SERVE_WRAP names
scripts/perf-serve.sh, which runs `perf_gate.py serve`). Each workload is
a fresh server, one vcon-scale client (--concurrency 1) with fixed inputs:

  base    COUNT stores
  create  2 x COUNT stores
  read    COUNT stores, then READS reads
  list    COUNT stores, then PAGES list pages of 100 (from the newest
          page again after the last, so PAGES may pass what COUNT fills)

Each runs twice: under cachegrind (instructions, as Ir, and with
--trace-syscalls the syscalls) and under DHAT (heap blocks allocated). A
metric per operation is the workload's total less that of the workload
it adds to (REFERENCE: the base for each), over
the operations it added, so start-up, the tenant's open and the shutdown
cancel out. The metrics, for each of create, read and list:

  instructions  every thread of the server, user space (cachegrind Ir),
                aws-lc's jitter entropy source left out (is_jitter)
  syscalls      every call valgrind traced but the waits and the clock
                (TIMING_CALLS)
  fsyncs        fsync, fdatasync, syncfs and sync_file_range, in a band
                that never holds one more per operation
  allocations   heap blocks allocated (DHAT's Total)

Wall time is not held here: it depends on the runner's CPU and on the
other tenants of its host. CI's "scale" job reports it.

  measure --server S --scale B --out FILE [--count N --reads R --pages P]
            runs the eight servers and writes the measured file.
  compare BASELINE MEASURED [--other-arch-report-only]
            exit 0 when every metric is inside its tolerance; exit 1
            naming each metric that is worse, better past its tolerance
            (the baseline then ratchets down, in a reviewed commit),
            missing or not held, or measured with other inputs or on
            another arch; stdout then has the baseline that would hold the
            measured numbers. Exit 2 when a file cannot be read.
  update BASELINE MEASURED
            writes the baseline that holds MEASURED, keeping each
            tolerance, to BASELINE. Only a reviewed commit changes the
            baseline: CI never writes it.
  serve -- SERVER ARGS...
            the wrapper: execs valgrind with the tool PERF_GATE_TOOL
            (cachegrind or dhat) writing into PERF_GATE_OUT.

The tolerances were set from measured noise; scripts/perf-baseline.json's
commit says how. Run with: python3 -I scripts/perf_gate.py <command>
(from the repository root).
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import pathlib
import platform
import re
import subprocess
import sys
import tempfile

OPERATIONS = ("create", "read", "list")
METRICS = ("instructions", "syscalls", "fsyncs", "allocations")
TOOLS = ("cachegrind", "dhat")
# The band of a metric the baseline does not hold yet.
DEFAULT_TOLERANCE = 0.02
# The band of an fsync count the baseline does not hold yet. A count is
# not a whole number (on 2026-11-27, 4.036 a create, and 1.014 and 1.012 a
# read in two runs). compare refuses a band that would hold one more sync
# per operation.
FSYNC_TOLERANCE = 0.01
# Per-operation numbers are rounded to three places; two equal counts
# differ by less than half the last place.
EPSILON = 0.0005
# Every call that waits for the disk.
SYNC_CALLS = ("fsync", "fdatasync", "syncfs", "sync_file_range")
# aws-lc's jitter entropy source (crypto/fipsmodule/rand/jitter): its
# static helpers carry plain names.
JITTER_HELPERS = frozenset(
    {"rol64", "xoshiro128starstar", "uint32rotl", "ptr_to_le32", "ptr_to_le64", "le32_to_ptr", "le64_to_ptr"}
)
# Calls whose count follows the run's time: waits, wake-ups and the clock
# (a syscall under valgrind on aarch64, where the vDSO is not used).
TIMING_CALLS = frozenset(
    {"clock_gettime", "clock_nanosleep", "nanosleep", "gettimeofday", "futex", "epoll_wait",
     "epoll_pwait", "epoll_pwait2", "sched_yield"}
)

_SYSCALL = re.compile(r"^SYSCALL\[\d+,\d+\]\(\d+\) sys_([a-z0-9_]+) ")
_DHAT_TOTAL = re.compile(r"^==\d+== Total:\s+[\d,]+ bytes in ([\d,]+) blocks?\s*$")


class GateError(Exception):
    """The gate cannot judge: the message says why."""


def metric_names() -> list[str]:
    """Every metric the gate holds, as `<operation>.<metric>`."""
    return [f"{op}.{m}" for op in OPERATIONS for m in METRICS]


def parse_cachegrind(text: str) -> int:
    """The Ir total of a cachegrind out file run with --cache-sim=no."""
    events = [line for line in text.splitlines() if line.startswith("events:")]
    if events and events[0].split()[1:] != ["Ir"]:
        raise GateError(f"cachegrind counted {events[0]!r}, not Ir alone: run it with --cache-sim=no")
    summaries = [line for line in text.splitlines() if line.startswith("summary:")]
    if len(summaries) != 1:
        raise GateError(f"a cachegrind file has one summary line, this one {len(summaries)}")
    value = summaries[0].split(":", 1)[1].strip()
    if not value.isdigit():
        raise GateError(f"the cachegrind summary {value!r} is not a count")
    return int(value)


def is_jitter(name: str) -> bool:
    """Whether a function is aws-lc's CPU jitter entropy source, whose
    rounds follow the clock: its instructions differ from run to run."""
    return name.startswith("jent_") or "_jent_" in name or name in JITTER_HELPERS


def function_instructions(text: str) -> dict[str, int]:
    """The Ir of each function of a cachegrind out file (one event), by
    name; a name in several files is summed. The costs must add up to
    the summary."""
    out: dict[str, int] = {}
    current = None
    for line in text.splitlines():
        if line.startswith("fn="):
            current = line[3:]
            out.setdefault(current, 0)
        elif line[:1].isdigit():
            parts = line.split()
            if len(parts) != 2 or not all(p.isdigit() for p in parts):
                raise GateError(f"{line!r} is not a cost line of one event")
            if current is None:
                raise GateError(f"the cost line {line!r} comes before any function")
            out[current] += int(parts[1])
    if sum(out.values()) != parse_cachegrind(text):
        raise GateError("the functions' costs do not add up to the summary")
    return out


def steady_instructions(text: str) -> int:
    """The instructions of every function but the jitter source's."""
    return sum(n for name, n in function_instructions(text).items() if not is_jitter(name))


def steady_syscalls(counts: dict[str, int]) -> int:
    """The calls but those that wait or read the clock: how many there are
    follows the time a run takes, not its work."""
    return sum(n for name, n in counts.items() if name not in TIMING_CALLS)


def parse_syscalls(text: str) -> dict[str, int]:
    """The calls of a valgrind --trace-syscalls log, by name. A call that
    completes later ([async]) has a second line without its name, so each
    call is counted once."""
    counts: dict[str, int] = {}
    for line in text.splitlines():
        match = _SYSCALL.match(line)
        if match:
            counts[match.group(1)] = counts.get(match.group(1), 0) + 1
    if not counts:
        raise GateError("the valgrind log names no syscall: was it run with --trace-syscalls=yes?")
    return counts


def fsyncs(counts: dict[str, int]) -> int:
    """The calls that wait for the disk."""
    return sum(counts.get(name, 0) for name in SYNC_CALLS)


def parse_dhat_blocks(text: str) -> int:
    """The heap blocks DHAT counted: its Total line."""
    found = [m.group(1) for m in map(_DHAT_TOTAL.match, text.splitlines()) if m]
    if len(found) != 1:
        raise GateError(f"a DHAT log has one Total line, this one {len(found)}")
    return int(found[0].replace(",", ""))


def per_operation(base: int, more: int, operations: int) -> float:
    """What each operation added: the difference over the operations."""
    if operations <= 0:
        raise GateError(f"{operations} operations: nothing to divide by")
    if more < base:
        raise GateError(f"a count fell from {base} to {more} when work was added: the measurement is broken")
    return round((more - base) / operations, 3)


def derive(totals: dict[str, dict[str, int]], operations: dict[str, int]) -> dict[str, float]:
    """Each metric per operation, from each workload's totals less those
    of the workload it adds to (REFERENCE)."""
    out = {}
    for op in OPERATIONS:
        ref = REFERENCE[op]
        for name in (ref, op):
            if name not in totals:
                raise GateError(f"the {name} workload was not run")
        for metric in METRICS:
            for name in (ref, op):
                if metric not in totals[name]:
                    raise GateError(f"the {name} workload has no {metric}")
            out[f"{op}.{metric}"] = per_operation(totals[ref][metric], totals[op][metric], operations[op])
    return out


def _number(value, what: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise GateError(f"{what} is {value!r}, not a number")
    return float(value)


def compare(baseline: dict, measured: dict, other_arch_report_only: bool = False):
    """(failures, notes): each failure fails the gate; the notes are the
    report."""
    failures: list[str] = []
    notes: list[str] = []
    if measured.get("arch") != baseline.get("arch"):
        why = (
            f"measured on {measured.get('arch')}, the baseline is of {baseline.get('arch')}: "
            "instruction counts of one arch hold nothing on another"
        )
        if other_arch_report_only:
            notes.append(f"report only: {why}")
        else:
            failures.append(why)
    if baseline.get("valgrind") and baseline.get("valgrind") != measured.get("valgrind"):
        notes.append(
            f"the baseline was measured with {baseline.get('valgrind')}, this run with {measured.get('valgrind')}"
        )
    for key in sorted(set(baseline.get("inputs", {})) | set(measured.get("inputs", {}))):
        if baseline.get("inputs", {}).get(key) != measured.get("inputs", {}).get(key):
            failures.append(
                f"input {key} is {measured.get('inputs', {}).get(key)!r}, the baseline's "
                f"{baseline.get('inputs', {}).get(key)!r}: the numbers are of another workload"
            )
    held = baseline.get("metrics", {})
    got = measured.get("metrics", {})
    for name in sorted(set(held) | set(got)):
        if name not in got:
            failures.append(f"{name}: held by the baseline, not measured")
            continue
        if name not in held:
            failures.append(f"{name}: measured, not held by the baseline")
            continue
        base = _number(held[name].get("value"), f"the baseline's {name}")
        tolerance = _number(held[name].get("tolerance"), f"the tolerance of {name}")
        if not 0 <= tolerance < 1:
            raise GateError(f"the tolerance of {name} is {tolerance}: it must be at least 0 and under 1")
        if name.endswith(".fsyncs") and base * tolerance >= 1:
            raise GateError(
                f"the tolerance of {name} holds {base * tolerance} syncs more per operation: one more must fail"
            )
        value = _number(got[name], f"the measured {name}")
        change = f"{(value - base) / base * 100:+.2f}%" if base else "from 0"
        notes.append(f"{name}: {base} -> {value} ({change}, tolerance {tolerance * 100:.2f}%)")
        if value > base * (1 + tolerance) + EPSILON:
            failures.append(f"{name}: worse, {value} against {base} ({change}; tolerance {tolerance * 100:.2f}%)")
        elif value < base * (1 - tolerance) - EPSILON:
            failures.append(
                f"{name}: better, {value} against {base} ({change}): commit the baseline with {value}"
            )
    if other_arch_report_only and measured.get("arch") != baseline.get("arch"):
        # Nothing of another arch fails; each would-be failure is reported.
        notes.extend(f"report only: {f}" for f in failures)
        failures = []
    return failures, notes


def updated_baseline(baseline: dict, measured: dict) -> dict:
    """The baseline that holds `measured`, each tolerance kept."""
    held = baseline.get("metrics", {})
    metrics = {}
    for name, value in measured["metrics"].items():
        if name in held:
            tolerance = held[name]["tolerance"]
        elif name.endswith(".fsyncs"):
            tolerance = FSYNC_TOLERANCE
        else:
            tolerance = DEFAULT_TOLERANCE
        metrics[name] = {"value": value, "tolerance": tolerance}
    return {
        "arch": measured["arch"],
        "inputs": copy.deepcopy(measured["inputs"]),
        "valgrind": measured.get("valgrind"),
        "metrics": metrics,
    }


def baseline_text(baseline: dict) -> str:
    """The baseline file's text."""
    return json.dumps(baseline, indent=2, sort_keys=True) + "\n"


def valgrind_command(tool: str, out_dir: str, argv: list[str]) -> list[str]:
    """The valgrind command that runs `argv` under `tool`."""
    if not argv:
        raise GateError("no server command to run under valgrind")
    log = f"--log-file={out_dir}/valgrind.log"
    if tool == "cachegrind":
        return [
            "valgrind", "--tool=cachegrind", "--cache-sim=no",
            f"--cachegrind-out-file={out_dir}/cachegrind.out", "--trace-syscalls=yes", log, *argv,
        ]
    if tool == "dhat":
        return ["valgrind", "--tool=dhat", f"--dhat-out-file={out_dir}/dhat.json", log, *argv]
    raise GateError(f"the tool {tool!r} is not one of {', '.join(TOOLS)}")


def workloads(count: int, reads: int, pages: int) -> dict[str, tuple[int, int, int]]:
    """(stores, reads, list pages) of each workload."""
    if pages < 1:
        raise GateError("no list pages: the list's counts would measure nothing")
    # vcon-scale asks for the newest page again after the last
    # (vcon_scale::pages::follow), so PAGES may pass the pages COUNT fills.
    return {
        "base": (count, 0, 0),
        "create": (2 * count, 0, 0),
        "read": (count, reads, 0),
        "list": (count, 0, pages),
    }


def scale_flags(inputs: dict, reads: int, pages: int) -> list[str]:
    """vcon-scale's flags after scale-run.sh's `--`: every input fixed."""
    return [
        "--concurrency", str(inputs["concurrency"]), "--reads", str(reads), "--list-pages", str(pages),
        "--seed", str(inputs["seed"]), "--mix", inputs["mix"],
        # The counts do not depend on the load; the start guard is for timings.
        "--max-load", "1000000",
    ]


_PHASE = {"create": "ingest", "read": "reads", "list": "lists"}
# The workload each operation's workload adds to.
REFERENCE = {"create": "base", "read": "base", "list": "base"}


def operations(base_result: dict, result: dict, workload: str) -> int:
    """The operations `workload` added to the base, from the two result
    files vcon-scale wrote (their `phases`); a run with an answered error
    is refused."""
    counts = {}
    for key, which in (("base", base_result), ("run", result)):
        for phase in _PHASE.values():
            try:
                errors = which["phases"][phase]["errors"]
                counts[key, phase] = which["phases"][phase]["requests"]
            except (KeyError, TypeError) as e:
                raise GateError(f"vcon-scale's result has no {phase} phase with its counts ({e})") from e
            if errors:
                raise GateError(f"{errors} {phase} requests failed: the counts include other work")
    phase = _PHASE[workload]
    added = counts["run", phase] - counts["base", phase]
    if added <= 0:
        raise GateError(f"the {workload} workload added no {phase} request")
    return added


def measure(run, inputs: dict, arch: str, valgrind: str) -> dict:
    """Runs every workload under each tool with `run(name, tool, count,
    reads, pages)`, which answers (files, vcon-scale's result), and derives
    the metrics."""
    plan = workloads(inputs["count"], inputs["reads"], inputs["pages"])
    totals: dict[str, dict[str, int]] = {}
    results = {}
    for name, (count, reads, pages) in plan.items():
        files, result = run(name, "cachegrind", count, reads, pages)
        calls = parse_syscalls(files["log"])
        totals[name] = {
            "instructions": steady_instructions(files["cachegrind"]),
            "syscalls": steady_syscalls(calls),
            "fsyncs": fsyncs(calls),
        }
        results[name] = result
        files, _ = run(name, "dhat", count, reads, pages)
        totals[name]["allocations"] = parse_dhat_blocks(files["log"])
    ops = {op: operations(results[REFERENCE[op]], results[op], op) for op in OPERATIONS}
    return {"arch": arch, "inputs": dict(inputs), "valgrind": valgrind, "metrics": derive(totals, ops)}


def _real_runner(server: str, scale: str, inputs: dict, root: pathlib.Path, work: pathlib.Path):
    """A runner that starts scale-run.sh. It runs valgrind and a server,
    so no unit test drives it: CI's perf-gate job does, every run."""

    def run(name, tool, count, reads, pages):
        out = work / f"{name}-{tool}"
        out.mkdir()
        env = dict(
            os.environ,
            SCALE_SERVE_WRAP=str(root / "scripts/perf-serve.sh"),
            SCALE_START_SECONDS="300",
            SCALE_LABEL=f"perf-{name}-{tool}",
            PERF_GATE_TOOL=tool,
            PERF_GATE_OUT=str(out),
        )
        result_file = out / "scale.json"
        cmd = [str(root / "scripts/scale-run.sh"), server, scale, str(count), str(result_file), str(work),
               "--", *scale_flags(inputs, reads, pages)]
        done = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=1800, check=False)
        if done.returncode != 0:
            raise GateError(f"{name} under {tool}: scale-run.sh exited {done.returncode}: {done.stderr[-2000:]}")
        files = {"log": (out / "valgrind.log").read_text()}
        if tool == "cachegrind":
            files["cachegrind"] = (out / "cachegrind.out").read_text()
        return files, json.loads(result_file.read_text())

    return run


def _read_json(path: str):
    try:
        return json.loads(pathlib.Path(path).read_text())
    except (OSError, ValueError) as e:
        raise GateError(f"{path}: {e}") from e


def _serve(argv: list[str], err) -> int:
    tool = os.environ.get("PERF_GATE_TOOL")
    out = os.environ.get("PERF_GATE_OUT")
    if not tool or not out:
        print("perf_gate serve: PERF_GATE_TOOL and PERF_GATE_OUT must name the tool and its directory", file=err)
        return 2
    cmd = valgrind_command(tool, out, argv)
    if os.environ.get("PERF_GATE_DRY"):
        print(json.dumps(cmd))
        return 0
    os.execvp(cmd[0], cmd)
    return 2  # not reached


def main(argv: list[str], out=sys.stdout, err=sys.stderr) -> int:
    if argv[:1] == ["serve"]:
        rest = argv[1:]
        if rest[:1] == ["--"]:
            rest = rest[1:]
        return _serve(rest, err)
    parser = argparse.ArgumentParser(prog="perf_gate.py")
    sub = parser.add_subparsers(dest="command", required=True)
    m = sub.add_parser("measure")
    m.add_argument("--server", required=True)
    m.add_argument("--scale", required=True)
    m.add_argument("--out", required=True)
    m.add_argument("--count", type=int, default=500)
    m.add_argument("--reads", type=int, default=500)
    m.add_argument("--pages", type=int, default=100)
    for name in ("compare", "update"):
        c = sub.add_parser(name)
        c.add_argument("baseline")
        c.add_argument("measured")
        if name == "compare":
            c.add_argument("--other-arch-report-only", action="store_true")
    try:
        args = parser.parse_args(argv)
    except SystemExit:
        return 2
    try:
        if args.command == "measure":
            root = pathlib.Path(__file__).resolve().parent.parent
            inputs = {"count": args.count, "reads": args.reads, "pages": args.pages, "seed": 7,
                      "mix": "1:2048", "concurrency": 1}
            version = subprocess.run(["valgrind", "--version"], capture_output=True, text=True,
                                     timeout=60, check=True).stdout.strip()
            with tempfile.TemporaryDirectory(prefix="perf-gate.") as work:
                run = _real_runner(os.path.realpath(args.server), os.path.realpath(args.scale), inputs, root,
                                   pathlib.Path(work))
                result = measure(run, inputs, platform.machine(), version)
            pathlib.Path(args.out).write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
            print(json.dumps(result, indent=2, sort_keys=True), file=out)
            return 0
        baseline = _read_json(args.baseline)
        measured = _read_json(args.measured)
        if args.command == "update":
            pathlib.Path(args.baseline).write_text(baseline_text(updated_baseline(baseline, measured)))
            return 0
        failures, notes = compare(baseline, measured, args.other_arch_report_only)
    except GateError as e:
        print(f"perf_gate: {e}", file=err)
        return 2
    for note in notes:
        print(note, file=err)
    if failures:
        for failure in failures:
            print(f"perf_gate: FAILED: {failure}", file=err)
        print(baseline_text(updated_baseline(baseline, measured)), end="", file=out)
        return 1
    print("perf_gate: every metric is inside its tolerance", file=err)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
