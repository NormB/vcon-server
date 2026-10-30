"""Checks that every download of a toolchain or tool retries.

A runner's network fails now and then: on 2026-10-28 main's "fuzz config"
job failed because rustup could not fetch a nightly's channel file once,
while the same job's curl step, which retries, never has. So in every
workflow, composite action and shell script:

  rustup toolchain install / component add / target add   through scripts/retry.sh
  npm ci / npm install                                      through scripts/retry.sh
  npx playwright install (browsers and their packages)      through scripts/retry.sh
  apt-get update / install                                  with -o Acquire::Retries=N
  curl (as a command) fetching from another host            with --retry

  dnf / yum / microdnf install                             retried, with dnf clean metadata between tries

dnf's own retries cover a failed download, not a mirror that serves an
empty repomd.xml (a parse error, seen on 2026-10-28 in Release's
almalinux:9 job), so a retry must start again from fresh metadata.
actions/setup-* download through the actions toolkit, which retries.

Run with: python3 -I scripts/check_workflow_downloads.py
(from anywhere in the repository). Exits 1 naming each step that does not
retry.
"""

import pathlib
import re
import sys

RUSTUP = re.compile(r"\brustup\s+(toolchain\s+install|component\s+add|target\s+add)\b")
NPM = re.compile(r"\bnpm\s+(ci|install)\b")
PLAYWRIGHT = re.compile(r"\bplaywright\s+install\b")
DNF = re.compile(r"\b(?:dnf|yum|microdnf)\s+(?:-\S+\s+)*install\b")
APT = re.compile(r"\bapt-get\s+(?:-\S+\s+(?:\S+\s+)?)*(update|install)\b")
# curl as a command: at the start of a command, after a separator, or
# after "$(", and followed by an option.
CURL = re.compile(r"(?:^|[;&|(]\s*|\$\(\s*)curl\s+-")
# A download names its source: an http(s) URL on another host. A curl
# that probes a server the script started itself (localhost, 127.0.0.1,
# or a URL held in a variable) is a readiness check, not a download.
REMOTE_URL = re.compile(r"https?://(?!localhost\b|127\.0\.0\.1\b|\[::1\])[A-Za-z0-9.-]+\.[A-Za-z]{2,}")


def logical_lines(text):
    """(first line number, text) for each line, with backslash
    continuations joined, so a command's options are read whole."""
    out, buf, start = [], "", None
    for n, line in enumerate(text.splitlines(), 1):
        if start is None:
            start = n
        if line.rstrip().endswith("\\"):
            buf += line.rstrip()[:-1] + " "
            continue
        out.append((start, buf + line))
        buf, start = "", None
    if start is not None:
        out.append((start, buf))
    return out


def unretried(path, text):
    """One line per download step in `text` that does not retry."""
    problems = []
    for n, line in logical_lines(text):
        code = line.strip()
        if code.startswith("#"):
            continue
        # A YAML key that only names a step is prose, not a command.
        if re.match(r"-?\s*name:", code):
            continue
        for rule, label in ((RUSTUP, "rustup"), (NPM, "npm"), (PLAYWRIGHT, "playwright install")):
            m = rule.search(code)
            if m and "retry.sh" not in code[: m.start()]:
                problems.append(f"{path}:{n}: {label} without scripts/retry.sh")
        for m in DNF.finditer(code):
            # A retry that starts again from fresh metadata: scripts/retry.sh
            # with RETRY_BETWEEN cleaning it, or (before the checkout, where
            # no script exists yet) an inline loop that cleans it.
            before = code[: m.start()]
            through_script = "retry.sh" in before and "RETRY_BETWEEN" in before and "dnf clean metadata" in before
            inline_loop = re.search(r"\bfor\b", before) is not None and "break" in code and "dnf clean metadata" in code
            if not (through_script or inline_loop):
                problems.append(f"{path}:{n}: dnf without a retry that cleans its metadata")
        for m in APT.finditer(code):
            if "Acquire::Retries" not in m.group(0):
                problems.append(f"{path}:{n}: apt-get without -o Acquire::Retries")
        if CURL.search(code) and "--retry" not in code and REMOTE_URL.search(code):
            problems.append(f"{path}:{n}: curl without --retry")
    return problems


def files(root):
    root = pathlib.Path(root)
    found = sorted((root / ".github/workflows").glob("*.y*ml"))
    found += sorted((root / ".github/actions").glob("*/action.y*ml"))
    found += sorted((root / "scripts").glob("*.sh"))
    return found


def check(root):
    root = pathlib.Path(root)
    problems = []
    for path in files(root):
        problems += unretried(path.relative_to(root).as_posix(), path.read_text(encoding="utf-8"))
    return problems


def main():
    root = pathlib.Path(__file__).resolve().parent.parent
    problems = check(root)
    for problem in problems:
        print(f"check_workflow_downloads: {problem}", file=sys.stderr)
    if problems:
        return 1
    print(f"check_workflow_downloads: ok ({len(files(root))} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
