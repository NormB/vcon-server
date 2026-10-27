#!/usr/bin/env bash
# Runs scripts/check_package.py on copies of this tree, as builds really
# reuse target directories:
#   1. after a change to vcon's public API with no version bump, a second
#      run in the same target directory must see the new API (cargo package's
#      own verify reused the vcon 0.1.0 built by the first run);
#   2. two trees checked at the same moment, sharing one target directory,
#      each using a symbol only its own vcon has, must both pass.
set -euo pipefail
src="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export CARGO_TARGET_DIR="$work/target"
copy() {
  mkdir -p "$1"
  (cd "$src" && git ls-files -co --exclude-standard -z | tar --null -T - -cf -) | tar -xf - -C "$1"
}
# Adds `pub const <name>: u8 = 1;` to vcon and makes vcon-server use it.
add_symbol() {
  printf '\n/// Only in this tree.\npub const %s: u8 = 1;\n' "$2" >> "$1/crates/vcon/src/lib.rs"
  printf '\n/// Only in this tree.\npub const USES: u8 = vcon::%s;\n' "$2" >> "$1/crates/vcon-server/src/lib.rs"
}
check() { (cd "$1" && python3 -I scripts/check_package.py) >"$1.log" 2>&1; }
fail=0

copy "$work/a"
check "$work/a" || { echo "FAIL: the unchanged tree"; tail -n 5 "$work/a.log"; exit 1; }
add_symbol "$work/a" ADDED_LATER
if check "$work/a"; then echo "ok: a changed vcon 0.1.0 is checked as it is now"
else echo "FAIL: the second run did not see vcon's new API"; tail -n 5 "$work/a.log"; fail=1; fi

copy "$work/b"; add_symbol "$work/b" ONLY_B
copy "$work/c"; add_symbol "$work/c" ONLY_C
check "$work/b" & pb=$!
check "$work/c" & pc=$!
wait "$pb" && rb=0 || rb=$?
wait "$pc" && rc=0 || rc=$?
if [ "$rb" -eq 0 ] && [ "$rc" -eq 0 ]; then echo "ok: two trees checked at once each saw only its own sources"
else echo "FAIL: concurrent runs (b=$rb c=$rc)"; tail -n 5 "$work/b.log" "$work/c.log"; fail=1; fi

[ "$fail" -eq 0 ] && echo "test-check-package: passed" || { echo "test-check-package: FAILED"; exit 1; }
