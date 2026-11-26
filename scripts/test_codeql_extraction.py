"""Tests scripts/codeql_extraction.py: CI fails when the CodeQL Rust
analysis extracts more files with errors than recorded (or fewer, until the
record is lowered), and the CodeQL workflow runs that check after the Rust
analysis, with the database and the CodeQL the analysis used.

`codeql bqrs decode` is replaced by a stand-in runner here: the commit hook
has no CodeQL bundle (it is a 500 MB download that only the CodeQL job
makes), so these cases test the decoding of its JSON and the rule; the
real decode runs in CI's CodeQL job on every push and pull request.

Run with: python3 -I scripts/test_codeql_extraction.py
"""

import importlib.util
import io
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("codeql_extraction", HERE / "codeql_extraction.py")
ex = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ex)

WORKFLOW = HERE.parent / ".github/workflows/codeql.yml"


def metric(n):
    return {"#select": {"columns": [{"kind": "Integer"}], "tuples": [[n]]}}


ERRORS = {
    "#select": {
        "columns": [{"kind": "Entity"}, {"kind": "String"}],
        "tuples": [[{"label": "listen.rs", "url": {"uri": "file:///w/crates/vcon-server/src/api/listen.rs", "startLine": 186}}, "macro expansion failed"]],
    }
}


class Runner:
    """Stands in for `codeql bqrs decode`: answers by the file's name."""

    def __init__(self, answers, code=0, stderr=""):
        self.answers, self.code, self.stderr, self.calls = answers, code, stderr, []

    def __call__(self, args, **kwargs):
        self.calls.append((args, kwargs))
        name = pathlib.Path(args[-1]).name
        answer = self.answers.get(name)
        stdout = answer if isinstance(answer, str) else json.dumps(answer)
        return subprocess.CompletedProcess(args, self.code, stdout=stdout, stderr=self.stderr)


def database(*names):
    """A database directory whose results hold `names`."""
    tmp = tempfile.TemporaryDirectory()
    root = pathlib.Path(tmp.name)
    for name in names:
        path = root / "results" / "codeql" / "rust-queries" / "queries" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"bqrs")
    return tmp, root


SUMMARY = "summary/NumberOfFilesExtractedWithErrors.bqrs"
DIAGNOSTICS = "diagnostics/ExtractionErrors.bqrs"


class Count(unittest.TestCase):
    def test_a_metric_of_zero_is_zero(self):
        self.assertEqual(ex.count(metric(0)), 0, "no file with errors")

    def test_a_metric_of_three_is_three(self):
        self.assertEqual(ex.count(metric(3)), 3, "three files")

    def test_a_large_count_is_read(self):
        self.assertEqual(ex.count(metric(10**6)), 10**6, "any count")

    def test_no_result_set_is_refused(self):
        with self.assertRaises(ValueError, msg="no #select"):
            ex.count({})

    def test_two_rows_are_refused(self):
        with self.assertRaises(ValueError, msg="a metric is one row"):
            ex.count({"#select": {"tuples": [[1], [2]]}})

    def test_no_row_is_refused(self):
        with self.assertRaises(ValueError, msg="an empty result is no count"):
            ex.count({"#select": {"tuples": []}})

    def test_two_columns_are_refused(self):
        with self.assertRaises(ValueError, msg="a metric is one column"):
            ex.count({"#select": {"tuples": [[1, 2]]}})

    def test_a_text_is_refused(self):
        with self.assertRaises(ValueError, msg="a count is a number"):
            ex.count(metric("1"))

    def test_a_negative_number_is_refused(self):
        with self.assertRaises(ValueError, msg="no count is negative"):
            ex.count(metric(-1))

    def test_a_boolean_is_refused(self):
        with self.assertRaises(ValueError, msg="True is no count"):
            ex.count(metric(True))

    def test_a_list_is_refused(self):
        with self.assertRaises(ValueError, msg="not an object"):
            ex.count([[1]])


class Judge(unittest.TestCase):
    def test_the_recorded_number_passes(self):
        self.assertIsNone(ex.judge(0, recorded=0), "as recorded")
        self.assertIsNone(ex.judge(2, recorded=2), "as recorded")

    def test_a_rise_fails_and_says_why_it_matters(self):
        failure = ex.judge(1, recorded=0)
        self.assertIn("more than the 0 recorded", failure, failure)
        self.assertIn("reports no finding", failure, failure)

    def test_a_fall_fails_and_names_the_new_record(self):
        failure = ex.judge(0, recorded=1)
        self.assertIn("set RECORDED to 0", failure, failure)

    def test_the_record_is_zero(self):
        self.assertEqual(ex.RECORDED, 0, "every Rust file is extracted whole since the listen.rs change")


