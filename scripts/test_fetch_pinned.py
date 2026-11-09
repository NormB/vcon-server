"""Tests scripts/fetch-pinned.sh: the one rule every pinned release download
in CI goes through.

On 2026-11-08, and again on 2026-11-09 at about 05:15Z, github.com answered
release downloads with HTTP 504. main's CI on 2cb225a30 failed at every job
that fetched cargo-nextest, DuckDB, cargo-fuzz or a vcon-server release,
each with one curl and about 25 s of retries, while the code was green.

fetch-pinned.sh verifies a file already at its destination (a restored
cache entry) before it trusts it, downloads on a miss or a mismatch,
retries a transient failure with exponential backoff inside a time budget,
and exits with a status that names the kind of failure:

    0  the destination holds the pinned bytes
    1  every attempt failed transiently: a re-run may pass
    2  the arguments are wrong
    3  the bytes served do not match the pin: do not re-run
    4  a permanent refusal (HTTP 404, a missing file, a bad URL): fix the pin

These tests drive it against a local HTTP server that answers from a list
of responses (504s, stalls, truncated bodies, redirects, closed
connections) and against file:// fixtures. Nothing leaves the machine.

Run with: python3 -I scripts/test_fetch_pinned.py
"""

import hashlib
import http.server
import os
import pathlib
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "fetch-pinned.sh"

PAYLOAD = b"pinned archive bytes\n" * 64
SHA = hashlib.sha256(PAYLOAD).hexdigest()
EMPTY_SHA = hashlib.sha256(b"").hexdigest()
OTHER = b"substituted bytes\n" * 64

# Short delays, so a test that retries ends in well under a second; the
# budget is wide, so only the server's list decides how many attempts run.
FAST = [
    "--first-delay", "0.01",
    "--max-delay", "0.05",
    "--budget", "20",
    "--attempt-timeout", "5",
    "--connect-timeout", "2",
]


class Server:
    """A local HTTP server that answers each request from a list.

    Each response is a dict: `status` (default 200), `body` (default
    PAYLOAD), `stall` (seconds to wait first), `truncate` (declare a longer
    Content-Length than the body, then close), `close` (close with no
    answer) and `location` (a redirect target). When the list runs out,
    its last response repeats. A path with no list answers 404.
    """

    def __init__(self):
        self.plans = {}
        self.requests = []
        self.lock = threading.Lock()
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.0"

            def log_message(self, *args):
                pass

            def do_GET(self):
                with outer.lock:
                    outer.requests.append({"path": self.path, "ua": self.headers.get("User-Agent", "")})
                    plan = outer.plans.get(self.path, [{"status": 404, "body": b"no plan"}])
                    r = plan.pop(0) if len(plan) > 1 else plan[0]
                if r.get("stall"):
                    time.sleep(r["stall"])
                if r.get("close"):
                    self.close_connection = True
                    try:
                        self.connection.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        pass
                    return
                body = r.get("body", PAYLOAD)
                try:
                    self.send_response(r.get("status", 200))
                    if "location" in r:
                        self.send_header("Location", r["location"])
                    declared = len(body) + 4096 if r.get("truncate") else len(body)
                    self.send_header("Content-Length", str(declared))
                    self.end_headers()
                    self.wfile.write(body)
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass

        self.httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.httpd.daemon_threads = True
        self.thread = threading.Thread(target=self.httpd.serve_forever, kwargs={"poll_interval": 0.05}, daemon=True)
        self.thread.start()

    def url(self, path):
        return f"http://127.0.0.1:{self.httpd.server_address[1]}{path}"

    def plan(self, path, *responses):
        self.plans[path] = [dict(r) for r in responses]
        return self.url(path)

    def hits(self, path):
        with self.lock:
            return sum(1 for r in self.requests if r["path"] == path)

    def stop(self):
        self.httpd.shutdown()
        self.httpd.server_close()


def run(*args, timeout=60):
    return subprocess.run(["bash", str(SCRIPT), *args], capture_output=True, text=True, timeout=timeout)


