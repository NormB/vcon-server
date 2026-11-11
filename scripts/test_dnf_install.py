"""Tests scripts/dnf_install.py: a dnf install that is tried again from
another source, because dnf stops at the first mirror whose repomd.xml does
not parse.

Run with: python3 -I scripts/test_dnf_install.py
"""

import importlib.util
import pathlib
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("dnf_install", HERE / "dnf_install.py")
dnf_install = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dnf_install)

# The start of almalinux:9's /etc/yum.repos.d/almalinux-appstream.repo, as
# read from the image release.yml names on 2026-11-11.
APPSTREAM = """[appstream]
name=AlmaLinux $releasever - AppStream
mirrorlist=https://mirrors.almalinux.org/mirrorlist/$releasever/appstream
# baseurl=https://repo.almalinux.org/almalinux/$releasever/AppStream/$basearch/os/
enabled=1
gpgcheck=1
countme=1
gpgkey=file:///etc/pki/rpm-gpg/RPM-GPG-KEY-AlmaLinux-9
metadata_expire=86400
enabled_metadata=1

[appstream-debuginfo]
name=AlmaLinux $releasever - AppStream - Debug
mirrorlist=https://mirrors.almalinux.org/mirrorlist/$releasever/appstream-debug
# baseurl=https://vault.almalinux.org/$releasever/AppStream/debug/$basearch/
enabled=0
gpgcheck=1
"""
BASEOS = """[baseos]
name=AlmaLinux $releasever - BaseOS
mirrorlist=https://mirrors.almalinux.org/mirrorlist/$releasever/baseos
# baseurl=https://repo.almalinux.org/almalinux/$releasever/BaseOS/$basearch/os/
enabled=1
"""
APPSTREAM_ORIGIN = "https://repo.almalinux.org/almalinux/$releasever/AppStream/$basearch/os/"
BASEOS_ORIGIN = "https://repo.almalinux.org/almalinux/$releasever/BaseOS/$basearch/os/"
BOTH = {"appstream": APPSTREAM_ORIGIN, "baseos": BASEOS_ORIGIN}
ORIGIN_OPTIONS = [
    "--setopt=appstream.mirrorlist=",
    "--setopt=appstream.metalink=",
    "--setopt=appstream.baseurl=" + APPSTREAM_ORIGIN,
    "--setopt=baseos.mirrorlist=",
    "--setopt=baseos.metalink=",
    "--setopt=baseos.baseurl=" + BASEOS_ORIGIN,
]
INSTALL = ["dnf", "install", "-y", "--setopt=install_weak_deps=False"]
CLEAN = ["dnf", "clean", "metadata"]


class Runner:
    """Stands for dnf: answers each install with the next of `codes` (the
    last one from then on), and keeps every command and every wait."""

    def __init__(self, *codes):
        self.codes = list(codes)
        self.commands = []
        self.waits = []
        self.lines = []

    def run(self, argv):
        self.commands.append(list(argv))
        if argv[:2] != ["dnf", "install"]:
            return 0
        return self.codes.pop(0) if len(self.codes) > 1 else self.codes[0]

    def installs(self):
        return [c for c in self.commands if c[:2] == ["dnf", "install"]]

    def go(self, packages=("git-core",), origins=None, **more):
        return dnf_install.install(
            list(packages), BOTH if origins is None else origins, run=self.run, sleep=self.waits.append, log=self.lines.append, **more
        )


