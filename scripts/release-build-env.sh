#!/usr/bin/env bash
# Prepares the release build container (AlmaLinux 9, glibc 2.34, the floor
# the release binaries are held to) for scripts/reproducible-build.sh: the
# C toolchain and Perl the vendored OpenSSL and SQLCipher build with,
# binutils and Python for scripts/check_glibc.py, diffutils for the
# reproducible check's cmp and diff, and rustup, pinned by
# sha256, with the toolchain rust-toolchain.toml names. release.yml and
# reproducible.yml both run it, so both build in the same environment.
# It installs git too: the image has none, so the workflows fetch the files
# without it, run this, and check out again to have .git.
#
# Usage, as root in the container, from the repository's files:
#   scripts/release-build-env.sh
set -euo pipefail

RUSTUP_VERSION=1.29.2
case "$(uname -m)" in
  x86_64) triple=x86_64-unknown-linux-gnu; sha=5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4 ;;
  aarch64) triple=aarch64-unknown-linux-gnu; sha=a9c168dbea81a84671c6b7a5c1795cb0eb699020110d107ddfca427bd797e697 ;;
  *) echo "release-build-env: no rustup pin for $(uname -m)" >&2; exit 1 ;;
esac

# perl-core: OpenSSL's Configure needs modules the minimal perl leaves out.
# One install, through the rule that tries a second source when a mirror
# serves a repomd.xml dnf cannot parse (scripts/dnf_install.py).
python3 -I scripts/dnf_install.py git-core gcc make perl-core binutils diffutils python3 xz >&2
ldd --version | sed -n 1p >&2

# The checkout belongs to the runner's user, not root, and git refuses
# another user's repository unless told it is safe. The build reads the
# commit's time from it.
git config --global --add safe.directory "${GITHUB_WORKSPACE:-$PWD}"

curl -fsSL --retry 5 --retry-delay 5 --retry-all-errors --max-time 300 -o /tmp/rustup-init \
  "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${triple}/rustup-init"
echo "${sha}  /tmp/rustup-init" | sha256sum -c - >&2
chmod +x /tmp/rustup-init
/tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none >&2
rm -f /tmp/rustup-init
export PATH="$HOME/.cargo/bin:$PATH"
if [ -n "${GITHUB_PATH:-}" ]; then echo "$HOME/.cargo/bin" >> "$GITHUB_PATH"; fi
scripts/retry.sh rustup toolchain install >&2
rustc --version >&2
