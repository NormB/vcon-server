"""Tests scripts/check_dependabot.py: every dependency file in the tree is
covered by a .github/dependabot.yml entry of its ecosystem and directory,
or is listed with a reason.

Run with: python3 -I scripts/test_check_dependabot.py
"""

import importlib.util
import pathlib
import sys
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

# fuzz/Cargo.lock on 2026-11-05, shortened: no git source.
FUZZ_LOCK = """version = 4

[[package]]
name = "rusqlite"
version = "0.41.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
"""

LOCKS = {"Cargo.lock": ROOT_LOCK, "fuzz/Cargo.lock": FUZZ_LOCK}
RUSQLITE = [{"dependency-name": "rusqlite"}]

# The cargo entry of 2026-11-05, which ignores nothing: Dependabot's update
# job failed on rewriting the fork's requirement.
ONE_ENTRY = {"updates": [{"package-ecosystem": "cargo", "directories": ["/", "/fuzz"]}]}

# The root on an entry of its own that ignores rusqlite; the fuzz package,
# which takes rusqlite from crates.io only, keeps its updates.
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

    def test_an_ignore_shared_with_the_fuzz_package_stops_its_crates_io_updates(self):
        config = {"updates": [{"package-ecosystem": "cargo", "directories": ["/", "/fuzz"], "ignore": RUSQLITE}]}
        self.assertEqual(
            git_problems(config),
            ["fuzz/Cargo.lock: the cargo entry for /fuzz ignores rusqlite, which comes from no git source there"],
        )

    def test_an_ignore_kept_after_the_fork_is_gone_is_found(self):
        without_fork = ROOT_LOCK.replace('version = "0.41.0-sqlcipher.1"\nsource = "git+', 'version = "0.41.0-sqlcipher.1"\nsource = "registry+')
        self.assertEqual(
            git_problems(SPLIT, {"Cargo.lock": without_fork, "fuzz/Cargo.lock": FUZZ_LOCK}),
            ["Cargo.lock: the cargo entry for / ignores rusqlite, which comes from no git source there"],
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


if __name__ == "__main__":
    unittest.main()