class Find(unittest.TestCase):
    def test_the_one_file_is_found_anywhere_under_results(self):
        tmp, root = database(SUMMARY)
        with tmp:
            self.assertEqual(ex.find(root, ex.COUNT_FILE).name, ex.COUNT_FILE, "found")

    def test_no_file_is_refused_with_the_reason(self):
        tmp, root = database()
        with tmp:
            with self.assertRaisesRegex(ValueError, "did not run the summary queries"):
                ex.find(root, ex.COUNT_FILE)

    def test_two_files_are_refused(self):
        tmp, root = database(SUMMARY, "other/" + ex.COUNT_FILE)
        with tmp:
            with self.assertRaisesRegex(ValueError, "2 files"):
                ex.find(root, ex.COUNT_FILE)


class Check(unittest.TestCase):
    def test_a_database_at_the_record_passes(self):
        tmp, root = database(SUMMARY, DIAGNOSTICS)
        with tmp:
            code, lines = ex.check("codeql", root, run=Runner({ex.COUNT_FILE: metric(ex.RECORDED)}))
        self.assertEqual(code, 0, lines)
        self.assertIn("as recorded", lines[0], lines)

    def test_a_rise_fails_and_lists_the_errors(self):
        tmp, root = database(SUMMARY, DIAGNOSTICS)
        with tmp:
            code, lines = ex.check("codeql", root, run=Runner({ex.COUNT_FILE: metric(ex.RECORDED + 1), ex.ERRORS_FILE: ERRORS}))
        self.assertEqual(code, 1, lines)
        self.assertTrue(any("listen.rs" in line and "macro expansion failed" in line for line in lines), lines)

    def test_a_rise_without_the_error_list_still_fails(self):
        tmp, root = database(SUMMARY)
        with tmp:
            code, lines = ex.check("codeql", root, run=Runner({ex.COUNT_FILE: metric(ex.RECORDED + 1)}))
        self.assertEqual(code, 1, lines)
        self.assertTrue(any("cannot be listed" in line for line in lines), lines)

    def test_a_failed_decode_fails_with_its_text(self):
        tmp, root = database(SUMMARY)
        with tmp:
            code, lines = ex.check("codeql", root, run=Runner({}, code=2, stderr="A fatal error occurred"))
        self.assertEqual(code, 1, lines)
        self.assertTrue(any("exit 2" in line and "A fatal error" in line for line in lines), lines)

    def test_a_decode_that_is_not_json_fails(self):
        tmp, root = database(SUMMARY)
        with tmp:
            code, lines = ex.check("codeql", root, run=Runner({ex.COUNT_FILE: "not json"}))
        self.assertEqual(code, 1, lines)
        self.assertTrue(any("no JSON" in line for line in lines), lines)

    def test_a_database_without_the_summary_fails(self):
        tmp, root = database()
        with tmp:
            code, lines = ex.check("codeql", root, run=Runner({}))
        self.assertEqual(code, 1, lines)

    def test_a_missing_codeql_fails_and_does_not_crash(self):
        tmp, root = database(SUMMARY)
        with tmp:
            code, lines = ex.check("/nonexistent/codeql", root)
        self.assertEqual(code, 1, lines)

    def test_the_decode_has_a_deadline_and_the_json_format(self):
        tmp, root = database(SUMMARY)
        runner = Runner({ex.COUNT_FILE: metric(ex.RECORDED)})
        with tmp:
            ex.check("/opt/codeql/codeql", root, run=runner)
        args, kwargs = runner.calls[0]
        self.assertEqual(args[:4], ["/opt/codeql/codeql", "bqrs", "decode", "--format=json"], args)
        self.assertEqual(kwargs.get("timeout"), ex.DECODE_DEADLINE, kwargs)

    def test_a_decode_past_its_deadline_fails(self):
        def late(args, **kwargs):
            raise subprocess.TimeoutExpired(args, kwargs.get("timeout"))

        tmp, root = database(SUMMARY)
        with tmp:
            code, lines = ex.check("codeql", root, run=late)
        self.assertEqual(code, 1, lines)

    def test_an_error_row_with_plain_text_is_printed_as_it_is(self):
        self.assertEqual(ex.error_lines({"#select": {"tuples": [["a.rs", "failed"]]}}), ["a.rs | failed"], "text cells")

    def test_an_error_result_that_is_no_result_set_is_said(self):
        self.assertIn("not a result set", ex.error_lines({})[0], "named")


# The metric table's line as the Analyze step printed it on main's run
# 45841313480 (22d356e, 2026-11-26), and the same line for a clean run.
MAIN_LINE = "2026-11-26T14:48:37.9985964Z | Total number of Rust files that were extracted with errors   |     1 |"
CLEAN_LINE = "2026-11-26T14:48:37.9985964Z | Total number of Rust files that were extracted with errors   |     0 |"


