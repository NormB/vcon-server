"""Checks that a commit leaves the released part of CHANGELOG.md as its base
has it.

git merges CHANGELOG.md line by line and knows nothing of its headings. On
2026-11-26 a rebase of a branch that added an [Unreleased] entry onto the
0.4.18 release merged without a conflict and put the entry under
`## [0.4.18] - 2026-11-26`: the release's text then named a change 0.4.18
does not hold. scripts/check_changelog.py judges the shape of each entry,
not where it is, and passed the file.

The released part is the text from the first `## [x.y.z]` heading down to
the link definitions at the end of the file, and those definitions but the
one of [Unreleased]. The rule: the released part of a commit ends with the
released part of its base, byte for byte, and holds each of the base's
definitions. A release commit, which turns [Unreleased] into a new section
on top and adds that section's definition, passes. Anything else is
refused, but a deliberate edit of released text: that commit writes the
sha256 of the released part it leaves (the digest the refusal prints) to
PIN, and its commit message names PIN. A pin that moves is always the
digest of the released part beside it; a pin that does not move excuses
nothing. A release made after such an edit, before it reaches main, writes
the pin again.

The bases:
- index (the commit hook): the index against HEAD, the commit it is built
  on; against MERGE_HEAD while a merge is open; and against the merge base
  of HEAD and origin/main. A rebase runs no commit hook, and after it the
  index is HEAD; the merge base with origin/main, which the rebase moved
  the branch onto, is the base that sees what it changed.
- commits SHA... (the push hook on the outgoing commits, CI on the commits
  of a push or pull request): each commit against each of its parents, and
  its message names PIN when the pin differs from its first parent's.
- message FILE (the commit-msg hook): the message names PIN when the
  index's pin differs from HEAD's. A first commit, like a commit without
  a parent, has no base and moves no pin.

Run with: python3 -I scripts/check_changelog_released.py index
          python3 -I scripts/check_changelog_released.py commits <sha>...
          python3 -I scripts/check_changelog_released.py message <message file>
Exit status: 0 no problem; 1 a problem, each named on stderr with the line,
the heading it is under and the fix; 2 a usage error or a commit git
cannot read.

Tested by scripts/test_check_changelog_released.py.
"""

import collections
import difflib
import hashlib
import re
import subprocess
import sys

FILE = "CHANGELOG.md"
PIN = ".config/changelog-released.sha256"
RELEASE_HEADING = re.compile(r"## \[[0-9]")
DEFINITION = re.compile(r"\[([^\]\s][^\]]*)\]: \S+")
# How many differing lines a refusal shows, and how much of each.
SHOWN = 6
WIDTH = 120
NAME = "check_changelog_released"
USAGE = f"usage: {NAME}.py index | commits <sha>... | message <message file>"

Released = collections.namedtuple("Released", "start body definitions")
Released.__doc__ = """The released part of a changelog: `start`, the index of
its first line in the file; `body`, its lines (each with its line ending)
down to the link definitions at the end; `definitions`, those definition
lines but [Unreleased]'s."""


def lines_of(text):
    """The lines of text, each with its line ending; only \\n ends a line."""
    return re.findall(r"[^\n]*\n|[^\n]+$", text)


def is_definition(line):
    return DEFINITION.fullmatch(line.rstrip("\r\n")) is not None


def released(text):
    """The released part of a changelog text (None: no changelog)."""
    lines = lines_of(text or "")
    first = next((i for i, line in enumerate(lines) if RELEASE_HEADING.match(line)), len(lines))
    # The definitions are the run of definition and blank lines at the end;
    # the blank lines before the first of them stay in the body.
    end = len(lines)
    at = len(lines)
    while at > first and (is_definition(lines[at - 1]) or not lines[at - 1].strip()):
        at -= 1
        if is_definition(lines[at]):
            end = at
    definitions = [
        line for line in lines[end:]
        if is_definition(line) and DEFINITION.fullmatch(line.rstrip("\r\n")).group(1) != "Unreleased"
    ]
    return Released(first, lines[first:end], definitions)


