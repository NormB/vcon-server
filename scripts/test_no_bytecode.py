"""No script of scripts/ writes bytecode beside the scripts.

A script that loads another by path (importlib's spec_from_file_location
and exec_module) writes scripts/__pycache__/<name>.cpython-*.pyc unless
sys.dont_write_bytecode is set, and `python3 -I` ignores
PYTHONDONTWRITEBYTECODE. A .pyc is keyed on the source's mtime in whole
seconds and its size, so a mutation of the same length made within one
second of the last run runs the cached code of the file before it: a
mutation run then judges the wrong code. On 2026-11-25 the hook's
`python3 -I scripts/test_docs_test.py` left
scripts/__pycache__/gh_rate_limit.cpython-312.pyc (docs_test.py loads
gh_rate_limit.py by path), and `python3 -I scripts/contract_job.py` left
contract_diff.cpython-312.pyc; the directory is gitignored, so no gate saw
either.

The rule, in one place (`problems` below): a script of scripts/ that loads
a module by path sets `sys.dont_write_bytecode = True` in a statement of
its own at the top level of the file, above its first load. The flag is
the process's, so it covers every load after it, in a function or not.

The Behavior cases run each such script's top level (the scripts that are
no script test), and the loader
functions of those that load in a function, in a scratch copy of
scripts/ with bytecode writing on, and find no __pycache__ after.

Run with: python3 -I scripts/test_no_bytecode.py
"""

import ast
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent

# The calls that load a module from a file: each writes bytecode for it.
LOADS = {"spec_from_file_location", "exec_module", "run_path", "import_module", "__import__", "load_module"}
# The scripts that load in a function, and the function that loads, which
# the Behavior cases call (codeql_pin.gh_api also runs `gh`, so its load is
# held by the rule alone).
LOADER_FUNCTIONS = {"contract_job.py": "_contract_diff", "crash_run.py": "load_writers"}
# The scripts the defect was seen in: the rule must find them.
SEEN = {"docs_test.py", "contract_job.py"}


def _called_name(node):
    func = node.func
    if isinstance(func, ast.Attribute):
        return func.attr
    if isinstance(func, ast.Name):
        return func.id
    return None


def first_load(tree):
    """The line of the first call that loads a module from a file, or None."""
    lines = [node.lineno for node in ast.walk(tree) if isinstance(node, ast.Call) and _called_name(node) in LOADS]
    return min(lines) if lines else None


def _is_flag(node, value):
    if not isinstance(node, ast.Assign) or len(node.targets) != 1:
        return False
    target = node.targets[0]
    return (
        isinstance(target, ast.Attribute)
        and target.attr == "dont_write_bytecode"
        and isinstance(target.value, ast.Name)
        and target.value.id == "sys"
        and isinstance(node.value, ast.Constant)
        and node.value.value is value
    )


def problems(name, text):
    """What breaks the rule in the script `name` with source `text`."""
    try:
        tree = ast.parse(text, filename=name)
    except SyntaxError as error:
        return [f"scripts/{name}: not Python: {error.msg} (line {error.lineno})"]
    load = first_load(tree)
    if load is None:
        return []
    found = []
    flags = [node.lineno for node in tree.body if _is_flag(node, True)]
    if not flags or min(flags) > load:
        found.append(
            f"scripts/{name}: loads a module by path at line {load} without `sys.dont_write_bytecode = True` "
            "at the top level above it, so the load writes scripts/__pycache__"
        )
    unset = [node.lineno for node in ast.walk(tree) if _is_flag(node, False)]
    if unset:
        found.append(f"scripts/{name}: sets sys.dont_write_bytecode back to False at line {unset[0]}")
    return found


def loaders(directory=HERE):
    """The scripts of `directory` that load a module by path, sorted."""
    names = []
    for path in sorted(directory.glob("*.py")):
        try:
            tree = ast.parse(path.read_text(encoding="utf-8"))
        except SyntaxError:
            continue
        if first_load(tree) is not None:
            names.append(path.name)
    return names


def repository_problems(directory=HERE):
    return [p for path in sorted(directory.glob("*.py")) for p in problems(path.name, path.read_text(encoding="utf-8"))]


LOAD = 'import importlib.util\nspec = importlib.util.spec_from_file_location("m", "m.py")\n'
FLAG = "import sys\nsys.dont_write_bytecode = True\n"


