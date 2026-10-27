"""Tests scripts/check_glibc.py: a release binary must need no glibc symbol
version above the floor the release supports.

Run with: python3 -I scripts/test_check_glibc.py
"""

import importlib.util
import pathlib
import sys
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


if __name__ == "__main__":
    unittest.main()
