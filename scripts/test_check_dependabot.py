"""Tests scripts/check_dependabot.py: every dependency file in the tree is
covered by a .github/dependabot.yml entry of its ecosystem and directory,
or is listed with a reason.

Run with: python3 -I scripts/test_check_dependabot.py
"""

import importlib.util
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_dependabot", HERE / "check_dependabot.py")
check_dependabot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_dependabot)

# The tree's dependency files on 2026-10-27, and the workflows.
TREE = [
    ".clusterfuzzlite/Dockerfile",
    ".github/actions/fuzz-setup/action.yml",
    ".github/workflows/ci.yml",
    "Cargo.lock",
    "Cargo.toml",
    "README.md",
    "clients/go/go.mod",
    "clients/typescript/package-lock.json",
    "console-tests/package-lock.json",
    "fuzz/Cargo.lock",
    "scripts/icons-requirements.txt",
]

# .github/dependabot.yml on 2026-10-27, as parsed.
TODAY = {
    "version": 2,
    "updates": [
        {"package-ecosystem": "cargo", "directories": ["/", "/fuzz"]},
        {"package-ecosystem": "github-actions", "directories": ["/", "/.github/actions/fuzz-setup"]},
        {"package-ecosystem": "docker", "directory": "/.clusterfuzzlite"},
    ],
}

FIXED = {
    "version": 2,
    "updates": TODAY["updates"]
    + [
        {"package-ecosystem": "npm", "directories": ["/clients/typescript", "/console-tests"]},
        {"package-ecosystem": "pip", "directory": "/scripts"},
    ],
}


def no_go_requires(path):
    return "module example.com/x\n\ngo 1.26\n"


def uncovered(tree, config, read=no_go_requires, exceptions=None):
    return check_dependabot.uncovered(tree, config, read, exceptions or {})


class Files(unittest.TestCase):
    def test_each_kind_of_file_is_named_with_its_ecosystem_and_directory(self):
        found = check_dependabot.dependency_files(TREE, no_go_requires)
        self.assertIn(("Cargo.lock", "cargo", "/"), found)
        self.assertIn(("fuzz/Cargo.lock", "cargo", "/fuzz"), found)
        self.assertIn(("clients/typescript/package-lock.json", "npm", "/clients/typescript"), found)
        self.assertIn(("scripts/icons-requirements.txt", "pip", "/scripts"), found)
        self.assertIn((".clusterfuzzlite/Dockerfile", "docker", "/.clusterfuzzlite"), found)
        self.assertIn((".github/actions/fuzz-setup/action.yml", "github-actions", "/.github/actions/fuzz-setup"), found)
        self.assertIn((".github/workflows", "github-actions", "/"), found)
        self.assertNotIn("README.md", [f for f, _, _ in found])
        self.assertNotIn("Cargo.toml", [f for f, _, _ in found])

    def test_requirements_files_are_found_by_any_name_holding_requirements(self):
        found = check_dependabot.dependency_files(["requirements.txt", "docs/requirements-dev.txt", "a/b-requirements.txt"], no_go_requires)
        self.assertEqual([d for _, _, d in found], ["/", "/docs", "/a"])

    def test_a_go_module_without_requirements_needs_no_entry(self):
        self.assertEqual(check_dependabot.dependency_files(["clients/go/go.mod"], no_go_requires), [])

    def test_a_go_module_with_requirements_needs_a_gomod_entry(self):
        def read(path):
            return "module x\n\ngo 1.26\n\nrequire golang.org/x/net v0.1.0\n"

        self.assertEqual(check_dependabot.dependency_files(["clients/go/go.mod"], read), [("clients/go/go.mod", "gomod", "/clients/go")])


