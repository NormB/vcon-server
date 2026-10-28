+++
title = "Add a second tenant"
description = "Make a second tenant over the admin API while the server runs, give it a key, see that its data and acme's stay apart, change its retention period, and remove it."
weight = 14
+++

A tenant is one customer, department or application whose vCons are kept
apart from every other tenant's. Each tenant has its own key, catalog,
files, retention period and audit log. This how-to adds the tenant `globex`
beside `acme`, over the admin API, with the server running.

CI runs every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A running server, set up as in [getting started](@/docs/getting-started.md),
  with the tenant `acme`, and `fullchain.pem` in the current directory.
- One shell for every step: later steps use variables that earlier steps set.

```sh
URL=https://vcon.example.com:8443
CONFIG=/etc/vcon-server/vcon-server.toml
ACME=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
```

The admin routes need an operator key, which only the server's host can
make. Make one for this page:

```sh
OPS=$(sudo -u vcon vcon-server key create --operator --label second-tenant-ops --config $CONFIG | jq -r .token)
```

## Make the tenant

`POST /admin/tenants` takes a name and, if you want one, a retention period.
Without `retention`, the tenant takes the server's `retention_default`:

```sh
curl -sS -i --cacert fullchain.pem -X POST "$URL/admin/tenants" \
  -H "Authorization: Bearer $OPS" -H "Content-Type: application/json" \
  -d '{"name": "globex", "retention": "90d"}'
```

```text
HTTP/2 201
content-type: application/json
content-length: 41
date: <date>

{"id":"<tenant id>"}
```

A name is unique on the server. A second `globex` is refused with
[`name-taken`](@/problems/name-taken.md):

```sh
curl -sS --cacert fullchain.pem -X POST "$URL/admin/tenants" \
  -H "Authorization: Bearer $OPS" -H "Content-Type: application/json" -d '{"name": "globex"}'
```

```text
{"status":409,"title":"Another tenant has the name","type":"https://vcon-server.com/problems/name-taken"}
```

A member the route does not take, such as `title`, is refused with
[`invalid-body`](@/problems/invalid-body.md), so a mistake is never ignored:

```sh
curl -sS --cacert fullchain.pem -X POST "$URL/admin/tenants" \
  -H "Authorization: Bearer $OPS" -H "Content-Type: application/json" -d '{"title": "initech"}'
```

```text
{"status":422,"title":"The body is not one this route takes","type":"https://vcon-server.com/problems/invalid-body"}
```

List the tenants, by name:

```sh
curl -sS --cacert fullchain.pem "$URL/admin/tenants" -H "Authorization: Bearer $OPS" | jq -c '.[] | {name, retention}'
```

```text
{"name":"acme","retention":null}
{"name":"globex","retention":"90d"}
```

`retention` `null` means the server default. Keep globex's id:

```sh
GLOBEX=$(curl -sS --cacert fullchain.pem "$URL/admin/tenants" -H "Authorization: Bearer $OPS" | jq -r '.[] | select(.name == "globex") | .id')
```

## Give it a key

`POST /admin/tenants/{id}/keys` makes a tenant key over HTTP. The answer
carries the token once, with `Cache-Control: no-store`:

```sh
curl -sS -i --cacert fullchain.pem -X POST "$URL/admin/tenants/$GLOBEX/keys" \
  -H "Authorization: Bearer $OPS" -H "Content-Type: application/json" \
  -d '{"scopes": ["write", "read"], "label": "globex-app"}' | tee globex-key.txt | grep -v '"token"'
```

```text
HTTP/2 201
content-type: application/json
cache-control: no-store
content-length: 100
date: <date>
```

The last line of the answer, left out above, is `{"id":"…","token":"vcs_…"}`.
Keep the token, and delete the file that held it:

```sh
GLOBEX_KEY=$(tail -n 1 globex-key.txt | jq -r .token)
rm globex-key.txt
curl -sS --cacert fullchain.pem "$URL/key" -H "Authorization: Bearer $GLOBEX_KEY" | jq -c --arg g "$GLOBEX" '{scopes, tenant: (.tenant == $g)}'
```

```text
{"scopes":["write","read"],"tenant":true}
```

