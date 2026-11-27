"""Tests scripts/perf_gate.py: the performance regression gate of CI's
"perf-gate" job.

The gate runs the server under valgrind through scripts/scale-run.sh, once
per workload and tool, and holds counts that do not depend on the CPU's
speed or the host's load (instructions, syscalls, fsyncs, heap
allocations, each per operation) to scripts/perf-baseline.json. These tests
drive each rule with text and numbers given here: the parsers of
valgrind's output, the per-operation differences, the comparison with its
tolerance and ratchet, the arch rule, the updated baseline, the valgrind
command lines and the measure step through a fake runner. No valgrind and
no server run here.

Run with: python3 -I scripts/test_perf_gate.py
"""

import copy
import importlib.util
import io
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("perf_gate", HERE / "perf_gate.py")
pg = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pg)

CACHEGRIND = """desc: I1 cache: 32768 B
cmd: /srv/vcon-server serve --config /w/vcon-server.toml
events: Ir
fl=/src/a.rs
fn=main
1 1284491
summary: 1284491
"""

SYSCALLS = """SYSCALL[17,1](214) sys_brk ( 0x0 ) --> [pre-success] Success(0x4042000)
SYSCALL[17,1](56) sys_openat ( -100, 0x4041be0(/x.so), 524288 ) --> [async] ...
SYSCALL[17,1](56) ... [async] --> Success(0x4)
SYSCALL[17,3](82) sys_fsync ( 5 ) --> [async] ...
SYSCALL[17,3](82) ... [async] --> Success(0x0)
SYSCALL[17,4](83) sys_fdatasync ( 6 ) --> [async] ...
SYSCALL[17,4](83) ... [async] --> Success(0x0)
SYSCALL[17,1](57) sys_close ( 4 )[sync] --> Success(0x0)
--17-- a line valgrind writes about something else
"""

DHAT = """==17== DHAT, a dynamic heap analysis tool
==17== Command: /srv/vcon-server serve
==17==
==17== Total:     1,234,567 bytes in 12,345 blocks
==17== At t-gmax: 99,000 bytes in 900 blocks
==17== At t-end:  0 bytes in 0 blocks
==17== Reads:     72,916 bytes
"""

INPUTS = {"count": 200, "reads": 400, "pages": 2, "seed": 7, "mix": "1:2048", "concurrency": 1}


def measured(**metrics):
    """A measured file: every metric 100.0 but those given."""
    values = {name: 100.0 for name in pg.metric_names()}
    values.update(metrics)
    return {"arch": "x86_64", "inputs": dict(INPUTS), "valgrind": "valgrind-3.22.0", "metrics": values}


def baseline(tolerance=0.02, exact=0.0, **metrics):
    """A baseline: every metric 100.0 but those given; fsyncs held exactly."""
    values = {name: 100.0 for name in pg.metric_names()}
    values.update(metrics)
    return {
        "arch": "x86_64",
        "inputs": dict(INPUTS),
        "metrics": {
            name: {"value": value, "tolerance": exact if name.endswith(".fsyncs") else tolerance}
            for name, value in values.items()
        },
    }


class Cachegrind(unittest.TestCase):
    def test_reads_the_summary_line(self):
        self.assertEqual(pg.parse_cachegrind(CACHEGRIND), 1284491, "the Ir total")

    def test_a_file_without_a_summary_is_refused(self):
        with self.assertRaises(pg.GateError, msg="no summary: the run did not end cleanly"):
            pg.parse_cachegrind("events: Ir\nfn=main\n1 5\n")

    def test_two_summaries_are_refused(self):
        with self.assertRaises(pg.GateError, msg="two runs in one file"):
            pg.parse_cachegrind(CACHEGRIND + "summary: 5\n")

    def test_a_summary_that_is_not_a_number_is_refused(self):
        with self.assertRaises(pg.GateError, msg="not a count"):
            pg.parse_cachegrind("events: Ir\nsummary: lots\n")

    def test_a_file_of_another_event_is_refused(self):
        with self.assertRaises(pg.GateError, msg="the summary must count Ir alone"):
            pg.parse_cachegrind("events: Ir Dr\nsummary: 5 6\n")

    def test_a_file_that_counts_one_other_event_is_refused(self):
        with self.assertRaises(pg.GateError, msg="one number, but not of instructions"):
            pg.parse_cachegrind("events: Dr\nsummary: 5\n")

    def test_a_summary_of_zero_is_read(self):
        self.assertEqual(pg.parse_cachegrind("events: Ir\nsummary: 0\n"), 0, "zero is a count")


