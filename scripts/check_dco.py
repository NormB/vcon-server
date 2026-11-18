"""Checks that every commit of a pull request carries a DCO sign-off.

The Developer Certificate of Origin (https://developercertificate.org/) is a
statement a contributor makes by adding a line to each commit message:

    Signed-off-by: Your Name <your GitHub noreply address>

`git commit -s` writes it from git's user.name and user.email. `verdict` is
the one rule. A commit passes when:

- its author is a GitHub Bot account (`author.type == "Bot"`) and GitHub
  verified its signature: Dependabot and similar accounts cannot make the
  statement, and GitHub names a commit's author from its email, which anyone
  can set, so the signature is what proves the commit came from the bot;
- or its author AND its committer are the repository owner's GitHub login,
  and GitHub verified its signature. The owner holds the rights already, so
  these commits need no statement; ~/Development/bin/cla-owner-exempt applies
  the same test to `license/cla`. No address of the owner is read or named;
- or its message has a line `Signed-off-by: Name <email>`, at the start of
  the line, with a name that is not empty and the commit's author email,
  compared without case, AND that email is a GitHub noreply address
  (scripts/scan_forbidden.py's GITHUB_NOREPLY). Any other address in a
  commit message is personal data, which the commit hook and CI's message
  scan refuse, so a sign-off can only name a noreply address.

A merge commit follows the same rule; the main branch requires linear
history, so one cannot land, but the rule still covers it.

Usage: check_dco.py --owner LOGIN FILE

FILE holds JSON arrays of GitHub commit objects written one after another:
.github/workflows/dco.yml writes one array per commit from
`gh api repos/OWNER/REPO/commits/SHA`, which needs only `contents: read`;
the pages of `gh api repos/OWNER/REPO/pulls/N/commits --paginate` have the
same shape.

Exit 0: every commit passes. 1: at least one does not; each is named with
the reason, and the fix is printed. 2: the input could not be read, held no
commit, or the owner login is missing; nothing was checked. The output
names an address only when it is a noreply address, since CI's log of a
pull request is public.

Usage: check_dco.py --warn-local MSGFILE AUTHOR

What .githooks/commit-msg runs. AUTHOR is an email or the output of
`git var GIT_AUTHOR_IDENT`. Prints `commit-msg: warning: ...` on standard
error when the message has no sign-off the `DCO` job would accept, and
nothing otherwise; always exits 0, since CI's `DCO` job decides. The hook
skips it when `git config dco.exempt` is true (MAINTAINERS.md).

Run with: python3 -I scripts/check_dco.py ...
"""

import importlib.util
import json
import pathlib
import re
import sys
from dataclasses import dataclass

# The noreply rule lives in scan_forbidden.py, which allows such an address
# on a sign-off line: one rule for both scripts.
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("scan_forbidden", pathlib.Path(__file__).resolve().parent / "scan_forbidden.py")
scan_forbidden = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(scan_forbidden)

# `Signed-off-by:` at the start of a line, a name, then `<email>` to the end
# of the line. The key compares without case.
_SIGN_OFF = re.compile(
    r"^signed-off-by:[ \t]*(?P<name>[^<\r\n]*?)[ \t]*<(?P<email>[^<>\s]*)>[ \t]*\r?$",
    re.IGNORECASE | re.MULTILINE,
)

FIX = (
    "fix: set git's user.email to your GitHub noreply address, then add the "
    "line with `git commit -s` for a new commit, or "
    "`git rebase --signoff origin/main` for every commit of the branch, and "
    "push again (CONTRIBUTING.md, \"Signing off commits\")"
)


@dataclass(frozen=True)
class Commit:
    """The fields of one pull request commit that the rule reads."""

    sha: str
    parents: int
    author_login: str | None
    author_type: str | None
    committer_login: str | None
    verified: bool
    author_name: str
    author_email: str
    message: str


def is_noreply(address):
    """Whether `address` is a GitHub noreply address."""
    return scan_forbidden.is_noreply(address)


def sign_offs(message):
    """Every (name, email) pair the message signs off with."""
    return [(m.group("name").strip(), m.group("email")) for m in _SIGN_OFF.finditer(message)]


def _same(a, b):
    return bool(a) and bool(b) and a.lower() == b.lower()


def _shown(address):
    """`address` for a public log: itself when it is a noreply address."""
    return f"<{address}>" if is_noreply(address) else "(not shown: not a noreply address)"


