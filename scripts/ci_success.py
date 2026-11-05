"""The rule of the "CI success" job of .github/workflows/ci.yml.

"CI success" is the one check of ci.yml that the main branch's ruleset has
to name. It needs every other job of the workflow and passes only when each
of them succeeded, so a job whose check name changes with its matrix (the
how-to scenarios, the fuzz targets) or that starts only after another job
(they wait for docs-binary and fuzz-targets) still gates the merge.

  verdict  reads toJSON(needs) from NEEDS and exits 1 naming each job that
           did not succeed. Only "success" passes. A skipped job fails too:
           no job of ci.yml has a condition of its own (`check` refuses
           one), so a job is skipped only when a job it needs did not
           succeed, or when the run was cancelled.
  check    reads ci.yml and exits 1 naming each way the "CI success" job
           would not gate every job: one missing from its needs, no
           if: always() (without it a failed job skips "CI success", and a
           skipped check passes the ruleset), another name, or no verdict
           step given the results.

Found on 2026-11-03: "how-to foreground" failed on main after #100 merged,
because no ruleset named it.

Run with: python3 -I scripts/ci_success.py verdict|check
(check from the repository root; needs PyYAML).
"""

import json
import os
import pathlib
import re
import sys

JOB = "ci-success"
NAME = "CI success"
VERDICT = "scripts/ci_success.py verdict"
RESULTS = "${{ toJSON(needs) }}"
ALWAYS = re.compile(r"^(\$\{\{\s*always\(\)\s*\}\}|always\(\))$")


def failures(needs):
    """One line per job in `needs` (toJSON(needs), parsed) whose result is
    not "success", in job order; a line also when there is no job."""
    if not needs:
        return ["no job results: the job needs nothing"]
    return [f"{job}: {(needs[job] or {}).get('result') or 'no result'}" for job in sorted(needs) if (needs[job] or {}).get("result") != "success"]


def as_list(value):
    if value is None:
        return []
    return [value] if isinstance(value, str) else list(value)


def workflow_problems(workflow):
    """One line per way ci.yml's `ci-success` job fails to gate every other
    job of `workflow` (ci.yml, parsed)."""
    jobs = (workflow or {}).get("jobs") or {}
    if JOB not in jobs:
        return [f"ci.yml has no {JOB} job"]
    job = jobs[JOB] or {}
    problems = []
    if job.get("name") != NAME:
        problems.append(f"{JOB} must be named '{NAME}', not '{job.get('name')}'")
    condition = str(job.get("if", "")).strip()
    if not ALWAYS.match(condition):
        problems.append(f"{JOB} must run with if: always(), not {condition or '(none)'}")
    others = set(jobs) - {JOB}
    needed = set(as_list(job.get("needs")))
    if others - needed:
        problems.append(f"{JOB} does not need: {', '.join(sorted(others - needed))}")
    if needed - others:
        problems.append(f"{JOB} needs jobs ci.yml lacks: {', '.join(sorted(needed - others))}")
    if not any(VERDICT in str(step.get("run", "")) and (step.get("env") or {}).get("NEEDS") == RESULTS for step in job.get("steps") or []):
        problems.append(f"{JOB} has no step running '{VERDICT}' with NEEDS: {RESULTS}")
    for name in sorted(others):
        if "if" in (jobs[name] or {}):
            problems.append(f"{name} has a condition of its own (if: {jobs[name]['if']}); the verdict fails a skipped job")
    return problems


def main(argv):
    if argv[1:] == ["verdict"]:
        problems = failures(json.loads(os.environ["NEEDS"]))
        ok = "every job succeeded"
    elif argv[1:] == ["check"]:
        import yaml  # Only this command needs it; the tests pass parsed data.

        problems = workflow_problems(yaml.safe_load(pathlib.Path(".github/workflows/ci.yml").read_text(encoding="utf-8")))
        ok = f"{JOB} needs every job of ci.yml"
    else:
        print("usage: ci_success.py verdict|check", file=sys.stderr)
        return 2
    for problem in problems:
        print(f"ci_success: {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"ci_success: ok ({ok})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
