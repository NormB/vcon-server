#!/usr/bin/env python3
"""Runs the commands the install and how-to pages show, in a fresh system,
and checks that each prints what the page says it prints.

A page is read top to bottom. Each ```sh block is a step, run in one bash
shell, so a variable a step sets is there for the next, as for a reader who
pastes the steps in order. A ```text block that follows a step, before the
next heading or block, is what that step prints: each line must match, with
`<words>` standing for any text on its line and a line holding only `…`
standing for any number of lines. A step must exit 0 unless its marker says
otherwise. A failing command inside a step stops the run there.

An HTML comment on the line before a block changes how it runs:

    <!-- docs-test: only apt release -->   run only in scenarios with every tag
    <!-- docs-test: skip <reason> -->      never run; the reason is required,
                                           and may not name a release
    <!-- docs-test: since 0.3.0 -->        run only when the page's VERSION is
                                           that release or a later one; the
                                           text before it must name 0.3.0
    <!-- docs-test: exit 1 -->             the step must exit 1
    <!-- docs-test: background serving --> run in the background, and wait
                                           until it prints "serving"
    <!-- docs-test: file <path> -->        a block (TOML, a unit) the page says
                                           to save as <path>; written there
    <!-- docs-test: always -->             run at the end, even after a failure

Markers combine with ";", such as `only systemd; exit 1`.

A scenario names the pages it runs, in order, where (a container image, a
container with systemd as its init, or this host), and its tags. The
container scenarios get the binary under test as /docs-test/vcon-server.

What the runner does around the pages, so that an outage elsewhere does not
fail a page whose commands are right:

- A GitHub release asset a step downloads (a vcon-server release, cosign,
  RustFS, MinIO, DuckDB) comes through the runner's mirror, an HTTP server
  on Docker's bridge address. The step's https://github.com/<owner>/<repo>/
  releases/download/ URL is rewritten to the mirror's, and the mirror fetches
  that exact github.com URL with scripts/fetch-pinned.sh: checked against its
  sha256 in scripts/docs-test-pins.txt, retried through an outage for up to
  five minutes, and kept in a cache (DOCS_TEST_PINNED, which CI restores and
  saves on main). A URL the release does not have still fails the step: the
  mirror answers 404, as github.com does. On 2026-11-09 github.com answered
  504 for most of an hour, and the release, upgrade and container scenarios
  failed at those downloads.
- Each image is pinned by digest (IMAGES) and pulled with retries; an image
  a page builds FROM is pulled the same way, then tagged with the name the
  page uses.
- In a container with apt, apt retries each download (Acquire::Retries).

Usage:
  python3 -I scripts/docs_test.py list               the scenario names, as JSON
  python3 -I scripts/docs_test.py lint               the rules every tested page keeps
  python3 -I scripts/docs_test.py run <scenario> [--binary <vcon-server>] [--keep <dir>]
"""

from __future__ import annotations

import dataclasses
import ipaddress
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOCS = os.path.join(ROOT, "website", "content", "docs")


class PageError(Exception):
    """A page that breaks a rule of this runner, with where."""


@dataclasses.dataclass
class Block:
    kind: str  # "sh", or "file" for a block saved as a file
    code: str
    at: str  # page:line of the opening fence
    expect: str | None = None
    only: frozenset = frozenset()
    skip: str | None = None
    exit: int = 0
    background: str | None = None
    path: str | None = None
    always: bool = False
    since: str | None = None
    # The page's text from the last block or heading to this block: what a
    # reader sees just before it.
    prose: str = ""


MARKER = re.compile(r"^<!--\s*docs-test:\s*(.*?)\s*-->\s*$")
FENCE = re.compile(r"^```\s*([A-Za-z0-9_+-]*)")
RELEASE = re.compile(r"^\d+\.\d+\.\d+$")


def parse_marker(text: str, at: str) -> dict:
    out: dict = {}
    for part in [p.strip() for p in text.split(";") if p.strip()]:
        word, _, rest = part.partition(" ")
        rest = rest.strip()
        if word == "only":
            if not rest:
                raise PageError(f"{at}: `only` needs a tag")
            out["only"] = frozenset(rest.split())
        elif word == "skip":
            if not rest:
                raise PageError(f"{at}: a skip needs a reason")
            out["skip"] = rest
        elif word == "exit":
            if not rest.isdigit():
                raise PageError(f"{at}: `exit` needs a number")
            out["exit"] = int(rest)
        elif word == "background":
            if not rest:
                raise PageError(f"{at}: `background` needs the text that says it is ready")
            out["background"] = rest
        elif word == "file":
            if not rest or " " in rest:
                raise PageError(f"{at}: `file` needs one path")
            out["path"] = rest
        elif word == "always":
            out["always"] = True
        elif word == "since":
            if not RELEASE.match(rest):
                raise PageError(f"{at}: `since` needs a release version such as 0.3.0")
            out["since"] = rest
        else:
            raise PageError(f"{at}: unknown docs-test marker `{word}`")
    return out