class Coverage(unittest.TestCase):
    def test_todays_tree_lacks_exactly_the_npm_lockfiles_and_the_pip_requirements(self):
        self.assertEqual(
            uncovered(TREE, TODAY),
            [
                "clients/typescript/package-lock.json: no npm entry for /clients/typescript",
                "console-tests/package-lock.json: no npm entry for /console-tests",
                "scripts/icons-requirements.txt: no pip entry for /scripts",
            ],
        )

    def test_the_fixed_configuration_covers_the_tree(self):
        self.assertEqual(uncovered(TREE, FIXED), [])

    def test_an_entry_of_another_ecosystem_does_not_cover_a_file(self):
        config = {"updates": [{"package-ecosystem": "npm", "directory": "/"}]}
        self.assertEqual(uncovered(["Cargo.lock"], config), ["Cargo.lock: no cargo entry for /"])

    def test_an_entry_for_another_directory_does_not_cover_a_file(self):
        config = {"updates": [{"package-ecosystem": "cargo", "directory": "/fuzz"}]}
        self.assertEqual(uncovered(["Cargo.lock"], config), ["Cargo.lock: no cargo entry for /"])

    def test_a_directory_written_with_a_trailing_slash_is_the_same_directory(self):
        config = {"updates": [{"package-ecosystem": "cargo", "directory": "/fuzz/"}]}
        self.assertEqual(uncovered(["fuzz/Cargo.lock"], config), [])

    def test_a_directory_glob_covers_what_it_matches(self):
        config = {"updates": [{"package-ecosystem": "github-actions", "directories": ["/", "/.github/actions/*"]}]}
        self.assertEqual(uncovered([".github/workflows/ci.yml", ".github/actions/a/action.yml"], config), [])

    def test_a_removed_entry_is_found(self):
        config = {"updates": [u for u in FIXED["updates"] if u["package-ecosystem"] != "docker"]}
        self.assertEqual(uncovered(TREE, config), [".clusterfuzzlite/Dockerfile: no docker entry for /.clusterfuzzlite"])

    def test_a_new_lockfile_is_found(self):
        self.assertEqual(
            uncovered(TREE + ["clients/python/requirements.txt"], FIXED),
            ["clients/python/requirements.txt: no pip entry for /clients/python"],
        )

    def test_a_listed_file_needs_no_entry_but_must_give_a_reason(self):
        self.assertEqual(uncovered(["Cargo.lock"], {"updates": []}, exceptions={"Cargo.lock": "a reason"}), [])
        with self.assertRaises(ValueError):
            uncovered(["Cargo.lock"], {"updates": []}, exceptions={"Cargo.lock": " "})

    def test_a_listed_file_that_is_gone_is_reported(self):
        self.assertEqual(
            uncovered([], {"updates": []}, exceptions={"old/Cargo.lock": "a reason"}),
            ["old/Cargo.lock: listed as an exception but not in the tree"],
        )

    def test_a_configuration_without_updates_covers_nothing(self):
        self.assertEqual(uncovered(["Cargo.lock"], {}), ["Cargo.lock: no cargo entry for /"])


# Cargo.lock on 2026-11-05, shortened: crates.io rusqlite for the server and
# the git fork the tests use, which keeps the name rusqlite.
ROOT_LOCK = """version = 4

[[package]]
name = "libsqlite3-sys"
version = "0.39.1"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "rusqlite"
version = "0.41.0-sqlcipher.1"
source = "git+https://github.com/NormB/rusqlite?rev=a2936ca6190f54e7ab54375385877cc96eebebe5#a2936ca6190f54e7ab54375385877cc96eebebe5"

[[package]]
name = "rusqlite"
version = "0.41.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "vcon-server"
version = "0.3.0"
"""

# fuzz/Cargo.lock on 2026-11-05, shortened: no git source, and the server
# crate by path, from the same crates/vcon-server/Cargo.toml.
FUZZ_LOCK = """version = 4

[[package]]
name = "rusqlite"
version = "0.41.1"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "vcon-fuzz"
version = "0.0.0"

[[package]]
name = "vcon-server"
version = "0.3.0"
"""

LOCKS = {"Cargo.lock": ROOT_LOCK, "fuzz/Cargo.lock": FUZZ_LOCK}
RUSQLITE = [{"dependency-name": "rusqlite"}]

