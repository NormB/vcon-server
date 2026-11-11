+++
title = "Roles, scopes and a tenant's role cap"
description = "See which scope each role of a membership gives, read a membership's scopes, set the roles a tenant's memberships may be given, and find each change in the audit logs."
weight = 37

[extra]
group = "how-to"
+++

A key holds scopes, and each route needs one scope. A membership holds
roles, and each role gives one scope, so a membership's scopes are those of
its roles. This how-to shows the scopes of a membership, and sets a role
cap on a tenant: the roles a membership in that tenant may be given. Use a
cap when a tenant must never have, for example, a user who deletes vCons or
a tenant administrator, whoever grants the roles later.

This is the second part of sign-in. As with
[users and memberships](@/docs/users-and-memberships.md), no request reads
a membership yet: nobody can sign in as a user, and a membership's scopes
let nobody do anything, until a later release adds sessions. API keys work
as before. Use these commands now to decide each tenant's roles, or leave
them until sign-in is released.

The commands are on `main` only; release 0.4.3 does not have them. CI runs
every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A server built from `main`, set up as in
  [getting started](@/docs/getting-started.md), with the tenant `acme`. The
  server may be running: every command here runs beside it.
- A shell on the server's host, as an account that may run commands as the
  server's account (`sudo -u vcon`). There is no route for a role cap: like
  an operator key, it is set only on the host.
- Nothing changes on any other system.
- One shell for every step: later steps use variables that earlier steps set.

```sh
CONFIG=/etc/vcon-server/vcon-server.toml
ACME=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
```

## The scope each role gives

| Role | Scope | The scope allows |
|---|---|---|
| `reader` | `read` | `GET /vcons` and `GET /vcons/{uuid}` |
| `producer` | `write` | `POST /vcons` and `PUT /vcons/{uuid}` |
| `records` | `delete` | `DELETE /vcons/{uuid}` |
| `legal` | `legal` | the `/legal/` routes |
| `auditor` | `audit` | `GET /audit` |
| `tenant-admin` | `tenant_admin` | the `/tenant/` routes |
| `operator` | `admin` | the `/admin/` routes |

The first six are the roles of a membership in a tenant; `operator` is the
one role of an operator membership. The role is spelled `tenant-admin` and
its scope `tenant_admin`. A tenant key may hold `tenant_admin` too
(`key create --scope tenant_admin`), and such a key reaches the `/tenant/`
routes: see [administer a tenant with a key](@/docs/tenant-administration.md).

## Read a membership's scopes

Make a user and give it two roles in `acme`:

```sh
CLERK=$(sudo -u vcon vcon-server user add "Records clerk" --config $CONFIG | jq -r .id)
sudo -u vcon vcon-server membership grant $CLERK --tenant $ACME \
  --role reader --role records --config $CONFIG | jq -c .roles
```

```text
["reader","records"]
```

`user list` prints each membership's roles and, beside them, its scopes:

```sh
sudo -u vcon vcon-server user list --config $CONFIG \
  | jq -c --arg id "$CLERK" 'select(.id == $id) | .memberships[] | {roles, scopes}'
```

```text
{"roles":["reader","records"],"scopes":["read","delete"]}
```

## Set a tenant's role cap

A new tenant has no cap: a membership in it may be given any of the six
tenant roles. `tenant role-cap` prints the cap, `null` for none:

```sh
sudo -u vcon vcon-server tenant role-cap $ACME --config $CONFIG | jq -c .role_cap
```

```text
null
```

`tenant set-role-cap` takes the tenant and `--role` once for each role the
cap allows. It prints the cap, in the order of the table:

```sh
sudo -u vcon vcon-server tenant set-role-cap $ACME \
  --role auditor --role reader --config $CONFIG | jq -c .role_cap
```

```text
["reader","auditor"]
```

