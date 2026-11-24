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
    <!-- docs-test: terminal <word> -->    a full-screen command: run in a
                                           terminal of 80 columns by 24 lines,
                                           in a bash of its own (it sees the
                                           exported variables only); when it
                                           writes <word>, q is typed; the text
                                           block after it is the screen it had
                                           when it ended
    <!-- docs-test: terminal <word>; keys Enter q -->
                                           the same, with these keys typed in
                                           place of q: each one character, or
                                           Enter

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
  mirror answers 404, as github.com does. Each asset the steps can name is
  fetched before the first step (prefetch), so the mirror answers a step at
  once; dnf gives up on a server that sends nothing for 30 s. On 2026-11-09 github.com answered
  504 for most of an hour, and the release, upgrade and container scenarios
  failed at those downloads.
- Each image is pinned by digest (IMAGES) and pulled with retries; an image
  a page builds FROM is pulled the same way, then tagged with the name the
  page uses.
- In a container with apt, apt retries each download 3 times
  (Acquire::Retries) and gives up one that stalls for 10 s
  (Acquire::http::Timeout).
- A container with apt reads its packages from a list of mirrors, not from
  one host. Before anything else the runner runs scripts/apt-mirror-setup.sh
  --dry-run on this host for the suites the image's apt reads (apt_suites):
  the script asks each mirror of each source (APT_MIRRORS) for the release
  file of each suite within 5 s, keeps those whose files are the origin's
  own (or, when the origin does not answer, the newest copy the mirrors
  agree on), and the runner says in the log which they are and points apt
  at them with its `mirror+file:` method. The container's release smoke job
  runs the same script, so both choose by one rule. The list gives each mirror its place as its
  priority, so apt asks the first one that answered and goes on to the next
  only when a download fails. Signatures are checked as before: only the
  host a file comes from changes. The page's own apt-get commands run as a
  reader types them. If no mirror of a source answers, the run ends at once
  with exit code 3 and says that the package mirrors are unreachable, so an
  outage is not read as a page's failure. On 2026-11-10 archive.ubuntu.com
  and security.ubuntu.com did not answer on port 80; `apt-get update` in
  the release-ubuntu scenario waited out the scenario's 900 s, and the run
  reported docs/install.md:84 as failed. A mirror that stops answering
  after it was chosen still costs apt its time limit and its retries for
  every file, which is why a mirror that does not answer is left out of the
  list and not only placed last. Fedora's dnf picks among mirrors itself
  and is left alone.

Usage:
  python3 -I scripts/docs_test.py list               the scenario names, as JSON
  python3 -I scripts/docs_test.py lint               the rules every tested page keeps
  python3 -I scripts/docs_test.py artifact <scenario>  the CI artifact holding the server it runs
  python3 -I scripts/docs_test.py run <scenario> [--binary <vcon-server>] [--keep <dir>]
  python3 -I scripts/docs_test.py push <sha> --since <rev>   run what a push of <rev>..<sha> needs; stamp <sha>
  python3 -I scripts/docs_test.py check [--remote <name>]    pre-push: refs on stdin; reads stamps and CI
"""

from __future__ import annotations

import dataclasses
import ipaddress
import json
import os
import platform
import posixpath
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import importlib.util

_spec = importlib.util.spec_from_file_location("gh_rate_limit", os.path.join(os.path.dirname(os.path.abspath(__file__)), "gh_rate_limit.py"))
gh_rate_limit = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gh_rate_limit)

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
    # The text that says a full-screen command has drawn its screen.
    terminal: str | None = None
    # The keys typed once that text is written, in order.
    keys: tuple = ("q",)
    # The page's text from the last block or heading to this block: what a
    # reader sees just before it.
    prose: str = ""


# The keys a `keys` marker names by a word, and the printf format that
# writes what a terminal sends for each. Any other key is its one character.
KEY_NAMES = {"Enter": r"\r"}

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
        elif word == "terminal":
            if not rest:
                raise PageError(f"{at}: `terminal` needs the text that says the screen is ready")
            if " " in rest:
                raise PageError(f"{at}: `terminal` takes one word, as the command writes it")
            out["terminal"] = rest
        elif word == "keys":
            keys = tuple(rest.split())
            if not keys:
                raise PageError(f"{at}: `keys` needs the keys to type, such as `keys Enter q`")
            for key in keys:
                if len(key) != 1 and key not in KEY_NAMES:
                    raise PageError(
                        f"{at}: `keys`: `{key}` is not one character or one of {', '.join(sorted(KEY_NAMES))}"
                    )
            out["keys"] = keys
        elif word == "since":
            if not RELEASE.match(rest):
                raise PageError(f"{at}: `since` needs a release version such as 0.3.0")
            out["since"] = rest
        else:
            raise PageError(f"{at}: unknown docs-test marker `{word}`")
    if "terminal" in out and "background" in out:
        raise PageError(f"{at}: a `terminal` step cannot be a `background` step")
    if "keys" in out and "terminal" not in out:
        raise PageError(f"{at}: `keys` is for a `terminal` step")
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
    # The cargo features its pages need beyond the default build's. CI builds
    # a server with them and gives it to the scenario (`artifact`).
    features: tuple = ()


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


def systemd_dockerfile(mirrors: dict) -> str:
    """The systemd image's Dockerfile, its packages read through `mirrors`
    (see apt_mirror_setup)."""
    setup = "".join(f"{c} \\\n && " for c in apt_mirror_setup(mirrors))
    limits = f"-o Acquire::Retries={APT_TRIES} -o Acquire::http::Timeout={APT_TIMEOUT}"
    return f"""FROM {pinned_image("debian:trixie-slim")}
RUN {setup}apt-get update -qq {limits} \\
 && apt-get install -y -qq {limits} --no-install-recommends systemd systemd-sysv dbus >/dev/null \\
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
    "read-audit-logs.md",
    "tui.md",
    "operator-listener.md",
    "users-and-memberships.md",
    "roles-and-scopes.md",
    "tenant-administration.md",
    "session-origin.md",
    "upgrading.md",
)
# docs/upgrading.md installs the binary it names as NEW: the one under test.
UNDER_TEST = {"NEW": "/docs-test/vcon-server"}
# The release before this one, which the upgrade scenario upgrades from.
PREVIOUS = "0.4.15"

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
    # The PostgreSQL catalog preview: a PostgreSQL server from Debian's own
    # packages beside the server and RustFS, each under systemd, in one
    # container. The page needs the objects in a bucket and a server built
    # with the postgres feature: the release binaries have it
    # (scripts/reproducible-build.sh), the default build does not.
    Scenario(
        "postgres",
        "systemd",
        frozenset({"debian", "apt", "systemd", "rustfs", "postgres"}),
        ("getting-started.md", "s3-storage.md", "postgres-catalog.md"),
        features=("postgres",),
    ),
    # The Parquet export to a directory, read with DuckDB, after the server
    # is set up with its objects in the data directory.
    Scenario("parquet", "systemd", frozenset({"debian", "apt", "systemd"}), ("getting-started.md", "export-parquet.md")),
    # A Let's Encrypt certificate in place of the self-signed one, once by
    # HTTP-01 and once by DNS-01, from Pebble (ACME_SETUP), under systemd,
    # whose unit the page's deploy hook restarts.
    Scenario("lets-encrypt-http", "systemd", frozenset({"debian", "apt", "systemd", "acme", "http-01"}), ("getting-started.md", "lets-encrypt.md")),
    Scenario("lets-encrypt-dns", "systemd", frozenset({"debian", "apt", "systemd", "acme", "dns-01"}), ("getting-started.md", "lets-encrypt.md")),
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
    head = f"# {n}: {b.at}\n" + _mark(n)
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
    if b.terminal is not None:
        return head + _terminal_step(n, b)
    if b.exit != 0:
        return head + (
            f"set +e\n{{\n{b.code}}} > {out} 2>&1 < /dev/null\necho $? > {rc}\n"
            f'[ "$(cat {rc})" = {b.exit} ] || exit 100\n'
        )
    return head + (
        f"set -eo pipefail\n{{\n{b.code}}} > {out} 2>&1 < /dev/null\n"
        f"set +eo pipefail\necho 0 > {rc}\n"
    )


# A terminal step may take this long, from its start to its end.
TERMINAL_SECONDS = 120
# How long a terminal step's ready text is waited for before q is typed.
READY_SECONDS = 60


def _terminal_step(n: int, b: Block) -> str:
    """A full-screen command: run by util-linux script(1), which gives it a
    pseudo-terminal, here of 80 columns by 24 lines. Everything the command
    writes, escape sequences included, is the step's output. Once that holds
    the ready text, or after READY_SECONDS without it, the step's keys are
    typed (q, unless its marker names others), one after the other with no
    wait between them: the terminal keeps them until the command reads
    them, and a full-screen command draws between one key and the next. The
    block runs in a bash of its own, which sees the exported variables
    only."""
    out = f'"$DOCS_TEST_OUT/{n}.out"'
    rc = f'"$DOCS_TEST_OUT/{n}.rc"'
    code = f'"$DOCS_TEST_OUT/{n}.sh"'
    ended = f'"$DOCS_TEST_OUT/{n}.ended"'
    ready = _quote(b.terminal)
    typed = "".join(
        f"  printf {_quote(KEY_NAMES[key])}\n" if key in KEY_NAMES else f"  printf %s {_quote(key)}\n"
        for key in b.keys
    )
    return (
        f"set +e\ncat > {code} <<'DOCS_TEST_EOF'\nset -eo pipefail\nstty rows 24 cols 80\n{b.code}DOCS_TEST_EOF\n"
        f": > {out}\n"
        "{\n"
        f"  for _ in $(seq {READY_SECONDS * 10}); do grep -qaF -- {ready} {out} && break; [ -e {ended} ] && break; sleep 0.1; done\n"
        f"{typed}"
        f"  for _ in $(seq {TERMINAL_SECONDS * 10}); do [ -e {ended} ] && break; sleep 0.1; done\n"
        "} | {\n"
        f'  timeout {TERMINAL_SECONDS} script -qefc "bash $DOCS_TEST_OUT/{n}.sh" /dev/null > {out} 2>&1\n'
        f"  echo $? > {ended}\n"
        "}\n"
        f"grep -qaF -- {ready} {out} || {{ echo never > {rc}; exit 101; }}\n"
        f"cp {ended} {rc}\n"
        f'[ "$(cat {rc})" = {b.exit} ] || exit 100\n'
    )


