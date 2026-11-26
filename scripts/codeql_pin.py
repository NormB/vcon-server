#!/usr/bin/env python3
"""Holds the CodeQL workflow to one pinned CodeQL bundle, analyzed on the
whole tree.

    codeql_pin.py check     codeql.yml keeps the pin's shape (offline)
    codeql_pin.py behind    fails while a newer CodeQL bundle is released

On 2026-11-20 main's CodeQL push run on 27e541e used CodeQL CLI 2.27.5 where
the run on 24a3cba the day before used 2.27.4, with the codeql-action pin
unchanged (v4.39.1, whose own defaults.json names 2.27.3): the action picks
its CLI at run time from GitHub's feature flags (default_codeql_version_*).
2.27.5's rust/cleartext-logging opened 24 alerts on main in code no commit
had changed. The pull request before it had passed: its analysis was
diff-informed, so it reported results on changed lines only, and
scripts/codeql_pr_gate.py found nothing new.

THE RULE, which `check` holds .github/workflows/codeql.yml to:

- env.CODEQL_VERSION names the bundle, three numbers.
- CODEQL_ACTION_DIFF_INFORMED_QUERIES is "false" (workflow or job env), so
  a pull request's analysis reports the whole tree and codeql_pr_gate.py
  sees an alert a change of the pin, or of code, opens anywhere.
- The analyze job's matrix is a list of `include` entries, each a language,
  its bundle (BUNDLES) and the bundle's sha256 (64 lowercase hex digits).
- A step with id `bundle` fetches that bundle through
  ./.github/actions/pinned-download (the one rule for release assets,
  scripts/check_workflow_downloads.py), with the name, version, sha256, url
  and file of PINNED_DOWNLOAD, before the init step.
- Every codeql-action init step has `tools: ${{ steps.bundle.outputs.path }}`.

A new CodeQL version is then a pull request that changes CODEQL_VERSION and
the three sha256 values, and its own analysis, on the whole tree with the new
CLI, shows each alert the new version opens. `behind`, which the scheduled
run runs, fails while github/codeql-action has released a newer
codeql-bundle-vX.Y.Z, and prints the values to pin (from the release's asset
digests). The how-to is in website/content/docs/development.md, "The CodeQL
version".

Exit 0 when it passes, 1 when it refuses (each reason on stderr), 2 when the
releases cannot be read or the arguments are wrong.
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import re
import subprocess
import sys

# No bytecode beside the scripts for the module loaded by path
# (scripts/test_no_bytecode.py).
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
WORKFLOW = HERE.parent / ".github" / "workflows" / "codeql.yml"
RELEASES = "repos/github/codeql-action/releases?per_page=50"
GH_DEADLINE = 60
RATE_LIMIT_WAIT = 900

# Each analyzed language and the codeql-action release asset that holds it.
BUNDLES = {"actions": "actions", "javascript-typescript": "javascript", "rust": "rust"}
TOOLS = "${{ steps.bundle.outputs.path }}"
PINNED_DOWNLOAD = {
    "name": "codeql-bundle-${{ matrix.bundle }}",
    "version": "${{ env.CODEQL_VERSION }}",
    "sha256": "${{ matrix.bundle_sha256 }}",
    "url": (
        "https://github.com/github/codeql-action/releases/download/"
        "codeql-bundle-v${{ env.CODEQL_VERSION }}/codeql-bundle-${{ matrix.bundle }}-linux64.tar.zst"
    ),
    "file": "codeql-bundle-${{ matrix.bundle }}-linux64.tar.zst",
}
VERSION = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+")
SHA256 = re.compile(r"[0-9a-f]{64}")
BUNDLE_TAG = re.compile(r"codeql-bundle-v([0-9]+)\.([0-9]+)\.([0-9]+)")


class PinError(Exception):
    """The releases could not be read."""


def load_workflow(path):
    import yaml

    return yaml.safe_load(pathlib.Path(path).read_text(encoding="utf-8"))


def pinned_version(workflow):
    """env.CODEQL_VERSION when it is three numbers, else None."""
    env = workflow.get("env") if isinstance(workflow, dict) else None
    value = env.get("CODEQL_VERSION") if isinstance(env, dict) else None
    return value if isinstance(value, str) and VERSION.fullmatch(value) else None


def _is_init(step):
    return isinstance(step, dict) and str(step.get("uses", "")).startswith("github/codeql-action/init@")


def problems(workflow):
    """Each way codeql.yml breaks THE RULE; empty when it keeps it."""
    if not isinstance(workflow, dict):
        return ["codeql.yml: the file holds no workflow"]
    job = (workflow.get("jobs") or {}).get("analyze")
    if not isinstance(job, dict):
        return ["codeql.yml: no `analyze` job"]
    found = []
    if pinned_version(workflow) is None:
        found.append("codeql.yml: env.CODEQL_VERSION is not a version of three numbers (e.g. 2.27.5)")
    diff_informed = [
        str(env.get("CODEQL_ACTION_DIFF_INFORMED_QUERIES")).lower()
        for env in (workflow.get("env"), job.get("env"))
        if isinstance(env, dict) and "CODEQL_ACTION_DIFF_INFORMED_QUERIES" in env
    ]
    if "false" not in diff_informed:
        found.append(
            'codeql.yml: CODEQL_ACTION_DIFF_INFORMED_QUERIES is not "false", so a pull request\'s '
            "analysis reports changed lines only"
        )
    matrix = (job.get("strategy") or {}).get("matrix")
    include = matrix.get("include") if isinstance(matrix, dict) else None
    if not isinstance(include, list) or not include:
        found.append("codeql.yml: the analyze matrix is not a list of `include` entries")
        include = []
    for entry in include:
        language = entry.get("language") if isinstance(entry, dict) else None
        if language not in BUNDLES or entry.get("bundle") != BUNDLES[language]:
            found.append(f"codeql.yml: matrix entry {language!r} does not name its bundle {BUNDLES.get(language)!r}")
        sha = entry.get("bundle_sha256") if isinstance(entry, dict) else None
        if not isinstance(sha, str) or not SHA256.fullmatch(sha):
            found.append(f"codeql.yml: matrix entry {language!r} has no bundle_sha256 of 64 lowercase hex digits")
    steps = job.get("steps") if isinstance(job.get("steps"), list) else []
    bundle_at = next((i for i, s in enumerate(steps) if isinstance(s, dict) and s.get("id") == "bundle"), None)
    if bundle_at is None:
        found.append("codeql.yml: no step with id `bundle` fetches the CodeQL bundle")
    else:
        bundle = steps[bundle_at]
        if bundle.get("uses") != "./.github/actions/pinned-download":
            found.append("codeql.yml: the bundle step does not use ./.github/actions/pinned-download")
        given = bundle.get("with") or {}
        for key, value in PINNED_DOWNLOAD.items():
            if given.get(key) != value:
                found.append(f"codeql.yml: the bundle step's {key} is not {value}")
    inits = [i for i, s in enumerate(steps) if _is_init(s)]
    if not inits:
        found.append("codeql.yml: no codeql-action init step")
    for i in inits:
        if (steps[i].get("with") or {}).get("tools") != TOOLS:
            found.append(f"codeql.yml: an init step's tools is not {TOOLS}")
        if bundle_at is not None and bundle_at > i:
            found.append("codeql.yml: the bundle step must come before the init step")
    return found


def _version(tag):
    match = BUNDLE_TAG.fullmatch(tag or "")
    return tuple(int(n) for n in match.groups()) if match else None


def newest_bundle(releases):
    """The release of the highest codeql-bundle-vX.Y.Z that is neither a
    draft nor a prerelease, or None."""
    best = None
    for rel in releases:
        if not isinstance(rel, dict) or rel.get("draft") or rel.get("prerelease"):
            continue
        version = _version(rel.get("tag_name"))
        if version is not None and (best is None or version > best[0]):
            best = (version, rel)
    return best[1] if best else None


def _digests(rel):
    """Each language's sha256 from the release's asset digests (None when
    GitHub gave none)."""
    by_name = {a.get("name"): a.get("digest") for a in rel.get("assets") or [] if isinstance(a, dict)}
    found = {}
    for bundle in sorted(set(BUNDLES.values())):
        digest = by_name.get(f"codeql-bundle-{bundle}-linux64.tar.zst")
        found[bundle] = digest[len("sha256:"):] if isinstance(digest, str) and digest.startswith("sha256:") else None
    return found


def gh_api(path, run=subprocess.run, err=sys.stderr):
    """`gh api path` as JSON; a spent budget is waited out by
    scripts/gh_rate_limit.py's rule."""
    spec = importlib.util.spec_from_file_location("gh_rate_limit", HERE / "gh_rate_limit.py")
    gh_rate_limit = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gh_rate_limit)
    try:
        done = gh_rate_limit.run_gh(
            ["gh", "api", path], run=run, max_wait=RATE_LIMIT_WAIT, err=err,
            capture_output=True, text=True, timeout=GH_DEADLINE,
        )
    except subprocess.TimeoutExpired as error:
        raise PinError(f"gh api {path} did not answer in {GH_DEADLINE} s") from error
    if done.returncode != 0:
        raise PinError(done.stderr.strip() or f"gh api {path} exited {done.returncode}")
    try:
        return json.loads(done.stdout)
    except json.JSONDecodeError as error:
        raise PinError(f"gh api {path} answered no JSON") from error