def parse(text: str, name: str) -> list[Block]:
    """The steps of one page, in order."""
    lines = text.splitlines()
    blocks: list[Block] = []
    marker: dict | None = None
    marker_at = ""
    # The last step, while a text block after it may still be its output.
    open_step: Block | None = None
    prose: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        at = f"{name}:{i + 1}"
        m = MARKER.match(line.strip())
        if m:
            if marker is not None:
                raise PageError(f"{marker_at}: a docs-test marker with no block")
            marker, marker_at = parse_marker(m.group(1), at), at
            i += 1
            continue
        f = FENCE.match(line.lstrip())
        if f:
            lang = f.group(1)
            # A block inside a list item is indented; its lines lose that indent.
            indent = len(line) - len(line.lstrip())
            body: list[str] = []
            i += 1
            while i < len(lines) and not lines[i].lstrip().startswith("```"):
                text = lines[i]
                body.append(text[indent:] if text[:indent].strip() == "" else text.lstrip())
                i += 1
            i += 1  # the closing fence
            code = "\n".join(body) + "\n"
            given = marker or {}
            marker = None
            before, prose = "\n".join(prose), []
            if lang == "text" and open_step is not None and not given:
                open_step.expect = code
                open_step = None
                continue
            open_step = None
            if lang in ("sh", "bash"):
                if "path" in given:
                    raise PageError(f"{at}: `file` is for blocks that are not commands")
                block = Block(kind="sh", code=code, at=at, prose=before, **given)
            elif "path" in given:
                block = Block(kind="file", code=code, at=at, prose=before, **given)
            elif given:
                raise PageError(f"{at}: a docs-test marker on a {lang or 'plain'} block without `file`")
            else:
                continue
            blocks.append(block)
            if block.kind == "sh" and block.skip is None:
                open_step = block
            continue
        if marker is not None and line.strip():
            raise PageError(f"{marker_at}: a docs-test marker with no block")
        if line.startswith("#"):
            open_step = None
            prose = []
        prose.append(line)
        i += 1
    if marker is not None:
        raise PageError(f"{marker_at}: a docs-test marker with no block")
    return blocks


PLACEHOLDER = re.compile(r"<[a-z][a-z0-9 ,./'-]*>")
ANY_LINES = "…"


def _clean(text: str) -> list[str]:
    lines = [l.rstrip() for l in text.replace("\r", "").split("\n")]
    while lines and not lines[0]:
        lines.pop(0)
    while lines and not lines[-1]:
        lines.pop()
    return lines


def _line_pattern(line: str) -> str:
    out, last = [], 0
    for m in PLACEHOLDER.finditer(line):
        out.append(re.escape(line[last : m.start()]))
        out.append(r".+?")
        last = m.end()
    out.append(re.escape(line[last:]))
    return "".join(out)


def mismatch(want: str, got: str) -> str | None:
    """None when `got` is what `want` describes; otherwise why not."""
    want_lines, got_lines = _clean(want), _clean(got)
    pattern = "".join(
        r"(?:[^\n]*\n)*?" if l == ANY_LINES else _line_pattern(l) + r"\n" for l in want_lines
    )
    actual = "".join(l + "\n" for l in got_lines)
    if re.fullmatch(pattern, actual):
        return None
    # The first line that differs, for the report.
    for n, (w, g) in enumerate(zip(want_lines, got_lines), start=1):
        if w == ANY_LINES:
            return "it printed something else after a `…` line"
        if not re.fullmatch(_line_pattern(w), g):
            return f"line {n}: the page says {w!r}, it printed {g!r}"
    if len(got_lines) < len(want_lines):
        return f"the page shows {len(want_lines)} lines, it printed {len(got_lines)}"
    return f"it printed {len(got_lines) - len(want_lines)} more lines than the page shows"


@dataclasses.dataclass(frozen=True)
class Scenario:
    name: str
    where: str  # an image, "systemd" (a container with systemd as init), or "host"
    tags: frozenset
    pages: tuple
    sets: dict = dataclasses.field(default_factory=dict)
    binary: bool = True  # gets the binary under test, as /docs-test/vcon-server
    install_binary: bool = True  # and installs it before the pages run
    timeout: int = 900
    docker: tuple = ()  # extra `docker run` options


# Each image a scenario runs in, or a page builds FROM, by the digest of its
# multi-platform index (amd64 and arm64 both), read with
# `docker buildx imagetools inspect <image>` on 2026-11-09. A tag is a name
# its owner moves; a digest is not. The development page says how to bump one.
IMAGES = {
    "debian:trixie-slim": "sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2",
    "fedora:44": "sha256:15da97ed9aa94dd3d92c3925eb89520f6c80e07ea7ee24cba848d1f8b6912b3e",
    "ubuntu:24.04": "sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852",
}


def pinned_image(name: str) -> str:
    """name@digest for an image IMAGES pins."""
    if name not in IMAGES:
        raise PageError(f"no digest pinned for {name} in IMAGES (scripts/docs_test.py)")
    return f"{name}@{IMAGES[name]}"


# The image of the systemd scenario: Debian with systemd as its init, and
# nothing else the pages would install.
SYSTEMD_IMAGE = "docs-test-systemd:trixie"
SYSTEMD_DOCKERFILE = f"""FROM {pinned_image("debian:trixie-slim")}
RUN apt-get update -qq -o Acquire::Retries=10 \\
 && apt-get install -y -qq -o Acquire::Retries=10 --no-install-recommends systemd systemd-sysv dbus >/dev/null \\
 && rm -rf /var/lib/apt/lists/*
STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
"""
SYSTEMD_RUN = (
    "--privileged",
    "--cgroupns=host",
    "-v",
    "/sys/fs/cgroup:/sys/fs/cgroup:rw",
    "--tmpfs",
    "/run",
    "--tmpfs",
    "/run/lock",
)
# A container's memory-lock limit is a ceiling its root cannot raise, unlike
# a host's. With CAP_SYS_RESOURCE it can, so the limits file the pages install
# raises it, as on a host, and a page that leaves that file out fails here too.
# CAP_IPC_LOCK lets root lock memory past its limit, as root can on a host.
AS_A_HOST = ("--cap-add", "SYS_RESOURCE", "--cap-add", "IPC_LOCK")

HOWTO = (
    "getting-started.md",
    "first-vcon.md",
    "keys-and-scopes.md",
    "second-tenant.md",
    "backup-and-restore.md",
    "rotate-keys.md",
    "connect-mcp.md",
    "upgrading.md",
)
# docs/upgrading.md installs the binary it names as NEW: the one under test.
UNDER_TEST = {"NEW": "/docs-test/vcon-server"}
# The release before this one, which the upgrade scenario upgrades from.
PREVIOUS = "0.3.0"

