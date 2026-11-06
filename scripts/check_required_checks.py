"""Checks the main branch's required checks against the workflows, both ways.

1. Every check the ruleset requires is reported on every pull request: a
   job of a workflow that runs on every pull request to main, under that
   name. A required check that does not report blocks the merge, so a job
   of a workflow filtered by path (or by branch) cannot be required, and
   neither can a matrix whose names are built at run time.
2. Every job that runs on every pull request is required by name, or is
   needed (directly or through other jobs) by a required job of the same
   workflow, as ci.yml's "CI success" needs every job of ci.yml. A job that
   runs on every PR and gates nothing lets a red result merge: on
   2026-10-28 "git hooks" was red on a merged PR, and on 2026-11-06 the
   CodeQL and OSV jobs ran on every PR without being required.

Contexts that no workflow reports (the CLA assistant's "license/cla") are
named in EXTERNAL. "CodeQL" is code scanning's own check-run (app
github-advanced-security), not a job: it reports on a pull request once a
CodeQL analysis of it is uploaded, and fails on a new alert in the code the
pull request changes, while the "Analyze (...)" jobs pass whatever they
find. It counts as reported when a workflow that runs on every pull request
uploads a CodeQL analysis, and it must then be required: on 2026-11-06 the
rust analysis of #131 found 6 new alerts and "Analyze (rust)" passed.

Run with: python3 -I scripts/check_required_checks.py RULES.json
where RULES.json is `gh api repos/<owner>/<repo>/rules/branches/main` (or
one ruleset, `gh api repos/<owner>/<repo>/rulesets/<id>`), from anywhere in
the repository; needs PyYAML. Exits 1 naming each problem.
"""

import itertools
import json
import pathlib
import re
import sys

EXTERNAL = {"license/cla"}
CODE_SCANNING = "CodeQL"
ANALYZE = "github/codeql-action/analyze@"
NO_UPLOAD = {"never", "false"}
MATRIX = re.compile(r"\$\{\{\s*matrix\.([A-Za-z0-9_-]+)\s*\}\}")
FILTERS = ("paths", "paths-ignore", "branches-ignore")


def triggers(workflow):
    on = workflow.get(True, workflow.get("on"))
    if isinstance(on, str):
        return {on: None}
    if isinstance(on, list):
        return {t: None for t in on}
    return on or {}


def on_every_pr(workflow):
    """Whether the workflow runs on every pull request to main."""
    on = triggers(workflow)
    if "pull_request" not in on:
        return False
    pr = on["pull_request"] or {}
    if any(f in pr for f in FILTERS):
        return False
    branches = pr.get("branches")
    return branches is None or "main" in branches or "**" in branches


def names(job_id, job):
    """The check names a job reports, or None when its matrix is built at
    run time."""
    name = str(job.get("name", job_id))
    keys = MATRIX.findall(name)
    if not keys:
        return [name]
    matrix = (job.get("strategy") or {}).get("matrix") or {}
    values = []
    for key in keys:
        v = matrix.get(key)
        if not isinstance(v, list):
            return None
        values.append([str(x) for x in v])
    out = []
    for combo in itertools.product(*values):
        n = name
        for key, value in zip(keys, combo):
            n = re.sub(r"\$\{\{\s*matrix\." + re.escape(key) + r"\s*\}\}", value, n)
        out.append(n)
    return out


def as_list(v):
    if v is None:
        return []
    return [v] if isinstance(v, str) else list(v)


def uploads_codeql(workflow):
    """Whether a job of the workflow uploads a CodeQL analysis: a
    github/codeql-action/analyze step whose `upload` is not turned off."""
    for job in ((workflow or {}).get("jobs") or {}).values():
        for step in (job or {}).get("steps") or []:
            if not str((step or {}).get("uses", "")).startswith(ANALYZE):
                continue
            upload = ((step or {}).get("with") or {}).get("upload", "always")
            if str(upload).lower() not in NO_UPLOAD:
                return True
    return False


def every_pr_jobs(workflows):
    """(file, job id, names or None) for each job of a workflow that runs
    on every pull request."""
    out = []
    for file, wf in sorted(workflows.items()):
        if on_every_pr(wf or {}):
            for job_id, job in ((wf or {}).get("jobs") or {}).items():
                out.append((file, job_id, names(job_id, job or {})))
    return out


def problems(workflows, required):
    found = []
    every = every_pr_jobs(workflows)
    reported = {n for _, _, ns in every for n in (ns or [])}
    elsewhere = set()
    for file, wf in workflows.items():
        if not on_every_pr(wf or {}):
            for job_id, job in ((wf or {}).get("jobs") or {}).items():
                elsewhere.update(names(job_id, job or {}) or [])
    scanned = any(on_every_pr(wf or {}) and uploads_codeql(wf) for wf in workflows.values())
    for context in required:
        if context in EXTERNAL or context in reported:
            continue
        if context == CODE_SCANNING:
            if not scanned:
                found.append(f"the ruleset requires '{context}', which code scanning reports only on a pull request whose CodeQL analysis is uploaded, and no workflow that runs on every pull request uploads one")
            continue
        if context in elsewhere:
            found.append(f"the ruleset requires '{context}', whose workflow does not run on every pull request (it is not on every pull request, so it never reports on some and blocks their merge)")
        else:
            found.append(f"the ruleset requires '{context}', which no workflow job reports")

    if scanned and CODE_SCANNING not in required:
        found.append(f"a workflow that runs on every pull request uploads a CodeQL analysis, but the ruleset does not require '{CODE_SCANNING}', the check that fails on a new alert")

    required_set = set(required)
    for file, wf in sorted(workflows.items()):
        if not on_every_pr(wf or {}):
            continue
        jobs = (wf or {}).get("jobs") or {}
        gated = set()
        todo = [j for j in jobs if set(names(j, jobs[j] or {}) or []) & required_set]
        while todo:
            j = todo.pop()
            for n in as_list((jobs.get(j) or {}).get("needs")):
                if n not in gated:
                    gated.add(n)
                    todo.append(n)
        for job_id, job in jobs.items():
            ns = names(job_id, job or {})
            if job_id in gated:
                continue
            for n in ns or [str((job or {}).get("name", job_id))]:
                if n not in required_set:
                    found.append(f"{file}: '{n}' runs on every pull request, but no required check needs it")
    return found


def required_contexts(rules):
    if isinstance(rules, dict):
        rules = rules.get("rules") or []
    out = []
    seen = False
    for rule in rules:
        if rule.get("type") == "required_status_checks":
            seen = True
            out += [c["context"] for c in (rule.get("parameters") or {}).get("required_status_checks") or []]
    if not seen:
        raise ValueError("the rules hold no required_status_checks rule")
    return out


def read_workflows(root):
    import yaml  # Only reading the files needs it; the tests pass parsed data.

    return {p.name: yaml.safe_load(p.read_text(encoding="utf-8")) for p in sorted(pathlib.Path(root, ".github/workflows").glob("*.yml"))}


def main(argv):
    if len(argv) != 2:
        print("usage: check_required_checks.py RULES.json", file=sys.stderr)
        return 2
    try:
        required = required_contexts(json.loads(pathlib.Path(argv[1]).read_text()))
    except (ValueError, KeyError, AttributeError) as e:
        print(f"check_required_checks: {argv[1]}: {e}", file=sys.stderr)
        return 1
    found = problems(read_workflows(pathlib.Path(__file__).resolve().parent.parent), required)
    for problem in found:
        print(f"check_required_checks: {problem}", file=sys.stderr)
    if found:
        return 1
    print(f"check_required_checks: ok ({len(required)} required checks, each reported on every pull request)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