JITTER = """events: Ir
fl=???
fn=main
1 1000
2 500
fn=jent_keccakp_chi
1 300
fn=aws_lc_0_47_0_jent_sha3_update
5 40
fn=rol64
1 60
fl=/src/b.rs
fn=main
3 100
summary: 2000
"""


class Instructions(unittest.TestCase):
    """aws-lc seeds its random generator from CPU jitter: the jitter
    entropy source runs as many rounds as the clock asks, so its
    instructions differ from run to run (on 2026-11-27, by 0.68 million
    between two runs whose totals differed by 0.66 million). They are
    left out."""

    def test_sums_every_function_but_the_jitter_sources(self):
        self.assertEqual(pg.steady_instructions(JITTER), 1600, "2000 less 300, 40 and 60")

    def test_a_function_named_twice_is_summed_both_times(self):
        self.assertEqual(pg.function_instructions(JITTER)["main"], 1600, "two files, one name")

    def test_the_costs_must_add_up_to_the_summary(self):
        with self.assertRaises(pg.GateError, msg="a file read wrongly"):
            pg.steady_instructions(JITTER.replace("summary: 2000", "summary: 2001"))

    def test_a_cost_line_before_any_function_is_refused(self):
        with self.assertRaises(pg.GateError, msg="whose cost is it"):
            pg.steady_instructions("events: Ir\n1 5\nsummary: 5\n")

    def test_a_cost_line_that_is_not_two_numbers_is_refused(self):
        with self.assertRaises(pg.GateError, msg="not a cost line"):
            pg.steady_instructions("events: Ir\nfn=main\n1 x\nsummary: 1\n")

    def test_a_file_with_no_jitter_is_its_summary(self):
        self.assertEqual(pg.steady_instructions(CACHEGRIND), 1284491, "one function, all of it")

    def test_each_jitter_name_is_one_of_the_rule(self):
        for name in ("jent_memaccess", "aws_lc_0_48_0_jent_read_entropy", "xoshiro128starstar", "uint32rotl"):
            self.assertTrue(pg.is_jitter(name), name)
        for name in ("main", "jentry", "sha3_update", "<tokio::runtime::Handle>::spawn"):
            self.assertFalse(pg.is_jitter(name), name)


class Syscalls(unittest.TestCase):
    def test_counts_each_call_once_even_when_it_completes_later(self):
        counts = pg.parse_syscalls(SYSCALLS)
        self.assertEqual(counts["openat"], 1, "the [async] completion line is the same call")
        self.assertEqual(sum(counts.values()), 5, "five calls in the text")

    def test_names_every_call_without_its_prefix(self):
        self.assertEqual(
            sorted(pg.parse_syscalls(SYSCALLS)),
            ["brk", "close", "fdatasync", "fsync", "openat"],
            "the call names",
        )

    def test_fsyncs_are_fsync_and_fdatasync_together(self):
        self.assertEqual(pg.fsyncs(pg.parse_syscalls(SYSCALLS)), 2, "one of each")

    def test_fsyncs_count_sync_file_range_and_syncfs_too(self):
        counts = {"sync_file_range": 2, "syncfs": 1, "fsync": 1, "read": 9}
        self.assertEqual(pg.fsyncs(counts), 4, "every call that waits for the disk")

    def test_a_log_without_any_call_is_refused(self):
        with self.assertRaises(pg.GateError, msg="valgrind ran without --trace-syscalls"):
            pg.parse_syscalls("==17== Cachegrind\n")

    def test_lines_of_other_text_are_ignored(self):
        text = "SYSCALL[1,1](57) sys_close ( 4 )[sync] --> Success(0x0) \nprogram output sys_fsync ( 3 )\n"
        self.assertEqual(pg.parse_syscalls(text), {"close": 1}, "only valgrind's own lines count")

    def test_the_steady_calls_leave_out_the_ones_that_wait_or_read_the_clock(self):
        counts = {"clock_gettime": 900, "futex": 40, "epoll_pwait": 30, "sched_yield": 2,
                  "clock_nanosleep": 1, "nanosleep": 1, "epoll_wait": 1, "pwrite64": 7, "fsync": 2}
        self.assertEqual(pg.steady_syscalls(counts), 9, "pwrite64 and fsync")

    def test_the_steady_calls_of_none_is_zero(self):
        self.assertEqual(pg.steady_syscalls({"futex": 3}), 0, "only waits")

    def test_fsyncs_of_none_is_zero(self):
        self.assertEqual(pg.fsyncs({"read": 3}), 0, "no sync at all")