# One entry for both lockfiles that ignores the fork, in both.
JOINED = {"updates": [{"package-ecosystem": "cargo", "directories": ["/", "/fuzz"], "ignore": RUSQLITE}]}

# The cargo entry of 2026-11-05, which ignores nothing: Dependabot's update
# job failed on rewriting the fork's requirement.
ONE_ENTRY = {"updates": [{"package-ecosystem": "cargo", "directories": ["/", "/fuzz"]}]}

# The root on an entry of its own that ignores rusqlite; the fuzz package,
# which takes rusqlite from crates.io only, keeps its updates. On 2026-11-07
# this split made #142 and #143: each bumped the server's =60.0.0 parquet pin
# in crates/vcon-server/Cargo.toml and only its own lockfile, so the other
# lockfile no longer matched and every --locked build failed.
SPLIT = {
    "updates": [
        {"package-ecosystem": "cargo", "directory": "/", "ignore": RUSQLITE},
        {"package-ecosystem": "cargo", "directory": "/fuzz"},
    ]
}


def git_problems(config, locks=None):
    locks = LOCKS if locks is None else locks
    return check_dependabot.git_problems(sorted(locks), config, locks.__getitem__)


class GitSources(unittest.TestCase):
    def test_the_packages_a_lockfile_takes_from_git_are_named(self):
        self.assertEqual(check_dependabot.git_packages(ROOT_LOCK), ["rusqlite"])
        self.assertEqual(check_dependabot.git_packages(FUZZ_LOCK), [])

    def test_the_entry_of_2026_11_05_does_not_ignore_the_fork(self):
        self.assertEqual(
            git_problems(ONE_ENTRY),
            ["Cargo.lock: rusqlite comes from git; the cargo entry for / must ignore it"],
        )

    def test_the_root_ignoring_rusqlite_on_an_entry_of_its_own_passes(self):
        self.assertEqual(git_problems(SPLIT), [])

    def test_one_entry_for_both_lockfiles_may_ignore_the_fork_in_both(self):
        # The fuzz package's crates.io rusqlite is then updated by hand with
        # the fork, which keeps both lockfiles on one entry (SharedManifests).
        self.assertEqual(git_problems(JOINED), [])

    def test_an_ignore_kept_after_the_fork_is_gone_is_found(self):
        without_fork = ROOT_LOCK.replace('version = "0.41.0-sqlcipher.1"\nsource = "git+', 'version = "0.41.0-sqlcipher.1"\nsource = "registry+')
        locks = {"Cargo.lock": without_fork, "fuzz/Cargo.lock": FUZZ_LOCK}
        self.assertEqual(
            git_problems(SPLIT, locks),
            ["Cargo.lock: the cargo entry for / ignores rusqlite, which no lockfile it covers takes from git"],
        )
        self.assertEqual(
            git_problems(JOINED, locks),
            ["Cargo.lock, fuzz/Cargo.lock: the cargo entry for /, /fuzz ignores rusqlite, which no lockfile it covers takes from git"],
        )

    def test_ignoring_only_some_updates_does_not_cover_a_git_package(self):
        config = {
            "updates": [
                {"package-ecosystem": "cargo", "directory": "/", "ignore": [{"dependency-name": "rusqlite", "update-types": ["version-update:semver-major"]}]},
                {"package-ecosystem": "cargo", "directory": "/fuzz"},
            ]
        }
        self.assertEqual(
            git_problems(config),
            ["Cargo.lock: rusqlite comes from git; the cargo entry for / must ignore it"],
        )

    def test_ignoring_only_some_updates_of_a_crates_io_package_is_allowed(self):
        config = {
            "updates": [
                {"package-ecosystem": "cargo", "directory": "/", "ignore": RUSQLITE},
                {"package-ecosystem": "cargo", "directory": "/fuzz", "ignore": [{"dependency-name": "rusqlite", "versions": [">= 0.42"]}]},
            ]
        }
        self.assertEqual(git_problems(config), [])

    def test_an_ignore_of_another_ecosystem_does_not_count(self):
        config = {
            "updates": [
                {"package-ecosystem": "cargo", "directory": "/"},
                {"package-ecosystem": "cargo", "directory": "/fuzz"},
                {"package-ecosystem": "npm", "directory": "/", "ignore": RUSQLITE},
            ]
        }
        self.assertEqual(
            git_problems(config),
            ["Cargo.lock: rusqlite comes from git; the cargo entry for / must ignore it"],
        )

    def test_a_lockfile_no_entry_covers_is_left_to_the_coverage_rule(self):
        self.assertEqual(git_problems({"updates": []}), [])



