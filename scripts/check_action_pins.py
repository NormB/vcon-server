"""Checks that every `uses:` in the workflows and composite actions is pinned.

A tag or a branch is a name its owner can move to other code at any time;
a commit id is not. So each `uses:` in .github/workflows/*.yml|yaml and in
.github/actions/**/action.yml|yaml is one of:

  ./<path>                                   an action or reusable workflow
                                             in this repository (it must
                                             exist), with no @ref
  owner/repo[/path]@<40-hex commit> # v<version>
                                             an action or reusable workflow
                                             of another repository, by its
                                             full commit id, with the tag
                                             it was taken at as a comment
                                             (Dependabot updates both)

Refused: a tag, a branch, a short or upper-case commit id, a docker://
image (by tag or by digest), a commit without the `# v<version>` comment,
an alias (*name) instead of the value, a value not on the `uses:` line,
and `uses:` in a flow mapping ({ ... }), which this line reader does not
parse. Comment lines and the contents of block scalars (`run: |`) are not
read.

Run with: python3 -I scripts/check_action_pins.py
(from anywhere in the repository). Exits 1 naming each `uses:` it refuses,
and when it finds no `uses:` at all, so a reading that lost every line
cannot pass.
"""

import pathlib
import re
import sys

# A `uses` key at the start of a line, after any "- " of a sequence item.
USES = re.compile(r"^(?P<lead>\s*(?:-\s+)*)uses\s*:(?:\s+(?P<rest>.*))?$")
# A key whose value is a block scalar (| or >, with optional indicators and
# a comment), or a sequence item that is one. Its contents are the lines
# indented further than the column of the key (or of the "-").
BLOCK = re.compile(r"^(?P<lead>\s*(?:-\s+)*)(?:[^\s#][^#]*?\s*:\s+)?[|>][-+0-9]*\s*(?:#.*)?$")
# `uses` as a key inside a flow mapping.
FLOW = re.compile(r"[{,]\s*uses\s*:")
# owner/repo, an optional path inside the repository, and an optional @ref
# (all of the rest, so a comment inside quotes shows as part of the ref).
ACTION = re.compile(r"^[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9._-]+(?:/[^@\s]+)?(?:@(?P<ref>.*))?$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
VERSION = re.compile(r"^v\d+(?:\.\d+)*(?:-[0-9A-Za-z.-]+)?(?:\s.*)?$")

NOT_ON_LINE = "the value is not on the uses: line: write owner/repo@<40-hex commit> # v<version> there"


def scan(text):
    """(line number, value, comment, unreadable) for each `uses:` key in
    `text`: the value and the comment after it (None if there is none), or
    why the value cannot be read, with value and comment None."""
    found = []
    block = None  # the column a block scalar's contents are indented past
    for n, line in enumerate(text.splitlines(), 1):
        if block is not None:
            if not line.strip() or len(line) - len(line.lstrip(" ")) > block:
                continue
            block = None
        if line.strip().startswith("#"):
            continue
        m = USES.match(line)
        if m:
            found.append((n, *value_of(m.group("rest"))))
            # A block scalar value is refused above; its lines are not keys.
            if BLOCK.match(line):
                block = len(m.group("lead"))
            continue
        m = BLOCK.match(line)
        if m:
            block = len(m.group("lead"))
            continue
        if FLOW.search(line):
            found.append((n, None, None, "uses: in a flow mapping: write it as a block key, one per line"))
    return found


def value_of(rest):
    """(value, comment, unreadable) of what follows `uses:`."""
    rest = (rest or "").strip()
    if rest.startswith("&"):
        anchored = re.match(r"&\S+\s+(.*)$", rest)
        rest = anchored.group(1) if anchored else ""
    if not rest or rest[0] in "#|>":
        return None, None, NOT_ON_LINE
    if rest[0] == "*":
        return None, None, "an alias: write the pin out where it is used"
    if rest[0] in "\"'":
        end = rest.find(rest[0], 1)
        if end < 0:
            return None, None, "no closing quote"
        value, tail = rest[1:end], rest[end + 1 :]
        if tail.strip() and not re.match(r"\s+#", tail):
            return None, None, "text after the closing quote that is not a comment"
        comment = tail.strip()[1:].strip() if tail.strip() else None
        return value, comment, None
    hash_mark = re.search(r"\s#", rest)
    if hash_mark is None:
        return rest, None, None
    return rest[: hash_mark.start()].strip(), rest[hash_mark.end() :].strip(), None


def is_action(root, value):
    """Whether the local path `value` (./...) names a reusable workflow
    file or an action directory inside `root`."""
    root = pathlib.Path(root).resolve()
    path = (root / value).resolve()
    if path != root and root not in path.parents:
        return False
    return path.is_file() or (path / "action.yml").is_file() or (path / "action.yaml").is_file()


def refusal(value, comment, root):
    """Why `value` with `comment` is not a pin this repository accepts, or
    None if it is one."""
    if value.startswith("docker://"):
        return "a docker:// image is not pinned by commit: use an action, owner/repo@<40-hex commit> # v<version>"
    if value.startswith("./"):
        if "@" in value:
            return "a local path takes no @ref"
        if not is_action(root, value):
            return f"{value} is not an action or workflow file in the repository"
        return None
    m = ACTION.match(value)
    if m is None:
        return f"{value} is not ./<path> or owner/repo[/path]@<40-hex commit>"
    ref = m.group("ref")
    if ref is None:
        return "no @<commit>: pin it as owner/repo@<40-hex commit> # v<version>"
    if not COMMIT.match(ref):
        return f"@{ref} is not a commit: pin the full 40-hex commit id, with # v<version>"
    if not comment or not VERSION.match(comment):
        return "no # v<version> comment after the commit, naming the tag it was taken at"
    return None


def problems(path, text, root):
    """One line per `uses:` in `text` (the file at `path`) that is refused."""
    out = []
    for n, value, comment, unreadable in scan(text):
        reason = unreadable or refusal(value, comment, root)
        if reason:
            out.append(f"{path}:{n}: {reason}")
    return out


def files(root):
    root = pathlib.Path(root)
    found = sorted((root / ".github/workflows").glob("*.yml"))
    found += sorted((root / ".github/workflows").glob("*.yaml"))
    found += sorted((root / ".github/actions").rglob("action.yml"))
    found += sorted((root / ".github/actions").rglob("action.yaml"))
    return found


def check(root):
    root = pathlib.Path(root)
    out, uses = [], 0
    for path in files(root):
        text = path.read_text(encoding="utf-8")
        uses += len(scan(text))
        out += problems(path.relative_to(root).as_posix(), text, root)
    if uses == 0:
        out.append("no uses: found in .github/workflows or .github/actions")
    return out


def report(root):
    """(exit status, lines to print) for the tree at `root`."""
    found = check(root)
    if found:
        return 1, [f"check_action_pins: {p}" for p in found]
    texts = [p.read_text(encoding="utf-8") for p in files(root)]
    return 0, [f"check_action_pins: ok ({sum(len(scan(t)) for t in texts)} uses in {len(texts)} files)"]


def main():
    status, lines = report(pathlib.Path(__file__).resolve().parent.parent)
    for line in lines:
        print(line, file=sys.stderr if status else sys.stdout)
    return status


if __name__ == "__main__":
    sys.exit(main())
