#!/usr/bin/env python3
"""Which release the website may name, and whether a commit may be deployed.

    advertised_release.py decide --repo OWNER/NAME [--expect X.Y.Z] [--github-output FILE] [--root DIR]
    advertised_release.py live --url URL --expect X.Y.Z [--wait SECONDS]
    advertised_release.py daily --repo OWNER/NAME --url URL [--root DIR]

The pages of the site name the version of the tree they are built from
(tests hold each page to Cargo.toml). A release commit changes that version
before its tag exists, and the site was deployed when the commit landed: at
the 0.4.6 release it said "0.4.6 is released", with the 0.4.6 download
commands, for 22 minutes 9 seconds before there was such a release.

THE RULE (`newest_to_advertise`, written here and nowhere else). The site
may name the numerically newest GitHub release that

* is tagged vX.Y.Z, with X.Y.Z not newer than the version of the tree;
* is neither a draft nor a prerelease (marking a release as a prerelease
  withdraws it from the site);
* has each of the six files of a vcon-server release (`ASSETS`), every
  one fully uploaded.

`decide` reads the releases and the tree's version and prints one line,
`verdict=deploy` or `verdict=held`, then `tree=` and `advertised=`. A tree
whose version is that release is deployed. A tree whose version is newer is
held: the site is not deployed and keeps what it shows, which is the
release before, whole. pages.yml runs this before it builds. With --expect
(the release workflow gives the version it has just published), anything
but a deploy of that version is exit code 1.

`live` reads /advertised-release.txt, the static file pages.yml writes
beside the pages, and exits 0 only when it holds the expected version. A
challenge from the network in front of the site, a page of HTML, an error
and no answer are all "not verified", exit code 1: never a pass.

`daily` is the check a scheduled workflow runs: the live file must name
the newest complete release, and a deploy must not have been held for
longer than HELD_BOUND_HOURS. Exit 0, or 1 with one line for each problem.

Needs `gh` with a token that can read the repository's releases.
"""

from __future__ import annotations

import argparse
import calendar
import json
import pathlib
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

# The files of a release, with the version written as <v>
# (release.yml: the two archives, the SBOM, the checksums and their
# signature, the provenance bundle).
ASSETS = (
    "SHA256SUMS.txt",
    "SHA256SUMS.txt.sigstore.json",
    "vcon-server-<v>-aarch64-unknown-linux-gnu.tar.gz",
    "vcon-server-<v>-x86_64-unknown-linux-gnu.tar.gz",
    "vcon-server-<v>.cdx.json",
    "vcon-server-<v>.intoto.jsonl",
)
# How long a deploy may be held before the daily check fails. The 0.4.6
# release took 108 minutes from its commit landing on main to the
# published release (main's CI, then the release workflow, on GitHub's
# runners). Six hours is more than three times that, and less than the
# day between two checks, so a release that is under way never fails the
# check and one that was abandoned fails the first check after it.
HELD_BOUND_HOURS = 6
_VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
_TREE = re.compile(r'(?m)^version = "([^"]*)"$')
AGENT = "vcon-server site check (https://github.com/NormB/vcon-server)"


class Refused(Exception):
    """An input that cannot be judged; the message says why."""


def version_key(text):
    """(X, Y, Z) of `X.Y.Z`, or None for anything else."""
    match = _VERSION.fullmatch(text) if isinstance(text, str) else None
    return tuple(int(part) for part in match.groups()) if match else None


def incomplete(release) -> str:
    """Why the site may not name this release; empty when it may."""
    tag = release.get("tag_name")
    version = tag[1:] if isinstance(tag, str) and tag.startswith("v") else None
    if version_key(version) is None:
        return f"{tag!r} is not a version tag"
    if release.get("draft"):
        return f"{tag} is a draft"
    if release.get("prerelease"):
        return f"{tag} is a prerelease"
    states = {asset.get("name"): asset.get("state") for asset in release.get("assets") or []}
    for name in ASSETS:
        name = name.replace("<v>", version)
        if name not in states:
            return f"{tag} has no asset {name}"
        if states[name] != "uploaded":
            return f"{tag}: the asset {name} is not uploaded yet (state {states[name]})"
    return ""