SCENARIOS = [
    # The how-tos, on a Debian and a Fedora with no systemd: the server runs
    # in the foreground.
    Scenario("foreground", "debian:trixie-slim", frozenset({"debian", "apt", "no-systemd"}), HOWTO, UNDER_TEST, docker=AS_A_HOST),
    Scenario("foreground-fedora", "fedora:44", frozenset({"dnf", "no-systemd"}), HOWTO, UNDER_TEST, docker=AS_A_HOST),
    # The same, under systemd. No memory-lock option: the unit and the
    # limits file are what allow it, as on a host.
    Scenario("systemd", "systemd", frozenset({"debian", "apt", "systemd"}), HOWTO, UNDER_TEST),
    # An upgrade from the previous release, installed as the install page
    # says, to the server under test, and back.
    Scenario(
        "upgrade",
        "debian:trixie-slim",
        frozenset({"debian", "apt", "no-systemd", "release"}),
        ("install.md", "getting-started.md", "upgrading.md"),
        {"VERSION": PREVIOUS, **UNDER_TEST},
        install_binary=False,
        docker=AS_A_HOST,
    ),
    # Objects in an S3 bucket: RustFS, or MinIO, and the server, each under
    # systemd, in one container, after the server is set up. The RustFS run
    # also exports to a bucket of its own (export-parquet.md), then turns
    # Parquet storage on over the server's bucket (parquet-storage.md).
    Scenario("s3", "systemd", frozenset({"debian", "apt", "systemd", "rustfs"}), ("getting-started.md", "s3-storage.md", "export-parquet.md", "parquet-storage.md")),
    Scenario("s3-minio", "systemd", frozenset({"debian", "apt", "systemd", "minio"}), ("getting-started.md", "s3-storage.md")),
    # The Parquet export to a directory, read with DuckDB, after the server
    # is set up with its objects in the data directory.
    Scenario("parquet", "systemd", frozenset({"debian", "apt", "systemd"}), ("getting-started.md", "export-parquet.md")),
    # The container how-to, with this host's Docker: it builds an image from
    # the release the page names, and removes what it made at the end.
    Scenario("container", "host", frozenset({"docker"}), ("container.md",), binary=False),
    # Each install path, from a system with nothing installed.
    Scenario("release-debian", "debian:trixie-slim", frozenset({"debian", "apt", "release"}), ("install.md",), binary=False),
    Scenario("release-ubuntu", "ubuntu:24.04", frozenset({"ubuntu", "apt", "release"}), ("install.md",), binary=False),
    Scenario("release-fedora", "fedora:44", frozenset({"dnf", "release"}), ("install.md",), binary=False),
    Scenario("cargo-debian", "debian:trixie-slim", frozenset({"debian", "apt", "cargo"}), ("install.md",), binary=False, timeout=3600),
    Scenario("cargo-fedora", "fedora:44", frozenset({"dnf", "cargo"}), ("install.md",), binary=False, timeout=3600),
    Scenario("source-debian", "debian:trixie-slim", frozenset({"debian", "apt", "source"}), ("install.md",), binary=False, timeout=3600),
    Scenario("source-fedora", "fedora:44", frozenset({"dnf", "source"}), ("install.md",), binary=False, timeout=3600),
]


def select(blocks: list[Block], scenario: Scenario) -> list[Block]:
    """The blocks a scenario runs, in page order."""
    return [b for b in blocks if b.skip is None and b.only <= scenario.tags]


def _release(version: str) -> tuple[int, ...]:
    return tuple(int(n) for n in version.split("."))


def release_steps(steps: list[Block], sets: dict) -> tuple[list[Block], list[Block]]:
    """The steps that run on the release the scenario runs with, and the
    `since` steps left out because that release is older than theirs. The
    release is the scenario's VERSION (set, or put there by release_sets),
    else the one the page sets."""
    if not any(b.since for b in steps):
        return list(steps), []
    version = sets.get("VERSION")
    if version is None:
        for b in steps:
            m = re.search(r"(?m)^VERSION=(\S+)$", b.code) if b.kind == "sh" else None
            if m:
                version = m.group(1)
                break
    kept, left_out = [], []
    for b in steps:
        if b.since is None:
            kept.append(b)
            continue
        if version is None or not RELEASE.match(version):
            raise PageError(f"{b.at}: a step from release {b.since} on, and no VERSION that names a release")
        (kept if _release(version) >= _release(b.since) else left_out).append(b)
    return kept, left_out


def _apply_sets(steps: list[Block], sets: dict) -> list[Block]:
    """The steps with each line that sets a scenario's variable rewritten to
    the scenario's value. Exactly one line may set it: a second one, perhaps
    on another page for another purpose, would be rewritten too."""
    out, done = [], set()
    for b in steps:
        code = b.code
        if b.kind == "sh":
            for name, value in sets.items():
                code, n = re.subn(rf"(?m)^{re.escape(name)}=.*$", f"{name}={value}", code)
                if n and (name in done or n > 1):
                    raise PageError(f"{b.at}: {name} is set again, and the scenario sets it")
                if n:
                    done.add(name)
        out.append(dataclasses.replace(b, code=code))
    for name in sets:
        if name not in done:
            raise PageError(f"no page block sets {name}, which the scenario sets")
    return out


def _quote(s: str) -> str:
    return "'" + s.replace("'", "'\\''") + "'"


