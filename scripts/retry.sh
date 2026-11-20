#!/usr/bin/env bash
# retry.sh <command> [args...]: runs the command until it succeeds, at most
# RETRY_ATTEMPTS times (default 5), waiting RETRY_DELAY seconds (default
# 10) after the first failure and twice as long after each later one. The
# last exit code is kept. For a download a runner's network can drop;
# scripts/check_workflow_downloads.py requires it there. Not for dnf: a
# second try of the same command reads the same mirror, so a dnf install
# goes through scripts/dnf_install.py, which changes the source.
#
# A failed `gh` reads the API budget (scripts/gh_rate_limit.py wait): when
# none is left and the reset is at most RETRY_RATE_LIMIT_MAX seconds away
# (default 900; 0 turns this off), it waits for the reset plus
# RETRY_RATE_LIMIT_MARGIN seconds (default 5) and runs the command again
# without using an attempt, once per command; a reset further off ends the
# retries at once, since every try before it would be refused the same way.
set -u
[ $# -gt 0 ] || { echo "usage: retry.sh <command> [args...]" >&2; exit 2; }
here="$(dirname "$0")"
attempts=${RETRY_ATTEMPTS:-5}
delay=${RETRY_DELAY:-10}
rate_max=${RETRY_RATE_LIMIT_MAX:-900}
rate_margin=${RETRY_RATE_LIMIT_MARGIN:-5}
waited=0
for ((i = 1; ; i++)); do
  "$@" && exit 0
  rc=$?
  if [ "$(basename -- "$1")" = gh ] && [ "$rate_max" -gt 0 ] && [ "$waited" -eq 0 ]; then
    python3 -I "$here/gh_rate_limit.py" wait --max "$rate_max" --margin "$rate_margin"
    case $? in
      3) waited=1; i=$((i - 1)); continue ;;
      1) echo "retry: '$*' failed (exit $rc) on a spent API budget; giving up" >&2; exit "$rc" ;;
    esac
  fi
  if [ "$i" -ge "$attempts" ]; then
    echo "retry: '$*' failed $i times; giving up (exit $rc)" >&2
    exit "$rc"
  fi
  echo "retry: '$*' failed (exit $rc), attempt $i of $attempts; again in ${delay}s" >&2
  sleep "$delay"
  delay=$((delay * 2))
done
