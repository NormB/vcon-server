"""Checks that every test run of the commit hook and of the workflows has a
per-test deadline.

cargo test has none: on 2026-11-06 a deadlock in a test hung a run for nine
minutes, and in the commit hook it would have hung with no end. So the
tests run under cargo-nextest, which ends a test still running at the
deadline in .config/nextest.toml and fails it by name. The rule:

  .config/nextest.toml   [profile.default] slow-timeout is
                         { period = "60s", terminate-after = 3 }
  cargo [+tc] test       only with --doc (nextest does not run doc tests),
                         or as `cargo miri test` (Miri runs the tests in its
                         interpreter, which nextest cannot drive)
  cargo nextest          only through scripts/nextest.sh, which pins the
                         version and passes the config; a job that runs it
                         installs it first with `scripts/nextest.sh install`
                         or with the .github/actions/nextest step, which
                         runs that install after fetching the archive
                         through .github/actions/pinned-download
  cargo llvm-cov         only as `llvm-cov nextest` (or `report`), after
                         `scripts/nextest.sh check` in the same job, since
                         llvm-cov starts cargo nextest itself
  the commit hook and    run the workspace with `scripts/nextest.sh run
  ci.yml's rust job      --workspace` and the doc tests with `cargo test
                         --workspace --doc`

Run with: python3 -I scripts/check_test_runner.py
(from anywhere in the repository; needs PyYAML). Exits 1 naming each place
that breaks the rule.
"""

import pathlib
import re
import sys
import tomllib

DEADLINE = {"period": "60s", "terminate-after": 3}
# cargo, an optional +toolchain (quoted or not), then the subcommand.
CARGO = re.compile(r"""\bcargo\s+(?:["']?\+\S+?["']?\s+)?([a-z][a-z-]*)""")
SCRIPT = re.compile(r"\bscripts/nextest\.sh\s+(install|check|run|list)\b")
LLVM_COV_ALLOWED = {"nextest", "report"}
NEXTEST_ACTION = "./.github/actions/nextest"
# The install as the action runs it: the script's path may be quoted.
ACTION_INSTALL = re.compile(r"\bscripts/nextest\.sh[\"']?\s+install\b")


def nextest_action_problems(text):
    """.github/actions/nextest installs nextest with scripts/nextest.sh."""
    if ACTION_INSTALL.search("\n".join(l for l in text.splitlines() if not l.strip().startswith("#"))):
        return []
    return [".github/actions/nextest/action.yml: no step runs `scripts/nextest.sh install`"]


def commands(text):
    """Each logical line of a shell script (continuations joined), without
    comment lines."""
    out, buf = [], ""
    for line in (text or "").splitlines():
        if line.strip().startswith("#") and not buf:
            continue
        if line.rstrip().endswith("\\"):
            buf += line.rstrip()[:-1] + " "
            continue
        out.append(buf + line)
        buf = ""
    if buf:
        out.append(buf)
    return out


def command_problems(where, text):
    """(problems, uses nextest, uses llvm-cov nextest) for one script."""
    found, nextest, cov = [], False, False
    for line in commands(text):
        for m in CARGO.finditer(line):
            sub, rest = m.group(1), line[m.end():]
            if sub == "test" and not re.search(r"(^|\s)--doc(\s|$)", rest):
                found.append(f"{where}: `cargo test` has no per-test deadline; run it with scripts/nextest.sh run: {line.strip()}")
            elif sub == "nextest":
                found.append(f"{where}: cargo nextest called directly; use scripts/nextest.sh (pinned version, the repository's config): {line.strip()}")
            elif sub == "llvm-cov":
                word = rest.split()[0] if rest.split() else ""
                if word not in LLVM_COV_ALLOWED:
                    found.append(f"{where}: `cargo llvm-cov` runs cargo test; use `cargo llvm-cov nextest`: {line.strip()}")
                elif word == "nextest":
                    cov = True
        if SCRIPT.search(line) and SCRIPT.search(line).group(1) in ("run", "list", "check"):
            nextest = True
    return found, nextest, cov


