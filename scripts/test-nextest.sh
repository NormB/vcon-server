#!/usr/bin/env bash
# Tests scripts/nextest.sh: it runs nextest only at the pinned version,
# always with the repository's config (the per-test deadline) and never a
# developer's own, and it installs only a download whose SHA-256 is the
# pinned one. A fake cargo stands in for cargo and cargo-nextest; the
# downloads are file:// URLs, so nothing leaves the machine.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
pass=0 fail=0
ok() { pass=$((pass + 1)); }
bad() { fail=$((fail + 1)); echo "FAIL ($1)"; [ -f "$work/err" ] && sed 's/^/    /' "$work/err"; return 0; }

pin="$(bash "$here/nextest.sh" version)"
[[ "$pin" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && ok || bad "version prints one x.y.z version, not '$pin'"

# The fake cargo: `cargo [+tc] nextest --version` prints FAKE_VERSION's
# line; any other call is written, one argument per line, to $work/args.
mkdir -p "$work/bin"
cat > "$work/bin/cargo" <<'SH'
#!/usr/bin/env bash
args=("$@"); [ "${args[0]:-}" != "${args[0]#+}" ] && args=("${args[@]:1}")
if [ "${args[0]:-}" = nextest ] && [ "${args[1]:-}" = --version ]; then
  [ -n "${FAKE_VERSION:-}" ] || { echo "error: no such command: nextest" >&2; exit 101; }
  printf 'cargo-nextest %s (0000000 2026-10-07)\nrelease: %s\n' "$FAKE_VERSION" "$FAKE_VERSION"; exit 0
fi
printf '%s\n' "$@" > "$FAKE_ARGS"
SH
chmod +x "$work/bin/cargo"
export FAKE_ARGS="$work/args"
fake() { PATH="$work/bin:$PATH" "$@"; }

rm -f "$work/args"
if FAKE_VERSION="$pin" fake bash "$here/nextest.sh" run --workspace --lib 2> "$work/err"; then
  want=$(printf '%s\n' nextest run --config-file "$root/.config/nextest.toml" --user-config-file none --workspace --lib)
  [ "$(cat "$work/args")" = "$want" ] && ok || { echo "got: $(tr '\n' ' ' < "$work/args")" > "$work/err"; bad "run passes the repository's config, no user config, then the arguments"; }
else bad "run at the pinned version succeeds"; fi

rm -f "$work/args"
if FAKE_VERSION="$pin" fake bash "$here/nextest.sh" run +nightly-2026-10-24 --lib 2> "$work/err"; then
  [ "$(head -1 "$work/args")" = "+nightly-2026-10-24" ] && [ "$(sed -n 2,3p "$work/args" | tr '\n' ' ')" = "nextest run " ] \
    && ok || bad "run +toolchain passes the toolchain to cargo first"
else bad "run +toolchain at the pinned version succeeds"; fi

rm -f "$work/args"
if FAKE_VERSION="$pin" fake bash "$here/nextest.sh" list --lib 2> "$work/err"; then
  [ "$(sed -n 1,3p "$work/args" | tr '\n' ' ')" = "nextest list --config-file " ] && ok || bad "list passes the repository's config too"
else bad "list at the pinned version succeeds"; fi

for v in 0.9.147 9.9.9; do
  rm -f "$work/args"
  if FAKE_VERSION="$v" fake bash "$here/nextest.sh" run --workspace 2> "$work/err"; then bad "run refuses nextest $v"
  elif [ -e "$work/args" ]; then bad "nothing runs under nextest $v"
  elif grep -q "scripts/nextest.sh install" "$work/err"; then ok
  else bad "the refusal of $v names the install command"; fi
done

rm -f "$work/args"
if FAKE_VERSION="" fake bash "$here/nextest.sh" check 2> "$work/err"; then bad "check refuses when nextest is not installed"
elif grep -q "scripts/nextest.sh install" "$work/err"; then ok; else bad "the refusal names the install command"; fi
if FAKE_VERSION="$pin" fake bash "$here/nextest.sh" check > /dev/null 2> "$work/err"; then ok; else bad "check passes at the pinned version"; fi

# A download whose SHA-256 is not the pinned one is refused, and nothing
# is installed.
mkdir -p "$work/dl/cargo-nextest-$pin" "$work/dest"
printf '#!/bin/sh\necho planted\n' > "$work/cargo-nextest" && chmod +x "$work/cargo-nextest"
for arch in x86_64 aarch64; do
  tar czf "$work/dl/cargo-nextest-$pin/cargo-nextest-$pin-$arch-unknown-linux-gnu.tar.gz" -C "$work" cargo-nextest
done
if NEXTEST_DOWNLOAD="file://$work/dl" bash "$here/nextest.sh" install "$work/dest" > /dev/null 2> "$work/err"; then
  bad "install refuses a download with another SHA-256"
elif [ -e "$work/dest/cargo-nextest" ]; then bad "a refused download installs nothing"
else ok; fi

# The two pinned hashes are the release's own, as GitHub reports them; this
# reads them only from the script, so a pin moved without its hashes fails
# at the first install in CI.
n=$(grep -cE '^ *(x86_64|aarch64)\) sha=[0-9a-f]{64} ;;$' "$here/nextest.sh" || true)
[ "$n" = 2 ] && ok || bad "one SHA-256 for each of x86_64 and aarch64, found $n"

if bash "$here/nextest.sh" frobnicate > /dev/null 2>&1; then bad "an unknown command is refused"; else ok; fi

rm -f "$work/err"
echo "test-nextest: $pass of $((pass + fail)) passed"
[ "$fail" -eq 0 ]
