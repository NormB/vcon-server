#!/usr/bin/env python3
"""Whether a push must run the hook tests that CI's "git hooks" job runs.

    git log --name-only --format= <range> | hook_tests.py needed [--root DIR]

scripts/test-hooks.sh and scripts/test-pre-push.sh test the hooks in
throwaway repositories made from files of this tree. Neither commit hook
runs them, so a change to a file they copy could pass every local hook and
fail only in CI: on #211 a script test the commit hook runs read a manifest
that test-hooks.sh's fixture did not have.

THE RULE: the paths they read are each "$src/<path>" their text names (a
directory stands for every file under it), and a push needs them when one
of the files it changes is one of those paths or under one. The list is
read from the two scripts, so a file they start to copy is covered with no
change here.

`needed` reads the changed paths, one per line, on stdin. Exit 0 and one
line when the tests are needed, 1 and one line when they are not, 2 with
the reason on stderr when the rule cannot be read. .githooks/pre-push asks
it once per push and runs TESTS when it answers 0.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

# The hook tests a push runs, in this order: each copies the files it reads.
TESTS = ("scripts/test-hooks.sh", "scripts/test-pre-push.sh")
# "$src/<path>": a path of the tree. "$src/$name" (a variable) is no path.
_SRC = re.compile(r'"\$src/([^"$]+)"')


class HookTestsError(Exception):
    """The rule cannot be read; the message says why."""


def read_paths(text: str) -> list[str]:
    """Each path of the tree a hook test's text copies or reads, once, sorted."""
    found = set()
    for line in text.splitlines():
        if line.strip().startswith("#"):
            continue
        for match in _SRC.finditer(line):
            path = match.group(1).rstrip("/")
            if path.startswith("/") or ".." in path.split("/"):
                raise HookTestsError(f"a hook test reads {path!r}, which is not a path of the tree")
            found.add(path)
    return sorted(found)


def tree_read_paths(root) -> list[str]:
    """The paths the hook tests of the tree at `root` read."""
    found = set()
    for name in TESTS:
        path = pathlib.Path(root) / name
        try:
            found.update(read_paths(path.read_text(encoding="utf-8")))
        except OSError as error:
            raise HookTestsError(f"cannot read {name}: {error}") from error
    return sorted(found)


def needed(changed, read) -> list[str]:
    """The changed paths the hook tests read, in the order given, each once."""
    out = []
    for raw in changed:
        path = raw.strip()
        if not path or path in out:
            continue
        if any(path == r or path.startswith(r + "/") for r in read):
            out.append(path)
    return out


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Whether a push must run the hook tests.")
    sub = parser.add_subparsers(dest="command")
    one = sub.add_parser("needed")
    one.add_argument("--root", default=str(pathlib.Path(__file__).resolve().parent.parent))
    try:
        args = parser.parse_args(argv)
    except SystemExit:
        return 2
    if args.command != "needed":
        print("hook_tests: usage: hook_tests.py needed [--root DIR]", file=sys.stderr)
        return 2
    try:
        found = needed(sys.stdin.read().splitlines(), tree_read_paths(args.root))
    except HookTestsError as error:
        print(f"hook_tests: {error}", file=sys.stderr)
        return 2
    if not found:
        print("hook tests: not needed: the push changes no file they read")
        return 1
    more = f" and {len(found) - 1} more files they read" if len(found) > 1 else ", which they read"
    print(f"hook tests: needed: the push changes {found[0]}{more}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