# The terminal a terminal step runs in.
COLUMNS, LINES = 80, 24
# ESC [, an optional ?, numbers between semicolons, and the letter that
# says what the sequence does. One this reader does not know changes nothing.
CONTROL_SEQUENCE = re.compile(r"\x1b\[(\??)([\d;]*)([@-~])")
# ESC ], text, and BEL or ESC \\: the window's title and the like.
OS_COMMAND = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


class _Screen:
    """The cells of a terminal, and its cursor."""

    def __init__(self) -> None:
        self.cells = [[" "] * COLUMNS for _ in range(LINES)]
        self.x = self.y = 0

    def put(self, ch: str) -> None:
        if ch == "\r":
            self.x = 0
        elif ch == "\n":
            self.down()
        else:
            if self.x == COLUMNS:
                self.x = 0
                self.down()
            self.cells[self.y][self.x] = ch
            self.x += 1

    def down(self) -> None:
        """To the next line; on the last one, every line moves up instead."""
        if self.y == LINES - 1:
            del self.cells[0]
            self.cells.append([" "] * COLUMNS)
        else:
            self.y += 1

    def text(self) -> str:
        lines = ["".join(row).rstrip() for row in self.cells]
        while lines and not lines[-1]:
            lines.pop()
        return "".join(l + "\n" for l in lines)


def screen(written: str) -> str:
    """The screen after a command wrote `written` to a terminal of COLUMNS
    by LINES: its lines, without the blanks that end a line and the empty
    lines that end the screen. For a command that used the alternate
    screen, as a full-screen command does, it is that screen as the command
    last left it: what its user saw before it ended."""
    s = main = _Screen()
    left = None
    at = 0
    while at < len(written):
        m = CONTROL_SEQUENCE.match(written, at)
        if m:
            private, numbers, final = m.groups()
            if final == "H" and not private:
                # A number left out is 1.
                at_line, at_column = ([int(n) if n else 1 for n in numbers.split(";")] + [1])[:2]
                s.y = min(max(at_line, 1), LINES) - 1
                s.x = min(max(at_column, 1), COLUMNS) - 1
            elif final == "K" and numbers in ("", "0"):
                s.cells[s.y][s.x:] = [" "] * (COLUMNS - s.x)
            elif final == "J" and numbers == "2":
                s.cells = _Screen().cells
            elif private and numbers == "1049" and final == "h":
                s = _Screen()
            elif private and numbers == "1049" and final == "l":
                left, s = s.text(), main
            at = m.end()
            continue
        m = OS_COMMAND.match(written, at)
        if m:
            at = m.end()
            continue
        ch = written[at]
        if ch == "\x1b":
            # Any other escape is ESC and one character.
            at += 2
            continue
        if ch in "\r\n" or ch >= " " and ch != "\x7f":
            s.put(ch)
        at += 1
    return s.text() if left is None else left


# Run before the first step of each page after the first: a reader comes
# back to a later page in another shell and directory, so a page that needs
# a file another page left in the current directory fails here too.
NEW_PAGE_DIR = 'cd "$(mktemp -d "$DOCS_TEST_WORK/page.XXXXXX")" || exit 102\n'


def _page(b: Block) -> str:
    return b.at.rsplit(":", 1)[0]


# `systemctl <verb> [--now] <unit>`: a step that starts, stops or changes a
# unit. Its unit is one the run records under systemd (systemd_units).
SYSTEMCTL_UNIT = re.compile(
    r"\bsystemctl\s+(?:start|stop|restart|try-restart|reload|reload-or-restart|enable|disable|kill)"
    r"(?:\s+--now)?\s+([A-Za-z0-9@_.:-]+)"
)


def systemd_units(steps: list[Block]) -> tuple:
    """The units the steps start, stop or restart, in the order a step first
    names each, a service without its `.service`."""
    found: list[str] = []
    for b in steps:
        for m in SYSTEMCTL_UNIT.finditer(b.code):
            unit = m.group(1).removesuffix(".service")
            if not unit.startswith("-") and unit not in found:
                found.append(unit)
    return tuple(found)


def _units_record(units: tuple) -> str:
    """A shell function that writes, for each unit, what systemd says of it
    when the run ends: its status; its result, automatic restarts and start
    limit; its last 200 journal lines; and every line systemd itself wrote
    about it (each start, stop, exit and refused start, with its time)."""
    lines = ["docs_test_units() {\n{\n"]
    for u in units:
        full = u if "." in u else u + ".service"
        for command in (
            f"systemctl status --no-pager -l {u}",
            f"systemctl show -p Result -p NRestarts -p StartLimitBurst -p StartLimitIntervalUSec {u}",
            f"journalctl -u {u} --no-pager -n 200 -o short-precise",
            f"journalctl _PID=1 UNIT={full} --no-pager -o short-precise",
        ):
            lines.append(f"echo {_quote('== ' + command)}\n{command}\n")
    lines.append('} > "$DOCS_TEST_OUT/systemd.txt" 2>&1 < /dev/null\n}\n')
    return "".join(lines)


def start_limit_reset(units: tuple) -> str:
    """The lines, run where a page begins under systemd, that reset each
    unit's start counter. systemd refuses a unit's sixth start within 10 s
    (DefaultStartLimitBurst=5, DefaultStartLimitIntervalSec=10s), counting
    every start whatever started it. An operator runs each how-to on its
    own occasion; this run puts the pages one after the other in one
    container, so without the reset the starts of earlier pages count
    against a later one's: on CI the sixth start of the systemd scenario
    (operator-listener.md, its second restart) came within 10 s of the
    first, and was refused (SYSTEMD-HOWTO-START-LIMIT). Within a page the
    limit holds as it does for an operator. A unit systemd does not know
    yet is not an error."""
    return "# Each page meets systemd's start limit on its own, as an operator does.\n" + "".join(
        f"systemctl reset-failed {u} < /dev/null 2>/dev/null || true\n" for u in units
    )


def _mark(n: int) -> str:
    """The line that records when step `n` starts."""
    return f'date +%s.%N > "$DOCS_TEST_OUT/{n}.start"\n'


def script(steps: list[Block], prelude: str, sets: dict | None = None, units: tuple = ()) -> str:
    """The bash script that runs the steps and records each one's output and
    start time, and the time the main steps and the last-by-rule steps end.
    With `units` (a systemd scenario), it records those units as systemd
    sees them when the run ends (systemd.txt)."""
    if sets is None:
        sets = {}
    steps = _apply_sets(steps, sets) if sets else steps
    main = [s for s in steps if not s.always]
    last = [s for s in steps if s.always]
    record = "; docs_test_units" if units else ""
    parts = [
        "#!/usr/bin/env bash\n",
        "# Written by scripts/docs_test.py from the pages; each step is one block.\n",
        "set -u\n",
        ': "${DOCS_TEST_OUT:?}" "${DOCS_TEST_WORK:?}"\n',
        "trap 'rc=$?; set +e; date +%s.%N > \"$DOCS_TEST_OUT/end\"; docs_test_always; "
        f"date +%s.%N > \"$DOCS_TEST_OUT/finished\"{record}; echo $rc > \"$DOCS_TEST_OUT/exit\"' EXIT\n",
    ]
    if units:
        parts.append(_units_record(units))
    parts.append("docs_test_always() {\n:\n")
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
            if units:
                parts.append(start_limit_reset(units))
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


# dpkg's name for each CPU uname -m names.
DPKG_ARCH = {"x86_64": "amd64", "aarch64": "arm64"}
# NAME=value alone on its line, and NAME=value anywhere in a case arm.
# A $( ... ) expansion in a value is taken whole, spaces and all.
ASSIGNMENT = re.compile(r"(?m)^\s*([A-Z_][A-Z0-9_]*)=((?:\$\([^)]*\)|\S)+)\s*$")
ARM_ASSIGNMENT = re.compile(r"\b([A-Z_][A-Z0-9_]*)=([^\s;]+)")
NAMED = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)")


def _known_values(steps: list[Block], sets: dict, machine: str) -> dict:
    """The values the steps give their variables, as far as reading them
    shows: the arm of a case for this CPU first, then each NAME=value line,
    the first one winning; the scenario's settings over both."""
    values: dict = {}
    for b in steps:
        if b.kind != "sh":
            continue
        for arm in re.finditer(rf"(?ms)^\s*{re.escape(machine)}\)(.*?);;", b.code):
            for name, value in ARM_ASSIGNMENT.findall(arm.group(1)):
                values.setdefault(name, value.strip("\"'"))
        for name, value in ASSIGNMENT.findall(b.code):
            values.setdefault(name, value.strip("\"'"))
    values.update(sets)
    return values


def _resolve(text: str, values: dict, machine: str) -> str:
    """text with $(uname -m), $(dpkg --print-architecture) and each known
    $NAME or ${NAME} put in; an unknown one is left as it is."""
    for _ in range(5):
        new = text.replace("$(uname -m)", machine).replace("$(dpkg --print-architecture)", DPKG_ARCH.get(machine, machine))
        new = NAMED.sub(lambda m: values.get(m.group(1) or m.group(2), m.group(0)), new)
        if new == text:
            break
        text = new
    return text


def prefetch_urls(steps: list[Block], sets: dict, pins: dict, machine: str) -> list[str]:
    """The pinned assets the steps can download on this CPU: each step URL
    with what is known put in, and each pin it then matches. What stays
    unknown matches any pin, so nothing a step needs is left out."""
    values = _known_values(steps, sets, machine)
    found = set()
    for b in steps:
        for url in PAGE_RELEASE_URL.findall(b.code):
            pattern = _template(_resolve(url, values, machine))
            found.update(p for p in pins if pattern.match(p))
    return sorted(found)


def prefetch(mirror: "Mirror", urls: list[str]) -> list[tuple[str, int]]:
    """Fetch each asset into the mirror's cache before the first step, four
    at a time, so the mirror answers a step at once. A client such as dnf
    gives up on a server that sends nothing for 30 s; fetch-pinned.sh may
    wait out an outage for five minutes. A failure here is reported, not
    fatal: the step that needs the asset asks again."""
    from concurrent.futures import ThreadPoolExecutor

    def one(url: str) -> tuple[str, int]:
        status, _ = mirror.answer(url[len(GITHUB):])
        print(f"docs-test: prefetched {url}: {'ok' if status == 200 else f'answer {status}'}", file=sys.stderr)
        return url, status

    with ThreadPoolExecutor(max_workers=4) as pool:
        return list(pool.map(one, urls))


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


