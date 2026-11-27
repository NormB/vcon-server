"""Prints the tests to run in the quick slot before a commit that touches PATHS.

    python3 -I scripts/tests_for_paths.py [--format text|commands|chain|names|json] [--everywhere] PATH...
    python3 -I scripts/tests_for_paths.py --whole-tree
    python3 -I scripts/tests_for_paths.py --check

Before a commit, the agents' rules ask for every test that reads a touched
file to run first, in the quick slot, found with `git grep -l <touched
path>`. A gate that scans every file under a directory names no file, so that
search never finds it, and the commit hook is the first to fail it, after its
wait in the build queue (sipnab, twice on 2026-11-26). This lists those gates
too, by one rule.

WHAT IS PRINTED. One job a line, a command for heavy-run, one test binary a
job (the quick slot refuses a whole suite), run the way the hook runs it:
`scripts/nextest.sh run -p <package> --locked --test <name>` (unit tests
`--lib --bins`), again with `--features postgres` for a vcon-server binary
that holds postgres code, and the library's doctests as `cargo test -p
<package> --locked --quiet --doc`:

* every test binary (crates/<c>/tests/*.rs of each workspace member, and
  each member's library unit tests, `--lib`) that names a touched path,
  includes a touched file as a module, compiles it (the library) or is it;
* every whole-tree binary (THE RULE, below) whose scope holds a touched path;
* every script test the pre-commit hook runs (`python3 -I scripts/test_*.py`,
  `scripts/test-*.sh`) that is the touched file, names it (a script by its
  path or its file name), or walks a directory its scope holds;
* every step of the pre-commit hook that runs a script of scripts/
  (`python3 -I scripts/check_style.py`, `scripts/rustfmt.sh --check`) and
  names a touched path, is the touched script, or walks a directory that
  holds one, by the script rule below; a step that runs cargo fmt for any
  changed .rs file. The hook's `--walk` is dropped (in a checkout the script
  reads the tracked files, which are what the hook's copy holds). RUNNERS
  names the hook's commands that are no step (the test suite, clippy);
* the how-to scenarios a push of the change needs (scripts/docs_test.py's
  `needed`, every crate source file counted as server source): one MAIN-slot
  run of `docs_test.py push`, before `git push`.

THE ORDER. The likeliest miss first: (1) the whole-tree gates that run for
every change; (2) the whole-tree gates whose scope holds the change; (3) the
tests that name, include or compile a touched file; (4) the how-to scenarios.
`--format chain` prints the jobs as one shell command, each joined to the
next with `&&`, so the run stops at the first failure.

`--format text` (the default) ends each command with its reason as a shell
comment; `commands` prints the commands alone, `names` the binary and test
names, `json` a list of jobs with their phase.

THE RULE. A Rust test binary is a whole-tree test when its test code lists a
directory: `read_dir` (of anything but /proc), `WalkDir`, `glob`, `git
ls-files`, `git grep` or `find`; or reads the change set from git
(`rev-list`, `merge-base`, `--name-only`, `--name-status`, `--porcelain`). Its test code is every file it compiles (its
own and every module it includes by `#[path]`, `mod` or `include!`); in a
library, a file's text from its first `#[cfg(test)]` on, and every module
declared under `#[cfg(test)]`. A script test is a whole-tree test when it, or
a script it loads (by "name.py", by "name", or by import, at any depth),
walks a directory (`os.walk`, `listdir`, `scandir`, `iterdir`, `glob`,
`rglob`, scripts/tree_files.py, `--walk`, git ls-files or git grep) or reads
the change set from git. The scope is the tracked paths its string literals
name, word by word, relative to the crate ("src") or climbing out of it
("../../website/content"); it runs for every change when it lists the whole
repository or its changes (git ls-files, git grep, find, tree_files, --walk,
git history, a walk of the repository root, a walk that skips "target" and
".git") or names no tracked directory. `--everywhere` drops the scope.

THE CENSUS (`--check`). A second count found another way: each file a
binary's code mentions (a `name::` path, a `mod name`, a ".rs" literal)
instead of the modules it resolves, each .rs file under a library's src/
instead of its module tree, and a wider list of walking calls. A binary the
census finds and the rule does not is a fault, unless EXEMPT names it with
the reason it reads no tracked directory. scripts/test_tests_for_paths.py runs
it on the real tree, so a gate that walks the tree in a new way fails there
until the rule finds it.

Exit status: 0; 1 when --check finds a fault; 2 on a usage error or a path
outside the repository.

Run with: python3 -I scripts/tests_for_paths.py (from anywhere in the repository).
"""

from __future__ import annotations

import argparse
import dataclasses
import importlib.util
import json
import os
import pathlib
import posixpath
import re
import sys
import tomllib
import typing

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent
HOOK = ".githooks/pre-commit"
QUICK = "heavy-run --quick -- "
PUSH_RUN = 'heavy-run -- python3 -I scripts/docs_test.py push "$(git rev-parse HEAD)" --since origin/main'

# The command name in two halves: only scripts/tree_files.py runs it.
LISTING = "ls-" + "files"

