"""Tests scripts/apt-get-retry.sh: `apt-get update` run again, after a
wait, when a mirror was in the middle of a sync, and every other apt-get
command run once, as it is.

apt itself cannot be driven here (no mirror is mid-sync on demand), so each
test stands a fake apt-get in its place that answers from a plan, and a
fake sleep that records each wait: the decision is what is tested.

Run with: python3 -I scripts/test_apt_get_retry.py
"""

import base64
import importlib.util
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
SCRIPT = HERE / "apt-get-retry.sh"
spec = importlib.util.spec_from_file_location("docs_test", HERE / "docs_test.py")
docs_test = importlib.util.module_from_spec(spec)
# Its dataclasses look their module up by name.
sys.modules["docs_test"] = docs_test
spec.loader.exec_module(docs_test)

# What apt 2.7 (Ubuntu 24.04) printed on main's run 45805685940 on
# 2026-11-26, with the URL shortened, and the shape of its hash form.
SIZE = (
    "E: Failed to fetch http://security.ubuntu.com/ubuntu/dists/noble-security/main/binary-amd64/by-hash/SHA256/x"
    "  File has unexpected size (1513652 != 1513649). Mirror sync in progress? [IP: 192.0.2.10 80]\n"
    "E: Some index files failed to download. They have been ignored, or old ones used instead.\n"
)
HASH = (
    "E: Failed to fetch http://deb.debian.org/debian/dists/bookworm/main/binary-amd64/Packages.xz"
    "  Hash Sum mismatch\n"
    "   Hashes of expected file:\n"
    "E: Some index files failed to download. They have been ignored, or old ones used instead.\n"
)
RESOLVE = "E: Failed to fetch http://archive.ubuntu.com/ubuntu/dists/noble/InRelease  Temporary failure resolving 'archive.ubuntu.com'\n"

# The fake apt-get: each run answers with the next line of plan.txt (the
# last one from then on): an exit code, a tab, and the text it writes to
# stderr, with \n for a line end. It writes "Get:1 <args>" on stdout and
# records its arguments, one run a line, in runs.txt.
FAKE_APT = r"""#!/bin/sh
dir=$(dirname "$0")
n=$(wc -l < "$dir/runs.txt")
printf '%s\n' "$*" >> "$dir/runs.txt"
total=$(wc -l < "$dir/plan.txt")
line=$((n + 1))
[ "$line" -gt "$total" ] && line=$total
answer=$(sed -n "${line}p" "$dir/plan.txt")
code=${answer%%	*}
text=${answer#*	}
printf 'Get:1 %s\n' "$*"
printf '%b' "$text" >&2
exit "$code"
"""

# The fake sleep: records each wait in waits.txt and does not wait.
FAKE_SLEEP = """#!/bin/sh
printf '%s\\n' "$1" >> "$(dirname "$0")/waits.txt"
"""


class Case(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.path = pathlib.Path(self.dir.name)
        for name, text in (("apt-get", FAKE_APT), ("sleep", FAKE_SLEEP)):
            p = self.path / name
            p.write_text(text)
            p.chmod(0o755)
        (self.path / "runs.txt").write_text("")
        (self.path / "waits.txt").write_text("")

    def plan(self, *answers):
        """Each run's answer: (exit code, stderr text)."""
        lines = [f"{code}\t{text.replace(chr(10), chr(92) + 'n')}" for code, text in answers]
        (self.path / "plan.txt").write_text("\n".join(lines) + "\n")

    def run_it(self, *args, delays=None, real=None):
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "APT_GET_REAL": real or str(self.path / "apt-get"),
            "APT_GET_RETRY_SLEEP": str(self.path / "sleep"),
        }
        if delays is not None:
            env["APT_UPDATE_DELAYS"] = delays
        return subprocess.run(
            ["bash", str(SCRIPT), *args],
            capture_output=True,
            text=True,
            env=env,
            timeout=60,
            check=False,
        )

    def runs(self):
        return (self.path / "runs.txt").read_text().splitlines()

    def waits(self):
        return (self.path / "waits.txt").read_text().split()


