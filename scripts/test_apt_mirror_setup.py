"""Tests scripts/apt-mirror-setup.sh: a container's apt reads each package
source from a list of hosts that answer now, not from one host.

The script is run whole, against a server on this machine that stands for
the mirrors: one that answers, one that answers 404, one that redirects,
one that sends nothing, one that sends no HTTP, and a port nobody listens
on. No test reaches another host.

Run with: python3 -I scripts/test_apt_mirror_setup.py
"""

import http.server
import importlib.util
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

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "apt-mirror-setup.sh"
spec = importlib.util.spec_from_file_location("docs_test", HERE / "docs_test.py")
dt = importlib.util.module_from_spec(spec)
sys.modules["docs_test"] = dt
spec.loader.exec_module(dt)

DEBIAN = "http://deb.debian.org/debian"
DEBIAN_SECURITY = "http://deb.debian.org/debian-security"
UBUNTU = "http://archive.ubuntu.com/ubuntu/"
UBUNTU_SECURITY = "http://security.ubuntu.com/ubuntu/"
PORTS = "http://ports.ubuntu.com/ubuntu-ports/"

# debian:12's /etc/apt/sources.list.d/debian.sources and ubuntu:22.04's
# /etc/apt/sources.list, shortened to the lines that name a source.
DEBIAN_SOURCES = """Types: deb
URIs: http://deb.debian.org/debian
Suites: bookworm bookworm-updates
Components: main

Types: deb
URIs: http://deb.debian.org/debian-security
Suites: bookworm-security
Components: main
"""
UBUNTU_SOURCES = """deb http://archive.ubuntu.com/ubuntu/ jammy main restricted
deb http://archive.ubuntu.com/ubuntu/ jammy-updates main restricted
deb http://security.ubuntu.com/ubuntu/ jammy-security main restricted
"""
PORTS_SOURCES = """deb http://ports.ubuntu.com/ubuntu-ports/ jammy main restricted
deb http://ports.ubuntu.com/ubuntu-ports/ jammy-security main restricted
"""


class Mirrors(http.server.BaseHTTPRequestHandler):
    """Every mirror the tests name, told apart by the first part of the
    path: /ok... answers, and the others each fail in their own way."""

    asked = []

    def do_GET(self):
        Mirrors.asked.append(self.path)
        kind = self.path.split("/")[1]
        if kind.startswith("ok11"):
            self.wfile.write(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        elif kind.startswith("ok") and self.path.endswith("/dists/"):
            self.send_response(200)
            self.end_headers()
        elif kind == "moved":
            self.send_response(301)
            self.send_header("Location", "/ok/dists/")
            self.end_headers()
        elif kind == "silent":
            time.sleep(8)
        elif kind == "noise":
            self.wfile.write(b"220 this is not a web server\r\n")
        elif kind == "almost":
            self.wfile.write(b"HTTP/1.0 2000 OK\r\n\r\n")
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, *args):
        pass


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True