# Binaries the census finds and the rule leaves out, each with the reason it
# reads no tracked directory. --check refuses a line the census does not need.
EXEMPT: dict[str, str] = {
    "crates/vcon-server/tests/catalog_temp.rs": "its one read_dir lists /proc/self/fd",
    "scripts/test_codeql_pin.py": "scripts/codeql_pin.py names check_workflow_downloads.py only in its docstring",
}


def _load(name: str):
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    return sys.modules[name]


# ── the tree ────────────────────────────────────────────────────────────────


class Tree:
    """The files of a tree, read on demand, and the list of its tracked paths."""

    def __init__(self, read: typing.Callable[[str], str | None], tracked: typing.Iterable[str]):
        self._read = read
        self.tracked = list(tracked)
        self._cache: dict[str, str | None] = {}

    def read(self, rel: str) -> str | None:
        if rel not in self._cache:
            text = self._read(rel)
            self._cache[rel] = None if text is None else text.replace("\r\n", "\n")
        return self._cache[rel]

    @classmethod
    def of_repo(cls, repo: pathlib.Path) -> "Tree":
        """The repository's tracked files, or, in a directory with no .git (the
        commit hook's copy of the staged files), every file under it."""
        tree_files = _load("tree_files")
        if (repo / ".git").exists():
            env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
            root, tracked = tree_files.tracked(repo, env)
        else:
            root, tracked = repo.resolve(), tree_files.walked(repo)

        def read(rel: str) -> str | None:
            try:
                return (root / rel).read_text(encoding="utf-8")
            except (FileNotFoundError, IsADirectoryError, NotADirectoryError, UnicodeDecodeError):
                return None

        return cls(read, tracked)


@dataclasses.dataclass(frozen=True)
class Target:
    """One test binary or script test."""

    file: str
    name: str
    package: str | None  # None for a script test
    base: str  # "crates/<c>/" for a Rust binary, "" for a script test
    lib: bool = False  # the package's library unit tests (`--lib`); file is its src/lib.rs


@dataclasses.dataclass(frozen=True)
class Scope:
    everywhere: bool
    roots: frozenset[str]
    why: str


@dataclasses.dataclass
class Job:
    kind: str  # "cargo", "script" or "docs-scenarios"
    name: str
    file: str | None
    command: str
    reasons: list[str]
    phase: int = 0  # the order the jobs run in: PHASE_* below


# The order of the jobs, the likeliest miss first: a whole-tree gate that
# runs for every change, then one whose scope holds the change, then a test
# that names, includes or compiles a touched file, then the how-to scenarios
# (a main-slot run). A run that stops at its first failure (`--format chain`)
# therefore shows the common miss in its first jobs.
PHASE_EVERYWHERE, PHASE_SCOPED, PHASE_NAMED, PHASE_SCENARIOS = 1, 2, 3, 4


def _toml(tree: Tree, rel: str) -> dict:
    text = tree.read(rel)
    if text is None:
        return {}
    try:
        return tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return {}


_CRATE_FILE = re.compile(r"(.+)/(?:tests/([^/]+)\.rs|src/lib\.rs)")
_HOOK_TEST = re.compile(r"^\s*(?:python3\s+-I\s+)?(scripts/test[_-][\w-]+\.(?:py|sh))\b", re.M)


def hook_script_tests(tree: Tree) -> list[str]:
    """The script tests the pre-commit hook runs, in its order."""
    found: list[str] = []
    for m in _HOOK_TEST.finditer(tree.read(HOOK) or ""):
        if m.group(1) not in found:
            found.append(m.group(1))
    return found


def targets(tree: Tree) -> list[Target]:
    """Every Rust test binary of each workspace member, then the hook's script tests."""
    members = set(_toml(tree, "Cargo.toml").get("workspace", {}).get("members", []))
    found: list[Target] = []
    for rel in sorted(tree.tracked):
        m = _CRATE_FILE.fullmatch(rel)
        if not m or m.group(1) not in members:
            continue
        package = _toml(tree, f"{m.group(1)}/Cargo.toml").get("package", {}).get("name")
        if not package:
            continue
        if m.group(2) is None:
            found.append(Target(rel, f"{package} lib", package, m.group(1) + "/", lib=True))
        else:
            found.append(Target(rel, m.group(2), package, m.group(1) + "/"))
    found += [Target(rel, rel, None, "") for rel in hook_script_tests(tree)]
    return found


def target_of(tree: Tree, file: str) -> Target:
    for target in targets(tree):
        if target.file == file:
            return target
    raise KeyError(f"{file} is no test binary")


# ── what a binary compiles ──────────────────────────────────────────────────

_PATH_ATTR = re.compile(r'#\[path\s*=\s*"([^"]+)"\]\s*')
_MOD_DECL = re.compile(r"(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-z0-9_]+)\s*;")
_INCLUDE = re.compile(r'\binclude!\(\s*"([^"]+)"\s*\)')
_TEST_MOD = re.compile(r"#\[cfg\(test\)\]\s*((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?mod\s+([a-z0-9_]+)\s*;")


def code_of(text: str) -> str:
    """The text without its comment lines."""
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith(("//", "#!")) or
                     line.lstrip().startswith("#!["))


def _py_code(text: str) -> str:
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))


