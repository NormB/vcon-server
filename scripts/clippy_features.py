"""Lints each feature set of .github/clippy-feature-sets.txt with clippy,
warnings denied, and checks that the commit hook and CI both lint from that
list.

The default features are linted by `cargo clippy --workspace` in the commit
hook and in CI. Code under another feature set (`cfg(feature = "postgres")`,
a build without the default features) is compiled only when that set is
asked for. Before this script the workflow named those sets and the hook
named none: on 2026-11-12 an unused `mut` in code only the postgres feature
compiles passed the hook and failed CI's lint. The rule:

  .github/clippy-feature-sets.txt   one row per feature set,
                                    `<group>: <arguments of cargo clippy>`
  the commit hook                   runs `python3 -I scripts/clippy_features.py
                                    run --quiet`: every row; it may also
                                    run `check`, as CI's scan job does
  a workflow step                   runs `python3 -I scripts/clippy_features.py
                                    run <group>`; every group is run by some
                                    step
  cargo clippy with a feature       nowhere else in the hook or a workflow
  selection

Run with: python3 -I scripts/clippy_features.py run [--quiet] [<group> ...]
          python3 -I scripts/clippy_features.py check
(`run` from anywhere in the repository; it runs cargo at the repository's
top.) `run` exits with the code of the first lint that fails, `check` with 1
naming each place that breaks the rule, and both with 2 when the list cannot
be read or a group is not in it.
"""

import pathlib
import re
import subprocess
import sys

LIST = ".github/clippy-feature-sets.txt"
HOOK = ".githooks/pre-commit"
WORKFLOWS = ".github/workflows"
SCRIPT = "scripts/clippy_features.py"
HOOK_RUN = f"python3 -I {SCRIPT} run --quiet"
USAGE = "usage: clippy_features.py run [--quiet] [<group> ...] | check"

GROUP = re.compile(r"[a-z][a-z0-9-]*")
ARGUMENT = re.compile(r"[A-Za-z0-9_,=./:+-]+")
# cargo, an optional +toolchain, then clippy.
CLIPPY = re.compile(r"\bcargo\s+(?:\+\S+\s+)?clippy\b(.*)")
# This script by name, and not its test (scripts/test_clippy_features.py).
MENTION = re.compile(r"(?<![A-Za-z0-9_])clippy_features\.py")
# The ways a workflow step runs this script, each a whole command.
WORKFLOW_RUN = re.compile(rf"python3 -I {re.escape(SCRIPT)} run(?: --quiet)?((?: [a-z][a-z0-9-]*)*)")
WORKFLOW_CHECK = f"python3 -I {SCRIPT} check"
# A step written on one line: `- run: <command>` or `run: <command>`.
STEP = re.compile(r"(?:-\s+)?run:\s+(?!\|)(.*)")


class ListError(Exception):
    """The list, or a group asked of it, is not in a shape this reader knows."""


def selects_features(arguments):
    """Whether cargo arguments ask for a feature set other than the default."""
    for argument in arguments:
        if argument == "--":
            break
        if argument in ("--no-default-features", "--all-features") or argument.startswith(("--features", "-F")):
            return True
    return False


def parse(text):
    """The rows of the list, each (group, arguments), in the list's order."""
    rows = []
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        where = f"{LIST} line {number}"
        group, colon, rest = line.partition(":")
        if not colon or not GROUP.fullmatch(group):
            raise ListError(f"{where}: a row is `<group>: <arguments>`, the group in lower-case letters, digits and hyphens: {line}")
        arguments = rest.split()
        if not arguments:
            raise ListError(f"{where}: the group `{group}` has no arguments")
        for argument in arguments:
            if argument == "--":
                raise ListError(f"{where}: `--` and the lint arguments are added by {SCRIPT}; leave them out")
            if argument in ("--quiet", "-q"):
                raise ListError(f"{where}: {argument} is the caller's choice (`run --quiet`); leave it out")
            if not ARGUMENT.fullmatch(argument):
                raise ListError(f"{where}: a character in `{argument}` that no cargo argument of this list needs")
        for needed, why in (("--all-targets", "the tests are linted too"), ("--locked", "the lock file is not changed")):
            if needed not in arguments:
                raise ListError(f"{where}: {needed} is missing ({why})")
        for index, argument in enumerate(arguments):
            last = index + 1 == len(arguments)
            if argument == "--features=" or (argument == "--features" and (last or arguments[index + 1].startswith("-"))):
                raise ListError(f"{where}: --features names no feature")
        if not selects_features(arguments):
            raise ListError(f"{where}: no feature set is named (--features, --no-default-features); the default features are linted by `cargo clippy --workspace`")
        if any(arguments == other for _, other in rows):
            raise ListError(f"{where}: the same arguments twice: {' '.join(arguments)}")
        rows.append((group, arguments))
    if not rows:
        raise ListError(f"{LIST}: no row")
    return rows


def groups_of(rows):
    """The groups of the list, each once, in the list's order."""
    return list(dict.fromkeys(group for group, _ in rows))