class Dhat(unittest.TestCase):
    def test_reads_the_total_blocks(self):
        self.assertEqual(pg.parse_dhat_blocks(DHAT), 12345, "the blocks of the Total line")

    def test_a_log_without_a_total_is_refused(self):
        with self.assertRaises(pg.GateError, msg="DHAT did not finish"):
            pg.parse_dhat_blocks("==17== DHAT\n==17== At t-end: 0 bytes in 0 blocks\n")

    def test_two_totals_are_refused(self):
        with self.assertRaises(pg.GateError, msg="two processes in one log"):
            pg.parse_dhat_blocks(DHAT + "==18== Total:     5 bytes in 1 blocks\n")

    def test_a_total_with_no_separators_is_read(self):
        self.assertEqual(pg.parse_dhat_blocks("==1== Total: 10 bytes in 7 blocks\n"), 7, "plain digits")

    def test_one_block_is_read(self):
        self.assertEqual(pg.parse_dhat_blocks("==1== Total: 10 bytes in 1 block\n"), 1, "singular")


class PerOperation(unittest.TestCase):
    def test_is_the_difference_over_the_operations(self):
        self.assertEqual(pg.per_operation(1000, 3000, 400), 5.0, "2000 over 400")

    def test_rounds_to_three_places(self):
        self.assertEqual(pg.per_operation(0, 1000, 3), 333.333, "three places")

    def test_no_operations_is_refused(self):
        with self.assertRaises(pg.GateError, msg="nothing to divide by"):
            pg.per_operation(1, 2, 0)

    def test_a_count_that_falls_when_work_is_added_is_refused(self):
        with self.assertRaises(pg.GateError, msg="the measurement is broken"):
            pg.per_operation(3000, 1000, 10)

    def test_no_difference_is_zero(self):
        self.assertEqual(pg.per_operation(7, 7, 10), 0.0, "an operation that costs nothing more")


class Derive(unittest.TestCase):
    def totals(self, ir, calls, syncs, blocks):
        return {"instructions": ir, "syscalls": calls, "fsyncs": syncs, "allocations": blocks}

    def test_each_operation_is_its_workload_less_the_base(self):
        runs = {
            "base": self.totals(1000, 100, 10, 50),
            "create": self.totals(3000, 300, 210, 250),
            "read": self.totals(1800, 180, 10, 130),
            # The list pages follow the base workload's stores.
            "list": self.totals(1100, 104, 10, 52),
        }
        ops = {"create": 200, "read": 400, "list": 2}
        got = pg.derive(runs, ops)
        self.assertEqual(got["create.instructions"], 10.0, "2000 over 200")
        self.assertEqual(got["create.fsyncs"], 1.0, "one sync per create")
        self.assertEqual(got["read.fsyncs"], 0.0, "no sync per read")
        self.assertEqual(got["list.syscalls"], 2.0, "4 over 2 pages, less the base")
        self.assertEqual(got["list.instructions"], 50.0, "100 over 2 pages")
        self.assertEqual(sorted(got), sorted(pg.metric_names()), "every metric, none more")

    def test_a_missing_workload_is_refused(self):
        runs = {"base": self.totals(1, 1, 1, 1)}
        with self.assertRaises(pg.GateError, msg="create was never run"):
            pg.derive(runs, {"create": 1, "read": 1, "list": 1})

    def test_a_missing_count_is_refused(self):
        t = self.totals(1, 1, 1, 1)
        runs = {"base": t, "create": t, "read": t, "list": {"instructions": 1}}
        with self.assertRaises(pg.GateError, msg="list has no syscalls"):
            pg.derive(runs, {"create": 1, "read": 1, "list": 1})


