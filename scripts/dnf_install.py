"""Installs packages with dnf, trying again from another source.

Usage, as root on a system with dnf:
  python3 -I scripts/dnf_install.py <package>...

On 2026-11-11 the Release workflow's build job failed at its first step:
five tries of `dnf install`, with `dnf clean metadata` between them, each
ended in "Failed to download metadata for repo 'appstream': repomd.xml
parser error". AlmaLinux's repository files name a mirror list. dnf asks
the next mirror of a list when a download fails, but a mirror that answers
200 with a body that is not a repomd.xml is a download that worked and a
parse that failed, and dnf stops there. Cleaning the metadata changes
nothing: the next try gets the same list and reads the same mirror first.
Tried in the almalinux:9 image that day, with a list whose first mirror
answered 1200 spaces and whose second was repo.almalinux.org: dnf failed
the same way on every run and never read the second.

So a try that fails is followed by one from another source. Each
repository file names it: beside `mirrorlist=` stands a commented
`# baseurl=`, the distribution's own server. The tries take turns:

  try 1, 3, 5   as the system is configured (the mirror list)
  try 2, 4      each enabled repository read from its commented baseurl

with the metadata cleaned and a wait of 5, 10, 20 and 40 seconds between
them, and one try ended after 600 seconds. After five tries the last exit
code is this script's. Packages are checked against the distribution's key
as before (gpgcheck is never touched): only the host a file comes from
changes. A repository with no commented baseurl is read as configured on
every try.

scripts/check_workflow_downloads.py refuses a `dnf install` anywhere else
in a workflow, an action or a shell script, so every one goes through this
rule. Python, because every system with dnf 4 has python3, and because the
rule is tested without a mirror (scripts/test_dnf_install.py).
"""

import pathlib
import re
import subprocess
import sys
import time

ATTEMPTS = 5
FIRST_WAIT = 5
# One try: a full install of the build tools took under a minute.
LIMIT = 600
# What `timeout` answers for a command it stopped.
TIMED_OUT = 124
REPOS_DIR = "/etc/yum.repos.d"

SECTION = re.compile(r"^\[\s*([^\]]*?)\s*\]\s*$")
REPO_ID = re.compile(r"^[A-Za-z0-9_.:-]+$")
KEY = re.compile(r"^(mirrorlist|metalink|baseurl|enabled)\s*=\s*(.*?)\s*$")
COMMENTED_BASEURL = re.compile(r"^#\s*baseurl\s*=\s*(.*?)\s*$")
# An address that is one word of a command line and names a web server.
ADDRESS = re.compile(r"^https?://[A-Za-z0-9.-]+(?::[0-9]+)?/[A-Za-z0-9$._~/%+-]*$")
PACKAGE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._+-]*$")
ON = ("1", "yes", "true", "on")


def origins(texts):
    """{repository id: address} for each enabled repository, in the texts of
    repository files, that reads a mirror list or a metalink and names
    another address in a commented baseurl line (the first one)."""
    found = {}
    for text in texts:
        sections, current = [], None
        for line in text.splitlines():
            line = line.strip()
            m = SECTION.match(line)
            if m:
                current = {"id": m.group(1), "enabled": True, "list": False, "other": None}
                sections.append(current)
                continue
            if current is None:
                continue
            m = KEY.match(line)
            if m and m.group(1) == "enabled":
                current["enabled"] = m.group(2).lower() in ON
            elif m and m.group(1) in ("mirrorlist", "metalink"):
                current["list"] = True
            m = COMMENTED_BASEURL.match(line)
            if m and current["other"] is None:
                current["other"] = m.group(1)
        for s in sections:
            if s["enabled"] and s["list"] and s["other"] and REPO_ID.match(s["id"]) and ADDRESS.match(s["other"]):
                found[s["id"]] = s["other"]
    return found


def read_origins(repos_dir=REPOS_DIR):
    """origins() of the *.repo files in a directory, read by name."""
    paths = sorted(pathlib.Path(repos_dir).glob("*.repo"))
    return origins([p.read_text(encoding="utf-8", errors="replace") for p in paths])


def options(attempt, other):
    """The dnf options of try number `attempt` (from 1): none on an odd
    try, and on an even one each repository's mirror list and metalink
    cleared and its baseurl set to its other address."""
    if attempt % 2 == 1:
        return []
    out = []
    for repo in sorted(other):
        out += [f"--setopt={repo}.mirrorlist=", f"--setopt={repo}.metalink=", f"--setopt={repo}.baseurl={other[repo]}"]
    return out


def source(attempt, other):
    """The name of the source a try reads, for the log."""
    return "the repository files' own addresses" if options(attempt, other) else "the configured mirrors"


def run_bounded(argv, limit=LIMIT):
    """Runs a command to its end or to `limit` seconds, whichever is
    first. Its exit code; TIMED_OUT if it was stopped; 127 if it could not
    be started."""
    try:
        return subprocess.run(argv, timeout=limit, check=False).returncode
    except subprocess.TimeoutExpired:
        return TIMED_OUT
    except OSError:
        return 127


def install(packages, other, run=run_bounded, sleep=time.sleep, log=None, attempts=ATTEMPTS, delay=FIRST_WAIT):
    """Installs the packages, in at most `attempts` tries. 0, or the exit
    code of the last try."""
    log = log or (lambda line: print(line, file=sys.stderr, flush=True))
    for attempt in range(1, attempts + 1):
        rc = run(["dnf", "install", "-y", "--setopt=install_weak_deps=False", *options(attempt, other), *packages])
        if rc == 0:
            return 0
        names = " ".join(packages)
        if attempt == attempts:
            log(f"dnf_install: giving up on {names} after {attempt} tries (exit {rc}); the configured mirrors and the repository files' own addresses were each tried")
            return rc
        log(
            f"dnf_install: try {attempt} of {attempts} from {source(attempt, other)} failed (exit {rc}); "
            f"in {delay} s, from {source(attempt + 1, other)}"
        )
        run(["dnf", "clean", "metadata"])
        sleep(delay)
        delay *= 2
    return 1


def main(argv, repos_dir=REPOS_DIR, run=run_bounded, sleep=time.sleep, log=None):
    log = log or (lambda line: print(line, file=sys.stderr, flush=True))
    if not argv or not all(PACKAGE.match(a) for a in argv):
        log("usage: python3 -I scripts/dnf_install.py <package>...   (package names only: no option, file or address)")
        return 2
    return install(argv, read_origins(repos_dir), run=run, sleep=sleep, log=log)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
