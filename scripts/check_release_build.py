"""Builds the libraries and binaries of the workspace with no test target,
so a cargo feature that only a dev-dependency turns on is not in the build.

cargo unifies the features of [dev-dependencies] into a build that has a
test target in it (a test, a bench, an example, `--all-targets`), and every
other cargo step of the commit hook builds one. On 2026-11-25
crates/vcon-server/src/store/search.rs called `time::macros::datetime!` in
non-test code; `time`'s `macros` feature was on only under
[dev-dependencies], so the lib tests, the workspace nextest run and clippy
`--all-targets` passed, and `cargo build --release -p vcon-server` failed
(E0433). The release build of scripts/reproducible-build.sh and CI's builds
of the server have no test target.

The commands, each `cargo check --locked --lib --bins`:

  --workspace                    every crate, default features
  -p <server> --no-default-features
  -p <server> --features <f>     for each feature of the server that is off
                                 by default and needs no live service (the
                                 rule of scripts/check_test_runner.py, read
                                 from there): postgres, which the release
                                 binaries carry

<server> is the [package] name of crates/vcon-server/Cargo.toml (the hook
tests keep a stand-in crate there). `cargo check` refuses everything this
case is, a name that does not resolve, and does no code generation; the
release profile changes no cfg the source reads (no `debug_assertions`, no
[profile.release]), so the commands use the dev profile: on thor-02 on
2026-11-26, `-p vcon-server --features postgres --lib --bins` took 62 s
in a new target directory and 1 s again; with --release 91 s and 1 s.
Each command runs in a process
group of its own, ended whole at the deadline.

`hook_problems` holds .githooks/pre-commit to running this script, unindented
and unconditional, after it sets CARGO_TARGET_DIR (scripts/
test_check_release_build.py runs it on the real hook).

Run with: python3 -I scripts/check_release_build.py [--deadline SECONDS]
(from anywhere in the repository). Exits 1 naming each command that failed.
"""

import importlib.util
import os
import pathlib
import re
import shlex
import signal
import subprocess
import sys
import threading

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
MANIFEST = "crates/vcon-server/Cargo.toml"
HOOK = ".githooks/pre-commit"
STEP = "python3 -I scripts/check_release_build.py"
BASE = ["cargo", "check", "--locked", "--lib", "--bins"]
# Seconds for one command; the first run in a new target directory checks
# every dependency (62 s on thor-02 on 2026-11-26, see above).
DEADLINE = 1200
TARGET_EXPORT = re.compile(r"^export CARGO_TARGET_DIR=")


def _runner_rules():
    spec = importlib.util.spec_from_file_location("check_test_runner", HERE / "check_test_runner.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


RULES = _runner_rules()


def commands(manifest_text):
    """The cargo commands, from the server's manifest text. Raises
    ValueError for a manifest that is not TOML or names no package."""
    try:
        name = RULES.package_name(manifest_text)
        extra = RULES.extra_features(manifest_text)
    except RULES.tomllib.TOMLDecodeError as error:
        raise ValueError(f"{MANIFEST} is not TOML: {error}") from error
    if name is None:
        raise ValueError(f"{MANIFEST} names no [package]")
    found = [BASE + ["--workspace"], BASE + ["-p", name, "--no-default-features"]]
    found += [BASE + ["-p", name, "--features", feature] for feature in extra]
    return found


def shown(command):
    return shlex.join(command)


def hook_problems(text):
    """What keeps the hook from running this script on every commit."""
    lines = text.splitlines()
    export = next((i for i, line in enumerate(lines) if TARGET_EXPORT.match(line)), None)
    step = next((i for i, line in enumerate(lines) if line.rstrip() == STEP), None)
    if step is None:
        return [f"{HOOK} does not run `{STEP}` on a line of its own, unindented: a feature only a dev-dependency turns on would pass every hook step"]
    if export is None:
        return [f"{HOOK} sets no CARGO_TARGET_DIR before `{STEP}`"]
    if step < export:
        return [f"{HOOK} runs `{STEP}` before it sets CARGO_TARGET_DIR, so it would build in the deleted copy of the index on every run"]
    return []


ENDING_SIGNALS = (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)


def execute(command, cwd, deadline):
    """Runs a command in a session of its own. Returns its status, or None
    when the deadline passed, after ending its whole process group. A
    SIGTERM, SIGHUP or SIGINT to this process while the command runs ends
    the command's group too, then this process: a session of its own is
    out of reach of a signal sent to the caller's group (heavy-run's or
    timeout's)."""
    try:
        child = subprocess.Popen(command, cwd=cwd, start_new_session=True, stdin=subprocess.DEVNULL)
    except OSError as error:
        print(f"check_release_build: cannot start `{shown(command)}`: {error}", file=sys.stderr)
        return 127

    def end_group():
        # The child is not reaped yet, so its group id names only the
        # processes this command started.
        os.killpg(child.pid, signal.SIGKILL)
        child.wait()

    def on_signal(number, _frame):
        end_group()
        raise SystemExit(128 + number)

    previous = {}
    if threading.current_thread() is threading.main_thread():
        previous = {number: signal.signal(number, on_signal) for number in ENDING_SIGNALS}
    try:
        return child.wait(timeout=deadline)
    except subprocess.TimeoutExpired:
        end_group()
        return None
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)


def run(command_list, root, runner=execute, deadline=DEADLINE):
    """Runs every command; returns (exit status, lines to print)."""
    if not command_list:
        return 1, ["check_release_build: no command to run"]
    lines, failed = [], 0
    for command in command_list:
        status = runner(command, root, deadline)
        if status is None:
            failed += 1
            lines.append(f"check_release_build: `{shown(command)}` did not end within the deadline of {deadline} s")
        elif status != 0:
            failed += 1
            lines.append(
                f"check_release_build: `{shown(command)}` failed (exit {status}): code outside the tests may use a "
                "feature that only [dev-dependencies] turns on, which builds with test targets unify"
            )
    if failed:
        return 1, lines
    return 0, [f"check_release_build: ok ({len(command_list)} builds of the libraries and binaries without test targets)"]


def main(argv):
    deadline = DEADLINE
    if argv[:1] == ["--deadline"] and len(argv) == 2 and argv[1].isdigit() and int(argv[1]) > 0:
        deadline = int(argv[1])
    elif argv:
        print("usage: check_release_build.py [--deadline SECONDS]", file=sys.stderr)
        return 2
    root = HERE.parent
    try:
        found = commands((root / MANIFEST).read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        print(f"check_release_build: {error}", file=sys.stderr)
        return 1
    code, lines = run([c + ["--quiet"] for c in found], root, deadline=deadline)
    for line in lines:
        print(line, file=sys.stderr if code else sys.stdout)
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
