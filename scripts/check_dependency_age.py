"""Checks the exception to the 14-day rule for dependency versions.

CONTRIBUTING.md, "Dependency versions": a version is taken once it has
been public for MIN_AGE_DAYS days. A version that fixes a published
security advisory is taken sooner, and the change that takes it writes
one comment line in the Cargo.toml that requires the crate (the workspace
Cargo.toml for a crate only a lockfile names):

  # dependency-age-exception: <crate> <fixed version> <advisory id> published <YYYY-MM-DD>

The advisory id is a RUSTSEC, GHSA or CVE id; the date is the UTC day
crates.io published the fixed version. Before this check the exception
was prose in the changelog (rmcp 3.9.0 on 2026-11-09, rusqlite 0.42.0 on
2026-11-12), and nothing refused a young version taken with no advisory.

Without --registry the check reads files only, so the commit hook runs it
with no network (with --walk, on its copy of the index, which is not a
repository):

  - every line that names the marker has the form above;
  - a crate and version are excepted once;
  - a Cargo.lock holds the excepted crate, at the fixed version or later.

A lockfile does not record when a version was published, so the age of a
version is known only from the registry. With `--registry` the check
reads crates.io's index (index.crates.io, one file per crate, each
version with its `pubtime`) and also refuses:

  - an exception whose recorded date is not the registry's;
  - with `--base <rev>`: a crates.io version that a Cargo.lock holds and
    the same Cargo.lock at <rev> did not, published under MIN_AGE_DAYS
    days ago, that no exception line names.

CI runs the registry mode against the pull request's base, or the commit
before a push (ci.yml, job `scan`). The check covers crates; npm and pip
packages, actions and images are held by Dependabot's cooldown
(scripts/check_dependabot.py) and by review.

Run with: python3 -I scripts/check_dependency_age.py [--walk] [--registry [--base <rev>]]
(from the repository root). Exits 1 naming each problem.
"""

import argparse
import collections
import datetime
import json
import os
import pathlib
import posixpath
import re
import subprocess
import sys
import tomllib
import urllib.error
import urllib.request

# The same number scripts/check_dependabot.py holds Dependabot to; a test
# compares the two.
MIN_AGE_DAYS = 14

FORM = "# dependency-age-exception: <crate> <fixed version> <advisory id> published <YYYY-MM-DD>"
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
INDEX_URL = "https://index.crates.io/"

# Any spelling of the marker: a line that holds one must have the form, so
# a typing error is refused and not read as an ordinary comment.
MENTION = re.compile(r"dependency[-_ ]?age[-_ ]?exception", re.IGNORECASE)
VERSION = r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?"
ADVISORY = r"RUSTSEC-\d{4}-\d{4}|GHSA(?:-[23456789cfghjmpqrvwx]{4}){3}|CVE-\d{4}-\d{4,}"
LINE = re.compile(
    rf"# dependency-age-exception: (?P<package>[A-Za-z0-9_-]+) (?P<version>{VERSION}) (?P<advisory>{ADVISORY}) published (?P<published>\d{{4}}-\d{{2}}-\d{{2}})"
)
NAME = re.compile(r"[A-Za-z0-9_-]+")
# Directories --walk does not enter.
WALK_SKIPS = {".git", "target", "node_modules"}

Exception_ = collections.namedtuple("Exception_", "package version advisory published path line")

# `published` for a version the index text does not hold; None is a
# version it holds with no publish time.
ABSENT = object()


class NotInRegistry(Exception):
    """The registry holds no package of this name."""


def exceptions_in(text, path):
    """The exception lines of one manifest, and the lines that name the
    marker without the form."""
    found, problems = [], []
    for number, raw in enumerate(text.splitlines(), start=1):
        if not MENTION.search(raw):
            continue
        match = LINE.fullmatch(raw.strip())
        day = None
        if match:
            try:
                day = datetime.date.fromisoformat(match["published"])
            except ValueError:
                day = None
        if day is None:
            problems.append(f"{path}:{number}: not of the form `{FORM}` (the id a RUSTSEC, GHSA or CVE id, the date a day of the calendar)")
            continue
        found.append(Exception_(match["package"], match["version"], match["advisory"], match["published"], path, number))
    return found, problems


def locked(lock_text):
    """The (name, version) of every crates.io package in a Cargo.lock."""
    packages = tomllib.loads(lock_text).get("package", [])
    return {(p["name"], p["version"]) for p in packages if p.get("source") == CRATES_IO}


def version_key(version):
    """Orders versions by their three numbers; a prerelease sorts before
    its release, and build metadata does not count."""
    core, _, _ = version.partition("+")
    numbers, dash, pre = core.partition("-")
    major, minor, patch = (int(n) for n in numbers.split("."))
    return (major, minor, patch, 0 if dash else 1, pre)