class Compare(unittest.TestCase):
    def test_equal_numbers_pass(self):
        failures, _ = pg.compare(baseline(), measured())
        self.assertEqual(failures, [], "nothing changed")

    def test_a_rise_inside_the_tolerance_passes(self):
        failures, _ = pg.compare(baseline(), measured(**{"read.instructions": 101.9}))
        self.assertEqual(failures, [], "1.9 % under a 2 % band")

    def test_a_rise_past_the_tolerance_fails_naming_the_metric(self):
        failures, _ = pg.compare(baseline(), measured(**{"read.instructions": 102.1}))
        self.assertEqual(len(failures), 1, "one metric")
        self.assertIn("read.instructions", failures[0], "named")
        self.assertIn("worse", failures[0], "said to be worse")

    def test_a_fall_past_the_tolerance_fails_with_the_new_value(self):
        failures, _ = pg.compare(baseline(), measured(**{"create.allocations": 90.0}))
        self.assertEqual(len(failures), 1, "the ratchet")
        self.assertIn("better", failures[0], "said to be better")
        self.assertIn("90.0", failures[0], "the value to write")

    def test_a_fall_inside_the_tolerance_passes(self):
        failures, _ = pg.compare(baseline(), measured(**{"create.allocations": 98.5}))
        self.assertEqual(failures, [], "noise")

    def test_one_fsync_more_per_read_fails(self):
        failures, _ = pg.compare(baseline(**{"read.fsyncs": 0.0}), measured(**{"read.fsyncs": 1.0}))
        self.assertEqual(len(failures), 1, "the mutation the gate exists for")
        self.assertIn("read.fsyncs", failures[0], "named")

    def test_fsyncs_with_no_band_are_held_exactly(self):
        failures, _ = pg.compare(baseline(), measured(**{"create.fsyncs": 100.005}))
        self.assertEqual(len(failures), 1, "no band on an exact count")

    def test_one_more_sync_per_operation_fails_inside_any_allowed_band(self):
        b = baseline(**{"create.fsyncs": 4.036})
        b["metrics"]["create.fsyncs"]["tolerance"] = 0.2
        failures, _ = pg.compare(b, measured(**{"create.fsyncs": 5.036}))
        self.assertEqual(len(failures), 1, "4.036 with a 20% band holds 0.81 more, not 1")

    def test_an_fsync_band_that_holds_one_more_sync_is_refused(self):
        b = baseline(**{"create.fsyncs": 4.0})
        b["metrics"]["create.fsyncs"]["tolerance"] = 0.25
        with self.assertRaises(pg.GateError, msg="4.0 with a 25% band would hold a fifth sync"):
            pg.compare(b, measured(**{"create.fsyncs": 4.0}))

    def test_a_rounding_difference_on_an_exact_count_passes(self):
        failures, _ = pg.compare(baseline(), measured(**{"create.fsyncs": 100.0004}))
        self.assertEqual(failures, [], "under half the last place")

    def test_a_count_from_zero_fails_whatever_the_band(self):
        failures, _ = pg.compare(
            baseline(tolerance=0.5, **{"list.allocations": 0.0}), measured(**{"list.allocations": 0.01})
        )
        self.assertEqual(len(failures), 1, "a share of zero is zero")

    def test_a_metric_missing_from_the_measurement_fails(self):
        m = measured()
        del m["metrics"]["read.syscalls"]
        failures, _ = pg.compare(baseline(), m)
        self.assertEqual(len(failures), 1, "missing")
        self.assertIn("read.syscalls", failures[0], "named")

    def test_a_metric_the_baseline_does_not_hold_fails(self):
        m = measured(**{"read.page_faults": 3.0})
        failures, _ = pg.compare(baseline(), m)
        self.assertEqual(len(failures), 1, "an unheld metric")
        self.assertIn("read.page_faults", failures[0], "named")

    def test_other_inputs_fail(self):
        m = measured()
        m["inputs"]["count"] = 100
        failures, _ = pg.compare(baseline(), m)
        self.assertEqual(len(failures), 1, "numbers of another workload")
        self.assertIn("count", failures[0], "the input named")

    def test_another_arch_fails_by_default(self):
        m = measured()
        m["arch"] = "aarch64"
        failures, _ = pg.compare(baseline(), m)
        self.assertEqual(len(failures), 1, "a baseline of another arch holds nothing")
        self.assertIn("aarch64", failures[0], "the arch named")

    def test_another_arch_is_reported_only_when_asked(self):
        m = measured(**{"read.instructions": 500.0})
        m["arch"] = "aarch64"
        failures, notes = pg.compare(baseline(), m, other_arch_report_only=True)
        self.assertEqual(failures, [], "report only")
        self.assertTrue(any("aarch64" in n for n in notes), "and said why")

    def test_another_valgrind_is_a_note_not_a_failure(self):
        m = measured()
        b = baseline()
        b["valgrind"] = "valgrind-3.21.0"
        failures, notes = pg.compare(b, m)
        self.assertEqual(failures, [], "the counts are the program's, not the tool's")
        self.assertTrue(any("valgrind-3.21.0" in n for n in notes), "noted")

    def test_every_failure_is_listed_not_only_the_first(self):
        failures, _ = pg.compare(
            baseline(), measured(**{"read.instructions": 200.0, "list.syscalls": 300.0})
        )
        self.assertEqual(len(failures), 2, "both")

    def test_a_tolerance_that_is_negative_is_refused(self):
        b = baseline()
        b["metrics"]["read.instructions"]["tolerance"] = -0.1
        with self.assertRaises(pg.GateError, msg="a band below zero"):
            pg.compare(b, measured())

    def test_a_tolerance_of_one_or_more_is_refused(self):
        b = baseline()
        b["metrics"]["read.instructions"]["tolerance"] = 1.0
        with self.assertRaises(pg.GateError, msg="a band that holds nothing"):
            pg.compare(b, measured())

    def test_a_value_that_is_not_a_number_is_refused(self):
        m = measured(**{"read.instructions": "fast"})
        with self.assertRaises(pg.GateError, msg="not a number"):
            pg.compare(baseline(), m)

    def test_a_boolean_is_not_a_number(self):
        m = measured(**{"read.instructions": True})
        with self.assertRaises(pg.GateError, msg="JSON true is no count"):
            pg.compare(baseline(), m)

    def test_a_rise_exactly_at_the_tolerance_passes(self):
        failures, _ = pg.compare(baseline(tolerance=0.25), measured(**{"read.instructions": 125.0}))
        self.assertEqual(failures, [], "the band is inclusive")

    def test_the_notes_give_each_metric_with_its_change(self):
        _, notes = pg.compare(baseline(), measured(**{"read.instructions": 101.0}))
        line = [n for n in notes if n.startswith("read.instructions")]
        self.assertEqual(len(line), 1, "one line for the metric")
        self.assertIn("+1.00%", line[0], "its change")


