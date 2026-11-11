"""Tests scripts/wait-for-s3.sh: an object store is ready when it answers a
signed request with 200, not when it first answers.

On 2026-11-11 CI's MinIO job failed at its first write: MinIO listens
before it is initialized and answers 503 until it is, and the step that
waited for it took the first answer of any status as ready. The script is
run whole against a server on this machine that answers the statuses a
test gives it, in order.

Run with: python3 -I scripts/test_wait_for_s3.py
"""

import http.server
import os
import pathlib
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "wait-for-s3.sh"


class Store(http.server.BaseHTTPRequestHandler):
    """Answers the next status of `statuses`, and the last one from then on."""

    statuses = [200]
    requests = []

    def do_GET(self):
        # The line as it was sent: http.server folds a path that starts // into /.
        Store.requests.append((self.raw_requestline.decode("ascii").split()[1], self.headers.get("Authorization", "")))
        status = Store.statuses.pop(0) if len(Store.statuses) > 1 else Store.statuses[0]
        body = b"<Error><Code>XMinioServerNotInitialized</Code></Error>" if status == 503 else b"<ListAllMyBucketsResult/>"
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Case(unittest.TestCase):
    def setUp(self):
        Store.requests = []
        self.server = Server(("127.0.0.1", 0), Store)
        self.endpoint = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)

    def wait(self, statuses, *args, endpoint=None, tries="5", env=None, name="MinIO"):
        """The script's (exit code, stdout, stderr) against a store that
        answers `statuses`."""
        Store.statuses = list(statuses)
        full = {
            "PATH": os.environ["PATH"],
            "VCON_TEST_S3_ENDPOINT": endpoint or self.endpoint,
            "VCON_TEST_S3_ACCESS_KEY": "vcontest",
            "VCON_TEST_S3_SECRET_KEY": "not-a-real-secret",
            "WAIT_FOR_S3_TRIES": tries,
            "WAIT_FOR_S3_INTERVAL": "0.05",
        }
        full.update(env or {})
        full = {k: v for k, v in full.items() if v is not None}
        command = ["bash", str(SCRIPT)] + ([name] if name is not None else []) + list(args)
        done = subprocess.run(command, capture_output=True, text=True, timeout=60, env=full, stdin=subprocess.DEVNULL, check=False)
        return done.returncode, done.stdout, done.stderr


class Success(Case):
    def test_a_store_that_answers_200_is_ready_at_once(self):
        rc, out, err = self.wait([200])
        self.assertEqual(rc, 0, err)
        self.assertEqual(len(Store.requests), 1, Store.requests)
        self.assertEqual(out, "wait-for-s3: MinIO answered a signed request with 200 at try 1\n", "stdout")

    def test_a_store_that_answers_503_first_is_waited_for(self):
        rc, out, err = self.wait([503, 503, 200])
        self.assertEqual(rc, 0, err)
        self.assertEqual(len(Store.requests), 3, Store.requests)
        self.assertIn("at try 3\n", out, "stdout")

    def test_the_request_lists_the_buckets_and_is_signed_with_the_key(self):
        rc, _, err = self.wait([200])
        self.assertEqual(rc, 0, err)
        path, authorization = Store.requests[0]
        self.assertEqual(path, "/", "the path asked")
        self.assertTrue(authorization.startswith("AWS4-HMAC-SHA256 Credential=vcontest/"), authorization)
        self.assertIn("/us-east-1/s3/aws4_request", authorization, "the region and the service")

    def test_a_store_that_starts_listening_later_is_waited_for(self):
        port = free_port()
        late = []

        def start():
            time.sleep(0.4)
            late.append(Server(("127.0.0.1", port), Store))
            late[0].serve_forever()

        threading.Thread(target=start, daemon=True).start()
        rc, out, err = self.wait([200], endpoint=f"http://127.0.0.1:{port}", tries="200")
        if late:
            late[0].shutdown()
            late[0].server_close()
        self.assertEqual(rc, 0, err)
        self.assertIn("answered a signed request with 200 at try ", out, "stdout")
        self.assertNotIn("at try 1\n", out, "it was ready before it listened")

    def test_an_endpoint_with_a_last_slash_is_asked_the_same(self):
        rc, _, err = self.wait([200], endpoint=self.endpoint + "/")
        self.assertEqual(rc, 0, err)
        self.assertEqual(Store.requests[0][0], "/", "the path asked")

    def test_the_secret_is_not_printed(self):
        rc, out, err = self.wait([503, 200])
        self.assertEqual(rc, 0, err)
        self.assertNotIn("not-a-real-secret", out + err, "the secret key")


