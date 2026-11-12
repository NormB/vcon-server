#!/usr/bin/env python3
"""Writes the text of a GitHub release: the changelog section of the
version, then the line that says how to verify a download.

release.yml's publish job runs this for the tag's version and gives the
output to `gh release create --notes-file`. Its version job runs it first,
so a tag whose version has no changelog section stops before the builds.

The section of a version is what CHANGELOG.md holds between the heading
`## [<version>] - <date>` and the next `## ` heading, without the link
definitions (`[name]: <address>`) at the end of the file. A link the
section writes by reference (`[text][name]` or `[name]`) is written in
full, since the release's text does not hold the definitions. Text in a
code block or a code span is copied as it is.

GitHub shows each line break of a release's text, so a paragraph or a
list item that the changelog writes on several lines is written on one.
The lines of a code block, a table and a quote stay as they are.

Refused, with exit code 1 and no output: a version with no section or
with two, a section with no text, a repository that is not `owner/name`,
and notes longer than a release may hold.

Usage:
    release_notes.py VERSION --repo OWNER/NAME [--changelog PATH]

VERSION is as Cargo.toml has it (0.4.4, not v0.4.4). PATH is CHANGELOG.md
in the current directory unless given.
"""

import pathlib
import re
import sys

USAGE = "usage: release_notes.py VERSION --repo OWNER/NAME [--changelog PATH]"

# The most characters GitHub takes as the body of a release.
LIMIT = 125000

_VERSION = re.compile(r"\d+\.\d+\.\d+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?")
_REPO = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")
_HEADING = re.compile(r"## \[([^\]]+)\] - \S.*")
_DEFINITION = re.compile(r"\[([^\]]+)\]:\s+(\S+)\s*")
_FENCE = re.compile(r" {0,3}(```|~~~)")
_CODE_SPAN = re.compile(r"(`+[^`]*`+)")
_LINK = re.compile(r"(?<![\]\\])\[([^\]\[]+)\](\[([^\]\[]+)\])?(?![\[(:])")
# The start of a list item: a bullet, or a number (group 2).
_ITEM = re.compile(r" *([-*+]|(\d{1,9})[.)])( |$)")
_NUMBERED = re.compile(r" *\d{1,9}[.)]( |$)")
# A line that is never joined to another: a heading, a table row, a quote,
# a rule, or the line under a heading written with `===` or `---`.
_OWN_LINE = re.compile(r" *(#{1,6}( |$)|\||>|([-*_] *){3,}$|=+ *$|-+ *$)")
# The first line of a code block written by indentation.
_INDENTED = re.compile(r"( {4}|\t)")


class Refused(Exception):
    """Why no notes are written."""


def _lines(changelog):
    """Each line of `changelog` with whether it is inside a code block (the
    lines of the fences are inside)."""
    fenced = False
    out = []
    for line in changelog.replace("\r\n", "\n").split("\n"):
        fence = _FENCE.match(line)
        out.append((line, fenced or bool(fence)))
        if fence:
            fenced = not fenced
    return out


def versions(changelog):
    """The versions that have a section, in the order of the file."""
    return [
        match.group(1)
        for line, code in _lines(changelog)
        if not code and (match := _HEADING.fullmatch(line)) and _VERSION.fullmatch(match.group(1))
    ]


def _resolved(line, definitions):
    """`line` with each link by reference written in full, outside code
    spans. A name with no definition is left as it is."""

    def link(match):
        name = match.group(3) if match.group(2) else match.group(1)
        address = definitions.get(name.lower())
        return match.group(0) if address is None else f"[{match.group(1)}]({address})"

    parts = _CODE_SPAN.split(line)
    return "".join(part if k % 2 else _LINK.sub(link, part) for k, part in enumerate(parts))