def free_port():
    """A port nothing listens on: bound, read, and closed again."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Case(unittest.TestCase):
    """A server and an empty directory for each test."""

    def setUp(self):
        self.server = Server()
        self.dir = pathlib.Path(tempfile.mkdtemp(prefix="fetch-pinned-test-"))
        self.dest = self.dir / "tool" / "tool.tar.gz"

    def tearDown(self):
        self.server.stop()
        shutil.rmtree(self.dir, ignore_errors=True)

    def fetch(self, url, sha=SHA, dest=None, extra=FAST):
        return run("--url", url, "--sha256", sha, "--dest", str(dest or self.dest), *extra)

    def assertPinned(self, r, dest=None):
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual((dest or self.dest).read_bytes(), PAYLOAD)

    def leftovers(self):
        """Files in the destination's directory other than the destination."""
        parent = self.dest.parent
        if not parent.exists():
            return []
        return sorted(p.name for p in parent.iterdir() if p != self.dest)


# ---------------------------------------------------------------------------
# SUCCESS: the destination ends up holding the pinned bytes.
# ---------------------------------------------------------------------------


class Success(Case):
    def test_a_plain_download_lands_at_the_destination(self):
        r = self.fetch(self.server.plan("/a", {}))
        self.assertPinned(r)
        self.assertEqual(self.server.hits("/a"), 1)

    def test_the_success_line_names_the_url_and_says_the_hash_was_checked(self):
        url = self.server.plan("/a", {})
        r = self.fetch(url)
        self.assertIn(url, r.stdout)
        self.assertIn("sha256 verified", r.stdout)

    def test_a_destination_already_holding_the_pinned_bytes_fetches_nothing(self):
        self.dest.parent.mkdir(parents=True)
        self.dest.write_bytes(PAYLOAD)
        r = self.fetch(self.server.plan("/a", {}))
        self.assertPinned(r)
        self.assertEqual(self.server.hits("/a"), 0)
        self.assertIn("restored from the cache", r.stdout)

    def test_three_504s_then_a_200_succeed_on_the_fourth_attempt(self):
        url = self.server.plan("/a", {"status": 504}, {"status": 504}, {"status": 504}, {})
        r = self.fetch(url)
        self.assertPinned(r)
        self.assertEqual(self.server.hits("/a"), 4)
        self.assertIn("attempt 4", r.stdout)

    def test_a_429_then_a_200_succeed(self):
        r = self.fetch(self.server.plan("/a", {"status": 429}, {}))
        self.assertPinned(r)
        self.assertEqual(self.server.hits("/a"), 2)

    def test_a_408_then_a_200_succeed(self):
        r = self.fetch(self.server.plan("/a", {"status": 408}, {}))
        self.assertPinned(r)

    def test_a_500_502_and_503_then_a_200_succeed(self):
        r = self.fetch(self.server.plan("/a", {"status": 500}, {"status": 502}, {"status": 503}, {}))
        self.assertPinned(r)
        self.assertEqual(self.server.hits("/a"), 4)

    def test_a_stalled_attempt_times_out_and_the_next_succeeds(self):
        url = self.server.plan("/a", {"stall": 3}, {})
        r = self.fetch(url, extra=[*FAST, "--attempt-timeout", "1"])
        self.assertPinned(r)
        self.assertIn("timed out", r.stderr)

    def test_a_transfer_cut_short_is_retried(self):
        r = self.fetch(self.server.plan("/a", {"truncate": True}, {}))
        self.assertPinned(r)
        self.assertEqual(self.server.hits("/a"), 2)

    def test_a_connection_closed_with_no_answer_is_retried(self):
        r = self.fetch(self.server.plan("/a", {"close": True}, {}))
        self.assertPinned(r)

    def test_a_redirect_is_followed(self):
        target = self.server.plan("/real", {})
        r = self.fetch(self.server.plan("/a", {"status": 302, "body": b"", "location": target}))
        self.assertPinned(r)
        self.assertEqual(self.server.hits("/real"), 1)

    def test_a_file_url_is_fetched(self):
        src = self.dir / "src.tar.gz"
        src.write_bytes(PAYLOAD)
        self.assertPinned(self.fetch(src.as_uri()))

    def test_missing_parent_directories_of_the_destination_are_made(self):
        dest = self.dir / "a" / "b" / "c" / "tool.tar.gz"
        self.assertPinned(self.fetch(self.server.plan("/a", {}), dest=dest), dest)

    def test_the_downloaded_file_is_readable_by_all_and_not_executable(self):
        self.fetch(self.server.plan("/a", {}))
        self.assertEqual(self.dest.stat().st_mode & 0o777, 0o644)

    def test_the_default_schedule_is_doubling_delays_capped_at_60_within_300_s(self):
        r = run("--print-schedule")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.split(), ["5", "10", "20", "40", "60", "60", "60"])

    def test_no_temporary_file_is_left_after_a_download(self):
        self.fetch(self.server.plan("/a", {"status": 504}, {}))
        self.assertEqual(self.leftovers(), [])


