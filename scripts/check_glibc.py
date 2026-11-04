"""Checks that a release binary needs no glibc symbol version above the
oldest glibc the release supports (the floor).

A binary linked on a newer glibc takes that glibc's newest symbol versions:
built on Ubuntu 24.04 (glibc 2.39), v0.2.0 needed GLIBC_2.38 for
__isoc23_strtol and its kin, which the C code's headers chose, and so did
not start on Debian 12, Ubuntu 22.04 or RHEL 9. The floor is held in
release.yml, which builds in a container with that glibc.

Run with: python3 -I scripts/check_glibc.py <floor> <binary>...
It reads each binary's dynamic symbols with `objdump -T` and exits 1,
naming every symbol above the floor, when any binary has one.

The pages in FLOOR_PAGES tell a reader the same floor. CI checks them
against release.yml's GLIBC_FLOOR with:
  python3 -I scripts/check_glibc.py --docs <repository root>
"""

import pathlib
import re
import subprocess
import sys

# A glibc symbol version as objdump prints it, with or without parentheses.
SYMBOL = re.compile(r"\(?GLIBC_([0-9]+(?:\.[0-9]+)+|PRIVATE)\)?\s+(\S+)\s*$")
FLOOR = re.compile(r"[0-9]+(?:\.[0-9]+)+")
WORKFLOW_FLOOR = re.compile(r"^\s*GLIBC_FLOOR:\s*'([0-9]+(?:\.[0-9]+)+)'\s*$", re.MULTILINE)
# How a page states the floor: "glibc 2.34 or later".
PAGE_FLOOR = re.compile(r"glibc\s+([0-9]+(?:\.[0-9]+)+)\s+or\s+later")
# The pages, under website/content, that state the release binary's floor.
FLOOR_PAGES = ("docs/install.md", "_index.md")


def needed(objdump_text):
    """Each (version, symbol) the dynamic symbol table asks of glibc."""
    found = []
    for text_line in objdump_text.splitlines():
        match = SYMBOL.search(text_line)
        if match:
            found.append((match.group(1), match.group(2)))
    return found


def as_tuple(version):
    return tuple(int(part) for part in version.split("."))


def too_new(objdump_text, floor):
    """The (version, symbol) pairs above `floor`, sorted; GLIBC_PRIVATE
    is never allowed, since no other glibc build promises it."""
    limit = as_tuple(floor)
    return sorted(
        (version, symbol)
        for version, symbol in needed(objdump_text)
        if version == "PRIVATE" or as_tuple(version) > limit
    )


def verdict(objdump_text, floor):
    """One line per problem; none when the binary runs on the floor."""
    if not FLOOR.fullmatch(floor):
        raise ValueError(f"the floor {floor!r} is not a version such as 2.34")
    if not needed(objdump_text):
        return ["no glibc symbol found: not a dynamically linked glibc binary, or objdump failed"]
    return [f"needs GLIBC_{version} for {symbol}, above the floor {floor}" for version, symbol in too_new(objdump_text, floor)]


def workflow_floor(workflow_text):
    """release.yml's GLIBC_FLOOR."""
    match = WORKFLOW_FLOOR.search(workflow_text)
    if not match:
        raise ValueError("release.yml sets no GLIBC_FLOOR")
    return match.group(1)


def docs_floor_problems(workflow_text, pages):
    """One line per page, of `pages` (name to text), that states a floor
    other than release.yml's or states none."""
    floor = workflow_floor(workflow_text)
    problems = []
    for name, text in pages.items():
        stated = PAGE_FLOOR.findall(" ".join(text.split()))
        if not stated:
            problems.append(f"{name} never says glibc {floor} or later")
        problems += [
            f"{name} says glibc {version} or later; release.yml builds for {floor}"
            for version in stated
            if version != floor
        ]
    return problems


def docs_problems(root):
    """docs_floor_problems for the repository at `root`, reading release.yml
    and FLOOR_PAGES; a file that cannot be read is a problem."""
    root = pathlib.Path(root)
    texts = {}
    for relative in (".github/workflows/release.yml", *(f"website/content/{p}" for p in FLOOR_PAGES)):
        try:
            texts[relative] = (root / relative).read_text(encoding="utf-8")
        except OSError:
            return [f"cannot read {relative}"]
    workflow = texts.pop(".github/workflows/release.yml")
    return docs_floor_problems(workflow, {k.removeprefix("website/content/"): v for k, v in texts.items()})


def main(argv):
    if len(argv) == 3 and argv[1] == "--docs":
        problems = docs_problems(argv[2])
        for problem in problems:
            print(f"check_glibc: {problem}", file=sys.stderr)
        if not problems:
            print("check_glibc: the pages name release.yml's glibc floor")
        return 1 if problems else 0
    if len(argv) < 3:
        print("usage: check_glibc.py <floor> <binary>... | check_glibc.py --docs <root>", file=sys.stderr)
        return 2
    floor, binaries = argv[1], argv[2:]
    bad = False
    for binary in binaries:
        result = subprocess.run(["objdump", "-T", binary], capture_output=True, text=True)
        problems = verdict(result.stdout if result.returncode == 0 else "", floor)
        for problem in problems:
            print(f"check_glibc: {binary}: {problem}", file=sys.stderr)
        if problems:
            bad = True
        else:
            newest = max((v for v, _ in needed(result.stdout)), key=as_tuple)
            print(f"check_glibc: {binary}: ok, newest glibc symbol version {newest}, floor {floor}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
