"""Checks the Rust style rules that rustfmt and clippy do not.

The rules are the Google Rust Style Guide's
(https://google.github.io/styleguide/rustguide.html):

- glob-import: no `use path::*`, except `use super::*` directly inside an
  inline module, `use Enum::*` inside a function, and `pub use sub::*` of a
  submodule declared in a non-leaf module. A prelude is a glob too.
- rustfmt-skip: `#[rustfmt::skip]` only with a `//` comment on the line
  above saying why (deliberate alignment or generated code).
- assert-message: every assertion (assert!, assert_eq!, assert_ne!, their
  debug_ and prop_ forms) carries a message saying why it holds. Clippy's
  missing_assert_message does not look at test code; this does. Commas
  inside a turbofish (`f::<A, B>()`) at the macro's top level are miscounted
  as arguments; write such a call outside the assertion.

- lint-table: every package holds the lints of [workspace.lints] in
  Cargo.toml, at their level or stricter: with `[lints] workspace = true`,
  or in a table of its own (vcon's adds unsafe_code = "forbid"). The fuzz
  package, which has no public API, may leave out the documentation lints.

Assertions without a message are being added file by file. Until that is
done, style/assert-message-baseline.txt holds, per file, how many each
may still have: a file over its count, or a file not listed with any, is
refused, and a file under its count must have the count lowered
(--write-baseline does it, and never raises a count).

Clippy checks the rest of what the hooks hold the code to (SAFETY
comments, assertion messages, unwrap); see [workspace.lints] in Cargo.toml.

Reads every tracked .rs file and prints each finding as path:line: rule:
message. Exits 1 if there is any. With --walk it reads every .rs file
below the current directory instead, outside target/: the commit hook runs
it in a copy of the index, which is not a repository.

Run with: python3 -I scripts/check_style.py [--walk] [--write-baseline]
"""

import dataclasses
import pathlib
import subprocess
import sys
import tomllib


@dataclasses.dataclass(frozen=True)
class Token:
    kind: str  # "ident", "punct", "literal" (a string or character) or "attr"
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
    """The identifiers, punctuation, attributes and literals of Rust source
    `text`, without comments. A string or character literal is one token
    with no text, so nothing inside it is read as code."""
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
            out.append(Token("literal", "", line))
            i, line = _skip_string(text, i, line)
        elif c in "rbc" and (raw := _skip_raw_string(text, i + (c != "r" and text[i + 1 : i + 2] == "r"), line)):
            out.append(Token("literal", "", line))
            i, line = raw
        elif c in "bc" and text[i + 1 : i + 2] == '"':
            out.append(Token("literal", "", line))
            i, line = _skip_string(text, i + 1, line)
        elif c == "b" and text[i + 1 : i + 2] == "'":
            out.append(Token("literal", "", line))
            i = _skip_char(text, i + 1)
        elif c == "'":
            # A character literal, or a lifetime or label ('a, 'static).
            if text[i + 1 : i + 2] == "\\" or text[i + 2 : i + 3] == "'":
                out.append(Token("literal", "", line))
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


# Each assertion macro and how many arguments it has with a message.
ASSERTIONS = {
    "assert": 2,
    "debug_assert": 2,
    "prop_assert": 2,
    "assert_eq": 3,
    "assert_ne": 3,
    "debug_assert_eq": 3,
    "debug_assert_ne": 3,
    "prop_assert_eq": 3,
    "prop_assert_ne": 3,
}

BASELINE = pathlib.Path("style/assert-message-baseline.txt")


def _arguments(toks, open_at):
    """How many comma-separated arguments the macro call whose opening
    bracket is toks[open_at] has at its top level."""
    depth = 0
    count = 0
    seen = False
    for t in toks[open_at:]:
        if t.kind != "punct":
            if depth == 1:
                seen = True
            continue
        if t.text in "([{":
            depth += 1
            if depth > 1:
                seen = True
        elif t.text in ")]}":
            depth -= 1
            if depth == 0:
                return count + (1 if seen else 0)
        elif t.text == "," and depth == 1:
            if seen:
                count += 1
            seen = False
        elif depth == 1:
            seen = True
    return count + (1 if seen else 0)


def ratchet(counts, baseline):
    """The problems with assertions without a message: `counts` per file
    now (every file read, 0 included) against `baseline`, the counts each
    file may still have. An entry for a file that was not read is stale."""
    problems = []
    for name in sorted(set(counts) | set(baseline)):
        if name not in counts:
            problems.append(
                f"{name}: in the baseline but not in the tree; "
                "drop the entry with check_style.py --write-baseline"
            )
            continue
        now = counts[name]
        allowed = baseline.get(name, 0)
        if now > allowed:
            problems.append(
                f"{name}: {now} assertion(s) without a message, {allowed} allowed; give each a message"
            )
        elif now < allowed:
            problems.append(
                f"{name}: {now} assertion(s) without a message, the baseline says {allowed}; "
                "lower it with check_style.py --write-baseline"
            )
    return problems


