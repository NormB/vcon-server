#!/usr/bin/env bash
# Runs the scale harness against a fresh server: makes a data directory, a
# master key, a CA and a server certificate, a tenant and a key, starts
# `serve`, runs `vcon-scale run`, and deletes every file it made, the stored
# vCons included, whatever the outcome.
#
#   scripts/scale-run.sh SERVER SCALE COUNT OUT [PARENT] [-- vcon-scale flags]
#
# SERVER and SCALE are the two binaries, COUNT the vCons to store, OUT the
# result file. The work directory is made under PARENT (default: the
# system's temporary directory); on a shared host, name a directory on the
# disk the numbers are meant for. SCALE_LABEL names the run (default
# "run"); SCALE_DISK states the disk (default: read with lsblk). Flags after
# `--` go to vcon-scale as they are (--concurrency, --reads, --seed,
# --mix, --max-load, --list-pages). See website/content/docs/scale.md.
#
# SCALE_WRITERS=N also runs N writers (scripts/contend_writes.py) for the
# whole of vcon-scale's run: each stores a vCon of its own in the same
# tenant and replaces it until vcon-scale ends, so reads are measured while
# the tenant is written. Their summary goes to OUT with `.json` replaced by
# `-writes.json`; a writer that failed fails the run. SCALE_CATALOG=FILE
# adds FILE's text to the end of the server's configuration (for example
# a `[catalog]` and a `[storage]` section for PostgreSQL and S3, whose
# database the caller has migrated).
set -euo pipefail

[ "$#" -ge 4 ] || { echo "usage: $0 SERVER SCALE COUNT OUT [PARENT] [-- flags]" >&2; exit 2; }
writers="${SCALE_WRITERS:-0}"
case "$writers" in
  '' | *[!0-9]*) echo "scale-run: SCALE_WRITERS must be a number of writers, not \"$writers\"" >&2; exit 2 ;;
esac
catalog="${SCALE_CATALOG:-}"
if [ -n "$catalog" ] && [ ! -r "$catalog" ]; then
  echo "scale-run: SCALE_CATALOG names $catalog, which cannot be read" >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/.." && pwd)"
server="$(realpath "$1")"
scale="$(realpath "$2")"
count="$3"
out="$(realpath -m "$4")"
shift 4
parent="${TMPDIR:-/tmp}"
if [ "$#" -gt 0 ] && [ "$1" != -- ]; then parent="$1"; shift; fi
if [ "$#" -gt 0 ] && [ "$1" = -- ]; then shift; fi
work="$(mktemp -d -p "$parent" vcon-scale.XXXXXX)"
pid=""
wpid=""
cleanup() {
  if [ -n "$wpid" ]; then kill "$wpid" 2>/dev/null || true; wait "$wpid" 2>/dev/null || true; fi
  if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT
# A signal ends the script through exit, so the EXIT trap deletes the data.
trap 'exit 143' TERM
trap 'exit 130' INT

fail() {
  echo "scale-run: $*" >&2
  exit 1
}

mkdir "$work/data"
head -c 32 /dev/urandom > "$work/master.key"
chmod 0400 "$work/master.key"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 1 \
  -subj "/CN=vcon-server scale test CA" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
  -keyout "$work/ca.key" -out "$work/ca.pem" 2>/dev/null
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -subj /CN=localhost -keyout "$work/key.pem" -out "$work/server.csr" 2>/dev/null
chmod 0600 "$work/key.pem"
printf 'subjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n' \
  > "$work/server.ext"
openssl x509 -req -in "$work/server.csr" -CA "$work/ca.pem" -CAkey "$work/ca.key" \
  -days 1 -extfile "$work/server.ext" -out "$work/cert.pem" 2>/dev/null
printf 'listen = "127.0.0.1:0"\ndata_dir = "%s"\nmaster_key = "%s"\n\n[tls]\ncert = "%s"\nkey = "%s"\n' \
  "$work/data" "$work/master.key" "$work/cert.pem" "$work/key.pem" > "$work/vcon-server.toml"
if [ -n "$catalog" ]; then
  { echo; cat "$catalog"; } >> "$work/vcon-server.toml"
fi
config=(--config "$work/vcon-server.toml")
tenant="$("$server" tenant create scale "${config[@]}" | jq -r .id)"
( umask 077; "$server" key create --tenant "$tenant" --scope write --scope read --label scale \
  "${config[@]}" | jq -r .token > "$work/api.key" )

# The log exists before the server starts: the background child creates
# its redirection target only when it runs, and a first read of the log
# before that failed the script under `set -e` (seen once in 40 runs of
# scripts/test_contend_writes.py at load 17).
: > "$work/server.log"
"$server" serve "${config[@]}" 2>> "$work/server.log" &
pid=$!
port=""
for _ in $(seq 100); do
  port="$(sed -n 's/.* serving listen=127\.0\.0\.1:\([0-9][0-9]*\)$/\1/p' "$work/server.log")"
  [ -n "$port" ] && break
  kill -0 "$pid" 2>/dev/null || { cat "$work/server.log" >&2; fail "the server stopped"; }
  sleep 0.1
done
[ -n "$port" ] || fail "the server never said where it listens"

disk="${SCALE_DISK:-}"
if [ -z "$disk" ]; then
  source="$(findmnt -no SOURCE --target "$work" 2>/dev/null || true)"
  parent_dev="$(lsblk -no PKNAME "$source" 2>/dev/null | head -n 1 || true)"
  disk="$(lsblk -dno MODEL,TRAN "/dev/${parent_dev:-none}" 2>/dev/null | xargs || true)"
  disk="${disk:-unknown disk}"
fi
commit="$(git -C "$root" rev-parse HEAD 2>/dev/null || echo unknown)"

if [ "$writers" -gt 0 ]; then
  python3 -I "$root/scripts/contend_writes.py" --port "$port" --ca "$work/ca.pem" \
    --key-file "$work/api.key" --stop "$work/stop-writers" --out "${out%.json}-writes.json" \
    --writers "$writers" &
  wpid=$!
fi

status=0
"$scale" run --url "https://localhost:$port" --ca "$work/ca.pem" --key-file "$work/api.key" \
  --count "$count" --out "$out" --data-dir "$work/data" --server-pid "$pid" \
  --commit "$commit" --disk "$disk" --label "${SCALE_LABEL:-run}" "$@" || status=$?
if [ "$status" -ne 0 ]; then
  echo "scale-run: vcon-scale exited $status; the server's last lines:" >&2
  tail -n 20 "$work/server.log" >&2
fi
if [ -n "$wpid" ]; then
  touch "$work/stop-writers"
  wstatus=0
  wait "$wpid" || wstatus=$?
  wpid=""
  if [ "$wstatus" -ne 0 ]; then
    echo "scale-run: the writers exited $wstatus" >&2
    [ "$status" -ne 0 ] || status="$wstatus"
  fi
fi
exit "$status"
