"""Checks that no workflow and no composite action names a floating runner
image: a label that ends in -latest (ubuntu-latest, macos-latest,
windows-latest), which GitHub moves to a new operating system release on a
day of its choosing.

On 2026-11-13 `runs-on: ubuntu-latest` began to give some jobs Ubuntu 26.04
where every job before had run on 24.04. The website job's
`npx playwright install --with-deps` asked apt for packages 26.04 does not
have and failed, on main, on a commit whose pull request had passed
minutes before on 24.04. A fixed label (ubuntu-24.04) changes only by a
commit, which CI tests before it lands.

The rule: no value of a workflow (.github/workflows/*.yml, *.yaml) or of a
composite action (.github/actions/*/action.yml) holds a label that ends in
-latest or has -latest- in it: a job's runs-on, a row of its matrix, an
input given to a called workflow or to an action, a default, an
environment value. A step's script (run), a name and a description are
not read, and neither is a comment. An image tag (postgres:latest) is no
runner label; container images are pinned by digest.

Run with: python3 -I scripts/check_runner_images.py
(from anywhere in the repository; needs PyYAML). Exits 1 naming each
place that breaks the rule.
"""

import importlib.util
import pathlib
import re
import sys

# Loading a script by path would otherwise leave a __pycache__ beside it.
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("check_hook_steps", pathlib.Path(__file__).resolve().parent / "check_hook_steps.py")
check_hook_steps = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_hook_steps)

# A runner label that floats: words joined by hyphens, one of them
# `latest` and not the first. GitHub compares labels without regard to
# case.
FLOATING = re.compile(r"(?<![A-Za-z0-9_./:-])[a-z][a-z0-9.]*(?:-[a-z0-9.]+)*-latest(?:-[a-z0-9.]+)*(?![A-Za-z0-9_./:-])", re.IGNORECASE)
# Keys whose text is for a reader or a shell, not a label.
NOT_READ = ("run", "name", "description")
PLAIN = re.compile(r"[A-Za-z0-9_-]+")
EXAMPLE = "ubuntu-24.04"


def floating(text):
    """Each floating runner label in text, once, in order."""
    found = []
    for match in FLOATING.finditer(text):
        if match.group(0) not in found:
            found.append(match.group(0))
    return found


def segment(key):
    """A mapping's key as a part of a place. PyYAML reads `on` as true."""
    if key is True:
        return "on"
    key = str(key)
    return key if PLAIN.fullmatch(key) else f"'{key}'"


def places(node, where=""):
    """(place, label) for each floating label under node."""
    if isinstance(node, str):
        return [(where, label) for label in floating(node)]
    found = []
    if isinstance(node, dict):
        for key, value in node.items():
            if isinstance(key, str):
                found += [(where, label) for label in floating(key)]
            if key in NOT_READ:
                continue
            inner = segment(key)
            found += places(value, f"{where}.{inner}" if where else inner)
    elif isinstance(node, list):
        for index, value in enumerate(node):
            found += places(value, f"{where}[{index}]")
    return found


def problems(documents):
    """One line per floating label. documents: file name to parsed YAML."""
    return [
        f"{name}: {where}: `{label}` is a floating runner image: name a fixed one (for example {EXAMPLE})"
        for name in sorted(documents)
        for where, label in places(documents[name])
    ]


def main(root):
    try:
        documents = check_hook_steps.read_workflows(root)
    except ValueError as error:
        print(f"check_runner_images: {error}", file=sys.stderr)
        return 1
    found = problems(documents)
    for problem in found:
        print(f"check_runner_images: {problem}", file=sys.stderr)
    if found:
        return 1
    print(f"check_runner_images: ok ({len(documents)} files name no floating runner image)")
    return 0


if __name__ == "__main__":
    sys.exit(main(pathlib.Path(__file__).resolve().parent.parent))