def includes(rel: str, text: str, crate_root: bool = False) -> list[str]:
    """The repository paths `rel` includes as modules, in order: a `#[path]`
    module against the directory of `rel`, a plain `mod name;` at both places
    Rust looks, an `include!("...")` against the directory of `rel`."""
    directory, name = posixpath.split(rel)
    base = directory if crate_root or name == "mod.rs" else posixpath.join(directory, name.removesuffix(".rs"))
    found: list[str] = []
    pending = None
    for raw in text.splitlines():
        rest = raw.strip()
        if rest.startswith("//"):
            continue
        for macro in _INCLUDE.finditer(rest):
            found.append(posixpath.normpath(posixpath.join(directory, macro.group(1))))
        attr = _PATH_ATTR.match(rest)
        if attr:
            pending = attr.group(1)
            rest = rest[attr.end():]
            if not rest:
                continue
        decl = _MOD_DECL.fullmatch(rest)
        if decl and pending:
            found.append(posixpath.normpath(posixpath.join(directory, pending)))
        elif decl:
            found += [posixpath.join(base, f"{decl.group(1)}.rs"), posixpath.join(base, decl.group(1), "mod.rs")]
        if decl or not rest.startswith("#["):
            pending = None
    return found


def compiled(target: Target, tree: Tree) -> dict[str, str]:
    """{path: text} of the binary's file and every module it includes, at any depth."""
    if target.package is None:
        text = tree.read(target.file)
        return {} if text is None else {target.file: text}
    seen: dict[str, str] = {}
    todo = [target.file]
    while todo:
        rel = todo.pop()
        if rel in seen:
            continue
        text = tree.read(rel)
        if text is None:
            continue
        seen[rel] = text
        todo.extend(includes(rel, text, crate_root=rel == target.file))
    return seen


def _test_part(code: str) -> str:
    lines = code.splitlines(keepends=True)
    for i, line in enumerate(lines):
        if line.lstrip().startswith("#[cfg(test)]"):
            return "".join(lines[i:])
    return ""


def _module_of(path: str) -> str:
    name = posixpath.basename(path)
    return posixpath.basename(posixpath.dirname(path)) if name == "mod.rs" else name.removesuffix(".rs")


_LOADED = re.compile(r'"([\w-]+\.(?:py|sh))"')
_IMPORT = re.compile(r"^\s*(?:from\s+(\w+)\s+import\b|import\s+([\w ,]+))", re.M)


_BARE = re.compile(r'"([\w-]+)"')


def _script_names(text: str, stems: set[str]) -> list[str]:
    """The files of scripts/ a script loads: by a "name.py" literal, by a "name"
    literal of a script's own name (`load("check_required_checks")`), or by import."""
    found = _LOADED.findall(text) + [f"{name}.py" for name in _BARE.findall(text) if name in stems]
    for m in _IMPORT.finditer(text):
        modules = [m.group(1)] if m.group(1) else [x.strip() for x in m.group(2).split(",")]
        found += [f"{name}.py" for name in modules if name]
    return found


def _script_code(target: Target, tree: Tree) -> dict[str, str]:
    """{path: code} of a script test and every script of scripts/ it loads, at any depth."""
    stems = {p[len("scripts/"):-3] for p in tree.tracked
             if p.startswith("scripts/") and p.endswith(".py") and "/" not in p[len("scripts/"):]}
    found: dict[str, str] = {}
    todo = [target.file]
    while todo:
        rel = todo.pop()
        if rel in found:
            continue
        text = tree.read(rel)
        if text is None:
            continue
        found[rel] = _py_code(text) if rel.endswith(".py") else text
        todo.extend(f"scripts/{name}" for name in _script_names(text, stems))
    return found


def test_code(target: Target, tree: Tree) -> dict[str, str]:
    """{path: code} of what the binary runs only as tests, comment lines removed.

    A test binary is test code throughout; a library's file from its first
    `#[cfg(test)]` on, and a module declared under `#[cfg(test)]` (with all
    it includes) throughout. A script test is its own text and the text of
    each script of scripts/ it loads, at any depth (`_script_code`).
    """
    if target.package is None:
        return _script_code(target, tree)
    if not target.lib:
        return {rel: code_of(text) for rel, text in compiled(target, tree).items()}
    seen: dict[str, str] = {}
    todo = [(target.file, False)]
    while todo:
        rel, whole = todo.pop()
        if rel in seen:
            continue
        text = tree.read(rel)
        if text is None:
            continue
        code = code_of(text)
        seen[rel] = code if whole else _test_part(code)
        gated = {m.group(2) for m in _TEST_MOD.finditer(code)}
        gated_paths = {
            posixpath.normpath(posixpath.join(posixpath.dirname(rel), p.group(1)))
            for m in _TEST_MOD.finditer(code) for p in _PATH_ATTR.finditer(m.group(1))
        }
        for included in includes(rel, text, crate_root=rel == target.file):
            todo.append((included, whole or included in gated_paths or _module_of(included) in gated))
    return {rel: code for rel, code in seen.items() if code}


# ── the rule ────────────────────────────────────────────────────────────────

