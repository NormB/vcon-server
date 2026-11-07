"""Checks that Dependabot watches every dependency file in the tree.

Each tracked file below must be covered by an entry in
.github/dependabot.yml of its ecosystem whose directory (or one of whose
directories, which may use *) is the file's directory, or be listed in
EXCEPTIONS with the reason it needs none:

  Cargo.lock                          cargo
  package-lock.json                   npm
  *requirements*.txt                  pip
  Dockerfile                          docker
  go.mod with a require directive     gomod
  .github/actions/<name>/action.yml   github-actions, /.github/actions/<name>
  .github/workflows/*                 github-actions, /

Found in sipnab on 2026-10-27: an npm lockfile no entry covered kept a
vulnerable qs for two weeks after the fix was released.

A package a Cargo.lock takes from a git source must be ignored, for every
version, by the cargo entry for that lockfile's directory, and an entry
may ignore a package for every version only while a lockfile it covers
takes that package from git. Bumping a crates.io package of the same name
rewrites the git one's requirement as well: on 2026-11-03 Dependabot's
update job failed on rusqlite and the fork vcon-server's tests use. The
git package is then tracked by hand.

Two Cargo.locks that take a package by path (no source) share that
package's Cargo.toml, so one cargo entry must cover both directories.
Dependabot then bumps a requirement there in one pull request with both
lockfiles. With an entry each, on 2026-11-07, #142 and #143 each bumped
the server's =60.0.0 parquet pin and only their own lockfile, so the
other lockfile no longer matched and every --locked build failed.

Every entry must hold a cooldown of at least MIN_AGE_DAYS for every
version (default-days, and each semver-*-days it sets), with no include
or exclude list: a new version is proposed only once it is that old
(CONTRIBUTING.md, "Dependency versions"). Dependabot's own default is 3
days; #142 and #143 proposed parquet 60.1.0 ten days after its release.

Run with: python3 -I scripts/check_dependabot.py
(from the repository root; needs PyYAML). Exits 1 naming each file not
covered and each break of the rules for git packages, shared manifests
and the cooldown.
"""

import fnmatch
import pathlib
import posixpath
import re
import subprocess
import sys
import tomllib

# A file listed here needs no Dependabot entry; the reason says why.
EXCEPTIONS = {}

# The age in days a new version must reach before it is proposed; the rule
# is in CONTRIBUTING.md, "Dependency versions".
MIN_AGE_DAYS = 14

REQUIRE = re.compile(r"^\s*require\b", re.MULTILINE)


def directory_of(path):
    parent = posixpath.dirname(path)
    return "/" + parent if parent else "/"


def dependency_files(paths, read):
    """(file, ecosystem, directory) for each dependency file in `paths`;
    `read` gives a file's text. All workflows count as one, the
    directory "/"."""
    found = []
    workflows = False
    for path in paths:
        name = posixpath.basename(path)
        if path.startswith(".github/workflows/"):
            workflows = True
        elif re.fullmatch(r"\.github/actions/[^/]+/action\.ya?ml", path):
            found.append((path, "github-actions", directory_of(path)))
        elif name == "Cargo.lock":
            found.append((path, "cargo", directory_of(path)))
        elif name == "package-lock.json":
            found.append((path, "npm", directory_of(path)))
        elif fnmatch.fnmatch(name, "*requirements*.txt"):
            found.append((path, "pip", directory_of(path)))
        elif name == "Dockerfile":
            found.append((path, "docker", directory_of(path)))
        elif name == "go.mod" and REQUIRE.search(read(path)):
            found.append((path, "gomod", directory_of(path)))
    if workflows:
        found.append((".github/workflows", "github-actions", "/"))
    return found


def normal(directory):
    return "/" + directory.strip("/") if directory.strip("/") else "/"


def entry_directories(update):
    dirs = list(update.get("directories") or [])
    if update.get("directory"):
        dirs.append(update["directory"])
    return dirs


def covered(config, ecosystem, directory):
    for update in (config or {}).get("updates") or []:
        if update.get("package-ecosystem") != ecosystem:
            continue
        if any(fnmatch.fnmatchcase(directory, normal(d)) for d in entry_directories(update)):
            return True
    return False


def uncovered(paths, config, read, exceptions):
    """One line per dependency file no entry covers, in path order."""
    for path, reason in exceptions.items():
        if not reason.strip():
            raise ValueError(f"the exception for {path} gives no reason")
    problems = []
    for path, ecosystem, directory in sorted(dependency_files(paths, read)):
        if path in exceptions:
            continue
        if not covered(config, ecosystem, directory):
            problems.append(f"{path}: no {ecosystem} entry for {directory}")
    for path in sorted(set(exceptions) - set(paths)):
        problems.append(f"{path}: listed as an exception but not in the tree")
    return problems


def git_packages(lock_text):
    """The names of the packages a Cargo.lock takes from a git source."""
    packages = tomllib.loads(lock_text).get("package") or []
    return sorted({p["name"] for p in packages if str(p.get("source", "")).startswith("git+")})


def path_packages(lock_text):
    """The names of the packages a Cargo.lock takes by path: those with no
    source, its workspace's members and their path dependencies."""
    packages = tomllib.loads(lock_text).get("package") or []
    return sorted({p["name"] for p in packages if "source" not in p})