class Failure(Case):
    def test_a_store_that_never_leaves_503_fails_and_names_the_status(self):
        rc, out, err = self.wait([503], tries="4")
        self.assertEqual(rc, 1, err)
        self.assertEqual(out, "", "stdout")
        self.assertEqual(len(Store.requests), 4, Store.requests)
        self.assertIn("wait-for-s3: MinIO never answered a signed request with 200 in 4 tries; its last answer was 503\n", err, "stderr")

    def test_a_store_that_refuses_the_key_is_not_ready(self):
        rc, _, err = self.wait([403], tries="3")
        self.assertEqual(rc, 1, err)
        self.assertIn("its last answer was 403\n", err, "stderr")

    def test_an_address_nobody_listens_on_fails_and_says_there_was_no_answer(self):
        rc, _, err = self.wait([200], endpoint=f"http://127.0.0.1:{free_port()}", tries="3")
        self.assertEqual(rc, 1, err)
        self.assertIn("in 3 tries; it gave no answer\n", err, "stderr")
        self.assertEqual(Store.requests, [], "requests to the other server")

    def test_the_store_s_log_is_printed_when_it_is_never_ready(self):
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "minio.log")
            with open(log, "w", encoding="utf-8") as f:
                f.write("FATAL the data directory is not writable\n")
            rc, _, err = self.wait([503], log, tries="2")
        self.assertEqual(rc, 1, err)
        self.assertIn("FATAL the data directory is not writable\n", err, "stderr")

    def test_the_store_s_log_is_not_printed_when_it_is_ready(self):
        with tempfile.TemporaryDirectory() as d:
            log = os.path.join(d, "minio.log")
            with open(log, "w", encoding="utf-8") as f:
                f.write("API: http://127.0.0.1:9000\n")
            rc, out, err = self.wait([200], log)
        self.assertEqual(rc, 0, err)
        self.assertNotIn("API:", out + err, "the log")

    def test_a_log_that_is_not_there_does_not_change_the_failure(self):
        rc, _, err = self.wait([503], "/nonexistent/minio.log", tries="2")
        self.assertEqual(rc, 1, err)
        self.assertIn("its last answer was 503\n", err, "stderr")
        self.assertTrue(err.endswith("its last answer was 503\n"), err)

    def test_no_name_is_a_usage_error(self):
        rc, _, err = self.wait([200], name=None)
        self.assertEqual(rc, 2, err)
        self.assertIn("usage: wait-for-s3.sh <name> [<log file>]", err, "stderr")
        self.assertEqual(Store.requests, [], "requests")

    def test_each_missing_setting_is_named(self):
        for name in ("VCON_TEST_S3_ENDPOINT", "VCON_TEST_S3_ACCESS_KEY", "VCON_TEST_S3_SECRET_KEY"):
            rc, _, err = self.wait([200], env={name: None})
            self.assertEqual(rc, 2, f"{name}: {err}")
            self.assertIn(f"wait-for-s3: {name} is not set", err, name)
        self.assertEqual(Store.requests, [], "requests")

    def test_a_number_of_tries_that_is_not_one_is_refused(self):
        for tries in ("many", "0", ""):
            rc, _, err = self.wait([200], tries=tries)
            self.assertEqual(rc, 2, f"{tries!r}: {err}")
        self.assertEqual(Store.requests, [], "requests")


class Edge(Case):
    def test_one_try_asks_once(self):
        rc, _, err = self.wait([503, 200], tries="1")
        self.assertEqual(rc, 1, err)
        self.assertEqual(len(Store.requests), 1, Store.requests)

    def test_ready_at_the_last_try_is_ready(self):
        rc, out, err = self.wait([503, 503, 200], tries="3")
        self.assertEqual(rc, 0, err)
        self.assertIn("at try 3\n", out, "stdout")

    def test_a_redirect_is_not_ready(self):
        rc, _, err = self.wait([301], tries="2")
        self.assertEqual(rc, 1, err)
        self.assertIn("its last answer was 301\n", err, "stderr")

    def test_the_name_is_the_one_given(self):
        rc, out, err = self.wait([200], name="RustFS")
        self.assertEqual(rc, 0, err)
        self.assertTrue(out.startswith("wait-for-s3: RustFS answered"), out)

    def test_without_the_two_settings_it_tries_sixty_times_a_second_apart(self):
        text = SCRIPT.read_text(encoding="utf-8")
        self.assertIn('tries=${WAIT_FOR_S3_TRIES-60}', text, "the default number of tries")
        self.assertIn('interval=${WAIT_FOR_S3_INTERVAL-1}', text, "the default wait")


class TheWorkflows(unittest.TestCase):
    """CI waits for each object store with the script, and nowhere takes a
    first answer of any status as ready."""

    def test_no_workflow_takes_any_answer_from_the_store_as_ready(self):
        for path in sorted((HERE.parent / ".github/workflows").glob("*.y*ml")):
            text = path.read_text(encoding="utf-8")
            self.assertNotIn('"$VCON_TEST_S3_ENDPOINT/" && exit 0', text, f"{path.name} takes the store's first answer as ready")

    def test_ci_waits_for_each_store_with_the_script(self):
        text = (HERE.parent / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertEqual(text.count("scripts/wait-for-s3.sh RustFS\n"), 2, "the two jobs with a RustFS service")
        self.assertEqual(text.count('scripts/wait-for-s3.sh MinIO "$RUNNER_TEMP/minio.log"\n'), 1, "the MinIO job")

    def test_the_first_write_to_minio_comes_after_the_wait(self):
        text = (HERE.parent / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertLess(text.index("scripts/wait-for-s3.sh MinIO"), text.index("The service refuses a second write with If-None-Match"), "the order of the steps")


if __name__ == "__main__":
    unittest.main()
