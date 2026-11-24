"""Tests scripts/contend_writes.py: the writers scripts/scale-run.sh runs
beside vcon-scale when SCALE_WRITERS is set, so reads are measured while
other clients of the same tenant replace vCons.

Each writer stores one vCon of its own and then replaces it again and
again, each replacement naming the version the one before returned. The
loop is driven here through a fake connection that answers what a test
gives it. scripts/scale-run.sh is run whole too, with a fake vcon-server
(a TLS server with the run's own certificate, answering stores and
replacements) and a fake vcon-scale (which ends once the fake server has
taken some replacements), so the writers' start, their TLS connection,
their stop and their summary are driven through the real script.

Run with: python3 -I scripts/test_contend_writes.py
"""

import importlib.util
import json
import os
import pathlib
import signal
import subprocess
import sys
import tempfile
import textwrap
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("contend_writes", HERE / "contend_writes.py")
cw = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cw)

UUID = "0b9c6a3e-2f41-4d5e-9a1b-7c8d9e0f1a2b"


class Fake:
    """A connection whose answers are (status, etag or None), in order."""

    def __init__(self, answers):
        self.answers = list(answers)
        self.sent = []

    def send(self, method, path, body, headers):
        self.sent.append((method, path, body, dict(headers)))
        if not self.answers:
            raise AssertionError("the writer sent more requests than the test answers")
        return self.answers.pop(0)


def stop_after(n):
    """A stop test that answers True once it has been asked `n` times."""
    asked = [0]

    def stop():
        asked[0] += 1
        return asked[0] > n

    return stop


class Clock:
    """A clock that moves `step` seconds each time it is read."""

    def __init__(self, step=0.001):
        self.now = 0.0
        self.step = step

    def __call__(self):
        self.now += self.step
        return self.now


class Loop(unittest.TestCase):
    def test_creates_then_replaces_naming_each_returned_version(self):
        fake = Fake([(201, '"v1"'), (200, '"v2"'), (200, '"v3"')])
        done = cw.write_loop(fake.send, UUID, stop_after(2), Clock(), "k")
        self.assertEqual(done.writes, 3, "one store and two replacements")
        self.assertIsNone(done.error, "no request failed")
        methods = [(m, p) for m, p, _, _ in fake.sent]
        self.assertEqual(
            methods,
            [("POST", "/vcons"), ("PUT", "/vcons/" + UUID), ("PUT", "/vcons/" + UUID)],
            "a store, then replacements of the same vCon",
        )
        self.assertNotIn("If-Match", fake.sent[0][3], "a store names no version")
        self.assertEqual(fake.sent[1][3]["If-Match"], '"v1"', "the first replacement names the stored version")
        self.assertEqual(fake.sent[2][3]["If-Match"], '"v2"', "each next one names the version the one before returned")

    def test_every_request_carries_the_key_and_the_vcon_media_type(self):
        fake = Fake([(201, '"v1"'), (200, '"v2"')])
        cw.write_loop(fake.send, UUID, stop_after(1), Clock(), "secret-token")
        for _, _, _, headers in fake.sent:
            self.assertEqual(headers["Authorization"], "Bearer secret-token", "the key is sent as a bearer token")
            self.assertEqual(headers["Content-Type"], "application/vcon", "the body is a vCon")

    def test_each_body_is_a_vcon_with_the_writers_uuid_and_a_new_subject(self):
        fake = Fake([(201, '"v1"'), (200, '"v2"'), (200, '"v3"')])
        cw.write_loop(fake.send, UUID, stop_after(2), Clock(), "k")
        bodies = [json.loads(body) for _, _, body, _ in fake.sent]
        self.assertTrue(all(b["uuid"] == UUID for b in bodies), "every body names the writer's vCon")
        subjects = [b["subject"] for b in bodies]
        self.assertEqual(len(set(subjects)), 3, f"each write changes the vCon: {subjects}")

    def test_a_stop_before_the_first_replacement_leaves_the_store_alone(self):
        fake = Fake([(201, '"v1"')])
        done = cw.write_loop(fake.send, UUID, stop_after(0), Clock(), "k")
        self.assertEqual(done.writes, 1, "only the store")
        self.assertEqual(len(fake.sent), 1, "no replacement was sent")

    def test_a_refused_store_ends_the_writer_with_the_status(self):
        fake = Fake([(503, None)])
        done = cw.write_loop(fake.send, UUID, stop_after(5), Clock(), "k")
        self.assertEqual(done.writes, 0, "nothing was written")
        self.assertIn("503", done.error, f"the error names the status: {done.error}")
        self.assertIn("POST", done.error, f"the error names the request: {done.error}")

    def test_a_refused_replacement_ends_the_writer_after_the_writes_before_it(self):
        fake = Fake([(201, '"v1"'), (200, '"v2"'), (412, None)])
        done = cw.write_loop(fake.send, UUID, stop_after(10), Clock(), "k")
        self.assertEqual(done.writes, 2, "the store and one replacement")
        self.assertIn("412", done.error, f"the error names the status: {done.error}")
        self.assertEqual(len(fake.sent), 3, "no request after the refused one")

    def test_an_answer_without_a_version_ends_the_writer(self):
        fake = Fake([(201, None)])
        done = cw.write_loop(fake.send, UUID, stop_after(5), Clock(), "k")
        self.assertIn("ETag", done.error, f"the error says the version is missing: {done.error}")
        self.assertEqual(len(fake.sent), 1, "no replacement without a version to name")

    def test_a_connection_error_ends_the_writer_with_its_text(self):
        def broken(method, path, body, headers):
            raise OSError("connection reset")

        done = cw.write_loop(broken, UUID, stop_after(5), Clock(), "k")
        self.assertEqual(done.writes, 0, "nothing was written")
        self.assertIn("connection reset", done.error, f"the error carries the cause: {done.error}")

    def test_each_write_has_its_latency_from_the_clock(self):
        fake = Fake([(201, '"v1"'), (200, '"v2"')])
        done = cw.write_loop(fake.send, UUID, stop_after(1), Clock(step=0.002), "k")
        self.assertEqual(len(done.latencies), 2, "one latency per write")
        for ms in done.latencies:
            self.assertAlmostEqual(ms, 2.0, places=6, msg="the time between the reads around the request, in ms")


