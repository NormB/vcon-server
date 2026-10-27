#!/usr/bin/env bash
# Tests the pre-push hook in a throwaway repository with its own SSH signing
# key and a local bare remote: a push succeeds only when every outgoing
# commit has a good signature from an allowed signer and its author and
# committer match the pusher's configured identity.
set -euo pipefail
src="$(git rev-parse --show-toplevel)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

ssh-keygen -q -t ed25519 -N '' -C test -f "$work/good"
ssh-keygen -q -t ed25519 -N '' -C test -f "$work/stranger"
printf 'test@example.com %s\n' "$(cut -d' ' -f1,2 "$work/good.pub")" > "$work/allowed_signers"

git init -q --bare "$work/remote.git"
repo="$work/repo"
mkdir -p "$repo/hooks"
cp "$src/.githooks/pre-push" "$repo/hooks/"
cd "$repo"
git init -q -b main
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
    sed 's/^/    /' "$work/out" | head -12
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

echo "test-pre-push: $pass of $n passed"
[ "$fail" -eq 0 ]
