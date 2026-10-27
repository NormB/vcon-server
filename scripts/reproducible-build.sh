#!/usr/bin/env bash
# Builds the vcon-server release binary so that one commit gives the same
# bytes wherever it is built, and checks that it does.
#
# Usage:
#   reproducible-build.sh build <target-triple>
#       the release build: `cargo build --release --locked -p vcon-server`
#       for <target-triple>, with the path remaps appended to RUSTFLAGS,
#       SOURCE_DATE_EPOCH set to the commit's time, and the target directory
#       at $TARGET_DIR below. Prints the binary's path last. release.yml
#       builds through this.
#   reproducible-build.sh check <target-triple> <workdir> [<rev>]
#       clones <rev> (default HEAD) into <workdir>/a and <workdir>/b, builds
#       each with `build`, the second with a CARGO_HOME of its own, and
#       compares the two binaries byte for byte. Exits 1 with the evidence
#       when they differ. <workdir> is deleted on success, kept on failure.
#   reproducible-build.sh compare <file-a> <file-b>
#       the verdict `check` uses: 0 when identical, else 1 and the evidence.
#
# Modeled on sipnab's scripts/reproducible-build.sh. Measured on the
# runners on 2026-10-27: two plain release builds of one commit, in two
# checkouts with two CARGO_HOMEs, differed in 15199184 bytes. Three causes,
# each removed here:
#
#   1. Panic locations in registry crates name $CARGO_HOME/registry/src.
#      Remapped for rustc with --remap-path-prefix.
#   2. The vendored OpenSSL compiles its install directory into libcrypto
#      as ENGINESDIR and MODULESDIR, and openssl-src puts that directory in
#      cargo's target directory. They are string constants, which no prefix
#      map reaches, so the target directory is one fixed path, $TARGET_DIR,
#      for every build. vcon-server loads no engine or module (openssl-src
#      builds with no-module), so the path is never used.
#   3. OpenSSL writes its build date into the library, from
#      SOURCE_DATE_EPOCH when it is set (util/mkbuildinf.pl), else from the
#      clock. Set here to the commit's time.
#
# Not done, and why: sipnab also passes the remaps to the C compiler as
# -ffile-prefix-map in CFLAGS. Here that makes the builds differ, because
# OpenSSL records its compiler flags in the library ("compiler: cc ..."),
# so the checkout's path entered the binary through them (seen in the run
# that tried it). Without them the two builds are identical: cargo's
# release profile strips the debug information that would name the C
# sources' paths.
#
# A rebuild must also hold fixed the Rust toolchain (rust-toolchain.toml)
# and the runner image's C compiler and linker.

set -euo pipefail

die() { printf 'reproducible-build: %s\n' "$*" >&2; exit 1; }

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
# Fixed for the reason in 2 above. Not under the checkout, so two checkouts
# share it; `check` builds one at a time and empties it in between.
TARGET_DIR="$ROOT/target"

# THE rule: each prefix that varies between builders and the fixed name it
# becomes. The checkout is last: rustc applies the last remap that matches.
remap_pairs() {
  local cargo_home="${CARGO_HOME:-$HOME/.cargo}" p
  for p in "$ROOT" "$cargo_home"; do
    case "$p" in
      *[[:space:]]*) die "cannot remap '$p': RUSTFLAGS is split on whitespace. Build from a path without it." ;;
    esac
  done
  printf '%s=/cargo\n' "$cargo_home"
  printf '%s=/vcon-server\n' "$ROOT"
}

rustflags() {
  local out="" p
  for p in $(remap_pairs); do out="${out:+$out }--remap-path-prefix=$p"; done
  printf '%s\n' "$out"
}

cmd_build() {
  [ $# -eq 1 ] || die "usage: $0 build <target-triple>"
  local target="$1"
  cd "$ROOT"
  if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
    SOURCE_DATE_EPOCH=$(git -C "$ROOT" log -1 --format=%ct HEAD 2>/dev/null) \
      || die "$ROOT is not a git checkout, so there is no commit time for SOURCE_DATE_EPOCH. Build from a checkout, or set it."
  fi
  export SOURCE_DATE_EPOCH
  # Appended, never replacing what the caller set.
  RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$(rustflags)"
  export RUSTFLAGS
  export CARGO_TARGET_DIR="$TARGET_DIR"
  cargo build --release --locked -p vcon-server --target "$target" >&2
  printf '%s\n' "$TARGET_DIR/$target/release/vcon-server"
}

cmd_compare() {
  [ $# -eq 2 ] || die "usage: $0 compare <file-a> <file-b>"
  local a="$1" b="$2"
  [ -f "$a" ] || die "no such file: $a"
  [ -f "$b" ] || die "no such file: $b"
  if cmp -s "$a" "$b"; then
    printf 'identical: %s  %s\n' "$(sha256sum "$a" | cut -d' ' -f1)" "$a"
    return 0
  fi
  printf 'NOT reproducible: %s and %s differ\n' "$a" "$b"
  sha256sum "$a" "$b"
  printf 'differing bytes: %s (the first ones: offset, octal a, octal b)\n' \
    "$(cmp -l "$a" "$b" | wc -l | tr -d ' ')"
  cmp -l "$a" "$b" | head -10 || true
  printf 'strings that differ (< a, > b):\n'
  diff <(strings -n 6 "$a") <(strings -n 6 "$b") | head -40 || true
  return 1
}

cmd_check() {
  [ $# -eq 2 ] || [ $# -eq 3 ] || die "usage: $0 check <target-triple> <workdir> [<rev>]"
  local target="$1" work="$2" head d bin cargo_home ok=0
  [ ! -e "$work" ] || die "$work exists; give check a new directory"
  # Never removes a directory it did not make.
  [ ! -e "$TARGET_DIR" ] || die "$TARGET_DIR exists; check needs it empty for each build. Remove it first."
  head=$(git -C "$ROOT" rev-parse --verify "${3:-HEAD}^{commit}")
  mkdir -p "$work"
  work=$(cd "$work" && pwd -P)
  for d in a b; do
    git clone --quiet --no-checkout "$ROOT" "$work/$d"
    git -C "$work/$d" checkout --quiet --detach "$head"
    # A CARGO_HOME of its own for b, so the registry's path varies as well
    # as the checkout's. rustup's toolchains are found through RUSTUP_HOME.
    if [ "$d" = a ]; then cargo_home="${CARGO_HOME:-$HOME/.cargo}"; else cargo_home="$work/cargo-home-b"; fi
    printf '== build %s in %s (CARGO_HOME=%s)\n' "$d" "$work/$d" "$cargo_home"
    bin=$(CARGO_HOME="$cargo_home" bash "$work/$d/scripts/reproducible-build.sh" build "$target" | tail -1)
    mkdir -p "$work/$d-dist"
    cp "$bin" "$work/$d-dist/vcon-server"
    rm -rf "$TARGET_DIR"
  done
  cmd_compare "$work/a-dist/vcon-server" "$work/b-dist/vcon-server" || ok=1
  if [ "$ok" -ne 0 ]; then
    printf 'kept %s for inspection\n' "$work" >&2
    return 1
  fi
  rm -rf "$work"
}

main() {
  case "${1:-}" in
    build) shift; cmd_build "$@" ;;
    check) shift; cmd_check "$@" ;;
    compare) shift; cmd_compare "$@" ;;
    *) die "usage: $0 build <target> | check <target> <workdir> [<rev>] | compare <a> <b>" ;;
  esac
}

main "$@"