def shared_problems(config, locks=None):
    locks = LOCKS if locks is None else locks
    return check_dependabot.shared_problems(sorted(locks), config, locks.__getitem__)


class SharedManifests(unittest.TestCase):
    """Two lockfiles that take a package by path share its Cargo.toml, so one
    cargo entry must cover both: Dependabot then bumps a requirement there in
    one pull request with both lockfiles, not in two that each leave the other
    lockfile behind (#142 and #143, 2026-11-07)."""

    def test_the_packages_a_lockfile_takes_by_path_are_named(self):
        self.assertEqual(check_dependabot.path_packages(ROOT_LOCK), ["vcon-server"])
        self.assertEqual(check_dependabot.path_packages(FUZZ_LOCK), ["vcon-fuzz", "vcon-server"])

    def test_the_split_entries_of_2026_11_06_are_found(self):
        self.assertEqual(
            shared_problems(SPLIT),
            ["Cargo.lock and fuzz/Cargo.lock both take vcon-server by path; one cargo entry must cover / and /fuzz"],
        )

    def test_one_entry_for_both_lockfiles_passes(self):
        self.assertEqual(shared_problems(JOINED), [])

    def test_a_glob_that_covers_both_directories_passes(self):
        config = {"updates": [{"package-ecosystem": "cargo", "directories": ["/", "/f*"]}]}
        self.assertEqual(shared_problems(config), [])

    def test_lockfiles_with_no_path_package_in_common_may_have_entries_of_their_own(self):
        other = FUZZ_LOCK.replace('name = "vcon-server"', 'name = "vcon-other"')
        self.assertEqual(shared_problems(SPLIT, {"Cargo.lock": ROOT_LOCK, "fuzz/Cargo.lock": other}), [])

    def test_a_package_both_take_from_crates_io_is_not_shared(self):
        self.assertNotIn("rusqlite", check_dependabot.path_packages(ROOT_LOCK))
        self.assertNotIn("rusqlite", check_dependabot.path_packages(FUZZ_LOCK))

    def test_an_entry_of_another_ecosystem_does_not_count(self):
        config = {
            "updates": SPLIT["updates"] + [{"package-ecosystem": "npm", "directories": ["/", "/fuzz"]}],
        }
        self.assertEqual(len(shared_problems(config)), 1)

    def test_a_lockfile_no_entry_covers_is_left_to_the_coverage_rule(self):
        self.assertEqual(shared_problems({"updates": [{"package-ecosystem": "cargo", "directory": "/"}]}), [])


def cooldown(**days):
    return {"updates": [{"package-ecosystem": "cargo", "directory": "/", "cooldown": days}]}