class Update(unittest.TestCase):
    def test_takes_the_measured_values_and_keeps_the_tolerances(self):
        b = baseline(tolerance=0.03)
        new = pg.updated_baseline(b, measured(**{"read.instructions": 120.0}))
        self.assertEqual(new["metrics"]["read.instructions"], {"value": 120.0, "tolerance": 0.03}, "kept")
        self.assertEqual(new["metrics"]["create.fsyncs"]["tolerance"], 0.0, "exact stays exact")

    def test_takes_the_arch_inputs_and_valgrind(self):
        m = measured()
        m["valgrind"] = "valgrind-3.23.0"
        new = pg.updated_baseline(baseline(), m)
        self.assertEqual(new["valgrind"], "valgrind-3.23.0", "the tool recorded")
        self.assertEqual(new["inputs"], INPUTS, "the workload recorded")

    def test_a_new_metric_gets_the_default_tolerance(self):
        b = baseline()
        del b["metrics"]["read.instructions"]
        new = pg.updated_baseline(b, measured())
        self.assertEqual(
            new["metrics"]["read.instructions"]["tolerance"], pg.DEFAULT_TOLERANCE, "the default band"
        )

    def test_a_new_fsync_metric_gets_the_fsync_band(self):
        b = baseline()
        del b["metrics"]["read.fsyncs"]
        new = pg.updated_baseline(b, measured())
        self.assertEqual(new["metrics"]["read.fsyncs"]["tolerance"], pg.FSYNC_TOLERANCE, "the fsync band")
        self.assertLess(pg.FSYNC_TOLERANCE, pg.DEFAULT_TOLERANCE + 1e-9, "never wider than the default")

    def test_the_updated_baseline_passes_its_own_measurement(self):
        m = measured(**{"read.instructions": 333.0, "list.fsyncs": 2.0})
        failures, _ = pg.compare(pg.updated_baseline(baseline(), m), m)
        self.assertEqual(failures, [], "an update ends the failure it was made for")

    def test_does_not_change_the_baseline_given(self):
        b = baseline()
        before = copy.deepcopy(b)
        pg.updated_baseline(b, measured(**{"read.instructions": 1.0}))
        self.assertEqual(b, before, "a new object")

    def test_a_metric_no_longer_measured_is_dropped(self):
        b = baseline()
        b["metrics"]["read.page_faults"] = {"value": 1.0, "tolerance": 0.1}
        new = pg.updated_baseline(b, measured())
        self.assertNotIn("read.page_faults", new["metrics"], "only what is measured")


