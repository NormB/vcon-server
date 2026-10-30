#!/usr/bin/env bash
# Tests the git hooks in a throwaway repository: each forbidden item, staged
# or written in the commit message, must be refused; clean changes and the
# reserved test values (RFC 2606 domains, 555-01xx numbers, RFC 5737 and
# RFC 3849 addresses) must pass. Forbidden samples are assembled at run time
# so that this file never contains them itself.
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
export CARGO_TARGET_DIR="$work/target"

repo="$work/repo"
mkdir -p "$repo/crates/fixture/src"
cd "$repo"
git init -q -b main
case "$(git rev-parse --absolute-git-dir)" in
  "$work"/*) ;;
  *) echo "test-hooks: refusing: the throwaway repository is not inside $work" >&2; exit 1 ;;
esac
git config user.name "Test User"
git config user.email "test@example.com"
git config commit.gpgsign false
git config core.hooksPath .githooks
cp -R "$src/.githooks" "$src/scripts" "$src/.gitignore" "$src/rust-toolchain.toml" .
# A workspace shaped like the real one: one crate under crates/, ready to
# publish, so the hook's package check has a package to read.
cat > Cargo.toml <<'TOML'
[workspace]
resolver = "3"
members = ["crates/fixture"]

[workspace.metadata]
published = ["hook-fixture"]
TOML
cat > crates/fixture/Cargo.toml <<'TOML'
[package]
name = "hook-fixture"
version = "0.1.0"
edition = "2024"
rust-version = "1.99"
description = "A crate the hook tests commit."
license = "MIT OR Apache-2.0"
repository = "https://github.com/NormB/vcon-server"
homepage = "https://vcon-server.com"
documentation = "https://docs.rs/hook-fixture"
readme = "README.md"
keywords = ["fixture"]
categories = ["development-tools::testing"]
include = ["/src/**/*.rs", "/README.md", "/LICENSE-MIT", "/LICENSE-APACHE"]
TOML
printf '# hook-fixture\n\nSee [the site](https://vcon-server.com/).\n' > crates/fixture/README.md
cp "$src/LICENSE-MIT" "$src/LICENSE-APACHE" .
cp "$src/LICENSE-MIT" "$src/LICENSE-APACHE" crates/fixture/
cat > crates/fixture/src/lib.rs <<'RS'
/// Adds one.
pub fn add_one(x: u32) -> u32 {
    x + 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn adds_one() {
        assert_eq!(super::add_one(1), 2, "one more than one is two");
    }
}
RS
# The hook checks that every path the Best Practices answers link to exists.
links="https://github.com/NormB/vcon-server/blob/main"
printf '{"build": "%s/Cargo.toml"}\n' "$links" > .bestpractices.json
cargo generate-lockfile -q
git add -A
git commit -q -m "Start the fixture" >"$work/first" 2>&1 || { echo "FAIL: the clean first commit was refused"; sed "s/^/    /" "$work/first"; exit 1; }
# The hook builds the staged copy in a directory of its own, even when the
# caller set CARGO_TARGET_DIR: a test binary records the directory it was
# built from, so one shared with the caller's builds would later run against
# the deleted copy.
if [ -d "$CARGO_TARGET_DIR/debug" ] || [ ! -d "$CARGO_TARGET_DIR/pre-commit/debug" ]; then
  echo "FAIL: the hook did not build in \$CARGO_TARGET_DIR/pre-commit"; exit 1
fi

pass=0
fail=0
n=0
# expect <accept|refuse> <description> <file> <content> [message]
# A refusal must be the one the case is about: set why to text its finding
# prints (why='notes.txt: AI tool name:' expect refuse ...). A commit
# refused only by some other check fails the case.
expect() {
  local want="$1" what="$2" file="$3" content="$4" msg="${5:-Change a file}"
  n=$((n + 1))
  mkdir -p "$(dirname "$file")"
  printf '%s\n' "$content" > "$file"
  git add -f -- "$file"
  if git commit -q -m "$msg" >"$work/out" 2>&1; then got=accept; else got=refuse; fi
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
  git reset -q --hard HEAD
  git clean -qfdx -e target
}

c=cl; a=an
ai_word="${c}aude"; ai_co="${a}thropic"
sur="br""andinger"
robot="$(printf '\xf0\x9f\xa4\x96')"

