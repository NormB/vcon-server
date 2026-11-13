"""Tests scripts/check_test_names.py: a Python test module must not define
a class, or a method within one class, under a name it already used,
since the later definition silently replaces the earlier one.

Run with: python3 -I scripts/test_check_test_names.py
"""

import importlib.util
import os
import pathlib
import subprocess
import sys
import tempfile
import textwrap
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_test_names", HERE / "check_test_names.py")
check_test_names = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_test_names)


def found(source):
    return check_test_names.duplicates(textwrap.dedent(source), "t.py")


class Duplicates(unittest.TestCase):
    def test_distinct_names_are_fine(self):
        self.assertEqual(
            found(
                """
                class A:
                    def test_x(self): pass
                    def test_y(self): pass
                class B:
                    def test_x(self): pass
                """
            ),
            [],
        )

    def test_a_class_defined_twice_is_named_with_both_lines(self):
        self.assertEqual(
            found(
                """
                class Selection:
                    def test_a(self): pass

                class Selection:
                    def test_a(self): pass
                """
            ),
            ["t.py:5: class Selection is defined again (first at line 2); the first never runs"],
        )

    def test_a_method_defined_twice_in_one_class_is_named(self):
        self.assertEqual(
            found(
                """
                class A:
                    def test_a(self): pass
                    def test_b(self): pass
                    def test_a(self): pass
                """
            ),
            ["t.py:5: A.test_a is defined again (first at line 3); the first never runs"],
        )

    def test_a_top_level_function_defined_twice_is_named(self):
        self.assertEqual(
            found(
                """
                def helper(): pass
                def helper(): pass
                """
            ),
            ["t.py:3: helper is defined again (first at line 2); the first never runs"],
        )

    def test_a_property_setter_is_not_a_duplicate(self):
        self.assertEqual(
            found(
                """
                class A:
                    @property
                    def x(self): return 1
                    @x.setter
                    def x(self, v): pass
                """
            ),
            [],
        )

    def test_a_definition_under_if_is_not_compared(self):
        # Two branches of an if define one name on purpose.
        self.assertEqual(
            found(
                """
                import sys
                if sys.platform == "linux":
                    def f(): pass
                else:
                    def f(): pass
                """
            ),
            [],
        )

    def test_every_test_module_in_a_tree_is_read(self):
        with tempfile.TemporaryDirectory() as root:
            (pathlib.Path(root) / "scripts").mkdir()
            (pathlib.Path(root) / "scripts/test_a.py").write_text("class A: pass\nclass A: pass\n")
            (pathlib.Path(root) / "scripts/tool.py").write_text("def f(): pass\ndef f(): pass\n")
            (pathlib.Path(root) / "scripts/test_b.py").write_text("class B: pass\n")
            problems = check_test_names.check(pathlib.Path(root), ["scripts/test_a.py", "scripts/test_b.py", "scripts/tool.py"])
            self.assertEqual(problems, ["scripts/test_a.py:2: class A is defined again (first at line 1); the first never runs"])


TWICE = "def test_a(): pass\ndef test_a(): pass\n"
ONCE = "def test_a(): pass\n"
OUTSIDE = "check_test_names: not in a git repository; --walk reads the files under this directory\n"


class Command(unittest.TestCase):
    """The command: in a checkout it reads git's tracked files; with --walk,
    in the commit hook's copy of the staged files, every file."""

    # Without git's variables, so a hook's GIT_DIR or GIT_INDEX_FILE cannot
    # point git at the repository the hook belongs to.
    ENV = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")} | {"GIT_CEILING_DIRECTORIES": tempfile.gettempdir()}

    def run_in(self, files, *arguments, tracked=None, env=None):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for name, text in files.items():
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_text(text, encoding="utf-8")
            if tracked is not None:
                subprocess.run(["git", "init", "-q"], cwd=root, env=self.ENV, check=True)
                subprocess.run(["git", "add", "--", *tracked], cwd=root, env=self.ENV, check=True)
            run = subprocess.run([sys.executable, "-I", str(HERE / "check_test_names.py"), *arguments], cwd=root, env=env or self.ENV, capture_output=True, text=True)
        return run.returncode, run.stdout, run.stderr

    def test_success_with_walk_the_command_needs_no_repository(self):
        files = {"scripts/test_a.py": ONCE, "clients/python/test_b.py": ONCE, "scripts/tool.py": TWICE}
        self.assertEqual(self.run_in(files, "--walk"), (0, "check_test_names: ok (2 test modules)\n", ""))

    def test_success_in_a_repository_the_tracked_modules_are_read(self):
        files = {"scripts/test_a.py": ONCE, "scripts/test_b.py": ONCE}
        self.assertEqual(self.run_in(files, tracked=["scripts/test_a.py", "scripts/test_b.py"]), (0, "check_test_names: ok (2 test modules)\n", ""))

    def test_failure_with_walk_a_name_defined_twice(self):
        code, out, err = self.run_in({"scripts/test_a.py": TWICE}, "--walk")
        self.assertEqual((code, out), (1, ""))
        self.assertEqual(err, "check_test_names: scripts/test_a.py:2: test_a is defined again (first at line 1); the first never runs\n")

    def test_failure_in_a_repository_a_name_defined_twice(self):
        code, _, err = self.run_in({"scripts/test_a.py": TWICE}, tracked=["scripts/test_a.py"])
        self.assertEqual(code, 1)
        self.assertIn("scripts/test_a.py:2: test_a is defined again", err)

    def test_failure_outside_a_repository_without_walk_the_command_says_so(self):
        self.assertEqual(self.run_in({"scripts/test_a.py": ONCE}), (2, "", OUTSIDE))

    def test_failure_another_argument_is_refused(self):
        self.assertEqual(self.run_in({"scripts/test_a.py": ONCE}, "--all"), (2, "", "usage: check_test_names.py [--walk]\n"))

    def test_edge_in_a_repository_an_untracked_module_is_not_read(self):
        files = {"scripts/test_a.py": ONCE, "scripts/test_new.py": TWICE}
        self.assertEqual(self.run_in(files, tracked=["scripts/test_a.py"]), (0, "check_test_names: ok (1 test modules)\n", ""))

    def test_edge_with_walk_installed_packages_are_passed_over(self):
        files = {"scripts/test_a.py": ONCE, "website/node_modules/x/test_x.py": TWICE}
        self.assertEqual(self.run_in(files, "--walk"), (0, "check_test_names: ok (1 test modules)\n", ""))

    def test_edge_with_walk_under_a_hook_s_git_variables(self):
        # The commit hook runs the check in its copy with GIT_DIR set.
        env = dict(self.ENV, GIT_DIR=str(HERE.parent / "no-such-directory"), GIT_INDEX_FILE="no-such-index")
        self.assertEqual(self.run_in({"scripts/test_a.py": ONCE}, "--walk", env=env), (0, "check_test_names: ok (1 test modules)\n", ""))


if __name__ == "__main__":
    unittest.main()
