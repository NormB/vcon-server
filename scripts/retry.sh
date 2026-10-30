#!/usr/bin/env bash
# retry.sh <command> [args...]: runs the command until it succeeds, at most
# RETRY_ATTEMPTS times (default 5), waiting RETRY_DELAY seconds (default
# 10) after the first failure and twice as long after each later one. The
# last exit code is kept. RETRY_BETWEEN, if set, is a shell command run
# between attempts (dnf: RETRY_BETWEEN='dnf clean metadata', since a mirror
# that serves an empty repomd.xml fails the parse, which dnf's own retries
# do not cover). For a download a runner's network can drop;
# scripts/check_workflow_downloads.py requires it there.
set -u
[ $# -gt 0 ] || { echo "usage: retry.sh <command> [args...]" >&2; exit 2; }
attempts=${RETRY_ATTEMPTS:-5}
delay=${RETRY_DELAY:-10}
for ((i = 1; ; i++)); do
  "$@" && exit 0
  rc=$?
  if [ "$i" -ge "$attempts" ]; then
    echo "retry: '$*' failed $i times; giving up (exit $rc)" >&2
    exit "$rc"
  fi
  echo "retry: '$*' failed (exit $rc), attempt $i of $attempts; again in ${delay}s" >&2
  if [ -n "${RETRY_BETWEEN:-}" ]; then bash -c "$RETRY_BETWEEN" >&2 || true; fi
  sleep "$delay"
  delay=$((delay * 2))
done