class Commands(unittest.TestCase):
    def test_cachegrind_runs_without_the_cache_simulation_and_traces_syscalls(self):
        cmd = pg.valgrind_command("cachegrind", "/out", ["/srv/vcon-server", "serve"])
        self.assertEqual(cmd[0], "valgrind", "the tool")
        self.assertIn("--tool=cachegrind", cmd, "cachegrind")
        self.assertIn("--cache-sim=no", cmd, "Ir alone")
        self.assertIn("--trace-syscalls=yes", cmd, "the syscalls")
        self.assertIn("--cachegrind-out-file=/out/cachegrind.out", cmd, "its file")
        self.assertIn("--log-file=/out/valgrind.log", cmd, "its log")
        self.assertEqual(cmd[-2:], ["/srv/vcon-server", "serve"], "the server last, as given")

    def test_dhat_writes_its_own_files(self):
        cmd = pg.valgrind_command("dhat", "/out", ["/srv/vcon-server", "serve"])
        self.assertIn("--tool=dhat", cmd, "DHAT")
        self.assertIn("--dhat-out-file=/out/dhat.json", cmd, "its file")
        self.assertIn("--log-file=/out/valgrind.log", cmd, "its log")
        self.assertNotIn("--trace-syscalls=yes", cmd, "the syscalls come from cachegrind's run")

    def test_another_tool_is_refused(self):
        with self.assertRaises(pg.GateError, msg="memcheck counts nothing the gate holds"):
            pg.valgrind_command("memcheck", "/out", ["/srv/vcon-server"])

    def test_no_server_command_is_refused(self):
        with self.assertRaises(pg.GateError, msg="nothing to run"):
            pg.valgrind_command("dhat", "/out", [])

    def test_the_workloads_differ_only_in_their_operations(self):
        w = pg.workloads(200, 400, 2)
        self.assertEqual(
            w,
            {
                "base": (200, 0, 0),
                "create": (400, 0, 0),
                "read": (200, 400, 0),
                "list": (200, 0, 2),
            },
            "count, reads, pages",
        )

    def test_more_list_pages_than_the_vcons_fill_are_followed_again(self):
        # vcon-scale asks for the newest page again after the last
        # (vcon_scale::pages::follow), so the pages are not bounded by the
        # stores: the noise of the base is spread over many pages.
        self.assertEqual(pg.workloads(200, 400, 100)["list"], (200, 0, 100), "100 pages of a list of 2")

    def test_no_list_pages_is_refused(self):
        with self.assertRaises(pg.GateError, msg="nothing to measure"):
            pg.workloads(200, 400, 0)

    def test_the_scale_flags_fix_every_input(self):
        flags = pg.scale_flags(INPUTS, 400, 0)
        self.assertEqual(
            flags,
            [
                "--concurrency", "1", "--reads", "400", "--list-pages", "0",
                "--seed", "7", "--mix", "1:2048", "--max-load", "1000000",
            ],
            "nothing left to a default",
        )


