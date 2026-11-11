+++
title = "Administer a tenant with a key"
description = "Give a tenant's administrator a key, then read and change the tenant's settings, issue and revoke the tenant's keys within its role cap, list its memberships, and find each call in the tenant's audit log."
weight = 38

[extra]
group = "how-to"
+++

A tenant's administrator looks after one tenant without the operator's
key: it reads and changes the tenant's settings, issues and revokes the
tenant's own keys, and lists the tenant's memberships. Each of those is a
route under `/tenant`, and each needs a key of the tenant that holds the
scope `tenant_admin`. Only the operator issues such a key. This how-to
gives a tenant an administrator's key and uses every route with it.

A route under `/tenant` takes no tenant's id: it works in the tenant of the
key that asks, and nowhere else. The operator's key is refused on all of
them.

This is the third part of sign-in. The settings are stored and answered
now, and nothing reads them yet: sessions, which a later release adds, will
read the two session limits, and the console will read the masking
settings. Until then, set them to decide them ahead of time.

The routes are on `main` only; release 0.4.3 does not have them. CI runs
every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A server built from `main`, set up as in
  [getting started](@/docs/getting-started.md), with the tenant `acme`,
  running and reachable at its address, here
  `https://vcon.example.com:8443`.
- A shell on the server's host, as an account that may run commands as the
  server's account (`sudo -u vcon`): the operator makes the
  administrator's key, the role cap and the example user there.
- `curl` and `jq` on that machine. Nothing changes on any other system.
- One shell for every step: later steps use variables that earlier steps set.

## Give the tenant an administrator's key

The operator makes a key of the tenant with the scope `tenant_admin`. Keep
its id: the audit log names a key by it.

```sh
CONFIG=/etc/vcon-server/vcon-server.toml
URL=https://vcon.example.com:8443
CA=/usr/local/share/vcon-server/ca.pem
TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
CREATED=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope tenant_admin --label tenant-administrator --config $CONFIG)
ADMIN=$(echo "$CREATED" | jq -r .token)
ADMIN_ID=$(echo "$CREATED" | jq -r .id)
unset CREATED
```

Hand the token to the tenant's administrator over a channel you trust. The
key reaches the routes under `/tenant` and the routes any key reaches
(`GET /key`, `GET /health`); it reads no vCon and no audit log unless it
holds those scopes too.

## Read the tenant's settings

`GET /tenant/settings` answers every setting, each name with what was set
or, as here for a tenant that set nothing, its default:

```sh
curl -sS --cacert $CA "$URL/tenant/settings" -H "Authorization: Bearer $ADMIN" | jq -cS .
```

```text
{"masking.auditor":true,"masking.legal":true,"masking.producer":true,"masking.reader":true,"masking.records":true,"masking.tenant-admin":true,"session.absolute_seconds":28800,"session.idle_seconds":900}
```

| Setting | Value | While unset | Meaning |
|---|---|---|---|
| `masking.reader`, `masking.producer`, `masking.records`, `masking.legal`, `masking.auditor`, `masking.tenant-admin` | `true` or `false` | `true` | whether the console masks personal data for a user who holds the role |
| `session.idle_seconds` | 60 to 900 | 900 | how long a session lasts without a request |
| `session.absolute_seconds` | 300 to 28800 | 28800 | how long a session lasts at most |

`session.idle_seconds` is never above `session.absolute_seconds`.

## Change settings

`PUT /tenant/settings` takes a JSON object of the names to change. A name
left out is not changed. The answer is every setting after the change:

```sh
curl -sS --cacert $CA -X PUT "$URL/tenant/settings" -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"masking.legal":false,"session.idle_seconds":300}' \
  | jq -c '[."masking.legal", ."session.idle_seconds", ."masking.reader"]'
```

```text
[false,300,true]
```

A change with a name outside the list, a value of the wrong kind or a limit
outside its bounds is refused whole with `422` and the problem
[`bad-setting`](@/problems/bad-setting.md), and stores nothing:

