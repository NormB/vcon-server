"""Checks the shape of the entries of CHANGELOG.md.

A changelog is read by someone deciding whether to upgrade and what to do
after. An entry is an outline, not prose:

    - **Lead: what changed, in the user's terms.**
      - Effect: what the user sees or can now do.
      - Do: what the user must do, with the command or setting.
      - Limit: one limit or caveat.
      - Docs: [page title](link to the page that holds the detail).

and a Fixed entry says what went wrong before, under `Was:`.

The rule, for every release section from the top of the file down to the
section of MARKER (the sections below it are in the older prose form and
are not judged; the marker moves down as they are rewritten):

- A section holds `### ` headings from HEADINGS and, before the first of
  them, an optional paragraph. Under a heading there are entries only.
- An entry is a top-level bullet whose whole text is a bold lead of at
  most LEAD_WORDS words.
- It has 1 to SUB_BULLETS sub-bullets, or one table of at most TABLE_ROWS
  rows in place of them (an entry about sizes or limits).
- A sub-bullet starts with one of LABELS, each used once, and is at most
  BULLET_WORDS words; `Effect:` or `Was:` is there.
- No third level, and no paragraph under a bullet.
- No line of an entry is longer than WRAP characters, but a line that is
  one word (a link's address) cannot be wrapped and passes.

A link counts as the words a reader sees: `[two words](address)` is two.

Run with: python3 -I scripts/check_changelog.py
Exits 1 naming the section, the entry's lead and the rule for each problem.
"""

import pathlib
import re
import sys

MARKER = "0.4.5"
HEADINGS = ("Added", "Changed", "Fixed", "Security", "Removed", "Deprecated")
LABELS = ("Effect", "Do", "Limit", "Docs", "Was")
LEAD_WORDS = 15
BULLET_WORDS = 25
SUB_BULLETS = 5
TABLE_ROWS = 6
WRAP = 80

LINK = re.compile(r"\[([^\]]*)\]\([^)]*\)")
LABELED = re.compile(r"^(\w+): ")


def words(text):
    """How many words a reader sees: a link is its text."""
    return len(LINK.sub(r"\1", text).split())


def sections(text):
    """(heading, lines) for each release section, in the file's order."""
    found = []
    for line in text.split("\n"):
        if line.startswith("## ["):
            found.append((line, []))
        elif found:
            found[-1][1].append(line)
    return found


def judged(text, marker):
    """The sections from the top down to the marker's, that one included."""
    kept = []
    for heading, lines in sections(text):
        kept.append((heading, lines))
        if heading.startswith(f"## [{marker}]"):
            return kept
    raise ValueError(f"the changelog has no section [{marker}], the marker of the sections that are judged")


def too_long(line):
    """Whether a line is over the wrap and could be wrapped."""
    if len(line) <= WRAP:
        return False
    rest = re.sub(r"^- (\w+: )?", "", line.strip())
    return len(rest.split()) > 1