class Operations(unittest.TestCase):
    def result(self, ingest, reads, lists, errors=0):
        phase = lambda n: {"requests": n, "errors": errors}  # noqa: E731
        # The shape of vcon-scale's result file (format vcon-scale/1).
        return {"format": "vcon-scale/1",
                "phases": {"ingest": phase(ingest), "reads": phase(reads), "lists": phase(lists)}}

    def test_reads_a_result_file_vcon_scale_wrote(self):
        # Part of the file one run wrote on 2026-11-27 (20 stores, 40 reads).
        real = json.loads(
            '{"count":20,"format":"vcon-scale/1","phases":{"ingest":{"errors":0,"latency_us":{"count":20,'
            '"max":1654598,"min":29489,"p50":37664,"p95":71965,"p99":1654598,"p999":1654598},"micros":2406478,'
            '"per_second":8.310900826851523,"requests":20},"lists":{"errors":0,"latency_us":null,"micros":0,'
            '"per_second":null,"requests":0},"reads":{"errors":0,"latency_us":{"count":40,"max":65650,'
            '"min":9333,"p50":11676,"p95":27867,"p99":65650,"p999":65650},"micros":613098,'
            '"per_second":65.24242453898071,"requests":40}},"seed":7}'
        )
        self.assertEqual(pg.operations(self.result(20, 0, 0), real, "read"), 40, "its reads")

    def test_a_result_without_its_phases_is_refused(self):
        with self.assertRaises(pg.GateError, msg="not vcon-scale's result"):
            pg.operations({"ingest": {}}, self.result(200, 400, 0), "read")

    def test_a_phase_without_its_counts_is_refused(self):
        broken = self.result(200, 400, 0)
        del broken["phases"]["reads"]["errors"]
        with self.assertRaises(pg.GateError, msg="no error count"):
            pg.operations(self.result(200, 0, 0), broken, "read")

    def test_reads_each_phases_requests(self):
        got = pg.operations(self.result(200, 0, 0), self.result(400, 0, 0), "create")
        self.assertEqual(got, 200, "the stores the create workload added")

    def test_reads_the_reads(self):
        got = pg.operations(self.result(200, 0, 0), self.result(200, 400, 0), "read")
        self.assertEqual(got, 400, "the reads")

    def test_reads_the_pages_followed(self):
        got = pg.operations(self.result(200, 0, 0), self.result(200, 0, 2), "list")
        self.assertEqual(got, 2, "the pages")

    def test_a_run_with_an_error_is_refused(self):
        with self.assertRaises(pg.GateError, msg="a refused request costs other work"):
            pg.operations(self.result(200, 0, 0), self.result(200, 400, 0, errors=1), "read")

    def test_a_workload_that_added_nothing_is_refused(self):
        with self.assertRaises(pg.GateError, msg="nothing to measure"):
            pg.operations(self.result(200, 0, 0), self.result(200, 0, 0), "list")


class Measure(unittest.TestCase):
    """The measure step, through a runner that answers what valgrind and
    vcon-scale would have written."""

    def fake_runner(self, per_op):
        calls = []

        def run(name, tool, count, reads, pages):
            calls.append((name, tool, count, reads, pages))
            ir = 1000 + per_op["ir"] * (count + reads + pages)
            fs = 3 + per_op["fs"] * count + per_op.get("rfs", 0) * reads
            if tool == "cachegrind":
                # The jitter source's share differs per run and is left out.
                jitter = 7 * len(calls)
                text = f"events: Ir\nfn=main\n1 {ir}\nfn=jent_x\n1 {jitter}\nsummary: {ir + jitter}\n"
                calls_text = "".join(
                    f"SYSCALL[1,1](82) sys_fsync ( 5 ) --> [async] ... \n" for _ in range(fs)
                ) + "SYSCALL[1,2](98) sys_futex ( 0x1 ) --> Success(0x0) \n" * len(calls)
                files = {"cachegrind": text, "log": calls_text}
            else:
                files = {"log": f"==1== Total: 9 bytes in {10 + count + reads + pages} blocks\n"}
            phase = lambda n: {"requests": n, "errors": 0}  # noqa: E731
            return files, {"phases": {"ingest": phase(count), "reads": phase(reads), "lists": phase(pages)}}

        return run, calls

    def test_runs_each_workload_under_each_tool(self):
        run, calls = self.fake_runner({"ir": 5, "fs": 1})
        pg.measure(run, INPUTS, "x86_64", "valgrind-3.22.0")
        self.assertEqual(len(calls), 8, "four workloads, two tools")
        self.assertEqual(sorted({c[1] for c in calls}), ["cachegrind", "dhat"], "both tools")

    def test_derives_each_metric_per_operation(self):
        run, _ = self.fake_runner({"ir": 5, "fs": 1})
        got = pg.measure(run, INPUTS, "x86_64", "valgrind-3.22.0")
        self.assertEqual(got["metrics"]["create.instructions"], 5.0, "5 per create")
        self.assertEqual(got["metrics"]["create.fsyncs"], 1.0, "one sync per create")
        self.assertEqual(got["metrics"]["read.fsyncs"], 0.0, "none per read")
        self.assertEqual(got["metrics"]["read.syscalls"], 0.0, "the futex calls are left out")
        self.assertEqual(got["metrics"]["list.allocations"], 1.0, "one block per page")
        self.assertEqual(got["arch"], "x86_64", "the arch")
        self.assertEqual(got["inputs"], INPUTS, "the inputs")

    def test_an_fsync_added_per_read_shows_as_one(self):
        run, _ = self.fake_runner({"ir": 5, "fs": 1, "rfs": 1})
        got = pg.measure(run, INPUTS, "x86_64", "valgrind-3.22.0")
        self.assertEqual(got["metrics"]["read.fsyncs"], 1.0, "the mutation, measured")


