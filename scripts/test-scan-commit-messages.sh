#!/usr/bin/env bash
# Tests scripts/scan-commit-messages.sh in a throwaway repository: a clean
# message passes; an AI credit on a line that starts with #, a plan task id
# and a real-looking email address are each refused and named, with the
# commit; one bad commit among clean ones fails the whole run.
# Forbidden samples are assembled at run time, so this file holds none.
set -euo pipefail
src="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# A git hook runs with GIT_DIR, GIT_INDEX_FILE and other GIT_* variables
# set (in a linked worktree, to absolute paths). Left set, every git command
# below acts on the caller's repository instead of the throwaway one: run
# by a worktree's commit hook on 2026-10-28, scan-commit-messages' test
# wrote its test identity and commit.gpgsign = false into the real
# repository's config, set core.bare, and committed its samples there.
# Every GIT_* variable goes, and each throwaway repository is checked to
# be inside this test's own directory before it is used.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"
git init -q -b main
case "$(git rev-parse --absolute-git-dir)" in
  "$work"/*) ;;
  *) echo "test-scan-commit-messages: refusing: the throwaway repository is not inside $work" >&2; exit 1 ;;
esac
git config user.name "Test User"
git config user.email "test@example.com"
git config commit.gpgsign false
pass=0 fail=0
# commit <message>: one new commit, no hooks; prints its id. It runs in a
# command substitution, so the file changes by the message, not a counter.
commit() { printf '%s\n' "$1" >> f; git add f; git commit -q --no-verify -m "$1"; git rev-parse HEAD; }
# expect <0|1> <description> <text the output must hold, or ""> <commit>...
expect() {
  local want="$1" what="$2" holds="$3" rc=0; shift 3
  bash "$src/scan-commit-messages.sh" "$@" > out 2>&1 || rc=$?
  if [ "$rc" = "$want" ] && { [ -z "$holds" ] || grep -qF -- "$holds" out; }; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL (exit $want and '$holds' expected, got exit $rc): $what"
    sed 's/^/    /' out | tail -8
  fi
}
c=cl a=an
ai="${c}aude" co="${a}thropic"
clean="$(commit "Explain the change

# a line that starts with #, with nothing forbidden")"
hashed="$(commit "Fix it

# Co-Authored-By: ${ai^} <noreply@${co}.com>")"
plan="$(commit "Finish P""0.3")"
mail="$(commit "Ask someone@gm""ail.com")"
expect 0 "a clean message, # line included" "" "$clean"
expect 1 "an AI credit on a # line" "<commit message>: AI tool name:" "$hashed"
expect 1 "the refused commit is named" "refused: $hashed" "$hashed"
expect 1 "a plan task id" "<commit message>: plan task id:" "$plan"
expect 1 "an email address" "<commit message>: email address:" "$mail"
expect 1 "one bad commit among clean ones" "refused: $plan" "$clean" "$plan" "$clean"
expect 1 "a commit that does not exist" "cannot read" "0000000000000000000000000000000000000000"
echo "test-scan-commit-messages: $pass of $((pass + fail)) passed"
[ "$fail" -eq 0 ]
