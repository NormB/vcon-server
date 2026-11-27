"""Proves that cargo-deny, run with this repository's deny.toml, refuses a
yanked crate version.

deny.toml's `[advisories] yanked = "deny"` is the repository's only check
for yanked versions (OSV-Scanner does not report a yank). A unit test can
read that setting, but not whether the pinned cargo-deny acts on it: that
needs cargo-deny, crates.io's index and the crate's download. So CI's
`deny` job runs this probe after its own check, and the commit hook does
not (it has no network). scripts/test_deny_config.py tests the parts that
need none.

The probe writes, in a temporary directory, a crate that depends on
autocfg 0.1.3, a version crates.io lists as yanked (checked 2026-11-26),
with a Cargo.lock that holds it, since cargo resolves a yanked version only
from a lockfile. It then runs `cargo-deny check advisories` with
deny.toml and passes only when cargo-deny fails naming the yanked crate.

Run with: python3 -I scripts/deny_yanked_probe.py [cargo-deny binary]
(from the repository root). Exits 0 when cargo-deny refused the version.
"""

import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
DENY_TOML = HERE.parent / "deny.toml"

# A version crates.io lists as yanked, with no dependencies of its own, so
# the probe downloads one small crate.
YANKED = ("autocfg", "0.1.3", "dfb37ca32a3d9d88f18d08bac8d28368b8ee1f14f8b08eb62999c51720035b55")

MANIFEST = """[package]
name = "yanked-probe"
version = "0.0.0"
edition = "2021"
license = "MIT OR Apache-2.0"
publish = false

[dependencies]
{name} = "={version}"

[workspace]
"""

LOCK = """version = 4

[[package]]
name = "{name}"
version = "{version}"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "{checksum}"

[[package]]
name = "yanked-probe"
version = "0.0.0"
dependencies = [
 "{name}",
]
"""


def write_crate(directory, yanked=YANKED):
    """The probe crate, in `directory`."""
    name, version, checksum = yanked
    root = pathlib.Path(directory)
    (root / "src").mkdir()
    (root / "src" / "lib.rs").write_text("", encoding="utf-8")
    (root / "Cargo.toml").write_text(MANIFEST.format(name=name, version=version), encoding="utf-8")
    (root / "Cargo.lock").write_text(LOCK.format(name=name, version=version, checksum=checksum), encoding="utf-8")
    return root / "Cargo.toml"


def command(binary, manifest):
    """The cargo-deny command the probe runs."""
    return [binary, "--manifest-path", str(manifest), "--config", str(DENY_TOML), "check", "advisories"]


def verdict(returncode, output, yanked=YANKED):
    """None when cargo-deny refused the yanked version, else the problem."""
    name, version, _ = yanked
    if returncode == 0:
        return f"cargo-deny accepted {name} {version}, which crates.io lists as yanked"
    named = f"{name} v{version}" in output or f"{name} {version}" in output
    if "yanked" not in output or not named:
        return f"cargo-deny failed (exit {returncode}) without naming {name} {version} as yanked:\n{output}"
    return None


def main(argv):
    binary = argv[1] if len(argv) > 1 else "cargo-deny"
    with tempfile.TemporaryDirectory(prefix="yanked-probe-") as directory:
        manifest = write_crate(directory)
        run = subprocess.run(command(binary, manifest), capture_output=True, text=True, timeout=600)
    problem = verdict(run.returncode, run.stdout + run.stderr)
    if problem:
        print(f"deny_yanked_probe: {problem}", file=sys.stderr)
        return 1
    name, version, _ = YANKED
    print(f"deny_yanked_probe: ok (cargo-deny refused yanked {name} {version})")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