expect accept "clean change" notes.txt "plain words"
why='notes.txt: AI tool name:' expect refuse "AI tool named" notes.txt "written by ${ai_word^}"
why='notes.txt: AI company name:' expect refuse "AI company named" notes.txt "see ${ai_co}.com"
why='notes.txt: robot emoji:' expect refuse "robot emoji" notes.txt "done $robot"
why='notes.txt: email address:' expect refuse "real-looking email" notes.txt "mail someone@gm""ail.com"
expect accept "reserved email" notes.txt "mail someone@example.com"
expect accept "reserved .invalid email" notes.txt "mail a@host.invalid"
why='notes.txt: phone number:' expect refuse "phone number" notes.txt "call 212-867-""5309"
why='notes.txt: phone number:' expect refuse "E.164 phone" notes.txt "call +44207""9460000"
expect accept "reserved 555-01xx" notes.txt "call 212-555-0100"
why='notes.txt: IP address:' expect refuse "public IPv4" notes.txt "host 8.8.""4.4"
why='notes.txt: IP address:' expect refuse "private IPv4" notes.txt "host 172.31.""78.11"
expect accept "RFC 5737 IPv4" notes.txt "host 192.0.2.10"
expect accept "loopback IPv4" notes.txt "bind 127.0.0.1"
why='notes.txt: IP address:' expect refuse "public IPv6" notes.txt "host 2606:47""00:4700:"":1111"
expect accept "RFC 3849 IPv6" notes.txt "host 2001:db8::5"
why="notes.txt: owner's name:" expect refuse "owner surname" notes.txt "by Norm ${sur^}"
expect accept "owner's credit in the README" README.md "Copyright 2026 Norm ${sur^}"
expect accept "owner's credit in the site config" website/config.toml "author = \"Norm ${sur^}\""
expect accept "Best Practices answer linking to a file in the tree" .bestpractices.json "{\"a\": \"$links/crates/fixture/src/lib.rs\"}"
why='test_every_path_bestpractices_links_to_exists' expect refuse "Best Practices answer linking to a file that moved" .bestpractices.json "{\"a\": \"$links/src/gone.rs\"}"
why="website/content/a.md: owner's name:" expect refuse "owner surname in a site page" website/content/a.md "by Norm ${sur^}"
why="crates/x/README.md: owner's name:" expect refuse "owner surname in a nested README" crates/x/README.md "by Norm ${sur^}"
why='notes.txt: design rule id:' expect refuse "design rule id" notes.txt "see FMT""-3"
why='notes.txt: design rule id:' expect refuse "design rule id in lower case" notes.txt "see sto""-4"
why='notes.txt: design rule id:' expect refuse "backend design rule id" notes.txt "see BKD""-4"
why='crates/x/src/a.rs: design rule id:' expect refuse "design rule id in a name" crates/x/src/a.rs "fn written_in_the_sto""_3_order() {}"
expect accept "word ending in a rule prefix" crates/x/src/b.rs "let restore_2 = histo_3;"
why='notes.txt: design decision id:' expect refuse "design decision id" notes.txt "see CD""8"
why='notes.txt: scope decision id:' expect refuse "scope decision id" notes.txt "see D""40"
why='notes.txt: plan task id:' expect refuse "plan task id" notes.txt "done P""0.3"
why='notes.txt: plan task id:' expect refuse "plan task id past phase ten" notes.txt "done P""11.7"
why='notes.txt: plan task id:' expect refuse "plan task id in phase fourteen" notes.txt "see P""14.12"
expect accept "crate README with absolute links" crates/fixture/README.md "# hook-fixture