# ---------------------------------------------------------------------------
# FAILURE: the right exit status, a message that names the cause, and no
# unverified bytes at the destination.
# ---------------------------------------------------------------------------


class Failure(Case):
    def test_an_outage_that_outlasts_the_budget_exits_1(self):
        url = self.server.plan("/a", {"status": 504})
        r = self.fetch(url, extra=[*FAST, "--budget", "0.2"])
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertFalse(self.dest.exists())

    def test_the_outage_message_names_the_url_the_last_status_and_a_re_run(self):
        url = self.server.plan("/a", {"status": 504})
        r = self.fetch(url, extra=[*FAST, "--budget", "0.2"])
        self.assertIn(url, r.stderr)
        self.assertIn("HTTP 504", r.stderr)
        self.assertIn("re-run", r.stderr)

    def test_a_404_exits_4_after_one_request(self):
        r = self.fetch(self.server.plan("/a", {"status": 404, "body": b"Not Found"}))
        self.assertEqual(r.returncode, 4, r.stderr)
        self.assertEqual(self.server.hits("/a"), 1)
        self.assertIn("HTTP 404", r.stderr)

    def test_a_403_exits_4_with_no_retry(self):
        r = self.fetch(self.server.plan("/a", {"status": 403, "body": b"Forbidden"}))
        self.assertEqual(r.returncode, 4, r.stderr)
        self.assertEqual(self.server.hits("/a"), 1)

    def test_substituted_bytes_exit_3_and_leave_no_file(self):
        r = self.fetch(self.server.plan("/a", {"body": OTHER}))
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertFalse(self.dest.exists())
        self.assertIn("do NOT re-run", r.stderr)

    def test_the_hash_message_names_both_hashes_and_the_size(self):
        r = self.fetch(self.server.plan("/a", {"body": OTHER}))
        self.assertIn(SHA, r.stderr)
        self.assertIn(hashlib.sha256(OTHER).hexdigest(), r.stderr)
        self.assertIn(f"{len(OTHER)} bytes", r.stderr)

    def test_a_truncated_archive_served_as_complete_exits_3(self):
        r = self.fetch(self.server.plan("/a", {"body": PAYLOAD[:100]}))
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertFalse(self.dest.exists())

    def test_a_transfer_cut_short_on_every_attempt_exits_1(self):
        r = self.fetch(self.server.plan("/a", {"truncate": True}), extra=[*FAST, "--budget", "0.2"])
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertFalse(self.dest.exists())

    def test_every_attempt_timing_out_exits_1_and_says_so(self):
        url = self.server.plan("/a", {"stall": 3})
        r = self.fetch(url, extra=[*FAST, "--attempt-timeout", "0.5", "--budget", "0.05"])
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn("timed out", r.stderr)

    def test_a_refused_connection_on_every_attempt_exits_1(self):
        url = f"http://127.0.0.1:{free_port()}/a"
        r = self.fetch(url, extra=[*FAST, "--budget", "0.1"])
        self.assertEqual(r.returncode, 1, r.stderr)

    def test_a_missing_file_url_exits_4(self):
        r = self.fetch((self.dir / "absent.tar.gz").as_uri())
        self.assertEqual(r.returncode, 4, r.stderr)

    def test_an_unsupported_scheme_exits_4(self):
        r = self.fetch("ftp://127.0.0.1/a")
        self.assertEqual(r.returncode, 4, r.stderr)

    def test_a_redirect_to_a_file_url_is_refused(self):
        src = self.dir / "src.tar.gz"
        src.write_bytes(PAYLOAD)
        r = self.fetch(self.server.plan("/a", {"status": 302, "body": b"", "location": src.as_uri()}))
        self.assertEqual(r.returncode, 4, r.stderr)
        self.assertFalse(self.dest.exists())

    def test_a_failed_download_leaves_no_temporary_file(self):
        self.fetch(self.server.plan("/a", {"body": OTHER}))
        self.assertEqual(self.leftovers(), [])