def newest_to_advertise(releases, tree):
    """THE RULE: the version the site may name, or None when there is none."""
    limit = version_key(tree)
    if limit is None:
        raise Refused(f"the tree's version {tree!r} is not a version X.Y.Z")
    if not isinstance(releases, list) or not all(isinstance(r, dict) for r in releases):
        raise Refused("the answer to the release listing is not a list of releases")
    keys = [version_key(r["tag_name"][1:]) for r in releases if not incomplete(r)]
    keys = [key for key in keys if key <= limit]
    return ".".join(str(part) for part in max(keys)) if keys else None


def decide(releases, tree):
    """("deploy", version) when the tree's version is the release the site
    may name, else ("held", that release or None)."""
    advertised = newest_to_advertise(releases, tree)
    return ("deploy" if advertised == tree else "held", advertised)


def held_notice(tree, advertised) -> str:
    """The one line a held run shows on its Actions page."""
    head = f"Site deploy held: main is at {tree}, which has no published release; "
    if advertised is None:
        return head + "no release is published yet, so the site is not deployed."
    return head + f"the site keeps showing {advertised} until the release workflow publishes {tree} and deploys the site."


def outputs(verdict, tree, advertised) -> str:
    """The decision as step outputs, one `name=value` line each."""
    return f"verdict={verdict}\ntree={tree}\nadvertised={advertised or ''}\n"


def expectation(verdict, tree, advertised, expect) -> str:
    """Why a run that was told which version to deploy must fail; empty
    when it was told none, or deploys that one."""
    if not expect or (verdict == "deploy" and tree == expect):
        return ""
    return (
        f"expected to deploy the site for {expect}, but main is at {tree} and the newest complete release is "
        f"{advertised}: the site is not deployed"
    )


def live_verdict(status, headers, body, expect):
    """What one read of the live file proves: ("verified" | "differs" |
    "not verified", why). Only "verified" is a pass."""
    if status is None:
        return ("not verified", f"no answer: {body}")
    lowered = {str(name).lower(): str(value).lower() for name, value in dict(headers).items()}
    if lowered.get("cf-mitigated") == "challenge":
        return ("not verified", f"the site answered {status} with a challenge")
    if status != 200:
        return ("not verified", f"the site answered {status}")
    named = body.strip()
    if version_key(named) is None:
        return ("not verified", f"the file holds {named[:40]!r}, which is not a version")
    if named != expect:
        return ("differs", f"the site names {named}, not {expect}")
    return ("verified", f"the site names {named}")


def seconds(stamp):
    """A time as GitHub writes it (2026-11-14T22:32:39Z) in seconds since
    1970, or None."""
    try:
        return calendar.timegm(time.strptime(stamp, "%Y-%m-%dT%H:%M:%SZ"))
    except (TypeError, ValueError):
        return None


def daily_problems(verdict, tree, advertised, held_since, live, now, published):
    """The daily check's findings, one line each. `held_since` is when
    main's version became `tree`, `published` when the advertised release
    was published, `live` a `live_verdict`."""
    found = []
    bound = HELD_BOUND_HOURS * 3600
    if verdict == "held":
        remedy = (
            f"Remedy: finish the release (push the tag v{tree}, or re-run the failed release workflow), "
            f"or set the version in Cargo.toml back to {advertised}."
        )
        if held_since is None:
            found.append(
                f"main is at {tree} and release {tree} is not published, so the site is held at {advertised}; "
                f"since when is not known. {remedy}"
            )
        elif now - held_since > bound:
            hours = (now - held_since) // 3600
            found.append(
                f"main has been at {tree} for {hours} hours and release {tree} is not published, so the site is "
                f"held at {advertised} and no change to the pages is deployed (the bound is {HELD_BOUND_HOURS} "
                f"hours). {remedy}"
            )
    kind, why = live
    if kind == "not verified":
        found.append(f"the live site was not verified: {why}")
    elif kind == "differs" and not (published is not None and now - published <= bound):
        found.append(f"the live site does not name the newest complete release: {why}")
    return found


def tree_version(manifest: str) -> str:
    """The workspace's version: the first `version = "..."` line of the
    root Cargo.toml."""
    match = _TREE.search(manifest)
    if not match:
        raise Refused("Cargo.toml has no line `version = \"...\"`")
    return match.group(1)