def runs_workspace_and_doc_tests(where, text):
    found = []
    lines = commands(text)
    if not any(re.search(r"\bscripts/nextest\.sh\s+run\b.*--workspace", line) for line in lines):
        found.append(f"{where}: does not run the workspace's tests with `scripts/nextest.sh run --workspace`")
    if not any(re.search(r"\bcargo\s+test\b.*--workspace", line) and re.search(r"(^|\s)--doc(\s|$)", line) for line in lines):
        found.append(f"{where}: does not run the doc tests with `cargo test --workspace --doc`")
    return found


def problems(config_text, hook_text, workflows):
    """One line per break of the rule. `workflows` maps a file name to its
    parsed YAML."""
    found = []
    try:
        config = tomllib.loads(config_text)
    except tomllib.TOMLDecodeError as e:
        config = {}
        found.append(f".config/nextest.toml is not TOML: {e}")
    slow = ((config.get("profile") or {}).get("default") or {}).get("slow-timeout")
    if slow != DEADLINE:
        found.append(f".config/nextest.toml: [profile.default] slow-timeout must be {DEADLINE}, not {slow}")

    hook_found, _, _ = command_problems(".githooks/pre-commit", hook_text)
    found += hook_found + runs_workspace_and_doc_tests(".githooks/pre-commit", hook_text)

    for file, workflow in sorted(workflows.items()):
        for job_id, job in ((workflow or {}).get("jobs") or {}).items():
            where = f"{file} job {job_id}"
            installed = checked = False
            for step in (job or {}).get("steps") or []:
                if str(step.get("uses", "")) == NEXTEST_ACTION:
                    installed = True
                # Line by line, so an install or a check counts only for
                # what runs after it.
                for line in commands(str(step.get("run", ""))):
                    line_found, nextest, cov = command_problems(where, line)
                    found += line_found
                    if (nextest or cov) and not installed:
                        found.append(f"{where}: runs nextest before any step runs `scripts/nextest.sh install`")
                    if cov and not checked:
                        found.append(f"{where}: runs `cargo llvm-cov nextest` without `scripts/nextest.sh check` before it in the job")
                    for m in SCRIPT.finditer(line):
                        installed |= m.group(1) == "install"
                        checked |= m.group(1) in ("check", "run", "list")
            if file == "ci.yml" and job_id == "rust":
                found += runs_workspace_and_doc_tests(where, "\n".join(str(s.get("run", "")) for s in job.get("steps") or []))
    return found


def repository_problems(root):
    import yaml  # Only the repository check needs it; the tests pass parsed data.

    root = pathlib.Path(root)
    workflows = {p.name: yaml.safe_load(p.read_text(encoding="utf-8")) for p in sorted((root / ".github/workflows").glob("*.yml"))}
    config = root / ".config/nextest.toml"
    found = problems(
        config.read_text(encoding="utf-8") if config.exists() else "",
        (root / ".githooks/pre-commit").read_text(encoding="utf-8"),
        workflows,
    )
    action = root / ".github/actions/nextest/action.yml"
    uses = any(str(s.get("uses", "")) == NEXTEST_ACTION for w in workflows.values() for j in ((w or {}).get("jobs") or {}).values() for s in (j or {}).get("steps") or [])
    if uses:
        found += nextest_action_problems(action.read_text(encoding="utf-8")) if action.is_file() else [".github/actions/nextest/action.yml is missing, and a job uses it"]
    return found


def main():
    root = pathlib.Path(__file__).resolve().parent.parent
    found = repository_problems(root)
    for problem in found:
        print(f"check_test_runner: {problem}", file=sys.stderr)
    if found:
        return 1
    print("check_test_runner: ok (every test run has the per-test deadline)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
