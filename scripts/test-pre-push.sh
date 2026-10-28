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
cp "$src/scripts/scan-commit-messages.sh" "$src/scripts/scan_forbidden.py" "$repo/scripts/"
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
expect() {
  local want="$1" what="$2"; shift 2
  n=$((n + 1))
  if git push -q "$@" >"$work/out" 2>&1; then got=accept; else got=refuse; fi
  if [ "$got" = "$want" ]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    echo "FAIL ($want expected, got $got): $what"
    sed 's/^/    /' "$work/out" | head -5
  fi
}

commit -S
expect accept "signed commit by the configured identity" origin main

commit --no-gpg-sign
expect refuse "unsigned commit" origin main
git reset -q --hard origin/main

commit -S --author "Someone Else <someone@example.com>"
expect refuse "wrong author" origin main
git reset -q --hard origin/main

GIT_COMMITTER_NAME="Someone Else" GIT_COMMITTER_EMAIL="someone@example.com" commit -S
expect refuse "wrong committer" origin main
git reset -q --hard origin/main

c=$((c + 1)); echo "$c" > f; git add f
git -c user.signingkey="$work/stranger.pub" commit -q -S -m "Change $c"
expect refuse "signed by a key that is not an allowed signer" origin main
git reset -q --hard origin/main

commit --no-gpg-sign
commit -S
expect refuse "unsigned commit under a signed one" origin main
git reset -q --hard origin/main

# A message the commit-msg hook would refuse, made where that hook did not
# run: on a line that starts with #, which git commit -m keeps.
c=$((c + 1)); echo "$c" > f; git add f
ai_word="cl""aude"; git commit -q -S -m "Change $c

# Co-Authored-By: ${ai_word^} <noreply@example.com>"
expect refuse "AI credit on a # line of a message" origin main
git reset -q --hard origin/main

commit -S -m "Finish P""0.3"
expect refuse "plan task id in a message" origin main
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
expect refuse "new branch hiding an unsigned commit under a signed one" origin topic
git checkout -q main

git clone -q -b main "$work/remote.git" "$work/other"
(cd "$work/other" && git -c user.name="Test User" -c user.email="test@example.com" commit -q --allow-empty --no-gpg-sign -m "Elsewhere" && git push -q --no-verify origin main)
commit -S
expect refuse "force push over commits never fetched here" --force origin main

echo "test-pre-push: $pass of $n passed"
[ "$fail" -eq 0 ]