See [the docs](https://docs.rs/hook-fixture)."
why="README.md: relative link 'docs/guide.md'" expect refuse "crate README with a relative link" crates/fixture/README.md "# hook-fixture

See [the guide](docs/guide.md)."
# Ids of any length: phases and tasks past the first ten, and decisions and
# rules past 99, are as private as the first ones.
why='notes.txt: plan task id:' expect refuse "plan task id in phase 11" notes.txt "done P""11.4"
why='notes.txt: plan task id:' expect refuse "plan task id in phase 13" notes.txt "done P""13.7"
why='notes.txt: plan task id:' expect refuse "plan task id with a two-digit task" notes.txt "done P""14.10"
why='notes.txt: plan task id:' expect refuse "plan task id in phase 100" notes.txt "done P""100.1"
why='notes.txt: plan task id:' expect refuse "plan task id with a three-digit task" notes.txt "done P""2.100"
why='notes.txt: design decision id:' expect refuse "design decision id past 99" notes.txt "see CD""100"
why='notes.txt: scope decision id:' expect refuse "scope decision id past 99" notes.txt "see D""100"
why='notes.txt: design rule id:' expect refuse "design rule id past 99" notes.txt "see FMT""-100"
expect accept "names like plan ids" notes.txt "RFC 9457, P-256, P256, XP11.4, HTTP/1.1, 0.1.0, AES-256-GCM, D-Bus"
why='notes.txt: private draft number:' expect refuse "draft number" notes.txt "from draft ""6.8"
ietf_tel="+1234567""8901"
expect accept "IETF example number in IETF fixtures" crates/x/tests/fixtures/ietf/a.vcon "{\"tel\": \"$ietf_tel\"}"
why='notes.txt: phone number:' expect refuse "IETF example number elsewhere" notes.txt "tel $ietf_tel"
why='crates/x/tests/fixtures/ietf/b.vcon: phone number:' expect refuse "other number in IETF fixtures" crates/x/tests/fixtures/ietf/b.vcon "call 212-867-""5309"
why='crates/x/tests/fixtures/ietf/d.vcon: phone number:' expect refuse "other E.164 number in IETF fixtures" crates/x/tests/fixtures/ietf/d.vcon "tel +44207""9460000"
why='crates/x/tests/fixtures/c.json: phone number:' expect refuse "IETF example number in other fixtures" crates/x/tests/fixtures/c.json "tel $ietf_tel"
expect accept "ids allowed in fixtures" crates/x/tests/fixtures/a.json '{"k": "D''40 FMT''-3"}'
why='pre-commit: refused: private document export staged: vcon-server — Notes.md' expect refuse "export staged by force" "vcon-server — Notes.md" "words"
why='<commit message>: AI tool name:' expect refuse "AI trailer in message" notes.txt "message case one" "Fix it

Co-Authored-By: ${ai_word^} <noreply@${ai_co}.com>"
# git commit -m keeps lines that start with # (cleanup mode whitespace),
# so a # line is stored in the commit and must be scanned too.
why='<commit message>: AI tool name:' expect refuse "AI trailer on a # line in message" notes.txt "message case six" "Fix it

# Co-Authored-By: ${ai_word^} <noreply@${ai_co}.com>"
why='<commit message>: plan task id:' expect refuse "plan task id in message" notes.txt "message case two" "Finish P""0.3"
why='<commit message>: plan task id:' expect refuse "later plan task id in message" notes.txt "message case five" "Finish P""14.10"
why='<commit message>: email address:' expect refuse "email in message" notes.txt "message case three" "Ask someone@gm""ail.com"
expect accept "clean message" notes.txt "message case four" "Explain the change"
why='Diff in' expect refuse "unformatted code" crates/fixture/src/lib.rs "pub fn add_one(x:u32)->u32{x+1}"
why='clippy::needless_return' expect refuse "clippy warning" crates/fixture/src/lib.rs '/// Adds one.
pub fn add_one(x: u32) -> u32 {
    let y = x;
    return y + 1;
}'
why='tests::adds_one' expect refuse "failing test" crates/fixture/src/lib.rs '/// Adds one.
pub fn add_one(x: u32) -> u32 {
    x + 2
}

#[cfg(test)]
mod tests {
    #[test]
    fn adds_one() {
        assert_eq!(super::add_one(1), 2, "one more than one is two");
    }
}'

why='crates/fixture/src/lib.rs: 1 assertion(s) without a message' expect refuse "assertion without a message" crates/fixture/src/lib.rs '/// Adds one.
pub fn add_one(x: u32) -> u32 {
    x + 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn adds_one() {
        assert_eq!(super::add_one(1), 2);
    }
}'
why='glob-import: ' expect refuse "glob import" crates/fixture/src/lib.rs '/// Adds one.
pub fn add_one(x: u32) -> u32 {
    use std::ops::*;
    x.add(1)
}'

echo "test-hooks: $pass of $n passed"
[ "$fail" -eq 0 ]
