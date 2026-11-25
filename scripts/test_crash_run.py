"""Tests scripts/crash_run.py: the process-crash harness that kills a server
under writers and readers and checks that nothing answered was lost.

The kill and the server are not driven here (they need a release build
and take minutes; the harness itself is their test, run as
website/content/docs/durability.md says). What is driven is every rule the
harness judges by: which body a vCon may have after a kill, which audit
records must be there, how `check`'s output is read, and how the server's
port and an ETag are parsed, each with a pass, a failure and an edge.

Run with: python3 -I scripts/test_crash_run.py
"""

import importlib.util
import pathlib
import sys
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("crash_run", HERE / "crash_run.py")
crash_run = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(crash_run)

U = "01a07da8-c2bb-83e5-b9a2-000000000001"


def ledger_with(base=None, pending=()):
    """A ledger whose uuid U was last answered as `base` (digest, version)
    and has `pending` digests sent since."""
    ledger = crash_run.Ledger()
    if base is not None:
        ledger.answered(U, base[0], base[1], "create")
    for d in pending:
        ledger.sent(U, d)
    return ledger


class Current(unittest.TestCase):
    """judge_current: the body a vCon may have after a kill."""

    def test_the_last_answered_body_passes(self):
        ledger = ledger_with(("d1", "v1"))
        self.assertIsNone(crash_run.judge_current(ledger, U, 200, "d1", "v1"))

    def test_a_body_sent_after_the_last_answer_passes_and_becomes_the_base(self):
        ledger = ledger_with(("d1", "v1"), ["d2"])
        self.assertIsNone(crash_run.judge_current(ledger, U, 200, "d2", "v2"))
        self.assertEqual(ledger.base[U], ("d2", "v2"), "the committed write is the new base")
        self.assertEqual(ledger.versions[U]["v2"], "replace", "its record is expected")

    def test_an_answered_write_found_missing_fails(self):
        ledger = ledger_with(("d1", "v1"))
        problem = crash_run.judge_current(ledger, U, 404, "", None)
        self.assertIn("answered write lost", problem or "", "a 404 after an answer")

    def test_an_older_body_than_the_last_answered_fails(self):
        ledger = ledger_with(("d1", "v1"))
        ledger.answered(U, "d2", "v2", "replace")
        problem = crash_run.judge_current(ledger, U, 200, "d1", "v1")
        self.assertIn("neither the last answered", problem or "", "the body went back")

    def test_a_body_no_client_sent_fails(self):
        ledger = ledger_with(None, ["d1"])
        problem = crash_run.judge_current(ledger, U, 200, "dX", "vX")
        self.assertIn("no client sent", problem or "", "an unknown body")

    def test_a_status_other_than_200_or_404_fails(self):
        ledger = ledger_with(("d1", "v1"))
        problem = crash_run.judge_current(ledger, U, 500, "", None)
        self.assertIn("GET answered 500", problem or "", "a server error")

    def test_a_create_never_answered_may_be_absent(self):
        ledger = ledger_with(None, ["d1"])
        self.assertIsNone(crash_run.judge_current(ledger, U, 404, "", None))

    def test_a_create_never_answered_may_be_there(self):
        ledger = ledger_with(None, ["d1"])
        self.assertIsNone(crash_run.judge_current(ledger, U, 200, "d1", "v1"))
        self.assertEqual(ledger.versions[U]["v1"], "create", "a create's record is expected")

    def test_the_same_body_under_another_version_fails(self):
        ledger = ledger_with(("d1", "v1"))
        problem = crash_run.judge_current(ledger, U, 200, "d1", "v9")
        self.assertIsNotNone(problem, "the version must be the answered one too")

    def test_once_seen_a_body_never_goes_back(self):
        ledger = ledger_with(("d1", "v1"), ["d2"])
        self.assertIsNone(crash_run.judge_current(ledger, U, 200, "d2", "v2"))
        problem = crash_run.judge_current(ledger, U, 200, "d1", "v1")
        self.assertIsNotNone(problem, "after d2 was seen, d1 is a loss")

    def test_an_unknown_uuid_absent_passes(self):
        ledger = crash_run.Ledger()
        self.assertIsNone(crash_run.judge_current(ledger, U, 404, "", None))

    def test_an_answer_clears_what_was_pending(self):
        ledger = ledger_with(("d1", "v1"), ["d2"])
        ledger.answered(U, "d3", "v3", "replace")
        problem = crash_run.judge_current(ledger, U, 200, "d2", "v2")
        self.assertIsNotNone(problem, "d2 was sent before the answer of d3")


