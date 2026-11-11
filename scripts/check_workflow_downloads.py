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

  dnf / yum / microdnf install                             through scripts/dnf_install.py, and nowhere else

A release asset (a github.com/<owner>/<repo>/releases/download/ URL) is
more than a download: it is a pinned tool. Each one goes through ONE rule,
.github/actions/pinned-download (cached by name, version and sha256) or
scripts/fetch-pinned.sh (which checks the sha256 of every download and every
restored file, and retries an outage for up to five minutes). On 2026-11-08
and 2026-11-09 github.com answered release downloads with HTTP 504 for most
of an hour, and main's CI went red at every job with a one-curl install.
So a step that names a release asset and runs curl or wget, or names it
without that action or that script, is refused; a shell script that names
one may not run curl or wget and must call fetch-pinned.sh. A cache entry
is saved only on main (actions/cache/save with a main-only `if:`; never the
combined actions/cache), a workflow run by workflow_run uses no cache, and
the action keeps its one-rule shape. Fewer
than MIN_RELEASE_DOWNLOADS release downloads through the rule fails too, so
a reading that lost the steps cannot pass.

dnf's own retries cover a failed download, not a mirror that serves a
repomd.xml it cannot parse (seen on 2026-10-28 in Release's almalinux:9
job). A retry from fresh metadata was the rule until 2026-11-11, when five
such tries in that job each read the same mirror and failed the same way.
scripts/dnf_install.py follows a failed try with one from another source,
so no other form of `dnf install` passes, and fewer than
MIN_DNF_RULE_CALLS installs through it fails. A job in a container has no
`run:` step before its checkout: no repository script exists there, so
such a step could install only with the package manager's own command.
actions/setup-* download through the actions toolkit, which retries.

apt's retries ask the same host again, and a container's image names one
host for each package source: on 2026-11-10 archive.ubuntu.com did not
answer on port 80 and an `apt-get update` in an ubuntu container did not
end. scripts/apt-mirror-setup.sh gives apt a list of hosts that answer,
from the table in scripts/apt-mirrors.txt. So an apt-get update or install
in a job that runs in a container comes after a run of that script in the
same job, and one in a composite action or a shell script after a run of it
in the same file; fewer than MIN_APT_RULE_CALLS runs of it fails.

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
        for _ in DNF.finditer(code):
            # No form of it passes: a retry of the same command reads the
            # same mirror, whatever is cleaned in between.
            problems.append(f"{path}:{n}: dnf install outside scripts/dnf_install.py, which tries a second source")
        for m in APT.finditer(code):
            if "Acquire::Retries" not in m.group(0):
                problems.append(f"{path}:{n}: apt-get without -o Acquire::Retries")
        if CURL.search(code) and "--retry" not in code and REMOTE_URL.search(code):
            problems.append(f"{path}:{n}: curl without --retry")
    return problems


DNF_RULE = re.compile(r"\bpython3\s+-I\s+(?:\S*/)?scripts/dnf_install\.py\s+\S")
# release.yml's smoke job, and scripts/release-build-env.sh, which
# release.yml's build job and reproducible.yml's job run.
MIN_DNF_RULE_CALLS = 2


def dnf_rule_calls(text):
    """How many command lines in `text` install through scripts/dnf_install.py."""
    return sum(1 for _, line in logical_lines(text) if _code(line) and DNF_RULE.search(line))


# scripts/apt-mirror-setup.sh run as a command, by bash: the script needs it.
APT_RULE = re.compile(r"""\bbash\s+["']?(?:[^\s"']*/)?scripts/apt-mirror-setup\.sh(?=["'\s]|$)""")
# release.yml's smoke job.
MIN_APT_RULE_CALLS = 1


def apt_rule_calls(text):
    """How many command lines in `text` run scripts/apt-mirror-setup.sh."""
    return sum(1 for _, line in logical_lines(text) if _code(line) and APT_RULE.search(line))


