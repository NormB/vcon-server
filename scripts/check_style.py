"""Checks the Rust style rules that rustfmt and clippy do not.

The rules are the Google Rust Style Guide's
(https://google.github.io/styleguide/rustguide.html):

- glob-import: no `use path::*`, except `use super::*` directly inside an
  inline module, `use Enum::*` inside a function, and `pub use sub::*` of a
  submodule declared in a non-leaf module. A prelude is a glob too.
- rustfmt-skip: `#[rustfmt::skip]` only with a `//` comment on the line
  above saying why (deliberate alignment or generated code).

Clippy checks the rest of what the hooks hold the code to (SAFETY
comments, assertion messages, unwrap); see [workspace.lints] in Cargo.toml.

Reads every tracked .rs file and prints each finding as path:line: rule:
message. Exits 1 if there is any. With --walk it reads every .rs file
below the current directory instead, outside target/: the commit hook runs
it in a copy of the index, which is not a repository.

Run with: python3 -I scripts/check_style.py [--walk]
"""

import dataclasses
import pathlib
import subprocess
import sys


@dataclasses.dataclass(frozen=True)
class Token:
    kind: str  # "ident", "punct" or "attr" (an attribute's text, whole)
    text: str
    line: int


@dataclasses.dataclass(frozen=True)
class Finding:
    line: int
    rule: str
    message: str


def _skip_string(text, i, line):
    """Index past the string starting at text[i] (the opening quote), and
    the line it ends on."""
    i += 1
    while i < len(text):
        c = text[i]
        if c == "\\":
            if text[i + 1 : i + 2] == "\n":
                line += 1
            i += 2
            continue
        if c == "\n":
            line += 1
        if c == '"':
            return i + 1, line
        i += 1
    return i, line


def _skip_raw_string(text, i, line):
    """Index past the raw string whose `r` is text[i], or None if it is not
    one."""
    j = i + 1
    hashes = 0
    while j < len(text) and text[j] == "#":
        hashes += 1
        j += 1
    if j >= len(text) or text[j] != '"':
        return None
    end = '"' + "#" * hashes
    k = text.find(end, j + 1)
    k = len(text) if k < 0 else k + len(end)
    return k, line + text.count("\n", i, k)


def tokens(text):
    """The identifiers and punctuation of Rust source `text`, without
    comments, strings or character literals."""
    out = []
    i = 0
    line = 1
    n = len(text)
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif c.isspace():
            i += 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
        elif text.startswith("/*", i):
            depth = 0
            while i < n:
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                    if depth == 0:
                        break
                else:
                    if text[i] == "\n":
                        line += 1
                    i += 1
        elif c == '"':
            i, line = _skip_string(text, i, line)
        elif c in "rbc" and (raw := _skip_raw_string(text, i + (c != "r" and text[i + 1 : i + 2] == "r"), line)):
            i, line = raw
        elif c in "bc" and text[i + 1 : i + 2] == '"':
            i, line = _skip_string(text, i + 1, line)
        elif c == "b" and text[i + 1 : i + 2] == "'":
            i = _skip_char(text, i + 1)
        elif c == "'":
            # A character literal, or a lifetime or label ('a, 'static).
            if text[i + 1 : i + 2] == "\\" or text[i + 2 : i + 3] == "'":
                i = _skip_char(text, i)
            else:
                j = i + 1
                while j < n and (text[j].isalnum() or text[j] == "_"):
                    j += 1
                i = j
        elif c == "#" and (text.startswith("#[", i) or text.startswith("#![", i)):
            start = i
            i = text.index("[", i)
            depth = 0
            start_line = line
            while i < n:
                if text[i] == "[":
                    depth += 1
                elif text[i] == "]":
                    depth -= 1
                    if depth == 0:
                        i += 1
                        break
                elif text[i] == "\n":
                    line += 1
                elif text[i] == '"':
                    i, line = _skip_string(text, i, line)
                    continue
                i += 1
            out.append(Token("attr", "".join(text[start:i].split()), start_line))
        elif c.isalpha() or c == "_":
            j = i
            while j < n and (text[j].isalnum() or text[j] == "_"):
                j += 1
            out.append(Token("ident", text[i:j], line))
            i = j
        elif text.startswith("::", i):
            out.append(Token("punct", "::", line))
            i += 2
        else:
            out.append(Token("punct", c, line))
            i += 1
    return out


def _skip_char(text, i):
    """Index past the character literal whose opening quote is text[i]."""
    j = i + 1
    if text[j] == "\\":
        j += 2
    else:
        j += 1
    k = text.find("'", j)
    return len(text) if k < 0 else k + 1


