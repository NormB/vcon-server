#!/usr/bin/env bash
# Runs the console's browser tests against the real server: builds the
# binary, makes a data directory, master key, certificate and operator key
# in a temporary directory, starts `vcon-server serve` on a port the kernel
# picks, runs the Playwright tests in console-tests/ against it, then stops
# it and removes everything it made. Needs Node and the Playwright browser:
#   npm ci --prefix console-tests
#   npx --prefix console-tests playwright install --only-shell chromium
# Any arguments are passed to `playwright test`.
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

cargo build --manifest-path "$root/Cargo.toml" -p vcon-server --locked --quiet
bin="${CARGO_TARGET_DIR:-$root/target}/debug/vcon-server"

mkdir -m 0700 "$work/data"
head -c 32 /dev/urandom >"$work/master.key"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 \
  -subj /CN=localhost -addext subjectAltName=DNS:localhost \
  -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
cat >"$work/vcon-server.toml" <<TOML
listen = "127.0.0.1:0"
data_dir = "$work/data"
master_key = "$work/master.key"

[tls]
cert = "$work/cert.pem"
key = "$work/key.pem"
TOML

operator="$("$bin" key create --operator --label console-tests --config "$work/vcon-server.toml" |
  python3 -I -c 'import json, sys; print(json.load(sys.stdin)["token"])')"

"$bin" serve --config "$work/vcon-server.toml" 2>"$work/server.log" &
server=$!
# The server binds port 0, so the kernel picks a free port; its serving line
# names the port it bound, and it listens from that line on.
port=""
for _ in $(seq 1 100); do
  port="$(sed -n 's/.* serving listen=127\.0\.0\.1:\([0-9][0-9]*\)$/\1/p' "$work/server.log")"
  [ -n "$port" ] && break
  if ! kill -0 "$server" 2>/dev/null; then
    cat "$work/server.log" >&2
    echo "test-console: the server did not start" >&2
    exit 1
  fi
  sleep 0.1
done
[ -n "$port" ] || { echo "test-console: the server never said where it listens" >&2; exit 1; }
url="https://localhost:$port"
curl -ksf -o /dev/null "$url/console/" || { echo "test-console: the server does not answer" >&2; exit 1; }

cd "$root/console-tests"
VCON_URL="$url" VCON_OPERATOR_KEY="$operator" VCON_TEST_OUTPUT="$work/results" npx playwright test "$@"
