"""Checks that every script a workflow runs is run by a git hook too.

A check that only CI runs reports a failure 10 to 30 minutes after the push,
on a commit the hooks accepted. On 2026-11-13 a how-to page lost a line of
expected output; the commit hook and the push hook passed, and CI's how-to
job was the first to run scripts/test_docs_test.py, which refused it. No
rule compared what the workflows run with what the hooks run.

The rule: each script of scripts/ (a .py or .sh file) that a run step of a
workflow (.github/workflows/*.yml) or of a composite action
(.github/actions/*/action.yml) runs is run by a hook in .githooks/, or is
listed in CI_ONLY below with the reason it runs only in CI. An entry of
CI_ONLY that a hook runs, or that no workflow runs any more, is refused, so
the list holds no stale line.

A script is compared with its subcommand, when the first argument is one
(a word of lower-case letters): `scripts/clippy_features.py run`, which the
hook runs, does not cover `scripts/clippy_features.py check`. Options are
not compared: `scripts/check_style.py --walk` in the hook covers
`scripts/check_style.py` in CI. What follows a script in WRAPPERS is the
command it runs, not a subcommand.

Run with: python3 -I scripts/check_hook_steps.py
(from anywhere in the repository; needs PyYAML). Exits 1 naming each script
that breaks the rule.
"""

import pathlib
import re
import sys

SCRIPT = "scripts/check_hook_steps.py"
HOOKS = ".githooks"
HOOK = f"{HOOKS}/pre-commit"
WORKFLOWS = ".github/workflows"
ACTIONS = ".github/actions"

# Scripts whose first argument is no subcommand: another command
# (retry.sh), the names of packages (dnf_install.py).
WRAPPERS = ("scripts/retry.sh", "scripts/dnf_install.py")

# A script of scripts/ in a shell text, and the argument after it.
COMMAND = re.compile(r"(?<![A-Za-z0-9_.-])scripts/([A-Za-z0-9_-]+\.(?:py|sh))(?![A-Za-z0-9_.-])[\"']?(?:[ \t]+([^\s;|&<>()\"']+))?")
SUBCOMMAND = re.compile(r"[a-z]+")
COMMENT = re.compile(r"(?:^|\s)#.*$")
# A git hook's file name: words of lower-case letters joined by hyphens.
HOOK_NAME = re.compile(r"[a-z]+(?:-[a-z]+)*")

# What only CI runs, and why no hook runs it.
CI_ONLY = {
    # Tests of the hooks themselves: each commits in a throwaway repository,
    # which runs the commit hook, so the commit hook cannot run them.
    "scripts/test-hooks.sh": "commits in a throwaway repository, and each commit there runs the commit hook",
    "scripts/test-pre-push.sh": "pushes in a throwaway repository with a signing key of its own; run by hand when a hook changes",
    "scripts/test-hook-isolation.sh": "runs the two hook tests from inside a real hook, in a scratch copy of the tree",
    # Checks that need a tool, a service or a build the commit hook has none of.
    "scripts/check-site.sh": "builds the site with zola, which the commit hook does not install",
    "scripts/test-site.sh": "needs zola, Node and the Playwright browser",
    "scripts/test-console.sh": "needs Node and the Playwright browser, and starts the server it builds",
    "scripts/icons.py": "needs the Python packages of scripts/icons-requirements.txt",
    "scripts/check_required_checks.py": "reads the main branch's ruleset from the GitHub API",
    "scripts/test-check-package.sh": "packages the crates in two more copies of the tree; the hook runs scripts/check_package.py once",
    "scripts/run-clients.sh": "runs each client example against a live server; needs go and node besides the build",
    "scripts/docs_test.py run": "runs a how-to scenario in a container, against a built server",
    "scripts/fuzz.sh run": "runs a fuzz target for a fixed time on the nightly toolchain",
    "scripts/fuzz.sh plant": "plants a panic and runs a fuzz target until it finds it",
    "scripts/reproducible-build.sh check": "builds the release binary twice in the release container",
    # Steps that prepare or describe a CI job and check nothing of a commit.
    "scripts/retry.sh": "repeats a download of a CI job; the hook downloads nothing",
    "scripts/fetch-pinned.sh": "downloads a pinned file for a CI job",
    "scripts/apt-mirror-setup.sh": "configures apt on the runner or in a container",
    "scripts/dnf_install.py": "installs packages in the release build container",
    "scripts/release-build-env.sh": "prepares the release build container",
    "scripts/reproducible-build.sh build": "builds the release binary",
    "scripts/release_notes.py": "writes the text of a GitHub release",
    "scripts/published_metrics.py": "derives the home page's numbers from the output of a CI run",
    "scripts/wait-for-s3.sh": "waits for the object store a CI job starts",
    "scripts/pg-test-server.sh start": "starts the PostgreSQL servers of a CI job",
    "scripts/pg-test-server.sh stop": "stops the PostgreSQL servers of a CI job",
    "scripts/pg-test-server.sh leftovers": "lists what the PostgreSQL tests of a CI job left behind",
    "scripts/rustfmt.sh install": "installs the pinned rustfmt on the runner",
    "scripts/nextest.sh install": "installs the pinned cargo-nextest on the runner",
    "scripts/nextest.sh pin": "prints the pinned cargo-nextest for the step that downloads it",
    "scripts/nextest.sh check": "checks the cargo-nextest that the coverage job installed",
    "scripts/nextest.sh list": "lists the tests the sanitizer job then runs",
    "scripts/fuzz.sh list": "lists the fuzz targets for the job matrix",
    "scripts/docs_test.py list": "lists the how-to scenarios for the job matrix",
    "scripts/docs_test.py artifact": "names the artifact of a how-to scenario's job",
    "scripts/ci_success.py verdict": "reads the results of the jobs of one CI run",
}


