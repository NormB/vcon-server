#!/usr/bin/env bash
# Tests the pre-push hook in a throwaway repository with its own SSH signing
# key and a local bare remote: a push succeeds only when every outgoing
# commit has a good signature from an allowed signer and its author and
# committer match the pusher's configured identity.
set -euo pipefail
src="$(git rev-parse --show-toplevel)"
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

ssh-keygen -q -t ed25519 -N '' -C test -f "$work/good"
ssh-keygen -q -t ed25519 -N '' -C test -f "$work/stranger"
printf 'test@example.com %s\n' "$(cut -d' ' -f1,2 "$work/good.pub")" > "$work/allowed_signers"

git init -q --bare "$work/remote.git"
repo="$work/repo"
mkdir -p "$repo/hooks"
cp "$src/.githooks/pre-push" "$repo/hooks/"
mkdir -p "$repo/scripts"
cp "$src/scripts/scan-commit-messages.sh" "$src/scripts/scan_forbidden.py" "$src/scripts/hook_tests.py" "$repo/scripts/"
# Stand-ins for the two hook tests the hook runs (scripts/hook_tests.py):
# each records its name in $work/calls and passes, or fails when
# HOOK_TESTS_FAIL is set. Each names the paths watched and watched-dir in
# the form the rule reads, so a push that changes one must run them. (The
# form is written with %s, so this file itself names no such path.)
for name in test-hooks test-pre-push; do
  printf '#!/bin/sh\n: "%s/watched" "%s/watched-dir"\necho %s >> "%s"\n[ -z "${HOOK_TESTS_FAIL:-}" ]\n' \
    '$src' '$src' "$name" "$work/calls" > "$repo/scripts/$name.sh"
  chmod +x "$repo/scripts/$name.sh"