# How often apt asks again for a download that failed, and the seconds it
# waits on a host that sends nothing. Both are small: apt spends them on
# every file before it asks the next mirror, and with a time limit of 30 s
# and 10 retries one `apt-get update` did not end in 170 s (2026-11-10).
APT_TRIES = 3
APT_TIMEOUT = 10
APT_RETRIES = f"""cat > /etc/apt/apt.conf.d/80docs-test-retries <<'DOCS_TEST_EOF'
Acquire::Retries "{APT_TRIES}";
Acquire::http::Timeout "{APT_TIMEOUT}";
Acquire::https::Timeout "{APT_TIMEOUT}";
DOCS_TEST_EOF
"""

# Each package source an image's apt reads (an origin), and the hosts that
# hold the same files: the origin itself and at least one other, in the
# order apt tries them. The table is a file of its own because
# scripts/apt-mirror-setup.sh reads it too, in a container that has no
# python3 yet (the release's smoke job).
APT_MIRRORS_NAME = "scripts/apt-mirrors.txt"


def load_apt_mirrors(text: str, where: str = APT_MIRRORS_NAME) -> dict:
    """The table of mirrors in `text`: {origin: (mirror, ...)}, a line an
    origin, in the order of the lines. A table that would leave an origin
    on one host is refused."""
    table = {}
    for n, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.strip().startswith("#"):
            continue
        origin, *mirrors = line.split()
        if not mirrors:
            raise PageError(f"{where}:{n}: {origin} has no mirror")
        if origin in table:
            raise PageError(f"{where}:{n}: {origin} is written twice")
        for m in mirrors:
            # A container has no certificates before its first install, and
            # apt-mirror-setup.sh asks over bash's /dev/tcp.
            if not m.startswith("http://"):
                raise PageError(f"{where}:{n}: {m} is not an http:// address")
        if origin not in mirrors:
            raise PageError(f"{where}:{n}: {origin} is not among its own mirrors")
        if len(set(mirrors)) < 2:
            raise PageError(f"{where}:{n}: {origin} has one host; an origin needs itself and at least one other")
        table[origin] = tuple(mirrors)
    if not table:
        raise PageError(f"{where}: no origin")
    return table


with open(os.path.join(ROOT, APT_MIRRORS_NAME), encoding="utf-8") as _f:
    APT_MIRRORS = load_apt_mirrors(_f.read())


class MirrorError(PageError):
    """No package mirror answered: an outage outside the pages."""


# The release name of each image whose apt the runner points at mirrors:
# the suites its sources read are named after it. The systemd image is
# built from debian:trixie-slim.
APT_RELEASES = {"debian:trixie-slim": "trixie", "systemd": "trixie", "ubuntu:24.04": "noble"}


def apt_suites(where: str, machine: str) -> dict:
    """The suites the apt of a scenario's system reads from each origin, as
    its image's sources name them: by its image, and for Ubuntu by the
    CPU. Nothing for a system without apt, and for this host, whose sources
    the runner does not touch."""
    debian = where == "systemd" or where.startswith("debian:")
    if not debian and not where.startswith("ubuntu:"):
        return {}
    name = APT_RELEASES.get(where)
    if name is None:
        raise PageError(f"{where}: no release name in APT_RELEASES, so the runner cannot name the suites its apt reads")
    if debian:
        return {"http://deb.debian.org/debian": (name, f"{name}-updates"), "http://deb.debian.org/debian-security": (f"{name}-security",)}
    suites = (name, f"{name}-updates", f"{name}-backports")
    if machine == "x86_64":
        return {"http://archive.ubuntu.com/ubuntu/": suites, "http://security.ubuntu.com/ubuntu/": (f"{name}-security",)}
    return {"http://ports.ubuntu.com/ubuntu-ports/": suites + (f"{name}-security",)}


def apt_origins(where: str, machine: str) -> tuple:
    """The origins the apt of a scenario's system reads (see apt_suites)."""
    return tuple(apt_suites(where, machine))


APT_MIRROR_SETUP = os.path.join(ROOT, "scripts/apt-mirror-setup.sh")


def ask_mirrors(suites: dict, table: str | None = None, timeout: int = 5) -> tuple:
    """scripts/apt-mirror-setup.sh's (exit code, stdout, stderr), run with
    --dry-run on this host for a system whose sources read `suites`
    ({origin: (suite, ...)}): it asks the mirrors and writes nothing. The
    system is a directory of its own, removed after."""
    with tempfile.TemporaryDirectory(prefix="docs-test-apt-") as root:
        os.makedirs(os.path.join(root, "etc/apt/sources.list.d"))
        with open(os.path.join(root, "etc/apt/sources.list"), "w", encoding="utf-8") as f:
            f.write("".join(f"deb {origin} {suite} main\n" for origin, names in suites.items() for suite in names))
        command = ["bash", APT_MIRROR_SETUP, "--dry-run", "--root", root, "--timeout", str(timeout)]
        if table is not None:
            command += ["--table", table]
        # The script asks every host at once, each within `timeout`.
        done = subprocess.run(command, capture_output=True, text=True, timeout=timeout * 4 + 60, stdin=subprocess.DEVNULL, check=False)
        return done.returncode, done.stdout, done.stderr


def choose_mirrors(suites: dict, ask=None) -> tuple:
    """For each origin of `suites`, the mirrors scripts/apt-mirror-setup.sh
    keeps, in its order, and the script's other lines as notes for the run's
    log: (chosen, notes). The rule is the script's alone. An origin with no
    mirror to read stops the run with a MirrorError: no step could install
    a package, and a run that went on would hang in the first one and read
    as that page's failure."""
    if not suites:
        return {}, []
    ask = ask or ask_mirrors
    prefix = "apt-mirror-setup: "
    try:
        rc, out, err = ask(suites)
    except subprocess.TimeoutExpired as e:
        raise MirrorError(f"scripts/apt-mirror-setup.sh did not end within {e.timeout} s; this is not a failure of the pages") from None
    said = [line.removeprefix(prefix) for line in err.splitlines() if line.strip()]
    if rc == 3:
        raise MirrorError("; ".join(said) + "; this is not a failure of the pages")
    if rc != 0:
        raise PageError(f"scripts/apt-mirror-setup.sh exited {rc}: {'; '.join(said)}")
    chosen, notes = {}, []
    for line in out.splitlines():
        found = re.fullmatch(r"packages from (\S+) come from (.+)", line.removeprefix(prefix))
        if found:
            chosen[found[1]] = found[2].split(", then ")
        elif line.strip():
            notes.append("docs-test: " + line.removeprefix(prefix))
    notes += ["docs-test: " + line for line in said]
    for origin in suites:
        if origin not in chosen:
            raise PageError(f"scripts/apt-mirror-setup.sh named no mirror for {origin}: {out.strip()}")
    return chosen, notes


def mirror_notes(chosen: dict) -> list[str]:
    """What the run's log says about the mirrors: one line an origin."""
    return [f"docs-test: packages from {origin} come from {', then '.join(mirrors)}" for origin, mirrors in chosen.items()]


def apt_mirror_setup(chosen: dict, root: str = "") -> list[str]:
    """The shell commands that point apt at the chosen mirrors: for each
    origin, a file listing them, and the origin's address in every source
    file replaced by apt's `mirror+file:` method on that file, which asks
    the mirror of the lowest priority first and the next when it fails. A
    page's own `apt-get` commands then run as a
    reader types them. `root` is where /etc is, for tests. Each command is
    one line, so they also join into a Dockerfile's RUN."""
    commands = []
    for n, (origin, mirrors) in enumerate(chosen.items()):
        listing = f"{root}/etc/apt/docs-test-mirrors-{n}.list"
        # Each with its place as its priority: among mirrors without one
        # apt's mirror method chooses for itself, not in the order written.
        places = " ".join(f"{_quote(m)} {place}" for place, m in enumerate(mirrors, 1))
        commands.append(f"printf '%s\\tpriority:%s\\n' {places} > {listing}")
        # The origin with or without its last slash, as a whole word: one
        # origin (.../debian) is the start of another (.../debian-security).
        found = re.sub(r"[.\[\]*^$\\#]", r"\\\g<0>", origin.rstrip("/"))
        # Each source file there is: an image has sources.list, files in
        # sources.list.d, or both, and the runner's setup stops at a
        # command that fails.
        commands.append(
            f"for f in {root}/etc/apt/sources.list {root}/etc/apt/sources.list.d/*; do "
            f"if [ -f \"$f\" ]; then sed -i 's#{found}/\\?\\([[:space:]]\\|$\\)#mirror+file:{listing}\\1#' \"$f\"; fi; done"
        )
    return commands


# script(1), which runs a terminal step, where the image lacks it: Fedora's
# has no util-linux at all. On Debian and Ubuntu it is in bsdutils, which
# every image has.
DNF_SCRIPT = "dnf install -y -q util-linux-script\n"


# The TSIG secret the Let's Encrypt page's DNS-01 credentials show, which
# the DNS server ACME_SETUP runs accepts. It is the base64 of a sentence
# that says it is an example.
ACME_TSIG_SECRET = "ZXhhbXBsZS10c2lnLXNlY3JldC1yZXBsYWNlLXdpdGgtdGhlLW9uZS15b3VyLWRucy1zZXJ2ZXItbWFkZSEh"