class Origins(unittest.TestCase):
    def test_an_enabled_repository_gives_the_address_its_file_names(self):
        self.assertEqual(dnf_install.origins([APPSTREAM]), {"appstream": APPSTREAM_ORIGIN})

    def test_each_file_is_read(self):
        self.assertEqual(dnf_install.origins([APPSTREAM, BASEOS]), BOTH)

    def test_a_repository_that_is_off_is_left_out(self):
        self.assertNotIn("appstream-debuginfo", dnf_install.origins([APPSTREAM]))

    def test_a_repository_with_no_enabled_line_is_on_as_dnf_has_it(self):
        self.assertEqual(dnf_install.origins(["[x]\nmirrorlist=https://m.example.com/x\n#baseurl=https://o.example.com/x/\n"]), {"x": "https://o.example.com/x/"})

    def test_a_repository_with_no_other_address_is_left_out(self):
        self.assertEqual(dnf_install.origins(["[x]\nmirrorlist=https://m.example.com/x\nenabled=1\n"]), {})

    def test_a_repository_that_already_reads_one_address_is_left_out(self):
        # Its baseurl is in use: there is no second source to change to.
        self.assertEqual(dnf_install.origins(["[x]\nbaseurl=https://o.example.com/x/\nenabled=1\n"]), {})
        # Nor when an older address is still there as a comment.
        self.assertEqual(dnf_install.origins(["[x]\n# baseurl=https://old.example.com/x/\nbaseurl=https://o.example.com/x/\nenabled=1\n"]), {})

    def test_a_metalink_repository_with_a_commented_address_counts(self):
        text = "[x]\nmetalink=https://m.example.com/metalink?repo=x\n#baseurl=https://o.example.com/x/\nenabled=1\n"
        self.assertEqual(dnf_install.origins([text]), {"x": "https://o.example.com/x/"})

    def test_the_first_commented_address_of_a_section_is_the_one(self):
        text = "[x]\nmirrorlist=https://m.example.com/x\n# baseurl=https://a.example.com/x/\n# baseurl=https://b.example.com/x/\n"
        self.assertEqual(dnf_install.origins([text]), {"x": "https://a.example.com/x/"})

    def test_an_address_belongs_to_its_own_section_only(self):
        text = "[a]\nmirrorlist=https://m.example.com/a\nenabled=1\n[b]\nmirrorlist=https://m.example.com/b\n# baseurl=https://o.example.com/b/\n"
        self.assertEqual(dnf_install.origins([text]), {"b": "https://o.example.com/b/"})

    def test_spaces_around_the_keys_and_values_are_dropped(self):
        text = "[ x ]\nmirrorlist = https://m.example.com/x\n#  baseurl =  https://o.example.com/x/  \nenabled = 1\n"
        self.assertEqual(dnf_install.origins([text]), {"x": "https://o.example.com/x/"})

    def test_enabled_takes_the_words_dnf_takes(self):
        for word, on in (("1", True), ("yes", True), ("true", True), ("True", True), ("on", True), ("0", False), ("no", False), ("false", False), ("off", False)):
            text = f"[x]\nmirrorlist=https://m.example.com/x\n# baseurl=https://o.example.com/x/\nenabled={word}\n"
            self.assertEqual(dnf_install.origins([text]) != {}, on, word)

    def test_an_address_that_is_not_http_is_not_taken(self):
        for bad in ("file:///srv/x/", "file://host.example.com/srv/x/", "ftp://o.example.com/x/"):
            text = f"[x]\nmirrorlist=https://m.example.com/x\n# baseurl={bad}\n"
            self.assertEqual(dnf_install.origins([text]), {}, bad)

    def test_an_address_with_a_space_or_a_quote_is_not_taken(self):
        for bad in ("https://o.example.com/x y/", "https://o.example.com/'x'/", 'https://o.example.com/"x"/'):
            self.assertEqual(dnf_install.origins([f"[x]\nmirrorlist=https://m.example.com/x\n# baseurl={bad}\n"]), {}, bad)

    def test_a_section_name_dnf_would_not_take_is_left_out(self):
        text = "[x y]\nmirrorlist=https://m.example.com/x\n# baseurl=https://o.example.com/x/\n"
        self.assertEqual(dnf_install.origins([text]), {})

    def test_no_file_and_an_empty_file_give_nothing(self):
        self.assertEqual(dnf_install.origins([]), {})
        self.assertEqual(dnf_install.origins(["", "\n\n# a comment\n"]), {})

    def test_lines_before_the_first_section_are_passed_over(self):
        self.assertEqual(dnf_install.origins(["# baseurl=https://o.example.com/x/\nmirrorlist=https://m.example.com/x\n" + BASEOS]), {"baseos": BASEOS_ORIGIN})

    def test_the_files_of_a_directory_are_read_by_name(self):
        with tempfile.TemporaryDirectory() as d:
            (pathlib.Path(d) / "b.repo").write_text(BASEOS, encoding="utf-8")
            (pathlib.Path(d) / "a.repo").write_text(APPSTREAM, encoding="utf-8")
            (pathlib.Path(d) / "c.repo.rpmsave").write_text("[old]\nmirrorlist=https://m.example.com/x\n# baseurl=https://o.example.com/x/\n", encoding="utf-8")
            self.assertEqual(dnf_install.read_origins(d), BOTH)

    def test_a_directory_that_is_not_there_gives_nothing(self):
        self.assertEqual(dnf_install.read_origins("/nonexistent/yum.repos.d"), {})