def record(action, target, version, status, count=1, seq=1):
    return {"seq": seq, "action": action, "target": target, "version": version,
            "status": status, "count": count}


class Audit(unittest.TestCase):
    """judge_audit: every version answered or seen has its record; every
    answered read has one."""

    def test_every_write_with_its_record_passes(self):
        ledger = ledger_with(("d1", "v1"))
        ledger.answered(U, "d2", "v2", "replace")
        records = [record("create", U, "v1", 201), record("replace", U, "v2", 200)]
        self.assertEqual(crash_run.judge_audit(ledger, records), [])

    def test_a_write_without_its_record_fails(self):
        ledger = ledger_with(("d1", "v1"))
        ledger.answered(U, "d2", "v2", "replace")
        problems = crash_run.judge_audit(ledger, [record("create", U, "v1", 201)])
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("no replace record for version v2", problems[0])

    def test_a_record_of_a_refusal_does_not_count(self):
        ledger = ledger_with(("d1", "v1"))
        problems = crash_run.judge_audit(ledger, [record("create", U, "v1", 409)])
        self.assertEqual(len(problems), 1, "a refused create is not the answered one")

    def test_a_record_under_another_action_does_not_count(self):
        ledger = ledger_with(("d1", "v1"))
        problems = crash_run.judge_audit(ledger, [record("replace", U, "v1", 200)])
        self.assertEqual(len(problems), 1, "a create needs a create record")

    def test_reads_answered_need_as_many_read_records(self):
        ledger = ledger_with(("d1", "v1"))
        ledger.read(U)
        ledger.read(U)
        records = [record("create", U, "v1", 201), record("read", U, None, 200, count=1)]
        problems = crash_run.judge_audit(ledger, records)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("2 reads answered, 1 read records", problems[0])

    def test_more_read_records_than_answers_pass(self):
        ledger = ledger_with(("d1", "v1"))
        ledger.read(U)
        records = [record("create", U, "v1", 201)] + [record("read", U, None, 200)] * 3
        self.assertEqual(crash_run.judge_audit(ledger, records), [],
                         "a read answered as the kill came is recorded and not counted")

    def test_a_read_record_counts_its_count(self):
        ledger = ledger_with(("d1", "v1"))
        for _ in range(3):
            ledger.read(U)
        records = [record("create", U, "v1", 201), record("read", U, None, 200, count=3)]
        self.assertEqual(crash_run.judge_audit(ledger, records), [])

    def test_a_refused_read_record_does_not_count(self):
        ledger = ledger_with(("d1", "v1"))
        ledger.read(U)
        records = [record("create", U, "v1", 201), record("read", U, None, 404)]
        self.assertEqual(len(crash_run.judge_audit(ledger, records)), 1)

    def test_an_empty_ledger_passes_any_log(self):
        self.assertEqual(crash_run.judge_audit(crash_run.Ledger(), [record("read", U, None, 200)]),
                         [])

    def test_a_seen_write_in_flight_needs_its_record(self):
        ledger = ledger_with(("d1", "v1"), ["d2"])
        crash_run.judge_current(ledger, U, 200, "d2", "v2")
        problems = crash_run.judge_audit(ledger, [record("create", U, "v1", 201)])
        self.assertEqual(len(problems), 1, "a committed change has its record in the commit")


class Check(unittest.TestCase):
    """judge_check: `vcon-server check` passes on exit 0 and no problem."""

    def test_exit_0_and_no_problem_passes(self):
        out = '{"name":"crash","missing":0}\n{"problems":0}\n'
        self.assertIsNone(crash_run.judge_check(0, out))

    def test_exit_1_fails(self):
        out = '{"name":"crash","missing":1}\n{"problems":1}\n'
        self.assertIn("exited 1", crash_run.judge_check(1, out) or "")

    def test_a_summary_with_problems_fails_even_on_exit_0(self):
        self.assertIn("reported", crash_run.judge_check(0, '{"problems":2}\n') or "")

    def test_no_summary_fails(self):
        self.assertIsNotNone(crash_run.judge_check(0, "not json\n"), "no summary line")

    def test_a_tenant_line_is_not_the_summary(self):
        out = '{"name":"crash","problems":0}\n'
        self.assertIsNotNone(crash_run.judge_check(0, out), "a tenant's line has a name")

    def test_log_lines_around_the_report_are_ignored(self):
        out = 'INFO something\n{"name":"crash"}\n\n{"problems":0}\nINFO done\n'
        self.assertIsNone(crash_run.judge_check(0, out))