def _step(n: int, b: Block) -> str:
    out = f'"$DOCS_TEST_OUT/{n}.out"'
    rc = f'"$DOCS_TEST_OUT/{n}.rc"'
    head = f"# {n}: {b.at}\n"
    if b.kind == "file":
        writer = f"sudo tee {_quote(b.path)} > /dev/null" if b.path.startswith("/") else f"cat > {_quote(b.path)}"
        return head + (
            f"set -eo pipefail\n{writer} <<'DOCS_TEST_EOF'\n{b.code}DOCS_TEST_EOF\n"
            f"set +eo pipefail\n: > {out}; echo 0 > {rc}\n"
        )
    if b.background is not None:
        ready = _quote(b.background)
        return head + (
            f"set +e\n( {b.code.rstrip()}\n) > {out} 2>&1 < /dev/null &\n"
            f"for _ in $(seq 600); do grep -qsF -- {ready} {out} && break; sleep 0.1; done\n"
            f"grep -qsF -- {ready} {out} || {{ echo never > {rc}; exit 101; }}\n"
            f"echo 0 > {rc}\n"
        )
    if b.exit != 0:
        return head + (
            f"set +e\n{{\n{b.code}}} > {out} 2>&1 < /dev/null\necho $? > {rc}\n"
            f'[ "$(cat {rc})" = {b.exit} ] || exit 100\n'
        )
    return head + (
        f"set -eo pipefail\n{{\n{b.code}}} > {out} 2>&1 < /dev/null\n"
        f"set +eo pipefail\necho 0 > {rc}\n"
    )


# Run before the first step of each page after the first: a reader comes
# back to a later page in another shell and directory, so a page that needs
# a file another page left in the current directory fails here too.
NEW_PAGE_DIR = 'cd "$(mktemp -d "$DOCS_TEST_WORK/page.XXXXXX")" || exit 102\n'


def _page(b: Block) -> str:
    return b.at.rsplit(":", 1)[0]


def script(steps: list[Block], prelude: str, sets: dict | None = None) -> str:
    """The bash script that runs the steps and records each one's output."""
    if sets is None:
        sets = {}
    steps = _apply_sets(steps, sets) if sets else steps
    main = [s for s in steps if not s.always]
    last = [s for s in steps if s.always]
    parts = [
        "#!/usr/bin/env bash\n",
        "# Written by scripts/docs_test.py from the pages; each step is one block.\n",
        "set -u\n",
        ': "${DOCS_TEST_OUT:?}" "${DOCS_TEST_WORK:?}"\n',
        "trap 'rc=$?; set +e; docs_test_always; echo $rc > \"$DOCS_TEST_OUT/exit\"' EXIT\n",
        "docs_test_always() {\n:\n",
    ]
    index = {id(s): n for n, s in enumerate(steps)}
    for s in last:
        parts.append(_step(index[id(s)], s).replace("set -eo pipefail\n", "set +e\n"))
    parts.append("}\n")
    parts.append('cd "$DOCS_TEST_WORK"\n')
    if prelude:
        parts.append("# The runner's own setup, not a page's.\nset -eo pipefail\n" + prelude + "set +eo pipefail\n")
    page = _page(main[0]) if main else None
    for s in main:
        if _page(s) != page:
            page = _page(s)
            parts.append(f"# {page} starts in a fresh directory.\n" + NEW_PAGE_DIR)
        parts.append(_step(index[id(s)], s))
    return "".join(parts)


def page_blocks(name: str) -> list[Block]:
    path = os.path.join(DOCS, name)
    with open(path, encoding="utf-8") as f:
        return parse(f.read(), f"docs/{name}")


PINS_NAME = "scripts/docs-test-pins.txt"
PINS_PATH = os.path.join(ROOT, PINS_NAME)
FETCH_PINNED = os.path.join(ROOT, "scripts", "fetch-pinned.sh")
GITHUB = "https://github.com"
# A pin names one release asset exactly.
RELEASE_ASSET_URL = re.compile(r"^https://github\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+/releases/download/[^/\s]+/[^/\s]+$")
# A release asset URL on a page, with the shell expansions it holds.
# A $( ... ) expansion is taken whole, spaces and all.
PAGE_RELEASE_URL = re.compile(r"https://github\.com/[^/\s\"']+/[^/\s\"']+/releases/download/(?:\$\([^)]*\)|[^\s\"'])+")
RELEASE_PREFIX = re.compile(r"https://github\.com/(?=[^/\s\"']+/[^/\s\"']+/releases/download/)")
EXPANSION = re.compile(r"\$\([^)]*\)|\$\{[^}]*\}|\$[A-Za-z_][A-Za-z0-9_]*")
HEX64 = re.compile(r"(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])")


def load_pins(text: str, where: str = PINS_NAME) -> dict:
    """URL -> sha256 from lines of `<sha256>  <url>`; # starts a comment."""
    pins: dict = {}
    for n, line in enumerate(text.splitlines(), 1):
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        parts = s.split()
        if len(parts) != 2 or not re.fullmatch(r"[0-9a-f]{64}", parts[0]):
            raise PageError(f"{where}:{n}: a pin is a sha256 of 64 lowercase hex digits, spaces, and a URL: {s!r}")
        sha, url = parts
        if not RELEASE_ASSET_URL.match(url):
            raise PageError(f"{where}:{n}: {url} is not a GitHub release asset")
        if url in pins:
            raise PageError(f"{where}:{n}: {url} is pinned twice")
        pins[url] = sha
    return pins


def read_pins() -> dict:
    with open(PINS_PATH, encoding="utf-8") as f:
        return load_pins(f.read())


def rewrite_releases(code: str, base: str) -> str:
    """The code with each GitHub release asset URL sent to the mirror at base."""
    return RELEASE_PREFIX.sub(base.rstrip("/") + "/", code)


def _template(url: str):
    """A page URL as a pattern: each shell expansion stands for one path part."""
    out, at = [], 0
    for m in EXPANSION.finditer(url):
        out.append(re.escape(url[at:m.start()]))
        out.append("[^/]+")
        at = m.end()
    out.append(re.escape(url[at:]))
    return re.compile("^" + "".join(out) + "$")


