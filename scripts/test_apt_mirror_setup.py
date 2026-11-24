"""Tests scripts/apt-mirror-setup.sh: a container's apt reads each package
source from a list of hosts that answer now and serve the same release as
the source's own host, not from one host.

The script is run whole, against a server on this machine that stands for
every host: the origins themselves, mirrors in sync, mirrors in the middle
of a sync, one that answers 404, one that redirects, one that sends
nothing, one that sends no HTTP, and a port nobody listens on. No test
reaches another host: every origin a test's system reads is on this
server, because the script asks the origin too.

The how-to runner's twin (docs_test.choose_mirrors) runs this script, so
the last class runs it through the runner against the same server.

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
import unittest.mock

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "apt-mirror-setup.sh"
spec = importlib.util.spec_from_file_location("docs_test", HERE / "docs_test.py")
dt = importlib.util.module_from_spec(spec)
sys.modules["docs_test"] = dt
spec.loader.exec_module(dt)

ME = "apt-mirror-setup"

# How late the `late` host answers: past a 1 s limit, as a host can on a
# loaded machine (2026-11-24, CI's "git hooks" job: two hosts of the tie
# test answered after the 1 s limit the tests then gave every run, and
# were left out without a word).
LATE = 1.5

# The script's --timeout in every run whose hosts all answer: a bound no
# answer on a loaded machine comes near, and below the 60 s each run may
# take. It costs no time, as a host that answers ends its wait. Only a
# test of a host that never answers gives a short limit, and says so.
ANSWER_WITHIN = 40

# The test of a host that never answers: it sleeps SILENT seconds, and the
# script's limit is SILENT_LIMIT, which the origin of that test, which must
# answer, is given too. 4 s holds an answer on a loaded machine, and the
# run still ends well before the silent host would.
SILENT = 20
SILENT_LIMIT = 4

# The Date of each copy of a release file the server hands out. A mirror
# in sync holds the origin's copy (NOW); one in the middle of a sync holds
# an older one (BEFORE); NEWER stands for a copy the origin has not been
# seen to serve.
BEFORE = "Sun, 22 Nov 2026 06:00:00 UTC"
NOW = "Mon, 23 Nov 2026 12:00:00 UTC"
NEWER = "Mon, 23 Nov 2026 18:00:00 UTC"


def release(suite, date, extra=""):
    """An InRelease file for `suite`, of the date `date`: what differs
    between two copies is the date and `extra`."""
    text = f"-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA512\n\nOrigin: Test\nSuite: {suite}\n"
    if date:
        text += f"Date: {date}\n"
    text += f"SHA256:\n 0123456789abcdef {len(suite) * 1000 + len(extra)} main/binary-amd64/Packages{extra}\n"
    return (text + "-----BEGIN PGP SIGNATURE-----\n\nc2lnbmF0dXJl\n-----END PGP SIGNATURE-----\n").encode()


# What a host serves, by the first part of its path. Each is a function of
# the suite, or None for a host that does not serve that suite's file.
COPIES = {
    "ok": lambda s: release(s, NOW),
    "old": lambda s: release(s, BEFORE, " old"),
    "new": lambda s: release(s, NEWER, " new"),
    # The date of `new`, other bytes.
    "twin": lambda s: release(s, NEWER, " twin"),
    # The date of `ok`, other bytes.
    "fork": lambda s: release(s, NOW, " fork"),
    "nodate": lambda s: release(s, None, " nodate"),
    # The newest copy of one suite and an older copy of the other.
    "mixa": lambda s: release(s, NEWER, " a") if s.endswith("updates") else release(s, BEFORE, " a"),
    "mixb": lambda s: release(s, BEFORE, " b") if s.endswith("updates") else release(s, NEWER, " b"),
    # Only the first suite of the test systems.
    "half": lambda s: release(s, NOW) if s in ("bookworm", "jammy") else None,
    "empty": lambda s: b"",
}


class Mirrors(http.server.BaseHTTPRequestHandler):
    """Every host the tests name, told apart by the first part of the path:
    /ok..., /old... and the others in COPIES serve a release file, and the
    rest each fail in their own way."""

    asked = []

    def do_GET(self):
        Mirrors.asked.append(self.path)
        kind = self.path.split("/")[1]
        name = kind.rstrip("0123456789")
        body = None
        if "/dists/" in self.path and self.path.endswith("/InRelease"):
            suite = self.path.split("/dists/")[1].split("/")[0]
            if kind.startswith("ok11"):
                body = COPIES["ok"](suite)
                self.wfile.write(b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\n\r\n" % len(body) + body)
                return
            if name in COPIES:
                body = COPIES[name](suite)
        if body is not None:
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif kind == "moved":
            self.send_response(301)
            self.send_header("Location", "/ok/dists/bookworm/InRelease")
            self.end_headers()
        elif kind == "silent":
            time.sleep(SILENT)
        elif kind == "late":
            # The `ok` copy, LATE seconds after it was asked for, as a
            # loaded host answers.
            time.sleep(LATE)
            body = COPIES["ok"](self.path.split("/dists/")[1].split("/")[0])
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif kind == "noise":
            self.wfile.write(b"220 this is not a web server\r\n")
        elif kind == "almost":
            self.wfile.write(b"HTTP/1.0 2000 OK\r\n\r\n" + COPIES["ok"]("bookworm"))
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


def deb822(main, security=None):
    """A Debian system's debian.sources, as debian:12 has it, its two
    origins `main` and `security`."""
    text = f"Types: deb\nURIs: {main}\nSuites: bookworm bookworm-updates\nComponents: main\n"
    if security:
        text += f"\nTypes: deb\nURIs: {security}\nSuites: bookworm-security\nComponents: main\n"
    return text


def one_line(main, security=None):
    """An Ubuntu system's sources.list, as ubuntu:22.04 has it, shortened to
    the lines that name a source."""
    text = f"deb {main} jammy main restricted\ndeb {main} jammy-updates main restricted\n"
    if security:
        text += f"deb {security} jammy-security main restricted\n"
    return text


class Case(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = Server(("127.0.0.1", 0), Mirrors)
        cls.base = f"http://127.0.0.1:{cls.server.server_address[1]}"
        cls.dead = f"http://127.0.0.1:{free_port()}"
        threading.Thread(target=cls.server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True).start()

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
        # The origins of the test systems: hosts on the test server that
        # serve the release files in sync.
        self.deb = self.at("ok0", "debian")
        self.deb_security = self.at("ok0", "debian-security")
        self.ubuntu = self.at("ok0", "ubuntu/")
        self.ubuntu_security = self.at("ok9", "ubuntu/")

    def at(self, kind, path="debian"):
        """The address of a host of the kind `kind` on the test server."""
        return f"{self.base}/{kind}/{path}"

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

    def listed(self, n):
        """The mirrors of the list `n`, in their places."""
        with open(self.listing(n), encoding="utf-8") as f:
            return [line.split("\t")[0] for line in f.read().splitlines()]

    def write_table(self, table):
        if not isinstance(table, str):
            table = "".join(" ".join([origin, *mirrors]) + "\n" for origin, mirrors in table)
        with open(self.table, "w", encoding="utf-8") as f:
            f.write(table)

    def run_script(self, table, *options, timeout=str(ANSWER_WITHIN), script=SCRIPT):
        """The script's (exit code, stdout, stderr) for the table `table`: a
        list of (origin, mirrors), or the table's text."""
        self.write_table(table)
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

    def untouched(self, text, name="sources.list.d/debian.sources"):
        self.assertEqual(self.read(name), text, "the source file after a run that wrote nothing")
        left = [n for n in os.listdir(os.path.join(self.root, "etc/apt")) if n.startswith("docs-test-mirrors")]
        self.assertEqual(left, [], "lists written by a run that wrote nothing")
        self.assertEqual(os.listdir(os.path.join(self.root, "etc/apt/apt.conf.d")), [], "apt settings written by a run that wrote nothing")

    def differs(self, mirror, suite, origin):
        return f"{ME}: left out {mirror}: its InRelease for {suite} is not the one {origin} serves (a mirror in the middle of a sync)"

    def unreachable(self, origin):
        return f"{ME}: {origin} could not be reached: its mirrors are compared with one another, and only those that hold its newest InRelease are kept"

    def older(self, mirror, suite):
        return f"{ME}: left out {mirror}: its InRelease for {suite} is older than another mirror's"

    def not_kept(self, mirror, suite, kept):
        return f"{ME}: left out {mirror}: its InRelease for {suite} is not the copy {kept} serves"

    def alone(self, origin, mirror):
        return f"{ME}: no two mirrors of {origin} hold the same newest InRelease: packages come from {mirror} alone, the one with the newest copy"

    def only(self, origin):
        return (f"{ME}: only {origin} itself remains for its packages: each of its other mirrors is out of sync or did not answer, "
                "so a failure of that one host fails the install")

    def come_from(self, origin, *mirrors):
        return f"{ME}: packages from {origin} come from {', then '.join(mirrors)}"


