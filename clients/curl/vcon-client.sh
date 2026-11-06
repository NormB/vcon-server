#!/usr/bin/env bash
# Works through the HTTP API with curl and jq: stores a vCon, finds it in
# the list, reads it back, stores and reads back a gzipped vCon, pages
# through both with a cursor, replaces the first under If-Match, places and
# releases a legal hold, finds its own create in the audit log, lists the
# tenants with an operator key, reads the vCon over MCP, and deletes both.
# python3 gzips the second vCon.
#
# Set VCON_URL (https://vcon.example.com) and four keys: VCON_KEY (the
# write, read and delete scopes), VCON_LEGAL_KEY (legal), VCON_AUDIT_KEY
# (audit) and VCON_OPERATOR_KEY (an operator key). For a certificate your
# system does not trust, set VCON_CA (its PEM file).
set -euo pipefail

: "${VCON_URL:?set VCON_URL}" "${VCON_KEY:?set VCON_KEY}"
: "${VCON_LEGAL_KEY:?set VCON_LEGAL_KEY}" "${VCON_AUDIT_KEY:?set VCON_AUDIT_KEY}"
: "${VCON_OPERATOR_KEY:?set VCON_OPERATOR_KEY}"
ca=()
[ -n "${VCON_CA:-}" ] && ca=(--cacert "$VCON_CA")
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# api METHOD PATH [curl options]: sends one request with VCON_KEY (or the
# key in $as) and prints the answer's body; the answer's headers are in
# $tmp/headers. An answer of 400 or more is a problem document: it stops
# the program with its type.
api() {
  local method="$1" path="$2" answer status body
  shift 2
  answer="$(curl --silent --show-error "${ca[@]}" -X "$method" -w '\n%{http_code}' \
    -D "$tmp/headers" -H "Authorization: Bearer ${as:-$VCON_KEY}" "$@" "$VCON_URL$path")"
  status="${answer##*$'\n'}"
  body="${answer%$'\n'*}"
  if [ "$status" -ge 400 ]; then
    echo "$method $path: $status $(jq -r .type <<< "$body")" >&2
    exit 1
  fi
  printf '%s' "$body"
}

# header NAME: the value of that header in the last answer.
header() {
  local name value
  while IFS=': ' read -r name value; do
    if [ "${name,,}" = "$1" ]; then printf '%s\n' "${value%$'\r'}"; fi
  done < "$tmp/headers"
}

# mcp MESSAGE: posts one JSON-RPC message to /mcp and prints the answer.
mcp() {
  api POST /mcp -H "Content-Type: application/json" \
    -H "Accept: application/json, text/event-stream" --data-binary "$1"
}

expect() {
  [ "$1" = "$2" ] || { echo "$3" >&2; exit 1; }
}

uuid="$(cat /proc/sys/kernel/random/uuid)"
vcon="$(jq -cn --arg uuid "$uuid" \
  '{created_at: "2026-10-25T12:00:00Z", parties: [], subject: "client example", uuid: $uuid}')"

# Store: 201, with the new version in ETag.
api POST /vcons -H "Content-Type: application/vcon" --data-binary "$vcon" > /dev/null
first_tag="$(header etag)"
echo "stored $uuid"

# List: the newest vCon comes first.
page="$(api GET "/vcons?limit=10")"
stored_at="$(jq -r --arg uuid "$uuid" '.vcons[] | select(.uuid == $uuid) | .stored_at' <<< "$page")"
[ -n "$stored_at" ] || { echo "the stored vCon is not in the list" >&2; exit 1; }
echo "listed $uuid"

# Read: the body comes back exactly as stored.
body="$(api GET "/vcons/$uuid")"
expect "$body" "$vcon" "the body read differs from the one stored"
echo "read $uuid ${#body} bytes"

# Gzip: a gzipped body is stored as sent, and read back with its media type.
gzip_uuid="$(cat /proc/sys/kernel/random/uuid)"
jq -cnj --arg uuid "$gzip_uuid" \
  '{created_at: "2026-10-25T12:00:00Z", parties: [], subject: "gzip example", uuid: $uuid}' \
  | python3 -I -c 'import gzip, sys; sys.stdout.buffer.write(gzip.compress(sys.stdin.buffer.read(), mtime=0))' \
  > "$tmp/sent.gz"
api POST /vcons -H "Content-Type: application/vcon+gzip" --data-binary "@$tmp/sent.gz" > /dev/null
api GET "/vcons/$gzip_uuid" -o "$tmp/read.gz" > /dev/null
expect "$(base64 -w 0 "$tmp/read.gz")" "$(base64 -w 0 "$tmp/sent.gz")" \
  "the gzipped body read differs from the one stored"
expect "$(header content-type)" application/vcon+gzip "a gzipped vCon was not read back as application/vcon+gzip"
echo "gzip $gzip_uuid read back as application/vcon+gzip"

# Page: one vCon a page, from the first one's stored_at on, following next
# until it is null. Each of the two must be seen exactly once.
since="$(jq -rn --arg t "$stored_at" '$t | @uri')"
seen=() pages=0 cursor=""
while :; do
  page="$(api GET "/vcons?limit=1&since=$since${cursor:+&cursor=$cursor}")"
  pages=$((pages + 1))
  mapfile -t -O "${#seen[@]}" seen < <(jq -r '.vcons[].uuid' <<< "$page")
  cursor="$(jq -r '.next // empty' <<< "$page")"
  [ -n "$cursor" ] || break