def offline_problems(exceptions, locks):
    """The rules that need no registry. `locks` maps a lockfile's path to
    its text."""
    held = set()
    for text in locks.values():
        held |= locked(text)
    problems, seen = [], {}
    for e in exceptions:
        where = f"{e.path}:{e.line}"
        first = seen.setdefault((e.package, e.version), where)
        if first != where:
            problems.append(f"{where}: {e.package} {e.version} already has an exception at {first}")
            continue
        versions = [v for name, v in held if name == e.package]
        if not versions:
            problems.append(f"{where}: no Cargo.lock holds a crates.io package named {e.package}")
        elif not any(version_key(v) >= version_key(e.version) for v in versions):
            problems.append(f"{where}: {e.package} {e.version} is named as the fixed version, and no Cargo.lock holds {e.package} {e.version} or later")
    return problems


def new_versions(base_locks, head_locks):
    """Every crates.io (name, version) a lockfile holds that the same
    lockfile at the base did not, sorted, each once."""
    new = set()
    for path, text in head_locks.items():
        before = locked(base_locks[path]) if base_locks.get(path) is not None else set()
        new |= locked(text) - before
    return sorted(new)


def index_path(name):
    """Where crates.io's index keeps a crate's file."""
    if not NAME.fullmatch(name):
        raise ValueError(f"not a crate name: {name!r}")
    lower = name.lower()
    if len(lower) <= 2:
        return f"{len(lower)}/{lower}"
    if len(lower) == 3:
        return f"3/{lower[0]}/{lower}"
    return f"{lower[:2]}/{lower[2:4]}/{lower}"


def published(index_text, version):
    """When the index says a version was published, in UTC: a datetime,
    None when the entry has no `pubtime`, ABSENT when there is no entry."""
    for raw in index_text.splitlines():
        if not raw.strip():
            continue
        entry = json.loads(raw)
        if entry.get("vers") != version:
            continue
        stamp = entry.get("pubtime")
        if stamp is None:
            return None
        return datetime.datetime.fromisoformat(stamp).astimezone(datetime.timezone.utc)
    return ABSENT


def verdict(name, version, when, now, exceptions):
    """None when the version may be taken, else the problem."""
    if now - when >= datetime.timedelta(days=MIN_AGE_DAYS):
        return None
    others = []
    for e in exceptions:
        if e.package != name:
            continue
        if e.version == version:
            return None
        others.append(f"the exception at {e.path}:{e.line} names {e.version}")
    days = max((now - when).days, 0)
    ago = "1 day ago" if days == 1 else f"{days} days ago"
    reason = "; ".join(others) if others else "no dependency-age-exception line names it with an advisory"
    return f"{name} {version} was published on {when.date().isoformat()}, {ago}, under the {MIN_AGE_DAYS} days CONTRIBUTING.md asks; {reason}"


def registry_problems(new, exceptions, fetch, now):
    """The rules that need the registry. `fetch(name)` returns the text of
    a crate's index file, or raises NotInRegistry, or OSError when the
    registry does not answer."""
    texts = {}

    def text_of(name):
        if name not in texts:
            try:
                texts[name] = fetch(name)
            except (NotInRegistry, OSError) as error:
                texts[name] = error
        return texts[name]

    problems = []
    for e in exceptions:
        where = f"{e.path}:{e.line}"
        text = text_of(e.package)
        if isinstance(text, NotInRegistry):
            problems.append(f"{where}: crates.io holds no package named {e.package}")
            continue
        if isinstance(text, OSError):
            problems.append(f"{where}: crates.io did not answer ({text})")
            continue
        when = published(text, e.version)
        if when is ABSENT:
            problems.append(f"{where}: crates.io holds no {e.package} {e.version}")
        elif when is None:
            problems.append(f"{where}: crates.io gives no publish time for {e.package} {e.version}")
        elif when.date().isoformat() != e.published:
            problems.append(f"{where}: {e.package} {e.version} is recorded as published on {e.published}; crates.io says {when.date().isoformat()}")
    for name, version in new:
        text = text_of(name)
        if isinstance(text, NotInRegistry):
            problems.append(f"{name} {version}: crates.io holds no package named {name}")
            continue
        if isinstance(text, OSError):
            problems.append(f"{name} {version}: crates.io did not answer ({text})")
            continue
        when = published(text, version)
        if when is ABSENT:
            problems.append(f"{name} {version}: crates.io holds no such version")
        elif when is None:
            problems.append(f"{name} {version}: crates.io gives no publish time, so its age is not known")
        else:
            problem = verdict(name, version, when, now, exceptions)
            if problem:
                problems.append(problem)
    return problems


