#!/usr/bin/env bash
# Runs every client example in clients/ against a live vcon-server: a new
# data directory, master key, certificate and tenant, a server on a
# loopback port the kernel picks, and one key with the write, read and
# delete scopes. Each client must store a vCon, find it in the list, read
# it back byte for byte, delete it, and print exactly these four lines, one
# uuid in all:
#
#   stored <uuid>
#   listed <uuid>
#   read <uuid> 123 bytes
#   deleted <uuid>
#
# Each must then exit non-zero with a key the server does not know, and
# say "POST /vcons: 401 <problem type>" on standard error.
#
# Usage: scripts/run-clients.sh <vcon-server binary> <rust client binary>
# Needs curl, jq, openssl, python3, go and node (22.18 or later) on PATH,
# and a memory lock limit the server can lock all its memory under.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
server="$(realpath "$1")"
rust_client="$(realpath "$2")"
work="$(mktemp -d)"
pid=""
cleanup() {
  if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT

fail() {
  echo "run-clients: $*" >&2
  exit 1
}

mkdir "$work/data"
head -c 32 /dev/urandom > "$work/master.key"
chmod 0400 "$work/master.key"
# A certificate authority and a server certificate it signs, as a real
# deployment has; the clients trust the authority (VCON_CA). Both carry the
# key usages RFC 5280 asks for, which strict verifiers (Python 3.13 and
# later by default) require.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 \
  -subj "/CN=vcon-server clients test CA" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -keyout "$work/ca.key" -out "$work/ca.pem" 2>/dev/null
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -subj /CN=localhost -keyout "$work/key.pem" -out "$work/server.csr" 2>/dev/null
printf 'subjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n' \
  > "$work/server.ext"
openssl x509 -req -in "$work/server.csr" -CA "$work/ca.pem" -CAkey "$work/ca.key" \
  -days 1 -extfile "$work/server.ext" -out "$work/cert.pem" 2>/dev/null
cat > "$work/vcon-server.toml" <<TOML
listen = "127.0.0.1:0"
data_dir = "$work/data"
master_key = "$work/master.key"

[tls]
cert = "$work/cert.pem"
key = "$work/key.pem"
TOML
config=(--config "$work/vcon-server.toml")
tenant="$("$server" tenant create clients "${config[@]}" | jq -r .id)"
key="$("$server" key create --tenant "$tenant" --scope write --scope read --scope delete \
  --label clients "${config[@]}" | jq -r .token)"

"$server" serve "${config[@]}" 2> "$work/server.log" &
pid=$!
# The server binds port 0, so the kernel picks a free port; its serving line
# names the port it bound, and it listens from that line on.
port=""
for _ in $(seq 100); do
  port="$(sed -n 's/.* serving listen=127\.0\.0\.1:\([0-9][0-9]*\)$/\1/p' "$work/server.log")"
  [ -n "$port" ] && break
  kill -0 "$pid" 2>/dev/null || { cat "$work/server.log" >&2; fail "the server stopped"; }
  sleep 0.1
done
[ -n "$port" ] || fail "the server never said where it listens"
(exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null || fail "the server does not listen on port $port"

export VCON_URL="https://localhost:$port"
export VCON_CA="$work/ca.pem"
# Node reads extra roots only from this variable, at start.
export NODE_EXTRA_CA_CERTS="$VCON_CA"

uuid='[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'
# Each client, as a user runs it, from the repository root.
clients=(
  "curl|bash clients/curl/vcon-client.sh"
  "python|python3 -I clients/python/vcon_client.py"
  "go|go -C clients/go run ./vcon-client"
  "typescript|node clients/typescript/vcon-client.ts"
  "rust|$rust_client"
)
cd "$root"
for entry in "${clients[@]}"; do
  name="${entry%%|*}"
  command="${entry#*|}"
  if ! out="$(VCON_KEY="$key" $command 2> "$work/$name.err")"; then
    cat "$work/$name.err" >&2
    fail "$name: exited non-zero"
  fi
  first="$(printf '%s\n' "$out" | head -n 1)"
  id="${first#stored }"
  [[ "$id" =~ ^$uuid$ ]] || fail "$name: the first line is not 'stored <uuid>': $first"
  pattern="^stored $id
listed $id
read $id 123 bytes
deleted $id$"
  [[ "$out" =~ $pattern ]] || fail "$name: printed something else:
$out"
  if VCON_KEY="vcs_0000000000000000_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" $command \
      > /dev/null 2> "$work/$name.refused"; then
    fail "$name: exited 0 with a key the server does not know"
  fi
  grep -qF "POST /vcons: 401 https://vcon-server.com/problems/unauthorized" "$work/$name.refused" \
    || fail "$name: a refusal did not name its route, status and problem type: $(cat "$work/$name.refused")"
  echo "run-clients: $name: ok"
done
