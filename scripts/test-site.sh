#!/usr/bin/env bash
# Runs the website's browser tests: builds a copy of website/ in a temporary
# directory, serves it on a port the server picks, runs the Playwright tests in
# site-tests/ against it, then stops the server and removes everything it
# made. The home page shows its numbers only when website/data/metrics.toml
# exists, which only the Pages workflow writes; without one, the copy gets
# made-up numbers, so the numbers are always on the page tested. Needs zola,
# Node and the Playwright browser:
#   npm ci --prefix site-tests
#   npx --prefix site-tests playwright install --only-shell chromium
# SITE_SCREENSHOTS=<dir> saves a full-page screenshot of each width and
# color scheme there. Any arguments are passed to `playwright test`.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
server=""
cleanup() {
  if [ -n "$server" ]; then
    kill -TERM "$server" 2>/dev/null || true
    wait "$server" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

cp -r "$root/website" "$work/website"
rm -rf "$work/website/public"
if [ ! -f "$work/website/data/metrics.toml" ]; then
  mkdir -p "$work/website/data"
  printf '%s\n' 'automated_tests = 4321' 'lines_covered = 765' 'lines_counted = 987' \
    'line_coverage = "77.5"' 'commit = "0123456789abcdef0123456789abcdef01234567"' \
    'run = "https://github.com/NormB/vcon-server/actions/runs/1"' >"$work/website/data/metrics.toml"
fi

# The server binds port 0 itself and says which port it took: a port picked
# beforehand and let go could be taken by another process first, and the
# tests would then load that process's pages. It serves the directory by
# name on each request, so the site is built into it once its port, which
# the site's links name, is known.
mkdir "$work/public"
python3 -I -u -m http.server --bind 127.0.0.1 --directory "$work/public" 0 >"$work/server.log" 2>&1 &
server=$!
port=""
for _ in $(seq 1 100); do
  port="$(sed -n 's/^Serving HTTP on 127\.0\.0\.1 port \([0-9][0-9]*\) .*/\1/p' "$work/server.log")"
  [ -n "$port" ] && break
  if ! kill -0 "$server" 2>/dev/null; then
    cat "$work/server.log" >&2
    echo "test-site: the server did not start" >&2
    exit 1
  fi
  sleep 0.1
done
[ -n "$port" ] || { cat "$work/server.log" >&2; echo "test-site: the server never said its port" >&2; exit 1; }
url="http://127.0.0.1:$port"
zola --root "$work/website" build --force --base-url "$url" --output-dir "$work/public" >/dev/null
curl -sf -o /dev/null "$url/" || { echo "test-site: the server does not answer" >&2; exit 1; }

cd "$root/site-tests"
SITE_URL="$url" SITE_TEST_OUTPUT="$work/results" npx playwright test "$@"
