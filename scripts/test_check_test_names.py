"""Tests scripts/check_test_names.py: a Python test module must not define
a class, or a method within one class, under a name it already used,
since the later definition silently replaces the earlier one.

Run with: python3 -I scripts/test_check_test_names.py
"""

import importlib.util
import pathlib
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


if __name__ == "__main__":
    unittest.main()