class Success(Case):
    def test_the_mirrors_that_serve_the_origin_s_release_are_listed_each_with_its_place(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.at("ok1"), self.deb, self.at("ok2")])])
        self.assertEqual(rc, 0, err)
        with open(self.listing(0), encoding="utf-8") as f:
            self.assertEqual(f.read(), f"{self.at('ok1')}\tpriority:1\n{self.deb}\tpriority:2\n{self.at('ok2')}\tpriority:3\n", "the list apt's mirror method reads")

    def test_a_deb822_source_names_the_list_in_place_of_the_origin(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertIn(f"URIs: mirror+file:{self.listing(0)}\nSuites: bookworm bookworm-updates\n", self.read("sources.list.d/debian.sources"), "the source file")

    def test_a_one_line_source_names_the_list_in_place_of_the_origin(self):
        self.source(one_line(self.ubuntu), "sources.list")
        rc, _, err = self.run_script([(self.ubuntu, [self.ubuntu])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(
            self.read("sources.list").splitlines()[:2],
            [f"deb mirror+file:{self.listing(0)} jammy main restricted", f"deb mirror+file:{self.listing(0)} jammy-updates main restricted"],
            "both lines of the origin",
        )

    def test_two_origins_get_two_lists_numbered_in_the_table_s_order(self):
        self.source(deb822(self.deb, self.deb_security))
        rc, _, err = self.run_script([(self.deb, [self.deb]), (self.deb_security, [self.deb_security])])
        self.assertEqual(rc, 0, err)
        text = self.read("sources.list.d/debian.sources")
        self.assertIn(f"URIs: mirror+file:{self.listing(0)}\nSuites: bookworm bookworm-updates\n", text, "the first origin")
        self.assertIn(f"URIs: mirror+file:{self.listing(1)}\nSuites: bookworm-security\n", text, "the second origin")

    def test_an_origin_the_system_does_not_read_gets_no_list_and_no_question(self):
        self.source(deb822(self.deb, self.deb_security))
        unread = f"{self.dead}/ubuntu/"
        rc, _, err = self.run_script([(unread, [unread]), (self.deb, [self.deb]), (self.deb_security, [self.deb_security])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(sorted(n for n in os.listdir(os.path.join(self.root, "etc/apt")) if n.endswith(".list")), ["docs-test-mirrors-0.list", "docs-test-mirrors-1.list"], "lists written")
        self.assertEqual(
            sorted(Mirrors.asked),
            ["/ok0/debian-security/dists/bookworm-security/InRelease", "/ok0/debian/dists/bookworm-updates/InRelease", "/ok0/debian/dists/bookworm/InRelease"],
            "files asked for",
        )

    def test_the_log_says_where_each_origin_s_packages_come_from(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.deb, self.at("ok2")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, self.come_from(self.deb, self.deb, self.at("ok2")) + "\n", "stdout")

    def test_apt_is_told_how_often_to_ask_again_as_the_how_to_runner_tells_it(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.deb])])
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
        rc, out, err = self.run_script(list(chosen.items()))
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, "".join(self.come_from(o, *m) + "\n" for o, m in chosen.items()), "every mirror kept")
        commands = dt.APT_RETRIES.replace("/etc/apt/", f"{other}/etc/apt/") + "".join(c + "\n" for c in dt.apt_mirror_setup(chosen, root=other))
        steps = os.path.join(self.dir, "runner.sh")
        with open(steps, "w", encoding="utf-8") as f:
            f.write("set -e\n" + commands)
        subprocess.run(["bash", steps], check=True, timeout=30)
        mine, runners = self.tree(self.root), self.tree(other)
        self.assertGreaterEqual(len(mine), 3, mine)
        self.assertEqual(mine, runners, "the files the script wrote and the files the how-to runner's commands wrote")

    def test_a_debian_system_ends_as_the_how_to_runner_leaves_it(self):
        self.same_as_the_runner(deb822(self.deb, self.deb_security), "sources.list.d/debian.sources", {
            self.deb: [self.at("ok1"), self.deb],
            self.deb_security: [self.deb_security, self.at("ok3", "debian-security")],
        })

    def test_an_ubuntu_system_ends_as_the_how_to_runner_leaves_it(self):
        self.same_as_the_runner(one_line(self.ubuntu, self.ubuntu_security), "sources.list", {
            self.ubuntu: [self.at("ok1", "ubuntu/"), self.ubuntu],
            self.ubuntu_security: [self.ubuntu_security, self.at("ok1", "ubuntu/")],
        })

    def test_an_ubuntu_ports_system_ends_as_the_how_to_runner_leaves_it(self):
        ports = self.at("ok0", "ubuntu-ports/")
        self.same_as_the_runner(one_line(ports, ports), "sources.list", {ports: [ports, self.at("ok2", "ubuntu-ports/")]})

    def test_without_a_table_named_the_one_beside_the_script_is_read(self):
        self.source(deb822(self.deb))
        copy = os.path.join(self.dir, "copy")
        os.makedirs(copy)
        shutil.copy(SCRIPT, copy)
        with open(os.path.join(copy, "apt-mirrors.txt"), "w", encoding="utf-8") as f:
            f.write(f"{self.deb} {self.deb}\n")
        done = subprocess.run(
            ["bash", os.path.join(copy, "apt-mirror-setup.sh"), "--root", self.root, "--timeout", str(ANSWER_WITHIN)],
            capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL, check=False,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertTrue(os.path.isfile(self.listing(0)), "the list from the table beside the script")
        self.assertTrue((HERE / "apt-mirrors.txt").is_file(), "the table the real script reads")

    def test_each_host_is_asked_for_the_release_file_of_each_suite_with_or_without_a_last_slash(self):
        self.source(one_line(self.ubuntu), "sources.list")
        rc, _, err = self.run_script([(self.ubuntu, [self.ubuntu, self.at("ok2", "pub/ubuntu")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(
            sorted(Mirrors.asked),
            ["/ok0/ubuntu/dists/jammy-updates/InRelease", "/ok0/ubuntu/dists/jammy/InRelease",
             "/ok2/pub/ubuntu/dists/jammy-updates/InRelease", "/ok2/pub/ubuntu/dists/jammy/InRelease"],
            "paths asked",
        )

    def test_a_mirror_of_two_origins_is_asked_once_for_each_suite(self):
        self.source(one_line(self.ubuntu, self.ubuntu_security), "sources.list")
        both = self.at("ok1", "ubuntu/")
        rc, _, err = self.run_script([(self.ubuntu, [both, self.ubuntu]), (self.ubuntu_security, [self.ubuntu_security, both])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(
            sorted(Mirrors.asked),
            ["/ok0/ubuntu/dists/jammy-updates/InRelease", "/ok0/ubuntu/dists/jammy/InRelease",
             "/ok1/ubuntu/dists/jammy-security/InRelease", "/ok1/ubuntu/dists/jammy-updates/InRelease", "/ok1/ubuntu/dists/jammy/InRelease",
             "/ok9/ubuntu/dists/jammy-security/InRelease"],
            "paths asked",
        )

    def test_an_http_1_1_answer_counts(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.deb, self.at("ok11")])])
        self.assertEqual(rc, 0, err)
        self.assertIn(f"then {self.at('ok11')}\n", out, "stdout")

    def test_the_official_host_comes_first_for_a_security_suite(self):
        self.source(one_line(self.ubuntu, self.ubuntu_security), "sources.list")
        rc, out, err = self.run_script([(self.ubuntu, [self.ubuntu]), (self.ubuntu_security, [self.at("ok1", "ubuntu/"), self.at("ok2", "ubuntu/"), self.ubuntu_security])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(1), [self.ubuntu_security, self.at("ok1", "ubuntu/"), self.at("ok2", "ubuntu/")], "the security list")

    def test_a_mirror_in_the_middle_of_a_sync_is_left_out_and_the_others_are_kept(self):
        # The 2026-11-23 case: run 45261208950 read jammy-security from a
        # mirror whose files were not the ones its release file named.
        self.source(one_line(self.ubuntu, self.ubuntu_security), "sources.list")
        stale = self.at("old1", "pub/ubuntu/")
        rc, out, err = self.run_script([(self.ubuntu, [self.ubuntu]), (self.ubuntu_security, [self.ubuntu_security, self.at("ok1", "ubuntu/"), stale])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(1), [self.ubuntu_security, self.at("ok1", "ubuntu/")], "the security list")
        self.assertIn(self.differs(stale, "jammy-security", self.ubuntu_security) + "\n", out, "the log names the mirror")

    def test_the_log_says_when_only_the_origin_remains(self):
        # Between two syncs of its mirrors the Ubuntu archive is often the
        # only host that serves its own release (measured 2026-11-23).
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.at("old1"), self.deb, self.at("old2")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(
            out.splitlines(),
            [self.differs(self.at("old1"), "bookworm", self.deb), self.differs(self.at("old2"), "bookworm", self.deb), self.only(self.deb), self.come_from(self.deb, self.deb)],
            "stdout: the hosts left out, then that one host remains",
        )

    def test_with_the_origin_down_the_mirrors_that_agree_on_the_newest_copy_are_kept(self):
        down = f"{self.dead}/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("ok1"), self.at("new1"), self.at("new2")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("new1"), self.at("new2")], "the list")
        self.assertEqual(
            out.splitlines(),
            [self.unreachable(down), self.older(self.at("ok1"), "bookworm"), self.come_from(down, self.at("new1"), self.at("new2"))],
            "stdout",
        )

    def test_with_the_origin_down_and_no_two_mirrors_agreeing_only_the_newest_is_kept(self):
        down = f"{self.dead}/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("old1"), self.at("new1"), self.at("ok1")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("new1")], "the list")
        self.assertIn(self.alone(down, self.at("new1")) + "\n", out, "stdout")
        self.assertIn(self.unreachable(down) + "\n", out, "stdout")

    def test_a_dry_run_asks_every_question_says_the_same_and_writes_nothing(self):
        self.source(deb822(self.deb))
        table = [(self.deb, [self.deb, self.at("old1"), self.at("ok2")])]
        rc, out, err = self.run_script(table, "--dry-run")
        self.assertEqual(rc, 0, err)
        self.untouched(deb822(self.deb))
        asked = sorted(Mirrors.asked)
        Mirrors.asked.clear()
        rc, wrote, err = self.run_script(table)
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, wrote, "the dry run's log and the real run's")
        self.assertEqual(asked, sorted(Mirrors.asked), "the questions of each")


class Failure(Case):
    def test_no_mirror_answering_ends_with_3_and_names_the_origin_and_each_mirror_tried(self):
        down = f"{self.dead}/a/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("gone")])])
        self.assertEqual(rc, 3, err)
        self.assertEqual(out, self.unreachable(down) + "\n", "stdout")
        self.assertEqual(
            err,
            f"{ME}: no package mirror answered for {down} (tried {down}, {self.at('gone')}): "
            "the package mirrors are unreachable from this machine\n",
            "stderr",
        )

    def test_a_run_that_ends_with_3_changes_nothing(self):
        down = f"{self.dead}/a/debian"
        self.source(deb822(down))
        rc, _, err = self.run_script([(down, [down])])
        self.assertEqual(rc, 3, err)
        self.untouched(deb822(down))

    def test_a_second_origin_with_no_mirror_leaves_the_first_untouched_too(self):
        down = f"{self.dead}/s"
        self.source(deb822(self.deb, down))
        rc, _, err = self.run_script([(self.deb, [self.deb]), (down, [down])])
        self.assertEqual(rc, 3, err)
        self.assertIn(f"no package mirror answered for {down} ", err, "stderr")
        self.untouched(deb822(self.deb, down))

    def test_a_mirror_that_answers_404_is_left_out(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.at("gone"), self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, self.only(self.deb) + "\n" + self.come_from(self.deb, self.deb) + "\n", "stdout")

    def test_a_mirror_that_redirects_is_left_out(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.at("moved"), self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertNotIn("moved", out, "stdout")
        self.assertIn("/moved/debian/dists/bookworm/InRelease", Mirrors.asked, "it was asked")

    def test_a_mirror_that_answers_late_within_the_limit_is_compared(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.at("late"), self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("late"), self.deb], "the late mirror is kept, in its place")

    def test_a_mirror_that_sends_nothing_is_left_out_within_the_time_limit(self):
        self.source(deb822(self.deb))
        start = time.monotonic()
        rc, out, err = self.run_script([(self.deb, [self.at("silent"), self.deb])], timeout=str(SILENT_LIMIT))
        took = time.monotonic() - start
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, self.only(self.deb) + "\n" + self.come_from(self.deb, self.deb) + "\n", "stdout")
        self.assertGreaterEqual(took, SILENT_LIMIT - 0.5, "the run waited for the silent host up to its limit")
        self.assertLess(took, SILENT - 5, f"the {SILENT_LIMIT} s limit, not the {SILENT} s the mirror sleeps")

    def test_a_host_that_does_not_speak_http_is_left_out(self):
        noise = self.at("noise")
        self.source(deb822(noise))
        rc, _, err = self.run_script([(noise, [noise])])
        self.assertEqual(rc, 3, err)

    def test_a_status_that_only_starts_with_200_is_left_out(self):
        almost = self.at("almost")
        self.source(deb822(almost))
        rc, _, err = self.run_script([(almost, [almost])])
        self.assertEqual(rc, 3, err)

    def test_an_empty_release_file_is_not_an_answer(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.deb, self.at("empty")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, self.only(self.deb) + "\n" + self.come_from(self.deb, self.deb) + "\n", "stdout")

    def test_a_mirror_that_lacks_one_suite_s_release_file_is_left_out(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.deb, self.at("half1")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.deb], "the list")
        self.assertIn("/half1/debian/dists/bookworm/InRelease", Mirrors.asked, "it was asked")

    def test_a_mirror_whose_release_file_differs_from_the_origin_s_is_left_out_and_named(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.at("new1"), self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.deb], "a newer copy than the origin's is left out too")
        self.assertEqual(out.splitlines(), [self.differs(self.at("new1"), "bookworm", self.deb), self.only(self.deb), self.come_from(self.deb, self.deb)], "stdout")

    def test_a_mirror_that_differs_in_one_suite_of_two_is_left_out(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.deb, self.at("mixa")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.deb], "the list")
        self.assertIn(self.differs(self.at("mixa"), "bookworm", self.deb) + "\n", out, "the first suite that differs is named")

    def test_a_mirror_with_the_origin_s_date_and_other_bytes_is_left_out(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.deb, self.at("fork1")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.deb], "the list")
        self.assertIn(self.differs(self.at("fork1"), "bookworm", self.deb), out, "stdout")

    def test_with_the_origin_down_an_older_copy_is_never_kept_even_when_two_agree(self):
        down = f"{self.dead}/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("old1"), self.at("old2"), self.at("new1")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("new1")], "the list")
        self.assertIn(self.older(self.at("old1"), "bookworm") + "\n", out, "stdout")
        self.assertIn(self.older(self.at("old2"), "bookworm") + "\n", out, "stdout")

    def test_with_the_origin_down_and_no_mirror_newest_in_every_suite_ends_with_3(self):
        down = f"{self.dead}/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("mixa"), self.at("mixb")])])
        self.assertEqual(rc, 3, err)
        self.assertEqual(
            err,
            f"{ME}: no mirror of {down} holds the newest InRelease of every suite (tried {down}, {self.at('mixa')}, {self.at('mixb')}): "
            "apt would read some of its files from an older copy\n",
            "stderr",
        )
        self.untouched(deb822(down))

    def test_sources_the_table_does_not_know_end_with_4(self):
        text = "deb http://packages.example.com/debian bookworm main\n"
        self.source(text, "sources.list")
        rc, out, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 4, err)
        self.assertEqual(out, "", "stdout")
        self.assertIn(f"{ME}: no package source of this system is in ", err, "stderr")
        self.assertEqual(Mirrors.asked, [], "mirrors asked")
        self.untouched(text, "sources.list")

    def test_a_system_with_no_source_file_ends_with_4(self):
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 4, err)

    def test_a_table_that_is_not_there_ends_with_2(self):
        self.source(deb822(self.deb))
        done = subprocess.run(
            ["bash", str(SCRIPT), "--root", self.root, "--table", os.path.join(self.dir, "none.txt")],
            capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL, check=False,
        )
        self.assertEqual(done.returncode, 2, done.stderr)
        self.assertIn("none.txt", done.stderr, "stderr")
        self.untouched(deb822(self.deb))

    def test_an_option_the_script_does_not_have_ends_with_2(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.deb])], "--fast")
        self.assertEqual(rc, 2, err)
        self.assertIn("--fast", err, "stderr")
        self.untouched(deb822(self.deb))

    def test_an_origin_with_no_mirror_in_the_table_ends_with_2(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script(f"{self.deb}\n")
        self.assertEqual(rc, 2, err)
        self.assertIn(f"table.txt:1: {self.deb} has no mirror", err, "stderr")
        self.untouched(deb822(self.deb))

    def test_an_origin_that_is_not_among_its_own_mirrors_ends_with_2(self):
        # The origin's release file is what every mirror is held to, so the
        # table cannot leave the origin out, as docs_test.load_apt_mirrors
        # refuses it too.
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.at("ok1")])])
        self.assertEqual(rc, 2, err)
        self.assertIn(f"table.txt:1: {self.deb} is not among its own mirrors", err, "stderr")
        self.assertEqual(Mirrors.asked, [], "mirrors asked")
        self.untouched(deb822(self.deb))

    def test_a_mirror_that_is_not_an_http_address_ends_with_2(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.deb, "https://deb.example.com/debian"])])
        self.assertEqual(rc, 2, err)
        self.assertIn("table.txt:1: https://deb.example.com/debian is not an http:// address", err, "stderr")
        self.assertEqual(Mirrors.asked, [], "mirrors asked")
        self.untouched(deb822(self.deb))

    def test_a_time_limit_that_is_not_a_number_ends_with_2(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.deb])], timeout="soon")
        self.assertEqual(rc, 2, err)
        self.untouched(deb822(self.deb))

    def test_an_option_without_its_value_ends_with_2(self):
        self.source(deb822(self.deb))
        done = subprocess.run(["bash", str(SCRIPT), "--root"], capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL, check=False)
        self.assertEqual(done.returncode, 2, done.stderr)