class Options(unittest.TestCase):
    def test_the_first_try_is_as_the_system_is_configured(self):
        self.assertEqual(dnf_install.options(1, BOTH), [])

    def test_the_second_try_reads_each_repository_from_its_other_address(self):
        self.assertEqual(dnf_install.options(2, BOTH), ORIGIN_OPTIONS)

    def test_the_sources_take_turns(self):
        self.assertEqual([bool(dnf_install.options(n, BOTH)) for n in range(1, 7)], [False, True, False, True, False, True])

    def test_the_mirror_list_and_the_metalink_are_cleared_before_the_address_is_set(self):
        # With both set, dnf 4.14 on AlmaLinux 9.8 still read the mirror list
        # first (tried in the image on 2026-11-11).
        got = dnf_install.options(2, {"x": "https://o.example.com/x/"})
        self.assertEqual(got, ["--setopt=x.mirrorlist=", "--setopt=x.metalink=", "--setopt=x.baseurl=https://o.example.com/x/"])

    def test_with_no_other_address_every_try_is_as_configured(self):
        self.assertEqual([dnf_install.options(n, {}) for n in (1, 2, 3)], [[], [], []])

    def test_the_repositories_come_in_one_order(self):
        self.assertEqual(dnf_install.options(2, dict(reversed(list(BOTH.items())))), ORIGIN_OPTIONS)


class Install(unittest.TestCase):
    def test_a_first_try_that_works_runs_dnf_once(self):
        r = Runner(0)
        self.assertEqual(r.go(), 0)
        self.assertEqual(r.commands, [INSTALL + ["git-core"]])
        self.assertEqual(r.waits, [])

    def test_the_packages_come_last_in_the_order_given(self):
        r = Runner(0)
        r.go(packages=("gcc", "make", "perl-core"))
        self.assertEqual(r.commands, [INSTALL + ["gcc", "make", "perl-core"]])

    def test_the_second_try_reads_the_other_source(self):
        r = Runner(1, 0)
        self.assertEqual(r.go(), 0)
        self.assertEqual(r.installs(), [INSTALL + ["git-core"], INSTALL + ORIGIN_OPTIONS + ["git-core"]])

    def test_the_metadata_is_cleaned_between_tries_and_not_before_the_first(self):
        r = Runner(1, 1, 0)
        r.go()
        self.assertEqual([c[:2] for c in r.commands], [["dnf", "install"], ["dnf", "clean"], ["dnf", "install"], ["dnf", "clean"], ["dnf", "install"]])
        self.assertEqual(r.commands[1], CLEAN)

    def test_the_third_try_is_the_mirror_list_again(self):
        r = Runner(1, 1, 0)
        self.assertEqual(r.go(), 0)
        self.assertEqual(r.installs()[2], INSTALL + ["git-core"])

    def test_five_tries_then_it_gives_up_with_dnf_s_last_code(self):
        r = Runner(1, 1, 1, 1, 7)
        self.assertEqual(r.go(), 7)
        self.assertEqual(len(r.installs()), 5)

    def test_it_never_runs_a_sixth_try(self):
        r = Runner(1)
        self.assertEqual(r.go(), 1)
        self.assertEqual(len(r.installs()), 5)
        self.assertEqual(r.commands[-1][:2], ["dnf", "install"], "nothing runs after the last try")

    def test_the_waits_double_and_are_bounded(self):
        r = Runner(1)
        r.go()
        self.assertEqual(r.waits, [5, 10, 20, 40])

    def test_no_wait_follows_the_last_try(self):
        r = Runner(1)
        r.go(attempts=2)
        self.assertEqual(r.waits, [5])
        self.assertEqual(len(r.installs()), 2)

    def test_one_try_means_one(self):
        r = Runner(1)
        self.assertEqual(r.go(attempts=1), 1)
        self.assertEqual(r.commands, [INSTALL + ["git-core"]])

    def test_a_try_that_ran_out_of_time_is_tried_again(self):
        r = Runner(dnf_install.TIMED_OUT, 0)
        self.assertEqual(r.go(), 0)
        self.assertEqual(len(r.installs()), 2)

    def test_a_failed_clean_does_not_stop_the_next_try(self):
        r = Runner(1, 0)
        run = r.run
        r.run = lambda argv: 1 if argv == CLEAN else run(argv)
        self.assertEqual(r.go(), 0)

    def test_with_no_other_source_it_still_tries_five_times(self):
        r = Runner(1)
        self.assertEqual(r.go(origins={}), 1)
        self.assertEqual(r.installs(), [INSTALL + ["git-core"]] * 5)

    def test_each_failure_is_said_with_its_source_and_its_number(self):
        r = Runner(1, 0)
        r.go()
        self.assertEqual(len(r.lines), 1)
        self.assertIn("try 1 of 5", r.lines[0])
        self.assertIn("the configured mirrors", r.lines[0])
        self.assertIn("exit 1", r.lines[0])
        self.assertIn("the repository files' own addresses", r.lines[0], "it names where the next try reads")

    def test_giving_up_says_both_sources_were_tried(self):
        r = Runner(1)
        r.go()
        self.assertIn("giving up", r.lines[-1])
        self.assertIn("5 tries", r.lines[-1])
        self.assertIn("git-core", r.lines[-1])

    def test_success_says_nothing(self):
        r = Runner(0)
        r.go()
        self.assertEqual(r.lines, [])


