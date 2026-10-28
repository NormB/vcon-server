+++
title = "Use keys and scopes"
description = "Make a key for each scope, see what each one may do and what it is refused, and make a key with more than one scope. Every request and its answer is shown."
weight = 13
+++

A key holds scopes, and each route needs one scope. This how-to makes a key
for each of the five tenant scopes and an operator key. It uses each key on
the route its scope allows and on one route its scope does not allow.
[Tenants and keys](@/docs/tenants-and-keys.md) lists every scope and route.

CI runs every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A running server, set up as in [getting started](@/docs/getting-started.md),
  with the tenant `acme`, and `fullchain.pem` in the current directory.
- One shell for every step: later steps use variables that earlier steps set.

```sh
URL=https://vcon.example.com:8443
CONFIG=/etc/vcon-server/vcon-server.toml
TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
```

## Make a key for each scope

A tenant key belongs to one tenant and holds any of `write`, `read`,
`delete`, `legal` and `audit`. Make one key for each, labelled with its
scope. `key create` runs beside the server:

```sh
for scope in write read delete legal audit; do
  token=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope $scope --label "scopes-$scope" \
    --config $CONFIG | jq -r .token)
  declare "KEY_${scope^^}=$token"
done
```

That sets `KEY_WRITE`, `KEY_READ`, `KEY_DELETE`, `KEY_LEGAL` and `KEY_AUDIT`.
An operator key belongs to no tenant and holds only `admin`. It is made on
the server's host only, never over HTTP:

```sh
KEY_ADMIN=$(sudo -u vcon vcon-server key create --operator --label scopes-admin --config $CONFIG | jq -r .token)
```

## Ask a key what it holds

`GET /key` answers any valid key with its own id, tenant and scopes:

```sh
curl -sS --cacert fullchain.pem "$URL/key" -H "Authorization: Bearer $KEY_LEGAL"
```

```text
{"id":"<key id>","scopes":["legal"],"tenant":"<tenant id>"}
```

An operator key's `tenant` is `null`:

```sh
curl -sS --cacert fullchain.pem "$URL/key" -H "Authorization: Bearer $KEY_ADMIN"
```

```text
{"id":"<key id>","scopes":["admin"],"tenant":null}
```

## write: store and replace

The write key stores a vCon:

```sh
UUID=$(cat /proc/sys/kernel/random/uuid)
printf '{"vcon":"0.4.0","uuid":"%s","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550102"}],"dialog":[],"attachments":[],"analysis":[]}' \
  "$UUID" > scopes.vcon
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $KEY_WRITE" -H "Content-Type: application/vcon" --data-binary @scopes.vcon
```

```text
201
```

It cannot read one back:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY_WRITE"
```

```text
403
```

## read: read and list

The read key reads the vCon and lists the tenant's vCons:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY_READ"
curl -sS --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $KEY_READ" | jq -r --arg u "$UUID" '.vcons[] | select(.uuid == $u) | .uuid'
```

```text
200
<uuid>
```

It cannot store one:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $KEY_READ" -H "Content-Type: application/vcon" --data-binary @scopes.vcon
```

```text
403
```

## delete: delete

The read key cannot delete the vCon. The delete key can:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY_READ"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY_DELETE"
```

```text
403
204
```

The delete key cannot read what it deleted, and neither can the read key:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY_DELETE"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY_READ"
```

```text
403
410
```

## legal: read what was deleted, hold it, restore it

The `legal` scope reaches the `/legal/` routes. They read a deleted vCon,
list its versions, hold it, and restore it. The legal key reads the deleted
vCon. `vcon-state` says it is deleted:

```sh
curl -sS -i --cacert fullchain.pem "$URL/legal/vcons/$UUID" -H "Authorization: Bearer $KEY_LEGAL" | grep -i -e '^HTTP' -e '^vcon-state'
```

```text
HTTP/2 200
vcon-state: deleted
```

It lists the vCon's kept versions, oldest first:

```sh
curl -sS --cacert fullchain.pem "$URL/legal/vcons/$UUID/versions" -H "Authorization: Bearer $KEY_LEGAL" | jq -c '.[] | {current, hidden: (.hidden_at != null)}'
```

```text
{"current":false,"hidden":true}
```

A legal hold keeps every version of the uuid from being purged, whatever its
retention period. Hold it, restore it, and release the hold:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X PUT "$URL/legal/vcons/$UUID/hold" -H "Authorization: Bearer $KEY_LEGAL"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X POST "$URL/legal/vcons/$UUID/restore" -H "Authorization: Bearer $KEY_LEGAL"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/legal/vcons/$UUID/hold" -H "Authorization: Bearer $KEY_LEGAL"
```

```text
204
200
204
```

The restored vCon is live again for the read key:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY_READ"
```

```text
200
```

The read key cannot use the legal routes:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/legal/vcons/$UUID" -H "Authorization: Bearer $KEY_READ"
```

```text
403
```

[Retention and legal holds](@/docs/retention-and-holds.md) describes when a
deleted vCon is purged and what a hold stops.

## audit: the tenant's audit log

The audit key reads the tenant's audit log, one JSON record a line, oldest
first. Each request above is in it, refusals included. The hold, the
restore and the release are recorded as `hold`, `restore` and `release`:

```sh
curl -sS --cacert fullchain.pem "$URL/audit?limit=10000" -H "Authorization: Bearer $KEY_AUDIT" \
  | jq -c 'select(.action == "hold" or .action == "restore" or .action == "release") | {action, status}'
```

```text
{"action":"hold","status":204}
{"action":"restore","status":200}
{"action":"release","status":204}
```

A key without `audit` is refused:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/audit" -H "Authorization: Bearer $KEY_LEGAL"
```

```text
403
```

[Audit logs](@/docs/audit-logs.md) describes every field of a record.

## admin: tenants and keys, never a vCon

The operator key lists the tenants:

```sh
curl -sS --cacert fullchain.pem "$URL/admin/tenants" -H "Authorization: Bearer $KEY_ADMIN" | jq -r '.[].name'
```

```text
acme
```

It cannot read a vCon, and a tenant key cannot use the admin routes:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $KEY_ADMIN"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/admin/tenants" -H "Authorization: Bearer $KEY_AUDIT"
```

```text
403
403
```

## A key with more than one scope

A producer that stores vCons and reads them back needs `write` and `read`.
Give `--scope` once for each:

```sh
KEY_RW=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope write --scope read --label scopes-rw \
  --config $CONFIG | jq -r .token)
curl -sS --cacert fullchain.pem "$URL/key" -H "Authorization: Bearer $KEY_RW" | jq -c .scopes
```

```text
["write","read"]
```

Give each producer, reader and process its own key, with only the scopes it
needs and a label that names who holds it. A request whose key lacks the
scope is refused with [`forbidden`](@/problems/forbidden.md), and the audit
log records the refusal.

## Clean up

Revoke the keys this page made. `key revoke` runs beside the server:

```sh
sudo -u vcon vcon-server key list --config $CONFIG | jq -r 'select(.label | startswith("scopes-")) | .id' \
  | while read -r id; do sudo -u vcon vcon-server key revoke "$id" --config $CONFIG; done
sudo -u vcon vcon-server key list --config $CONFIG | jq -r 'select(.label | startswith("scopes-")) | select(.revoked_at == null) | .label'
```

The second command prints nothing: no key from this page is left unrevoked.