class Success(Case):
    def test_an_update_that_works_runs_once(self):
        self.plan((0, ""))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(self.runs(), ["update"], "one run")
        self.assertEqual(self.waits(), [], "no wait")

    def test_an_update_that_meets_a_mirror_mid_sync_once_runs_again_and_works(self):
        self.plan((100, SIZE), (0, ""))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(len(self.runs()), 2, "two runs")

    def test_an_update_that_meets_it_twice_works_on_the_third_run(self):
        self.plan((100, SIZE), (100, SIZE), (0, ""))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(len(self.runs()), 3, "three runs")

    def test_the_waits_are_15_30_and_60_seconds_by_default(self):
        self.plan((100, SIZE))
        self.run_it("update")
        self.assertEqual(self.waits(), ["15", "30", "60"], "about two minutes in all")

    def test_the_waits_come_from_apt_update_delays_when_it_is_set(self):
        self.plan((100, SIZE))
        self.run_it("update", delays="1 2")
        self.assertEqual(self.waits(), ["1", "2"], "the delays given")

    def test_apt_s_standard_output_passes_through(self):
        self.plan((100, SIZE), (0, ""))
        out = self.run_it("update")
        self.assertEqual(out.stdout, "Get:1 update\nGet:1 update\n", "each run's lines, in order")

    def test_apt_s_errors_pass_through_on_standard_error(self):
        self.plan((100, SIZE), (0, ""))
        out = self.run_it("update")
        self.assertIn("Mirror sync in progress?", out.stderr, "apt's own text is shown")

    def test_the_arguments_reach_apt_get_as_given(self):
        self.plan((100, SIZE), (0, ""))
        self.run_it("-o", "Acquire::Retries=3", "update")
        self.assertEqual(self.runs(), ["-o Acquire::Retries=3 update"] * 2, "the same arguments each run")

    def test_a_hash_sum_mismatch_is_tried_again(self):
        self.plan((100, HASH), (0, ""))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(len(self.runs()), 2, "two runs")

    def test_each_wait_is_logged_with_its_try(self):
        self.plan((100, SIZE), (0, ""))
        out = self.run_it("update")
        self.assertIn(
            "apt-get-retry: a mirror was in the middle of a sync (try 1 of 4); the update runs again in 15 s",
            out.stderr,
        )

    def test_a_quiet_update_is_tried_again_too(self):
        self.plan((100, SIZE), (0, ""))
        out = self.run_it("-qq", "update")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(self.runs(), ["-qq update"] * 2, "two quiet runs")

    def test_an_update_that_works_at_last_says_no_more(self):
        self.plan((100, SIZE), (0, ""))
        out = self.run_it("update")
        self.assertNotIn("still", out.stderr, "no line of a failure")

    def test_an_install_runs_once_with_its_arguments(self):
        self.plan((0, ""))
        out = self.run_it("install", "-y", "--no-install-recommends", "curl")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(self.runs(), ["install -y --no-install-recommends curl"], "once, as given")