def digest(text):
    """The sha256 of the released part of a changelog text: the value a
    deliberate edit writes to PIN."""
    part = released(text)
    return hashlib.sha256("".join(part.body + part.definitions).encode("utf-8")).hexdigest()


def pin_of(text):
    """The digest a pin file holds: the first word of its first line that is
    neither blank nor a `#` comment; empty without one."""
    for line in (text or "").splitlines():
        words = line.split()
        if words and not words[0].startswith("#"):
            return words[0]
    return ""


def shown(line):
    text = line.rstrip("\r\n")
    return f"`{text[:WIDTH]}`" if text.strip() else "(a blank line)"


def heading_at(part, at):
    """The release heading the body line `at` of part is under."""
    return next((line.rstrip("\r\n") for line in reversed(part.body[:at + 1]) if RELEASE_HEADING.match(line)), "")


def differences(ours, base):
    """One line for each line of ours's released part the base's does not
    hold, and for each of the base's that ours does not."""
    added, gone = [], []
    matcher = difflib.SequenceMatcher(None, base.body, ours.body, autojunk=False)
    for tag, i1, i2, j1, j2 in matcher.get_opcodes():
        if tag in ("replace", "insert"):
            added += [f"line {ours.start + j + 1}, under `{heading_at(ours, j)}`: {shown(ours.body[j])}"
                      for j in range(j1, j2)]
        if tag in ("replace", "delete"):
            gone += [f"not here: the base's line {base.start + i + 1}, under `{heading_at(base, i)}`: "
                     f"{shown(base.body[i])}" for i in range(i1, i2)]
    missing = collections.Counter(base.definitions) - collections.Counter(ours.definitions)
    gone += [f"not here: the base's definition {shown(line)}" for line in missing.elements()]
    found = []
    for group in (added, gone):
        found += group[:SHOWN]
        if len(group) > SHOWN:
            found.append(f"and {len(group) - SHOWN} more")
    return found


def problems(ours, base, ours_pin, base_pin, label):
    """The problem, if any, of the changelog text ours (None: none) with the
    pin ours_pin, against the base's text and pin; label names the base."""
    if base is None:
        return []
    mine, theirs = released(ours), released(base)
    tail = mine.body[len(mine.body) - len(theirs.body):] if theirs.body else []
    kept = tail == theirs.body and not (collections.Counter(theirs.definitions) - collections.Counter(mine.definitions))
    value = digest(ours)
    moved = ours_pin != base_pin
    found = []
    if moved and ours_pin != value:
        found.append(f"{PIN} holds {ours_pin or 'no digest'}, not that of the base, and it is not the sha256 of "
                     f"the released part, {value}: a pin that moves is the digest of the released part it leaves; "
                     f"write {value} to {PIN}, or put the base's pin back.")
    elif kept or moved:
        return []
    if not kept:
        lines = "".join(f"\n  {line}" for line in differences(mine, theirs))
        found.append(
            f"the released part of {FILE} (from the first `## [x.y.z]` heading down) is not the end of the one of "
            f"{label}:{lines}\n  Fix: a merge or a rebase put an [Unreleased] entry into a released section: move "
            "it back to [Unreleased], under its `###` heading, until the released part is the base's byte for "
            f"byte. A deliberate edit of released text writes {value} to {PIN} and names {PIN} in the commit "
            "message."
        )
    return [" ".join(found) if len(found) > 1 else found[0]]


def message_problems(message, pin_moved):
    """The problem, if any, of a commit message: a commit that moves the pin
    names PIN in a line that is not a `#` comment."""
    if not pin_moved:
        return []
    if any(PIN in line for line in message.splitlines() if not line.startswith("#")):
        return []
    return [f"the commit moves the pin in {PIN} and its message does not name {PIN}: a deliberate edit of "
            "released text says so in its message."]