class Main(unittest.TestCase):
    def main(self, *argv, codes=(0,), repos=None):
        r = Runner(*codes)
        with tempfile.TemporaryDirectory() as d:
            if repos is not None:
                (pathlib.Path(d) / "a.repo").write_text(repos, encoding="utf-8")
            rc = dnf_install.main(list(argv), repos_dir=d, run=r.run, sleep=r.waits.append, log=r.lines.append)
        return rc, r

    def test_no_package_is_a_usage_error(self):
        rc, r = self.main()
        self.assertEqual(rc, 2)
        self.assertEqual(r.commands, [])
        self.assertIn("usage", r.lines[0])

    def test_an_option_in_place_of_a_package_is_refused(self):
        for bad in ("-y", "--setopt=gpgcheck=0", "--nogpgcheck", "--repofrompath=x,https://example.com/"):
            rc, r = self.main("git-core", bad)
            self.assertEqual(rc, 2, bad)
            self.assertEqual(r.commands, [], bad)

    def test_a_package_given_as_a_file_or_an_address_is_refused(self):
        for bad in ("https://example.com/x.rpm", "./x.rpm", "/tmp/x.rpm", "/x", ".x", "a b", "", "x;y", "$(x)"):
            rc, r = self.main(bad)
            self.assertEqual(rc, 2, bad)
            self.assertEqual(r.commands, [], bad)

    def test_package_names_with_their_usual_characters_are_taken(self):
        rc, r = self.main("git-core", "perl-core", "python3", "libstdc++", "gcc-c++", "glibc.i686", "util-linux_x")
        self.assertEqual(rc, 0)
        self.assertEqual(r.commands, [INSTALL + ["git-core", "perl-core", "python3", "libstdc++", "gcc-c++", "glibc.i686", "util-linux_x"]])

    def test_the_repository_files_of_the_system_choose_the_second_source(self):
        rc, r = self.main("jq", codes=(1, 0), repos=BASEOS)
        self.assertEqual(rc, 0)
        self.assertEqual(r.installs()[1], INSTALL + ["--setopt=baseos.mirrorlist=", "--setopt=baseos.metalink=", "--setopt=baseos.baseurl=" + BASEOS_ORIGIN, "jq"])

    def test_dnf_s_code_is_the_exit_code(self):
        rc, _ = self.main("jq", codes=(3,))
        self.assertEqual(rc, 3)

    def test_gpg_checking_is_never_turned_off(self):
        _, r = self.main("jq", codes=(1,), repos=APPSTREAM + BASEOS)
        for command in r.commands:
            for word in command:
                self.assertNotIn("gpgcheck", word)
                self.assertNotIn("nogpg", word)
                self.assertNotIn("sslverify", word)


class RunDnf(unittest.TestCase):
    def test_a_command_s_exit_code_is_returned(self):
        self.assertEqual(dnf_install.run_bounded([sys.executable, "-I", "-c", "raise SystemExit(4)"], limit=30), 4)
        self.assertEqual(dnf_install.run_bounded([sys.executable, "-I", "-c", "pass"], limit=30), 0)

    def test_a_command_that_outlasts_its_limit_is_stopped(self):
        self.assertEqual(dnf_install.run_bounded([sys.executable, "-I", "-c", "import time; time.sleep(30)"], limit=0.5), dnf_install.TIMED_OUT)

    def test_a_command_that_is_not_installed_is_127(self):
        self.assertEqual(dnf_install.run_bounded(["/nonexistent/dnf", "install"], limit=5), 127)


if __name__ == "__main__":
    unittest.main()