def next_baseline(counts, baseline):
    """The baseline after this tree: each file's count, never above what
    it was, and no entry for a file with none."""
    out = {}
    for name, allowed in baseline.items():
        now = min(counts.get(name, 0), allowed)
        if now:
            out[name] = now
    return out


def format_baseline(baseline):
    lines = [
        "# Assertions without a message each file may still have, written by",
        "# scripts/check_style.py --write-baseline. A count only goes down.",
    ]
    lines += [f"{n}\t{name}" for name, n in sorted(baseline.items())]
    return "\n".join(lines) + "\n"


def parse_baseline(text):
    out = {}
    for line in text.splitlines():
        if not line.strip() or line.startswith("#"):
            continue
        n, name = line.split("\t")
        out[name] = int(n)
    return out


# Lint levels, weakest first.
LEVELS = ["allow", "warn", "deny", "forbid"]

# The workspace lints that concern documentation and a public API, which the
# fuzz package (no public API) leaves out.
API_LINTS = {
    "missing_docs",
    "missing_debug_implementations",
    "missing_errors_doc",
    "missing_panics_doc",
    "must_use_candidate",
    "exhaustive_enums",
    "exhaustive_structs",
}


def _level(value):
    return value["level"] if isinstance(value, dict) else value


def lint_table_problems(workspace, name, lints, omit=()):
    """What the [lints] table `lints` of package `name` lacks of the
    workspace's `workspace` lints: every one, at its level or stricter,
    unless the package takes them with `workspace = true` or the lint is in
    `omit`."""
    if lints.get("workspace") is True:
        return []
    problems = []
    for group, table in sorted(workspace.items()):
        for lint, value in sorted(table.items()):
            if lint in omit:
                continue
            want = _level(value)
            have = lints.get(group, {}).get(lint)
            if have is None or LEVELS.index(_level(have)) < LEVELS.index(want):
                problems.append(f"{name}: [lints.{group}] {lint} must be {want} or stricter, as in [workspace.lints]")
    return problems


def lint_tables(root):
    """The lint-table problems of every package under `root`: the
    workspace's members, and the fuzz package, which leaves out API_LINTS."""
    root = pathlib.Path(root)
    if not (root / "Cargo.toml").exists():
        return []  # Not a Cargo tree: there are no packages to compare.
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    workspace = manifest["workspace"].get("lints", {})
    packages = [(m, ()) for m in manifest["workspace"]["members"]]
    if (root / "fuzz" / "Cargo.toml").exists():
        packages.append(("fuzz", API_LINTS))
    problems = []
    for path, omit in packages:
        member = tomllib.loads((root / path / "Cargo.toml").read_text(encoding="utf-8"))
        problems += lint_table_problems(workspace, f"{path}/Cargo.toml", member.get("lints", {}), omit)
    return problems


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
        if (
            t.kind == "ident"
            and t.text in ASSERTIONS
            and k + 2 < len(toks)
            and toks[k + 1].text == "!"
            and toks[k + 2].text in ("(", "[", "{")
            and _arguments(toks, k + 2) < ASSERTIONS[t.text]
        ):
            out.append(Finding(t.line, "assert-message", f"`{t.text}!` without a message saying why it holds"))
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
    flags = set(argv)
    if not flags <= {"--walk", "--write-baseline"} or len(flags) != len(argv):
        print("usage: check_style.py [--walk] [--write-baseline]", file=sys.stderr)
        return 2
    baseline = parse_baseline(BASELINE.read_text(encoding="utf-8")) if BASELINE.exists() else {}
    total = 0
    counts = {}
    for name in sources(walk="--walk" in flags):
        text = pathlib.Path(name).read_text(encoding="utf-8")
        counts[name] = 0
        for f in findings(text, nonleaf=is_nonleaf(name)):
            if f.rule == "assert-message":
                counts[name] += 1
                continue
            print(f"{name}:{f.line}: {f.rule}: {f.message}", file=sys.stderr)
            total += 1
    if "--write-baseline" in flags:
        BASELINE.write_text(format_baseline(next_baseline(counts, baseline)), encoding="utf-8")
        baseline = parse_baseline(BASELINE.read_text(encoding="utf-8"))
    for problem in ratchet(counts, baseline):
        print(f"assert-message: {problem}", file=sys.stderr)
        total += 1
    for problem in lint_tables("."):
        print(f"lint-table: {problem}", file=sys.stderr)
        total += 1
    if total:
        print(f"check_style: {total} finding(s)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