def _apt_before_rule(path, numbered):
    """One line per apt-get update or install in the (line number, text)
    pairs `numbered` that no run of scripts/apt-mirror-setup.sh comes
    before."""
    problems = []
    ready = False
    for n, line in numbered:
        if not _code(line):
            continue
        events = [(m.start(), True) for m in APT_RULE.finditer(line)] + [(m.start(), False) for m in APT.finditer(line)]
        for _, is_rule in sorted(events):
            if is_rule:
                ready = True
            elif not ready:
                problems.append(f"{path}:{n}: apt-get before scripts/apt-mirror-setup.sh: its apt reads each source from one host")
    return problems


def apt_outside_rule(path, text):
    """One line per apt-get update or install that reads each source from
    the one host its system names: in a workflow, one in a job that runs in
    a container, before any run of scripts/apt-mirror-setup.sh in that job;
    in a composite action or a shell script, whose system this check cannot
    see, one before any run of it in that file. A job on a runner's own
    system keeps the sources its runner image has."""
    if "/workflows/" not in f"/{path}":
        return _apt_before_rule(path, logical_lines(text))
    problems = []
    lines = text.splitlines()
    for found in step_lists(text):
        if not found or not _in_container(lines, found[0][0]):
            continue
        numbered = [(first + n - 1, line) for first, body in found for n, line in logical_lines("\n".join(body))]
        problems += _apt_before_rule(path, numbered)
    return problems


RELEASE_ASSET = re.compile(r"/releases/download(?:/|\b)")
FETCHER = re.compile(r"\b(?:curl|wget)\b")
# In a shell script, curl or wget as a command: at the start, after a
# separator or "$(", or after a shell keyword. Not a word in a pattern.
FETCH_COMMAND = re.compile(r"(?:^|[;&|(]\s*|\$\(\s*|\b(?:then|do|else|exec|time)\s+)(?:curl|wget)\s")
PINNED_ACTION = re.compile(r"^\s*(?:-\s+)?uses:\s*\./\.github/actions/pinned-download\s*(?:#.*)?$")
STEP_KEY = re.compile(r"^(?P<indent>\s*)-\s+\S")
STEPS_KEY = re.compile(r"^(?P<indent>\s*)steps\s*:\s*$")
# The workflows' release downloads when this rule was written: DuckDB twice,
# MinIO, cargo-llvm-cov, cargo-deny, Zola twice, resvg, cargo-fuzz and
# cargo-cyclonedx. cargo-nextest goes through .github/actions/nextest,
# whose URL scripts/nextest.sh builds, so no step names it.
MIN_RELEASE_DOWNLOADS = 10


def _code(line):
    """A line that is a command or a key, not a comment or a step's name."""
    s = line.strip()
    return bool(s) and not s.startswith("#") and not re.match(r"-?\s*name\s*:", s)


def steps(text):
    """(first line number, lines) for each item of every `steps:` list."""
    return [step for found in step_lists(text) for step in found]


def step_lists(text):
    """Each `steps:` list (a job's, or a composite action's) as its own list
    of (first line number, lines)."""
    lists = []
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        m = STEPS_KEY.match(lines[i])
        i += 1
        if not m:
            continue
        item = None  # the column of this list's "-"
        current = None
        out = []
        lists.append(out)
        while i < len(lines):
            line = lines[i]
            if not line.strip() or line.strip().startswith("#"):
                if current is not None:
                    current[1].append(line)
                i += 1
                continue
            indent = len(line) - len(line.lstrip(" "))
            d = STEP_KEY.match(line)
            if item is None and d:
                item = len(d.group("indent"))
            if item is None or indent < item:
                break
            if indent == item and d:
                current = (i + 1, [line])
                out.append(current)
            elif current is not None:
                current[1].append(line)
            i += 1
    return lists


CHECKOUT = re.compile(r"^\s*(?:-\s+)?uses:\s*actions/checkout@")
RUN_KEY = re.compile(r"^\s*(?:-\s+)?run\s*:")


