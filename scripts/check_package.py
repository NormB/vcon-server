"""Checks what each crate published from this workspace would hold.

Builds every published crate's package with `cargo package --no-verify`
and reads the .crate files themselves, so the check sees exactly what
crates.io would get. Then it builds each crate from its .crate alone, in a
directory made for the run, so a file the build reads but the package
leaves out fails here. For each package it refuses:

- a file outside the allowed set: the manifest files cargo writes, the
  README, the two licences, Rust sources under src/, and the files those
  sources read with include_str! or include_bytes!;
- a missing README, licence or src/lib.rs, or a licence that differs from
  the repository's own copy;
- a manifest without the fields the crates.io page shows, with
  `publish = false`, more than five keywords, or a dependency taken by git
  or by path;
- a package taken from git in the packaged Cargo.lock;
- a relative link in the README, which crates.io cannot resolve;
- anything the commit scan refuses (scripts/scan_forbidden.py): AI-tool
  attribution, personal data, private design-document ids.

Run: python3 -I scripts/check_package.py
"""

import importlib.util
import io
import json
import os
import pathlib
import posixpath
import re
import subprocess
import sys
import tarfile
import tempfile
import tomllib

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("scan_forbidden", HERE / "scan_forbidden.py")
scan_forbidden = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(scan_forbidden)

# Files cargo writes into every package.
CARGO_FILES = {"Cargo.toml", "Cargo.toml.orig", "Cargo.lock", ".cargo_vcs_info.json"}
LICENCES = ("LICENSE-MIT", "LICENSE-APACHE")
REQUIRED = ("README.md", *LICENCES, "src/lib.rs")
PAGE_FIELDS = (
    "description",
    "homepage",
    "documentation",
    "readme",
    "keywords",
    "categories",
    "license",
    "repository",
    "rust-version",
)
DEPENDENCY_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")
MAX_KEYWORDS = 5
def published_crates(manifest):
    """The crates the workspace manifest's [workspace.metadata] `published`
    lists: those this repository publishes on crates.io."""
    published = tomllib.loads(manifest).get("workspace", {}).get("metadata", {}).get("published")
    if not published:
        raise SystemExit("check-package: Cargo.toml has no [workspace.metadata] published list")
    return published


def to_publish(members, published):
    """The workspace members to package (each a `cargo metadata` package),
    and a finding for each crate in `published` that is missing or marked
    publish = false. Any other member, such as a client example, says
    publish = false and is not packaged."""
    crates = [m for m in members if m.get("publish") != []]
    names = {m["name"] for m in crates}
    found = [f"{name}: not published (missing, or publish = false)" for name in published if name not in names]
    return crates, found


INCLUDE = re.compile(r"""include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)""")


def build_data(files):
    """The paths, within the package, that its Rust sources read with
    include_str! or include_bytes!: data the build needs."""
    data = set()
    for path, content in files.items():
        if not (path.startswith("src/") and path.endswith(".rs")):
            continue
        for m in INCLUDE.finditer(content.decode("utf-8", errors="replace")):
            # A path that leaves the package never matches a packaged file.
            data.add(posixpath.normpath(posixpath.join(posixpath.dirname(path), m.group(1))))
    return data


def allowed(path, data):
    if path in CARGO_FILES or path in ("README.md", *LICENCES) or path in data:
        return True
    return path.startswith("src/") and path.endswith(".rs")


def dependency_findings(manifest):
    tables = [(t, manifest.get(t, {})) for t in DEPENDENCY_TABLES]
    for target, spec in manifest.get("target", {}).items():
        tables += [(f"target.{target}.{t}", spec.get(t, {})) for t in DEPENDENCY_TABLES]
    for table, deps in tables:
        for dep, value in deps.items():
            if isinstance(value, dict):
                for source in ("git", "path"):
                    if source in value:
                        yield f"[{table}] {dep} is taken by {source}"


def manifest_findings(text):
    try:
        manifest = tomllib.loads(text)
    except tomllib.TOMLDecodeError as e:
        yield f"Cargo.toml does not parse: {e}"
        return
    package = manifest.get("package", {})
    for field in PAGE_FIELDS:
        if field not in package:
            yield f"Cargo.toml: {field} is missing"
    if package.get("publish") is False or package.get("publish") == []:
        yield "Cargo.toml: publish is false"
    if len(package.get("keywords", [])) > MAX_KEYWORDS:
        yield f"Cargo.toml: keywords holds more than {MAX_KEYWORDS}"
    for finding in dependency_findings(manifest):
        yield f"Cargo.toml: {finding}"


FENCE = re.compile(r"^(```|~~~).*?^\1", re.M | re.S)
CODE_SPAN = re.compile(r"`[^`\n]*`")
INLINE_LINK = re.compile(r"\]\(\s*<?([^)\s>]+)")
REFERENCE_LINK = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*<?(\S+?)>?(?:\s|$)", re.M)
HTML_LINK = re.compile(r"""\b(?:src|href|srcset)\s*=\s*["']([^"'\s]+)""", re.I)
ABSOLUTE = ("https://", "http://", "mailto:", "#")


def lock_findings(text):
    try:
        lock = tomllib.loads(text)
    except tomllib.TOMLDecodeError as e:
        yield f"Cargo.lock does not parse: {e}"
        return
    for package in lock.get("package", []):
        if package.get("source", "").startswith("git+"):
            yield f"Cargo.lock: {package.get('name')} comes from git"