def free_port():
    """A port on this machine that nobody listens on."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Case(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = Server(("127.0.0.1", 0), Mirrors)
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"
        cls.dead = f"http://127.0.0.1:{free_port()}"
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        Mirrors.asked.clear()
        self.dir = tempfile.mkdtemp(prefix="apt-mirror-setup-")
        self.addCleanup(shutil.rmtree, self.dir)
        self.root = os.path.join(self.dir, "root")
        os.makedirs(os.path.join(self.root, "etc/apt/sources.list.d"))
        os.makedirs(os.path.join(self.root, "etc/apt/apt.conf.d"))
        self.table = os.path.join(self.dir, "table.txt")

    def source(self, text, name="sources.list.d/debian.sources", root=None):
        path = os.path.join(root or self.root, "etc/apt", name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)

    def read(self, name, root=None):
        with open(os.path.join(root or self.root, "etc/apt", name), encoding="utf-8") as f:
            return f.read()

    def listing(self, n):
        return os.path.join(self.root, f"etc/apt/docs-test-mirrors-{n}.list")

    def run_script(self, table, *options, timeout="1", script=SCRIPT):
        """The script's (exit code, stdout, stderr) for the table `table`: a
        list of (origin, mirrors), or the table's text."""
        if not isinstance(table, str):
            table = "".join(" ".join([origin, *mirrors]) + "\n" for origin, mirrors in table)
        with open(self.table, "w", encoding="utf-8") as f:
            f.write(table)
        done = subprocess.run(
            ["bash", str(script), "--root", self.root, "--table", self.table, "--timeout", timeout, *options],
            capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL, check=False,
        )
        return done.returncode, done.stdout, done.stderr

    def tree(self, root):
        """Every file under root's /etc/apt, by name, with the root's own
        path taken out of the text."""
        out = {}
        top = os.path.join(root, "etc/apt")
        for folder, _, names in os.walk(top):
            for name in names:
                path = os.path.join(folder, name)
                with open(path, encoding="utf-8") as f:
                    out[os.path.relpath(path, top)] = f.read().replace(root, "ROOT")
        return out