def commands(rows, groups, quiet):
    """The cargo command of each row in groups (of every row when groups is
    empty), in the list's order."""
    known = groups_of(rows)
    for group in groups:
        if group not in known:
            raise ListError(f"{LIST} holds no group `{group}`; its groups: {', '.join(known)}")
    tail = (["--quiet"] if quiet else []) + ["--", "-D", "warnings"]
    return [["cargo", "clippy", *arguments, *tail] for group, arguments in rows if not groups or group in groups]


def run(rows, groups, quiet, runner, root, err):
    """Runs each command in root until one fails; answers its exit code."""
    for command in commands(rows, groups, quiet):
        if not quiet:
            print(f"clippy_features: {' '.join(command)}", file=err, flush=True)
        code = runner(command, cwd=root)
        if code != 0:
            print(f"clippy_features: failed: {' '.join(command)}", file=err)
            return code if code > 0 else 1
    return 0


def logical_lines(text):
    """(line number, command) for each line that is not blank or a comment,
    a one-line workflow step reduced to its command."""
    for number, raw in enumerate(text.splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        step = STEP.fullmatch(line)
        yield number, (step.group(1).strip() if step else line)


def outside_the_list(where, text):
    """Each cargo clippy command in text that names a feature set itself."""
    found = []
    for number, line in logical_lines(text):
        match = CLIPPY.search(line)
        if match and selects_features(match.group(1).split()):
            found.append(f"{where} line {number}: lints a feature set outside {LIST}: {line}")
    return found


def problems(list_text, hook_text, workflows):
    """What breaks the rule. workflows maps each workflow's file name to its text."""
    try:
        rows = parse(list_text)
    except ListError as error:
        return [str(error)]
    known = groups_of(rows)
    found = outside_the_list(HOOK, hook_text)
    hook_runs = False
    for number, line in logical_lines(hook_text):
        if line == HOOK_RUN:
            hook_runs = True
        elif line == WORKFLOW_CHECK:
            # The hook runs this check too, as CI's scan job does.
            continue
        elif MENTION.search(line):
            found.append(f"{HOOK} line {number}: runs {SCRIPT} in a form this reader does not know: {line}")
    if not hook_runs:
        found.append(f"{HOOK}: never runs `{HOOK_RUN}`, which lints every row of {LIST}")
    run_by_a_workflow = set()
    for name in sorted(workflows):
        where = f"{WORKFLOWS}/{name}"
        found += outside_the_list(where, workflows[name])
        for number, line in logical_lines(workflows[name]):
            if not MENTION.search(line) or line == WORKFLOW_CHECK:
                continue
            match = WORKFLOW_RUN.fullmatch(line)
            if match is None:
                found.append(f"{where} line {number}: runs {SCRIPT} in a form this reader does not know: {line}")
                continue
            named = match.group(1).split()
            for group in named:
                if group not in known:
                    found.append(f"{where} line {number}: runs the group `{group}`, which {LIST} does not hold")
            run_by_a_workflow.update(named or known)
    found += [
        f"no workflow runs the group `{group}` of {LIST}: add a step `python3 -I {SCRIPT} run {group}`"
        for group in known
        if group not in run_by_a_workflow
    ]
    return found


def read(path):
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise ListError(f"{path}: {error.strerror}") from error


def repository_problems(root):
    """problems() for the list, the hook and the workflows under root."""
    root = pathlib.Path(root)
    try:
        list_text = read(root / LIST)
        hook_text = read(root / HOOK)
    except ListError as error:
        return [str(error)]
    workflows = {
        path.name: path.read_text(encoding="utf-8")
        for path in sorted((root / WORKFLOWS).glob("*.y*ml"))
    }
    return problems(list_text, hook_text, workflows)


def call(command, cwd=None):
    """Runs one command; its exit code."""
    return subprocess.call(command, cwd=cwd)


def main(argv, root, runner, out, err):
    if argv == ["check"]:
        found = repository_problems(root)
        for problem in found:
            print(f"clippy_features: {problem}", file=err)
        if found:
            return 1
        rows = parse(read(pathlib.Path(root) / LIST))
        print(
            f"clippy_features: {len(rows)} feature sets in {len(groups_of(rows))} groups, "
            "each linted by the commit hook and by a workflow",
            file=out,
        )
        return 0
    if not argv or argv[0] != "run":
        print(USAGE, file=err)
        return 2
    quiet = "--quiet" in argv[1:]
    groups = [argument for argument in argv[1:] if argument != "--quiet"]
    if any(group.startswith("-") for group in groups):
        print(USAGE, file=err)
        return 2
    try:
        rows = parse(read(pathlib.Path(root) / LIST))
        return run(rows, groups, quiet, runner, root, err)
    except ListError as error:
        print(f"clippy_features: {error}", file=err)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:], pathlib.Path(__file__).resolve().parent.parent, call, sys.stdout, sys.stderr))