RUST_RULE: list[tuple[str, re.Pattern]] = [
    ("read_dir", re.compile(r'\bread_dir\s*\(\s*(?!&?\s*(?:format!\s*\(\s*)?"/proc)')),
    ("WalkDir", re.compile(r"\bWalkDir\b|\bwalkdir::")),
    ("glob", re.compile(r"\bglob::|\bglob\s*\(")),
    ("git " + LISTING, re.compile(r'\bls-files\b|"ls-"\s*,\s*"files"')),
    ("git grep", re.compile(r'"grep"')),
    ("find", re.compile(r'Command::new\(\s*"find"\s*\)')),
    # The change set itself (`git diff --name-only <tag>`, `git rev-list`); a word such as "diff" alone is no
    # git argument.
    ("git history", re.compile(r'"(?:rev-list|merge-base|--name-only|--name-status|--porcelain)"')),
]
SCRIPT_RULE: list[tuple[str, re.Pattern]] = [
    ("os.walk", re.compile(r"\bos\.walk\s*\(")),
    ("listdir", re.compile(r"\bos\.(?:listdir|scandir)\s*\(")),
    ("iterdir", re.compile(r"\.iterdir\s*\(")),
    ("glob", re.compile(r"\.r?glob\s*\(|\bglob\.i?glob\s*\(")),
    ("tree_files", re.compile(r"\btree_files\b|--walk\b")),
    ("git " + LISTING, re.compile(r'\bls-files\b|"ls-"\s*\+?\s*"files"')),
    ("git grep", re.compile(r'"grep"')),
    ("git history", re.compile(r"[\"'](?:rev-list|merge-base|--name-only|--name-status|--porcelain)[\"']")),
]
WHOLE_REPO_MARKS = {"git " + LISTING, "git grep", "find", "tree_files", "git history"}
_ANCHOR = (
    r'(?:repo|repo_root|root|workspace_root|workspace)\(\)'
    r'|Path::new\(\s*env!\(\s*"CARGO_MANIFEST_DIR"\s*\)\s*\)'
    r'|env!\(\s*"CARGO_MANIFEST_DIR"\s*\)'
    r'|"\."'
)
ROOT_WALK = re.compile(
    rf"\b(?:read_dir|WalkDir::new)\s*\(\s*&?\s*(?:{_ANCHOR})\s*\)"
    r"|\b(?:REPO|ROOT|REPO_ROOT|TOP)\s*\.\s*(?:rglob|glob|iterdir)\s*\("
    r'|\bos\.(?:walk|listdir|scandir)\s*\(\s*(?:REPO|ROOT|REPO_ROOT|TOP|"\.")'
)
_STRING = re.compile(r'"((?:[^"\\\n]|\\.)*)"' r"|'((?:[^'\\\n]|\\.)*)'")


def marks(target: Target, tree: Tree) -> list[str]:
    """The labels of the rule that find a directory walk in the binary; empty for none."""
    code = list(test_code(target, tree).values())
    rule = SCRIPT_RULE if target.package is None else RUST_RULE
    return [label for label, pattern in rule if any(pattern.search(c) for c in code)]


def _tops(tree: Tree, base: str) -> set[str]:
    return {p[len(base):].split("/", 1)[0] for p in tree.tracked if p.startswith(base) and len(p) > len(base)}


def literal_root(literal: str, tops: set[str]) -> str | None:
    """The tracked path a word names, from its first tracked top-level component."""
    parts = literal.split("/")
    for i, part in enumerate(parts):
        if part in tops:
            taken = []
            for piece in parts[i:]:
                if not piece or piece in (".", "..") or any(c in piece for c in "{}*?$"):
                    break
                taken.append(piece)
            return "/".join(taken)
    return None


def word_roots(word: str, base: str, crate_tops: set[str], repo_tops: set[str]) -> set[str]:
    """The tracked paths one word of a literal can name, from `base` (a crate) or the repository."""
    word = word.lstrip("`'\"([").rstrip("`'\"()[]:;,")
    if word.startswith("../"):
        # Relative to the crate, or to scripts/ for a script test.
        climbed = posixpath.normpath((base or "scripts/") + word)
        root = None if climbed.startswith("..") else literal_root(climbed, repo_tops)
        return {root} if root else set()
    if base:
        root = literal_root(word, crate_tops)
        if root:
            return {base + root}
    root = literal_root(word, repo_tops)
    return {root} if root else set()


def scope(target: Target, tree: Tree) -> Scope:
    """Where a whole-tree binary reads: everywhere, or under its literal roots."""
    found = marks(target, tree)
    if not found:
        return Scope(False, frozenset(), "lists no directory")
    code = list(test_code(target, tree).values())
    whole = sorted(WHOLE_REPO_MARKS.intersection(found))
    if whole:
        return Scope(True, frozenset(), f"lists the repository by {', '.join(whole)}")
    if any(ROOT_WALK.search(c) for c in code):
        return Scope(True, frozenset(), "lists the repository root")
    if any(re.search(r"[\"']target[\"']", c) and re.search(r"[\"']\.git[\"']", c) for c in code):
        return Scope(True, frozenset(), "walks from the root, skipping target and .git")
    crate_tops, repo_tops = _tops(tree, target.base) if target.base else set(), _tops(tree, "")
    roots: set[str] = set()
    for c in code:
        for m in _STRING.finditer(c):
            for word in (m.group(1) if m.group(1) is not None else m.group(2)).split():
                roots |= word_roots(word, target.base, crate_tops, repo_tops)
    if not roots:
        return Scope(True, frozenset(), "names no tracked directory")
    return Scope(False, frozenset(roots), "")


