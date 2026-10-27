#!/usr/bin/env bash
# Runs the website's browser tests: builds a copy of website/ in a temporary
# directory, serves it on a free local port, runs the Playwright tests in
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

port="$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
url="http://127.0.0.1:$port"
zola --root "$work/website" build --base-url "$url" --output-dir "$work/public" >/dev/null

python3 -I -m http.server --bind 127.0.0.1 --directory "$work/public" "$port" 2>"$work/server.log" &
server=$!
for _ in $(seq 1 100); do
  if curl -sf -o /dev/null "$url/"; then break; fi
  if ! kill -0 "$server" 2>/dev/null; then
    cat "$work/server.log" >&2
    echo "test-site: the server did not start" >&2
    exit 1
  fi
  sleep 0.1
done
curl -sf -o /dev/null "$url/" || { echo "test-site: the server does not answer" >&2; exit 1; }

cd "$root/site-tests"
SITE_URL="$url" SITE_TEST_OUTPUT="$work/results" npx playwright test "$@"