# An acme scenario's stand-in for Let's Encrypt, so the Let's Encrypt page
# runs unchanged: Pebble, Let's Encrypt's test ACME server, on this
# container, with root's certbot cli.ini naming it as the server (the one in
# /etc/letsencrypt is the package's conffile: one written there first stops
# the package's install at dpkg's prompt). BIND serves
# example.com, resolves vcon.example.com to this container for Pebble's
# HTTP-01 check, and takes the page's TSIG key for DNS-01 updates. Pebble's
# listener and its roots are made trusted the way a public CA's are, so
# clients that pass no CA file trust the certificate certbot gets.
ACME_SETUP = f"""\
apt-get update -qq
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq pebble bind9 openssl curl ca-certificates >/dev/null
cat > /etc/bind/docs-test-certbot.key <<'DOCS_TEST_EOF'
key "certbot." {{ algorithm hmac-sha512; secret "{ACME_TSIG_SECRET}"; }};
DOCS_TEST_EOF
cat > /etc/bind/named.conf.local <<'DOCS_TEST_EOF'
include "/etc/bind/docs-test-certbot.key";
zone "example.com" {{
  type primary;
  file "/var/lib/bind/example.com.zone";
  update-policy {{ grant certbot. name _acme-challenge.vcon.example.com. TXT; }};
}};
DOCS_TEST_EOF
cat > /etc/bind/named.conf.options <<'DOCS_TEST_EOF'
options {{
  directory "/var/cache/bind";
  listen-on {{ 127.0.0.1; }};
  listen-on-v6 {{ none; }};
  recursion no;
  dnssec-validation no;
}};
DOCS_TEST_EOF
cat > /var/lib/bind/example.com.zone <<'DOCS_TEST_EOF'
$TTL 60
@    IN SOA ns.example.com. hostmaster.example.com. 1 60 60 600 60
@    IN NS  ns.example.com.
ns   IN A   127.0.0.1
vcon IN A   127.0.0.1
DOCS_TEST_EOF
chown bind:bind /var/lib/bind/example.com.zone
systemctl restart named
install -d /etc/pebble
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 -subj /CN=localhost \\
  -addext subjectAltName=DNS:localhost -keyout /etc/pebble/key.pem -out /etc/pebble/cert.pem 2>/dev/null
cat > /etc/pebble/pebble.json <<'DOCS_TEST_EOF'
{{"pebble": {{"listenAddress": "127.0.0.1:14000", "managementListenAddress": "127.0.0.1:15000",
  "certificate": "/etc/pebble/cert.pem", "privateKey": "/etc/pebble/key.pem",
  "httpPort": 80, "tlsPort": 443, "ocspResponderURL": "", "externalAccountBindingRequired": false}}}}
DOCS_TEST_EOF
cp /etc/pebble/cert.pem /usr/local/share/ca-certificates/docs-test-pebble-listener.crt
update-ca-certificates >/dev/null 2>&1
systemd-run --unit pebble --setenv PEBBLE_VA_NOSLEEP=1 --setenv PEBBLE_WFE_NONCEREJECT=0 \\
  /usr/bin/pebble -config /etc/pebble/pebble.json -dnsserver 127.0.0.1:53
timeout 30 sh -c 'until curl -sf -o /dev/null https://localhost:14000/dir; do sleep 0.2; done'
curl -sf https://localhost:15000/roots/0 > /usr/local/share/ca-certificates/docs-test-pebble-root.crt
update-ca-certificates >/dev/null 2>&1
install -d -m 0700 /root/.config/letsencrypt
echo 'server = https://localhost:14000/dir' > /root/.config/letsencrypt/cli.ini
"""


def runner_prelude(scenario: Scenario, steps: list[Block] | tuple = (), mirrors: dict | None = None) -> str:
    """The runner's own setup inside a scenario's container: apt retries each
    download, with apt's own growing delay between tries, gives up one that
    stalls, and reads the mirrors chosen for this run; a dnf system whose
    steps hold a terminal step gets script(1). Nothing on a host."""
    if scenario.where == "host":
        return ""
    if "apt" in scenario.tags:
        apt = APT_RETRIES + "".join(c + "\n" for c in apt_mirror_setup(mirrors or {}))
        return apt + ACME_SETUP if "acme" in scenario.tags else apt
    if "dnf" in scenario.tags and any(b.terminal is not None for b in steps):
        return DNF_SCRIPT
    return ""


# A skip reason that names a release: a version, or the word.
SKIP_FOR_A_RELEASE = re.compile(r"\b\d+\.\d+\b|\brelease", re.IGNORECASE)

# --cacert followed by a path that is not absolute and not a variable.
RELATIVE_CACERT = re.compile(r"--cacert[= ]\s*[\"']?(?![/\"'$])\S")

SERVER_REQUEST = re.compile(r"\bcurl\b[^\n]*https://vcon\.example\.com|https://vcon\.example\.com[^\n]*\bcurl\b")


# A `grep` inside an `until` loop: the step waits for a line holding its
# pattern. The pattern is the first word after grep's options (and `--`),
# or the words of a quoted one, with or without a backslash before the quote.
WAIT = re.compile(
    r"""\buntil\b[^;]*?\bgrep\s+(?:-[A-Za-z]+\s+)*(?:--\s+)?"""
    r"""(?:\\?'([^'\\]*)\\?'|\\?"([^"\\]*)\\?"|([^\s;'"|)]+))"""
)

# The lines of serve's background jobs (the purge sweep, the pack run and
# the rotation warning), by the start of their message. The server logs
# `serving` before it starts any of them (serve.rs, serve_opened).
BACKGROUND_LINES = (
    "purge sweep",
    "pack run",
    "key rotation overdue",
    "the key rotation backlog could not be read",
)
SERVE_LINE = re.compile(r"\bvcon_server::serve: (.*)$")


def waited_for(code: str) -> str | None:
    """The pattern the last `until ... grep` of `code` waits for, if any."""
    found = list(WAIT.finditer(code))
    if not found:
        return None
    last = found[-1]
    return next(g for g in last.groups() if g is not None)


def wait_problems(b: Block) -> list[str]:
    """A step that waits for a line and shows lines after it: those may
    not be written yet when the line waited for is."""
    if b.expect is None:
        return []
    word = waited_for(b.code)
    if word is None:
        return []
    shown = [l for l in _clean(b.expect) if l.strip() != ANY_LINES]
    if not shown or word in shown[-1] or not any(word in l for l in shown):
        return []
    return [
        f"{b.at}: the step waits for {word!r} and shows lines after that one, which may not be "
        "written yet; wait for a word of the last line it shows"
    ]


# A loop, from its `for`, `while` or `until` to its first `done`, and a
# `grep` whose pattern holds the key prefix `objects/`.
LOOP = re.compile(r"\b(?:for|while|until)\b(.*?)\bdone\b", re.S)
OBJECTS_GREP = re.compile(r"\bgrep\b[^|\n;]*?objects/")
# A comment: a line that starts with `#`, or ` #` and what follows it.
COMMENT = re.compile(r"(?m)(?:^\s*|\s+)#.*$")


def waits_for_objects(code: str) -> bool:
    """A loop of `code` greps for the `objects/` keys: it waits for a pack
    run's objects to leave the bucket."""
    code = COMMENT.sub("", code.replace("\\\n", " "))
    return any(OBJECTS_GREP.search(m.group(1)) for m in LOOP.finditer(code))


def objects_wait_problems(b: Block) -> list[str]:
    """A step that waits for a pack run by its objects. A pack run removes
    the objects of each file it records before it merges the date's small
    files, writes the manifest and logs `pack run` (Store::pack, then
    serve.rs pack_all), so the objects are gone while the run goes on."""
    if b.kind != "sh" or not waits_for_objects(b.code):
        return []
    return [
        f"{b.at}: the step waits for the objects to leave the bucket, which a pack run does before "
        "it merges files, writes the manifest and logs `pack run`; wait for that run's `pack run` line"
    ]


def start_order_problems(b: Block) -> list[str]:
    """A text that shows a background job's line before the `serving` line
    of the same start. A line after `serving` belongs to that start, up to
    its `stopped` line."""
    if b.expect is None:
        return []
    problems, before, served = [], [], False
    for line in _clean(b.expect):
        m = SERVE_LINE.search(line)
        if m is None:
            continue
        message = m.group(1)
        if message.startswith("serving"):
            problems += [
                f"{b.at}: the text shows {l.strip()!r} before the `serving` line; the server logs "
                "`serving` before any background job starts"
                for l in before
            ]
            before, served = [], True
        elif message.startswith("stopped"):
            served = False
        elif message.startswith(BACKGROUND_LINES) and not served:
            before.append(line)
    return problems


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
        if b.terminal is not None and b.expect is None:
            problems.append(f"{b.at}: a terminal step with no screen shown after it")
        if host and b.background is not None:
            problems.append(f"{b.at}: a background step on this host would outlive the run")
        problems += wait_problems(b)
        problems += objects_wait_problems(b)
        problems += start_order_problems(b)
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


ROOT_ONLY_DIRECTORY = re.compile(r"\binstall -d -m 0?700 (/[\w./-]+)\s*$")
WHOLE_LINE_ASSIGNMENT = re.compile(r"^([A-Z_][A-Z0-9_]*)=(.+)$")


def _commands(text: str) -> list[tuple[int, str]]:
    """Each command line inside the ```sh blocks of a page's text, with the
    number of its first line; a line ending in a backslash is joined to the
    next."""
    found = []
    inside = False
    pending: tuple[int, str] | None = None
    for n, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if stripped.startswith("```"):
            inside = not inside and stripped == "```sh"
            pending = None
            continue
        if not inside:
            continue
        start, sofar = pending if pending else (n, "")
        if stripped.endswith("\\"):
            pending = (start, sofar + stripped[:-1])
            continue
        pending = None
        found.append((start, sofar + stripped))
    return found


def root_only_directory_problems(texts: dict[str, str]) -> list[str]:
    """The paths a page's command writes as the service account under a
    directory that a page makes for root alone.

    `install -d -m 0700 <dir>` with no owner (the pattern takes the mode
    and then the directory, nothing between) makes <dir>, or changes one
    that exists, so that only root may enter it. A command run with
    `sudo -u vcon` that names a path under <dir> is then refused, whichever
    page it is on and whichever was followed first. The scenarios run the
    pages in one order, so they do not see it: on 2026-11-10 the upgrade
    page and the operations page made /backup that way, and `vcon-server
    backup /backup/vcon-server/...`, as the backup page and the operations
    page run it, was refused for a reader who had followed them first.
    `texts` is each page's text by its name.
    """
    root_only: list[tuple[str, int, str]] = []
    as_vcon: list[tuple[str, int, str]] = []
    for page, text in sorted(texts.items()):
        variables: dict[str, str] = {}
        for n, command in _commands(text):
            made = ROOT_ONLY_DIRECTORY.search(command)
            if made:
                root_only.append((page, n, made.group(1).rstrip("/")))
            assigned = WHOLE_LINE_ASSIGNMENT.match(command)
            if assigned:
                variables[assigned.group(1)] = assigned.group(2).strip('"')
            if "sudo -u vcon " in command:
                for name, value in variables.items():
                    command = command.replace("${" + name + "}", value).replace("$" + name, value)
                as_vcon.append((page, n, command))
    problems = []
    for made_on, made_at, directory in root_only:
        under = re.compile(re.escape(directory) + r"/[\w.$-]")
        for page, n, command in as_vcon:
            if under.search(command):
                problems.append(
                    f"docs/{made_on}:{made_at}: makes {directory} for root alone, and docs/{page}:{n} "
                    f"writes under it as vcon; make a directory of its own for what root keeps there"
                )
    return problems


def artifact(scenario: Scenario) -> str:
    """The name of the CI artifact that holds the server a scenario runs:
    the default build's, or the build with the scenario's features."""
    return "-".join(("docs-vcon-server", *scenario.features))


GROUP = re.compile(r'(?m)^group = "([^"]*)"$')
GUIDE_GROUPS = ("start", "how-to")
GUIDE_INDEX = "how-to.md"


def page_group(text: str) -> str:
    """The sidebar group a page's front matter names, or "" for none."""
    parts = text.split("+++\n")
    found = GROUP.search(parts[1]) if len(parts) > 2 else None
    return found.group(1) if found else ""


