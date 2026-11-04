"""Tests scripts/check_glibc.py: a release binary must need no glibc symbol
version above the floor the release supports.

Run with: python3 -I scripts/test_check_glibc.py
"""

import importlib.util
import pathlib
import shutil
import sys
import tempfile
import unittest

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_glibc", HERE / "check_glibc.py")
check_glibc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_glibc)


def line(version, name, flags="    "):
    return f"0000000000000000 {flags} DF *UND*\t0000000000000000 ({version}) {name}"


# objdump -T as binutils 2.42 prints it for the v0.2.0 x86_64 binary,
# shortened.
V020 = "\n".join(
    [
        "",
        "vcon-server:     file format elf64-x86-64",
        "",
        "DYNAMIC SYMBOL TABLE:",
        line("GLIBC_2.38", "__isoc23_strtol"),
        line("GLIBC_2.38", "__isoc23_sscanf"),
        line("GLIBC_2.34", "__libc_start_main"),
        line("GLIBC_2.2.5", "__cxa_finalize", flags="w   "),
        line("GLIBC_2.2.5", "close"),
        line("GLIBC_2.33", "fstat64"),
    ]
)


class Needed(unittest.TestCase):
    def test_each_symbol_is_read_with_its_version(self):
        needed = check_glibc.needed(V020)
        self.assertIn(("2.38", "__isoc23_strtol"), needed)
        self.assertIn(("2.2.5", "__cxa_finalize"), needed)
        self.assertEqual(len(needed), 6)

    def test_an_older_objdump_prints_the_version_without_parentheses(self):
        text = "0000000000000000      DF *UND*\t0000000000000000  GLIBC_2.17  memcpy"
        self.assertEqual(check_glibc.needed(text), [("2.17", "memcpy")])

    def test_other_libraries_versions_are_not_glibc(self):
        text = line("GCC_3.0", "_Unwind_Resume") + "\n" + line("GLIBC_2.17", "read")
        self.assertEqual(check_glibc.needed(text), [("2.17", "read")])


class TooNew(unittest.TestCase):
    def test_the_v020_binary_needs_more_than_2_34(self):
        self.assertEqual(
            check_glibc.too_new(V020, "2.34"),
            [("2.38", "__isoc23_sscanf"), ("2.38", "__isoc23_strtol")],
        )

    def test_a_binary_at_the_floor_passes(self):
        text = line("GLIBC_2.34", "__libc_start_main") + "\n" + line("GLIBC_2.2.5", "close")
        self.assertEqual(check_glibc.too_new(text, "2.34"), [])

    def test_versions_compare_as_numbers_not_text(self):
        # As text "2.4" > "2.34" and "2.10" < "2.9".
        self.assertEqual(check_glibc.too_new(line("GLIBC_2.4", "f"), "2.34"), [])
        self.assertEqual(check_glibc.too_new(line("GLIBC_2.10", "f"), "2.9"), [("2.10", "f")])

    def test_a_private_symbol_is_never_allowed(self):
        self.assertEqual(
            check_glibc.too_new(line("GLIBC_PRIVATE", "_dl_x"), "2.34"),
            [("PRIVATE", "_dl_x")],
        )


class Verdict(unittest.TestCase):
    def test_output_with_no_glibc_symbol_is_an_error_not_a_pass(self):
        # A static binary, a failed objdump or the wrong file: nothing was
        # checked, so nothing may pass.
        problems = check_glibc.verdict("vcon-server: file format elf64-x86-64\n", "2.34")
        self.assertEqual(len(problems), 1)
        self.assertIn("no glibc symbol", problems[0])

    def test_each_symbol_above_the_floor_is_named(self):
        problems = check_glibc.verdict(V020, "2.34")
        self.assertEqual(len(problems), 2)
        self.assertTrue(all("GLIBC_2.38" in p for p in problems))
        self.assertIn("__isoc23_strtol", "".join(problems))

    def test_a_binary_within_the_floor_has_no_problem(self):
        text = line("GLIBC_2.34", "__libc_start_main") + "\n" + line("GLIBC_2.33", "fstat64")
        self.assertEqual(check_glibc.verdict(text, "2.34"), [])

    def test_a_floor_that_is_not_a_version_is_refused(self):
        with self.assertRaises(ValueError):
            check_glibc.verdict(V020, "2.x")


class DocsFloor(unittest.TestCase):
    """The pages that tell a reader which glibc the release binary needs
    name the floor release.yml builds on, so the two cannot drift."""

    WORKFLOW = "env:\n  GLIBC_FLOOR: '2.34'\njobs: {}\n"

    def test_the_floor_is_read_from_the_workflow(self):
        self.assertEqual(check_glibc.workflow_floor(self.WORKFLOW), "2.34", "the env value")
        with self.assertRaises(ValueError):
            check_glibc.workflow_floor("env: {}\n")

    def test_a_page_that_names_another_floor_is_found(self):
        pages = {"install.md": "For the release binary, glibc 2.38 or later."}
        self.assertEqual(
            check_glibc.docs_floor_problems(self.WORKFLOW, pages),
            ["install.md says glibc 2.38 or later; release.yml builds for 2.34"],
            "the stale floor is named with its page",
        )

    def test_pages_that_name_the_floor_are_clean(self):
        pages = {"install.md": "glibc 2.34 or later.", "_index.md": "need glibc 2.34 or later, such as"}
        self.assertEqual(check_glibc.docs_floor_problems(self.WORKFLOW, pages), [], "both name 2.34")

    def test_a_page_that_names_no_floor_is_found(self):
        pages = {"install.md": "Any Linux will do."}
        self.assertEqual(
            check_glibc.docs_floor_problems(self.WORKFLOW, pages),
            ["install.md never says glibc 2.34 or later"],
            "each page must state the floor",
        )

    def tree(self, install, home):
        """A repository root holding release.yml and the two pages."""
        root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root)
        (root / ".github/workflows").mkdir(parents=True)
        (root / ".github/workflows/release.yml").write_text(self.WORKFLOW, encoding="utf-8")
        (root / "website/content/docs").mkdir(parents=True)
        (root / "website/content/docs/install.md").write_text(install, encoding="utf-8")
        (root / "website/content/_index.md").write_text(home, encoding="utf-8")
        return root

    def test_a_tree_whose_pages_name_the_floor_is_clean(self):
        root = self.tree("glibc 2.34 or later", "glibc 2.34 or later")
        self.assertEqual(check_glibc.docs_problems(root), [], "release.yml and both pages agree")

    def test_a_tree_with_a_stale_page_is_found(self):
        root = self.tree("glibc 2.34 or later", "glibc 2.38 or later")
        self.assertEqual(
            check_glibc.docs_problems(root),
            ["_index.md says glibc 2.38 or later; release.yml builds for 2.34"],
            "the home page's stale floor is named",
        )

    def test_a_missing_page_is_a_problem_not_a_pass(self):
        root = self.tree("glibc 2.34 or later", "")
        (root / "website/content/_index.md").unlink()
        self.assertEqual(
            check_glibc.docs_problems(root),
            ["cannot read website/content/_index.md"],
            "a page that is gone cannot state the floor",
        )

    def test_the_docs_mode_exits_1_on_a_problem(self):
        stale = self.tree("glibc 2.38 or later", "glibc 2.34 or later")
        clean = self.tree("glibc 2.34 or later", "glibc 2.34 or later")
        self.assertEqual(check_glibc.main(["check_glibc.py", "--docs", str(stale)]), 1, "stale")
        self.assertEqual(check_glibc.main(["check_glibc.py", "--docs", str(clean)]), 0, "clean")

if __name__ == "__main__":
    unittest.main()
