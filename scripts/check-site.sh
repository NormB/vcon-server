#!/usr/bin/env bash
# Builds the website from website/ and checks what would be published:
# the home page names the project and links to the repository, the custom
# domain is vcon-server.com, every problem type has its built page, the
# search, 404 and llms.txt files exist, the icons the pages name are built
# and every SVG is a plain drawing, and no page carries an inline
# script, an inline style or an event-handler attribute (the pages'
# content-security policy refuses them, so they would not run anyway).
# Then `zola check` fails on any broken internal link, and
# check_site_links.py on any internal link or anchor in the built pages that
# does not resolve, or a problem type without its page. The Pages workflow
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
# The title's tagline is the headline, first letter lowered.
headline="$(grep -oP '(?<=<h1>)[^<]+(?=</h1>)' "$home" | head -n 1)"
[ -n "$headline" ] || fail "the home page has no headline"
grep -qF "<title>vcon-server — ${headline,}</title>" "$home" || fail "the home page title is not its headline"
grep -qF 'https://github.com/NormB/vcon-server' "$home" || fail "the home page does not link to the repository"
[ "$(cat "$site/CNAME" 2>/dev/null)" = "vcon-server.com" ] || fail "CNAME is not vcon-server.com"
grep -qF 'https://www.linkedin.com/in/nbrandinger' "$home" || fail "the home page lacks the author's credit"
grep -qF 'https://github.com/sponsors/NormB' "$home" || fail "the home page lacks the sponsor link"

for f in 404.html llms.txt robots.txt openapi.json search_index.en.js elasticlunr.min.js js/search.js; do
  [ -s "$site/$f" ] || fail "the build has no $f"
done

# The icons each page names are built, and every SVG on the site or in the
# brand files is a plain drawing: no script, style, link or embedded object.
for f in favicon.svg favicon.ico apple-touch-icon.png og-image.png; do
  [ -s "$site/$f" ] || fail "the build has no $f"
done
grep -qF 'rel="icon" href="https://vcon-server.com/favicon.ico" sizes="32x32"' "$home" || fail "the home page does not name favicon.ico"
grep -qF 'rel="icon" href="https://vcon-server.com/favicon.svg" type="image/svg+xml"' "$home" || fail "the home page does not name favicon.svg"
grep -qF 'rel="apple-touch-icon" href="https://vcon-server.com/apple-touch-icon.png"' "$home" || fail "the home page does not name the touch icon"
grep -qF 'property="og:image" content="https://vcon-server.com/og-image.png"' "$home" || fail "the home page does not name its preview image"
for svg in "$site"/*.svg "$root"/assets/brand/*.svg; do
  if grep -Pqi '<script|<style|<foreignObject|href\s*=|url\(' "$svg"; then
    rel="${svg#"$site"/}"
    fail "${rel#"$root"/} is not a plain drawing"
  fi
done

# The contributor agreement on the site is CLA.md, word for word, after the
# page's front matter.
[ -s "$root/CLA.md" ] || fail "no CLA.md"
cla_page="$root/website/content/cla.md"
[ -s "$cla_page" ] || fail "no website/content/cla.md"
awk 'n >= 2 { print } /^\+\+\+$/ { n++ }' "$cla_page" | tail -n +2 | cmp -s - "$root/CLA.md" \
  || fail "website/content/cla.md does not reproduce CLA.md"
[ -s "$site/cla/index.html" ] || fail "the build has no CLA page"

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

# The home page's numbers come only from website/data/metrics.toml, which
# pages.yml writes from CI's runs of the commit it deploys
# (scripts/published_metrics.py) and nothing commits. The page states exactly
# what that file says, or no number when it is absent. Then the same site,
# built with a made-up file, must state the made-up numbers: a number typed
# into the template, or a template that stopped reading the file, fails.
metrics_line() { grep -oP '<p class="metrics"[^>]*>.*?</p>' "$1" || true; }
metrics_check() { # built page, data file
  local page=$1 data=$2 line tests coverage
  line="$(metrics_line "$page")"
  if [ ! -f "$data" ]; then
    [ -z "$line" ] || fail "the home page states numbers with no data file: $line"
    return
  fi
  tests="$(sed -n 's/^automated_tests = \([0-9][0-9]*\)$/\1/p' "$data")"
  coverage="$(sed -n 's/^line_coverage = "\([0-9.]*\)"$/\1/p' "$data")"
  [ -n "$tests" ] && [ -n "$coverage" ] || fail "$data does not hold automated_tests and line_coverage"
  grep -qF "data-tests=\"$tests\"" <<<"$line" && grep -qF "$tests automated tests" <<<"$line" \
    || fail "the home page does not state the $tests automated tests $data holds: ${line:-no metrics line}"
  grep -qF "data-coverage=\"$coverage\"" <<<"$line" && grep -qF "$coverage% of lines" <<<"$line" \
    || fail "the home page does not state the $coverage% coverage $data holds: ${line:-no metrics line}"
}
metrics_check "$home" "$root/website/data/metrics.toml"
# Two made-up files with different numbers: a number typed into the template
# could match one of them, never both.
fixture="$out/fixture"
cp -r "$root/website" "$fixture"
mkdir -p "$fixture/data"
for made_up in '4321 765 987 77.5 0123456' '1234 81 90 90.0 89abcde'; do
  read -r tests covered counted coverage commit <<<"$made_up"
  printf '%s\n' "automated_tests = $tests" "lines_covered = $covered" "lines_counted = $counted" \
    "line_coverage = \"$coverage\"" "commit = \"${commit}89abcdef0123456789abcdef012345678\"" \
    'run = "https://github.com/NormB/vcon-server/actions/runs/1"' > "$fixture/data/metrics.toml"
  zola --root "$fixture" build --output-dir "$out/fixture-public" --force >/dev/null
  metrics_check "$out/fixture-public/index.html" "$fixture/data/metrics.toml"
  grep -qF "$covered of $counted lines" "$out/fixture-public/index.html" || fail "the home page does not state the lines counted"
  grep -qF "<code>$commit</code>" "$out/fixture-public/index.html" || fail "the home page does not name the commit measured"
done

zola --root "$root/website" check >/dev/null || fail "zola check found a broken link"
# Every internal link and anchor in the built pages, and the problem types
# in openapi.json against the built problem pages; nothing is fetched.
python3 -I "$root/scripts/check_site_links.py" "$site" "https://vcon-server.com" \
  || fail "the built site has a broken internal link or problem type (listed above)"
echo "check-site: ok ($pages problem pages)"
