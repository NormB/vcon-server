+++
title = "Add users and memberships"
description = "Make a user on the server's host, give it roles in a tenant or the operator role, list the users, take roles away, deactivate a user, and find each change in the audit logs."
weight = 36

[extra]
group = "how-to"
+++

A user is a person: someone who will sign in to the console with a
passkey or through an identity provider. A membership gives a user roles in
one tenant, or the operator role on the server. This how-to makes users and
memberships with the commands `vcon-server user` and `vcon-server
membership`, on the server's host.

This is the first part of sign-in, and the only part there is yet. The
server keeps users and memberships and records every change to them. No
request reads them: nobody can sign in as a user, and a membership lets
nobody do anything, until a later release adds a sign-in method. API keys work as
before. Use these commands now to prepare the list of people and roles, or
leave them until sign-in is released.

The commands are new in 0.4.4; release 0.4.3 does not have them. CI runs
every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A server of release 0.4.4 or later, set up as in
  [getting started](@/docs/getting-started.md), with the tenant `acme`. The
  server may be running: every command here runs beside it.
- A shell on the server's host, as an account that may run commands as the
  server's account (`sudo -u vcon`). There is no route for users or
  memberships: like an operator key, they are made only on the host.
- Nothing changes on any other system. If a user will sign in through an
  identity provider, you need two values from that provider, and you only
  copy them: its issuer, the `https://` URL in the `issuer` member of its
  `/.well-known/openid-configuration` document, and the user's subject, the
  `sub` claim the provider gives for that person. Both are stored as given
  and are compared exactly at sign-in, so copy them without changing case,
  and without a trailing `/` unless the provider's issuer has one.
- One shell for every step: later steps use variables that earlier steps set.

```sh
CONFIG=/etc/vcon-server/vcon-server.toml
ACME=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
```

## Make a user

`user add` takes a label, which is what the user is shown as: 1 to 128
characters, with no control characters and no space at either end. With
nothing else, the user is a local account, one that will sign in with a
passkey. The command prints the user's id:

```sh
LEAD=$(sudo -u vcon vcon-server user add "Night shift lead" --config $CONFIG | jq -r .id)
echo "$LEAD" | grep -Ec '^[0-9a-f]{32}$'
```

```text
1
```

For a user who will sign in through an identity provider, give the
provider's issuer and the user's subject, both or neither:

```sh
AUDITOR=$(sudo -u vcon vcon-server user add "Compliance auditor" \
  --issuer https://idp.example.com/realms/main --subject 7f3a9c1e --config $CONFIG | jq -r .id)
```

An issuer and subject belong to one active user. A second user with the
same two is refused:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server user add "Second auditor" \
  --issuer https://idp.example.com/realms/main --subject 7f3a9c1e --config $CONFIG
```

```text
vcon-server: an active user has this issuer and subject
```

## Give a user roles in a tenant

A tenant membership holds one or more of six roles. This release keeps the
names; what each will allow, once users can sign in, is given here so that
you can choose now:

| Role | For a person who |
|---|---|
| `reader` | reads the tenant's vCons |
| `producer` | stores vCons in the tenant |
| `records` | deletes the tenant's vCons |
| `legal` | reads deleted versions and sets legal holds |
| `auditor` | reads the tenant's audit log |
| `tenant-admin` | administers the tenant |

`membership grant` takes the user, the tenant, and `--role` once for each
role. It prints the roles the membership then holds, in the order of the
table:

```sh
sudo -u vcon vcon-server membership grant $AUDITOR --tenant $ACME \
  --role auditor --role reader --config $CONFIG | jq -c --arg acme "$ACME" '{tenant: (.tenant == $acme), roles}'
```

```text
{"tenant":true,"roles":["reader","auditor"]}
```

A later grant adds to the roles the membership holds:

```sh
sudo -u vcon vcon-server membership grant $AUDITOR --tenant $ACME --role legal --config $CONFIG | jq -c .roles
```

```text
["reader","legal","auditor"]
```

## Make a user an operator

An operator membership is not in a tenant, and holds one role, `operator`,
which the command does not name:

```sh
sudo -u vcon vcon-server membership grant $LEAD --operator --config $CONFIG | jq -c '{tenant, roles}'
```

```text
{"tenant":null,"roles":["operator"]}
```

A user may hold an operator membership and a membership in each of several
tenants. Each is granted and revoked by itself.

## List the users

`user list` prints one line for each user, oldest first, with its
memberships:

```sh
sudo -u vcon vcon-server user list --config $CONFIG \
  | jq -c '{label, issuer, subject, deactivated_at, roles: [.memberships[].roles[]]}'