def in_scope(path: str, root: str) -> bool:
    return path == root or path.startswith(root + "/") or root.startswith(path + "/")


def _climbs_to(path: str, base: str, text: str) -> bool:
    """True when a word of a literal in `text` climbs out of `base` ("../../docs/x.md") to `path`."""
    for m in _STRING.finditer(text):
        for word in (m.group(1) if m.group(1) is not None else m.group(2)).split():
            word = word.lstrip("`'\"([").rstrip("`'\"()[]:;,")
            if word.startswith("../") and posixpath.normpath((base or "scripts/") + word) == path:
                return True
    return False


def _names(path: str, text: str, after_slash: bool = False) -> bool:
    """True when `text` holds `path` as a whole word; `after_slash` lets a file
    name follow a directory (`"$here/retry.sh"`)."""
    before = r"(?<![\w.-])" if after_slash else r"(?<![\w./-])"
    return re.search(rf"{before}{re.escape(path)}(?![\w.-])", text) is not None


# ── selection ───────────────────────────────────────────────────────────────


def command(target: Target) -> str:
    """The quick line of a script test, as the hook runs it."""
    if target.file.endswith(".py"):
        return f"{QUICK}python3 -I {target.file}"
    return f"{QUICK}{target.file}"


# The hook runs the suite as `scripts/nextest.sh run --workspace --locked`, each
# test in a process of its own, and vcon-server again with `--features postgres`;
# the doctests as `cargo test --workspace --locked --quiet --doc`. A quick line
# runs one binary the same way (on 2026-11-26 a sipnab quick line under `cargo
# test`, unit tests as threads of one process, met a race the hook cannot meet).
POSTGRES_PACKAGE = "vcon-server"
_POSTGRES = re.compile(r'feature\s*=\s*"postgres"')


def commands(target: Target, tree: Tree) -> list[str]:
    """The quick lines of a Rust binary: the hook's nextest run, and its postgres run when the binary holds
    postgres code."""
    selection = "--lib --bins" if target.lib else f"--test {target.name}"
    lines = [f"{QUICK}scripts/nextest.sh run -p {target.package} --locked {selection}"]
    if target.package == POSTGRES_PACKAGE and any(_POSTGRES.search(t) for t in compiled(target, tree).values()):
        lines.append(f"{QUICK}scripts/nextest.sh run -p {target.package} --locked --features postgres {selection}")
    return lines


def doctest_command(package: str) -> str:
    return f"{QUICK}cargo test -p {package} --locked --quiet --doc"


def _scenarios_needed(paths: list[str]) -> list[str]:
    docs_test = _load("docs_test")
    run, _skipped = docs_test.needed(paths, docs_test.SCENARIOS, bool(os.environ.get("GH_TOKEN")))
    return [name for name, _why in run]


def select(
    tree: Tree,
    paths: list[str],
    *,
    everywhere: bool = False,
    scenarios_needed: typing.Callable[[list[str]], list[str]] | None = None,
) -> list[Job]:
    """The jobs a change to `paths` (repository-relative) needs."""
    scenarios_needed = _scenarios_needed if scenarios_needed is None else scenarios_needed
    jobs: list[Job] = []
    for target in targets(tree):
        files = compiled(target, tree)
        tests = test_code(target, tree)
        reasons = []
        for path in paths:
            local = path[len(target.base):] if target.base and path.startswith(target.base) else None
            script = posixpath.basename(path) if target.package is None and path.startswith("scripts/") else None
            if target.lib and path in files:
                reasons.append(f"compiles {path}")
            elif path == target.file:
                reasons.append("is the test binary")
            elif path in files:
                reasons.append(f"includes {path}")
            elif any(_names(path, t) or (local and _names(local, t)) or (script and _names(script, t, True))
                     or _climbs_to(path, target.base, t) for t in tests.values()):
                reasons.append(f"names {path}")
        found = marks(target, tree)
        phase = PHASE_NAMED
        if found:
            sc = scope(target, tree)
            if sc.everywhere:
                reasons.append(f"walks the whole tree: {sc.why} ({', '.join(found)})")
                phase = PHASE_EVERYWHERE
            elif everywhere or any(in_scope(p, r) for p in paths for r in sc.roots):
                reasons.append(f"walks {', '.join(sorted(sc.roots))} ({', '.join(found)})")
                phase = PHASE_SCOPED
        if reasons:
            if target.package is None:
                jobs.append(Job("script", target.name, target.file, command(target), reasons, phase))
            else:
                for i, line in enumerate(commands(target, tree)):
                    name = target.name if i == 0 else f"{target.name} (postgres)"
                    jobs.append(Job("cargo", name, target.file, line, reasons, phase))
        compiles = [p for p in paths if target.lib and p in files]
        if compiles:
            jobs.append(Job("cargo", f"{target.package} doctests", None, doctest_command(target.package),
                            [f"the hook runs the library's doctests, and it compiles {', '.join(compiles)}"],
                            PHASE_NAMED))
    scenarios = scenarios_needed(list(paths))
    if scenarios:
        jobs.append(Job("docs-scenarios", "docs scenarios", None, PUSH_RUN,
                        [f"the push runs these how-to scenarios: {', '.join(scenarios)} (main slot, before git push)"],
                        PHASE_SCENARIOS))
    jobs += step_jobs(tree, paths, everywhere=everywhere)
    return sorted(jobs, key=lambda j: (j.phase, j.name))


