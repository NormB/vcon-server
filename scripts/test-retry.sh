#!/usr/bin/env bash
# Tests scripts/retry.sh: a command that fails then succeeds is run again
# until it succeeds; one that keeps failing is run exactly RETRY_ATTEMPTS
# times and its exit code is kept; one that succeeds runs once.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
pass=0 fail=0
# flaky <failures> <counter>: fails the first <failures> calls with exit 7.
cat > "$work/flaky" <<'SH'
#!/usr/bin/env bash
n=$(( $(cat "$2" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$2"
[ "$n" -gt "$1" ] || exit 7
SH
chmod +x "$work/flaky"
expect() { # <description> <want rc> <want runs> <failures> [attempts]
  local what=$1 want_rc=$2 want_runs=$3 failures=$4 rc=0
  rm -f "$work/count"
  RETRY_DELAY=0 RETRY_ATTEMPTS="${5:-5}" bash "$here/retry.sh" "$work/flaky" "$failures" "$work/count" 2> "$work/err" || rc=$?
  runs=$(cat "$work/count")
  if [ "$rc" = "$want_rc" ] && [ "$runs" = "$want_runs" ]; then pass=$((pass + 1)); else
    fail=$((fail + 1)); echo "FAIL ($what): exit $rc and $runs runs, wanted exit $want_rc and $want_runs runs"; sed 's/^/    /' "$work/err"; fi
}
expect "a command that succeeds runs once" 0 1 0
expect "two failures, then success" 0 3 2
expect "it gives up after RETRY_ATTEMPTS runs and keeps the exit code" 7 5 9
expect "RETRY_ATTEMPTS is honoured" 7 2 9 2
# RETRY_BETWEEN runs between attempts, not before the first or after the last.
rm -f "$work/count" "$work/between"
RETRY_DELAY=0 RETRY_ATTEMPTS=4 RETRY_BETWEEN="echo x >> '$work/between'" bash "$here/retry.sh" "$work/flaky" 2 "$work/count" 2> "$work/err" || true
if [ "$(wc -l < "$work/between" 2>/dev/null || echo 0)" = 2 ] && [ "$(cat "$work/count")" = 3 ]; then pass=$((pass + 1)); else
  fail=$((fail + 1)); echo "FAIL (RETRY_BETWEEN runs once between each pair of attempts): ran $(wc -l < "$work/between" 2>/dev/null || echo 0) times for $(cat "$work/count") runs, wanted 2 for 3"; fi
echo "test-retry: $pass of $((pass + fail)) passed"
[ "$fail" -eq 0 ]
