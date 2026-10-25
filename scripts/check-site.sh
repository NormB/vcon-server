#!/usr/bin/env bash
# Builds the website from website/ and checks what would be published:
# the home page names the project and links to the repository, the custom
# domain is vcon-server.com, every problem type has its built page, the
# search, 404 and llms.txt files exist, and no page carries an inline
# script, an inline style or an event-handler attribute (the pages'
# content-security policy refuses them, so they would not run anyway).
# Then `zola check` fails on any broken internal link. The Pages workflow
# runs this before it publishes; run it by hand after changing the site.
# That the problem pages and the configuration page match the code is
# checked by the test suite (api::site_pages, config::tests::site_page).
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
site="$out/public"

home="$site/index.html"
[ -s "$home" ] || fail "the build has no home page"
grep -qF '<title>vcon-server' "$home" || fail "the home page title does not name vcon-server"
grep -qF 'https://github.com/NormB/vcon-server' "$home" || fail "the home page does not link to the repository"
[ "$(cat "$site/CNAME" 2>/dev/null)" = "vcon-server.com" ] || fail "CNAME is not vcon-server.com"
grep -qF 'https://www.linkedin.com/in/nbrandinger' "$home" || fail "the home page lacks the author's credit"
grep -qF 'https://github.com/sponsors/NormB' "$home" || fail "the home page lacks the sponsor link"

for f in 404.html llms.txt robots.txt search_index.en.js elasticlunr.min.js js/search.js; do
  [ -s "$site/$f" ] || fail "the build has no $f"
done

# Search works only if the pages load its three scripts.
for script in elasticlunr.min.js search_index.en.js js/search.js; do
  grep -qF "$script" "$home" || fail "the home page does not load $script"
done

pages=0
for src in "$root"/website/content/problems/*.md; do
  name="$(basename "$src" .md)"
  [ "$name" = "_index" ] && continue
  built="$site/problems/$name/index.html"
  [ -s "$built" ] || fail "problem type $name has no built page"
  grep -qF "https://vcon-server.com/problems/$name" "$built" || fail "the $name page does not show its type"
  pages=$((pages + 1))
done
[ "$pages" -gt 0 ] || fail "no problem pages"

# Every <script> must load a file; no <style> element, style attribute or
# on...= handler anywhere.
while IFS= read -r -d '' html; do
  rel="${html#"$site"/}"
  if grep -Pq '<script(?![^>]*\ssrc=)[^>]*>' "$html"; then fail "$rel has an inline script"; fi
  if grep -Pqi '<style[\s>]' "$html"; then fail "$rel has a <style> element"; fi
  if grep -Pqi '<[a-z][^>]*\sstyle\s*=' "$html"; then fail "$rel has a style attribute"; fi
  if grep -Pqi '<[a-z][^>]*\son[a-z]+\s*=' "$html"; then fail "$rel has an event-handler attribute"; fi
done < <(find "$site" -name '*.html' -print0)

zola --root "$root/website" check >/dev/null || fail "zola check found a broken link"
echo "check-site: ok ($pages problem pages)"
