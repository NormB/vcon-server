+++
title = "HTTP API"
description = "Every route: the scope it needs, what it takes, and what it answers."
weight = 70
+++

## Basics

- **TLS only**, HTTP/2 or HTTP/1.1.
- **Every request carries a bearer key:** `Authorization: Bearer vcs_…`.
  A missing, malformed, unknown or revoked key is
  [`unauthorized`](@/problems/unauthorized.md) (401); a key without the
  route's scope is [`forbidden`](@/problems/forbidden.md) (403). See
  [tenants and keys](@/docs/tenants-and-keys.md).
- **A key names its tenant.** No route takes a tenant id for vCons; the
  key decides.
- **Bodies** are limited to `limits.body` bytes
  ([`too-large`](@/problems/too-large.md), 413, checked against
  `Content-Length` before anything is read) and must arrive within
  `limits.timeouts.read` seconds ([`timeout`](@/problems/timeout.md), 408).
- **Errors** are RFC 9457 problem documents, `application/problem+json`:

  ```json
  {
    "type": "https://vcon-server.com/problems/precondition-failed",
    "title": "If-Match does not name the current version",
    "status": 412
  }
  ```

  The `type` is a page on this site; [problem types](@/problems/_index.md)
  lists them all. No problem repeats any part of the request.
- **Every request is recorded** in an [audit log](@/docs/audit-logs.md),
  refused ones included.

## vCons

| Route | Scope | Takes | Answers |
|---|---|---|---|
| `POST /vcons` | `write` | a vCon body | `201` with `Location: /vcons/{uuid}` and `ETag`; `200` with the same `ETag` when the same bytes are already current |
| `GET /vcons/{uuid}` | `read` | | `200` with the body as stored, `ETag`, and `Content-Type` `application/vcon` or `application/vcon+gzip` |
| `PUT /vcons/{uuid}` | `write` | `If-Match: "<current version>"` and a vCon body with that uuid | `200` with the new `ETag` |
| `DELETE /vcons/{uuid}` | `delete` | | `204`, also when already deleted or purged |

Request media types: `application/vcon`, `application/vcon+gzip`,
`application/json`, or none. What is checked, and the refusals, are on
[storing vCons](@/docs/storing-vcons.md).

Typical refusals: [`conflict`](@/problems/conflict.md),
[`gone`](@/problems/gone.md), [`not-found`](@/problems/not-found.md),
[`immutable`](@/problems/immutable.md),
[`precondition-required`](@/problems/precondition-required.md),
[`precondition-failed`](@/problems/precondition-failed.md),
[`path-uuid-mismatch`](@/problems/path-uuid-mismatch.md),
[`bad-path-uuid`](@/problems/bad-path-uuid.md), and the format refusals
(422).

```sh
# Replace, naming the version you read
etag=$(curl -s -o /dev/null -D - https://vcon.example.com/vcons/$UUID -H "Authorization: Bearer $KEY" | awk -F': ' 'tolower($1)=="etag"{print $2}' | tr -d '\r')
curl -X PUT https://vcon.example.com/vcons/$UUID \
  -H "Authorization: Bearer $KEY" -H "If-Match: $etag" \
  -H "Content-Type: application/vcon" --data-binary @call-v2.vcon
```

## Legal

All need the `legal` scope. See [retention and legal holds](@/docs/retention-and-holds.md).

| Route | Answers |
|---|---|
| `GET /legal/vcons/{uuid}` | `200`: the current version, or the latest kept version of a deleted uuid, with `ETag` and `Vcon-State: live` or `deleted` |
| `GET /legal/vcons/{uuid}/versions` | `200`: JSON array, oldest first, of `{"version", "stored_at", "hidden_at", "current"}` |
| `GET /legal/vcons/{uuid}/versions/{version}` | `200`: that kept version, as stored, with `ETag` |
| `POST /legal/vcons/{uuid}/restore` | `200` with the restored version's `ETag`; [`not-deleted`](@/problems/not-deleted.md) if it is live |
| `PUT /legal/vcons/{uuid}/hold` | `204`: the uuid is held |
| `DELETE /legal/vcons/{uuid}/hold` | `204`: the hold is released |

A malformed version id is [`bad-path-version`](@/problems/bad-path-version.md)
(400); a purged uuid is [`gone`](@/problems/gone.md) (410).

## Admin

All need an operator key (scope `admin`). A tenant key gets
[`forbidden`](@/problems/forbidden.md). Bodies are JSON objects, and a member
a route does not take is refused with
[`invalid-body`](@/problems/invalid-body.md) (422), so a misspelt member never
goes unnoticed.

| Route | Takes | Answers |
|---|---|---|
| `POST /admin/tenants` | `{"name": "acme", "retention": "10y"}`; `retention` optional, `null` for the server default | `201` `{"id"}` |
| `GET /admin/tenants` | | `200` `[{"id", "name", "retention"}]` by name; `retention` `null` means the server default |
| `PATCH /admin/tenants/{id}` | `{"retention": "90d"}` or `{"retention": null}`; the member is required | `200` `{"id", "retention"}` |
| `DELETE /admin/tenants/{id}` | optional `?force=true` (or `force=false`) | `204` |
| `POST /admin/tenants/{id}/keys` | `{"scopes": ["write", "read"], "label": "producer-1"}` | `201` `{"id", "token"}` with `Cache-Control: no-store` |
| `GET /admin/tenants/{id}/keys` | | `200` `[{"id", "scopes", "label", "created_at", "revoked_at"}]`, oldest first; never a token |
| `DELETE /admin/keys/{id}` | | `204`, also when already revoked |

Refusals: [`unknown-tenant`](@/problems/unknown-tenant.md),
[`name-taken`](@/problems/name-taken.md), [`holds-vcons`](@/problems/holds-vcons.md),
[`under-hold`](@/problems/under-hold.md), [`bad-scopes`](@/problems/bad-scopes.md),
[`bad-query`](@/problems/bad-query.md), [`unknown-key`](@/problems/unknown-key.md),
[`master-key-limit`](@/problems/master-key-limit.md).

A tenant name is 1 to 128 characters, with no control characters and no
space at either end. A tenant key's scopes are one or more of `write`,
`read`, `delete` and `legal`, each named once.

## Health

| Route | Scope | Answers |
|---|---|---|
| `GET /health` | any valid key | `200` `{"status": "ok"}` when `server.sqlite` answers; [`unavailable`](@/problems/unavailable.md) (503) otherwise, and for a key whose tenant is being removed |

Health requests are counted in the audit log, one record per key, status and
minute. A health check that cannot be counted is answered as unhealthy.