class Arguments(Case):
    """Exit 2, before any request, for arguments that cannot be right."""

    def assertUsage(self, r, words):
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn(words, r.stderr)
        self.assertEqual(self.server.requests, [])

    def test_an_upper_case_hash_exits_2(self):
        self.assertUsage(self.fetch(self.server.plan("/a", {}), sha=SHA.upper()), "lowercase hex")

    def test_a_short_hash_exits_2(self):
        self.assertUsage(self.fetch(self.server.plan("/a", {}), sha=SHA[:63]), "has 63")

    def test_a_missing_url_exits_2(self):
        self.assertUsage(run("--sha256", SHA, "--dest", str(self.dest)), "--url is required")

    def test_a_missing_hash_exits_2(self):
        self.assertUsage(run("--url", self.server.url("/a"), "--dest", str(self.dest)), "--sha256 is required")

    def test_a_missing_destination_exits_2(self):
        self.assertUsage(run("--url", self.server.url("/a"), "--sha256", SHA), "--dest is required")

    def test_an_unknown_option_exits_2(self):
        self.assertUsage(run("--retries", "9"), "unknown option --retries")

    def test_an_option_with_no_value_exits_2(self):
        self.assertUsage(run("--url"), "--url needs a value")

    def test_a_destination_that_is_a_directory_exits_2(self):
        self.assertUsage(self.fetch(self.server.plan("/a", {}), dest=self.dir), "is a directory")

    def test_a_budget_that_is_not_a_number_exits_2(self):
        self.assertUsage(self.fetch(self.server.plan("/a", {}), extra=["--budget", "5m"]), "not a number of seconds")

    def test_a_zero_first_delay_exits_2(self):
        self.assertUsage(self.fetch(self.server.plan("/a", {}), extra=["--first-delay", "0"]), "more than 0")


# ---------------------------------------------------------------------------
# EDGE: the cache, the budget's boundary, and the classification of a
# failure after a recovery.
# ---------------------------------------------------------------------------


