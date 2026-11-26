#!/usr/bin/env python3
"""The advisory `Contract` job (.github/workflows/contract.yml): compares
this tree's contract with the newest release tag.

    python3 -I scripts/contract_job.py

It picks the newest release tag (`vMAJOR.MINOR.PATCH`, compared as
numbers; a tag with a suffix such as `-rc1` is not a release), says which
one, and runs `scripts/contract_diff.py --against <tag>`, whose output and
exit status it keeps: 0 no breaking change, 1 at least one, 2 nothing was
decided. With no release tag it compares nothing and exits 0.

Releases up to 0.4.10 do not mark the problem `type` member of
openapi.json as an open list (`x-vcon-server-open-enum`). While the tag
compared lacks the mark, the job says so first: each problem type added
since then reads as a breaking change to the responses that can carry a
problem document, until a release carries the mark.

It reads the tags and the files at the tag with git only, from the clone
the job's checkout made, and calls no GitHub API.
"""

from __future__ import annotations

import importlib.util
import pathlib
import re
import sys

# No bytecode beside the scripts for the module loaded by path
# (scripts/test_no_bytecode.py).
sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
RELEASE = re.compile(r"v(\d+)\.(\d+)\.(\d+)")


def _contract_diff():
    spec = importlib.util.spec_from_file_location("contract_diff", HERE / "contract_diff.py")
    module = importlib.util.module_from_spec(spec)
    # A dataclass with postponed annotations finds its module in sys.modules.
    sys.modules["contract_diff"] = module
    spec.loader.exec_module(module)
    return module


cd = _contract_diff()


def newest_tag(names) -> str | None:
    """The newest release tag of `names` (one tag name each), or None."""
    best = None
    for name in names:
        m = RELEASE.fullmatch(name.strip())
        if m:
            key = tuple(int(p) for p in m.groups())
            if best is None or key > best[0]:
                best = (key, name.strip())
    return best[1] if best else None


def open_enum_note(tag: str, openapi_text: str | None) -> str | None:
    """The note the job prints when the openapi.json at `tag` (its text, or
    None when the tag has none) lacks the open-enum mark."""
    if openapi_text is None or cd.OPEN_ENUM in openapi_text:
        return None
    return (f"contract: note: {tag}'s openapi.json does not mark the problem `type` as an open list "
            f"({cd.OPEN_ENUM}), as releases up to 0.4.10 do not. Each problem type added since then "
            "is reported as breaking for every response that can carry a problem document, until the "
            "newest release tag carries the mark.")


def run(root, git) -> int:
    """The job on the work tree `root`, reading git through
    `git(args) -> (code, stdout)`. Returns the exit status."""
    rc, out = git(["tag", "--list", "v*"])
    if rc != 0:
        print("contract: git could not list the tags; nothing was decided", file=sys.stderr)
        return 2
    tag = newest_tag(out.splitlines())
    if tag is None:
        print("contract: no release tag (vMAJOR.MINOR.PATCH); nothing to compare")
        return 0
    print(f"contract: comparing the work tree with {tag}, the newest release tag")
    rc, text = git(["show", f"{tag}:{cd.OPENAPI_PATH}"])
    note = open_enum_note(tag, text if rc == 0 else None)
    if note:
        print(note)
    return cd.against(tag, root, git)


def main(argv: list[str]) -> int:
    if argv:
        print("usage: contract_job.py", file=sys.stderr)
        return 2
    root = HERE.parent
    return run(root, cd.git_runner(root))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