class Rule(unittest.TestCase):
    def test_a_script_that_sets_the_flag_before_its_load_passes(self):
        self.assertEqual(problems("a.py", FLAG + LOAD), [], "the flag is set before the load")

    def test_a_script_that_loads_nothing_needs_no_flag(self):
        self.assertEqual(problems("a.py", "import json\nprint(json.dumps(1))\n"), [], "a plain import of the standard library writes nothing beside the script")

    def test_a_load_in_a_function_after_a_top_level_flag_passes(self):
        text = FLAG + "def load():\n    import importlib.util\n    return importlib.util.spec_from_file_location('m', 'm.py')\n"
        self.assertEqual(problems("a.py", text), [], "the flag is the process's, so a later call is covered")

    def test_a_script_without_the_flag_is_refused(self):
        found = problems("a.py", LOAD)
        self.assertTrue(any("a.py" in p and "line 2" in p for p in found), f"the script and the load's line are named: {found}")

    def test_a_flag_below_the_load_is_refused(self):
        found = problems("a.py", LOAD + FLAG)
        self.assertTrue(found, "the load above the flag wrote its bytecode already")

    def test_a_flag_inside_a_condition_is_refused(self):
        text = "import sys\nif sys.argv:\n    sys.dont_write_bytecode = True\n" + LOAD
        self.assertTrue(problems("a.py", text), "a flag that may not be set is no rule")

    def test_a_flag_inside_a_function_is_refused(self):
        text = "import sys\ndef quiet():\n    sys.dont_write_bytecode = True\n" + LOAD
        self.assertTrue(problems("a.py", text), "a flag in a function is set only if the function runs")

    def test_a_flag_set_to_false_is_refused(self):
        text = "import sys\nsys.dont_write_bytecode = False\n" + LOAD
        self.assertTrue(problems("a.py", text), "False writes bytecode")

    def test_a_flag_set_back_to_false_is_refused(self):
        found = problems("a.py", FLAG + LOAD + "sys.dont_write_bytecode = False\n")
        self.assertTrue(any("back to False" in p for p in found), f"turning it off again is named: {found}")

    def test_exec_module_alone_counts_as_a_load(self):
        self.assertTrue(problems("a.py", "spec.loader.exec_module(m)\n"), "exec_module writes the bytecode")

    def test_runpy_counts_as_a_load(self):
        self.assertTrue(problems("a.py", "import runpy\nrunpy.run_path('m.py')\n"), "run_path loads the file's imports")

    def test_import_module_counts_as_a_load(self):
        self.assertTrue(problems("a.py", "import importlib\nimportlib.import_module('m')\n"), "import_module writes bytecode")

    def test_the_text_of_a_comment_is_no_load(self):
        self.assertEqual(problems("a.py", "# spec_from_file_location(...)\nx = 'exec_module'\n"), [], "only calls count")

    def test_a_script_that_is_not_python_is_named(self):
        found = problems("a.py", "def (:\n")
        self.assertTrue(any("not Python" in p for p in found), f"the parse error is named: {found}")

    def test_an_annotated_flag_is_not_the_statement_the_rule_names(self):
        text = "import sys\nsys.dont_write_bytecode: bool = True\n" + LOAD
        self.assertTrue(problems("a.py", text), "the rule names one statement form, so a reader finds it")


class Repository(unittest.TestCase):
    def test_every_script_keeps_the_rule(self):
        self.assertEqual(repository_problems(), [], "every script of scripts/ that loads a module by path sets the flag first")

    def test_the_scripts_the_defect_was_seen_in_are_found(self):
        found = set(loaders())
        self.assertTrue(SEEN <= found, f"the rule reads {sorted(SEEN - found)} as loading nothing: {sorted(found)}")

    def test_each_loader_function_named_here_exists(self):
        for name, function in LOADER_FUNCTIONS.items():
            tree = ast.parse((HERE / name).read_text(encoding="utf-8"))
            names = {node.name for node in tree.body if isinstance(node, ast.FunctionDef)}
            self.assertIn(function, names, f"scripts/{name} defines {function}")


DRIVER = """import os, runpy, sys
script, function = sys.argv[1], sys.argv[2]
assert not sys.dont_write_bytecode, "the driver writes bytecode, as a plain `python3 -I` does"
spec = runpy.run_path(script, run_name="probe")
if function:
    spec[function]()
print("flag", sys.dont_write_bytecode)
"""


class Behavior(unittest.TestCase):
    """Each loader in a scratch copy of scripts/, bytecode writing on."""

    def run_in_copy(self, name, function=""):
        with tempfile.TemporaryDirectory() as tmp:
            copy = pathlib.Path(tmp) / "scripts"
            shutil.copytree(HERE, copy, ignore=shutil.ignore_patterns("__pycache__"))
            driver = pathlib.Path(tmp) / "driver.py"
            driver.write_text(DRIVER, encoding="utf-8")
            env = {k: v for k, v in os.environ.items() if k != "PYTHONDONTWRITEBYTECODE"}
            done = subprocess.run(
                [sys.executable, "-I", str(driver), str(copy / name), function],
                cwd=tmp, env=env, capture_output=True, text=True, timeout=120, check=False, stdin=subprocess.DEVNULL,
            )
            caches = sorted(str(p.relative_to(tmp)) for p in pathlib.Path(tmp).rglob("__pycache__"))
            return done, caches

    def test_each_loader_writes_no_bytecode(self):
        # A script test's top level reads files outside scripts/ (the
        # hooks, the pages), which the copy does not hold; the rule holds
        # those, and every one of them sets the flag on its first lines.
        names = [n for n in loaders() if not n.startswith("test_")]
        self.assertGreaterEqual(len(names), len(SEEN), f"the loaders found: {names}")
        for name in names:
            with self.subTest(script=name):
                done, caches = self.run_in_copy(name, LOADER_FUNCTIONS.get(name, ""))
                self.assertEqual(done.returncode, 0, f"scripts/{name} ran: {done.stderr[-2000:]}")
                self.assertEqual(caches, [], f"scripts/{name} wrote bytecode")

    def test_the_driver_sees_bytecode_written_without_the_flag(self):
        """The cases above prove something: a by-path load with the flag
        off writes a __pycache__ in the copy."""
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            (tmp / "m.py").write_text("X = 1\n", encoding="utf-8")
            (tmp / "loads.py").write_text(
                "import importlib.util, pathlib\n"
                "spec = importlib.util.spec_from_file_location('m', pathlib.Path(__file__).parent / 'm.py')\n"
                "m = importlib.util.module_from_spec(spec)\n"
                "spec.loader.exec_module(m)\n",
                encoding="utf-8",
            )
            (tmp / "driver.py").write_text(DRIVER, encoding="utf-8")
            done = subprocess.run([sys.executable, "-I", str(tmp / "driver.py"), str(tmp / "loads.py"), ""], cwd=tmp, capture_output=True, text=True, timeout=60, check=False)
            self.assertEqual(done.returncode, 0, done.stderr)
            self.assertTrue(list(tmp.rglob("__pycache__")), "without the flag the load writes bytecode")


if __name__ == "__main__":
    unittest.main()
