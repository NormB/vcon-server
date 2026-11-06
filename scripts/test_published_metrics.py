"""Tests scripts/published_metrics.py: the home page's numbers are derived
from CI's own runs, by one rule each, and a run that cannot be counted is
refused rather than published.

Run with: python3 -I scripts/test_published_metrics.py
"""

import importlib.util
import pathlib
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("published_metrics", HERE / "published_metrics.py")
published_metrics = importlib.util.module_from_spec(spec)
spec.loader.exec_module(published_metrics)
count_tests = published_metrics.count_tests
line_coverage = published_metrics.line_coverage
Refused = published_metrics.Refused

PASSED = """
    Starting 136 tests across 2 binaries (5 tests skipped)
        PASS [   0.031s] (1/136) vcon json::tests::a_key_given_twice_is_refused
        SLOW [> 60.000s] (135/136) vcon-server::serve a_slow_test
        PASS [  64.020s] (136/136) vcon-server::serve a_slow_test
────────────
     Summary [  64.311s] 136 tests run: 136 passed (1 slow), 5 skipped
   Doc-tests vcon
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.20s
"""


def summary(count, covered):
    return {
        "type": "llvm.coverage.json.export",
        "version": "3.1.0",
        "data": [{"files": [], "totals": {"lines": {"count": count, "covered": covered, "percent": 100.0 * covered / count if count else 0}}}],
    }


class Tests(unittest.TestCase):
    def test_each_nextest_summary_and_each_doc_test_result_adds_its_passed_count(self):
        self.assertEqual(count_tests(PASSED), 137)

    def test_every_nextest_summary_counts(self):
        fuzz = "     Summary [   0.100s] 9 tests run: 9 passed, 0 skipped\n"
        self.assertEqual(count_tests(PASSED + fuzz), 146)

    def test_colour_codes_do_not_hide_a_summary_or_a_result_line(self):
        self.assertEqual(
            count_tests("\x1b[32;1m     Summary\x1b[0m [   1.000s] 5 tests run: 5 passed, 1 skipped\n"
                        "\x1b[32mtest result: ok.\x1b[0m 2 passed; 0 failed;"),
            7,
        )

    def test_a_line_that_merely_mentions_a_result_is_not_one(self):
        noise = "  see the test result: ok. 999 passed above\n  Summary [ 1s] 999 tests run: 999 passed\n"
        self.assertEqual(count_tests(PASSED + noise), 137)

    def test_a_failed_test_refuses_the_run(self):
        failed = PASSED + "     Summary [   4.006s] 4 tests run: 3 passed, 1 failed, 1 skipped\n"
        with self.assertRaisesRegex(Refused, "the run reported 1 failed"):
            count_tests(failed)

    def test_a_summary_whose_passed_tests_are_not_all_that_ran_is_refused(self):
        with self.assertRaisesRegex(Refused, "unreadable summary"):
            count_tests("     Summary [   1.000s] 5 tests run: 4 passed, 1 skipped\n")

    def test_a_test_ended_at_its_deadline_refuses_the_run(self):
        timed_out = PASSED + "     Summary [ 180.006s] 4 tests run: 3 passed, 1 timed out, 1 skipped\n"
        with self.assertRaisesRegex(Refused, "the run reported 1 timed out"):
            count_tests(timed_out)

    def test_a_run_that_stopped_early_refuses_the_run(self):
        cancelled = PASSED.replace("136 tests run: 136 passed", "3/136 tests run: 3 passed")
        with self.assertRaisesRegex(Refused, "unreadable summary"):
            count_tests(cancelled)

    def test_a_failed_doc_test_binary_refuses_the_run(self):
        failed = PASSED + "test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n"
        with self.assertRaisesRegex(Refused, "FAILED"):
            count_tests(failed)

    def test_output_with_no_nextest_summary_is_refused(self):
        with self.assertRaisesRegex(Refused, "no nextest `Summary` line"):
            count_tests("   Compiling vcon v0.1.0\nerror: could not compile\n")

    def test_doc_tests_alone_are_not_a_complete_run(self):
        with self.assertRaisesRegex(Refused, "no nextest `Summary` line"):
            count_tests("test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n")

    def test_a_run_of_no_tests_is_refused(self):
        with self.assertRaisesRegex(Refused, "0 tests"):
            count_tests("     Summary [   0.000s] 0 tests run: 0 passed, 5 skipped\n")

    def test_an_unreadable_result_line_is_refused(self):
        with self.assertRaisesRegex(Refused, "unreadable"):
            count_tests(PASSED + "test result: ok. many passed\n")

    def test_an_unreadable_summary_is_refused(self):
        with self.assertRaisesRegex(Refused, "unreadable summary"):
            count_tests("     Summary [   1.000s] many tests run: many passed\n")


