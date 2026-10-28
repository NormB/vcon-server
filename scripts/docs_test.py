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
    <!-- docs-test: skip <reason> -->      never run; the reason is required
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

Usage:
  python3 -I scripts/docs_test.py list               the scenario names, as JSON
  python3 -I scripts/docs_test.py lint               the rules every tested page keeps
  python3 -I scripts/docs_test.py run <scenario> [--binary <vcon-server>] [--keep <dir>]
"""

from __future__ import annotations

import dataclasses
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
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


MARKER = re.compile(r"^<!--\s*docs-test:\s*(.*?)\s*-->\s*$")
FENCE = re.compile(r"^```\s*([A-Za-z0-9_+-]*)")


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
            if lang == "text" and open_step is not None and not given:
                open_step.expect = code
                open_step = None
                continue
            open_step = None
            if lang in ("sh", "bash"):
                if "path" in given:
                    raise PageError(f"{at}: `file` is for blocks that are not commands")
                block = Block(kind="sh", code=code, at=at, **given)
            elif "path" in given:
                block = Block(kind="file", code=code, at=at, **given)
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


# The image of the systemd scenario: Debian with systemd as its init, and
# nothing else the pages would install.
SYSTEMD_IMAGE = "docs-test-systemd:trixie"
SYSTEMD_DOCKERFILE = """FROM debian:trixie-slim
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends systemd systemd-sysv dbus >/dev/null \\
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
AS_A_HOST = ("--cap-add", "SYS_RESOURCE")

HOWTO = ("getting-started.md", "first-vcon.md")

SCENARIOS = [
    # The how-tos, on a Debian and a Fedora with no systemd: the server runs
    # in the foreground.
    Scenario("foreground", "debian:trixie-slim", frozenset({"apt", "no-systemd"}), HOWTO, docker=AS_A_HOST),
    Scenario("foreground-fedora", "fedora:44", frozenset({"dnf", "no-systemd"}), HOWTO, docker=AS_A_HOST),
    # The same, under systemd. No memory-lock option: the unit and the
    # limits file are what allow it, as on a host.
    Scenario("systemd", "systemd", frozenset({"apt", "systemd"}), HOWTO),
    # Each install path, from a system with nothing installed.
    Scenario("release-debian", "debian:trixie-slim", frozenset({"apt", "release"}), ("install.md",), binary=False),
    Scenario("release-ubuntu", "ubuntu:24.04", frozenset({"apt", "release"}), ("install.md",), binary=False),
    Scenario("release-fedora", "fedora:44", frozenset({"dnf", "release"}), ("install.md",), binary=False),
    Scenario("cargo-debian", "debian:trixie-slim", frozenset({"apt", "cargo"}), ("install.md",), binary=False, timeout=3600),
    Scenario("cargo-fedora", "fedora:44", frozenset({"dnf", "cargo"}), ("install.md",), binary=False, timeout=3600),
    Scenario("source-debian", "debian:trixie-slim", frozenset({"apt", "source"}), ("install.md",), binary=False, timeout=3600),
    Scenario("source-fedora", "fedora:44", frozenset({"dnf", "source"}), ("install.md",), binary=False, timeout=3600),
]


def select(blocks: list[Block], scenario: Scenario) -> list[Block]:
    """The blocks a scenario runs, in page order."""
    return [b for b in blocks if b.skip is None and b.only <= scenario.tags]


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
    for s in main:
        parts.append(_step(index[id(s)], s))
    return "".join(parts)


def page_blocks(name: str) -> list[Block]:
    path = os.path.join(DOCS, name)
    with open(path, encoding="utf-8") as f:
        return parse(f.read(), f"docs/{name}")


SERVER_REQUEST = re.compile(r"\bcurl\b[^\n]*https://vcon\.example\.com|https://vcon\.example\.com[^\n]*\bcurl\b")


def lint_page(blocks: list[Block], tag_sets: list[frozenset], host: bool = False) -> list[str]:
    problems = []
    for b in blocks:
        if b.skip is not None:
            continue
        if not any(b.only <= tags for tags in tag_sets):
            problems.append(f"{b.at}: no scenario runs this block (only {' '.join(sorted(b.only))})")
        if b.kind == "sh" and b.expect is None and b.background is None:
            joined = b.code.replace("\\\n", " ")
            if SERVER_REQUEST.search(joined):
                problems.append(f"{b.at}: a request to the server with no output shown after it")
        if host and b.background is not None:
            problems.append(f"{b.at}: a background step on this host would outlive the run")
    return problems


def lint() -> list[str]:
    """Every rule, over every page a scenario runs."""
    problems: list[str] = []
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


def _latest_release() -> str | None:
    """The newest published release's version, from the GitHub API."""
    out = subprocess.run(
        ["curl", "-fsSL", "--retry", "3", "https://api.github.com/repos/NormB/vcon-server/releases/latest"],
        capture_output=True,
        text=True,
        timeout=60,
    )
    if out.returncode != 0:
        return None
    return json.loads(out.stdout)["tag_name"].removeprefix("v")


def _released(version: str) -> bool:
    out = subprocess.run(
        ["curl", "-fsSL", "-o", "/dev/null", "--retry", "3",
         f"https://api.github.com/repos/NormB/vcon-server/releases/tags/v{version}"],
        timeout=60,
    )
    return out.returncode == 0


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
    sets = release_sets(steps, scenario.sets, _released, _latest_release)
    for name, value in sets.items():
        if name not in scenario.sets:
            print(f"docs-test: the release a page names is not published yet; running with {name}={value}")
    prelude = ""
    if scenario.binary:
        if not binary:
            print(f"docs-test: {scenario.name} needs --binary", file=sys.stderr)
            return 2
        if scenario.where != "host" and scenario.install_binary:
            prelude = "install -m 0755 /docs-test/vcon-server /usr/local/bin/vcon-server\n"
    text = script(steps, prelude, sets)
    work = tempfile.mkdtemp(prefix=f"docs-test-{scenario.name}-")
    try:
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
        shutil.rmtree(work, ignore_errors=True)


def _run_in_container(scenario: Scenario, binary: str | None, work: str, outdir: str) -> None:
    name = f"docs-test-{scenario.name}-{os.getpid()}"
    if scenario.where == "systemd":
        _docker("build", "-q", "-t", SYSTEMD_IMAGE, "-", input=SYSTEMD_DOCKERFILE.encode(), stdout=subprocess.DEVNULL)
        start = ["run", "-d", "--name", name, *SYSTEMD_RUN, *scenario.docker, SYSTEMD_IMAGE]
    else:
        start = ["run", "-d", "--name", name, *scenario.docker, scenario.where, "sleep", "infinity"]
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