class Summary(unittest.TestCase):
    def test_percentiles_are_nearest_rank(self):
        values = [float(v) for v in range(1, 101)]
        self.assertEqual(cw.percentile(values, 50), 50.0, "the 50th of 100 ranked values")
        self.assertEqual(cw.percentile(values, 99), 99.0, "the 99th of 100 ranked values")
        self.assertEqual(cw.percentile([7.0], 99), 7.0, "one value is every percentile")

    def test_percentile_of_no_values_is_none(self):
        self.assertIsNone(cw.percentile([], 50), "no writes have no latency")

    def test_percentile_does_not_depend_on_the_order(self):
        self.assertEqual(cw.percentile([3.0, 1.0, 2.0], 50), 2.0, "values are ranked first")

    def test_summary_adds_the_writers(self):
        a = cw.Done(writes=3, latencies=[1.0, 2.0, 3.0], error=None)
        b = cw.Done(writes=1, latencies=[4.0], error=None)
        s = cw.summary([a, b], seconds=2.0)
        self.assertEqual(s["writers"], 2, "two writers")
        self.assertEqual(s["writes"], 4, "every write counted")
        self.assertEqual(s["writes_per_s"], 2.0, "writes over the time the writers ran")
        self.assertEqual(s["p50_ms"], 2.0, "latency over all writers")
        self.assertEqual(s["p99_ms"], 4.0, "the largest of four at the 99th")
        self.assertEqual(s["errors"], [], "no writer failed")

    def test_summary_lists_each_failed_writer(self):
        a = cw.Done(writes=0, latencies=[], error="POST /vcons: 503")
        s = cw.summary([a], seconds=1.0)
        self.assertEqual(s["errors"], ["POST /vcons: 503"], "the error of each writer that stopped")

    def test_summary_of_no_time_has_no_rate(self):
        s = cw.summary([cw.Done(writes=0, latencies=[], error=None)], seconds=0.0)
        self.assertIsNone(s["writes_per_s"], "no rate without elapsed time")


class StopFile(unittest.TestCase):
    def test_stops_once_the_file_exists(self):
        with tempfile.TemporaryDirectory() as d:
            flag = pathlib.Path(d) / "stop"
            stop = cw.stop_when_exists(flag)
            self.assertFalse(stop(), "the file is not there yet")
            flag.touch()
            self.assertTrue(stop(), "the file is there")


class Args(unittest.TestCase):
    def test_refuses_no_writers(self):
        with self.assertRaises(SystemExit, msg="zero writers is refused"):
            cw.parse(["--port", "1", "--ca", "c", "--key-file", "k", "--stop", "s", "--out", "o", "--writers", "0"])

    def test_reads_every_option(self):
        a = cw.parse(["--port", "8443", "--ca", "c.pem", "--key-file", "k", "--stop", "s", "--out", "o", "--writers", "2"])
        self.assertEqual((a.port, a.ca, a.key_file, a.stop, a.out, a.writers), (8443, "c.pem", "k", "s", "o", 2), "each option kept")