done
for each in "$uuid" "$gzip_uuid"; do
  expect "$(printf '%s\n' "${seen[@]}" | grep -cx "$each")" 1 "the pages did not list $each exactly once"
done
echo "paged ${#seen[@]} vCons in $pages pages of 1"

# Replace: If-Match names the current version; the answer has the new one.
# The first version's tag is then stale, and refused with 412.
replaced="$(jq -c '.subject = "client example, replaced"' <<< "$vcon")"
api PUT "/vcons/$uuid" -H "If-Match: $first_tag" -H "Content-Type: application/vcon" \
  --data-binary "$replaced" > /dev/null
tag="$(header etag)"
[ -n "$tag" ] && [ "$tag" != "$first_tag" ] || { echo "a replace did not answer a new ETag" >&2; exit 1; }
status="$(curl --silent "${ca[@]}" -X PUT -o "$tmp/stale" -w '%{http_code}' \
  -H "Authorization: Bearer $VCON_KEY" -H "If-Match: $first_tag" -H "Content-Type: application/vcon" \
  --data-binary "$replaced" "$VCON_URL/vcons/$uuid")"
expect "$status $(jq -r .type "$tmp/stale")" "412 https://vcon-server.com/problems/precondition-failed" \
  "a stale If-Match answered $status, not 412 precondition-failed"
echo "replaced $uuid, a stale If-Match refused with 412"

# Legal hold: placed and released with the legal key, 204 each.
as="$VCON_LEGAL_KEY" api PUT "/legal/vcons/$uuid/hold" > /dev/null
as="$VCON_LEGAL_KEY" api DELETE "/legal/vcons/$uuid/hold" > /dev/null
echo "held $uuid and released"

# Audit: the tenant's log, read with the audit key, holds this program's
# create, made by VCON_KEY, whose id and tenant GET /key gives.
me="$(api GET /key)"
key_id="$(jq -r .id <<< "$me")"
after=0 found=""
while [ -z "$found" ]; do
  as="$VCON_AUDIT_KEY" api GET "/audit?after=$after&limit=1000" > "$tmp/audit"
  found="$(jq -r --arg uuid "$uuid" --arg key "$key_id" \
    'select(.action == "create" and .target == $uuid and .actor == $key and .status == 201) | .seq' \
    "$tmp/audit")"
  [ -n "$found" ] && break
  [ "$(wc -l < "$tmp/audit")" -ge 1000 ] || { echo "the audit log has no create of $uuid" >&2; exit 1; }
  after="$(tail -n 1 "$tmp/audit" | jq .seq)"
done
echo "audit create $uuid by $key_id"

# Admin: the operator key lists every tenant, this key's among them.
tenant="$(jq -r .tenant <<< "$me")"
tenants="$(as="$VCON_OPERATOR_KEY" api GET /admin/tenants)"
jq -e --arg id "$tenant" 'any(.[]; .id == $id)' <<< "$tenants" > /dev/null \
  || { echo "the admin tenant list does not have $tenant" >&2; exit 1; }
echo "admin tenant $tenant listed"

# MCP: initialize, tools/list, and vcon_get, which answers the replaced
# vCon's text between the untrusted-data markers, and its version.
init="$(mcp '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"vcon-client","version":"1"}}}')"
server="$(jq -r .result.serverInfo.name <<< "$init")"
expect "$(jq -r .result.protocolVersion <<< "$init")" 2025-06-18 "initialize did not agree to 2025-06-18"
mcp '{"jsonrpc":"2.0","method":"notifications/initialized"}' > /dev/null
tools="$(mcp '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | jq -r '.result.tools[].name')"
grep -qx vcon_get <<< "$tools" || { echo "tools/list has no vcon_get" >&2; exit 1; }
got="$(mcp "$(jq -cn --arg uuid "$uuid" \
  '{jsonrpc: "2.0", id: 3, method: "tools/call", params: {name: "vcon_get", arguments: {uuid: $uuid}}}')")"
expect "$(jq -r .result.isError <<< "$got")" false "vcon_get answered an error: $got"
expect "\"$(jq -r .result.structuredContent.version <<< "$got")\"" "$tag" "vcon_get answered another version"
expect "$(jq -r .result.structuredContent.vcon <<< "$got")" "⟦untrusted-vcon-data⟧$replaced⟦/untrusted-vcon-data⟧" \
  "vcon_get answered another text"
echo "mcp vcon_get $uuid from $server, $(wc -l <<< "$tools") tools listed"

# Delete: 204, and each vCon is gone (410) from then on.
for each in "$uuid" "$gzip_uuid"; do
  api DELETE "/vcons/$each" > /dev/null
  status="$(curl --silent "${ca[@]}" -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $VCON_KEY" "$VCON_URL/vcons/$each")"
  expect "$status" 410 "a deleted vCon answered $status, not 410"
done
echo "deleted $uuid $gzip_uuid"