The cap holds at once, with the server running. A grant that names a role
outside it is refused, and gives none of the roles it names:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server membership grant $CLERK --tenant $ACME --role auditor --role legal --config $CONFIG
```

```text
vcon-server: a role named is outside the tenant's role cap
```

A grant within the cap is made:

```sh
sudo -u vcon vcon-server membership grant $CLERK --tenant $ACME --role auditor --config $CONFIG | jq -c .roles
```

```text
["reader","records","auditor"]
```

## A role granted before the cap

The clerk was given `records` before the cap was set, and the cap leaves
`records` out. The membership keeps the role in its row, and the role stops
counting: the membership's scopes are those of the roles the cap allows.

```sh
sudo -u vcon vcon-server user list --config $CONFIG \
  | jq -c --arg id "$CLERK" 'select(.id == $id) | .memberships[] | {roles, scopes}'
```

```text
{"roles":["reader","records","auditor"],"scopes":["read","audit"]}
```

To take the role out of the row as well, revoke it; a role outside the cap
can always be revoked:

```sh
sudo -u vcon vcon-server membership revoke $CLERK --tenant $ACME --role records --config $CONFIG | jq -c .roles
```

```text
["reader","auditor"]
```

A cap is on one tenant. It changes no other tenant's memberships and no
operator membership.

## Remove the cap

`--all` removes the cap, so every tenant role may be granted again, and a
role that had stopped counting counts again:

```sh
sudo -u vcon vcon-server tenant set-role-cap $ACME --all --config $CONFIG | jq -c .role_cap
```

```text
null
```

## Check the audit logs

Both commands are recorded in the admin log, refusals too. The target of a
`role_cap_set` record is the tenant's id, then the cap's roles or `all`;
the target of a `role_cap_read` record is the tenant's id:

```sh
sudo -u vcon vcon-server audit export --admin --config $CONFIG \
  | jq -r --arg acme "$ACME" 'select(.action | test("^role_cap_")) | "\(.action) \(.target | sub($acme; "acme")) \(.status)"'
```

```text
role_cap_read acme 200
role_cap_set acme/reader,auditor 200
role_cap_set acme/all 200
```

The refused grant is there as `membership_grant` with status 422:

```sh
sudo -u vcon vcon-server audit export --admin --config $CONFIG \
  | jq -r --arg id "$CLERK" 'select(.action == "membership_grant" and (.target | startswith($id))) | .status'
```

```text
200
422
200
```

A change of the cap is copied to the tenant's own log; a read of it is not:

```sh
sudo -u vcon vcon-server audit export --tenant $ACME --config $CONFIG \
  | jq -r 'select(.action | test("^role_cap_")) | .action'
```

```text
role_cap_set
role_cap_set
```

See [audit log records](@/docs/audit-log-records.md) for every action.

## When a step fails

- **`error: unrecognized subcommand 'set-role-cap'`:** the binary is
  release 0.4.3 or older. The commands are on `main` only.
- **`no tenant has this id`:** `tenant list` prints each tenant's id.
- **`a role cap is one or more of reader, producer, records, legal,
  auditor and tenant-admin`:** a `--role` is not one of the six names, or
  is `operator`, which no tenant membership holds. To allow every role, use
  `--all`.
- **`a role named is outside the tenant's role cap`:** `tenant role-cap`
  prints the roles the tenant allows. Grant only those, or widen the cap
  first.
- **`error: the argument '--all' cannot be used with '--role <ROLES>'`:**
  give the roles of the cap, or `--all` to remove it, not both.
- **A membership's `scopes` are fewer than its `roles`:** the tenant's cap
  leaves a role out. `tenant role-cap` prints the cap.
- **`a tenant key's scopes are one or more of write, read, delete, legal,
  audit and tenant_admin, each named once`:** a key takes the scope's name,
  `tenant_admin`, not the role's, `tenant-admin`.
- **`the registry cannot be opened: … schema version 7 is not one this
  build knows`:** an older binary is reading a data directory that a newer
  one has opened. See
  [upgrade](@/docs/upgrading.md#schema-version-7-and-no-way-back-without-the-copy).

## Clean up

The cap was removed above. To leave the server with no active user from
this page:

```sh
sudo -u vcon vcon-server user deactivate $CLERK --config $CONFIG
sudo -u vcon vcon-server tenant role-cap $ACME --config $CONFIG | jq -c .role_cap
```

```text
null
```
