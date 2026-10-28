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

Run with: python3 -I scripts/check_dependabot.py
(from the repository root; needs PyYAML). Exits 1 naming each file not
covered.
"""

import fnmatch
import pathlib
import posixpath
import re
import subprocess
import sys

# A file listed here needs no Dependabot entry; the reason says why.
EXCEPTIONS = {}

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


def covered(config, ecosystem, directory):
    for update in (config or {}).get("updates") or []:
        if update.get("package-ecosystem") != ecosystem:
            continue
        dirs = list(update.get("directories") or [])
        if update.get("directory"):
            dirs.append(update["directory"])
        if any(fnmatch.fnmatchcase(directory, normal(d)) for d in dirs):
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


def main():
    import yaml  # Only the command needs it; the tests pass parsed data.

    root = pathlib.Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True, check=True).stdout.strip())
    paths = subprocess.run(["git", "ls-files"], cwd=root, capture_output=True, text=True, check=True).stdout.splitlines()
    config = yaml.safe_load((root / ".github/dependabot.yml").read_text(encoding="utf-8"))
    problems = uncovered(paths, config, lambda p: (root / p).read_text(encoding="utf-8"), EXCEPTIONS)
    for problem in problems:
        print(f"check_dependabot: {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"check_dependabot: ok ({len(dependency_files(paths, lambda p: (root / p).read_text(encoding='utf-8')))} dependency files covered)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