def run(paths, read, base_read, fetch, now):
    """Every problem. `read(path)` gives a tracked file's text and
    `base_read(path)` the same file's text at the base, or None when the
    base has no such file; `base_read` is None when there is no base, and
    `fetch` is None when the registry is not to be asked."""
    exceptions, problems, locks = [], [], {}
    for path in paths:
        name = posixpath.basename(path)
        if name == "Cargo.toml":
            found, bad = exceptions_in(read(path), path)
            exceptions += found
            problems += bad
        elif name == "Cargo.lock":
            locks[path] = read(path)
    try:
        for path in locks:
            locked(locks[path])
    except tomllib.TOMLDecodeError as error:
        raise ValueError(f"{path}: not a Cargo.lock ({error})") from error
    if problems:
        return problems
    problems = offline_problems(exceptions, locks)
    if problems or fetch is None:
        return problems
    new = []
    if base_read is not None:
        new = new_versions({path: base_read(path) for path in locks}, locks)
    return registry_problems(new, exceptions, fetch, now)


def walked(root):
    """Every Cargo.toml and Cargo.lock under a directory that is no
    repository, as git would name them; build output, installed packages
    and git's own files are passed over."""
    paths = []
    for directory, names, files in os.walk(root):
        names[:] = sorted(n for n in names if n not in WALK_SKIPS)
        for name in sorted(files):
            if name in ("Cargo.toml", "Cargo.lock"):
                paths.append((pathlib.Path(directory) / name).relative_to(root).as_posix())
    return paths


def fetch_index(name):
    """A crate's index file from crates.io. Tried three times; the tests
    do not reach this function (see the test file's first lines)."""
    request = urllib.request.Request(INDEX_URL + index_path(name), headers={"User-Agent": "check_dependency_age"})
    last = None
    for _ in range(3):
        try:
            with urllib.request.urlopen(request, timeout=20) as answer:
                return answer.read().decode("utf-8")
        except urllib.error.HTTPError as error:
            if error.code in (404, 410, 451):
                raise NotInRegistry(name) from error
            last = error
        except OSError as error:
            last = error
    raise OSError(str(last))


def main():
    parser = argparse.ArgumentParser(description="The exception to the 14-day rule for dependency versions.")
    parser.add_argument("--walk", action="store_true", help="read the files under this directory, not git's tracked ones")
    parser.add_argument("--registry", action="store_true", help="also ask crates.io for publish times")
    parser.add_argument("--base", metavar="REV", help="with --registry: refuse a young version this revision's lockfiles did not hold")
    args = parser.parse_args()
    if args.base and not args.registry:
        parser.error("--base needs --registry")
    if args.base and args.walk:
        parser.error("--base needs a repository, not --walk")

    if args.walk:
        root = pathlib.Path.cwd()
        paths = walked(root)
    else:
        top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
        if top.returncode != 0:
            print("check_dependency_age: not in a git repository; --walk reads the files under this directory", file=sys.stderr)
            return 2
        root = pathlib.Path(top.stdout.strip())
        paths = subprocess.run(["git", "ls-files"], cwd=root, capture_output=True, text=True, check=True).stdout.splitlines()

    def read(path):
        return (root / path).read_text(encoding="utf-8")

    def base_read(path):
        shown = subprocess.run(["git", "show", f"{base}:{path}"], cwd=root, capture_output=True, text=True)
        return shown.stdout if shown.returncode == 0 else None

    base = None
    if args.base:
        resolved = subprocess.run(["git", "rev-parse", "--verify", "--quiet", f"{args.base}^{{commit}}"], cwd=root, capture_output=True, text=True)
        if resolved.returncode != 0:
            print(f"check_dependency_age: --base {args.base}: no such commit here", file=sys.stderr)
            return 1
        base = resolved.stdout.strip()

    try:
        found = run(paths, read, base_read if base else None, fetch_index if args.registry else None, datetime.datetime.now(datetime.timezone.utc))
    except ValueError as error:
        print(f"check_dependency_age: {error}", file=sys.stderr)
        return 1
    for problem in found:
        print(f"check_dependency_age: {problem}", file=sys.stderr)
    if found:
        return 1
    lockfiles = sum(1 for p in paths if posixpath.basename(p) == "Cargo.lock")
    exceptions = sum(len(exceptions_in(read(p), p)[0]) for p in paths if posixpath.basename(p) == "Cargo.toml")
    plural = "" if exceptions == 1 else "s"
    files = "lockfile" if lockfiles == 1 else "lockfiles"
    how = "publish dates read from crates.io" if args.registry else "publish dates not checked without --registry"
    against = f", new versions against {base[:12]}" if base else ""
    print(f"check_dependency_age: ok ({exceptions} exception{plural}, {lockfiles} {files}; {how}{against})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
