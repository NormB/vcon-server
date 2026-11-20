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

# A `gh` that fails reads the API budget through scripts/gh_rate_limit.py:
# a spent budget is waited for (up to RETRY_RATE_LIMIT_MAX seconds) and the
# command is run again; a reset further off ends the retries at once. The
# stub gh answers `gh api rate_limit` from $work/budget (exit $work/budget-rc)
# and fails its first <failures> other calls with gh's rate-limit message.
mkdir -p "$work/bin"
cat > "$work/bin/gh" <<'SH'
#!/usr/bin/env bash
if [ "$*" = "api rate_limit" ]; then
  cat "$GH_STUB/budget"; exit "$(cat "$GH_STUB/budget-rc")"
fi
n=$(( $(cat "$GH_STUB/count" 2>/dev/null || echo 0) + 1 )); echo "$n" > "$GH_STUB/count"
[ "$n" -gt "$(cat "$GH_STUB/failures")" ] && exit 0
echo "gh: API rate limit exceeded for installation." >&2; exit 1
SH
chmod +x "$work/bin/gh"
expect_gh() { # <description> <want rc> <want runs> <failures> <remaining> <reset from now> <budget rc> [max] [attempts]
  local what=$1 want_rc=$2 want_runs=$3 rc=0 runs
  rm -f "$work/count"
  echo "$4" > "$work/failures"
  echo "{\"resources\":{\"core\":{\"limit\":5000,\"remaining\":$5,\"reset\":$(( $(date +%s) + $6 )),\"used\":1}}}" > "$work/budget"
  echo "$7" > "$work/budget-rc"
  GH_STUB="$work" PATH="$work/bin:$PATH" RETRY_DELAY=0 RETRY_ATTEMPTS="${9:-5}" RETRY_RATE_LIMIT_MARGIN=0 \
    RETRY_RATE_LIMIT_MAX="${8:-900}" bash "$here/retry.sh" gh api repos/o/r 2> "$work/err" || rc=$?
  runs=$(cat "$work/count")
  if [ "$rc" = "$want_rc" ] && [ "$runs" = "$want_runs" ]; then pass=$((pass + 1)); else
    fail=$((fail + 1)); echo "FAIL ($what): exit $rc and $runs runs, wanted exit $want_rc and $want_runs runs"; sed 's/^/    /' "$work/err"; fi
}
expect_gh "a spent budget is waited for, and that run uses no attempt" 0 2 1 0 0 0 900 1
expect_gh "only one wait for the reset per command" 1 3 9 0 0 0 900 2
expect_gh "a reset further off than RETRY_RATE_LIMIT_MAX ends the retries" 1 1 9 0 5000 0
expect_gh "with requests left, the usual retries run" 1 5 9 10 0 0
expect_gh "a budget that cannot be read leaves the usual retries" 1 5 9 0 0 1
expect_gh "RETRY_RATE_LIMIT_MAX=0 turns the budget check off" 1 5 9 0 1 0 0
# Only gh reads the budget: another command that fails with a spent budget
# on hand is retried the usual way (one attempt here, so one run).
rm -f "$work/count"; rc=0
GH_STUB="$work" PATH="$work/bin:$PATH" RETRY_DELAY=0 RETRY_ATTEMPTS=1 RETRY_RATE_LIMIT_MARGIN=0 \
  bash "$here/retry.sh" "$work/flaky" 1 "$work/count" 2> "$work/err" || rc=$?
if [ "$rc" = 7 ] && [ "$(cat "$work/count")" = 1 ]; then pass=$((pass + 1)); else
  fail=$((fail + 1)); echo "FAIL (only gh reads the budget): exit $rc and $(cat "$work/count") runs, wanted exit 7 and 1 run"; fi
echo "test-retry: $pass of $((pass + fail)) passed"
[ "$fail" -eq 0 ]