def git(*arguments):
    """git's output (bytes as UTF-8), or None when git refuses."""
    try:
        run = subprocess.run(["git", *arguments], capture_output=True, check=False, timeout=60)
    except (OSError, subprocess.SubprocessError):
        return None
    return run.stdout.decode("utf-8", "replace") if run.returncode == 0 else None


def commit_of(rev):
    """The full name of a commit, or an empty string."""
    return (git("rev-parse", "-q", "--verify", f"{rev}^{{commit}}") or "").strip()


def report(found):
    for problem in found:
        print(f"{NAME}: refused: {problem}", file=sys.stderr)
    if found:
        print(f"{NAME}: {len(found)} problem(s); the rule is in website/content/docs/development.md "
              "(Changelog entries)", file=sys.stderr)
        return 1
    return 0


def check_index():
    ours, ours_pin = git("show", f":{FILE}"), pin_of(git("show", f":{PIN}"))
    head = commit_of("HEAD")
    if not head:
        print(f"{NAME}: ok, no HEAD: the first commit has no released part to keep")
        return 0
    bases = [(head, "HEAD, the commit this one is built on")]
    merging = commit_of("MERGE_HEAD")
    if merging:
        bases.append((merging, "MERGE_HEAD, the commit being merged"))
    note = ""
    if commit_of("origin/main"):
        main_base = (git("merge-base", "HEAD", "origin/main") or "").strip()
        if main_base:
            bases.append((main_base, "the merge base of HEAD and origin/main"))
    else:
        note = "; no origin/main, so not against its merge base"
    found, seen = [], set()
    for sha, why in bases:
        if sha in seen:
            continue
        seen.add(sha)
        found += problems(ours, git("show", f"{sha}:{FILE}"), ours_pin, pin_of(git("show", f"{sha}:{PIN}")),
                          f"{sha[:12]} ({why})")
    if report(found):
        return 1
    print(f"{NAME}: ok, the index keeps the released part of {len(seen)} base(s){note}")
    return 0


def check_commits(shas):
    found = []
    for rev in shas:
        sha = commit_of(rev)
        listed = git("rev-list", "--parents", "-n", "1", sha) if sha else None
        if listed is None:
            print(f"{NAME}: git cannot read the commit {rev}", file=sys.stderr)
            return 2
        parents = listed.split()[1:]
        ours, ours_pin = git("show", f"{sha}:{FILE}"), pin_of(git("show", f"{sha}:{PIN}"))
        for parent in parents:
            found += problems(ours, git("show", f"{parent}:{FILE}"), ours_pin,
                              pin_of(git("show", f"{parent}:{PIN}")), f"{parent[:12]}, parent of {sha[:12]}")
        if parents:
            moved = ours_pin != pin_of(git("show", f"{parents[0]}:{PIN}"))
            found += [f"{sha[:12]}: {p}" for p in message_problems(git("log", "-1", "--format=%B", sha) or "", moved)]
    if report(found):
        return 1
    print(f"{NAME}: ok, {len(shas)} commit(s) keep the released part of each parent")
    return 0


def check_message(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as file:
            message = file.read()
    except OSError as error:
        print(f"{NAME}: {path}: {error.strerror}", file=sys.stderr)
        return 2
    # A first commit has no base whose released text it could change.
    moved = bool(commit_of("HEAD")) and pin_of(git("show", f":{PIN}")) != pin_of(git("show", f"HEAD:{PIN}"))
    return report(message_problems(message, moved))


def main(argv):
    if argv[:1] == ["index"] and len(argv) == 1:
        return check_index()
    if argv[:1] == ["commits"] and len(argv) > 1:
        return check_commits(argv[1:])
    if argv[:1] == ["message"] and len(argv) == 2:
        return check_message(argv[1])
    print(USAGE, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