# ── the hook's own steps ────────────────────────────────────────────────────
#
# 2026-11-26 (sipnab): a commit hook failed in codespell, a step that reads
# the changed source files, on a word the agent had not checked: it had run
# the tests and not the hook's own steps. Each step of the pre-commit hook
# that reads source files is selected by the same rule as a script test.

# Commands of the hook that are no step of their own, by prefix, each with the reason.
# --check refuses a line the hook no longer runs.
RUNNERS: dict[str, str] = {
    "scripts/nextest.sh": "runs the test suite, which this script selects one binary at a time",
    "python3 -I scripts/clippy_features.py run": "runs clippy for every feature set, a main-slot job",
}
# A command at the start of a line is a step; one fed by a pipe
# (`scan_input | python3 -I scripts/scan_forbidden.py`, the staged diff on its
# standard input) or named as an argument (`install ... scripts/cc-lld.sh`) is not.
_STEP = re.compile(
    r"^\s*(?:if\s+)?(?:!\s*)?(?:[A-Z_]+=\S*\s+)*((?:python3\s+-I\s+)?(scripts/[\w.-]+\.(?:py|sh)))"
    r"((?:[ \t]+[^\s\"'$`()|;&<>]+)*)"
)


@dataclasses.dataclass(frozen=True)
class Step:
    script: str
    command: str


def hook_steps(tree: Tree, runners: dict[str, str] | None = None) -> list[Step]:
    """The commands of the pre-commit hook that run a script of scripts/, in order:
    runners and script tests aside, each once, with its arguments. `--walk`
    is dropped: the hook passes it because it runs in a copy of the staged
    files; in a checkout the script reads the tracked files, which are the same."""
    runners = RUNNERS if runners is None else runners
    found: list[Step] = []
    for line in (tree.read(HOOK) or "").splitlines():
        if line.lstrip().startswith("#"):
            continue
        m = _STEP.match(line)
        if not m:
            continue
        script = m.group(2)
        args = m.group(3).split()
        if args and args[-1].isdigit() and line[m.end():m.end() + 1] == ">":
            args = args[:-1]
        command = " ".join([m.group(1), *[a for a in args if a != "--walk"]])
        name = posixpath.basename(script)
        if name.startswith(("test_", "test-")) or any(command.startswith(r) for r in runners):
            continue
        if command not in {s.command for s in found}:
            found.append(Step(script, command))
    return found


def step_jobs(tree: Tree, paths: list[str], *, everywhere: bool = False) -> list[Job]:
    """The hook's own steps a change to `paths` needs: each step that names a touched path,
    is the touched script, or walks a directory that holds one."""
    jobs: list[Job] = []
    for step in hook_steps(tree):
        target = Target(step.script, step.script, None, "")
        code = test_code(target, tree)
        reasons, phase = [], PHASE_NAMED
        for path in paths:
            script = posixpath.basename(path) if path.startswith("scripts/") else None
            if path == step.script:
                reasons.append("is the hook step")
            elif path in code:
                reasons.append(f"includes {path}")
            elif any(_names(path, c) or (script and _names(script, c, True)) for c in code.values()):
                reasons.append(f"names {path}")
        found = marks(target, tree)
        if found:
            sc = scope(target, tree)
            if sc.everywhere:
                reasons.append(f"walks the whole tree: {sc.why} ({', '.join(found)})")
                phase = PHASE_EVERYWHERE
            elif everywhere or any(in_scope(p, r) for p in paths for r in sc.roots):
                reasons.append(f"walks {', '.join(sorted(sc.roots))} ({', '.join(found)})")
                phase = PHASE_SCOPED
        elif any(_FORMATS.search(c) for c in code.values()) and any(p.endswith(".rs") for p in paths):
            # `cargo fmt --all` lists the workspace's .rs files itself, with no call the rule can see.
            reasons.append("formats every .rs file (cargo fmt)")
            phase = PHASE_EVERYWHERE
        if reasons:
            jobs.append(Job("hook-step", step.script, step.script, f"{QUICK}{step.command}", reasons, phase))
    return jobs


_FORMATS = re.compile(r"^[^#\n]*\bcargo\b[^\n#]*\bfmt\b|^[^#\n]*\brustfmt\s+(?!--version)", re.M)


# ── the census ──────────────────────────────────────────────────────────────