def guides_not_run(texts: dict[str, str], scenarios: list) -> list[str]:
    """The guides no scenario runs. A guide is a page in the group `start`
    or `how-to`, but for the page that lists them. The how-to index says CI
    runs each guide's commands; this holds it to that."""
    run_pages = {page for s in scenarios for page in s.pages}
    return sorted(
        name
        for name, text in texts.items()
        if name != GUIDE_INDEX and page_group(text) in GUIDE_GROUPS and name not in run_pages
    )


def docs_texts() -> dict[str, str]:
    """The text of every documentation page, by its file name."""
    texts = {}
    for name in sorted(os.listdir(DOCS)):
        if name.endswith(".md"):
            with open(os.path.join(DOCS, name), encoding="utf-8") as f:
                texts[name] = f.read()
    return texts


def lint() -> list[str]:
    """Every rule, over every page a scenario runs, and the home page's
    install steps."""
    problems: list[str] = []
    try:
        texts = docs_texts()
        problems += root_only_directory_problems(texts)
        problems += [
            f"docs/{name}: a guide no scenario runs; add it to a scenario in scripts/docs_test.py"
            for name in guides_not_run(texts, SCENARIOS)
        ]
    except OSError as e:
        problems.append(str(e))
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


def _github_api_once(path: str, token: str | None, run) -> tuple[int, str, int | None, int | None]:
    cmd = ["curl", "-sS", "--retry", "3", "-w", "\n%header{x-ratelimit-remaining} %header{x-ratelimit-reset} %{http_code}", GITHUB_API + path]
    stdin = None
    if token:
        cmd[1:1] = ["-H", "@-"]
        stdin = f"Authorization: Bearer {token}\n"
    out = run(cmd, input=stdin, capture_output=True, text=True, timeout=60)
    if out.returncode != 0:
        raise PageError(f"GitHub API {path}: {out.stderr.strip() or f'curl exited {out.returncode}'}")
    body, _, last = out.stdout.rpartition("\n")
    fields = last.split()
    remaining, reset = (int(fields[0]), int(fields[1])) if len(fields) == 3 else (None, None)
    return int(fields[-1]), body, remaining, reset


def github_api(path: str, token: str | None, run=subprocess.run, clock=time.time, sleep=time.sleep, err=sys.stderr) -> tuple[int, str]:
    """One GitHub API request: its HTTP status and body. With a token (CI's
    GH_TOKEN), the request carries it, so it is not held to the hosted
    runners' shared limit for unauthenticated requests. curl reads the header
    from standard input, so the token is never in a process's arguments.
    A 403 or 429 with no request left is asked once more after the budget's
    reset, when that is at most 900 s away (scripts/gh_rate_limit.py's rule)."""
    code, body, remaining, reset = _github_api_once(path, token, run)
    if code not in (403, 429) or remaining != 0 or reset is None:
        return code, body
    budget = gh_rate_limit.Budget(0, 0, reset)
    verdict, seconds = gh_rate_limit.plan(budget, clock(), gh_rate_limit.DEFAULT_MAX, gh_rate_limit.DEFAULT_MARGIN)
    if verdict != "wait":
        return code, body
    print(f"docs-test: GitHub API {path}: no request left; waiting {seconds} s for the reset", file=err)
    sleep(seconds)
    return _github_api_once(path, token, run)[:2]


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


RELEASE_VARIABLES = ("VERSION", "NEW_VERSION")
# What `run` prints (as "docs-test: <note>") when it runs a page's release
# variable with the newest pinned release (release_plan); `push` reads it back with
# `substituted`, so the summary and the stamp name the release that ran.
SUBSTITUTED_NOTE = "the release a page names is not published with pins yet; running with {name}={value}"
_SUBSTITUTED = re.compile(
    "^docs-test: "
    + re.escape(SUBSTITUTED_NOTE)
    .replace(re.escape("{name}"), "(" + "|".join(RELEASE_VARIABLES) + ")")
    .replace(re.escape("{value}"), r"(\S+)")
    + "$",
    re.M,
)


def substituted(output: str) -> list[str]:
    """The NAME=VALUE of each release a run's output says it ran in place
    of a page's unpublished one, in order, each once."""
    return list(dict.fromkeys(f"{m.group(1)}={m.group(2)}" for m in _SUBSTITUTED.finditer(output)))


def release_words(note: str) -> str:
    """The words a summary line gives a stamp's release note
    ("VERSION=0.4.6", comma-separated), or "" for none."""
    runs = [r for r in note.split(",") if r]
    if not runs:
        return ""
    plural = "releases" if len(runs) > 1 else "release"
    return f"it ran {' and '.join(runs)} in place of the {plural} its page names, not yet published with pins"


def release_sets(steps: list[Block], sets: dict, released, latest) -> dict:
    """The scenario's settings, plus `latest()` for each release variable a
    page sets to a version `released` says is not there yet. A version bump
    lands before its tag, and the tag before the pins of its assets, so
    until then the steps run with the stand-in, and the commands are still
    tested."""
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


_PINNED_RELEASE = re.compile(rf"^{re.escape(GITHUB)}/{re.escape(REPO)}/releases/download/v(\d+\.\d+\.\d+)/SHA256SUMS\.txt$")


def release_pinned(version: str, pins: dict) -> bool:
    """Whether vcon-server release `version` has its assets pinned: its
    SHA256SUMS.txt, as release_pin_problems asks."""
    return f"{GITHUB}/{REPO}/releases/download/v{version}/SHA256SUMS.txt" in pins


def newest_pinned(pins: dict) -> str | None:
    """The newest vcon-server release whose assets are pinned, or None."""
    versions = [m.group(1) for m in map(_PINNED_RELEASE.match, pins) if m]
    return max(versions, key=_release, default=None)


def release_plan(steps: list[Block], scenario_sets: dict, pins: dict, released) -> tuple[list[Block], dict, list[str]]:
    """`plan` for a run: a page's release runs as the page names it once it
    is published and pinned, and otherwise the newest pinned release stands
    in. A release is published before the follow-up that pins its assets
    lands, and a run downloads only pinned assets, so between the two the
    how-to still runs, on the newest release it can check. GitHub is asked
    only of a pinned release, newest first."""

    def available(v: str) -> bool:
        return release_pinned(v, pins) and released(v)

    def stand_in() -> str | None:
        versions = sorted({m.group(1) for m in map(_PINNED_RELEASE.match, pins) if m}, key=_release, reverse=True)
        return next((v for v in versions if released(v)), None)

    return plan(steps, scenario_sets, available, stand_in)


def plan(steps: list[Block], scenario_sets: dict, released, latest) -> tuple[list[Block], dict, list[str]]:
    """The steps a run takes, the variables it sets, and what it tells the
    reader of its log: the release it runs with in place of one not yet
    published, and each `since` step that release is too old for."""
    sets = release_sets(steps, scenario_sets, released, latest)
    notes = [SUBSTITUTED_NOTE.format(name=name, value=value) for name, value in sets.items() if name not in scenario_sets]
    steps, left_out = release_steps(steps, sets)
    notes += [
        f"{b.at} is for release {b.since} and later; this run's release is older, so it does not run"
        for b in left_out
    ]
    return steps, sets, notes


def _read_time(path: str) -> float | None:
    try:
        with open(path, encoding="utf-8") as f:
            return float(f.read().strip())
    except (OSError, ValueError):
        return None


def step_times(count: int, outdir: str) -> list[str]:
    """For each of `count` steps, " (at S s, for L s)": its start from the
    first recorded start, and its length, to the next time the run recorded
    (a later step's start, the end of the main steps, or the end of the
    last-by-rule ones). "" for a step with no readable start."""
    starts = [_read_time(os.path.join(outdir, f"{n}.start")) for n in range(count)]
    marks = sorted(t for t in starts + [_read_time(os.path.join(outdir, e)) for e in ("end", "finished")] if t is not None)
    known = [t for t in starts if t is not None]
    if not known:
        return [""] * count
    first = min(known)
    out = []
    for t in starts:
        if t is None:
            out.append("")
            continue
        later = [m for m in marks if m > t]
        length = f", for {later[0] - t:.2f} s" if later else ""
        out.append(f" (at {t - first:.2f} s{length})")
    return out


def report(steps: list[Block], outdir: str) -> int:
    """Compare each step with its page; print one line per step, with its
    time (step_times). Returns how many steps ran as their page says. When
    a step did not, prints the units' record of a systemd run
    (systemd.txt), if the run wrote one."""
    passed = 0
    stopped = False
    times = step_times(len(steps), outdir)
    for n, b in enumerate(steps):
        out_path = os.path.join(outdir, f"{n}.out")
        rc_path = os.path.join(outdir, f"{n}.rc")
        if stopped and not b.always:
            print(f"not run  {b.at}")
            continue
        got = open(out_path, encoding="utf-8", errors="replace").read() if os.path.exists(out_path) else ""
        if b.terminal is not None:
            # What the page shows is the screen, not the bytes that drew it.
            got = screen(got)
        rc = open(rc_path).read().strip() if os.path.exists(rc_path) else None
        why = None
        if rc is None:
            why = "it stopped the run: a command in it failed"
            stopped = True
        elif rc == "never":
            why = f"it never printed {b.background or b.terminal!r}"
            stopped = True
        elif rc != str(b.exit):
            why = f"it exited {rc}, the page expects {b.exit}"
            stopped = True
        elif b.expect is not None:
            why = mismatch(b.expect, got)
        if why is None:
            print(f"ok       {b.at}{times[n]}")
            passed += 1
            continue
        print(f"FAIL     {b.at}: {why}{times[n]}")
        print("  the block:")
        print("".join(f"    {l}\n" for l in b.code.rstrip().split("\n")), end="")
        if b.expect is not None:
            print("  the page says it prints:")
            print("".join(f"    {l}\n" for l in _clean(b.expect)), end="")
        print("  it printed:")
        print("".join(f"    {l}\n" for l in _clean(got)[-60:]), end="")
    record = os.path.join(outdir, "systemd.txt")
    if passed < len(steps) and os.path.exists(record):
        print("the units, as systemd saw them when the run ended:")
        with open(record, encoding="utf-8", errors="replace") as f:
            print("".join(f"    {l}\n" for l in f.read().rstrip("\n").split("\n")), end="")
    return passed


