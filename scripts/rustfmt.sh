#!/usr/bin/env bash
# Runs rustfmt over the workspace and the fuzz package with rustfmt.toml.
# One script, so the commit hook, CI and a developer format the same way.
#
#   scripts/rustfmt.sh            format every file
#   scripts/rustfmt.sh --check    change nothing; fail if a file would change
#   scripts/rustfmt.sh toolchain  print the toolchain it runs
#   scripts/rustfmt.sh install    install that toolchain's rustfmt
#
# rustfmt.toml sets options that are unstable (imports_granularity,
# hex_literal_case, format_macro_matchers), which only a nightly rustfmt
# reads. The nightly is pinned by date, here and nowhere else, and is used
# for formatting only: everything is built and linted with the stable
# toolchain rust-toolchain.toml names.
set -euo pipefail
toolchain="nightly-2026-10-06"
root="$(cd "$(dirname "$0")/.." && pwd)"

case "${1:-}" in
  toolchain)
    echo "$toolchain"
    exit 0 ;;
  install)
    exec rustup toolchain install "$toolchain" --profile minimal --component rustfmt ;;
  "" | --check) ;;
  *)
    echo "usage: scripts/rustfmt.sh [--check | toolchain | install]" >&2
    exit 2 ;;
esac

if ! rustup run "$toolchain" rustfmt --version >/dev/null 2>&1; then
  echo "rustfmt.sh: $toolchain's rustfmt is not installed; run scripts/rustfmt.sh install" >&2
  exit 1
fi

cd "$root"
# The fuzz package is a workspace of its own, which --all does not reach.
# rustfmt finds rustfmt.toml at the root from there too.
cargo "+$toolchain" fmt --all ${1:+"$1"}
if [ -f fuzz/Cargo.toml ]; then
  cargo "+$toolchain" fmt --manifest-path fuzz/Cargo.toml ${1:+"$1"}
fi