def readme_findings(text):
    prose = CODE_SPAN.sub("", FENCE.sub("", text))
    for pattern in (INLINE_LINK, REFERENCE_LINK, HTML_LINK):
        for m in pattern.finditer(prose):
            target = m.group(1)
            if not target.startswith(ABSOLUTE):
                yield f"README.md: relative link {target!r}; crates.io cannot resolve it"


def scan_findings(name, files):
    for path, content in sorted(files.items()):
        text = content.decode("utf-8", errors="replace")
        shown = f"{name}/{path}"
        for reason, hit in scan_forbidden.scan(shown, shown):
            yield f"{path}: path: {reason}: {hit!r}"
        for line in text.splitlines():
            for reason, hit in scan_forbidden.scan(shown, line):
                yield f"{path}: {reason}: {hit!r}"


def findings(name, files, root):
    """Every reason the package `name` (as "<crate>-<version>"), holding
    `files` (path within the package -> bytes), may not be published.
    `root` maps each licence file name to the repository's own copy."""
    found = []
    data = build_data(files)
    for path in sorted(files):
        if not allowed(path, data):
            found.append(f"{path}: not allowed in the package")
    for path in REQUIRED:
        if path not in files:
            found.append(f"{path}: missing from the package")
    for path in LICENCES:
        if path in files and files[path] != root[path]:
            found.append(f"{path}: differs from the repository's copy")
    if "Cargo.toml" in files:
        found += manifest_findings(files["Cargo.toml"].decode("utf-8", errors="replace"))
    if "Cargo.lock" in files:
        found += lock_findings(files["Cargo.lock"].decode("utf-8", errors="replace"))
    if "README.md" in files:
        found += readme_findings(files["README.md"].decode("utf-8", errors="replace"))
    found += scan_findings(name, files)
    return found


def read_crate(path, name):
    """The files of the .crate at `path`, by path within package `name`."""
    files = {}
    with tarfile.open(path, "r:gz") as tar:
        for member in tar.getmembers():
            if not member.isfile():
                continue
            prefix, _, inner = member.name.partition("/")
            if prefix != name or not inner:
                raise SystemExit(f"check-package: {path}: unexpected entry {member.name!r}")
            files[inner] = tar.extractfile(member).read()
    return files


def patches(unpacked, crates):
    """The --config arguments that make every published crate in `crates`
    ((package name, crate name) pairs) come from its unpacked .crate under
    `unpacked`, by path, instead of from a registry."""
    args = []
    for package, name in crates:
        path = (unpacked / package).as_posix()
        args += ["--config", f'patch.crates-io.{name}.path="{path}"']
    return args


def verify_build(run, build, crates):
    """Builds each packaged crate from its .crate file alone, in a directory
    made for this run and removed after it, with the other published crates
    patched in by path. cargo fingerprints a path dependency by its content,
    so no copy of a crate from an earlier run or another tree can be built
    in its place (cargo package's own verify takes them from a local registry
    whose extracted copies are reused while their version stays the same).
    `crates` holds (package name, crate name) pairs."""
    unpacked = run / "unpacked"
    for package, _ in crates:
        with tarfile.open(run / "package" / f"{package}.crate", "r:gz") as tar:
            tar.extractall(unpacked, filter="data")
    for package, _ in crates:
        built = subprocess.run(
            ["cargo", "build", "--quiet", "--manifest-path", str(unpacked / package / "Cargo.toml"),
             *patches(unpacked, crates)],
            capture_output=True,
            text=True,
            check=False,
            env={**os.environ, "CARGO_TARGET_DIR": str(build)},
        )
        if built.returncode != 0:
            print(built.stderr, file=sys.stderr)
            print(f"check-package: {package} does not build from its package alone", file=sys.stderr)
            return False
    return True


def main():
    # The tree this script belongs to, not git's idea of it: the commit hook
    # runs this in a copy of what is staged, where git would name the
    # working tree instead.
    root = HERE.parent
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )
    target = pathlib.Path(os.environ.get("CARGO_TARGET_DIR") or metadata["target_directory"])
    licences = {p: (root / p).read_bytes() for p in LICENCES}
    # Every file this run writes is in a directory of its own, removed after
    # it, so a run in another tree at the same moment cannot read or replace
    # them. Only the verify build shares a target directory, where cargo
    # locks it and fingerprints each patched crate by its content.
    with tempfile.TemporaryDirectory(prefix="check-package-") as run:
        return check(root, pathlib.Path(run), target / "check-package-build", metadata, licences)


def check(root, run, build, metadata, licences):
    crates, found = to_publish(metadata["packages"], published_crates((root / "Cargo.toml").read_text()))
    for finding in found:
        print(f"check-package: {finding}")
    failed = bool(found)
    selected = [arg for c in crates for arg in ("-p", c["name"])]
    packaged = subprocess.run(
        ["cargo", "package", *selected, "--no-verify", "--locked", "--allow-dirty", "--quiet"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
        env={**os.environ, "CARGO_TARGET_DIR": str(run)},
    )
    if packaged.returncode != 0:
        print(packaged.stderr, file=sys.stderr)
        print("check-package: cargo package failed", file=sys.stderr)
        return 1
    for package in crates:
        name = f"{package['name']}-{package['version']}"
        crate = run / "package" / f"{name}.crate"
        if not crate.is_file():
            print(f"check-package: {crate} was not written", file=sys.stderr)
            failed = True
            continue
        for finding in findings(name, read_crate(crate, name), licences):
            print(f"{name}: {finding}")
            failed = True
    if not failed:
        failed = not verify_build(run, build, [(f"{c['name']}-{c['version']}", c["name"]) for c in crates])
    if failed:
        print("check-package: FAILED", file=sys.stderr)
        return 1
    print(f"check-package: ok ({len(crates)} crates)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
