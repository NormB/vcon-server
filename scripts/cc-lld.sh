#!/usr/bin/env bash
# The linker cargo uses for aarch64 Linux builds of this repository
# (.cargo/config.toml): cc, told to link with the Rust toolchain's own lld
# when the toolchain has one, and plain cc (GNU ld) when it does not.
#
# Why: the tests are some 30 binaries of 100 MB and more, each linked again
# on every change to the library. Measured on 2026-11-02 on an aarch64
# host, one test binary linked in 3.6-4.4 s with GNU ld and in 2.1-2.6 s with
# lld. On x86_64 Linux rustc already links with its own lld, so this is not
# needed there. Release builds do not use it: scripts/reproducible-build.sh
# sets the plain cc for them, so the binaries shipped are linked as before.
#
# Stable rustc on aarch64 has no option that selects its own lld, so the
# toolchain is found from rustc's arguments: rustc passes
# `-L <sysroot>/lib/rustlib/<target>/lib`, and rustup's toolchains ship
# `<sysroot>/lib/rustlib/<target>/bin/gcc-ld/ld.lld`, which cc runs for
# -fuse-ld=lld when -B names its directory.
set -euo pipefail
gcc_ld=""
prev=""
for arg in "$@"; do
  if [ "$prev" = -L ]; then
    case "$arg" in
      */lib/rustlib/*/lib)
        if [ -x "${arg%/lib}/bin/gcc-ld/ld.lld" ]; then gcc_ld="${arg%/lib}/bin/gcc-ld"; fi ;;
    esac
  fi
  prev="$arg"
done
if [ -n "$gcc_ld" ]; then
  exec cc -fuse-ld=lld -B "$gcc_ld" "$@"
fi
exec cc "$@"
