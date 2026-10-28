#!/usr/bin/env bash
# Runs each named hook test from inside a real git hook, the way the commit
# hook runs scripts/test-scan-commit-messages.sh, and checks that it leaves
# the repository the hook belongs to alone.
#
# A scratch copy of this tree is made a repository with a linked worktree,
# whose pre-commit hook runs the test; a commit is then made in the
# worktree. Its config files must be byte for byte as they were, and the
# worktree's branch must gain exactly that one commit. The real repository
# is never the one tested.
#
# Why: inside a hook git sets GIT_DIR, GIT_INDEX_FILE and other GIT_*
# variables, absolute in a linked worktree. On 2026-10-28 a hook test that
# left them set wrote its test identity, commit.gpgsign = false and
# core.bare = true into this repository's .git/config and committed its
# samples onto the branch being committed.
#
# Usage: scripts/test-hook-isolation.sh <test script, relative to the tree>...
set -euo pipefail
src="$(git rev-parse --show-toplevel)"
# This test may itself run inside a hook; the same rule applies to it.
while IFS='=' read -r name _; do
  case "$name" in GIT_*) unset "$name" ;; esac
done < <(env)
[ $# -gt 0 ] || { echo "usage: $0 <test script>..." >&2; exit 2; }
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
pass=0 fail=0
for script in "$@"; do
  [ -f "$src/$script" ] || { echo "test-hook-isolation: no $script" >&2; exit 2; }
  case_dir="$work/$(basename "$script" .sh)"
  mkdir -p "$case_dir/repo" "$case_dir/hooks"
  # The tracked tree, as the test sees it in a checkout.
  (cd "$src" && git ls-files -z | xargs -0 tar cf - --no-recursion) | tar xf - -C "$case_dir/repo"
  cp "$src/$script" "$case_dir/repo/$script"
  cd "$case_dir/repo"
  git init -q -b main
  case "$(git rev-parse --absolute-git-dir)" in
    "$case_dir"/*) ;;
    *) echo "test-hook-isolation: refusing: the scratch repository is not inside $case_dir" >&2; exit 1 ;;
  esac
  git config user.name "Scratch Owner"
  git config user.email "owner@example.com"
  git config commit.gpgsign false
  git add -A
  git commit -q --no-verify -m "Scratch copy"
  git worktree add -q -b linked "$case_dir/linked"
  printf '#!/usr/bin/env bash\nexec bash %q\n' "$script" > "$case_dir/hooks/pre-commit"
  chmod +x "$case_dir/hooks/pre-commit"
  git config core.hooksPath "$case_dir/hooks"
  configs=("$case_dir/repo/.git/config")
  [ -f "$case_dir/repo/.git/worktrees/linked/config.worktree" ] && configs+=("$case_dir/repo/.git/worktrees/linked/config.worktree")
  before="$(cat "${configs[@]}" | sha256sum)"
  cp "$case_dir/repo/.git/config" "$case_dir/config.before"
  head="$(git -C "$case_dir/linked" rev-parse HEAD)"
  cd "$case_dir/linked"
  echo change > isolation-change.txt
  git add isolation-change.txt
  if git commit -q -m "Change made under the hook" > "$case_dir/out" 2>&1; then committed=yes; else committed=no; fi
  # A test that broke isolation may have left this repository unusable
  # (core.bare set), so every read below tolerates a failing git.
  problems=()
  [ "$committed" = yes ] || problems+=("the commit failed")
  if [ "$(cat "${configs[@]}" | sha256sum)" != "$before" ]; then
    problems+=("the repository's config changed: $( (diff "$case_dir/config.before" "$case_dir/repo/.git/config" || true) | grep '^[<>]' | tr '\n' ' ')")
  fi
  gained="$(git -C "$case_dir/linked" rev-list --count "$head..HEAD" 2>&1 || true)"
  [ "$gained" = 1 ] || problems+=("the branch gained ${gained:-?} commits, not 1")
  tip="$(git -C "$case_dir/linked" log -1 --format='%s|%cn' 2>&1 || true)"
  [ "$tip" = "Change made under the hook|Scratch Owner" ] || problems+=("the branch tip is not the change made: $tip")
  if [ "${#problems[@]}" -eq 0 ]; then
    pass=$((pass + 1))
    echo "test-hook-isolation: $script: ok"
  else
    fail=$((fail + 1))
    echo "FAIL: $script run from a hook:"
    printf '    %s\n' "${problems[@]}"
    sed 's/^/    /' "$case_dir/out" | tail -8
  fi
  cd "$work"
done
echo "test-hook-isolation: $pass of $((pass + fail)) passed"
[ "$fail" -eq 0 ]