def _in_container(lines, first_step):
    """Whether the job whose first step starts at line `first_step` (from
    1) has a `container:` key beside its `steps:` key."""
    i = first_step - 2
    while i >= 0 and not STEPS_KEY.match(lines[i]):
        i -= 1
    if i < 0:
        return False
    column = len(STEPS_KEY.match(lines[i]).group("indent"))
    key = re.compile(rf"^ {{{column}}}container\s*:")
    # The job's other keys are above its steps here and could be below.
    for step in (-1, 1):
        j = i + step
        while 0 <= j < len(lines):
            line = lines[j]
            if line.strip() and not line.strip().startswith("#"):
                if len(line) - len(line.lstrip(" ")) < column:
                    break
                if key.match(line):
                    return True
            j += step
    return False


def runs_before_checkout(path, text):
    """One line per `run:` step that comes before the checkout of its own
    job, in a job that runs in a container. No repository script exists there, so such a step can only install
    a package with the package manager's own command, from one source: the
    "Install git" step that failed on 2026-11-11 was one."""
    problems = []
    lines = text.splitlines()
    for found in step_lists(text):
        if not found or not _in_container(lines, found[0][0]):
            # A runner's own system has git and the other tools.
            continue
        early = []
        for n, body in found:
            code = [l for l in body if _code(l)]
            if any(CHECKOUT.match(l) for l in code):
                # Only a job that checks out has scripts to wait for.
                problems += early
                break
            if any(RUN_KEY.match(l) for l in code):
                early.append(f"{path}:{n}: a run step before the job's checkout: no repository script exists there")
    return problems


def release_problems(path, text):
    """(problems, how many steps fetch a release asset through the rule)."""
    problems, ok = [], 0
    for n, body in steps(text):
        code = [l for l in body if _code(l)]
        if not any(RELEASE_ASSET.search(l) for l in code):
            continue
        if any(FETCHER.search(l) for l in code):
            problems.append(f"{path}:{n}: a release asset fetched with curl or wget, not through .github/actions/pinned-download or scripts/fetch-pinned.sh")
        elif any(PINNED_ACTION.match(l) for l in code) or any("fetch-pinned.sh" in l for l in code):
            ok += 1
        else:
            problems.append(f"{path}:{n}: a release asset named outside .github/actions/pinned-download and scripts/fetch-pinned.sh")
    return problems, ok


def script_release_problems(path, text):
    """A shell script that names a release asset fetches it only through
    scripts/fetch-pinned.sh."""
    code = [(n, l) for n, l in logical_lines(text) if _code(l)]
    if not any(RELEASE_ASSET.search(l) for _, l in code):
        return []
    problems = [
        f"{path}:{n}: curl or wget in a script that names a release asset; fetch it with scripts/fetch-pinned.sh"
        for n, l in code
        if FETCH_COMMAND.search(l.strip()) and "fetch-pinned.sh" not in l
    ]
    if not any("fetch-pinned.sh" in l for _, l in code):
        problems.append(f"{path}: names a release asset and never calls scripts/fetch-pinned.sh")
    return problems


MAIN_ONLY = "github.ref == 'refs/heads/main'"


WORKFLOW_RUN = re.compile(r"^\s*workflow_run\s*:")


def cache_problems(path, text):
    """actions/cache entries are saved only on main, and a workflow run by
    workflow_run, which runs in main's context with main's cache scope, uses
    no cache at all: code it checks out could write an entry that main's
    jobs then restore (CodeQL actions/cache-poisoning)."""
    problems = []
    privileged = any(WORKFLOW_RUN.match(l) for l in text.splitlines() if _code(l))
    for n, body in steps(text):
        code = [l for l in body if _code(l)]
        if privileged and any(re.search(r"\buses:\s*(?:actions/cache(?:/restore|/save)?@|\./\.github/actions/(?:pinned-download|nextest)\b)", l) for l in code):
            problems.append(f"{path}:{n}: a workflow run by workflow_run shares main's cache; fetch with scripts/fetch-pinned.sh, without the cache")
        if any(re.search(r"\buses:\s*actions/cache@", l) for l in code):
            problems.append(f"{path}:{n}: actions/cache saves on every branch; restore with actions/cache/restore, and save with actions/cache/save only on main")
        if any(re.search(r"\buses:\s*actions/cache/save@", l) for l in code):
            cond = [l for l in code if re.match(r"\s*(?:-\s+)?if\s*:", l)]
            if not any(MAIN_ONLY in l for l in cond):
                problems.append(f"{path}:{n}: actions/cache/save without `if: ... {MAIN_ONLY}`")
    return problems