class Parsing(unittest.TestCase):
    def test_the_serving_line_gives_the_port(self):
        log = "x INFO vcon_server: serving listen=127.0.0.1:43211\n"
        self.assertEqual(crash_run.parse_port(log), 43211)

    def test_no_serving_line_gives_none(self):
        self.assertIsNone(crash_run.parse_port("INFO starting\n"))

    def test_a_serving_line_on_another_address_gives_none(self):
        log = "x INFO vcon_server: serving listen=0.0.0.0:443\n"
        self.assertIsNone(crash_run.parse_port(log), "only the loopback listener is ours")

    def test_a_quoted_etag_gives_its_version(self):
        self.assertEqual(crash_run.version_of('"0199e5c6"'), "0199e5c6")

    def test_an_unquoted_or_empty_etag_gives_none(self):
        for tag in (None, "", '""', "0199e5c6", '"0199e5c6'):
            self.assertIsNone(crash_run.version_of(tag), repr(tag))

    def test_a_term_ends_the_run_through_system_exit(self):
        import signal

        with self.assertRaises(SystemExit) as stopped:
            crash_run.stop_on_signal(signal.SIGTERM, None)
        self.assertEqual(stopped.exception.code, 143, "so `finally` kills the server and cleans up")

    def test_a_weak_etag_gives_none(self):
        self.assertIsNone(crash_run.version_of('W/"0199e5c6"'))


class Clients(unittest.TestCase):
    """The writer and reader loops, through fake connections."""

    class FakeWriters:
        @staticmethod
        def body(uuid, n):
            return f"{uuid}:{n}".encode()

    def writer(self, answers):
        import random
        import threading

        ledger = crash_run.Ledger()
        stop = threading.Event()
        sent = []

        def send(method, path, data, headers):
            sent.append((method, path, headers.get("If-Match")))
            answer = answers.pop(0)
            if isinstance(answer, Exception):
                raise answer
            if not answers:
                stop.set()
            return answer

        serial = iter(range(100))
        w = crash_run.Writer(self.FakeWriters, send, "t", ledger, random.Random(1), stop, {},
                             lambda: f"01a07da8-c2bb-83e5-b9a2-{next(serial):012x}")
        w.run()
        return w, ledger, sent

    def test_an_answered_create_is_in_the_ledger(self):
        w, ledger, sent = self.writer([(201, '"v1"')])
        self.assertEqual(w.answered, 1)
        self.assertEqual(sent[0][0], "POST", "the first write is a create")
        (uuid,) = ledger.base
        self.assertEqual(ledger.base[uuid][1], "v1")

    def test_a_replace_names_the_version_answered(self):
        w, _, sent = self.writer([(201, '"v1"')] + [(200, '"v2"')] * 5)
        replaces = [s for s in sent if s[0] == "PUT"]
        self.assertTrue(replaces, "some write is a replace")
        self.assertIn(replaces[0][2], ('"v1"', '"v2"'), "If-Match is an answered version")

    def test_a_connection_error_ends_the_writer_with_its_write_pending(self):
        w, ledger, _ = self.writer([ConnectionResetError("killed")])
        self.assertEqual(w.answered, 0)
        self.assertEqual(ledger.base, {}, "nothing answered")
        self.assertEqual(len(ledger.pending), 1, "the write in flight is pending")

    def test_an_unexpected_status_ends_the_writer_unanswered(self):
        w, ledger, _ = self.writer([(500, None)])
        self.assertEqual(w.answered, 0)
        self.assertEqual(ledger.base, {})

    def test_a_refusal_with_an_etag_is_not_answered(self):
        w, ledger, _ = self.writer([(409, '"v9"')])
        self.assertEqual(w.answered, 0, "a 409 is not the 201 a create wants")
        self.assertEqual(ledger.base, {}, "nothing answered")


if __name__ == "__main__":
    unittest.main()