def run(scenario: Scenario, binary: str | None, keep: str | None) -> int:
    # First, before anything is fetched or started: with no package mirror
    # the run could only hang in its first step. A system without apt has no
    # origin (apt_origins), so nothing is asked for it.
    try:
        mirrors, notes = choose_mirrors(apt_suites(scenario.where, platform.machine()))
    except MirrorError as e:
        print(f"docs-test: {scenario.name}: {e}", file=sys.stderr)
        return 3
    for note in notes + mirror_notes(mirrors):
        print(note, flush=True)
    blocks: list[Block] = []
    for p in scenario.pages:
        blocks += page_blocks(p)
    steps = select(blocks, scenario)
    token = os.environ.get("GH_TOKEN") or None
    pins = read_pins()
    steps, sets, notes = release_plan(steps, scenario.sets, pins, lambda v: released(v, token))
    for note in notes:
        print(f"docs-test: {note}")
    missing = release_pin_problems(release_versions(steps, sets), pins)
    if missing:
        raise PageError("; ".join(missing))
    prelude = runner_prelude(scenario, steps, mirrors)
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
            prefetch(mirror, prefetch_urls(steps, sets, pins, platform.machine()))
            print(f"docs-test: release assets come through the mirror at {base}, each fetched from github.com "
                  f"with scripts/fetch-pinned.sh and checked against {PINS_NAME}")
            steps = [dataclasses.replace(b, code=rewrite_releases(b.code, base)) for b in steps]
        for image in base_images(steps):
            pull_with_retry(image)
        text = script(steps, prelude, sets, units=systemd_units(steps) if scenario.where == "systemd" else ())
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
            _run_in_container(scenario, binary, work, outdir, mirrors)
        passed = report(steps, outdir)
        if keep:
            shutil.copytree(work, keep, dirs_exist_ok=True)
        print(f"docs-test: {scenario.name}: {passed} of {len(steps)} steps ran as the pages say")
        return 0 if steps and passed == len(steps) else 1
    finally:
        if mirror is not None:
            mirror.stop()
        shutil.rmtree(work, ignore_errors=True)


def _run_in_container(scenario: Scenario, binary: str | None, work: str, outdir: str, mirrors: dict | None = None) -> None:
    name = f"docs-test-{scenario.name}-{os.getpid()}"
    if scenario.where == "systemd":
        pull_with_retry("debian:trixie-slim")
        _docker("build", "-q", "-t", SYSTEMD_IMAGE, "-", input=systemd_dockerfile(mirrors or {}).encode(), stdout=subprocess.DEVNULL)
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


# The scenarios a push runs (`push`, from .githooks/pre-push). The rule:
# a scenario runs when the push changes a page it runs; every scenario given
# the server under test runs when the push changes a file that server is
# built from; every scenario runs when the push changes the runner itself.
# Skipped, and named: a scenario that may take longer than a push hook
# should wait (CI runs it), and, with no GH_TOKEN, one whose steps call gh
# for something that needs a token.
SERVER_SOURCE_FILES = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")
SERVER_SOURCE_DIRS = ("crates/", ".cargo/")
RUNNER_FILES = ("scripts/docs_test.py", PINS_NAME, "scripts/fetch-pinned.sh", APT_MIRRORS_NAME)
# The longest a scenario may take to run from a push (a scenario's timeout).
PUSH_SCENARIO_SECONDS = 900
# gh calls that read GitHub's API, and so need a token; --help and
# --version read nothing.
GH_NEEDS_TOKEN = re.compile(r"\bgh\s+(?:attestation\s+verify|api|release)\b")
DOCS_PREFIX = "website/content/docs/"


_MOD = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
_CFG_TEST = re.compile(r"^#\[cfg\(\s*(?:test\s*|all\(\s*test\s*,.*\))\)\]$")


def _declarations(text: str, name: str) -> list[list[str]]:
    """The attributes of each `mod <name>;` in a module file: the `#[...]`
    lines directly above it (doc and comment lines between are passed)."""
    lines = text.splitlines()
    found = []
    for i, line in enumerate(lines):
        m = _MOD.match(line)
        if not m or m.group(1) != name:
            continue
        attributes = []
        for above in reversed(lines[:i]):
            above = above.strip()
            if above.startswith("#["):
                attributes.append(above)
            elif above.startswith("//"):
                continue
            else:
                break
        found.append(attributes)
    return found


def test_only(path: str, read, _depth: int = 0) -> bool:
    """Whether `path`, a file under a crate's src/, compiles only into the
    crate's tests: each `mod` declaration of its module carries
    `#[cfg(test)]` (or `cfg(all(test, ...))`), or sits in a module file that
    is itself test-only. `read(path)` gives a file's text at the commit, or
    None. A crate root, a file that is not Rust, a module whose declaration
    is not found or has `#[path]`, and an unreadable module file all count
    as server source (False)."""
    parts = path.split("/")
    if _depth > 32 or len(parts) < 4 or parts[0] != "crates" or parts[2] != "src" or not path.endswith(".rs"):
        return False
    directory, file = path.rsplit("/", 1)
    name = file[:-3]
    if name == "mod":
        directory, name = directory.rsplit("/", 1)
    if directory.endswith("/src") and len(directory.split("/")) == 3:
        if name in ("lib", "main", "build"):
            return False
        parents = [f"{directory}/lib.rs", f"{directory}/main.rs"]
    else:
        parents = [f"{directory}.rs", f"{directory}/mod.rs"]
    declared = []
    for parent in parents:
        text = read(parent)
        if text is None:
            continue
        for attributes in _declarations(text, name):
            declared.append((parent, attributes))
    if not declared:
        return False
    for parent, attributes in declared:
        if any(a.startswith("#[path") for a in attributes):
            return False
        if not (any(_CFG_TEST.match(a) for a in attributes) or test_only(parent, read, _depth + 1)):
            return False
    return True


# An include macro, and the path it reads when its argument is one plain
# string literal (no escape, not raw, not built by another macro).
INCLUDE_GREP = r"\binclude(_str|_bytes)?!"  # git grep -E (Git.includes) and Python read it alike
INCLUDE_MACRO = re.compile(INCLUDE_GREP)
_INCLUDE_LITERAL = re.compile(r'\binclude(?:_str|_bytes)?!\s*\(\s*"([^"\\]*)"\s*\)')


def crate_readme(path: str, read, includes) -> bool:
    """Whether `path` is the readme a crate's manifest names (`readme` in
    [package], a string) and no build reads it: cargo packages it for
    crates.io, so scripts/check_package.py checks it, but no scenario runs
    it. `read(path)` gives a file's text at the commit, or None;
    `includes()` gives (file, line) for each line of the crates' Rust files
    that holds an include macro, or None when git cannot tell. It counts as
    read by a build (False) when an include's literal path, taken from the
    file that holds it, is `path`; when an include on a line is not a plain
    literal (it may name any file); when the crate's build script names the
    file; and whenever the manifest or the includes cannot be read. Comments
    are not parsed, so a commented-out include counts."""
    parts = path.split("/")
    if len(parts) < 3 or parts[0] != "crates":
        return False
    crate = "/".join(parts[:2])
    manifest = read(f"{crate}/Cargo.toml")
    if manifest is None:
        return False
    try:
        package = tomllib.loads(manifest).get("package")
    except tomllib.TOMLDecodeError:
        return False
    if not isinstance(package, dict) or not isinstance(package.get("readme"), str):
        return False
    if posixpath.normpath(f"{crate}/{package['readme']}") != path:
        return False
    name = posixpath.basename(path)
    build = package.get("build", "build.rs")
    if isinstance(build, str):
        script = read(posixpath.normpath(f"{crate}/{build}"))
        if script is not None and name in script:
            return False
    lines = includes()
    if lines is None:
        return False
    for file, line in lines:
        literals = _INCLUDE_LITERAL.findall(line)
        if len(literals) != len(INCLUDE_MACRO.findall(line)):
            return False
        for literal in literals:
            if posixpath.normpath(posixpath.join(posixpath.dirname(file), literal)) == path:
                return False
    return True


def _server_source(path: str, test_only=None) -> bool:
    if path in SERVER_SOURCE_FILES:
        return True
    if not path.startswith(SERVER_SOURCE_DIRS):
        return False
    # A crate's tests are not built into the server.
    parts = path.split("/")
    if len(parts) > 2 and parts[0] == "crates" and parts[2] == "tests":
        return False
    # Nor is a file only `#[cfg(test)] mod` declarations reach, or the
    # crate's readme when no build reads it (test_only_at says which).
    return not (test_only is not None and len(parts) > 2 and test_only(path))


def _scenario_steps(s: Scenario) -> list[Block]:
    return select([b for p in s.pages for b in page_blocks(p)], s)


def needed(changed, scenarios, token: bool, steps=_scenario_steps, test_only=None) -> tuple[list, list]:
    """The scenarios a push of `changed` runs and those it skips, each as
    (name, why), in the order of `scenarios`. `test_only(path)` says whether
    a file under a crate's src/ compiles only into its tests (None: every
    such file is server source)."""
    paths = [p.strip() for p in changed if p.strip()]
    pages = [p[len(DOCS_PREFIX) :] for p in paths if p.startswith(DOCS_PREFIX)]
    source = next((p for p in paths if _server_source(p, test_only)), None)
    runner = next((p for p in paths if p in RUNNER_FILES), None)
    run, skipped = [], []
    for s in scenarios:
        page = next((p for p in pages if p in s.pages), None)
        if page is not None:
            why = f"the push changes docs/{page}, which it runs"
        elif source is not None and s.binary:
            why = f"the push changes {source}, which the server under test is built from"
        elif runner is not None:
            why = f"the push changes {runner}, which runs every scenario"
        else:
            continue
        if s.timeout > PUSH_SCENARIO_SECONDS:
            skipped.append((s.name, f"it may take up to {s.timeout // 60} min; CI runs it"))
        elif not token and any(
            GH_NEEDS_TOKEN.search(b.code) for b in steps(s) if b.kind == "sh" and b.skip is None
        ):
            skipped.append((s.name, "its steps call gh, which needs GH_TOKEN here; CI runs it"))
        else:
            run.append((s.name, why))
    return run, skipped