FAKE_SERVER = textwrap.dedent(
    '''\
    #!/usr/bin/env python3
    """A stand-in for vcon-server: `tenant create`, `key create`, and a
    `serve` that answers stores and replacements over TLS with the run's
    certificate. Each replacement's count goes to $FAKE_DIR/puts."""
    import http.server, json, os, pathlib, ssl, sys, threading, time
    args = sys.argv[1:]
    rec = pathlib.Path(os.environ["FAKE_DIR"])
    config = args[args.index("--config") + 1]
    if args[0] == "tenant":
        print(json.dumps({"id": "00112233445566778899aabbccddeeff"}))
        sys.exit(0)
    if args[0] == "key":
        print(json.dumps({"token": "fake-token"}))
        sys.exit(0)
    if args[0] != "serve":
        sys.exit(2)
    text = pathlib.Path(config).read_text()
    (rec / "config.toml").write_text(text)
    def value(name):
        line = [l for l in text.splitlines() if l.startswith(name + " = ")][0]
        return line.split(" = ", 1)[1].strip('"')
    fail_put = os.environ.get("FAKE_FAIL_PUT") == "1"
    lock = threading.Lock()
    puts = [0]
    def event(text):
        with lock, open(rec / "events", "a") as f:
            f.write("%.3f %s\\n" % (time.monotonic(), text))
    class Handler(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        def setup(self):
            super().setup()
            event("connection from port %d" % self.client_address[1])
        def parse_request(self):
            ok = super().parse_request()
            event("request %s %s from port %d" % (self.command, self.path, self.client_address[1]))
            return ok
        def answer(self, status, tag):
            event("answer %d to port %d" % (status, self.client_address[1]))
            self.send_response(status)
            if tag:
                self.send_header("ETag", tag)
            self.send_header("Content-Length", "0")
            self.end_headers()
        def well_formed(self):
            data = self.rfile.read(int(self.headers["Content-Length"]))
            return (
                self.headers.get("Authorization") == "Bearer fake-token"
                and self.headers.get("Content-Type") == "application/vcon"
                and bool(json.loads(data)["uuid"])
            )
        def do_POST(self):
            self.answer(201 if self.well_formed() else 400, '"v0"')
        def do_PUT(self):
            ok = self.well_formed() and self.headers.get("If-Match", "").startswith('"v')
            with lock:
                puts[0] += 1
                n = puts[0]
                (rec / "puts.tmp").write_text(str(n))
                os.replace(rec / "puts.tmp", rec / "puts")
            if fail_put:
                self.answer(500, None)
            else:
                self.answer(200 if ok else 400, '"v%d"' % n)
        def log_message(self, *args):
            pass
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(value("cert"), value("key"))
    server.socket = context.wrap_socket(server.socket, server_side=True)
    print("INFO serving listen=127.0.0.1:%d" % server.server_address[1], file=sys.stderr, flush=True)
    server.serve_forever()
    '''
)

FAKE_SCALE = textwrap.dedent(
    '''\
    #!/usr/bin/env python3
    """A stand-in for vcon-scale: ends once the fake server has taken
    $FAKE_PUTS replacements (at once for 0), then writes its result file."""
    import os, pathlib, sys, time
    args = sys.argv[1:]
    out = args[args.index("--out") + 1]
    rec = pathlib.Path(os.environ["FAKE_DIR"])
    want = int(os.environ.get("FAKE_PUTS", "3"))
    deadline = time.monotonic() + 60
    while want > 0:
        puts = rec / "puts"
        if puts.exists() and int(puts.read_text()) >= want:
            break
        if time.monotonic() > deadline:
            sys.exit(5)
        time.sleep(0.01)
    pathlib.Path(out).write_text('{"fake": true}\\n')
    '''
)

SCALE_RUN = HERE / "scale-run.sh"

# How long one run of scale-run.sh may take: a run here takes under 2 s.
DEADLINE = 90