def pin_problems(blocks: list[Block], pins: dict) -> list[str]:
    """Each release asset a block downloads has a pin, and each sha256 written
    in such a block is one of the pins: the page and the pins name one file."""
    problems = []
    shas = set(pins.values())
    for b in blocks:
        urls = PAGE_RELEASE_URL.findall(b.code)
        if not urls:
            continue
        for url in urls:
            pattern = _template(url)
            if not any(pattern.match(p) for p in pins):
                problems.append(f"{b.at}: no pin in {PINS_NAME} matches {url}")
        for h in HEX64.findall(b.code):
            if h not in shas:
                problems.append(f"{b.at}: sha256 {h} on the page is not in {PINS_NAME}; the page and the pins name one file")
    return problems


def release_versions(steps: list[Block], sets: dict) -> dict:
    """The release each release variable names in this run: the scenario's,
    else the page's."""
    out = {}
    for name in RELEASE_VARIABLES:
        if name in sets:
            out[name] = sets[name]
            continue
        for b in steps:
            m = re.search(rf"(?m)^{name}=(\S+)$", b.code) if b.kind == "sh" else None
            if m:
                out[name] = m.group(1)
                break
    return out


def release_pin_problems(versions: dict, pins: dict) -> list[str]:
    """A vcon-server release a run downloads has its assets pinned."""
    out = []
    for name in RELEASE_VARIABLES:
        v = versions.get(name)
        if v is None or not RELEASE.match(v):
            continue
        if f"{GITHUB}/{REPO}/releases/download/v{v}/SHA256SUMS.txt" not in pins:
            out.append(f'{name}={v} has no pins in {PINS_NAME}; add its assets as the development page says under "Pinned tool downloads"')
    return out


def fetch_pinned_with(extra: list):
    """A fetch for the mirror: scripts/fetch-pinned.sh with extra options.
    Its messages go to this process's standard error, so the job's log shows
    every retry."""
    def fetch(url: str, sha: str, dest: str) -> int:
        cmd = ["bash", FETCH_PINNED, "--url", url, "--sha256", sha, "--dest", dest, *extra]
        out = subprocess.run(cmd, capture_output=True, text=True)
        sys.stderr.write(out.stdout + out.stderr)
        return out.returncode
    return fetch


class Mirror:
    """An HTTP server that serves the pinned release assets the pages
    download: each GET of /<owner>/<repo>/releases/download/<tag>/<file> is
    fetched from upstream + that path with scripts/fetch-pinned.sh into the
    cache, then served. A path with no pin answers 404 and is never fetched.
    fetch-pinned.sh's exit status maps to an answer: 0 the file, 4 (a
    permanent refusal) 404, 3 (bytes not the pin) 502, else 504."""

    PATH = re.compile(r"^/([A-Za-z0-9._-]+)/([A-Za-z0-9._-]+)/releases/download/([^/]+)/([^/]+)$")
    ANSWER = {4: 404, 3: 502}

    def __init__(self, pins: dict, cache: str, host: str, upstream: str = GITHUB, fetch=None):
        self.pins, self.cache, self.host, self.upstream = pins, cache, host, upstream.rstrip("/")
        self.fetch = fetch or fetch_pinned_with([])
        self.httpd = None
        self.locks: dict = {}
        self.lock = threading.Lock()

    def answer(self, path: str) -> tuple[int, bytes]:
        path = path.split("?", 1)[0]
        if any(part in (".", "..") for part in path.split("/")):
            return 400, b"docs-test mirror: a path may not hold . or .. parts\n"
        m = self.PATH.match(path)
        if not m:
            return 404, b"docs-test mirror: not a GitHub release asset\n"
        url = GITHUB + path
        sha = self.pins.get(url)
        if sha is None:
            return 404, f"docs-test mirror: no pin for {url} in {PINS_NAME}\n".encode()
        dest = os.path.join(self.cache, *m.groups())
        with self.lock:
            lock = self.locks.setdefault(url, threading.Lock())
        with lock:
            rc = self.fetch(self.upstream + path, sha, dest)
            if rc == 0:
                with open(dest, "rb") as f:
                    return 200, f.read()
        print(f"docs-test: the mirror could not fetch {url}: scripts/fetch-pinned.sh exited {rc}", file=sys.stderr)
        return self.ANSWER.get(rc, 504), f"docs-test mirror: {url}: scripts/fetch-pinned.sh exited {rc}\n".encode()

    def start(self) -> str:
        import http.server
        outer = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                status, body = outer.answer(self.path)
                self.send_response(status)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.httpd = http.server.ThreadingHTTPServer((self.host, 0), Handler)
        self.httpd.daemon_threads = True
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        return f"http://{self.host}:{self.httpd.server_address[1]}"

    def stop(self) -> None:
        if self.httpd is not None:
            self.httpd.shutdown()
            self.httpd.server_close()
            self.httpd = None


def mirror_host(inspect_output: str) -> str:
    """The address `docker network inspect bridge` reports for its gateway:
    the host as the containers, and a build's RUN steps, reach it."""
    s = inspect_output.strip()
    try:
        ipaddress.ip_address(s)
    except ValueError:
        raise PageError(f"docker reports no address for its bridge network (got {s!r}); the mirror needs one") from None
    return s


FROM_LINE = re.compile(r"(?im)^\s*FROM\s+(\S+)(?:\s+AS\s+(\S+))?")


def base_images(blocks: list[Block]) -> list[str]:
    """Each image a file block (a Dockerfile) builds FROM, once, in order;
    not an earlier stage of the same file."""
    out = []
    for b in blocks:
        if b.kind != "file":
            continue
        stages = set()
        for m in FROM_LINE.finditer(b.code):
            image, stage = m.group(1), m.group(2)
            if image not in stages and image not in out:
                out.append(image)
            if stage:
                stages.add(stage)
    return out