def _git_dir() -> str:
    return subprocess.run(
        ["git", "rev-parse", "--absolute-git-dir"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.strip()


def _stamp_path(sha: str, git_dir: str | None) -> str:
    return os.path.join(git_dir or _git_dir(), f"docs-scenarios-{sha}.pass")


def _stamp_entries(entries) -> dict:
    """A stamp's scenarios as {name: release note}, from a dict or from
    names alone (no note)."""
    return dict(entries) if isinstance(entries, dict) else dict.fromkeys(entries, "")


def write_stamp(sha: str, entries, git_dir: str | None = None) -> None:
    """Records that the scenarios of `entries` ran as their pages say on
    commit `sha`: one a line, with the releases it ran in place of a page's
    unpublished one after it ("container VERSION=0.4.6")."""
    with open(_stamp_path(sha, git_dir), "w", encoding="utf-8") as f:
        f.write("".join(f"{n} {note}\n" if note else f"{n}\n" for n, note in _stamp_entries(entries).items()))


def read_stamp(sha: str, git_dir: str | None = None) -> dict | None:
    """The scenarios that passed on `sha`, each with its release note ("" for
    none), or None when none were recorded."""
    try:
        with open(_stamp_path(sha, git_dir), encoding="utf-8") as f:
            return {name: note.strip() for name, _, note in (l.strip().partition(" ") for l in f) if name}
    except FileNotFoundError:
        return None


def changed_since(base: str, sha: str) -> list[str]:
    """The paths the commits from `base` (their merge base) to `sha` change."""
    return subprocess.run(
        ["git", "diff", "--name-only", f"{base}...{sha}"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.splitlines()


ZERO_SHA = "0" * 40


@dataclasses.dataclass(frozen=True)
class Ref:
    """One line of what git gives pre-push on standard input."""

    local_ref: str
    local_sha: str
    remote_ref: str
    remote_sha: str


def parse_refs(lines) -> list[Ref]:
    """The refs of pre-push's lines; blank lines are left out. A line
    without four fields is a PageError naming it."""
    refs = []
    for line in lines:
        fields = line.split()
        if not fields:
            continue
        if len(fields) != 4:
            raise PageError(f"not a pushed ref (<local ref> <local sha> <remote ref> <remote sha>): {line.strip()}")
        refs.append(Ref(*fields))
    return refs


class Git:
    """The git questions the stamps rule asks, as pre-push asks them. Each
    failure is a PageError with git's message."""

    def __init__(self, cwd: str = ROOT, env=None, run=subprocess.run):
        self.cwd, self.env, self.run = cwd, env, run

    def _call(self, *args: str) -> subprocess.CompletedProcess:
        return self.run(["git", *args], cwd=self.cwd, env=self.env, capture_output=True, text=True)

    def _lines(self, *args: str) -> list[str]:
        done = self._call(*args)
        if done.returncode != 0:
            raise PageError(done.stderr.strip() or f"git {args[0]} exited {done.returncode}")
        return [l for l in done.stdout.splitlines() if l.strip()]

    @staticmethod
    def _range(ref: Ref, remote: str) -> list[str]:
        # As .githooks/pre-push reads it: a new ref holds the commits the
        # remote has on no branch.
        if ref.remote_sha == ZERO_SHA:
            return [ref.local_sha, "--not", f"--remotes={remote}"]
        return [f"{ref.remote_sha}..{ref.local_sha}"]

    def commits(self, ref: Ref, remote: str) -> list[str]:
        """The commits the push of `ref` sends, newest first."""
        return self._lines("rev-list", *self._range(ref, remote))

    def changed(self, ref: Ref, remote: str) -> list[str]:
        """The paths each of those commits changes."""
        return self._lines("log", "--name-only", "--format=", *self._range(ref, remote))

    def diff(self, a: str, b: str) -> list[str]:
        """The paths whose content differs between the trees of `a` and `b`."""
        return self._lines("diff", "--name-only", a, b)

    def show(self, sha: str, path: str) -> str | None:
        """The text of `path` in commit `sha`, or None when git cannot give it."""
        done = self._call("show", f"{sha}:{path}")
        return done.stdout if done.returncode == 0 else None

    def includes(self, sha: str) -> list[tuple[str, str]] | None:
        """(path, line) for each line of a Rust file under crates/ in commit
        `sha` that holds an include macro (`crate_readme` reads them), or
        None when git cannot give them."""
        done = self._call("grep", "-z", "-n", "-I", "-E", INCLUDE_GREP, sha, "--", "crates/")
        if done.returncode == 1:
            return []
        if done.returncode != 0:
            return None
        found = []
        for record in done.stdout.splitlines():
            # <sha>:<path> NUL <line number> NUL <text>
            name, _, rest = record.partition("\0")
            text = rest.partition("\0")[2]
            path = name[len(sha) + 1 :] if name.startswith(f"{sha}:") else name
            if path.endswith(".rs"):
                found.append((path, text))
        return found

    def rev_list(self, *args: str) -> list[str]:
        return self._lines("rev-list", *args)

    def is_ancestor(self, a: str, b: str) -> bool:
        done = self._call("merge-base", "--is-ancestor", a, b)
        if done.returncode in (0, 1):
            return done.returncode == 0
        raise PageError(done.stderr.strip() or f"git merge-base exited {done.returncode}")


def range_commits(base: str, sha: str) -> list[str]:
    """The commits from `base` to `sha`, newest first."""
    return Git().rev_list(sha, "--not", base)


def affected(paths, scenarios, steps=_scenario_steps, test_only=None) -> set:
    """The scenarios a change of `paths` would need. With a token, `needed`
    skips only a scenario longer than a push waits for, and a push never
    needs one of those, so the ones it runs are all that can be affected."""
    return {n for n, _ in needed(paths, scenarios, True, steps, test_only)[0]}


def test_only_at(sha: str, show, includes=None) -> "callable":
    """`needed`'s test_only for the tree of commit `sha`: whether a file
    under crates/ is built into no server (a test-only module, or the crate
    readme no build reads). Each file is read once with `show(sha, path)`,
    and the include lines once with `includes(sha)`, only when a readme is
    asked about. Without `includes` every readme counts as server source."""
    texts: dict = {}
    found: list = []

    def read(path):
        if path not in texts:
            texts[path] = show(sha, path)
        return texts[path]

    def lines():
        if not found:
            found.append(includes(sha) if includes is not None else None)
        return found[0]

    return lambda path: test_only(path, read) or crate_readme(path, read, lines)


def carried(names, tip: str, commits, read, delta, scenarios, steps=_scenario_steps, test_only=None) -> dict:
    """Which of `names` hold for `tip` on a stamp, and the commit of that
    stamp: one of `commits` (newest first, the tip among them) whose stamp
    names the scenario, when what differs between that commit's tree and
    the tip's (`delta(sha)`) is nothing the scenario runs. A scenario reads
    only its pages, the server it is given and the runner, so its result on
    that commit is its result on the tip."""
    found: dict = {}
    for sha in commits:
        left = [n for n in names if n not in found]
        if not left:
            break
        stamp = read(sha) or set()
        here = [n for n in left if n in stamp]
        if not here:
            continue
        voided = set() if sha == tip else affected(delta(sha), scenarios, steps, test_only)
        for n in here:
            if n not in voided:
                found[n] = sha
    return found


CI_WORKFLOW = ".github/workflows/ci.yml"
# A run of these events checks out the commit itself. A pull_request run
# checks out GitHub's merge of the commit into its base branch instead.
EXACT_EVENTS = ("push", "workflow_dispatch")


def ci_job_name(scenario: str) -> str:
    """The name of CI's how-to job for `scenario` (ci.yml, job docs)."""
    return f"how-to {scenario} against a live server"


def gh_api(path: str, run=subprocess.run, clock=time.time, sleep=time.sleep, err=sys.stderr) -> list:
    """Every page of a GitHub API listing, read with gh (its own login, or
    GH_TOKEN), as a list of the pages' bodies. A spent API budget is waited
    for, at most 900 s (scripts/gh_rate_limit.py)."""
    try:
        done = gh_rate_limit.run_gh(
            ["gh", "api", "--paginate", "--slurp", path], run=run, clock=clock, sleep=sleep, err=err,
            capture_output=True, text=True, timeout=60,
        )
    except (OSError, subprocess.TimeoutExpired) as e:
        raise PageError(f"gh api {path}: {e}") from None
    if done.returncode != 0:
        raise PageError(f"gh api {path}: {(done.stderr or '').strip() or f'gh exited {done.returncode}'}")
    try:
        return json.loads(done.stdout)
    except ValueError:
        raise PageError(f"gh api {path}: the answer is not JSON") from None


def ci_results(tip: str, names, *, pull_requests_ok: bool, api=gh_api) -> tuple[dict, dict]:
    """Which of `names` CI's how-to jobs passed on commit `tip` itself, each
    with the job and run that show it, and why each other one does not
    count. Only runs of ci.yml on `tip` are read; for each scenario the
    newest such run that has its job decides, and only a completed,
    successful job on `tip` counts. A pull_request run tested GitHub's
    merge of `tip` into main, which has `tip`'s tree only when that main is
    an ancestor of `tip`: it counts only when `pull_requests_ok` says the
    push is a fast-forward of main (main only moves forward, so the main
    of any earlier run is an ancestor too)."""
    names = list(names)
    if not names:
        return {}, {}
    passed: dict = {}
    problems: dict = {}
    try:
        pages = api(f"repos/{REPO}/actions/runs?head_sha={tip}&per_page=100")
        usable, pull_request_only = [], False
        for r in (r for p in pages for r in p.get("workflow_runs", [])):
            if r.get("path") != CI_WORKFLOW or r.get("head_sha") != tip:
                continue
            if r.get("event") in EXACT_EVENTS or (r.get("event") == "pull_request" and pull_requests_ok):
                usable.append(r)
            elif r.get("event") == "pull_request":
                pull_request_only = True
        usable.sort(key=lambda r: (r.get("created_at") or "", r.get("id") or 0), reverse=True)
        left = list(names)
        for r in usable:
            if not left:
                break
            jobs = {
                j.get("name"): j
                for p in api(f"repos/{REPO}/actions/runs/{r['id']}/jobs?per_page=100")
                for j in p.get("jobs", [])
            }
            for n in list(left):
                j = jobs.get(ci_job_name(n))
                if j is None:
                    continue
                left.remove(n)
                if j.get("head_sha") != tip:
                    problems[n] = f"CI's job for it in run {r['id']} ran on {j.get('head_sha')}, not {tip}"
                elif j.get("status") == "completed" and j.get("conclusion") == "success":
                    passed[n] = f'CI\'s job "{ci_job_name(n)}" passed on {tip} (run {r["id"]})'
                else:
                    state = j.get("conclusion") or j.get("status") or "unknown"
                    problems[n] = f"CI's newest job for it on {tip} (run {r['id']}) is {state}"
    except (PageError, KeyError, TypeError, AttributeError) as e:
        return {}, {n: f"CI's results for {tip} cannot be read: {e}" for n in names}
    for n in left:
        problems[n] = (
            f"CI ran it on {tip} only for a pull request, which tests the merge with main; that merge is this "
            "commit's tree only on a fast-forward push to main"
            if pull_request_only
            else f"CI has run no how-to job for it on {tip}"
        )
    return passed, problems


def check(
    refs,
    *,
    remote: str = "origin",
    env=os.environ,
    scenarios=SCENARIOS,
    steps=_scenario_steps,
    read=read_stamp,
    git=None,
    ci=ci_results,
    out=sys.stdout,
) -> int:
    """What .githooks/pre-push asks of each pushed ref: 0 when every
    scenario the ref's commits need (or none) has passed on its tip: on a
    stamp of the tip, on a stamp of a commit of the range that holds for the
    tip (`carried`), or in CI's how-to job on the tip itself (`ci_results`);
    1 otherwise, naming what is missing and the command that runs it. It
    builds and runs nothing: the connection `git push` holds to GitHub
    would not wait for the scenarios."""
    git = git or Git()

    def say(line: str) -> None:
        print(line, file=out, flush=True)

    refused = False
    for ref in refs:
        tip = ref.local_sha
        try:
            commits = git.commits(ref, remote)
            if not commits:
                continue
            changed = git.changed(ref, remote)
        except PageError as e:
            say(f"docs scenarios: refused: cannot read the commits pushed to {ref.remote_ref}: {e}")
            refused = True
            continue
        at_tip = test_only_at(tip, git.show, git.includes)
        todo, skipped = needed(changed, scenarios, bool(env.get("GH_TOKEN")), steps, at_tip)
        for name, why in skipped:
            say(f"docs scenarios: skipped {name}: {why}")
        if not todo:
            if not skipped:
                say(
                    f"docs scenarios: not needed for {ref.remote_ref}: the push changes no page a scenario runs, "
                    "no file the server is built from and no file of the runner"
                )
            continue
        names = [n for n, _ in todo]
        try:
            held = carried(names, tip, commits, read, lambda sha: git.diff(sha, tip), scenarios, steps, at_tip)
            missing = [n for n in names if n not in held]
            pull_requests_ok = (
                bool(missing)
                and ref.remote_ref == "refs/heads/main"
                and ref.remote_sha != ZERO_SHA
                and git.is_ancestor(ref.remote_sha, tip)
            )
        except PageError as e:
            say(f"docs scenarios: refused: cannot compare the commits pushed to {ref.remote_ref}: {e}")
            refused = True
            continue
        for n in names:
            if n not in held:
                continue
            ran = release_words(_stamp_entries(read(held[n]) or {}).get(n, ""))
            ran = f"; {ran}" if ran else ""
            if held[n] == tip:
                say(f"docs scenario {n}: passed on {tip} before this push{ran}")
            else:
                say(f"docs scenario {n}: passed on {held[n]}, and the commits after it change nothing it runs{ran}")
        if not missing:
            continue
        passed, problems = ci(tip, missing, pull_requests_ok=pull_requests_ok)
        for n in missing:
            say(f"docs scenario {n}: {passed[n] if n in passed else problems.get(n, 'not passed')}")
        still = [n for n in missing if n not in passed]
        if still:
            refused = True
            say(
                f"docs scenarios: {', '.join(still)} have not passed on {tip}; run them first, from a checkout of "
                f"that commit: python3 -I scripts/docs_test.py push {tip} --since origin/main"
            )
    return 1 if refused else 0


def push(
    shas,
    changed,
    *,
    run=subprocess.run,
    env=os.environ,
    scenarios=SCENARIOS,
    steps=_scenario_steps,
    copy=shutil.copy2,
    tempdir=tempfile.mkdtemp,
    out=sys.stdout,
    clock=time.monotonic,
    stamp=write_stamp,
    read=read_stamp,
    commits=None,
    delta=None,
    test_only=None,
) -> int:
    """Runs the scenarios a push of `shas` that changes `changed` needs,
    with the server built from the checkout, which must be that commit,
    and stamps the commit when each ran as its pages say (`check` reads
    the stamp), with the release each ran in place of a page's unpublished
    one. A scenario a stamp already holds for the commit is not run again
    (`carried`): one of the commit's own, or of one of `commits` (the
    range's, newest first; `delta(sha)` names what differs between that
    commit and this one). `test_only` is `needed`'s, for the commit's tree.
    0 when each ran so (or none was needed), 1 otherwise."""

    def say(line: str) -> None:
        print(line, file=out, flush=True)

    todo, skipped = needed(changed, scenarios, bool(env.get("GH_TOKEN")), steps, test_only)
    for name, why in skipped:
        say(f"docs scenarios: skipped {name}: {why}")
    if not todo:
        if not skipped:
            say(
                "docs scenarios: not needed: the push changes no page a scenario runs, "
                "no file the server is built from and no file of the runner"
            )
        return 0
    pushed = sorted(set(shas))
    if len(pushed) != 1:
        say(f"docs scenarios: refused: the push holds {len(pushed)} commits that need them; push one branch at a time")
        return 1
    tip = pushed[0]
    held = carried(
        [n for n, _ in todo], tip, commits or [tip], read, delta or (lambda sha: []), scenarios, steps, test_only
    )
    for name, _ in todo:
        if name not in held:
            continue
        ran = release_words(_stamp_entries(read(held[name]) or {}).get(name, ""))
        ran = f"; {ran}" if ran else ""
        if held[name] == tip:
            say(f"docs scenario {name}: passed on {tip} before{ran}; not run again")
        else:
            say(f"docs scenario {name}: passed on {held[name]}, and the commits after it change nothing it runs"
                f"{ran}; not run again")
    todo = [(n, why) for n, why in todo if n not in held]
    if not todo:
        say("docs scenarios: each one this push needs has passed already")
        return 0

    def git(*args):
        return run(["git", *args], cwd=ROOT, capture_output=True, text=True)

    head = git("rev-parse", "HEAD")
    if head.returncode != 0:
        say("docs scenarios: refused: git cannot read which commit is checked out")
        return 1
    if head.stdout.strip() != pushed[0]:
        say(
            f"docs scenarios: refused: the checkout is at {head.stdout.strip()}, not the pushed {pushed[0]}; "
            "the scenarios run the server built from the checkout, so push from a checkout of that commit"
        )
        return 1
    status = git("status", "--porcelain", "--untracked-files=no")
    if status.returncode != 0 or status.stdout.strip():
        say("docs scenarios: refused: the checkout has uncommitted changes, so its server is not the pushed commit's")
        return 1
    by_name = {s.name: s for s in scenarios}
    target = env.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
    builds = tempdir()
    try:
        binaries: dict = {}
        for name, _ in todo:
            features = by_name[name].features
            if features in binaries:
                continue
            command = ["cargo", "build", "--locked", "-p", "vcon-server"]
            if features:
                command += ["--features", ",".join(features)]
            built = run(command, cwd=ROOT, capture_output=True, text=True)
            if built.returncode != 0:
                say(f"docs scenarios: refused: {' '.join(command)} failed:")
                say("".join(f"  {l}\n" for l in (built.stdout + built.stderr).splitlines()[-40:]).rstrip())
                return 1
            place = os.path.join(builds, "-".join(features) or "default")
            os.makedirs(place)
            binaries[features] = os.path.join(place, "vcon-server")
            copy(os.path.join(target, "debug", "vcon-server"), binaries[features])
        passed = 0
        ran: dict = {}
        for name, why in todo:
            started = clock()
            done = run(
                [sys.executable, "-I", os.path.join(ROOT, "scripts", "docs_test.py"), "run", name,
                 "--binary", binaries[by_name[name].features]],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            seconds = int(clock() - started)
            if done.returncode == 0:
                passed += 1
                ran[name] = ",".join(substituted(done.stdout))
                words = release_words(ran[name])
                say(f"docs scenario {name}: ok in {seconds} s ({why}){'; ' + words if words else ''}")
            else:
                say(f"docs scenario {name}: FAILED in {seconds} s ({why}); the end of its report:")
                say("".join(f"  {l}\n" for l in (done.stdout + done.stderr).splitlines()[-60:]).rstrip())
        say(f"docs scenarios: {passed} of {len(todo)} ran as their pages say")
        if passed != len(todo):
            return 1
        merged = {**_stamp_entries(read(tip) or {}), **ran}
        stamp(tip, dict(sorted(merged.items())))
        return 0
    finally:
        shutil.rmtree(builds, ignore_errors=True)


def stdin_lines() -> list[str]:
    """The lines of standard input, read as bytes and decoded as Python
    names a file: UTF-8, each byte that is not kept as a surrogate escape.
    git prints a path's bytes as they are when core.quotePath is false, and
    a ref name may hold any byte; neither is a crash."""
    return sys.stdin.buffer.read().decode("utf-8", errors="surrogateescape").splitlines()


def main(argv: list[str]) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if argv else 2
    command, rest = argv[0], argv[1:]
    if command == "list":
        print(json.dumps([s.name for s in SCENARIOS]))
        return 0
    if command == "push":
        if not rest:
            print(
                "docs-test: push needs the commit: docs_test.py push <sha> [--since <rev>] (or the changed paths on stdin)",
                file=sys.stderr,
            )
            return 2
        if len(rest) == 3 and rest[1] == "--since":
            sha, base = rest[0], rest[2]
            return push(
                rest[:1],
                changed_since(base, sha),
                commits=range_commits(base, sha),
                delta=lambda other: Git().diff(other, sha),
                test_only=test_only_at(sha, Git().show, Git().includes),
            )
        return push(rest, stdin_lines(), test_only=test_only_at(rest[0], Git().show, Git().includes))
    if command == "check":
        usage = "docs-test: check reads the pushed refs, as pre-push gets them, on stdin: docs_test.py check [--remote <name>]"
        if rest and not (len(rest) == 2 and rest[0] == "--remote"):
            print(usage, file=sys.stderr)
            return 2
        try:
            refs = parse_refs(stdin_lines())
        except PageError as e:
            print(f"docs-test: {e}", file=sys.stderr)
            return 2
        if not refs:
            print(usage, file=sys.stderr)
            return 2
        return check(refs, remote=rest[1] if rest else "origin")
    if command == "lint":
        problems = lint()
        for p in problems:
            print(p)
        return 1 if problems else 0
    if command == "artifact" and len(rest) == 1:
        found = [s for s in SCENARIOS if s.name == rest[0]]
        if not found:
            print(f"docs-test: no scenario {rest[0]}; see `list`", file=sys.stderr)
            return 2
        print(artifact(found[0]))
        return 0
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
    # A path or ref with a byte that is not UTF-8 prints as \udcXX.
    sys.stdout.reconfigure(encoding="utf-8", errors="backslashreplace")
    sys.stderr.reconfigure(encoding="utf-8", errors="backslashreplace")
    sys.exit(main(sys.argv[1:]))
