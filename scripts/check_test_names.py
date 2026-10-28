"""Checks that no Python test module defines a name twice in one scope.

In Python a second `class Selection` or a second `def test_a` in the same
class replaces the first without a word, so the first's tests never run.
scripts/test_check_package.py held Selection twice from 2026-10-26 to
2026-10-27: 49 test methods written, 44 run.

Read: every tracked scripts/test_*.py and every other tracked test_*.py.
Compared: the classes and functions at module level, and the methods of
each class. A name defined in an if or try block is not compared (two
branches may define one name on purpose), nor a property's setter or
deleter.

Run with: python3 -I scripts/check_test_names.py
(from the repository root). Exits 1 naming each name defined again.
"""

import ast
import pathlib
import posixpath
import subprocess
import sys


def is_accessor(node):
    """A @x.setter or @x.deleter, which reuses its property's name."""
    for decorator in getattr(node, "decorator_list", []):
        if isinstance(decorator, ast.Attribute) and decorator.attr in ("setter", "deleter", "getter"):
            return True
    return False


def scan(body, prefix, path, problems):
    seen = {}
    for node in body:
        if not isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        if is_accessor(node):
            continue
        if node.name in seen:
            kind = "class " if isinstance(node, ast.ClassDef) else ""
            problems.append(
                f"{path}:{node.lineno}: {kind}{prefix}{node.name} is defined again (first at line {seen[node.name]}); the first never runs"
            )
        else:
            seen[node.name] = node.lineno
        if isinstance(node, ast.ClassDef):
            scan(node.body, f"{prefix}{node.name}.", path, problems)


def duplicates(source, path):
    problems = []
    scan(ast.parse(source, filename=path).body, "", path, problems)
    return sorted(problems, key=lambda p: int(p.split(":")[1]))


def is_test_module(path):
    name = posixpath.basename(path)
    return name.startswith("test_") and name.endswith(".py")


def check(root, paths):
    problems = []
    for path in sorted(p for p in paths if is_test_module(p)):
        problems += duplicates((root / path).read_text(encoding="utf-8"), path)
    return problems


def main():
    root = pathlib.Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip())
    paths = subprocess.run(["git", "ls-files"], cwd=root, capture_output=True, text=True, check=True).stdout.splitlines()
    problems = check(root, paths)
    for problem in problems:
        print(f"check_test_names: {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"check_test_names: ok ({sum(1 for p in paths if is_test_module(p))} test modules)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