# The waits between pulls: as scripts/fetch-pinned.sh's, 255 s in all.
PULL_DELAYS = (5, 10, 20, 40, 60, 60, 60)


def _docker_quiet(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["docker", *args], capture_output=True, text=True, timeout=600)


def pull_with_retry(name: str, run=_docker_quiet, sleep=time.sleep) -> str:
    """Pull an image by its pinned digest, retrying a registry that fails,
    then tag it with its name, so a page that builds FROM the name gets
    the pinned image. Returns name@digest."""
    ref = pinned_image(name)
    last = ""
    for attempt in range(len(PULL_DELAYS) + 1):
        out = run("pull", "-q", ref)
        if out.returncode == 0:
            run("tag", ref, name)
            return ref
        last = (out.stderr or out.stdout or "").strip()
        if attempt < len(PULL_DELAYS):
            print(f"docs-test: pulling {ref} failed ({last}); again in {PULL_DELAYS[attempt]} s", file=sys.stderr)
            sleep(PULL_DELAYS[attempt])
    raise PageError(f"{ref} could not be pulled after {len(PULL_DELAYS) + 1} attempts: {last}")


APT_RETRIES = """cat > /etc/apt/apt.conf.d/80docs-test-retries <<'DOCS_TEST_EOF'
Acquire::Retries "10";
DOCS_TEST_EOF
"""


def runner_prelude(scenario: Scenario) -> str:
    """The runner's own setup inside a scenario's container: apt retries each
    download, with apt's own growing delay between tries. Nothing on a host."""
    if scenario.where != "host" and "apt" in scenario.tags:
        return APT_RETRIES
    return ""


# A skip reason that names a release: a version, or the word.
SKIP_FOR_A_RELEASE = re.compile(r"\b\d+\.\d+\b|\brelease", re.IGNORECASE)

# --cacert followed by a path that is not absolute and not a variable.
RELATIVE_CACERT = re.compile(r"--cacert[= ]\s*[\"']?(?![/\"'$])\S")

SERVER_REQUEST = re.compile(r"\bcurl\b[^\n]*https://vcon\.example\.com|https://vcon\.example\.com[^\n]*\bcurl\b")


def lint_page(blocks: list[Block], tag_sets: list[frozenset], host: bool = False) -> list[str]:
    problems = []
    seen: set[str] = set()
    for b in blocks:
        if b.kind == "sh" and b.skip is None:
            for name in RELEASE_VARIABLES:
                if re.search(rf"(?m)^{name}=", b.code):
                    if name in seen:
                        problems.append(
                            f"{b.at}: {name} is set again; a scenario that sets it would rewrite both"
                        )
                    seen.add(name)
        if b.skip is not None:
            if SKIP_FOR_A_RELEASE.search(b.skip):
                problems.append(
                    f"{b.at}: a skip that names a release; mark the step `since <version>`, "
                    "so it runs once that release is published"
                )
            continue
        if b.since is not None and b.since not in b.prose:
            problems.append(
                f"{b.at}: the step runs from release {b.since} on; the text before it must name "
                f"{b.since}, so a reader of an older release knows to skip it"
            )
        if not any(b.only <= tags for tags in tag_sets):
            problems.append(f"{b.at}: no scenario runs this block (only {' '.join(sorted(b.only))})")
        if b.kind == "sh" and RELATIVE_CACERT.search(b.code):
            problems.append(
                f"{b.at}: --cacert names a relative path, which works only in the directory that made it"
            )
        if b.kind == "sh" and b.expect is None and b.background is None:
            joined = b.code.replace("\\\n", " ")
            if SERVER_REQUEST.search(joined):
                problems.append(f"{b.at}: a request to the server with no output shown after it")
        if host and b.background is not None:
            problems.append(f"{b.at}: a background step on this host would outlive the run")
    return problems


def home_install_problems(front: str, install: list[Block]) -> list[str]:
    """The home page's install steps that docs/install.md does not show.

    `front` is the home page's TOML front matter. Each step's code must be a
    run of whole lines in one block of the install page, which CI runs, so the
    home page cannot drift from it.
    """
    import tomllib

    steps = tomllib.loads(front).get("extra", {}).get("install", [])
    blocks = [b.code.splitlines() for b in install if b.kind == "sh"]
    problems = []
    for step in steps:
        lines = [l for l in step.get("code", "").splitlines() if l.strip()]
        if not lines:
            continue
        found = any(
            block[i : i + len(lines)] == lines
            for block in blocks
            for i in range(len(block) - len(lines) + 1)
        )
        if not found:
            problems.append(
                f'_index.md: the install step "{step.get("title", "")}" is not a run of lines '
                "in any block of docs/install.md"
            )
    return problems