def _one_line(lines):
    """`lines`, as `_lines` gives them, with each paragraph and each list
    item on one line: a line that continues the one before it is joined to
    it with one space. A blank line, a line of a code block, a heading, a
    table row, a quote and a rule stay as they are, and so does the line
    after a break the text asks for (a line that ends with a backslash or
    with two spaces)."""
    out = []
    # The first line of the paragraph or item being joined, if one is open.
    first = None
    for line, code in lines:
        if code or not line.strip() or _OWN_LINE.match(line):
            out.append((line, code))
            first = None
            continue
        item = _ITEM.match(line)
        # A number other than 1 starts an item only in a numbered list.
        starts = bool(item) and (item.group(2) in (None, "1") or (first is not None and _NUMBERED.match(first)))
        before = out[-1][0] if out else ""
        if first is None or starts or before.endswith(("\\", "  ")) or _INDENTED.match(first):
            out.append((line, False))
            first = line
        else:
            out[-1] = (f"{before.rstrip(' ')} {line.strip(' ')}", False)
    return out


def section(changelog, version):
    """The changelog section of `version`, as text with no blank line at
    its start or end and with each paragraph and list item on one line."""
    if not _VERSION.fullmatch(version):
        raise Refused(f"`{version}` is not a version: give it as Cargo.toml has it, such as 0.4.4")
    lines = _lines(changelog)
    definitions = {
        match.group(1).lower(): match.group(2)
        for line, code in lines
        if not code and (match := _DEFINITION.fullmatch(line))
    }
    found = versions(changelog).count(version)
    if found == 0:
        raise Refused(f"CHANGELOG.md has no section `## [{version}] - <date>`")
    if found > 1:
        raise Refused(f"CHANGELOG.md has {found} sections `## [{version}]`")
    kept = []
    inside = False
    for line, code in lines:
        if not code and line.startswith("## "):
            match = _HEADING.fullmatch(line)
            inside = bool(match) and match.group(1) == version
            continue
        if not inside or (not code and _DEFINITION.fullmatch(line)):
            continue
        kept.append((line, code))
    # Joined first, so a link or a code span that breaks across two lines
    # of the changelog is whole when links are resolved.
    text = "\n".join(line if code else _resolved(line, definitions) for line, code in _one_line(kept)).strip("\n")
    if not any(line.strip() and not line.startswith("### ") for line in text.split("\n")):
        raise Refused(f"the section `## [{version}]` of CHANGELOG.md is empty")
    return text


def notes(changelog, version, repo):
    """The text of the release of `version` in the repository `repo`."""
    if not _REPO.fullmatch(repo):
        raise Refused(f"`{repo}` is not a repository: give it as owner/name")
    text = (
        f"{section(changelog, version)}\n\n"
        "## Verify a download\n\n"
        f"Verify a download with: gh attestation verify <file> --repo {repo}. "
        "SHA256SUMS.txt.sigstore.json signs SHA256SUMS.txt; SECURITY.md has the cosign command.\n"
    )
    if len(text) > LIMIT:
        raise Refused(
            f"the notes are {len(text)} characters, and a GitHub release holds at most {LIMIT}: shorten the section"
        )
    return text


def _arguments(argv):
    """The version, the repository and the changelog's path, or None when
    the arguments are not the usage's."""
    positional = []
    options = {"--repo": None, "--changelog": "CHANGELOG.md"}
    given = set()
    k = 0
    while k < len(argv):
        if argv[k] in options:
            if k + 1 >= len(argv) or argv[k] in given:
                return None
            options[argv[k]] = argv[k + 1]
            given.add(argv[k])
            k += 2
        elif argv[k].startswith("--"):
            return None
        else:
            positional.append(argv[k])
            k += 1
    if len(positional) != 1 or options["--repo"] is None:
        return None
    return positional[0], options["--repo"], options["--changelog"]


def main(argv):
    arguments = _arguments(argv)
    if arguments is None:
        print(USAGE, file=sys.stderr)
        return 2
    version, repo, path = arguments
    try:
        changelog = pathlib.Path(path).read_text(encoding="utf-8")
    except OSError as error:
        print(f"release_notes: {path}: {error.strerror}", file=sys.stderr)
        return 1
    except UnicodeDecodeError:
        print(f"release_notes: {path}: not UTF-8 text", file=sys.stderr)
        return 1
    try:
        text = notes(changelog, version, repo)
    except Refused as refused:
        print(f"release_notes: {refused}", file=sys.stderr)
        return 1
    sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