# The edges: gh, git and the network. Everything above is judged on values.


def _releases(repo):
    out = subprocess.run(
        ["gh", "api", f"repos/{repo}/releases", "--paginate", "--slurp"],
        check=False, capture_output=True, text=True, timeout=120,
    )
    if out.returncode != 0:
        raise Refused(f"gh could not list the releases of {repo}: {out.stderr.strip()}")
    try:
        pages = json.loads(out.stdout)
    except ValueError as error:
        raise Refused(f"the release listing is not JSON: {error}") from error
    if not isinstance(pages, list) or not all(isinstance(page, list) for page in pages):
        raise Refused("the answer to the release listing is not a list of releases")
    return [release for page in pages for release in page]


def _tree(root):
    return tree_version((pathlib.Path(root) / "Cargo.toml").read_text(encoding="utf-8"))


def _held_since(root, tree):
    """When the root manifest's version line became this one: the time of
    the newest commit that changed that line."""
    out = subprocess.run(
        ["git", "-C", str(root), "log", "-1", "--format=%ct", "-G", f'^version = "{re.escape(tree)}"$', "--", "Cargo.toml"],
        check=False, capture_output=True, text=True, timeout=60,
    )
    text = out.stdout.strip()
    return int(text) if out.returncode == 0 and text.isdigit() else None


def _read(url):
    request = urllib.request.Request(url, headers={"User-Agent": AGENT, "Cache-Control": "no-cache"})
    try:
        with urllib.request.urlopen(request, timeout=30) as answer:
            return answer.status, dict(answer.headers), answer.read(4096).decode("utf-8", "replace")
    except urllib.error.HTTPError as error:
        return error.code, dict(error.headers), ""
    except (urllib.error.URLError, OSError) as error:
        return None, {}, str(error)


def _live(url, expect, wait):
    end = time.monotonic() + wait
    while True:
        verdict = live_verdict(*_read(url), expect)
        if verdict[0] == "verified" or time.monotonic() >= end:
            return verdict
        time.sleep(15)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Which release the website may name.")
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("decide", "daily"):
        one = sub.add_parser(name)
        one.add_argument("--repo", required=True)
        one.add_argument("--root", default=".")
    sub.choices["decide"].add_argument("--expect", default="")
    # The step's $GITHUB_OUTPUT: the decision is appended there too, so the
    # held notice on stdout stays a workflow command.
    sub.choices["decide"].add_argument("--github-output", default="")
    sub.choices["daily"].add_argument("--url", required=True)
    live = sub.add_parser("live")
    live.add_argument("--url", required=True)
    live.add_argument("--expect", required=True)
    live.add_argument("--wait", type=int, default=0)
    args = parser.parse_args(argv)
    try:
        if args.command == "live":
            verdict, why = _live(args.url, args.expect, args.wait)
            print(f"{verdict}: {why}")
            return 0 if verdict == "verified" else 1
        tree = _tree(args.root)
        releases = _releases(args.repo)
        verdict, advertised = decide(releases, tree)
        if args.command == "decide":
            print(outputs(verdict, tree, advertised), end="")
            if args.github_output:
                with open(args.github_output, "a", encoding="utf-8") as file:
                    file.write(outputs(verdict, tree, advertised))
            if verdict == "held":
                print(f"::notice title=Site deploy held::{held_notice(tree, advertised)}")
            problem = expectation(verdict, tree, advertised, args.expect)
            if problem:
                print(f"advertised_release: {problem}", file=sys.stderr)
                return 1
            return 0
        published = next((seconds(r.get("published_at")) for r in releases if r.get("tag_name") == f"v{advertised}"), None)
        live_now = live_verdict(*_read(args.url), advertised) if advertised else ("not verified", "no release is published")
        since = _held_since(args.root, tree) if verdict == "held" else None
        problems = daily_problems(verdict, tree, advertised, since, live_now, int(time.time()), published)
        for problem in problems:
            print(f"advertised_release: {problem}", file=sys.stderr)
        if not problems:
            print(f"advertised_release: ok ({verdict}; main is at {tree}; the site names {advertised})")
        return 1 if problems else 0
    except Refused as refusal:
        print(f"advertised_release: {refusal}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