class Failure(Case):
    def test_a_mirror_mid_sync_on_every_try_fails_with_apt_s_code_after_four_runs(self):
        self.plan((100, SIZE))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 100, "apt's exit code")
        self.assertEqual(len(self.runs()), 4, "the first run and three more")

    def test_the_last_failure_says_how_many_tries_were_made(self):
        self.plan((100, SIZE))
        out = self.run_it("update")
        self.assertIn("apt-get-retry: a mirror was still in the middle of a sync after 4 tries", out.stderr)

    def test_a_name_that_does_not_resolve_is_not_tried_again(self):
        self.plan((100, RESOLVE))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 100, "apt's exit code")
        self.assertEqual(len(self.runs()), 1, "one run")
        self.assertEqual(self.waits(), [], "no wait")

    def test_another_exit_code_is_kept_and_not_tried_again(self):
        self.plan((1, "E: Could not open lock file /var/lib/apt/lists/lock\n"))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 1, "apt's exit code")
        self.assertEqual(len(self.runs()), 1, "one run")

    def test_an_install_that_meets_a_mirror_mid_sync_is_not_tried_again(self):
        self.plan((100, SIZE))
        out = self.run_it("install", "-y", "curl")
        self.assertEqual(out.returncode, 100, "apt's exit code")
        self.assertEqual(len(self.runs()), 1, "an install is not the update's rule")

    def test_a_timeout_s_end_is_not_tried_again(self):
        self.plan((124, SIZE))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 124, "the code kept")
        self.assertEqual(len(self.runs()), 1, "a run ended from outside is not a sync")

    def test_an_apt_get_that_is_not_there_ends_with_127(self):
        out = self.run_it("update", real=str(self.path / "no-such-apt-get"))
        self.assertEqual(out.returncode, 127, out.stderr)

    def test_delays_that_are_not_numbers_end_with_2_and_run_nothing(self):
        self.plan((0, ""))
        out = self.run_it("update", delays="15 soon")
        self.assertEqual(out.returncode, 2, out.stderr)
        self.assertIn("APT_UPDATE_DELAYS", out.stderr, "the setting is named")
        self.assertEqual(self.runs(), [], "apt-get not run")

    def test_a_failure_after_a_retry_that_is_not_a_sync_ends_there(self):
        self.plan((100, SIZE), (100, RESOLVE), (0, ""))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 100, "the second failure's code")
        self.assertEqual(len(self.runs()), 2, "no third run")

    def test_the_sync_text_on_standard_output_only_is_not_a_sync(self):
        # apt writes its errors on stderr; a package's name or description
        # on stdout is not one.
        self.plan((100, RESOLVE))
        out = self.run_it("update", "Mirror-sync-in-progress")
        self.assertEqual(len(self.runs()), 1, f"one run: {out.stdout}")

    def test_a_warning_with_the_sync_text_and_exit_0_is_a_success(self):
        self.plan((0, "W: Mirror sync in progress? said once\n"))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(len(self.runs()), 1, "a run that worked is not tried again")

    def test_a_crash_of_apt_get_is_not_tried_again(self):
        self.plan((139, ""))
        out = self.run_it("update")
        self.assertEqual(out.returncode, 139, "the code kept")
        self.assertEqual(len(self.runs()), 1, "one run")


class Edge(Case):
    def test_no_delays_means_no_retry(self):
        self.plan((100, SIZE))
        out = self.run_it("update", delays="")
        self.assertEqual(out.returncode, 100, "apt's code")
        self.assertEqual(len(self.runs()), 1, "one run")

    def test_one_delay_means_one_retry(self):
        self.plan((100, SIZE))
        self.run_it("update", delays="5")
        self.assertEqual(len(self.runs()), 2, "two runs")
        self.assertEqual(self.waits(), ["5"], "one wait")

    def test_update_after_options_with_values_is_still_the_update(self):
        self.plan((100, SIZE), (0, ""))
        self.run_it("-o", "APT::Get::List-Cleanup=0", "-q", "update")
        self.assertEqual(len(self.runs()), 2, "two runs")

    def test_upgrade_is_not_update(self):
        self.plan((100, SIZE))
        self.run_it("upgrade")
        self.assertEqual(len(self.runs()), 1, "one run")

    def test_dist_upgrade_is_not_update(self):
        self.plan((100, SIZE))
        self.run_it("dist-upgrade", "-y")
        self.assertEqual(len(self.runs()), 1, "one run")

    def test_no_argument_runs_apt_get_once(self):
        self.plan((100, ""))
        out = self.run_it()
        self.assertEqual(out.returncode, 100, "apt's code")
        self.assertEqual(self.runs(), [""], "once, with nothing")

    def test_the_text_of_the_sync_is_matched_as_apt_writes_it(self):
        self.plan((100, "E: mirror sync in progress\n"))
        self.run_it("update")
        self.assertEqual(len(self.runs()), 1, "apt's own capitals only")

    def test_a_delay_of_0_still_runs_again(self):
        self.plan((100, SIZE), (0, ""))
        out = self.run_it("update", delays="0")
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(self.waits(), ["0"], "a wait of 0")

    def test_an_argument_with_spaces_reaches_apt_get_whole(self):
        self.plan((0, ""))
        self.run_it("-o", "Dir::Etc::sourcelist=a b", "update")
        self.assertEqual(self.runs(), ["-o Dir::Etc::sourcelist=a b update"], "as given")

    def test_the_script_reads_nothing_from_its_standard_input(self):
        self.plan((0, ""))
        out = subprocess.run(
            ["bash", str(SCRIPT), "update"],
            input="update\n",
            capture_output=True,
            text=True,
            env={"PATH": os.environ.get("PATH", ""), "APT_GET_REAL": str(self.path / "apt-get")},
            timeout=60,
            check=False,
        )
        self.assertEqual(out.returncode, 0, out.stderr)

    def test_the_script_parses_as_bash(self):
        out = subprocess.run(["bash", "-n", str(SCRIPT)], capture_output=True, text=True, timeout=60, check=False)
        self.assertEqual(out.returncode, 0, out.stderr)

    def test_it_never_calls_itself_by_its_name(self):
        text = SCRIPT.read_text()
        self.assertIn('APT_GET_REAL:-/usr/bin/apt-get', text, "the real apt-get is named by its path")


