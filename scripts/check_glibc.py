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
"""

import re
import subprocess
import sys

# A glibc symbol version as objdump prints it, with or without parentheses.
SYMBOL = re.compile(r"\(?GLIBC_([0-9]+(?:\.[0-9]+)+|PRIVATE)\)?\s+(\S+)\s*$")
FLOOR = re.compile(r"[0-9]+(?:\.[0-9]+)+")


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


def main(argv):
    if len(argv) < 3:
        print("usage: check_glibc.py <floor> <binary>...", file=sys.stderr)
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