class Success(Case):
    def test_the_mirrors_that_answer_are_listed_each_with_its_place(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian", f"{self.base}/ok2/debian"])])
        self.assertEqual(rc, 0, err)
        with open(self.listing(0), encoding="utf-8") as f:
            self.assertEqual(f.read(), f"{self.base}/ok1/debian\tpriority:1\n{self.base}/ok2/debian\tpriority:2\n", "the list apt's mirror method reads")

    def test_a_deb822_source_names_the_list_in_place_of_the_origin(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertIn(f"URIs: mirror+file:{self.listing(0)}\nSuites: bookworm bookworm-updates\n", self.read("sources.list.d/debian.sources"), "the source file")

    def test_a_one_line_source_names_the_list_in_place_of_the_origin(self):
        self.source(UBUNTU_SOURCES, "sources.list")
        rc, _, err = self.run_script([(UBUNTU, [f"{self.base}/ok1/ubuntu/"])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(
            self.read("sources.list").splitlines()[:2],
            [f"deb mirror+file:{self.listing(0)} jammy main restricted", f"deb mirror+file:{self.listing(0)} jammy-updates main restricted"],
            "both lines of the origin",
        )

    def test_two_origins_get_two_lists_numbered_in_the_table_s_order(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"]), (DEBIAN_SECURITY, [f"{self.base}/ok2/debian-security"])])
        self.assertEqual(rc, 0, err)
        text = self.read("sources.list.d/debian.sources")
        self.assertIn(f"URIs: mirror+file:{self.listing(0)}\nSuites: bookworm bookworm-updates\n", text, "the first origin")
        self.assertIn(f"URIs: mirror+file:{self.listing(1)}\nSuites: bookworm-security\n", text, "the second origin")

    def test_an_origin_the_system_does_not_read_gets_no_list_and_no_question(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(UBUNTU, [f"{self.dead}/ubuntu/"]), (DEBIAN, [f"{self.base}/ok1/debian"]), (DEBIAN_SECURITY, [f"{self.base}/ok2/s"])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(sorted(n for n in os.listdir(os.path.join(self.root, "etc/apt")) if n.endswith(".list")), ["docs-test-mirrors-0.list", "docs-test-mirrors-1.list"], "lists written")
        self.assertEqual(sorted(Mirrors.asked), ["/ok1/debian/dists/", "/ok2/s/dists/"], "mirrors asked")

    def test_the_log_says_where_each_origin_s_packages_come_from(self):
        self.source(DEBIAN_SOURCES)
        rc, out, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian", f"{self.base}/ok2/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, f"apt-mirror-setup: packages from {DEBIAN} come from {self.base}/ok1/debian, then {self.base}/ok2/debian\n", "stdout")

    def test_apt_is_told_how_often_to_ask_again_as_the_how_to_runner_tells_it(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 0, err)
        body = dt.APT_RETRIES.split("<<'DOCS_TEST_EOF'\n")[1].split("DOCS_TEST_EOF")[0]
        self.assertIn(f'Acquire::Retries "{dt.APT_TRIES}";\n', body, "the runner's own text")
        self.assertEqual(self.read("apt.conf.d/80docs-test-retries"), body, "the retries file")

    def same_as_the_runner(self, sources, name, chosen):
        """The script and docs_test.apt_mirror_setup, each on its own copy
        of one system, for the same mirrors."""
        other = os.path.join(self.dir, "other")
        self.source(sources, name)
        self.source(sources, name, root=other)
        os.makedirs(os.path.join(other, "etc/apt/apt.conf.d"))
        rc, _, err = self.run_script(list(chosen.items()))
        self.assertEqual(rc, 0, err)
        commands = dt.APT_RETRIES.replace("/etc/apt/", f"{other}/etc/apt/") + "".join(c + "\n" for c in dt.apt_mirror_setup(chosen, root=other))
        steps = os.path.join(self.dir, "runner.sh")
        with open(steps, "w", encoding="utf-8") as f:
            f.write("set -e\n" + commands)
        subprocess.run(["bash", steps], check=True, timeout=30)
        mine, runners = self.tree(self.root), self.tree(other)
        self.assertGreaterEqual(len(mine), 3, mine)
        self.assertEqual(mine, runners, "the files the script wrote and the files the how-to runner's commands wrote")

    def test_a_debian_system_ends_as_the_how_to_runner_leaves_it(self):
        self.same_as_the_runner(DEBIAN_SOURCES, "sources.list.d/debian.sources", {
            DEBIAN: [f"{self.base}/ok1/debian", f"{self.base}/ok2/debian"],
            DEBIAN_SECURITY: [f"{self.base}/ok3/debian-security"],
        })

    def test_an_ubuntu_system_ends_as_the_how_to_runner_leaves_it(self):
        self.same_as_the_runner(UBUNTU_SOURCES, "sources.list", {
            UBUNTU: [f"{self.base}/ok1/ubuntu/", f"{self.base}/ok2/ubuntu/"],
            UBUNTU_SECURITY: [f"{self.base}/ok3/ubuntu/", f"{self.base}/ok1/ubuntu/"],
        })

    def test_an_ubuntu_ports_system_ends_as_the_how_to_runner_leaves_it(self):
        self.same_as_the_runner(PORTS_SOURCES, "sources.list", {PORTS: [f"{self.base}/ok1/ubuntu-ports/", f"{self.base}/ok2/ubuntu-ports/"]})

    def test_without_a_table_named_the_one_beside_the_script_is_read(self):
        self.source(DEBIAN_SOURCES)
        copy = os.path.join(self.dir, "copy")
        os.makedirs(copy)
        shutil.copy(SCRIPT, copy)
        with open(os.path.join(copy, "apt-mirrors.txt"), "w", encoding="utf-8") as f:
            f.write(f"{DEBIAN} {self.base}/ok1/debian\n")
        done = subprocess.run(
            ["bash", os.path.join(copy, "apt-mirror-setup.sh"), "--root", self.root, "--timeout", "1"],
            capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL, check=False,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(os.path.isfile(self.listing(0)), "the list from the table beside the script")
        self.assertTrue((HERE / "apt-mirrors.txt").is_file(), "the table the real script reads")

    def test_a_mirror_is_asked_for_its_index_of_suites_with_or_without_a_last_slash(self):
        self.source(UBUNTU_SOURCES, "sources.list")
        rc, _, err = self.run_script([(UBUNTU, [f"{self.base}/ok1/ubuntu/", f"{self.base}/ok2/pub/ubuntu"])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(sorted(Mirrors.asked), ["/ok1/ubuntu/dists/", "/ok2/pub/ubuntu/dists/"], "paths asked")

    def test_a_mirror_of_two_origins_is_asked_once(self):
        self.source(UBUNTU_SOURCES, "sources.list")
        both = f"{self.base}/ok1/ubuntu/"
        rc, _, err = self.run_script([(UBUNTU, [both]), (UBUNTU_SECURITY, [f"{self.base}/ok2/ubuntu/", both])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(sorted(Mirrors.asked), ["/ok1/ubuntu/dists/", "/ok2/ubuntu/dists/"], "paths asked")

    def test_an_http_1_1_answer_counts(self):
        self.source(DEBIAN_SOURCES)
        rc, out, err = self.run_script([(DEBIAN, [f"{self.base}/ok11/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertIn(f"come from {self.base}/ok11/debian\n", out, "stdout")


class Failure(Case):
    def untouched(self, text=DEBIAN_SOURCES, name="sources.list.d/debian.sources"):
        self.assertEqual(self.read(name), text, "the source file after a run that failed")
        left = [n for n in os.listdir(os.path.join(self.root, "etc/apt")) if n.startswith("docs-test-mirrors")]
        self.assertEqual(left, [], "lists written by a run that failed")
        self.assertEqual(os.listdir(os.path.join(self.root, "etc/apt/apt.conf.d")), [], "apt settings written by a run that failed")

    def test_no_mirror_answering_ends_with_3_and_names_the_origin_and_each_mirror_tried(self):
        self.source(DEBIAN_SOURCES)
        rc, out, err = self.run_script([(DEBIAN, [f"{self.dead}/a/debian", f"{self.base}/gone/debian"])])
        self.assertEqual(rc, 3, err)
        self.assertEqual(out, "", "stdout")
        self.assertEqual(
            err,
            f"apt-mirror-setup: no package mirror answered for {DEBIAN} (tried {self.dead}/a/debian, {self.base}/gone/debian): "
            "the package mirrors are unreachable from this machine\n",
            "stderr",
        )

    def test_a_run_that_ends_with_3_changes_nothing(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.dead}/a/debian"])])
        self.assertEqual(rc, 3, err)
        self.untouched()

    def test_a_second_origin_with_no_mirror_leaves_the_first_untouched_too(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"]), (DEBIAN_SECURITY, [f"{self.dead}/s"])])
        self.assertEqual(rc, 3, err)
        self.assertIn(f"no package mirror answered for {DEBIAN_SECURITY} ", err, "stderr")
        self.untouched()

    def test_a_mirror_that_answers_404_is_left_out(self):
        self.source(DEBIAN_SOURCES)
        rc, out, err = self.run_script([(DEBIAN, [f"{self.base}/gone/debian", f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, f"apt-mirror-setup: packages from {DEBIAN} come from {self.base}/ok1/debian\n", "stdout")

    def test_a_mirror_that_redirects_is_left_out(self):
        self.source(DEBIAN_SOURCES)
        rc, out, err = self.run_script([(DEBIAN, [f"{self.base}/moved/debian", f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertNotIn("moved", out, "stdout")
        self.assertIn("/moved/debian/dists/", Mirrors.asked, "it was asked")

    def test_a_mirror_that_sends_nothing_is_left_out_within_the_time_limit(self):
        self.source(DEBIAN_SOURCES)
        start = time.monotonic()
        rc, out, err = self.run_script([(DEBIAN, [f"{self.base}/silent/debian", f"{self.base}/ok1/debian"])], timeout="1")
        took = time.monotonic() - start
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, f"apt-mirror-setup: packages from {DEBIAN} come from {self.base}/ok1/debian\n", "stdout")
        self.assertLess(took, 6, "the 1 s limit, not the 8 s the mirror sleeps")

    def test_a_host_that_does_not_speak_http_is_left_out(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/noise/debian"])])
        self.assertEqual(rc, 3, err)

    def test_a_status_that_only_starts_with_200_is_left_out(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/almost/debian"])])
        self.assertEqual(rc, 3, err)

    def test_sources_the_table_does_not_know_end_with_4(self):
        text = "deb http://packages.example.com/debian bookworm main\n"
        self.source(text, "sources.list")
        rc, out, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 4, err)
        self.assertEqual(out, "", "stdout")
        self.assertIn("apt-mirror-setup: no package source of this system is in ", err, "stderr")
        self.assertEqual(Mirrors.asked, [], "mirrors asked")
        self.untouched(text, "sources.list")

    def test_a_system_with_no_source_file_ends_with_4(self):
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 4, err)

    def test_a_table_that_is_not_there_ends_with_2(self):
        self.source(DEBIAN_SOURCES)
        done = subprocess.run(
            ["bash", str(SCRIPT), "--root", self.root, "--table", os.path.join(self.dir, "none.txt")],
            capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL, check=False,
        )
        self.assertEqual(done.returncode, 2, done.stderr)
        self.assertIn("none.txt", done.stderr, "stderr")
        self.untouched()

    def test_an_option_the_script_does_not_have_ends_with_2(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])], "--fast")
        self.assertEqual(rc, 2, err)
        self.assertIn("--fast", err, "stderr")
        self.untouched()

    def test_an_origin_with_no_mirror_in_the_table_ends_with_2(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script(f"{DEBIAN}\n")
        self.assertEqual(rc, 2, err)
        self.assertIn(f"table.txt:1: {DEBIAN} has no mirror", err, "stderr")
        self.untouched()

    def test_a_mirror_that_is_not_an_http_address_ends_with_2(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian", "https://deb.example.com/debian"])])
        self.assertEqual(rc, 2, err)
        self.assertIn("table.txt:1: https://deb.example.com/debian is not an http:// address", err, "stderr")
        self.assertEqual(Mirrors.asked, [], "mirrors asked")
        self.untouched()

    def test_a_time_limit_that_is_not_a_number_ends_with_2(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])], timeout="soon")
        self.assertEqual(rc, 2, err)
        self.untouched()

    def test_an_option_without_its_value_ends_with_2(self):
        self.source(DEBIAN_SOURCES)
        done = subprocess.run(["bash", str(SCRIPT), "--root"], capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL, check=False)
        self.assertEqual(done.returncode, 2, done.stderr)


class Edge(Case):
    def test_a_first_mirror_that_is_down_is_left_out_and_the_rest_move_up(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.dead}/debian", f"{self.base}/ok1/debian", f"{self.base}/gone/debian", f"{self.base}/ok2/debian"])])
        self.assertEqual(rc, 0, err)
        with open(self.listing(0), encoding="utf-8") as f:
            self.assertEqual(f.read(), f"{self.base}/ok1/debian\tpriority:1\n{self.base}/ok2/debian\tpriority:2\n", "the list")

    def test_comments_and_empty_lines_in_the_table_are_passed_over(self):
        self.source(DEBIAN_SOURCES)
        rc, out, err = self.run_script(f"# the mirrors\n\n   # indented\n{DEBIAN}   {self.base}/ok1/debian\n")
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, f"apt-mirror-setup: packages from {DEBIAN} come from {self.base}/ok1/debian\n", "stdout")

    def test_an_origin_matches_with_or_without_its_last_slash(self):
        self.source("deb http://archive.ubuntu.com/ubuntu jammy main\ndeb http://deb.debian.org/debian/ bookworm main\n", "sources.list")
        rc, _, err = self.run_script([(UBUNTU, [f"{self.base}/ok1/ubuntu/"]), (DEBIAN, [f"{self.base}/ok2/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(
            self.read("sources.list"),
            f"deb mirror+file:{self.listing(0)} jammy main\ndeb mirror+file:{self.listing(1)} bookworm main\n",
            "the source file",
        )

    def test_an_origin_that_starts_another_does_not_take_its_lines(self):
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertIn("URIs: http://deb.debian.org/debian-security\n", self.read("sources.list.d/debian.sources"), "the security source")

    def test_only_the_longer_origin_in_the_sources_is_not_taken_for_the_shorter(self):
        text = "deb http://deb.debian.org/debian-security bookworm-security main\n"
        self.source(text, "sources.list")
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 4, err)
        self.assertEqual(self.read("sources.list"), text, "the source file")

    def test_a_dot_in_an_origin_is_a_dot(self):
        text = "deb http://debXdebian.org/debian bookworm main\n"
        self.source(text, "sources.list")
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 4, err)
        self.assertEqual(self.read("sources.list"), text, "the source file")

    def test_both_kinds_of_source_file_are_rewritten(self):
        self.source("deb http://deb.debian.org/debian bookworm main\n", "sources.list")
        self.source(DEBIAN_SOURCES)
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.read("sources.list"), f"deb mirror+file:{self.listing(0)} bookworm main\n", "sources.list")
        self.assertIn(f"URIs: mirror+file:{self.listing(0)}\n", self.read("sources.list.d/debian.sources"), "debian.sources")

    def test_a_second_run_changes_nothing_and_ends_with_0(self):
        self.source(DEBIAN_SOURCES)
        table = [(DEBIAN, [f"{self.base}/ok1/debian"]), (DEBIAN_SECURITY, [f"{self.base}/ok2/s"])]
        rc, _, err = self.run_script(table)
        self.assertEqual(rc, 0, err)
        before = self.tree(self.root)
        Mirrors.asked.clear()
        rc, out, err = self.run_script(table)
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, "apt-mirror-setup: this system's sources already read their mirror lists\n", "stdout")
        self.assertEqual(self.tree(self.root), before, "the files after the second run")
        self.assertEqual(Mirrors.asked, [], "mirrors asked by the second run")

    def test_a_directory_among_the_source_files_is_passed_over(self):
        self.source(DEBIAN_SOURCES)
        os.makedirs(os.path.join(self.root, "etc/apt/sources.list.d/old"))
        rc, _, err = self.run_script([(DEBIAN, [f"{self.base}/ok1/debian"])])
        self.assertEqual(rc, 0, err)
        self.assertTrue(os.path.isfile(self.listing(0)), "the list")
        self.assertEqual(err, "", "stderr")

    def test_the_script_reads_nothing_from_its_standard_input(self):
        self.source(DEBIAN_SOURCES)
        with open(self.table, "w", encoding="utf-8") as f:
            f.write(f"{DEBIAN} {self.base}/ok1/debian\n{DEBIAN_SECURITY} {self.base}/ok2/s\n")
        done = subprocess.run(
            ["bash", str(SCRIPT), "--root", self.root, "--table", self.table, "--timeout", "1"],
            capture_output=True, text=True, timeout=60, input="one\ntwo\n", check=False,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(len(done.stdout.splitlines()), 2, done.stdout)

    def test_the_real_table_gives_every_origin_itself_and_one_other_host(self):
        rows = [line.split() for line in (HERE / "apt-mirrors.txt").read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#")]
        self.assertEqual(len(rows), 5, rows)
        for origin, *mirrors in rows:
            self.assertIn(origin, mirrors, origin)
            self.assertGreaterEqual(len(set(mirrors)), 2, origin)
            for m in mirrors:
                self.assertTrue(m.startswith("http://"), m)

    def test_the_real_table_is_the_how_to_runner_s_table(self):
        rows = [line.split() for line in (HERE / "apt-mirrors.txt").read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#")]
        self.assertEqual({origin: tuple(mirrors) for origin, *mirrors in rows}, dt.APT_MIRRORS, "scripts/apt-mirrors.txt and docs_test.APT_MIRRORS")
        self.assertEqual([origin for origin, *_ in rows], list(dt.APT_MIRRORS), "the order of the origins")


if __name__ == "__main__":
    unittest.main()