def _block_kind(header):
    """What a `{` opens, from the tokens since the previous `;`, `{` or `}`."""
    words = [t.text for t in header if t.kind == "ident"]
    if "fn" in words:
        return "fn"
    if "mod" in words:
        return "mod"
    return "other"


def _check_use(use_tokens, line, stack, is_pub, nonleaf, submodules):
    """A finding for the `use` whose tokens (after `use`, before `;`) are
    `use_tokens`, or None."""
    if not any(t.text == "*" for t in use_tokens):
        return None
    path = [t.text for t in use_tokens]
    # A single path ending in ::*, with no braces.
    simple = "{" not in path and path[-1] == "*"
    segments = [p for p in path[:-1] if p != "::"] if simple else []
    shown = "".join(path)
    if "prelude" in segments:
        return Finding(line, "glob-import", f"`use {shown}`: import what is used by name, not a prelude")
    if simple:
        if segments == ["super"] and stack and stack[-1] == "mod" and not is_pub:
            return None
        if "fn" in stack and segments and segments[-1][:1].isupper() and not is_pub:
            return None
        if is_pub and not stack and nonleaf:
            name = segments[1] if segments[:1] == ["self"] and len(segments) == 2 else (
                segments[0] if len(segments) == 1 else None)
            if name in submodules:
                return None
    return Finding(line, "glob-import", f"`use {shown}`: import the names used, not a glob")


# The tokens of a visibility: pub, pub(crate), pub(super), pub(in path).
VISIBILITY = {"pub", "(", ")", "crate", "super", "self", "in", "::"}


def findings(text, nonleaf=False):
    """The style findings in Rust source `text`. `nonleaf` says whether the
    file is a non-leaf module (a crate root, a mod.rs, or a file with a
    directory of submodules beside it)."""
    toks = tokens(text)
    out = []
    submodules = {
        toks[k + 1].text
        for k in range(len(toks) - 2)
        if toks[k].text == "mod" and toks[k + 1].kind == "ident" and toks[k + 2].text == ";"
    }
    lines = text.split("\n")
    stack = []
    header = []
    k = 0
    while k < len(toks):
        t = toks[k]
        if t.kind == "attr" and t.text in ("#[rustfmt::skip]", "#![rustfmt::skip]") or (
            t.kind == "attr" and t.text.startswith(("#[rustfmt::skip::", "#![rustfmt::skip::"))
        ):
            above = lines[t.line - 2].strip() if t.line >= 2 else ""
            if not (above.startswith("//") and not above.startswith(("///", "//!"))):
                out.append(Finding(t.line, "rustfmt-skip", "say why on the line above: `// ...`"))
        words = [h.text for h in header if h.kind != "attr"]
        # A use item starts a statement, after nothing but a visibility;
        # `use<'a>` in an `impl Trait` bound is not one.
        if t.kind == "ident" and t.text == "use" and set(words) <= VISIBILITY:
            is_pub = bool(words)
            end = k + 1
            while end < len(toks) and toks[end].text != ";":
                end += 1
            found = _check_use(toks[k + 1 : end], t.line, stack, is_pub, nonleaf, submodules)
            if found:
                out.append(found)
            k = end + 1
            header = []
            continue
        if t.text == "{" and t.kind == "punct":
            stack.append(_block_kind(header))
            header = []
        elif t.text == "}" and t.kind == "punct":
            if stack:
                stack.pop()
            header = []
        elif t.text == ";" and t.kind == "punct":
            header = []
        else:
            header.append(t)
        k += 1
    return out


def is_nonleaf(path):
    """Whether the .rs file at `path` is a non-leaf module."""
    path = pathlib.Path(path)
    if path.name in ("lib.rs", "main.rs", "mod.rs"):
        return True
    return path.with_suffix("").is_dir()


def sources(walk):
    """The .rs files to read: git's tracked ones, or with `walk` every one
    below the current directory outside build output (target/)."""
    if walk:
        return sorted(
            p.as_posix()
            for p in pathlib.Path(".").rglob("*.rs")
            if "target" not in p.parts
        )
    listed = subprocess.run(
        ["git", "ls-files", "-z", "--", "*.rs"], capture_output=True, check=True
    ).stdout.decode()
    return sorted(p for p in listed.split("\0") if p)


def main(argv):
    if argv not in ([], ["--walk"]):
        print("usage: check_style.py [--walk]", file=sys.stderr)
        return 2
    total = 0
    for name in sources(walk=argv == ["--walk"]):
        text = pathlib.Path(name).read_text(encoding="utf-8")
        for f in findings(text, nonleaf=is_nonleaf(name)):
            print(f"{name}:{f.line}: {f.rule}: {f.message}", file=sys.stderr)
            total += 1
    if total:
        print(f"check_style: {total} finding(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
