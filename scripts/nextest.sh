#!/usr/bin/env bash
# Runs the tests with cargo-nextest at one pinned version, with the
# repository's .config/nextest.toml (the per-test deadline). One script, so
# the commit hook, CI and a developer run the tests the same way.
#
#   scripts/nextest.sh version            print the pinned version
#   scripts/nextest.sh install [DIR]      install that version's release binary
#                                         into DIR (default: cargo's bin
#                                         directory), checked by its SHA-256
#   scripts/nextest.sh check              fail unless cargo nextest is that version
#   scripts/nextest.sh run [+TOOLCHAIN] [ARGS...]
#   scripts/nextest.sh list [+TOOLCHAIN] [ARGS...]
#                                         check, then cargo nextest run/list
#                                         with the repository's config and no
#                                         user config
#
# Why nextest: cargo test has no per-test deadline, so one hung test hung
# the commit hook with no end and CI until its job's timeout. nextest ends a
# test still running after the deadline in .config/nextest.toml and fails it
# by name. It runs each test in a process of its own. It does not run doc
# tests; the hook and CI run those with `cargo test --doc`.
#
# The version and the two hashes are pinned here and nowhere else. The
# hashes are those GitHub lists for the release's assets.
set -euo pipefail
version="0.9.148"
root="$(cd "$(dirname "$0")/.." && pwd)"
download="${NEXTEST_DOWNLOAD:-https://github.com/nextest-rs/nextest/releases/download}"

check() {
  local have
  have="$(cargo ${toolchain:+"$toolchain"} nextest --version 2>/dev/null | sed -n '1s/^cargo-nextest \([^ ]*\).*/\1/p' || true)"
  if [ "$have" != "$version" ]; then
    echo "nextest.sh: cargo-nextest ${have:-is not installed}${have:+ is installed}; this repository runs $version. Run scripts/nextest.sh install" >&2
    exit 1
  fi
}

toolchain=""
case "${1:-}" in
  version)
    echo "$version"
    exit 0 ;;
  install)
    case "$(uname -m)" in
      x86_64) sha=223ba936714cc861a9cdedad5a8eb664dfdd69d6fbdf6cea6aa8184f523290dc ;;
      aarch64) sha=81469eb963ef883fd590e60189e77d6aba3d91b418591d3391f50fe3e9480d2b ;;
      *) echo "nextest.sh: no pinned cargo-nextest for $(uname -m)" >&2; exit 1 ;;
    esac
    dest="${2:-${CARGO_HOME:-$HOME/.cargo}/bin}"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    name="cargo-nextest-$version-$(uname -m)-unknown-linux-gnu.tar.gz"
    curl -fsSL --retry 5 --retry-delay 5 --retry-all-errors --max-time 300 -o "$tmp/$name" \
      "$download/cargo-nextest-$version/$name"
    echo "$sha  $tmp/$name" | sha256sum -c --quiet - >&2 || { echo "nextest.sh: $name is not the pinned download" >&2; exit 1; }
    tar xzf "$tmp/$name" -C "$tmp" cargo-nextest
    mkdir -p "$dest"
    install -m 0755 "$tmp/cargo-nextest" "$dest/cargo-nextest"
    "$dest/cargo-nextest" nextest --version | sed -n 1p
    exit 0 ;;
  check)
    check
    echo "nextest.sh: cargo-nextest $version"
    exit 0 ;;
  run | list)
    command="$1"
    shift
    if [ "${1:-}" != "${1#+}" ]; then toolchain="$1"; shift; fi
    check
    exec cargo ${toolchain:+"$toolchain"} nextest "$command" \
      --config-file "$root/.config/nextest.toml" --user-config-file none "$@" ;;
  *)
    echo "usage: scripts/nextest.sh version | install [DIR] | check | run|list [+TOOLCHAIN] [ARGS...]" >&2
    exit 2 ;;
esac