done
: > "$work/calls"
cd "$repo"
git init -q -b main
case "$(git rev-parse --absolute-git-dir)" in
  "$work"/*) ;;
  *) echo "test-pre-push: refusing: the throwaway repository is not inside $work" >&2; exit 1 ;;
esac
git config user.name "Test User"
git config user.email "test@example.com"
git config gpg.format ssh
git config user.signingkey "$work/good.pub"
git config gpg.ssh.allowedSignersFile "$work/allowed_signers"
git config core.hooksPath hooks
git remote add origin "$work/remote.git"

n=0
pass=0
fail=0
c=0
# commit [extra git-commit options...]: one new commit
commit() { c=$((c + 1)); echo "$c" > f; git add f; git commit -q -m "Change $c" "$@"; }
# expect <accept|refuse> <description> <push arguments...>
# A refusal must be the one the case is about: set why to text the hook
# prints for it (why='author is not' expect refuse ...).
expect() {
  local want="$1" what="$2"; shift 2
  n=$((n + 1))
  if git push -q "$@" >"$work/out" 2>&1; then got=accept; else got=refuse; fi
  if [ "$got" = "$want" ] && { [ "$want" = accept ] || { [ -n "${why:-}" ] && grep -qF -- "$why" "$work/out"; }; }; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    if [ "$got" = "$want" ]; then
      echo "FAIL (refused, but not with '${why:-<no why given>}'): $what"
    else
      echo "FAIL ($want expected, got $got): $what"
    fi
    sed 's/^/    /' "$work/out" | tail -15
  fi
}

commit -S
expect accept "signed commit by the configured identity" origin main

commit --no-gpg-sign
why='has no good signature from an allowed signer' expect refuse "unsigned commit" origin main
git reset -q --hard origin/main

commit -S --author "Someone Else <someone@example.com>"
why='author is not the configured identity' expect refuse "wrong author" origin main
git reset -q --hard origin/main

GIT_COMMITTER_NAME="Someone Else" GIT_COMMITTER_EMAIL="someone@example.com" commit -S
why='committer is not the configured identity' expect refuse "wrong committer" origin main
git reset -q --hard origin/main

c=$((c + 1)); echo "$c" > f; git add f
git -c user.signingkey="$work/stranger.pub" commit -q -S -m "Change $c"
why='has no good signature from an allowed signer' expect refuse "signed by a key that is not an allowed signer" origin main
git reset -q --hard origin/main

commit --no-gpg-sign
commit -S
why='has no good signature from an allowed signer' expect refuse "unsigned commit under a signed one" origin main
git reset -q --hard origin/main

# A message the commit-msg hook would refuse, made where that hook did not
# run: on a line that starts with #, which git commit -m keeps.
c=$((c + 1)); echo "$c" > f; git add f
ai_word="cl""aude"; git commit -q -S -m "Change $c

# Co-Authored-By: ${ai_word^} <noreply@example.com>"
why='has a message the commit rules refuse' expect refuse "AI credit on a # line of a message" origin main
git reset -q --hard origin/main

commit -S -m "Finish P""0.3"
why='has a message the commit rules refuse' expect refuse "plan task id in a message" origin main
git reset -q --hard origin/main

git branch copy main
expect accept "new branch holding only pushed commits" origin copy
expect accept "branch deletion" origin --delete copy

commit -S
commit -S
expect accept "several signed commits" origin main

git checkout -q -b topic
commit --no-gpg-sign
commit -S
why='has no good signature from an allowed signer' expect refuse "new branch hiding an unsigned commit under a signed one" origin topic
git checkout -q main

git clone -q -b main "$work/remote.git" "$work/other"
(cd "$work/other" && git -c user.name="Test User" -c user.email="test@example.com" commit -q --allow-empty --no-gpg-sign -m "Elsewhere" && git push -q --no-verify origin main)
commit -S
why='cannot list the commits going to' expect refuse "force push over commits never fetched here" --force origin main

git fetch -q origin
git reset -q --hard origin/main

# The hook tests: run once a push changes a file they read, never otherwise.
# called <names...>: the stand-ins that ran since the last check, in order.
called() {
  n=$((n + 1))
  local got
  got="$(tr '\n' ' ' < "$work/calls" | sed 's/ $//')"
  if [ "$got" = "$*" ]; then pass=$((pass + 1)); else fail=$((fail + 1)); echo "FAIL (hook tests run: '$got', expected '$*'): $what_called"; fi
  : > "$work/calls"
}
commit -S
what_called="a change no hook test reads" expect accept "push of a change no hook test reads" origin main
what_called="a change no hook test reads" called

echo 1 > watched; git add watched; git commit -q -S -m "Watched"
expect accept "push of a change a hook test reads" origin main
what_called="a change a hook test reads" called test-hooks test-pre-push

mkdir -p watched-dir; echo 1 > watched-dir/x; git add watched-dir; git commit -q -S -m "Under a watched directory"
expect accept "push of a change under a directory a hook test reads" origin main
what_called="a file under a directory they read" called test-hooks test-pre-push

echo 2 > watched; git add watched; git commit -q -S -m "Watched again"
HOOK_TESTS_FAIL=1 why='the hook tests failed' expect refuse "push whose hook tests fail" origin main
what_called="a failing hook test" called test-hooks
git reset -q --hard origin/main

echo 3 > watched; git add watched; git commit -q --no-gpg-sign -m "Watched, unsigned"
why='has no good signature from an allowed signer' expect refuse "unsigned push of a change a hook test reads" origin main
what_called="a push already refused" called
git reset -q --hard origin/main

echo 4 > watched; git add watched; git commit -q -S -m "Watched, rule unreadable"
mv scripts/test-pre-push.sh "$work/aside.sh"
why='cannot tell whether the hook tests are needed' expect refuse "push when the rule cannot be read" origin main
mv "$work/aside.sh" scripts/test-pre-push.sh
what_called="an unreadable rule" called
git reset -q --hard origin/main

git checkout -q -b watched-topic
echo 5 > watched; git add watched; git commit -q -S -m "Watched on a new branch"
expect accept "new branch with a change a hook test reads" origin watched-topic
what_called="a new branch" called test-hooks test-pre-push
git checkout -q main

echo "test-pre-push: $pass of $n passed"
[ "$fail" -eq 0 ]