CENSUS: list[tuple[str, re.Pattern]] = [
    ("read_dir", re.compile(r"\bread_dir\b")),
    ("WalkDir", re.compile(r"\bWalkDir\b|\bwalkdir\b")),
    ("glob", re.compile(r"\bglob(?:set|walk)?(?:\s*\(|::)")),
    ("git " + LISTING, re.compile(r"\bls-files\b|\bls_files\b|\"ls-\"\s*,\s*\"files\"")),
    ("jwalk", re.compile(r"\bjwalk\b")),
    ("ignore::", re.compile(r"\bignore::")),
    ("include_dir!", re.compile(r"\binclude_dir!")),
    ("a listing command", re.compile(r'Command::new\(\s*"(?:find|ls|rg|fd|grep)"\s*\)|"(?:grep|find)"')),
    ("git history", re.compile(r'"(?:rev-list|merge-base|whatchanged|--name-only|--name-status|--porcelain|--stat|--numstat)"')),
]
_PATH_USE = re.compile(r"\b([a-z_][a-z0-9_]*)::|\bmod\s+([a-z_][a-z0-9_]*)")


def _mentioned(target: Target, tree: Tree, roots: set[str]) -> dict[str, str]:
    """{path: code} of the binary's file and every .rs file its code mentions, at any depth.

    A `name::` path or a `mod name` mentions tests/**/name.rs and
    tests/**/name/mod.rs of the binary's crate, other binaries' own files
    aside; a ".rs" literal mentions the files of that tests directory that end
    with it, or, climbing out with "../", any tracked file that does.
    """
    tests_dir = target.base + "tests/"
    rust = [p for p in tree.tracked if p.endswith(".rs")]
    by_stem: dict[str, list[str]] = {}
    for p in rust:
        if p.startswith(tests_dir) and p not in roots:
            by_stem.setdefault(_module_of(p), []).append(p)
    seen: dict[str, str] = {}
    todo = [target.file]
    while todo:
        rel = todo.pop()
        if rel in seen:
            continue
        text = tree.read(rel)
        if text is None:
            continue
        code = code_of(text)
        seen[rel] = code
        for m in _PATH_USE.finditer(code):
            todo.extend(by_stem.get(m.group(1) or m.group(2), []))
        for m in _STRING.finditer(code):
            lit = m.group(1) or ""
            if lit.endswith(".rs"):
                tail = "/".join(x for x in lit.split("/") if x not in ("", ".", ".."))
                pool = rust if lit.startswith("../") else [p for p in rust if p.startswith(tests_dir)]
                todo.extend(p for p in pool if p.endswith("/" + tail))
    return seen


def _lib_census_files(target: Target, tree: Tree) -> dict[str, str]:
    """{path: test text} of every .rs file under the package's src/, module tree aside."""
    src = target.base + "src/"
    found: dict[str, str] = {}
    for rel in tree.tracked:
        if not (rel.startswith(src) and rel.endswith(".rs")):
            continue
        text = code_of(tree.read(rel) or "")
        if rel.endswith("tests.rs") or "/tests/" in rel[len(src) - 1:]:
            found[rel] = text
        elif "#[cfg(test)]" in text:
            found[rel] = text[text.index("#[cfg(test)]"):]
    return found


SCRIPT_CENSUS: list[tuple[str, re.Pattern]] = [
    ("os.walk", re.compile(r"\bwalk\s*\(")),
    ("listdir", re.compile(r"\b(?:listdir|scandir)\b")),
    ("iterdir", re.compile(r"\biterdir\b")),
    ("glob", re.compile(r"\b(?:r?glob|iglob|fnmatch)\b")),
    ("tree_files", re.compile(r"\btree_files\b|--walk\b")),
    ("git " + LISTING, re.compile(r"\bls-files\b|\bls_files\b|\"ls-\"\s*\+?\s*\"files\"")),
    ("a listing command", re.compile(r"[\"'](?:grep|find)[\"']")),
]


def _script_mentioned(target: Target, tree: Tree) -> dict[str, str]:
    """{path: code} of the script test and every scripts/*.py whose name its code holds as a word, at any depth."""
    stems = {p[len("scripts/"):-3]: p for p in tree.tracked
             if p.startswith("scripts/") and p.endswith(".py") and "/" not in p[len("scripts/"):]}
    seen: dict[str, str] = {}
    todo = [target.file]
    while todo:
        rel = todo.pop()
        if rel in seen:
            continue
        text = tree.read(rel)
        if text is None:
            continue
        code = _py_code(text) if rel.endswith(".py") else text
        seen[rel] = code
        todo.extend(p for stem, p in stems.items() if re.search(rf"\b{re.escape(stem)}\b", code))
    return seen


def census(tree: Tree) -> dict[str, str]:
    """{binary file: what walks} of every Rust binary and script test the census finds."""
    found: dict[str, str] = {}
    every = targets(tree)
    roots = {t.file for t in every if t.package is not None}
    for target in every:
        if target.package is None:
            files, vocabulary = _script_mentioned(target, tree), SCRIPT_CENSUS
        elif target.lib:
            files, vocabulary = _lib_census_files(target, tree), CENSUS
        else:
            files, vocabulary = _mentioned(target, tree, roots), CENSUS
        for rel, code in sorted(files.items()):
            hit = next((label for label, pattern in vocabulary if pattern.search(code)), None)
            if hit:
                found[target.file] = f"{hit} in {rel}"
                break
    return found