```sh
curl -sS --cacert $CA -X PUT "$URL/tenant/settings" -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"masking.reader":false,"session.idle_seconds":30}' \
  | jq -r '"\(.status) \(.type)"'
```

```text
422 https://vcon-server.com/problems/bad-setting
```

`null` puts a name back to its default:

```sh
curl -sS --cacert $CA -X PUT "$URL/tenant/settings" -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"session.idle_seconds":null}' \
  | jq -c '[."masking.legal", ."session.idle_seconds", ."masking.reader"]'
```

```text
[false,900,true]
```

## Issue a key for the tenant

`POST /tenant/keys` makes a key of the administrator's tenant. The body
names the scopes and a label; the answer carries the token once.

```sh
CREATED=$(curl -sS --cacert $CA "$URL/tenant/keys" -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"scopes":["read"],"label":"reports"}')
REPORTS=$(echo "$CREATED" | jq -r .token)
REPORTS_ID=$(echo "$CREATED" | jq -r .id)
unset CREATED
curl -sS --cacert $CA "$URL/key" -H "Authorization: Bearer $REPORTS" | jq -c '{scopes}'
```

```text
{"scopes":["read"]}
```

An administrator gives a key one or more of `write`, `read`, `delete`,
`legal` and `audit`. It cannot make another administrator's key: only the
operator gives `tenant_admin`. The refusal is `422` with the problem
[`bad-scopes`](@/problems/bad-scopes.md):

```sh
curl -sS --cacert $CA "$URL/tenant/keys" -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"scopes":["tenant_admin"],"label":"second-administrator"}' \
  | jq -r '"\(.status) \(.type)"'
```

```text
422 https://vcon-server.com/problems/bad-scopes
```

## Keys stay within the tenant's role cap

The operator may set a [role cap](@/docs/roles-and-scopes.md) on the
tenant. The administrator then gives a key only the scopes of the roles the
cap allows. With a cap of `reader` and `tenant-admin`, a key that writes is
refused:

```sh
sudo -u vcon vcon-server tenant set-role-cap $TENANT --role reader --role tenant-admin --config $CONFIG | jq -c .role_cap
curl -sS --cacert $CA "$URL/tenant/keys" -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"scopes":["write"],"label":"producer"}' \
  | jq -r '"\(.status) \(.type)"'
```

```text
["reader","tenant-admin"]
422 https://vcon-server.com/problems/bad-scopes
```

The cap bounds the keys the administrator makes from then on. A key made
before the cap keeps its scopes until someone revokes it, and the cap does
not bound the operator's own `key create`. Remove the cap again for the
steps that follow:

```sh
sudo -u vcon vcon-server tenant set-role-cap $TENANT --all --config $CONFIG | jq -c .role_cap
```

```text
null
```

## List and revoke the tenant's keys

`GET /tenant/keys` lists the tenant's keys, oldest first, with their ids,
scopes, labels and times, and never a token:

```sh
curl -sS --cacert $CA "$URL/tenant/keys" -H "Authorization: Bearer $ADMIN" \
  | jq -r --arg a "$ADMIN_ID" --arg r "$REPORTS_ID" '.[] | select(.id == $a or .id == $r) | "\(.label) \(.scopes | join(","))"'
```

```text
tenant-administrator tenant_admin
reports read
```

`DELETE /tenant/keys/{id}` revokes a key of the tenant, and the key's next
request is refused:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA -X DELETE "$URL/tenant/keys/$REPORTS_ID" -H "Authorization: Bearer $ADMIN"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/key" -H "Authorization: Bearer $REPORTS"
```

```text
204
401
```

The id of a key of another tenant, or of an operator key, is answered `404`
[`unknown-key`](@/problems/unknown-key.md), as an id no key has, and
nothing is revoked. An administrator may revoke any key of its tenant, its
own included.

## List the tenant's memberships

A [membership](@/docs/users-and-memberships.md) gives a user roles in the
tenant; the operator makes it on the host. Make one:

```sh
AUDITOR=$(sudo -u vcon vcon-server user add "Tenant auditor" --config $CONFIG | jq -r .id)
sudo -u vcon vcon-server membership grant $AUDITOR --tenant $TENANT --role auditor --config $CONFIG | jq -c .roles
```

```text
["auditor"]
```

`GET /tenant/memberships` lists the memberships in the tenant, oldest
first: each user's id and label, the roles granted, the scopes those roles
give under the tenant's role cap, and the time of the grant.

```sh
curl -sS --cacert $CA "$URL/tenant/memberships" -H "Authorization: Bearer $ADMIN" \
  | jq -c --arg id "$AUDITOR" '.[] | select(.user == $id) | {label, roles, scopes}'
