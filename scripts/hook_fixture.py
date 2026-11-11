"""Keeps the hook tests from re-running every script's own test per case.

scripts/test-hooks.sh commits one change per case in a throwaway repository,
and each commit that passes the scan runs the whole commit hook. The hook
runs every script's own test (scripts/test_*.py, scripts/test-*.sh), so each
of those ran once per case: about 25 times in each of two steps of CI's
"git hooks" job, which a 12-second script test pushed past its 20 minutes.

The fixture's first commit runs the hook with every script test as it is.
After it, `stub` replaces each script test in the fixture with a stand-in
that records its name in the file $HOOK_FIXTURE_CALLS names and passes, so
the cases still prove that the hook runs each of them. The tests in KEPT
stay as they are: a case commits a change that one of them must refuse.

The list of script tests is read from the hook itself, so a script test
added to the hook is stubbed without a change here.

Run with: python3 -I scripts/hook_fixture.py stub <fixture repository>
          python3 -I scripts/hook_fixture.py check <fixture repository> <calls file> <runs>
"""

import collections
import pathlib
import re
import sys

HOOK = ".githooks/pre-commit"
CALLS = "HOOK_FIXTURE_CALLS"
# Script tests a case of scripts/test-hooks.sh exercises, and why.
KEPT = {
    "scripts/test_check_repo_links.py": "refuses a Best Practices answer that links to a file that moved",
}
USAGE = "usage: hook_fixture.py stub <fixture repository> | check <fixture repository> <calls file> <runs>"

# The two ways the hook runs a script's own test, each a whole line.
RUNS = (
    re.compile(r"python3 -I (scripts/test_[A-Za-z0-9_]+\.py)"),
    re.compile(r"(scripts/test-[A-Za-z0-9-]+\.sh)"),
)
MENTION = re.compile(r"scripts/test[_-]")


class HookError(Exception):
    """The hook, or the fixture, is not in a shape this reader knows."""


def script_tests(hook_text):
    """Each script test the hook runs, as scripts/<name>, in the hook's order.

    A line that names a script test in any other form is refused, so a
    script test this reader cannot see is never left running once per case.
    """
    names = []
    for number, raw in enumerate(hook_text.splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#") or not MENTION.search(line):
            continue
        match = next((m for m in (run.fullmatch(line) for run in RUNS) if m), None)
        if match is None:
            raise HookError(f"{HOOK} line {number}: a script test run in a form this reader does not know: {line}")
        name = match.group(1)
        if name in names:
            raise HookError(f"{HOOK} line {number}: {name} is run twice")
        names.append(name)
    if not names:
        raise HookError(f"{HOOK} runs no script test")
    return names


def stubbed(hook_text):
    """The script tests the fixture replaces: all of them but those in KEPT."""
    names = script_tests(hook_text)
    for name in KEPT:
        if name not in names:
            raise HookError(f"{HOOK} no longer runs {name}, which scripts/hook_fixture.py keeps for a case")
    return [name for name in names if name not in KEPT]


def stub_text(path):
    """The stand-in for the script test at path."""
    if not any(run.fullmatch(path) or run.fullmatch(f"python3 -I {path}") for run in RUNS):
        raise HookError(f"{path} is not a script test's path")
    what = f"Stands in for {path} in the hook tests' fixture (scripts/hook_fixture.py)."
    if path.endswith(".py"):
        return (
            f'"""{what}"""\n'
            "\n"
            "import os\n"
            "import sys\n"
            "\n"
            f'calls = os.environ.get("{CALLS}")\n'
            "if not calls:\n"
            f'    sys.exit("{path}: stand-in: {CALLS} is not set")\n'
            'with open(calls, "a", encoding="utf-8") as record:\n'
            f'    record.write("{path}\\n")\n'
        )
    return (
        "#!/usr/bin/env bash\n"
        f"# {what}\n"
        "set -euo pipefail\n"
        f'[ -n "${{{CALLS}:-}}" ] || {{ echo "{path}: stand-in: {CALLS} is not set" >&2; exit 1; }}\n'
        f"printf '%s\\n' '{path}' >> \"${CALLS}\"\n"
    )


def read_hook(root):
    try:
        return (pathlib.Path(root) / HOOK).read_text(encoding="utf-8")
    except OSError as error:
        raise HookError(f"{pathlib.Path(root) / HOOK}: {error.strerror}") from error


def install(root):
    """Replaces each stubbed script test under root; returns their paths."""
    root = pathlib.Path(root)
    names = stubbed(read_hook(root))
    # Every file is checked before the first is written.
    for name in names:
        path = root / name
        if path.is_symlink():
            raise HookError(f"{path} is a symbolic link; a stand-in would be written through it")
        if not path.is_file():
            raise HookError(f"{path}: the hook runs it and the fixture does not hold it")
    for name in names:
        # Written in place, so the file keeps its mode.
        (root / name).write_text(stub_text(name), encoding="utf-8")
    return names


def findings(names, calls_text, runs):
    """What is wrong when each of names should have been called runs times."""
    counts = collections.Counter(line for line in calls_text.splitlines() if line.strip())
    found = [f"{name}: the hook ran it {counts[name]} times, not {runs}" for name in names if counts[name] != runs]
    found += [
        f"{name}: recorded a call, and is not a stubbed script test"
        for name in sorted(set(counts) - set(names))
    ]
    return found


def main(argv):
    if len(argv) == 2 and argv[0] == "stub":
        command = "stub"
    elif len(argv) == 4 and argv[0] == "check" and argv[3].isascii() and argv[3].isdigit():
        command = "check"
    else:
        print(USAGE, file=sys.stderr)
        return 2
    try:
        if command == "stub":
            for name in install(argv[1]):
                print(name)
            return 0
        names = stubbed(read_hook(argv[1]))
        calls = pathlib.Path(argv[2])
        text = calls.read_text(encoding="utf-8") if calls.exists() else ""
        found = findings(names, text, int(argv[3]))
    except HookError as error:
        print(f"hook_fixture: {error}", file=sys.stderr)
        return 1
    for finding in found:
        print(f"hook_fixture: {finding}", file=sys.stderr)
    if found:
        return 1
    print(f"hook_fixture: the hook ran each of {len(names)} script tests {argv[3]} time(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