class Edge(Case):
    def test_a_first_mirror_that_is_down_is_left_out_and_the_rest_move_up(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [f"{self.dead}/debian", self.at("ok1"), self.at("gone"), self.deb])])
        self.assertEqual(rc, 0, err)
        with open(self.listing(0), encoding="utf-8") as f:
            self.assertEqual(f.read(), f"{self.at('ok1')}\tpriority:1\n{self.deb}\tpriority:2\n", "the list")

    def test_comments_and_empty_lines_in_the_table_are_passed_over(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script(f"# the mirrors\n\n   # indented\n{self.deb}   {self.deb}\n")
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, self.come_from(self.deb, self.deb) + "\n", "stdout")

    def test_an_origin_matches_with_or_without_its_last_slash(self):
        self.source(f"deb {self.ubuntu.rstrip('/')} jammy main\ndeb {self.deb}/ bookworm main\n", "sources.list")
        rc, _, err = self.run_script([(self.ubuntu, [self.ubuntu]), (self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(
            self.read("sources.list"),
            f"deb mirror+file:{self.listing(0)} jammy main\ndeb mirror+file:{self.listing(1)} bookworm main\n",
            "the source file",
        )
        self.assertIn("/ok0/ubuntu/dists/jammy/InRelease", Mirrors.asked, "the suite of the line without a slash")

    def test_an_origin_that_starts_another_does_not_take_its_lines(self):
        self.source(deb822(self.deb, self.deb_security))
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertIn(f"URIs: {self.deb_security}\n", self.read("sources.list.d/debian.sources"), "the security source")
        self.assertNotIn("/ok0/debian/dists/bookworm-security/InRelease", Mirrors.asked, "the other origin's suite")

    def test_only_the_longer_origin_in_the_sources_is_not_taken_for_the_shorter(self):
        text = f"deb {self.deb_security} bookworm-security main\n"
        self.source(text, "sources.list")
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 4, err)
        self.assertEqual(self.read("sources.list"), text, "the source file")

    def test_a_dot_in_an_origin_is_a_dot(self):
        text = f"deb {self.deb.replace('127.0.0.1', '127X0X0X1')} bookworm main\n"
        self.source(text, "sources.list")
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 4, err)
        self.assertEqual(self.read("sources.list"), text, "the source file")

    def test_both_kinds_of_source_file_are_rewritten(self):
        self.source(f"deb {self.deb} bookworm main\n", "sources.list")
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.read("sources.list"), f"deb mirror+file:{self.listing(0)} bookworm main\n", "sources.list")
        self.assertIn(f"URIs: mirror+file:{self.listing(0)}\n", self.read("sources.list.d/debian.sources"), "debian.sources")

    def test_a_second_run_changes_nothing_and_ends_with_0(self):
        self.source(deb822(self.deb, self.deb_security))
        table = [(self.deb, [self.deb]), (self.deb_security, [self.deb_security])]
        rc, _, err = self.run_script(table)
        self.assertEqual(rc, 0, err)
        before = self.tree(self.root)
        Mirrors.asked.clear()
        rc, out, err = self.run_script(table)
        self.assertEqual(rc, 0, err)
        self.assertEqual(out, f"{ME}: this system's sources already read their mirror lists\n", "stdout")
        self.assertEqual(self.tree(self.root), before, "the files after the second run")
        self.assertEqual(Mirrors.asked, [], "mirrors asked by the second run")

    def test_a_directory_among_the_source_files_is_passed_over(self):
        self.source(deb822(self.deb))
        os.makedirs(os.path.join(self.root, "etc/apt/sources.list.d/old"))
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertTrue(os.path.isfile(self.listing(0)), "the list")
        self.assertEqual(err, "", "stderr")

    def test_the_script_reads_nothing_from_its_standard_input(self):
        self.source(deb822(self.deb, self.deb_security))
        self.write_table([(self.deb, [self.deb]), (self.deb_security, [self.deb_security])])
        done = subprocess.run(
            ["bash", str(SCRIPT), "--root", self.root, "--table", self.table, "--timeout", str(ANSWER_WITHIN)],
            capture_output=True, text=True, timeout=60, input="one\ntwo\n", check=False,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(len(done.stdout.splitlines()), 2, done.stdout)

    def test_a_commented_source_line_names_no_suite(self):
        self.source(f"# deb {self.deb} bookworm-proposed main\ndeb {self.deb} bookworm main # the release\n", "sources.list")
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(sorted(Mirrors.asked), ["/ok0/debian/dists/bookworm/InRelease"], "paths asked")

    def test_a_deb822_address_with_a_last_slash_is_the_origin_without_one(self):
        self.source(deb822(self.deb + "/"))
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(sorted(Mirrors.asked), ["/ok0/debian/dists/bookworm-updates/InRelease", "/ok0/debian/dists/bookworm/InRelease"], "paths asked")
        self.assertIn(f"URIs: mirror+file:{self.listing(0)}\n", self.read("sources.list.d/debian.sources"), "the source file")

    def test_a_comment_inside_a_deb822_stanza_does_not_end_it(self):
        self.source(f"Types: deb\nURIs: {self.deb}\n# the suites this system reads\nSuites: bookworm\nComponents: main\n")
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(Mirrors.asked, ["/ok0/debian/dists/bookworm/InRelease"], "paths asked")

    def test_options_in_brackets_before_the_address_are_passed_over(self):
        self.source(f"deb [arch=amd64 signed-by=/usr/share/keyrings/k.gpg] {self.deb} bookworm main\ndeb [ arch=arm64 ] {self.deb} bookworm-updates main\n", "sources.list")
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(sorted(Mirrors.asked), ["/ok0/debian/dists/bookworm-updates/InRelease", "/ok0/debian/dists/bookworm/InRelease"], "paths asked")

    def test_a_suite_named_by_several_lines_is_asked_for_once(self):
        self.source(f"deb {self.deb} bookworm main\ndeb-src {self.deb} bookworm main\ndeb {self.deb}/ bookworm contrib\n", "sources.list")
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(Mirrors.asked, ["/ok0/debian/dists/bookworm/InRelease"], "paths asked")

    def test_a_deb822_stanza_with_two_addresses_gives_its_suites_to_each(self):
        other = self.at("ok5")
        self.source(f"Types: deb deb-src\nURIs: {other} {self.deb}\nSuites: bookworm\nComponents: main\n")
        rc, _, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(Mirrors.asked, ["/ok0/debian/dists/bookworm/InRelease"], "paths asked")

    def test_an_origin_listed_alone_has_no_line_saying_it_alone_remains(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertNotIn(" itself remains", out, "nothing was left out")

    def test_no_line_saying_the_origin_alone_remains_while_another_mirror_is_kept(self):
        self.source(deb822(self.deb))
        rc, out, err = self.run_script([(self.deb, [self.at("old1"), self.deb, self.at("ok1")])])
        self.assertEqual(rc, 0, err)
        self.assertNotIn(" itself remains", out, "stdout")

    def test_a_security_origin_left_alone_says_so_once(self):
        self.source(one_line(self.ubuntu, self.ubuntu_security), "sources.list")
        rc, out, err = self.run_script([(self.ubuntu, [self.ubuntu, self.at("ok1", "ubuntu/")]), (self.ubuntu_security, [self.at("old1", "ubuntu/"), self.ubuntu_security])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(out.count(" itself remains"), 1, out)
        self.assertIn(self.only(self.ubuntu_security) + "\n", out, "stdout")

    def test_with_the_origin_down_and_one_mirror_kept_no_line_names_the_origin_as_remaining(self):
        down = f"{self.dead}/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("new1"), self.at("ok1")])])
        self.assertEqual(rc, 0, err)
        self.assertNotIn(" itself remains", out, "the origin is not among the hosts kept")

    def test_an_origin_without_a_security_suite_keeps_the_table_s_order(self):
        self.source(deb822(self.deb))
        rc, _, err = self.run_script([(self.deb, [self.at("ok1"), self.deb])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("ok1"), self.deb], "the list")

    def test_one_security_suite_among_others_puts_the_origin_first(self):
        # ubuntu-ports serves the security suite from the same host as the rest.
        ports = self.at("ok0", "ubuntu-ports/")
        self.source(one_line(ports, ports), "sources.list")
        rc, _, err = self.run_script([(ports, [self.at("ok2", "ubuntu-ports/"), ports])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [ports, self.at("ok2", "ubuntu-ports/")], "the list")

    def test_with_the_origin_down_two_newest_copies_that_differ_keep_only_the_first(self):
        down = f"{self.dead}/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("fork1"), self.at("ok1")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("fork1")], "the first in the table's order")
        self.assertIn(self.alone(down, self.at("fork1")) + "\n", out, "stdout")
        self.assertIn(self.not_kept(self.at("ok1"), "bookworm", self.at("fork1")) + "\n", out, "the other is named")

    def test_with_the_origin_down_a_tie_of_two_groups_keeps_the_group_listed_first(self):
        down = f"{self.dead}/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("twin1"), self.at("new1"), self.at("twin2"), self.at("new2")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("twin1"), self.at("twin2")], "the list")
        self.assertIn(self.not_kept(self.at("new1"), "bookworm", self.at("twin1")) + "\n", out, "stdout")
        self.assertIn(self.not_kept(self.at("new2"), "bookworm", self.at("twin1")) + "\n", out, "stdout")

    def test_with_the_origin_down_a_release_file_without_a_date_is_the_oldest(self):
        down = f"{self.dead}/debian"
        self.source(deb822(down))
        rc, out, err = self.run_script([(down, [down, self.at("nodate1"), self.at("old1")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("old1")], "the list")
        self.assertIn(self.older(self.at("nodate1"), "bookworm") + "\n", out, "stdout")

    def test_with_the_origin_answering_404_it_counts_as_down(self):
        gone = self.at("gone")
        self.source(deb822(gone))
        rc, out, err = self.run_script([(gone, [gone, self.at("new1"), self.at("new2")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(0), [self.at("new1"), self.at("new2")], "the list")
        self.assertIn(self.unreachable(gone) + "\n", out, "stdout")

    def test_with_the_origin_down_a_security_list_has_no_origin_to_put_first(self):
        down = f"{self.dead}/ubuntu/"
        self.source(one_line(self.ubuntu, down), "sources.list")
        rc, out, err = self.run_script([(self.ubuntu, [self.ubuntu]), (down, [down, self.at("new2", "ubuntu/"), self.at("new1", "ubuntu/")])])
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.listed(1), [self.at("new2", "ubuntu/"), self.at("new1", "ubuntu/")], "the table's order")

    def real_rows(self):
        return [line.split() for line in (HERE / "apt-mirrors.txt").read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#")]

    def test_the_real_table_gives_every_origin_itself_and_one_other_host(self):
        rows = self.real_rows()
        self.assertEqual(len(rows), 5, rows)
        for origin, *mirrors in rows:
            self.assertIn(origin, mirrors, origin)
            self.assertGreaterEqual(len(set(mirrors)), 2, origin)
            for m in mirrors:
                self.assertTrue(m.startswith("http://"), m)

    def test_the_real_table_lists_each_security_origin_first(self):
        security = [row for row in self.real_rows() if "security" in row[0]]
        self.assertEqual(len(security), 2, security)
        for origin, *mirrors in security:
            self.assertEqual(mirrors[0], origin, origin)

    def test_the_real_table_is_the_how_to_runner_s_table(self):
        rows = self.real_rows()
        self.assertEqual({origin: tuple(mirrors) for origin, *mirrors in rows}, dt.APT_MIRRORS, "scripts/apt-mirrors.txt and docs_test.APT_MIRRORS")
        self.assertEqual([origin for origin, *_ in rows], list(dt.APT_MIRRORS), "the order of the origins")


class Runner(Case):
    """docs_test.choose_mirrors, the how-to runner's choice of mirrors, is
    this script run on the runner's host: one rule for both."""

    def choose(self, suites, table):
        self.write_table(table)
        return dt.choose_mirrors(suites, ask=lambda s: dt.ask_mirrors(s, table=self.table, timeout=ANSWER_WITHIN))

    def test_the_runner_keeps_the_mirrors_the_script_keeps(self):
        chosen, notes = self.choose({self.deb: ("bookworm",)}, [(self.deb, [self.at("ok1"), self.at("old1"), self.deb])])
        self.assertEqual(chosen, {self.deb: [self.at("ok1"), self.deb]}, "the chosen mirrors")
        self.assertEqual(notes, ["docs-test: " + self.differs(self.at("old1"), "bookworm", self.deb).split(": ", 1)[1]], "the script's notes")

    def test_the_runner_s_log_says_when_only_the_origin_remains(self):
        chosen, notes = self.choose({self.deb: ("trixie",)}, [(self.deb, [self.at("old1"), self.deb])])
        self.assertEqual(chosen, {self.deb: [self.deb]}, "the chosen mirrors")
        self.assertEqual(notes[-1], "docs-test: " + self.only(self.deb).split(": ", 1)[1], "the last note")

    def test_the_runner_asks_for_the_suites_it_names(self):
        self.choose({self.deb: ("trixie", "trixie-updates"), self.deb_security: ("trixie-security",)}, [(self.deb, [self.deb]), (self.deb_security, [self.deb_security])])
        self.assertEqual(
            sorted(Mirrors.asked),
            ["/ok0/debian-security/dists/trixie-security/InRelease", "/ok0/debian/dists/trixie-updates/InRelease", "/ok0/debian/dists/trixie/InRelease"],
            "paths asked",
        )

    def test_the_runner_keeps_the_newest_agreeing_copies_when_the_origin_is_down(self):
        down = f"{self.dead}/debian"
        chosen, notes = self.choose({down: ("trixie",)}, [(down, [down, self.at("new1"), self.at("ok1"), self.at("new2")])])
        self.assertEqual(chosen, {down: [self.at("new1"), self.at("new2")]}, "the chosen mirrors")
        self.assertIn("docs-test: " + self.unreachable(down).split(": ", 1)[1], notes, "the notes")

    def test_the_runner_puts_the_official_host_first_for_a_security_suite(self):
        chosen, _ = self.choose({self.ubuntu_security: ("noble-security",)}, [(self.ubuntu_security, [self.at("ok1", "ubuntu/"), self.ubuntu_security])])
        self.assertEqual(chosen, {self.ubuntu_security: [self.ubuntu_security, self.at("ok1", "ubuntu/")]}, "the chosen mirrors")

    def test_no_mirror_answering_is_an_unreachable_mirror_for_the_runner(self):
        down = f"{self.dead}/debian"
        with self.assertRaises(dt.MirrorError) as raised:
            self.choose({down: ("trixie",)}, [(down, [down, self.at("gone")])])
        said = str(raised.exception)
        self.assertEqual(
            said,
            f"no package mirror answered for {down} (tried {down}, {self.at('gone')}): "
            "the package mirrors are unreachable from this machine; this is not a failure of the pages",
        )

    def test_no_mirror_newest_in_every_suite_is_an_unreachable_mirror_for_the_runner(self):
        down = f"{self.dead}/debian"
        with self.assertRaisesRegex(dt.MirrorError, "holds the newest InRelease of every suite"):
            self.choose({down: ("trixie", "trixie-updates")}, [(down, [down, self.at("mixa"), self.at("mixb")])])

    def test_the_runner_s_question_leaves_no_file_behind(self):
        scratch = os.path.join(self.dir, "tmp")
        os.makedirs(scratch)
        with unittest.mock.patch.object(tempfile, "tempdir", scratch), unittest.mock.patch.dict(os.environ, {"TMPDIR": scratch}):
            chosen, _ = self.choose({self.deb: ("trixie",)}, [(self.deb, [self.deb])])
        self.assertEqual(chosen, {self.deb: [self.deb]}, "the chosen mirrors")
        self.assertEqual(os.listdir(scratch), [], "files left in the temporary directory, by the runner or the script")

    def test_a_table_the_script_refuses_is_a_page_error_not_an_outage(self):
        with self.assertRaises(dt.PageError) as raised:
            self.choose({self.deb: ("trixie",)}, [(self.deb, [self.at("ok1")])])
        self.assertNotIsInstance(raised.exception, dt.MirrorError)
        self.assertIn("exited 2", str(raised.exception))
        self.assertIn("is not among its own mirrors", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