```

```text
{"label":"Tenant auditor","roles":["auditor"],"scopes":["audit"]}
```

The list is read only: granting and revoking a role stay with the operator
until sign-in is released.

## Check the tenant's audit log

Every call above is in the tenant's own log under the administrator's key,
refusals too, and in the operator's admin log. A `settings_set` record
names the settings changed and never a value:

```sh
sudo -u vcon vcon-server audit export --tenant $TENANT --config $CONFIG \
  | jq -r --arg key "$ADMIN_ID" 'select(.actor == $key) | "\(.action) \(.status)"'
```

```text
settings_read 200
settings_set 200
settings_set 422
settings_set 200
key_create 201
key_create 422
key_create 422
key_list 200
key_revoke 204
membership_list 200
```

```sh
sudo -u vcon vcon-server audit export --tenant $TENANT --config $CONFIG \
  | jq -r --arg key "$ADMIN_ID" --arg tenant "$TENANT" 'select(.actor == $key and .action == "settings_set" and .status == 200) | .target | sub($tenant; "acme")'
```

```text
acme/masking.legal,session.idle_seconds
acme/session.idle_seconds
```

See [audit log records](@/docs/audit-log-records.md) for every action.

## When a step fails

- **`404` on a route under `/tenant`:** the server is release 0.4.3 or
  older. The routes are on `main` only.
- **`403` [`forbidden`](@/problems/forbidden.md):** the key does not hold
  `tenant_admin`, or is an operator key. `GET /key` shows a key's scopes.
- **`403` [`wrong-listener`](@/problems/wrong-listener.md):** the server
  has an [operator listener](@/docs/operator-listener.md), and the request
  went to it. The routes under `/tenant` are served at `listen`.
- **`422` [`bad-setting`](@/problems/bad-setting.md):** the body names a
  setting outside the table above, gives a masking setting something other
  than `true` or `false`, gives a limit outside its bounds, or would leave
  `session.idle_seconds` above `session.absolute_seconds`. To raise both
  limits, send both in one request.
- **`422` [`bad-scopes`](@/problems/bad-scopes.md):** the key asked for
  holds `tenant_admin`, `admin`, no scope, or a scope outside the tenant's
  role cap. `vcon-server tenant role-cap` prints the cap on the host.
- **`422` [`invalid-body`](@/problems/invalid-body.md):** the body is not
  JSON, or a key's body lacks `scopes` or `label`.
- **`key create` refuses `--scope tenant_admin`:** the binary
  is release 0.4.3 or older.
- **`the registry cannot be opened: … schema version 7 is not one this
  build knows`:** an older binary is reading a data directory that a newer
  one has opened. See
  [upgrade](@/docs/upgrading.md#schema-version-7-and-no-way-back-without-the-copy).

## Clean up

Put the setting changed above back to its default, revoke the
administrator's key, and deactivate the example user:

```sh
curl -sS --cacert $CA -X PUT "$URL/tenant/settings" -H "Authorization: Bearer $ADMIN" \
  -H 'Content-Type: application/json' -d '{"masking.legal":null}' | jq -c '."masking.legal"'
sudo -u vcon vcon-server key revoke $ADMIN_ID --config $CONFIG
sudo -u vcon vcon-server user deactivate $AUDITOR --config $CONFIG
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/tenant/settings" -H "Authorization: Bearer $ADMIN"
```

```text
true
401
```