def _front_matter(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        text = f.read()
    parts = text.split("+++\n")
    return parts[1] if len(parts) > 2 else ""


def lint() -> list[str]:
    """Every rule, over every page a scenario runs, and the home page's
    install steps."""
    problems: list[str] = []
    pins: dict = {}
    try:
        pins = read_pins()
    except (OSError, PageError) as e:
        problems.append(str(e))
    try:
        problems += home_install_problems(
            _front_matter(os.path.join(ROOT, "website", "content", "_index.md")), page_blocks("install.md")
        )
    except (OSError, PageError) as e:
        problems.append(str(e))
    pages: dict[str, list] = {}
    for s in SCENARIOS:
        for p in s.pages:
            pages.setdefault(p, []).append(s)
    for page, scenarios in sorted(pages.items()):
        try:
            blocks = page_blocks(page)
        except (OSError, PageError) as e:
            problems.append(str(e))
            continue
        problems += lint_page(
            blocks, [s.tags for s in scenarios], host=any(s.where == "host" for s in scenarios)
        )
        problems += pin_problems(blocks, pins)
    for s in SCENARIOS:
        if not s.sets:
            continue
        try:
            _apply_sets(select([b for p in s.pages for b in page_blocks(p)], s), s.sets)
        except (OSError, PageError) as e:
            problems.append(f"{s.name}: {e}")
    return problems


def _docker(*args: str, check: bool = True, timeout: int | None = 600, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(["docker", *args], check=check, timeout=timeout, **kw)


GITHUB_API = "https://api.github.com/"
REPO = "NormB/vcon-server"


def github_api(path: str, token: str | None, run=subprocess.run) -> tuple[int, str]:
    """One GitHub API request: its HTTP status and body. With a token (CI's
    GH_TOKEN), the request carries it, so it is not held to the hosted
    runners' shared limit for unauthenticated requests. curl reads the header
    from standard input, so the token is never in a process's arguments."""
    cmd = ["curl", "-sS", "--retry", "3", "-w", "\n%{http_code}", GITHUB_API + path]
    stdin = None
    if token:
        cmd[1:1] = ["-H", "@-"]
        stdin = f"Authorization: Bearer {token}\n"
    out = run(cmd, input=stdin, capture_output=True, text=True, timeout=60)
    if out.returncode != 0:
        raise PageError(f"GitHub API {path}: {out.stderr.strip() or f'curl exited {out.returncode}'}")
    body, _, code = out.stdout.rpartition("\n")
    return int(code), body


def _refused(path: str, code: int, body: str) -> PageError:
    try:
        message = json.loads(body).get("message", body)
    except ValueError:
        message = body
    return PageError(f"GitHub API {path} answered {code}: {message}")


def released(version: str, token: str | None, run=subprocess.run) -> bool:
    """Whether release v<version> is published: 200 yes, 404 no, and any
    other answer an error, never a guess."""
    path = f"repos/{REPO}/releases/tags/v{version}"
    code, body = github_api(path, token, run)
    if code in (200, 404):
        return code == 200
    raise _refused(path, code, body)


def latest_release(token: str | None, run=subprocess.run) -> str:
    """The newest published release's version."""
    path = f"repos/{REPO}/releases/latest"
    code, body = github_api(path, token, run)
    if code != 200:
        raise _refused(path, code, body)
    return json.loads(body)["tag_name"].removeprefix("v")


RELEASE_VARIABLES = ("VERSION", "NEW_VERSION")


def release_sets(steps: list[Block], sets: dict, released, latest) -> dict:
    """The scenario's settings, plus a published release for each release
    variable a page sets to a version not published yet. A version bump
    lands before its tag, so until the release exists the steps run with the
    newest published one, and the commands are still tested."""
    out = dict(sets)
    newest = None
    for name in RELEASE_VARIABLES:
        if name in out:
            continue
        for b in steps:
            m = re.search(rf"(?m)^{name}=(\S+)$", b.code) if b.kind == "sh" else None
            if not m:
                continue
            if not released(m.group(1)):
                newest = newest or latest()
                if newest is None:
                    raise PageError("cannot learn the newest release")
                out[name] = newest
            break
    return out


def plan(steps: list[Block], scenario_sets: dict, released, latest) -> tuple[list[Block], dict, list[str]]:
    """The steps a run takes, the variables it sets, and what it tells the
    reader of its log: the release it runs with in place of one not yet
    published, and each `since` step that release is too old for."""
    sets = release_sets(steps, scenario_sets, released, latest)
    notes = [
        f"the release a page names is not published yet; running with {name}={value}"
        for name, value in sets.items()
        if name not in scenario_sets
    ]
    steps, left_out = release_steps(steps, sets)
    notes += [
        f"{b.at} is for release {b.since} and later; this run's release is older, so it does not run"
        for b in left_out
    ]
    return steps, sets, notes


def report(steps: list[Block], outdir: str) -> int:
    """Compare each step with its page; print one line per step. Returns
    how many steps ran as their page says."""
    passed = 0
    stopped = False
    for n, b in enumerate(steps):
        out_path = os.path.join(outdir, f"{n}.out")
        rc_path = os.path.join(outdir, f"{n}.rc")
        if stopped and not b.always:
            print(f"not run  {b.at}")
            continue
        got = open(out_path, encoding="utf-8", errors="replace").read() if os.path.exists(out_path) else ""
        rc = open(rc_path).read().strip() if os.path.exists(rc_path) else None
        why = None
        if rc is None:
            why = "it stopped the run: a command in it failed"
            stopped = True
        elif rc == "never":
            why = f"it never printed {b.background!r}"
            stopped = True
        elif rc != str(b.exit):
            why = f"it exited {rc}, the page expects {b.exit}"
            stopped = True
        elif b.expect is not None:
            why = mismatch(b.expect, got)
        if why is None:
            print(f"ok       {b.at}")
            passed += 1
            continue
        print(f"FAIL     {b.at}: {why}")
        print("  the block:")
        print("".join(f"    {l}\n" for l in b.code.rstrip().split("\n")), end="")
        if b.expect is not None:
            print("  the page says it prints:")
            print("".join(f"    {l}\n" for l in _clean(b.expect)), end="")
        print("  it printed:")
        print("".join(f"    {l}\n" for l in _clean(got)[-60:]), end="")
    return passed


def run(scenario: Scenario, binary: str | None, keep: str | None) -> int:
    blocks: list[Block] = []
    for p in scenario.pages:
        blocks += page_blocks(p)
    steps = select(blocks, scenario)
    token = os.environ.get("GH_TOKEN") or None
    steps, sets, notes = plan(
        steps, scenario.sets, lambda v: released(v, token), lambda: latest_release(token)
    )
    for note in notes:
        print(f"docs-test: {note}")
    pins = read_pins()
    missing = release_pin_problems(release_versions(steps, sets), pins)
    if missing:
        raise PageError("; ".join(missing))
    prelude = runner_prelude(scenario)
    if scenario.binary:
        if not binary:
            print(f"docs-test: {scenario.name} needs --binary", file=sys.stderr)
            return 2
        if scenario.where != "host" and scenario.install_binary:
            prelude += "install -m 0755 /docs-test/vcon-server /usr/local/bin/vcon-server\n"
    work = tempfile.mkdtemp(prefix=f"docs-test-{scenario.name}-")
    mirror = None
    try:
        if any(PAGE_RELEASE_URL.search(b.code) for b in steps):
            gateway = _docker("network", "inspect", "bridge", "--format", "{{(index .IPAM.Config 0).Gateway}}",
                              capture_output=True, text=True).stdout
            cache = os.environ.get("DOCS_TEST_PINNED") or os.path.join(work, "pinned")
            mirror = Mirror(pins, cache, mirror_host(gateway))
            base = mirror.start()
            print(f"docs-test: release assets come through the mirror at {base}, each fetched from github.com "
                  f"with scripts/fetch-pinned.sh and checked against {PINS_NAME}")
            steps = [dataclasses.replace(b, code=rewrite_releases(b.code, base)) for b in steps]
        for image in base_images(steps):
            pull_with_retry(image)
        text = script(steps, prelude, sets)
        with open(os.path.join(work, "run.sh"), "w", encoding="utf-8") as f:
            f.write(text)
        outdir = os.path.join(work, "out")
        os.mkdir(outdir)
        if scenario.where == "host":
            home = os.path.join(work, "work")
            os.mkdir(home)
            env = dict(os.environ, DOCS_TEST_OUT=outdir, DOCS_TEST_WORK=home)
            if binary:
                env["DOCS_TEST_BINARY"] = os.path.abspath(binary)
            try:
                subprocess.run(["bash", os.path.join(work, "run.sh")], env=env, timeout=scenario.timeout)
            except subprocess.TimeoutExpired:
                print(f"docs-test: {scenario.name} ran past {scenario.timeout} s")
        else:
            _run_in_container(scenario, binary, work, outdir)
        passed = report(steps, outdir)
        if keep:
            shutil.copytree(work, keep, dirs_exist_ok=True)
        print(f"docs-test: {scenario.name}: {passed} of {len(steps)} steps ran as the pages say")
        return 0 if steps and passed == len(steps) else 1
    finally:
        if mirror is not None:
            mirror.stop()
        shutil.rmtree(work, ignore_errors=True)


def _run_in_container(scenario: Scenario, binary: str | None, work: str, outdir: str) -> None:
    name = f"docs-test-{scenario.name}-{os.getpid()}"
    if scenario.where == "systemd":
        pull_with_retry("debian:trixie-slim")
        _docker("build", "-q", "-t", SYSTEMD_IMAGE, "-", input=SYSTEMD_DOCKERFILE.encode(), stdout=subprocess.DEVNULL)
        start = ["run", "-d", "--name", name, *SYSTEMD_RUN, *scenario.docker, SYSTEMD_IMAGE]
    else:
        start = ["run", "-d", "--name", name, *scenario.docker, pull_with_retry(scenario.where), "sleep", "infinity"]
    _docker(*start, stdout=subprocess.DEVNULL)
    try:
        if scenario.where == "systemd":
            # Up far enough to start units ("degraded" is fine for this). The
            # bus may not exist yet when the first ask is made.
            for _ in range(120):
                state = _docker("exec", name, "systemctl", "is-system-running", "--wait", check=False,
                                capture_output=True, text=True, timeout=120).stdout.strip()
                if state in ("running", "degraded"):
                    break
                time.sleep(0.5)
        _docker("exec", name, "mkdir", "-p", "/docs-test/out", "/docs-test/work")
        _docker("cp", os.path.join(work, "run.sh"), f"{name}:/docs-test/run.sh")
        if binary:
            _docker("cp", os.path.abspath(binary), f"{name}:/docs-test/vcon-server")
        env = ["-e", "DOCS_TEST_OUT=/docs-test/out", "-e", "DOCS_TEST_WORK=/docs-test/work"]
        if os.environ.get("GH_TOKEN"):
            env += ["-e", "GH_TOKEN"]
        commit = os.environ.get("DOCS_TEST_COMMIT")
        if commit:
            env += ["-e", f"DOCS_TEST_COMMIT={commit}"]
        try:
            _docker("exec", *env, name, "bash", "/docs-test/run.sh", check=False, timeout=scenario.timeout)
        except subprocess.TimeoutExpired:
            print(f"docs-test: {scenario.name} ran past {scenario.timeout} s")
        shutil.rmtree(outdir)
        _docker("cp", f"{name}:/docs-test/out", outdir)
    finally:
        _docker("rm", "-f", name, check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def main(argv: list[str]) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if argv else 2
    command, rest = argv[0], argv[1:]
    if command == "list":
        print(json.dumps([s.name for s in SCENARIOS]))
        return 0
    if command == "lint":
        problems = lint()
        for p in problems:
            print(p)
        return 1 if problems else 0
    if command == "run" and rest:
        by_name = {s.name: s for s in SCENARIOS}
        if rest[0] not in by_name:
            print(f"docs-test: no scenario {rest[0]}; see `list`", file=sys.stderr)
            return 2
        binary = keep = None
        args = rest[1:]
        while args:
            if args[0] == "--binary" and len(args) > 1:
                binary, args = args[1], args[2:]
            elif args[0] == "--keep" and len(args) > 1:
                keep, args = args[1], args[2:]
            else:
                print(f"docs-test: unknown argument {args[0]}", file=sys.stderr)
                return 2
        try:
            return run(by_name[rest[0]], binary, keep)
        except PageError as e:
            print(f"docs-test: {e}", file=sys.stderr)
            return 1
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