class LogText(unittest.TestCase):
    """The same rule read from an Analyze step's log, for a run that has
    ended (`log`): main's run of 2026-11-26 is refused."""

    def test_the_line_of_main_on_2026_11_26_is_read_as_one(self):
        self.assertEqual(ex.count_in_log("a\n" + MAIN_LINE + "\nb\n"), 1, "one file")

    def test_main_on_2026_11_26_is_refused(self):
        tmp = tempfile.TemporaryDirectory()
        with tmp:
            path = pathlib.Path(tmp.name) / "job.log"
            path.write_text(MAIN_LINE + "\n", encoding="utf-8")
            err = io.StringIO()
            self.assertEqual(ex.main(["log", str(path)], out=io.StringIO(), err=err), 1, "a file extracted with errors fails")
            self.assertIn("more than the 0 recorded", err.getvalue(), err.getvalue())

    def test_a_clean_log_passes(self):
        tmp = tempfile.TemporaryDirectory()
        with tmp:
            path = pathlib.Path(tmp.name) / "job.log"
            path.write_text(CLEAN_LINE + "\n", encoding="utf-8")
            self.assertEqual(ex.main(["log", str(path)], out=io.StringIO(), err=io.StringIO()), 0, "no file with errors")

    def test_a_line_without_a_timestamp_is_read(self):
        self.assertEqual(ex.count_in_log("| Total number of Rust files that were extracted with errors | 2 |"), 2, "bare")

    def test_a_log_without_the_line_is_refused(self):
        with self.assertRaisesRegex(ValueError, "no line"):
            ex.count_in_log("| Total number of lines of Rust code | 900 |\n")

    def test_two_lines_are_refused(self):
        with self.assertRaisesRegex(ValueError, "2 lines"):
            ex.count_in_log(MAIN_LINE + "\n" + CLEAN_LINE + "\n")

    def test_a_count_that_is_no_number_is_refused(self):
        with self.assertRaises(ValueError, msg="no number"):
            ex.count_in_log("| Total number of Rust files that were extracted with errors | x |")

    def test_a_missing_log_file_fails_and_does_not_crash(self):
        self.assertEqual(ex.main(["log", "/nonexistent/job.log"], out=io.StringIO(), err=io.StringIO()), 1, "unreadable")


class Main(unittest.TestCase):
    def test_the_arguments_in_either_order(self):
        self.assertEqual(ex.parse(["check", "--codeql", "c", "--database", "d"]), ("c", "d"), "in order")
        self.assertEqual(ex.parse(["check", "--database", "d", "--codeql", "c"]), ("c", "d"), "swapped")

    def test_wrong_arguments_print_the_usage_and_exit_2(self):
        for argv in ([], ["check"], ["run", "--codeql", "c", "--database", "d"], ["check", "--codeql", "c", "--db", "d"], ["check", "--codeql", "", "--database", "d"]):
            err = io.StringIO()
            self.assertEqual(ex.main(argv, out=io.StringIO(), err=err), 2, argv)
            self.assertIn("usage", err.getvalue(), argv)

    def test_a_pass_prints_on_stdout_and_a_failure_on_stderr(self):
        tmp, root = database(SUMMARY)
        with tmp:
            out, err = io.StringIO(), io.StringIO()
            self.assertEqual(ex.main(["check", "--codeql", "c", "--database", str(root)], run=Runner({ex.COUNT_FILE: metric(ex.RECORDED)}), out=out, err=err), 0)
            self.assertIn("ok", out.getvalue(), "the pass")
            out, err = io.StringIO(), io.StringIO()
            self.assertEqual(ex.main(["check", "--codeql", "c", "--database", str(root)], run=Runner({ex.COUNT_FILE: metric(ex.RECORDED + 1)}), out=out, err=err), 1)
            self.assertIn("more than", err.getvalue(), "the failure")


class Workflow(unittest.TestCase):
    """The CodeQL workflow runs the check after the Rust analysis."""

    @classmethod
    def setUpClass(cls):
        import yaml

        cls.steps = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))["jobs"]["analyze"]["steps"]

    def index(self, predicate):
        found = [i for i, step in enumerate(self.steps) if predicate(step)]
        self.assertEqual(len(found), 1, f"one such step: {found}")
        return found[0]

    def test_a_step_runs_the_check_after_the_analysis(self):
        analyze = self.index(lambda s: str(s.get("uses", "")).startswith("github/codeql-action/analyze@"))
        check = self.index(lambda s: "scripts/codeql_extraction.py check" in str(s.get("run", "")))
        self.assertGreater(check, analyze, "the database is complete only after the analysis")

    def test_the_check_runs_for_rust_only(self):
        step = self.steps[self.index(lambda s: "scripts/codeql_extraction.py check" in str(s.get("run", "")))]
        self.assertEqual(step.get("if"), "matrix.language == 'rust'", "the summary query is the Rust one")

    def test_the_check_reads_the_codeql_and_the_database_of_the_analysis(self):
        step = self.steps[self.index(lambda s: "scripts/codeql_extraction.py check" in str(s.get("run", "")))]
        init = self.steps[self.index(lambda s: str(s.get("uses", "")).startswith("github/codeql-action/init@"))]
        env = step.get("env") or {}
        self.assertEqual(env.get("CODEQL"), "${{ steps.%s.outputs.codeql-path }}" % init.get("id"), env)
        self.assertEqual(env.get("DATABASE"), "${{ runner.temp }}/codeql_databases/rust", env)
        self.assertIn('--codeql "$CODEQL" --database "$DATABASE"', step["run"], step["run"])


if __name__ == "__main__":
    unittest.main()