## Its data and acme's stay apart

Make an acme key, and store one vCon with each tenant's key. The same uuid
is stored under each tenant:

```sh
ACME_KEY=$(sudo -u vcon vcon-server key create --tenant "$ACME" --scope write --scope read --label second-tenant-acme \
  --config $CONFIG | jq -r .token)
UUID=$(cat /proc/sys/kernel/random/uuid)
printf '{"vcon":"0.4.0","uuid":"%s","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550103"}],"dialog":[],"attachments":[],"analysis":[]}' \
  "$UUID" > shared-uuid.vcon
for key in "$ACME_KEY" "$GLOBEX_KEY"; do
  curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
    -H "Authorization: Bearer $key" -H "Content-Type: application/vcon" --data-binary @shared-uuid.vcon
done
```

```text
201
201
```

Both answer `201`. A uuid is unique within its tenant, so two tenants can
each hold a vCon with the same uuid, and neither can tell. Each tenant's list
holds only its own vCons:

```sh
curl -sS --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $GLOBEX_KEY" | jq '.vcons | length'
```

```text
1
```

A vCon the globex key cannot see answers `404`, whether it belongs to another
tenant or does not exist:

```sh
OTHER=$(cat /proc/sys/kernel/random/uuid)
printf '{"vcon":"0.4.0","uuid":"%s","created_at":"2026-10-27T12:00:00Z","parties":[],"dialog":[],"attachments":[],"analysis":[]}' \
  "$OTHER" > acme-only.vcon
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $ACME_KEY" -H "Content-Type: application/vcon" --data-binary @acme-only.vcon
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$OTHER" -H "Authorization: Bearer $GLOBEX_KEY"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$OTHER" -H "Authorization: Bearer $ACME_KEY"
```

```text
201
404
200
```

## Change its retention period

A tenant's retention period is how long a deleted vCon is kept, counted from
when it was stored. `PATCH /admin/tenants/{id}` changes it, at the next purge
sweep, for every version the tenant keeps:

```sh
curl -sS --cacert fullchain.pem -X PATCH "$URL/admin/tenants/$GLOBEX" \
  -H "Authorization: Bearer $OPS" -H "Content-Type: application/json" -d '{"retention": "1y"}' | jq -c '{retention}'
```

```text
{"retention":"1y"}
```

`{"retention": null}` returns it to the server default. See
[retention and legal holds](@/docs/retention-and-holds.md).

## Remove it

`DELETE /admin/tenants/{id}` removes a tenant that keeps no version. Globex
keeps one, so the server refuses with
[`holds-vcons`](@/problems/holds-vcons.md):

```sh
curl -sS --cacert fullchain.pem -X DELETE "$URL/admin/tenants/$GLOBEX" -H "Authorization: Bearer $OPS"
```

```text
{"status":409,"title":"The tenant holds vCons; removing them needs force","type":"https://vcon-server.com/problems/holds-vcons"}
```

`?force=true` removes it with its vCons and revokes its keys in the same
step. It refuses a tenant with a vCon under a legal hold, with
[`under-hold`](@/problems/under-hold.md):

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/admin/tenants/$GLOBEX?force=true" -H "Authorization: Bearer $OPS"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $GLOBEX_KEY"
```

```text
204
401
```

The tenant's key is refused at once. Acme's copy of the shared uuid is
untouched:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $ACME_KEY"
```

```text
200
```

## Clean up

Revoke the two keys this page made on the host:

```sh
sudo -u vcon vcon-server key list --config $CONFIG | jq -r 'select(.label | startswith("second-tenant-")) | .id' \
  | while read -r id; do sudo -u vcon vcon-server key revoke "$id" --config $CONFIG; done
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/admin/tenants" -H "Authorization: Bearer $OPS"
```

```text
401
```

## The same with the command line

With the server stopped, `vcon-server tenant create` makes a tenant, as
[getting started](@/docs/getting-started.md#make-the-first-tenant-and-its-keys)
does. With the server running or not, `key create --tenant` makes its keys
and `tenant set-retention` changes its period. `tenant remove` runs only
with the server stopped. See the [command line](@/docs/command-line.md).