class Cooldown(unittest.TestCase):
    """A new version is proposed only once it is at least 14 days old
    (CONTRIBUTING.md, "Dependency versions"); Dependabot's own default
    cooldown is 3 days. #142 and #143 proposed parquet 60.1.0 on day 10."""

    def test_the_rule_is_14_days(self):
        self.assertEqual(check_dependabot.MIN_AGE_DAYS, 14)

    def test_14_days_for_every_version_passes(self):
        self.assertEqual(check_dependabot.cooldown_problems(cooldown(**{"default-days": 14})), [])

    def test_a_longer_wait_for_some_updates_passes(self):
        config = cooldown(**{"default-days": 14, "semver-major-days": 30})
        self.assertEqual(check_dependabot.cooldown_problems(config), [])

    def test_an_entry_without_a_cooldown_is_found(self):
        config = {"updates": [{"package-ecosystem": "cargo", "directories": ["/", "/fuzz"]}]}
        self.assertEqual(
            check_dependabot.cooldown_problems(config),
            ["cargo entry for /, /fuzz: no cooldown; a new version must wait 14 days"],
        )

    def test_a_short_default_is_found(self):
        self.assertEqual(
            check_dependabot.cooldown_problems(cooldown(**{"default-days": 13})),
            ["cargo entry for /: cooldown default-days is 13, under 14"],
        )

    def test_a_missing_default_is_found_even_with_every_semver_wait(self):
        # Without default-days Dependabot waits 3 days for versions that are
        # not SemVer, and Docker and GitHub Actions take no semver-*-days.
        config = cooldown(**{"semver-major-days": 14, "semver-minor-days": 14, "semver-patch-days": 14})
        self.assertEqual(
            check_dependabot.cooldown_problems(config),
            ["cargo entry for /: cooldown default-days is missing, under 14"],
        )

    def test_a_short_semver_wait_is_found(self):
        for key in ("semver-major-days", "semver-minor-days", "semver-patch-days"):
            with self.subTest(key=key):
                self.assertEqual(
                    check_dependabot.cooldown_problems(cooldown(**{"default-days": 14, key: 3})),
                    [f"cargo entry for /: cooldown {key} is 3, under 14"],
                )

    def test_a_wait_that_is_not_a_whole_number_is_found(self):
        for value in ("14", 14.5, True, None):
            with self.subTest(value=value):
                self.assertEqual(
                    check_dependabot.cooldown_problems(cooldown(**{"default-days": value})),
                    [f"cargo entry for /: cooldown default-days is {value!r}, under 14"],
                )

    def test_limiting_the_cooldown_to_some_dependencies_is_found(self):
        for key in ("include", "exclude"):
            with self.subTest(key=key):
                self.assertEqual(
                    check_dependabot.cooldown_problems(cooldown(**{"default-days": 14, key: ["parquet"]})),
                    [f"cargo entry for /: cooldown {key} exempts some dependencies from the 14 days"],
                )

    def test_every_entry_is_checked(self):
        config = {
            "updates": [
                {"package-ecosystem": "cargo", "directory": "/", "cooldown": {"default-days": 14}},
                {"package-ecosystem": "docker", "directory": "/.clusterfuzzlite"},
            ]
        }
        self.assertEqual(
            check_dependabot.cooldown_problems(config),
            ["docker entry for /.clusterfuzzlite: no cooldown; a new version must wait 14 days"],
        )