def cargo_locks(paths, read):
    """(file, directory) for each Cargo.lock in `paths`, in path order."""
    return [(path, directory) for path, ecosystem, directory in sorted(dependency_files(paths, read)) if ecosystem == "cargo"]


def ignores_every_version(update):
    """The dependency names `update` ignores whatever the version."""
    return {i.get("dependency-name") for i in update.get("ignore") or [] if not i.get("versions") and not i.get("update-types")}


def git_problems(paths, config, read):
    """One line per break between the git packages of the Cargo.locks a
    cargo entry covers and the entry's ignores. Dependabot's bump of a
    crates.io package of the same name as a git one rewrites the git one's
    requirement too (rusqlite and the fork the tests use, found on
    2026-11-03), so the entry must ignore the name; an ignore of a name no
    lockfile it covers takes from git only stops that package's updates, so
    it may not stay."""
    locks = cargo_locks(paths, read)
    problems = []
    for update in (config or {}).get("updates") or []:
        if update.get("package-ecosystem") != "cargo":
            continue
        mine = [(path, directory) for path, directory in locks if covered({"updates": [update]}, "cargo", directory)]
        if not mine:
            continue
        ignored = ignores_every_version(update)
        every_git = set()
        for path, directory in mine:
            git = set(git_packages(read(path)))
            every_git |= git
            for name in sorted(git - ignored):
                problems.append(f"{path}: {name} comes from git; the cargo entry for {directory} must ignore it")
        files = ", ".join(path for path, _ in mine)
        dirs = ", ".join(entry_directories(update))
        for name in sorted(ignored - every_git):
            problems.append(f"{files}: the cargo entry for {dirs} ignores {name}, which no lockfile it covers takes from git")
    return problems


def shared_problems(paths, config, read):
    """One line per pair of Cargo.locks that take a package by path, so share
    its Cargo.toml, and are not covered by one cargo entry. A bump of a
    requirement in that Cargo.toml must update both lockfiles in one pull
    request; an entry each gives two pull requests that each leave the other
    lockfile behind (#142 and #143, 2026-11-07)."""
    locks = cargo_locks(paths, read)
    updates = [u for u in (config or {}).get("updates") or [] if u.get("package-ecosystem") == "cargo"]
    problems = []
    for i, (path, directory) in enumerate(locks):
        for other, other_directory in locks[i + 1 :]:
            shared = sorted(set(path_packages(read(path))) & set(path_packages(read(other))))
            if not shared:
                continue
            mine = [u for u in updates if covered({"updates": [u]}, "cargo", directory)]
            theirs = [u for u in updates if covered({"updates": [u]}, "cargo", other_directory)]
            if not mine or not theirs:
                continue  # The coverage rule names a lockfile no entry covers.
            if any(u is t for u in mine for t in theirs):
                continue
            problems.append(f"{path} and {other} both take {', '.join(shared)} by path; one cargo entry must cover {directory} and {other_directory}")
    return problems


def is_days(value):
    # A bool is an int in Python, but True and False are both under 14.
    return isinstance(value, int) and value >= MIN_AGE_DAYS


def cooldown_problems(config):
    """One line per entry whose cooldown lets a version younger than
    MIN_AGE_DAYS through: no cooldown, a default-days missing or under it,
    a semver-*-days under it, or an include or exclude list, which leaves
    some dependencies on no cooldown at all."""
    problems = []
    for update in (config or {}).get("updates") or []:
        label = f"{update.get('package-ecosystem')} entry for {', '.join(entry_directories(update))}"
        cooldown = update.get("cooldown")
        if not isinstance(cooldown, dict):
            problems.append(f"{label}: no cooldown; a new version must wait {MIN_AGE_DAYS} days")
            continue
        for key in ("default-days", "semver-major-days", "semver-minor-days", "semver-patch-days"):
            if key not in cooldown:
                if key == "default-days":
                    problems.append(f"{label}: cooldown default-days is missing, under {MIN_AGE_DAYS}")
                continue
            if not is_days(cooldown[key]):
                problems.append(f"{label}: cooldown {key} is {cooldown[key]!r}, under {MIN_AGE_DAYS}")
        for key in ("include", "exclude"):
            if key in cooldown:
                problems.append(f"{label}: cooldown {key} exempts some dependencies from the {MIN_AGE_DAYS} days")
    return problems


def problems(paths, config, read, exceptions):
    """Every rule's problems, in the order of the rules above."""
    return (
        uncovered(paths, config, read, exceptions)
        + git_problems(paths, config, read)
        + shared_problems(paths, config, read)
        + cooldown_problems(config)
    )


def main():
    import yaml  # Only the command needs it; the tests pass parsed data.

    root = pathlib.Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip())
    paths = subprocess.run(["git", "ls-files"], cwd=root, capture_output=True, text=True, check=True).stdout.splitlines()
    config = yaml.safe_load((root / ".github/dependabot.yml").read_text(encoding="utf-8"))
    def read(path):
        return (root / path).read_text(encoding="utf-8")

    found = problems(paths, config, read, EXCEPTIONS)
    for problem in found:
        print(f"check_dependabot: {problem}", file=sys.stderr)
    if found:
        return 1
    print(f"check_dependabot: ok ({len(dependency_files(paths, read))} dependency files covered)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