```

```text
{"label":"Night shift lead","issuer":null,"subject":null,"deactivated_at":null,"roles":["operator"]}
{"label":"Compliance auditor","issuer":"https://idp.example.com/realms/main","subject":"7f3a9c1e","deactivated_at":null,"roles":["reader","legal","auditor"]}
```

Each line also holds the user's `id` and `created_at`, and each membership
its `tenant` (`null` for the operator membership), `scopes` (see
[roles and scopes](@/docs/roles-and-scopes.md)) and `granted_at`. The
labels, issuers and subjects are in `server.sqlite`, which is encrypted
under the master key, and in this command's output; they are in no audit
record.

## Take roles away

`membership revoke` with `--role` takes those roles and prints what is
left:

```sh
sudo -u vcon vcon-server membership revoke $AUDITOR --tenant $ACME --role legal --config $CONFIG | jq -c .roles
```

```text
["reader","auditor"]
```

Without `--role` it takes every role, which removes the membership:

```sh
sudo -u vcon vcon-server membership revoke $LEAD --operator --config $CONFIG | jq -c .roles
```

```text
[]
```

## Deactivate a user

When a person leaves, deactivate the user. The command removes every
membership the user holds and prints nothing. The user stays in the list,
with the time in `deactivated_at`, because audit records name its id:

```sh
sudo -u vcon vcon-server user deactivate $AUDITOR --config $CONFIG
sudo -u vcon vcon-server user list --config $CONFIG \
  | jq -c 'select(.label == "Compliance auditor") | {deactivated: (.deactivated_at != null), memberships}'
```

```text
{"deactivated":true,"memberships":[]}
```

A deactivated user is not made active again, and is given no membership:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server membership grant $AUDITOR --tenant $ACME --role reader --config $CONFIG
```

```text
vcon-server: the user is deactivated
```

If the person comes back, add a new user. The issuer and subject of a
deactivated user are free to give again, and the new user has a new id, so
the logs tell the two apart.

## Check the audit logs

Every one of these commands is recorded in the admin log, refusals too. The
target of a `user_add` or `user_deactivate` record is the user's id; the
target of a membership record is the user's id, then the tenant's id or
`operator`, then the roles the command named:

```sh
sudo -u vcon vcon-server audit export --admin --config $CONFIG \
  | jq -r 'select(.action | test("^(user|membership)_")) | "\(.action) \(.status)"'
```

```text
user_add 201
user_add 201
user_add 409
membership_grant 200
membership_grant 200
membership_grant 200
user_list 200
membership_revoke 200
membership_revoke 200
user_deactivate 204
user_list 200
membership_grant 409
```

A change to a membership in a tenant is copied to that tenant's own log,
and so is the deactivation of a user who held a membership there. `acme`'s
log holds the two grants, the revoke and the deactivation:

```sh
sudo -u vcon vcon-server audit export --tenant $ACME --config $CONFIG \
  | jq -r 'select(.action | test("^(user|membership)_")) | .action'
```

```text
membership_grant
membership_grant
membership_revoke
user_deactivate
```

See [audit log records](@/docs/audit-log-records.md) for every action.

## When a step fails

- **`error: unrecognized subcommand 'user'`:** the binary is release 0.4.3
  or older. The commands are new in 0.4.4:
  [upgrade](@/docs/upgrading.md).
- **`no user has this id`:** the id is not a user's. `user list` prints
  each id; a user id is 32 lowercase hex digits.
- **`no tenant has this id`:** `tenant list` prints each tenant's id.
- **`a user label is 1 to 128 characters, …`:** the label is empty, too
  long, or begins or ends with a space. Quote a label that has spaces.
- **`an issuer is an https:// URL …`:** the issuer must begin `https://`,
  be at most 255 bytes, and have no query, fragment or white space; the
  subject must be 1 to 255 ASCII characters. Copy both from the provider.
- **`an active user has this issuer and subject`:** `user list` shows
  which user. Deactivate that user first only if it is the same person
  under a record you no longer want.
- **`a tenant membership's roles are one or more of reader, producer,
  records, legal, auditor and tenant-admin; …`:** a `--role` is not one of
  the six names, or is `operator`, which only `--operator` gives.
- **`the user has no such membership`:** there is nothing to revoke:
  `user list` shows the memberships the user holds.
- **`the user is deactivated`:** add a new user for the person.
- **`the registry cannot be opened: … schema version 9 is not one this
  build knows`:** an older binary is reading a data directory that a newer
  one has opened. See
  [upgrade](@/docs/upgrading.md#schema-version-9-and-no-way-back-without-the-copy).

## Clean up

A user cannot be deleted, only deactivated. To leave the server with no
active user from this page:

```sh
sudo -u vcon vcon-server user deactivate $LEAD --config $CONFIG
sudo -u vcon vcon-server user list --config $CONFIG | jq -c 'select(.deactivated_at == null)' | wc -l
```

```text
0
```