class Edge(Case):
    def test_a_corrupted_cache_file_is_deleted_and_fetched_again(self):
        self.dest.parent.mkdir(parents=True)
        self.dest.write_bytes(PAYLOAD[:-1] + b"X")
        r = self.fetch(self.server.plan("/a", {}))
        self.assertPinned(r)
        self.assertEqual(self.server.hits("/a"), 1)

    def test_the_corrupted_cache_warning_names_both_hashes_and_the_cache_entry_fix(self):
        self.dest.parent.mkdir(parents=True)
        self.dest.write_bytes(OTHER)
        r = self.fetch(self.server.plan("/a", {}))
        self.assertIn(hashlib.sha256(OTHER).hexdigest(), r.stderr)
        self.assertIn("gh cache delete", r.stderr)

    def test_an_empty_cache_file_is_fetched_again(self):
        self.dest.parent.mkdir(parents=True)
        self.dest.write_bytes(b"")
        self.assertPinned(self.fetch(self.server.plan("/a", {})))

    def test_a_corrupted_cache_file_is_gone_even_when_the_fetch_then_fails(self):
        self.dest.parent.mkdir(parents=True)
        self.dest.write_bytes(OTHER)
        r = self.fetch(self.server.plan("/a", {"status": 404}))
        self.assertEqual(r.returncode, 4, r.stderr)
        self.assertFalse(self.dest.exists())

    def test_a_cache_hit_needs_no_network_at_all(self):
        self.dest.parent.mkdir(parents=True)
        self.dest.write_bytes(PAYLOAD)
        r = self.fetch(f"http://127.0.0.1:{free_port()}/a")
        self.assertPinned(r)

    def test_a_pinned_empty_file_is_fetched(self):
        r = self.fetch(self.server.plan("/a", {"body": b""}), sha=EMPTY_SHA)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.dest.read_bytes(), b"")

    def test_wrong_bytes_after_a_recovery_exit_3_not_1(self):
        r = self.fetch(self.server.plan("/a", {"status": 504}, {"status": 504}, {"body": OTHER}))
        self.assertEqual(r.returncode, 3, r.stderr)

    def test_a_404_after_504s_exits_4_not_1(self):
        r = self.fetch(self.server.plan("/a", {"status": 504}, {"status": 404}))
        self.assertEqual(r.returncode, 4, r.stderr)
        self.assertEqual(self.server.hits("/a"), 2)

    def test_a_budget_below_the_first_delay_allows_one_attempt_only(self):
        url = self.server.plan("/a", {"status": 504})
        r = self.fetch(url, extra=[*FAST, "--first-delay", "1", "--max-delay", "1", "--budget", "0.5"])
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertEqual(self.server.hits("/a"), 1)
        self.assertIn("after 1 attempt ", r.stderr)

    def test_a_first_delay_above_the_cap_is_capped(self):
        r = run("--print-schedule", "--first-delay", "90", "--max-delay", "60", "--budget", "200")
        self.assertEqual(r.stdout.split(), ["60", "60", "60"])

    def test_delays_whose_sum_equals_the_budget_are_all_allowed(self):
        r = run("--print-schedule", "--first-delay", "1", "--max-delay", "4", "--budget", "7")
        self.assertEqual(r.stdout.split(), ["1", "2", "4"])

    def test_fractional_delays_print_without_trailing_zeros(self):
        r = run("--print-schedule", "--first-delay", "0.01", "--max-delay", "0.05", "--budget", "0.1")
        self.assertEqual(r.stdout.split(), ["0.01", "0.02", "0.04"])

    def test_the_whole_run_stays_within_the_budget_plus_one_attempt(self):
        url = self.server.plan("/a", {"status": 504})
        start = time.monotonic()
        r = self.fetch(url, extra=["--first-delay", "0.2", "--max-delay", "0.4", "--budget", "1.7",
                                   "--attempt-timeout", "5", "--connect-timeout", "2"])
        took = time.monotonic() - start
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertLess(took, 1.7 + 5)
        # The delays are 0.2, 0.4, 0.4, 0.4 (1.4 s), and another 0.4 would
        # pass 1.7, so at most 5 attempts. The budget counts the time the
        # attempts take too, so a loaded machine fits fewer (CI's hooks job
        # made 3 or 4 on 2026-11-09): the count is a bound, not a number.
        self.assertGreaterEqual(self.server.hits("/a"), 2)
        self.assertLessEqual(self.server.hits("/a"), 5)

    def test_a_destination_path_with_spaces_works(self):
        dest = self.dir / "a dir" / "the tool.tar.gz"
        self.assertPinned(self.fetch(self.server.plan("/a", {}), dest=dest), dest)

    def test_the_user_agent_names_curl_and_nothing_else(self):
        self.fetch(self.server.plan("/a", {}))
        ua = self.server.requests[0]["ua"]
        self.assertRegex(ua, r"^curl/[0-9.]+$")

    def test_each_retry_is_announced_with_its_delay(self):
        r = self.fetch(self.server.plan("/a", {"status": 503}, {}))
        self.assertIn("retrying in 0.01 s", r.stderr)
        self.assertIn("HTTP 503", r.stderr)


if __name__ == "__main__":
    unittest.main()