def logical_lines(text):
    """The lines of a shell text, a line ending in a backslash joined to
    the next, comments left out."""
    lines = []
    pending = ""
    for raw in text.splitlines():
        line = pending + COMMENT.sub("", raw)
        if line.endswith("\\"):
            pending = line[:-1] + " "
            continue
        pending = ""
        lines.append(line)
    if pending:
        lines.append(pending)
    return lines


def units(text):
    """Each script the shell text runs, as `scripts/<name>` or
    `scripts/<name> <subcommand>`, once, in the order of first appearance."""
    found = []
    for line in logical_lines(text):
        position = 0
        while match := COMMAND.search(line, position):
            unit = f"scripts/{match.group(1)}"
            argument = match.group(2)
            is_subcommand = unit not in WRAPPERS and argument is not None and SUBCOMMAND.fullmatch(argument)
            if is_subcommand:
                unit = f"{unit} {argument}"
            if unit not in found:
                found.append(unit)
            # An argument that is no subcommand is read again: it may be a
            # script itself.
            position = match.end() if is_subcommand else match.end(1)
    return found


def run_texts(document):
    """The text of every run step of a parsed workflow or composite action."""
    if not isinstance(document, dict):
        return []
    jobs = document.get("jobs")
    step_lists = [(job or {}).get("steps") for job in jobs.values()] if isinstance(jobs, dict) else []
    runs = document.get("runs")
    if isinstance(runs, dict):
        step_lists.append(runs.get("steps"))
    return [str(step["run"]) for steps in step_lists for step in steps or [] if isinstance(step, dict) and "run" in step]


def problems(workflows, hooks, ci_only):
    """One line per script that breaks the rule.

    workflows: file name to parsed YAML, of each workflow and composite
    action. hooks: hook name to text. ci_only: unit to reason.
    """
    in_ci = {}
    for name in sorted(workflows):
        for text in run_texts(workflows[name]):
            for unit in units(text):
                files = in_ci.setdefault(unit, [])
                if name not in files:
                    files.append(name)
    in_hooks = {}
    for name in sorted(hooks):
        for unit in units(hooks[name]):
            in_hooks.setdefault(unit, name)
    found = []
    for unit in sorted(in_ci):
        if unit not in in_hooks and unit not in ci_only:
            found.append(
                f"{', '.join(in_ci[unit])}: runs `{unit}`, which no hook runs: add it to {HOOK}, "
                f"or to CI_ONLY in {SCRIPT} with the reason it runs only in CI"
            )
    for unit in sorted(ci_only):
        if unit in in_hooks:
            found.append(f"CI_ONLY lists `{unit}`, which {HOOKS}/{in_hooks[unit]} runs: remove it from the list")
        elif unit not in in_ci:
            found.append(f"CI_ONLY lists `{unit}`, which no workflow runs: remove it from the list")
        elif not ci_only[unit].strip():
            found.append(f"CI_ONLY gives no reason for `{unit}`")
    return found


def counts(workflows, ci_only):
    """How many scripts the workflows run, and how many of them only CI runs."""
    in_ci = {unit for document in workflows.values() for text in run_texts(document) for unit in units(text)}
    return len(in_ci), len(in_ci & set(ci_only))


def read_workflows(root):
    import yaml  # Only the repository check needs it; the tests pass parsed data.

    root = pathlib.Path(root)
    paths = sorted((root / WORKFLOWS).glob("*.yml")) + sorted((root / WORKFLOWS).glob("*.yaml"))
    if not paths:
        raise ValueError(f"{WORKFLOWS} holds no workflow")
    paths += sorted((root / ACTIONS).glob("*/action.yml"))
    workflows = {}
    for path in paths:
        name = path.relative_to(root).as_posix()
        try:
            workflows[name] = yaml.safe_load(path.read_text(encoding="utf-8"))
        except yaml.YAMLError as error:
            raise ValueError(f"{name} is not YAML: {' '.join(str(error).split())}") from error
    return workflows


def read_hooks(root):
    """Each hook of .githooks: a file git would run, by its name. A saved
    copy or a sample (pre-commit.orig, pre-commit.sample) runs nothing."""
    directory = pathlib.Path(root) / HOOKS
    if not directory.is_dir():
        return {}
    return {
        path.name: path.read_text(encoding="utf-8")
        for path in sorted(directory.iterdir())
        if path.is_file() and HOOK_NAME.fullmatch(path.name)
    }


def repository_problems(root, ci_only):
    return problems(read_workflows(root), read_hooks(root), ci_only)


def main(root, ci_only):
    try:
        workflows = read_workflows(root)
        found = problems(workflows, read_hooks(root), ci_only)
    except ValueError as error:
        print(f"check_hook_steps: {error}", file=sys.stderr)
        return 1
    for problem in found:
        print(f"check_hook_steps: {problem}", file=sys.stderr)
    if found:
        return 1
    total, only = counts(workflows, ci_only)
    print(f"check_hook_steps: ok ({total} scripts run in workflows: {total - only} in a hook, {only} only in CI)")
    return 0


if __name__ == "__main__":
    sys.exit(main(pathlib.Path(__file__).resolve().parent.parent, CI_ONLY))
