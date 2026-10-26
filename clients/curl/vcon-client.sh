#!/usr/bin/env bash
# Stores a vCon, finds it in the list, reads it back, and deletes it, with
# curl and jq. Set VCON_URL (https://vcon.example.com), VCON_KEY (a key with
# the write, read and delete scopes) and, for a certificate your system does
# not trust, VCON_CA (its PEM file).
set -euo pipefail

: "${VCON_URL:?set VCON_URL}" "${VCON_KEY:?set VCON_KEY}"
ca=()
[ -n "${VCON_CA:-}" ] && ca=(--cacert "$VCON_CA")

# api METHOD PATH [curl options]: prints the answer's body. An answer of 400
# or more is a problem document: it stops the program with its type.
api() {
  local method="$1" path="$2" answer status body
  shift 2
  answer="$(curl --silent --show-error "${ca[@]}" -X "$method" -w '\n%{http_code}' \
    -H "Authorization: Bearer $VCON_KEY" "$@" "$VCON_URL$path")"
  status="${answer##*$'\n'}"
  body="${answer%$'\n'*}"
  if [ "$status" -ge 400 ]; then
    echo "$method $path: $status $(jq -r .type <<< "$body")" >&2
    exit 1
  fi
  printf '%s' "$body"
}

uuid="$(cat /proc/sys/kernel/random/uuid)"
vcon="$(jq -cn --arg uuid "$uuid" \
  '{created_at: "2026-10-25T12:00:00Z", parties: [], subject: "client example", uuid: $uuid}')"

# Store: 201, with the new version in ETag.
api POST /vcons -H "Content-Type: application/vcon" --data-binary "$vcon" > /dev/null
echo "stored $uuid"

# List: the newest vCon comes first.
page="$(api GET "/vcons?limit=10")"
jq -e --arg uuid "$uuid" 'any(.vcons[]; .uuid == $uuid)' <<< "$page" > /dev/null \
  || { echo "the stored vCon is not in the list" >&2; exit 1; }
echo "listed $uuid"

# Read: the body comes back exactly as stored.
body="$(api GET "/vcons/$uuid")"
[ "$body" = "$vcon" ] || { echo "the body read differs from the one stored" >&2; exit 1; }
echo "read $uuid ${#body} bytes"

# Delete: 204, and the vCon is gone (410) from then on.
api DELETE "/vcons/$uuid" > /dev/null
status="$(curl --silent "${ca[@]}" -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer $VCON_KEY" "$VCON_URL/vcons/$uuid")"
[ "$status" = 410 ] || { echo "a deleted vCon answered $status, not 410" >&2; exit 1; }
echo "deleted $uuid"