def check(tree: Tree, exempt: dict[str, str] | None = None) -> list[str]:
    """Why the rule and the census disagree; empty when they agree."""
    exempt = EXEMPT if exempt is None else exempt
    every = targets(tree)
    files = {t.file for t in every}
    rule = {t.file for t in every if marks(t, tree)}
    found = census(tree)
    problems = []
    for file, why in sorted(found.items()):
        if file not in rule and file not in exempt:
            problems.append(
                f"{file}: the census finds a directory walk ({why}) and the rule does not: extend the rule "
                "in scripts/tests_for_paths.py, or add the binary to EXEMPT with the reason it reads no "
                "tracked directory"
            )
    for file in sorted(exempt):
        if file not in files:
            problems.append(f"EXEMPT names {file}, which is no test binary; remove the line")
        elif file in rule:
            problems.append(f"EXEMPT names {file}, and the rule finds it: the line is wrong")
        elif file not in found:
            problems.append(f"EXEMPT names {file}, and the census finds no walk in it; remove the line")
    if tree.read(HOOK) is not None:
        every_step = hook_steps(tree, runners={})
        for runner in sorted(RUNNERS):
            if not any(s.command.startswith(runner) for s in every_step):
                problems.append(f"RUNNERS names {runner}, which {HOOK} does not run; remove the line")
        for step in hook_steps(tree):
            target = Target(step.script, step.script, None, "")
            if marks(target, tree) or step.script in exempt:
                continue
            for rel, code in sorted(_script_mentioned(target, tree).items()):
                hit = next((label for label, pattern in SCRIPT_CENSUS if pattern.search(code)), None)
                if hit:
                    problems.append(
                        f"{step.script}: the census finds a directory walk ({hit} in {rel}) in this hook step "
                        "and the rule does not: extend SCRIPT_RULE in scripts/tests_for_paths.py, or add the "
                        "step to EXEMPT with the reason it reads no tracked directory"
                    )
                    break
    return problems


# ── the command line ────────────────────────────────────────────────────────


def normalize(path: str, cwd: pathlib.Path, repo: pathlib.Path) -> str | None:
    """`path` relative to the repository, or None when it is outside it."""
    absolute = pathlib.Path(os.path.abspath(cwd / path))
    for root in (repo, repo.resolve()):
        try:
            rel = absolute.relative_to(root).as_posix()
        except ValueError:
            continue
        return rel if rel not in ("", ".") else None
    return None


def _print(jobs: list[Job], form: str) -> None:
    if form == "json":
        print(json.dumps([dataclasses.asdict(j) for j in jobs], indent=2))
        return
    if form == "chain":
        # One shell command: each job runs only when the one before passed.
        for i, job in enumerate(jobs):
            print(job.command + (" && \\" if i < len(jobs) - 1 else ""))
        return
    for job in jobs:
        if form == "names":
            print(job.name)
        elif form == "commands":
            print(job.command)
        else:
            print(f"{job.command}  # {'; '.join(job.reasons)}")


def main(
    argv: list[str] | None = None,
    *,
    tree: Tree | None = None,
    exempt: dict[str, str] | None = None,
    scenarios_needed: typing.Callable[[list[str]], list[str]] | None = None,
    repo: pathlib.Path = REPO,
    cwd: pathlib.Path | None = None,
) -> int:
    """The command line; a relative PATH is read from `cwd` (default: the process's directory)."""
    cwd = pathlib.Path.cwd() if cwd is None else cwd
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("paths", nargs="*", help="the paths the change touches")
    parser.add_argument("--format", choices=["text", "commands", "chain", "names", "json"], default="text")
    parser.add_argument("--everywhere", action="store_true", help="every whole-tree binary, without its scope")
    parser.add_argument("--whole-tree", action="store_true", help="list every whole-tree binary and its scope")
    parser.add_argument("--check", action="store_true", help="hold the rule to the census")
    args = parser.parse_args(argv)
    if not (args.paths or args.check or args.whole_tree):
        parser.error("name the paths the change touches, or --check, or --whole-tree")
    tree = tree if tree is not None else Tree.of_repo(repo)
    if args.check:
        problems = check(tree, exempt)
        for problem in problems:
            print(f"tests_for_paths: {problem}", file=sys.stderr)
        return 1 if problems else 0
    if args.whole_tree:
        for target in targets(tree):
            found = marks(target, tree)
            if found:
                sc = scope(target, tree)
                where = f"everywhere: {sc.why}" if sc.everywhere else ", ".join(sorted(sc.roots))
                print(f"{target.file}  {where} ({', '.join(found)})")
        return 0
    paths = []
    for raw in args.paths:
        rel = normalize(raw, cwd, repo)
        if rel is None:
            print(f"tests_for_paths: {raw} is outside the repository", file=sys.stderr)
            return 2
        paths.append(rel)
    _print(select(tree, paths, everywhere=args.everywhere, scenarios_needed=scenarios_needed), args.format)
    return 0


if __name__ == "__main__":
    sys.exit(main())