class Problems(unittest.TestCase):
    """The command reports every rule's problems, through one function."""

    def test_each_rule_reaches_the_command(self):
        config = {
            "updates": [
                {"package-ecosystem": "cargo", "directory": "/", "ignore": RUSQLITE, "cooldown": {"default-days": 14}},
                {"package-ecosystem": "cargo", "directory": "/fuzz", "ignore": RUSQLITE},
            ]
        }
        paths = sorted(LOCKS) + ["clients/typescript/package-lock.json"]
        found = check_dependabot.problems(paths, config, LOCKS.get, {})
        self.assertEqual(
            found,
            [
                "clients/typescript/package-lock.json: no npm entry for /clients/typescript",
                "fuzz/Cargo.lock: the cargo entry for /fuzz ignores rusqlite, which no lockfile it covers takes from git",
                "Cargo.lock and fuzz/Cargo.lock both take vcon-server by path; one cargo entry must cover / and /fuzz",
                "cargo entry for /fuzz: no cooldown; a new version must wait 14 days",
            ],
        )


    def test_the_command_fails_on_an_entry_without_a_cooldown(self):
        # In a repository of its own, with git's variables cleared, so a
        # hook's GIT_DIR or GIT_INDEX_FILE cannot point git elsewhere.
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            (root / ".github").mkdir()
            (root / ".github/dependabot.yml").write_text(
                'version: 2\nupdates:\n  - package-ecosystem: "cargo"\n    directory: "/"\n'
                '    schedule:\n      interval: "weekly"\n',
                encoding="utf-8",
            )
            (root / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "x"\nversion = "0.1.0"\n', encoding="utf-8")
            subprocess.run(["git", "init", "-q"], cwd=root, env=env, check=True)
            subprocess.run(["git", "add", "."], cwd=root, env=env, check=True)
            run = subprocess.run([sys.executable, "-I", str(HERE / "check_dependabot.py")], cwd=root, env=env, capture_output=True, text=True)
        self.assertEqual(run.returncode, 1, run.stderr)
        self.assertEqual(run.stderr, "check_dependabot: cargo entry for /: no cooldown; a new version must wait 14 days\n")


NO_COOLDOWN = 'version: 2\nupdates:\n  - package-ecosystem: "cargo"\n    directory: "/"\n    schedule:\n      interval: "weekly"\n'
NO_ENTRY = "version: 2\nupdates: []\n"
LOCK = 'version = 4\n\n[[package]]\nname = "x"\nversion = "0.1.0"\n'


class Walk(unittest.TestCase):
    """--walk: the command in the commit hook's copy of the staged files,
    which is no repository."""

    ENV = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")} | {"GIT_CEILING_DIRECTORIES": tempfile.gettempdir()}

    def run_in(self, files, *arguments, env=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name, text in files.items():
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_text(text, encoding="utf-8")
            run = subprocess.run([sys.executable, "-I", str(HERE / "check_dependabot.py"), *arguments], cwd=root, env=env or self.ENV, capture_output=True, text=True)
        return run.returncode, run.stdout, run.stderr

    def test_success_with_walk_the_command_needs_no_repository(self):
        self.assertEqual(self.run_in({".github/dependabot.yml": NO_ENTRY}, "--walk"), (0, "check_dependabot: ok (0 dependency files covered)\n", ""))

    def test_failure_with_walk_a_lockfile_no_entry_covers(self):
        files = {".github/dependabot.yml": NO_ENTRY, "tools/package-lock.json": "{}\n"}
        self.assertEqual(self.run_in(files, "--walk"), (1, "", "check_dependabot: tools/package-lock.json: no npm entry for /tools\n"))

    def test_failure_with_walk_an_entry_without_a_cooldown(self):
        files = {".github/dependabot.yml": NO_COOLDOWN, "Cargo.lock": LOCK}
        self.assertEqual(self.run_in(files, "--walk"), (1, "", "check_dependabot: cargo entry for /: no cooldown; a new version must wait 14 days\n"))

    def test_failure_outside_a_repository_without_walk_the_command_says_so(self):
        self.assertEqual(
            self.run_in({".github/dependabot.yml": NO_ENTRY}),
            (2, "", "check_dependabot: not in a git repository; --walk reads the files under this directory\n"),
        )

    def test_failure_another_argument_is_refused(self):
        self.assertEqual(self.run_in({".github/dependabot.yml": NO_ENTRY}, "--walk", "."), (2, "", "usage: check_dependabot.py [--walk]\n"))

    def test_edge_with_walk_installed_packages_are_passed_over(self):
        files = {".github/dependabot.yml": NO_ENTRY, "website/node_modules/x/package-lock.json": "{}\n"}
        self.assertEqual(self.run_in(files, "--walk")[0], 0)

    def test_edge_with_walk_under_a_hook_s_git_variables(self):
        env = dict(self.ENV, GIT_DIR=str(HERE.parent / "no-such-directory"), GIT_INDEX_FILE="no-such-index")
        self.assertEqual(self.run_in({".github/dependabot.yml": NO_ENTRY}, "--walk", env=env)[0], 0)


if __name__ == "__main__":
    unittest.main()
