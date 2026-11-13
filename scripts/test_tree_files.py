"""Tests scripts/tree_files.py: the files a check of the tree reads, git's
tracked ones in a repository, or every file under a directory that is none
(the commit hook's copy of the staged files).

Run with: python3 -I scripts/test_tree_files.py
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
spec = importlib.util.spec_from_file_location("tree_files", HERE / "tree_files.py")
tree_files = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tree_files)

# A git hook runs with GIT_DIR and GIT_INDEX_FILE set to the repository the
# hook belongs to. Every git command of these tests runs without them, so
# it acts on the test's own directory only.
ENV = {name: value for name, value in os.environ.items() if not name.startswith("GIT_")}
ENV["GIT_CONFIG_GLOBAL"] = os.devnull
ENV["GIT_CONFIG_SYSTEM"] = os.devnull
ENV["GIT_CEILING_DIRECTORIES"] = tempfile.gettempdir()


class Case(unittest.TestCase):
    def directory(self, *files):
        root = pathlib.Path(self.enterContext(tempfile.TemporaryDirectory())).resolve()
        for name in files:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text("x\n", encoding="utf-8")
        return root

    def repository(self, tracked, untracked=()):
        root = self.directory(*tracked, *untracked)
        self.git(root, "init", "-q", "-b", "main")
        if tracked:
            self.git(root, "add", "--", *tracked)
        return root

    def git(self, root, *arguments):
        subprocess.run(["git", *arguments], cwd=root, env=ENV, check=True, capture_output=True)


class Walked(Case):
    def test_success_every_file_by_its_path_from_the_directory(self):
        root = self.directory("b.txt", "a/c.txt", "a/b/d.py")
        self.assertEqual(tree_files.walked(root), ["a/b/d.py", "a/c.txt", "b.txt"])

    def test_success_files_and_directories_whose_names_start_with_a_dot(self):
        root = self.directory(".github/workflows/ci.yml", ".gitignore")
        self.assertEqual(tree_files.walked(root), [".github/workflows/ci.yml", ".gitignore"])

    def test_success_the_paths_are_sorted(self):
        root = self.directory("z", "m/a", "a")
        self.assertEqual(tree_files.walked(root), sorted(tree_files.walked(root)))

    def test_success_a_directory_given_as_text(self):
        root = self.directory("a.txt")
        self.assertEqual(tree_files.walked(str(root)), ["a.txt"])

    def test_edge_an_empty_directory_has_no_file(self):
        self.assertEqual(tree_files.walked(self.directory()), [])

    def test_edge_git_s_own_files_are_passed_over(self):
        root = self.directory(".git/config", ".git/objects/ab/cd", "a.txt")
        self.assertEqual(tree_files.walked(root), ["a.txt"])

    def test_edge_build_output_is_passed_over_at_any_depth(self):
        root = self.directory("target/debug/x", "fuzz/target/y", "src/target.rs")
        self.assertEqual(tree_files.walked(root), ["src/target.rs"])

    def test_edge_installed_packages_are_passed_over_at_any_depth(self):
        root = self.directory("node_modules/a/package-lock.json", "website/node_modules/b/package-lock.json", "website/package-lock.json")
        self.assertEqual(tree_files.walked(root), ["website/package-lock.json"])

    def test_edge_a_file_named_like_a_passed_over_directory_is_listed(self):
        root = self.directory("docs/target", "node_modules")
        self.assertEqual(tree_files.walked(root), ["docs/target", "node_modules"])

    def test_edge_an_empty_subdirectory_adds_nothing(self):
        root = self.directory("a.txt")
        (root / "empty").mkdir()
        self.assertEqual(tree_files.walked(root), ["a.txt"])

    def test_edge_a_link_to_a_directory_is_not_entered(self):
        root = self.directory("real/a.txt")
        (root / "link").symlink_to(root / "real", target_is_directory=True)
        self.assertEqual(tree_files.walked(root), ["real/a.txt"])


class Tracked(Case):
    def test_success_the_tracked_files_of_a_repository(self):
        root = self.repository(["a.txt", "scripts/test_a.py"])
        self.assertEqual(tree_files.tracked(root, ENV), (root, ["a.txt", "scripts/test_a.py"]))

    def test_success_from_a_subdirectory_the_paths_start_at_the_top(self):
        root = self.repository(["a.txt", "scripts/test_a.py"])
        self.assertEqual(tree_files.tracked(root / "scripts", ENV), (root, ["a.txt", "scripts/test_a.py"]))

    def test_success_a_name_with_a_space(self):
        root = self.repository(["a b.txt"])
        self.assertEqual(tree_files.tracked(root, ENV)[1], ["a b.txt"])

    def test_edge_an_untracked_file_is_not_listed(self):
        root = self.repository(["a.txt"], untracked=["b.txt"])
        self.assertEqual(tree_files.tracked(root, ENV)[1], ["a.txt"])

    def test_edge_a_repository_with_nothing_tracked(self):
        root = self.repository([])
        self.assertEqual(tree_files.tracked(root, ENV), (root, []))

    def test_edge_a_name_with_a_character_outside_ascii_is_not_quoted(self):
        root = self.repository(["café.txt"])
        self.assertEqual(tree_files.tracked(root, ENV)[1], ["café.txt"])

    def test_failure_outside_a_repository(self):
        root = self.directory("a.txt")
        with self.assertRaises(tree_files.NotARepository) as raised:
            tree_files.tracked(root, ENV)
        self.assertEqual(str(raised.exception), "not in a git repository; --walk reads the files under this directory")


class Listed(Case):
    def test_success_with_walk_the_directory_is_the_root(self):
        root = self.directory("a.txt")
        self.assertEqual(tree_files.listed(True, root, ENV), (root, ["a.txt"]))

    def test_success_without_walk_the_repository_is_the_root(self):
        root = self.repository(["a.txt"], untracked=["b.txt"])
        self.assertEqual(tree_files.listed(False, root, ENV), (root, ["a.txt"]))

    def test_edge_with_walk_in_a_repository_untracked_files_are_listed_too(self):
        root = self.repository(["a.txt"], untracked=["b.txt"])
        self.assertEqual(tree_files.listed(True, root, ENV), (root, ["a.txt", "b.txt"]))

    def test_failure_without_walk_outside_a_repository(self):
        with self.assertRaises(tree_files.NotARepository):
            tree_files.listed(False, self.directory("a.txt"), ENV)

    def test_success_with_walk_git_is_not_asked(self):
        # The hook's copy of the staged files is no repository, and the hook
        # runs with GIT_DIR naming another one.
        root = self.directory("a.txt")
        env = dict(ENV, GIT_DIR=str(root / "no-such-repository"), PATH="")
        self.assertEqual(tree_files.listed(True, root, env), (root, ["a.txt"]))


class Arguments(unittest.TestCase):
    def test_success_no_argument(self):
        self.assertIs(tree_files.walk_flag([]), False)

    def test_success_walk(self):
        self.assertIs(tree_files.walk_flag(["--walk"]), True)

    def test_failure_another_option(self):
        self.assertIsNone(tree_files.walk_flag(["--all"]))

    def test_failure_walk_twice(self):
        self.assertIsNone(tree_files.walk_flag(["--walk", "--walk"]))

    def test_failure_a_path(self):
        self.assertIsNone(tree_files.walk_flag(["."]))

    def test_failure_walk_and_a_path(self):
        self.assertIsNone(tree_files.walk_flag(["--walk", "."]))


if __name__ == "__main__":
    unittest.main()
