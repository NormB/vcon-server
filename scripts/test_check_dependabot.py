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


if __name__ == "__main__":
    unittest.main()