class Wiring(unittest.TestCase):
    """Where the rule runs: the how-to runner's apt containers and the
    release's smoke job."""

    def scenario(self, where, tags):
        return docs_test.Scenario("t", where, frozenset(tags), ("install.md",), binary=False)

    def installed(self, command):
        """The text the install command writes, decoded from its base64."""
        found = re.search(r"printf %s ([A-Za-z0-9+/=]+) \| base64 -d > (\S+)", command)
        self.assertIsNotNone(found, f"a base64 install: {command[:120]}")
        return base64.b64decode(found.group(1)).decode(), found.group(2)

    def test_the_install_command_writes_the_script_s_own_text_as_apt_get(self):
        text, path = self.installed(docs_test.apt_get_retry_install())
        self.assertEqual(text, SCRIPT.read_text(), "byte for byte")
        self.assertEqual(path, "/usr/local/sbin/apt-get", "where root's PATH finds it before /usr/bin")

    def test_the_install_command_makes_it_executable(self):
        self.assertIn("chmod 0755 /usr/local/sbin/apt-get", docs_test.apt_get_retry_install())

    def test_the_runner_installs_it_in_an_apt_container(self):
        prelude = docs_test.runner_prelude(self.scenario("ubuntu:24.04", {"apt"}))
        self.assertIn(docs_test.apt_get_retry_install(), prelude, "before the page's first apt-get")

    def test_the_runner_installs_it_in_the_systemd_image_before_its_apt_get_update(self):
        dockerfile = docs_test.systemd_dockerfile({})
        install = dockerfile.find(docs_test.apt_get_retry_install())
        self.assertNotEqual(install, -1, "installed")
        self.assertLess(install, dockerfile.find("apt-get update"), "before the update")

    def test_the_runner_leaves_a_dnf_container_alone(self):
        prelude = docs_test.runner_prelude(self.scenario("fedora:44", {"dnf"}))
        self.assertNotIn("apt-get", prelude)

    def test_the_release_smoke_job_installs_it_before_its_apt_get_update(self):
        text = (ROOT / ".github/workflows/release.yml").read_text()
        install = text.find("install -m 0755 scripts/apt-get-retry.sh /usr/local/sbin/apt-get")
        update = re.search(r"apt-get -o Acquire::Retries=3 update", text)
        self.assertNotEqual(install, -1, "installed")
        self.assertIsNotNone(update, "an update")
        self.assertLess(install, update.start(), "before the update")


if __name__ == "__main__":
    unittest.main()