def main(argv, read_workflow, api, out, err):
    if len(argv) != 1 or argv[0] not in ("check", "behind"):
        print("usage: codeql_pin.py check | behind", file=err)
        return 2
    workflow = read_workflow()
    found = problems(workflow)
    if found:
        for line in found:
            print(line, file=err)
        return 1
    pinned = pinned_version(workflow)
    if argv[0] == "check":
        print(f"codeql.yml analyzes with the pinned CodeQL {pinned}, on the whole tree", file=out)
        return 0
    try:
        releases = api(RELEASES)
    except PinError as error:
        print(f"the CodeQL releases cannot be read: {error}", file=err)
        return 2
    if not isinstance(releases, list):
        print("the CodeQL releases cannot be read: the answer is not a list", file=err)
        return 2
    newest = newest_bundle(releases)
    if newest is None:
        print("github/codeql-action lists no codeql-bundle-vX.Y.Z release", file=err)
        return 2
    version = _version(newest["tag_name"])
    if version <= _version(f"codeql-bundle-v{pinned}"):
        print(f"CodeQL {pinned} is the newest bundle released", file=out)
        return 0
    text = ".".join(str(n) for n in version)
    print(f"CodeQL {text} is released; codeql.yml pins {pinned}. Pin, in a pull request:", file=err)
    print(f"  CODEQL_VERSION: {text}", file=err)
    for bundle, sha in _digests(newest).items():
        print(f"  {bundle}: {sha}" if sha else f"  {bundle}: no sha256 in the release; read it from the asset", file=err)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:], lambda: load_workflow(WORKFLOW), gh_api, sys.stdout, sys.stderr))
