#!/usr/bin/env bash
# Tests scripts/cc-lld.sh and where it applies: test and development
# builds for aarch64 Linux link with the Rust toolchain's own lld when the
# toolchain has it, and with the plain cc (GNU ld) when it does not; release
# builds (scripts/reproducible-build.sh) always link with the plain cc, as
# before. A fake cc records what it is given; nothing is linked.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
pass=0 fail=0
ok() { pass=$((pass + 1)); }
bad() { fail=$((fail + 1)); echo "FAIL ($1)"; [ -s "$work/args" ] && sed 's/^/    got: /' "$work/args"; return 0; }

mkdir -p "$work/bin"
printf '#!/usr/bin/env bash\nprintf "%%s\\n" "$@" > "%s/args"\n' "$work" > "$work/bin/cc"
chmod +x "$work/bin/cc"
link() { rm -f "$work/args"; PATH="$work/bin:$PATH" bash "$here/cc-lld.sh" "$@"; }

# A toolchain with lld: <sysroot>/lib/rustlib/<target>/bin/gcc-ld/ld.lld.
with="$work/with/lib/rustlib/aarch64-unknown-linux-gnu"
mkdir -p "$with/lib" "$with/bin/gcc-ld"
printf '#!/bin/sh\n' > "$with/bin/gcc-ld/ld.lld" && chmod +x "$with/bin/gcc-ld/ld.lld"
link a.o -L "$with/lib" -o out
want=$(printf '%s\n' -fuse-ld=lld -B "$with/bin/gcc-ld" a.o -L "$with/lib" -o out)
[ "$(cat "$work/args")" = "$want" ] && ok || bad "with the toolchain's lld, cc is told to use it, and every argument follows unchanged"

# A toolchain without it (a distribution's rustc, say): plain cc.
without="$work/without/lib/rustlib/aarch64-unknown-linux-gnu"
mkdir -p "$without/lib"
link a.o -L "$without/lib" -o out
[ "$(cat "$work/args")" = "$(printf '%s\n' a.o -L "$without/lib" -o out)" ] && ok || bad "without lld, cc gets exactly rustc's arguments"

# An ld.lld that cannot run is not used.
mkdir -p "$work/noexec/lib/rustlib/x/lib" "$work/noexec/lib/rustlib/x/bin/gcc-ld"
: > "$work/noexec/lib/rustlib/x/bin/gcc-ld/ld.lld"
link a.o -L "$work/noexec/lib/rustlib/x/lib"
[ "$(head -1 "$work/args")" = a.o ] && ok || bad "an ld.lld that is not executable is not used"

# Only rustc's own library directory names the toolchain; any other -L does not.
mkdir -p "$work/other/lib" "$work/other/bin/gcc-ld"
printf '#!/bin/sh\n' > "$work/other/bin/gcc-ld/ld.lld" && chmod +x "$work/other/bin/gcc-ld/ld.lld"
link a.o -L "$work/other/lib"
[ "$(head -1 "$work/args")" = a.o ] && ok || bad "a -L outside lib/rustlib/<target>/lib is not taken for the toolchain"

# The directory counts only as the argument of -L.
link a.o "$with/lib" -o out
[ "$(head -1 "$work/args")" = a.o ] && ok || bad "the toolchain's library directory named other than by -L is not taken"

# The exit code of the link is cc's.
printf '#!/bin/sh\nexit 3\n' > "$work/bin/cc"
rc=0; link a.o -L "$with/lib" || rc=$?
[ "$rc" = 3 ] && ok || bad "the link's exit code is cc's (got $rc)"

# Where it applies: .cargo/config.toml names it for aarch64 Linux only (on
# x86_64 Linux, rustc already links with its own lld), and release builds
# override it with the plain cc.
got=$(python3 -I -c 'import sys, tomllib; c = tomllib.load(open(sys.argv[1], "rb")); print(sorted((t, v.get("linker")) for t, v in c.get("target", {}).items()))' "$root/.cargo/config.toml")
[ "$got" = "[('aarch64-unknown-linux-gnu', 'scripts/cc-lld.sh')]" ] && ok || bad ".cargo/config.toml names scripts/cc-lld.sh for aarch64-unknown-linux-gnu only: $got"

# The commit hook builds in a new temporary copy each run, and cargo's
# fingerprint of every crate holds the linker's path, so the path the config
# gives (relative to that copy) would rebuild every dependency on every run.
# The hook gives the staged script a fixed path instead, for each target the
# config names.
for t in $(python3 -I -c 'import sys, tomllib; print(" ".join(tomllib.load(open(sys.argv[1], "rb")).get("target", {})))' "$root/.cargo/config.toml"); do
  u=$(printf '%s' "$t" | tr 'a-z-' 'A-Z_')
  grep -qxF "export CARGO_TARGET_${u}_LINKER=\"\$CARGO_TARGET_DIR/cc-lld.sh\"" "$root/.githooks/pre-commit" \
    && ok || bad "the commit hook links $t builds with the staged scripts/cc-lld.sh at a fixed path"
done
grep -qxF 'install -m 0755 scripts/cc-lld.sh "$CARGO_TARGET_DIR/cc-lld.sh"' "$root/.githooks/pre-commit" \
  && ok || bad "the commit hook puts the staged scripts/cc-lld.sh at that path"

printf '#!/usr/bin/env bash\nenv | grep "^CARGO_TARGET_.*_LINKER=" > "%s/env" || true\n' "$work" > "$work/bin/cargo"
chmod +x "$work/bin/cargo"
# A release build for aarch64 sets the plain cc, rustc's default there; one
# for x86_64 names no linker, so rustc keeps its own choice.
for case in "aarch64-unknown-linux-gnu CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=cc" "x86_64-unknown-linux-gnu "; do
  t=${case%% *} want=${case#* }
  rm -f "$work/env"
  PATH="$work/bin:$PATH" SOURCE_DATE_EPOCH=0 bash "$here/reproducible-build.sh" build "$t" > /dev/null 2>&1 || true
  [ -e "$work/env" ] && [ "$(cat "$work/env")" = "$want" ] && ok || bad "a release build for $t has the linker setting '$want' (env: $(cat "$work/env" 2>/dev/null))"
done

echo "test-cc-lld: $pass of $((pass + fail)) passed"
[ "$fail" -eq 0 ]