class Coverage(unittest.TestCase):
    def test_lines_covered_and_counted_come_from_the_totals(self):
        self.assertEqual(line_coverage(summary(1000, 873)), (873, 1000, "87.3"))

    def test_the_percent_has_one_decimal_from_the_counts(self):
        self.assertEqual(line_coverage(summary(3, 2))[2], "66.7")
        self.assertEqual(line_coverage(summary(8, 8))[2], "100.0")

    def test_a_summary_of_another_kind_is_refused(self):
        other = summary(10, 5)
        other["type"] = "something.else"
        with self.assertRaisesRegex(Refused, "llvm.coverage.json.export"):
            line_coverage(other)

    def test_a_summary_without_one_set_of_totals_is_refused(self):
        empty = summary(10, 5)
        empty["data"] = []
        with self.assertRaisesRegex(Refused, "one"):
            line_coverage(empty)

    def test_no_lines_counted_is_refused(self):
        with self.assertRaisesRegex(Refused, "no lines"):
            line_coverage(summary(0, 0))

    def test_more_covered_than_counted_is_refused(self):
        with self.assertRaisesRegex(Refused, "more"):
            line_coverage(summary(10, 11))


class Write(unittest.TestCase):
    def test_the_data_file_holds_each_number_and_where_it_came_from(self):
        with tempfile.TemporaryDirectory() as d:
            path = pathlib.Path(d) / "data" / "metrics.toml"
            published_metrics.write(path, 137, (873, 1000, "87.3"), "a" * 40, "https://github.com/NormB/vcon-server/actions/runs/1")
            self.assertEqual(
                path.read_text(),
                'automated_tests = 137\n'
                'lines_covered = 873\n'
                'lines_counted = 1000\n'
                'line_coverage = "87.3"\n'
                f'commit = "{"a" * 40}"\n'
                'run = "https://github.com/NormB/vcon-server/actions/runs/1"\n',
            )

    def test_the_command_writes_nothing_when_a_run_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            (d / "tests.txt").write_text("error: could not compile\n")
            (d / "cov.json").write_text(__import__("json").dumps(summary(10, 5)))
            out = d / "metrics.toml"
            rc = published_metrics.main([
                "--tests", str(d / "tests.txt"), "--coverage", str(d / "cov.json"),
                "--commit", "b" * 40, "--run", "https://example.com/r", "--write", str(out),
            ])
            self.assertEqual(rc, 1)
            self.assertFalse(out.exists())

    def test_the_command_writes_the_file_from_the_two_runs(self):
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            (d / "tests.txt").write_text(PASSED)
            (d / "cov.json").write_text(__import__("json").dumps(summary(1000, 873)))
            out = d / "metrics.toml"
            rc = published_metrics.main([
                "--tests", str(d / "tests.txt"), "--coverage", str(d / "cov.json"),
                "--commit", "c" * 40, "--run", "https://example.com/r", "--write", str(out),
            ])
            self.assertEqual(rc, 0)
            self.assertIn("automated_tests = 137\n", out.read_text())
            self.assertIn('line_coverage = "87.3"\n', out.read_text())

    def test_the_coverage_summary_alone_is_checked(self):
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            (d / "cov.json").write_text(__import__("json").dumps(summary(0, 0)))
            self.assertEqual(published_metrics.main(["--coverage", str(d / "cov.json")]), 1)
            (d / "cov.json").write_text(__import__("json").dumps(summary(4, 3)))
            self.assertEqual(published_metrics.main(["--coverage", str(d / "cov.json")]), 0)

    def test_a_command_with_neither_run_is_refused(self):
        self.assertEqual(published_metrics.main([]), 1)

    def test_a_commit_that_is_not_a_full_sha_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaisesRegex(Refused, "commit"):
                published_metrics.write(pathlib.Path(d) / "m.toml", 1, (1, 1, "100.0"), "main", "https://example.com/r")


if __name__ == "__main__":
    unittest.main()
