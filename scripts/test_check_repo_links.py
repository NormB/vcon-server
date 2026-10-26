"""Tests scripts/check_repo_links.py: every file or directory that
.bestpractices.json links to on main must exist in the tree.

Run with: python3 -I scripts/test_check_repo_links.py
"""

import importlib.util
import pathlib
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_repo_links", HERE / "check_repo_links.py")
check_repo_links = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_repo_links)

BASE = "https://github.com/NormB/vcon-server"


class LinkedPaths(unittest.TestCase):
    def paths(self, text):
        return check_repo_links.linked_paths(text)

    def test_a_file_link_names_its_path(self):
        self.assertEqual(self.paths(f"see {BASE}/blob/main/crates/vcon/tests/properties.rs"), ["crates/vcon/tests/properties.rs"])

    def test_a_directory_link_names_its_path(self):
        self.assertEqual(self.paths(f"in {BASE}/tree/main/LICENSES"), ["LICENSES"])

    def test_trailing_punctuation_is_not_part_of_the_path(self):
        text = f"({BASE}/blob/main/.githooks/pre-commit). {BASE}/blob/main/CONTRIBUTING.md: and {BASE}/blob/main/ci.yml, x"
        self.assertEqual(self.paths(text), [".githooks/pre-commit", "CONTRIBUTING.md", "ci.yml"])

    def test_an_anchor_is_not_part_of_the_path(self):
        self.assertEqual(self.paths(f"{BASE}/blob/main/README.md#building"), ["README.md"])

    def test_links_that_are_not_paths_on_main_are_ignored(self):
        text = f"{BASE} {BASE}/issues {BASE}/actions/runs/1 {BASE}/commit/abc {BASE}/commits/main https://example.com/blob/main/x"
        self.assertEqual(self.paths(text), [])


class Missing(unittest.TestCase):
    def test_a_path_that_exists_is_not_missing(self):
        with tempfile.TemporaryDirectory() as root:
            (pathlib.Path(root) / "crates/vcon/tests").mkdir(parents=True)
            (pathlib.Path(root) / "crates/vcon/tests/properties.rs").write_text("")
            text = f"{BASE}/blob/main/crates/vcon/tests/properties.rs {BASE}/tree/main/crates/vcon"
            self.assertEqual(check_repo_links.missing(text, pathlib.Path(root)), [])

    def test_a_path_that_moved_is_missing(self):
        with tempfile.TemporaryDirectory() as root:
            (pathlib.Path(root) / "crates/vcon/tests").mkdir(parents=True)
            text = f"{BASE}/blob/main/crates/old-name/tests/properties.rs"
            self.assertEqual(
                check_repo_links.missing(text, pathlib.Path(root)),
                ["crates/old-name/tests/properties.rs"],
            )

    def test_a_path_out_of_the_tree_is_missing_even_if_it_exists(self):
        with tempfile.TemporaryDirectory() as outer:
            root = pathlib.Path(outer) / "repo"
            root.mkdir()
            (pathlib.Path(outer) / "outside.txt").write_text("")
            text = f"{BASE}/blob/main/../outside.txt"
            self.assertEqual(check_repo_links.missing(text, root), ["../outside.txt"])


class TheRealFile(unittest.TestCase):
    def test_every_path_bestpractices_links_to_exists(self):
        root = HERE.parent
        text = (root / ".bestpractices.json").read_text()
        self.assertTrue(check_repo_links.linked_paths(text), "the file links to no path; the pattern is wrong")
        self.assertEqual(check_repo_links.missing(text, root), [])


if __name__ == "__main__":
    result = unittest.main(exit=False, verbosity=1).result
    sys.exit(0 if result.wasSuccessful() else 1)
