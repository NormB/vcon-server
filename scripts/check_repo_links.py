"""Checks that every file or directory .bestpractices.json links to on the
main branch exists in the tree, so moving or renaming a file cannot leave
an answer pointing at nothing.

Run: python3 -I scripts/check_repo_links.py
"""

import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
LINK = re.compile(r"https://github\.com/NormB/vcon-server/(?:blob|tree)/main/([^\s\"'<>()#?]+)")
# Prose punctuation that can follow a link without being part of it.
TRAILING = ".,:;!"


def linked_paths(text):
    """The repository paths, in order of appearance, that `text` links to
    on main."""
    return [m.group(1).rstrip(TRAILING) for m in LINK.finditer(text)]


def missing(text, root):
    """The linked paths that are not a file or directory inside `root`."""
    root = root.resolve()
    gone = []
    for path in linked_paths(text):
        target = (root / path).resolve()
        if not target.is_relative_to(root) or not target.exists():
            gone.append(path)
    return gone


def main():
    root = HERE.parent
    text = (root / ".bestpractices.json").read_text()
    gone = missing(text, root)
    for path in gone:
        print(f".bestpractices.json: links to {path}, which is not in the tree")
    if gone:
        return 1
    print(f"check-repo-links: ok ({len(set(linked_paths(text)))} paths)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
