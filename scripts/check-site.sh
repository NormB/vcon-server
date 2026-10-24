#!/usr/bin/env bash
# Builds the website from website/ and checks what would be published:
# the home page exists and names the project, links to the repository,
# and the custom domain is vcon-server.com. Then `zola check` fails on any
# broken internal link. The Pages workflow runs this before it publishes;
# run it by hand after changing the site.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

fail() {
  echo "check-site: $*" >&2
  exit 1
}

[ -d "$root/website" ] || fail "no website/ directory"
zola --root "$root/website" build --output-dir "$out/public" >/dev/null

home="$out/public/index.html"
[ -s "$home" ] || fail "the build has no home page"
grep -qF '<title>vcon-server' "$home" || fail "the home page title does not name vcon-server"
grep -qF 'https://github.com/NormB/vcon-server' "$home" || fail "the home page does not link to the repository"
[ "$(cat "$out/public/CNAME" 2>/dev/null)" = "vcon-server.com" ] || fail "CNAME is not vcon-server.com"

zola --root "$root/website" check >/dev/null || fail "zola check found a broken link"
echo "check-site: ok"