class Cli(unittest.TestCase):
    def run_cli(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        code = pg.main(list(argv), out, err)
        return code, out.getvalue(), err.getvalue()

    def files(self, b, m):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        bp, mp = pathlib.Path(d.name, "b.json"), pathlib.Path(d.name, "m.json")
        bp.write_text(json.dumps(b))
        mp.write_text(json.dumps(m))
        return str(bp), str(mp)

    def test_compare_passes_with_exit_0(self):
        code, out, _ = self.run_cli("compare", *self.files(baseline(), measured()))
        self.assertEqual(code, 0, out)

    def test_compare_fails_with_exit_1_and_prints_the_update(self):
        code, out, err = self.run_cli(
            "compare", *self.files(baseline(), measured(**{"read.fsyncs": 101.0}))
        )
        self.assertEqual(code, 1, "a failed gate")
        self.assertIn("read.fsyncs", err, "named")
        self.assertIn('"value": 101.0', out, "the baseline an intended change would commit")

    def test_compare_of_a_missing_file_exits_2(self):
        code, _, err = self.run_cli("compare", "/nonexistent/b.json", "/nonexistent/m.json")
        self.assertEqual(code, 2, "not a verdict")
        self.assertIn("b.json", err, "the file named")

    def test_compare_of_a_file_that_is_not_json_exits_2(self):
        b, m = self.files(baseline(), measured())
        pathlib.Path(m).write_text("{")
        code, _, err = self.run_cli("compare", b, m)
        self.assertEqual(code, 2, "not a verdict")
        self.assertIn("m.json", err, "the file named")

    def test_an_unknown_command_exits_2(self):
        code, _, _ = self.run_cli("bless")
        self.assertEqual(code, 2, "usage")

    def test_the_committed_baseline_holds_every_metric(self):
        committed = json.loads((HERE / "perf-baseline.json").read_text())
        self.assertEqual(sorted(committed["metrics"]), sorted(pg.metric_names()), "every metric")
        self.assertEqual(committed["arch"], "x86_64", "CI's arch")
        for name, held in committed["metrics"].items():
            if name.endswith(".fsyncs"):
                self.assertLess(held["value"] * held["tolerance"], 1, f"{name}: one more sync fails")

    def test_the_committed_baseline_is_its_own_update(self):
        path = HERE / "perf-baseline.json"
        committed = json.loads(path.read_text())
        m = {"arch": committed["arch"], "inputs": committed["inputs"], "valgrind": committed["valgrind"],
             "metrics": {k: v["value"] for k, v in committed["metrics"].items()}}
        self.assertEqual(
            pg.baseline_text(pg.updated_baseline(committed, m)), path.read_text(),
            "the file is in the form `update` writes",
        )

    def test_the_wrapper_execs_valgrind_with_the_command(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        env = dict(os.environ, PERF_GATE_TOOL="cachegrind", PERF_GATE_OUT=d.name, PERF_GATE_DRY="1")
        done = subprocess.run(
            ["bash", str(HERE / "perf-serve.sh"), "/srv/vcon-server", "serve", "--config", "a b.toml"],
            env=env, capture_output=True, text=True, timeout=60, check=False,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        argv = json.loads(done.stdout)
        self.assertEqual(argv[0], "valgrind", "valgrind first")
        self.assertEqual(argv[-4:], ["/srv/vcon-server", "serve", "--config", "a b.toml"], "kept whole")

    def test_the_wrapper_without_its_settings_exits_2(self):
        env = {k: v for k, v in os.environ.items() if not k.startswith("PERF_GATE_")}
        done = subprocess.run(
            ["bash", str(HERE / "perf-serve.sh"), "/srv/vcon-server", "serve"],
            env=env, capture_output=True, text=True, timeout=60, check=False,
        )
        self.assertEqual(done.returncode, 2, "no tool named")
        self.assertIn("PERF_GATE_TOOL", done.stderr, "says which setting")


if __name__ == "__main__":
    unittest.main()