class ScaleRun(unittest.TestCase):
    """scripts/scale-run.sh, run whole with the fakes above."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = pathlib.Path(self.tmp.name)
        self.rec = self.dir / "rec"
        self.rec.mkdir()
        self.parent = self.dir / "parent"
        self.parent.mkdir()
        for name, text in (("server", FAKE_SERVER), ("scale", FAKE_SCALE)):
            path = self.dir / name
            path.write_text(text)
            path.chmod(0o755)
        self.out = self.dir / "run.json"
        self.writes = self.dir / "run-writes.json"

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, **env):
        full = dict(os.environ, FAKE_DIR=str(self.rec), SCALE_DISK="test disk")
        for name in ("SCALE_WRITERS", "SCALE_CATALOG"):
            full.pop(name, None)
        full.update(env)
        # A session of its own, so the deadline ends the script and every
        # process it started, the fake server and the writers included.
        proc = subprocess.Popen(
            [str(SCALE_RUN), str(self.dir / "server"), str(self.dir / "scale"), "10", str(self.out), str(self.parent)],
            env=full,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        try:
            out, err = proc.communicate(timeout=DEADLINE)
        except subprocess.TimeoutExpired:
            left = subprocess.run(
                ["ps", "-o", "pid,ppid,stat,args", "-s", str(proc.pid)], capture_output=True, text=True
            ).stdout
            os.killpg(proc.pid, signal.SIGKILL)
            out, err = proc.communicate()
            events = self.rec / "events"
            self.fail(
                f"scale-run.sh did not end in {DEADLINE} s; its stderr: {err!r}; still running:\n{left}"
                "the fake server's events:\n" + (events.read_text() if events.exists() else "(none)")
            )
        return subprocess.CompletedProcess(proc.args, proc.returncode, out, err)

    def test_writers_replace_vcons_while_the_run_goes_on(self):
        done = self.run_script(SCALE_WRITERS="2", FAKE_PUTS="6")
        self.assertEqual(done.returncode, 0, f"the run passes: {done.stderr}")
        self.assertTrue(self.out.exists(), "vcon-scale wrote its result")
        result = json.loads(self.writes.read_text())
        self.assertEqual(result["writers"], 2, f"two writers ran: {result}")
        self.assertGreaterEqual(result["writes"], 8, f"two stores and at least six replacements: {result}")
        self.assertEqual(result["errors"], [], f"no writer failed: {result}")
        self.assertIsNotNone(result["p99_ms"], f"the writes' latency is reported: {result}")

    def test_without_writers_no_writer_runs(self):
        done = self.run_script(FAKE_PUTS="0")
        self.assertEqual(done.returncode, 0, f"the run passes: {done.stderr}")
        self.assertFalse(self.writes.exists(), "no writers' summary without SCALE_WRITERS")
        self.assertFalse((self.rec / "puts").exists(), "no replacement was sent")

    def test_a_failed_writer_fails_the_run(self):
        # Each writer ends at its first refused replacement: two in all.
        done = self.run_script(SCALE_WRITERS="2", FAKE_FAIL_PUT="1", FAKE_PUTS="2")
        self.assertNotEqual(done.returncode, 0, "a writer that failed fails the run")
        self.assertIn("500", done.stderr, f"the writer's error is shown: {done.stderr}")
        result = json.loads(self.writes.read_text())
        self.assertEqual(len(result["errors"]), 2, f"each writer's error is in the summary: {result}")

    def test_a_writer_count_that_is_not_a_number_is_refused(self):
        done = self.run_script(SCALE_WRITERS="two", FAKE_PUTS="0")
        self.assertEqual(done.returncode, 2, f"refused as a usage error: {done.stderr}")
        self.assertIn("SCALE_WRITERS", done.stderr, f"the message names the variable: {done.stderr}")
        self.assertFalse((self.rec / "config.toml").exists(), "no server was started")

    def test_the_catalog_file_is_added_to_the_servers_configuration(self):
        catalog = self.dir / "catalog.toml"
        catalog.write_text('[catalog]\nbackend = "postgres"\n')
        done = self.run_script(SCALE_CATALOG=str(catalog), FAKE_PUTS="0")
        self.assertEqual(done.returncode, 0, f"the run passes: {done.stderr}")
        config = (self.rec / "config.toml").read_text()
        self.assertTrue(config.endswith('[catalog]\nbackend = "postgres"\n'), f"the section ends the configuration: {config}")
        self.assertIn("[tls]", config, f"the run's own sections are kept: {config}")

    def test_every_file_the_run_made_is_deleted(self):
        done = self.run_script(SCALE_WRITERS="1", FAKE_PUTS="2")
        self.assertEqual(done.returncode, 0, f"the run passes: {done.stderr}")
        self.assertEqual(list(self.parent.iterdir()), [], "the work directory, its stop file included, is gone")


if __name__ == "__main__":
    unittest.main()