def entry_problems(lines):
    """(name, [rules broken]) for one entry: its lines, the first being the
    top-level bullet, with no trailing blank line."""
    found = []
    own = [lines[0][2:]]
    at = 1
    while at < len(lines) and lines[at].startswith("  ") and not lines[at].startswith("  - ") and not lines[at].startswith("  |") and lines[at].strip():
        if lines[at].startswith("    "):
            break
        own.append(lines[at].strip())
        at += 1
    own = " ".join(own)
    lead = re.match(r"\*\*(.+?)\*\*", own)
    if not lead:
        return lines[0][2:].strip(), ["the entry does not start with a bold lead (`- **Lead.**`)"]
    name = lead.group(1)
    if own != lead.group(0):
        found.append("the lead is the whole bullet: what follows it goes into sub-bullets")
    if words(name) > LEAD_WORDS:
        found.append(f"the lead has {words(name)} words, at most {LEAD_WORDS}")

    bullets = []
    table = []
    after_blank = False
    paragraph = third = False
    for line in lines[at:]:
        if not line.strip():
            after_blank = True
            continue
        if line.startswith("  - "):
            bullets.append(line[4:].strip())
        elif re.match(r"^ {4,}- ", line):
            third = True
        elif line.startswith("  |"):
            table.append(line.strip())
        elif line.startswith("    ") and bullets and not after_blank:
            bullets[-1] += " " + line.strip()
        else:
            paragraph = True
        after_blank = False
    if third:
        found.append("a third level of nesting")
    if paragraph:
        found.append("a paragraph under the bullet: an entry is a lead and sub-bullets")

    rows = [row for row in table[1:] if not re.match(r"^\|[\s|:-]+\|$", row)]
    if table and bullets:
        found.append("the entry has a table and sub-bullets; a table stands in place of them")
    elif table:
        if len(rows) > TABLE_ROWS:
            found.append(f"the table has {len(rows)} rows, at most {TABLE_ROWS}")
    elif not bullets:
        found.append("the entry has no sub-bullet and no table")
    if len(bullets) > SUB_BULLETS:
        found.append(f"the entry has {len(bullets)} sub-bullets, at most {SUB_BULLETS}")

    used = []
    for bullet in bullets:
        label = LABELED.match(bullet)
        if not label or label.group(1) not in LABELS:
            allowed = ", ".join(f"{name}:" for name in LABELS)
            found.append(f"the sub-bullet `{bullet[:40]}` does not start with one of {allowed}")
            continue
        label = label.group(1)
        if label in used:
            found.append(f"the label {label}: is used twice")
        used.append(label)
        count = words(bullet[len(label) + 2:])
        if count > BULLET_WORDS:
            found.append(f"the {label}: sub-bullet has {count} words, at most {BULLET_WORDS}")
    if bullets and not table and "Effect" not in used and "Was" not in used:
        found.append("the entry has neither Effect: nor Was:")

    for line in lines:
        if too_long(line):
            found.append(f"a line of {len(line)} characters, at most {WRAP}: `{line.strip()[:40]}`")
    return name, found


def section_problems(heading, lines):
    """One line per problem of a section."""
    label = heading[3:].split("]")[0] + "]"
    found = []
    under_heading = False
    entry = []

    def close():
        while entry and not entry[-1].strip():
            entry.pop()
        if entry:
            name, rules = entry_problems(entry)
            found.extend(f"{label}: `{name}`: {rule}" for rule in rules)
        entry.clear()

    for line in lines:
        if line.startswith("### "):
            close()
            under_heading = True
            if line[4:].strip() not in HEADINGS:
                found.append(f"{label}: `{line}` is not one of {', '.join(HEADINGS)}")
        elif line.startswith("- "):
            close()
            entry.append(line)
        elif entry and (line.startswith(" ") or not line.strip()):
            entry.append(line)
        elif line.strip():
            close()
            if under_heading:
                found.append(f"{label}: `{line.strip()[:40]}`: text outside an entry, under a heading")
    close()
    return found


def problems(text, marker):
    """One line per problem of the sections that are judged."""
    found = []
    for heading, lines in judged(text, marker):
        found.extend(section_problems(heading, lines))
    return found


def main(root):
    path = root / "CHANGELOG.md"
    try:
        found = problems(path.read_text(encoding="utf-8"), MARKER)
    except (OSError, ValueError) as error:
        print(f"check_changelog: {error}", file=sys.stderr)
        return 1
    for problem in found:
        print(f"CHANGELOG.md: {problem}", file=sys.stderr)
    if found:
        print(
            f"check_changelog: {len(found)} problem(s); the format is in website/content/docs/development.md "
            "(Changelog entries)",
            file=sys.stderr,
        )
        return 1
    print(f"check_changelog: ok, every entry from the top to [{MARKER}] has the outline form")
    return 0


if __name__ == "__main__":
    sys.exit(main(pathlib.Path(__file__).resolve().parent.parent))
