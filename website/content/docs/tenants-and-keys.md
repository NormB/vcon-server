+++
title = "Tenants and keys"
description = "How tenants keep their data apart, what each key scope allows, and how keys are made and revoked."
weight = 30
+++

## Tenants

A tenant is one customer, department or application whose vCons must be kept
apart from everyone else's. Each tenant has:

- its own **tenant key**, wrapped by the master key, which wraps the data key
  of each of its stored versions and keys its catalog;
- its own **directory** under `data_dir/tenants/`, holding its catalog
  (`catalog.sqlite`) and its sealed object files;
- its own **retention period**, or the server default;
- its own **audit log**, inside its catalog.

A tenant's name is 1 to 128 characters, with no control characters and no
space at either end, and unique on the server. Its id is 32 hexadecimal
digits, chosen by the server.

A uuid is unique within its tenant: two tenants may each hold a vCon with the
same uuid, and neither can tell.

## Keys and scopes

Every request carries a bearer key:

```http
Authorization: Bearer vcs_0123456789abcdef_<43 characters>
```

A token is `vcs_`, a 16-hex-digit key id, `_`, and a 32-byte random secret in
base64url without padding. The server keeps only the SHA-256 of the token, so
a token is shown once, when the key is made, and cannot be recovered. The key
id in the token is not secret; it names the key in lists and audit records.

A **tenant key** belongs to one tenant and holds any of four scopes:

| Scope | Allows |
|---|---|
| `write` | `POST /vcons` and `PUT /vcons/{uuid}` |
| `read` | `GET /vcons/{uuid}` |
| `delete` | `DELETE /vcons/{uuid}` |
| `legal` | the `/legal/` routes: reads of deleted vCons and old versions, restore, legal holds |

An **operator key** belongs to no tenant and holds only `admin`, which allows
the `/admin/` routes: tenants and their keys. A tenant key can never hold
`admin`, and an operator key can never reach a tenant's vCons.

Any valid key may ask `GET /health`.

Give each producer, reader and process its own key with only the scopes it
needs, and a label that says who holds it. A request whose key lacks the scope
is refused with [`forbidden`](@/problems/forbidden.md) and recorded.

## Making and revoking keys

- **Operator keys** are made only on the server's host, with
  `vcon-server key create --operator`, never over HTTP. A leaked operator key
  cannot be used to make another. See the [command line](@/docs/command-line.md).
- **Tenant keys** are made with `POST /admin/tenants/{id}/keys` (or
  `vcon-server key create --tenant`). The answer carries the token once, with
  `Cache-Control: no-store`.
- **Revoking** a key (`DELETE /admin/keys/{id}`) refuses its next request; a
  request already under way finishes.
- **Removing a tenant** revokes its keys in the same step.

## Removing a tenant

`DELETE /admin/tenants/{id}` removes a tenant that keeps no version. With
`?force=true` it also removes one that does, with its vCons, unless a vCon of
the tenant is under a legal hold: then it is refused with
[`under-hold`](@/problems/under-hold.md), whatever the query. Over HTTP nothing
passes a legal hold; only `vcon-server tenant remove --force --override-holds`,
run with the server stopped, can.

While a tenant is being removed, its keys get [`unavailable`](@/problems/unavailable.md)
(503); once it is gone they get [`unauthorized`](@/problems/unauthorized.md) (401).