def verdict(commit, owner):
    """Whether `commit` passes, and why, under the rule in the module doc."""
    if commit.author_type == "Bot" and commit.verified:
        return True, f"bot account {commit.author_login}, signature verified"
    if _same(commit.author_login, owner) and _same(commit.committer_login, owner) and commit.verified:
        return True, f"owner {owner}, signature verified"
    found = sign_offs(commit.message)
    if not found:
        return False, "no Signed-off-by line"
    for name, email in found:
        if name and _same(email, commit.author_email):
            if not is_noreply(commit.author_email):
                return False, "the author email is not a GitHub noreply address"
            return True, f"signed off by {name}"
    return False, f"no Signed-off-by line names the author email {_shown(commit.author_email)} with a name"


def check(commits, owner):
    """The exit status and the lines to print for a whole pull request."""
    if not commits:
        return 2, ["DCO: no commit to check, so nothing was checked"]
    lines = []
    failed = 0
    for c in commits:
        ok, why = verdict(c, owner)
        lines.append(f"{'OK' if ok else 'FAIL'} {c.sha[:9]}: {why}")
        failed += not ok
    if failed:
        lines.append(FIX)
    lines.append(f"DCO: {len(commits)} commit(s), {failed} without a sign-off")
    return (1 if failed else 0), lines


def from_api(raw):
    """A `Commit` from one GitHub commit object."""
    author = raw.get("author") or {}
    committer = raw.get("committer") or {}
    body = raw["commit"]
    return Commit(
        sha=raw["sha"],
        parents=len(raw.get("parents") or []),
        author_login=author.get("login"),
        author_type=author.get("type"),
        committer_login=committer.get("login"),
        verified=bool((body.get("verification") or {}).get("verified", False)),
        author_name=body["author"]["name"],
        author_email=body["author"]["email"],
        message=body["message"],
    )


def read_pages(text):
    """The elements of one or more JSON arrays written one after another."""
    decoder = json.JSONDecoder()
    out = []
    i = 0
    while True:
        while i < len(text) and text[i].isspace():
            i += 1
        if i == len(text):
            return out
        page, i = decoder.raw_decode(text, i)
        if not isinstance(page, list):
            raise ValueError("a page is not a JSON array")
        out.extend(page)


def local_warning(message, author_email):
    """The commit-msg hook's warning for `message`, or None when the `DCO`
    job would accept its sign-off."""
    if not is_noreply(author_email):
        return (
            "your author email is not a GitHub noreply address, so a sign-off "
            "cannot name it: set `git config user.email` to the noreply address "
            "GitHub shows under Settings, Emails (CONTRIBUTING.md, \"Signing off commits\")"
        )
    if any(name and _same(email, author_email) for name, email in sign_offs(message)):
        return None
    return (
        f"no Signed-off-by line names your author email <{author_email}>; "
        "the pull request's DCO check will fail. Commit with `git commit -s`."
    )


def ident_email(ident):
    """The email of `git var GIT_AUTHOR_IDENT` output (`Name <email> time
    tz`); a string without `<...>` is taken as the email itself."""
    m = re.search(r"<([^<>]*)>", ident)
    return m.group(1) if m else ident.strip()


def warn_local(argv):
    """`--warn-local MSGFILE AUTHOR`: the warning on standard error; 0."""
    try:
        message = pathlib.Path(argv[0]).read_text(encoding="utf-8")
        author = argv[1]
    except (OSError, IndexError, UnicodeDecodeError) as error:
        print(f"commit-msg: DCO sign-off not checked: {error}", file=sys.stderr)
        return 0
    warning = local_warning(message, ident_email(author))
    if warning:
        print(f"commit-msg: warning: {warning}", file=sys.stderr)
    return 0


def main(argv):
    if argv[:1] == ["--warn-local"]:
        return warn_local(argv[1:])
    if len(argv) != 3 or argv[0] != "--owner" or not argv[1]:
        print("usage: check_dco.py --owner LOGIN FILE", file=sys.stderr)
        print("       check_dco.py --warn-local MSGFILE AUTHOR", file=sys.stderr)
        return 2
    owner, path = argv[1], argv[2]
    try:
        commits = [from_api(raw) for raw in read_pages(pathlib.Path(path).read_text(encoding="utf-8"))]
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"DCO: cannot read {path}: {error}; nothing was checked", file=sys.stderr)
        return 2
    rc, lines = check(commits, owner)
    for line in lines:
        print(line)
    return rc


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