def _value(body, key):
    for l in body:
        m = re.match(rf"\s*(?:-\s+)?{key}\s*:\s*(.*?)\s*$", l)
        if m:
            return m.group(1)
    return None


def pinned_action_problems(text):
    """.github/actions/pinned-download keeps its one-rule shape."""
    problems = []
    found = steps(text)

    def using(name):
        return [b for _, b in found if any(re.search(rf"\buses:\s*actions/cache/{name}@", l) for l in b if _code(l))]

    restore, save = using("restore"), using("save")
    fetch = [b for _, b in found if any("fetch-pinned.sh" in l for l in b if _code(l))]
    if len(restore) != 1 or len(save) != 1:
        problems.append("the action needs exactly one actions/cache/restore step and one actions/cache/save step")
    if not fetch:
        problems.append("no step runs scripts/fetch-pinned.sh")
    if problems:
        return problems
    key = _value(restore[0], "key") or ""
    if not all(f"${{{{ inputs.{k} }}}}" in key for k in ("name", "version", "sha256")):
        problems.append("the cache key does not name inputs.name, inputs.version and inputs.sha256")
    if key != (_value(save[0], "key") or "") or _value(restore[0], "path") != _value(save[0], "path"):
        problems.append("the restore and save steps use different keys")
    if _value(fetch[0], "if") is not None:
        problems.append("the fetch-pinned.sh step has an if:, so a restored file could go unchecked")
    if "cache-hit != 'true'" not in (_value(save[0], "if") or ""):
        problems.append("the save runs after a cache hit too; an entry is never overwritten")
    return problems


def release_downloads(root):
    """How many workflow and action steps fetch a release asset through the rule."""
    root = pathlib.Path(root)
    return sum(release_problems("", p.read_text(encoding="utf-8"))[1] for p in files(root) if p.suffix in (".yml", ".yaml"))


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
        rel, text = path.relative_to(root).as_posix(), path.read_text(encoding="utf-8")
        problems += unretried(rel, text)
        problems += apt_outside_rule(rel, text)
        if path.suffix == ".sh":
            # A test script drives a script against file:// fixtures: it
            # names a release URL to check one, and fetches none.
            if not path.name.startswith("test-"):
                problems += script_release_problems(rel, text)
        else:
            problems += release_problems(rel, text)[0]
            problems += cache_problems(rel, text)
            problems += runs_before_checkout(rel, text)
    action = root / ".github/actions/pinned-download/action.yml"
    if action.is_file():
        problems += [f"{action.relative_to(root).as_posix()}: {p}" for p in pinned_action_problems(action.read_text(encoding="utf-8"))]
    else:
        problems.append(".github/actions/pinned-download/action.yml is missing: every release asset goes through it or scripts/fetch-pinned.sh")
    count = release_downloads(root)
    if count < MIN_RELEASE_DOWNLOADS:
        problems.append(f"found {count} release downloads through the one rule; expected at least {MIN_RELEASE_DOWNLOADS}. A reading that lost the steps cannot pass")
    calls = sum(dnf_rule_calls(p.read_text(encoding="utf-8")) for p in files(root))
    if calls < MIN_DNF_RULE_CALLS:
        problems.append(f"found {calls} installs through scripts/dnf_install.py; expected at least {MIN_DNF_RULE_CALLS}. A reading that lost the steps cannot pass")
    calls = sum(apt_rule_calls(p.read_text(encoding="utf-8")) for p in files(root))
    if calls < MIN_APT_RULE_CALLS:
        problems.append(f"found {calls} runs of scripts/apt-mirror-setup.sh; expected at least {MIN_APT_RULE_CALLS}. A reading that lost the steps cannot pass")
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
