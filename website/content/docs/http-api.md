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
- **An OpenAPI 3.1 description** of every route, status and problem type is
  at <https://vcon-server.com/openapi.json>, and each server answers the same
  file at `GET /openapi.json` to any valid key. Tests hold it to the code.
  Load it into an OpenAPI tool to generate a client:

  ```sh
  curl -s https://vcon.example.com/openapi.json -H "Authorization: Bearer $KEY" -o openapi.json
  ```

## vCons

| Route | Scope | Takes | Answers |
|---|---|---|---|
| `POST /vcons` | `write` | a vCon body | `201` with `Location: /vcons/{uuid}` and `ETag`; `200` with the same `ETag` when the same bytes are already current |
| `GET /vcons` | `read` | optional `limit`, `cursor`, `since`, `until` | `200` with a page of the tenant's live vCons, newest first; see [listing](#listing) |
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

### Listing

`GET /vcons` answers a page of the key's tenant's vCons as JSON. Deleted
vCons are left out. The order is newest first by when each uuid was first
stored; a replace, a delete or a restore does not move a vCon.

```json
{
  "next": "GOHD-NKkTRUBoH2owruD5bmiJ54NFrxG",
  "vcons": [
    {
      "stored_at": "2026-10-25T12:00:00.123456789Z",
      "updated_at": "2026-10-25T12:30:00Z",
      "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
      "version": "4f0c4b0a1d2e3f405162738495a6b7c8"
    }
  ]
}
```

- `stored_at` is when the uuid was first stored, `updated_at` when its
  current version was, and `version` is the current version's `ETag`
  without quotes. Times are RFC 3339 in UTC.
- `next` is the cursor of the next page, `null` on the last. Send it back
  unchanged as `cursor`.
- `limit` is 1 to 1000 vCons a page (default 100).
- `since` (inclusive) and `until` (exclusive) are RFC 3339 times that bound
  `stored_at`. Send the same bounds with every page. Percent-encode `:` and
  `+` in them.
- A parameter the route does not take, one given twice, or a bad value is
  [`bad-query`](@/problems/bad-query.md) (400).

Paging with a cursor returns each vCon that stays live while you page
exactly once, whatever is stored, replaced, deleted or restored meanwhile.

```sh
# Every vCon stored on 25 October 2026 (UTC), 500 a page
query="limit=500&since=2026-10-25T00%3A00%3A00Z&until=2026-10-26T00%3A00%3A00Z"
page=$(curl -s "https://vcon.example.com/vcons?$query" -H "Authorization: Bearer $KEY")
while :; do
  echo "$page" | jq -r '.vcons[].uuid'
  next=$(echo "$page" | jq -r '.next // empty')
  [ -n "$next" ] || break
  page=$(curl -s "https://vcon.example.com/vcons?$query&cursor=$next" -H "Authorization: Bearer $KEY")
done
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
| `GET /admin/tenants/{id}/keys` | | `200` `[{"id", "tenant", "scopes", "label", "created_at", "revoked_at"}]`, oldest first; never a token |
| `DELETE /admin/keys/{id}` | | `204`, also when already revoked |

Refusals: [`unknown-tenant`](@/problems/unknown-tenant.md),
[`name-taken`](@/problems/name-taken.md), [`holds-vcons`](@/problems/holds-vcons.md),
[`under-hold`](@/problems/under-hold.md), [`bad-scopes`](@/problems/bad-scopes.md),
[`bad-query`](@/problems/bad-query.md), [`unknown-key`](@/problems/unknown-key.md),
[`master-key-limit`](@/problems/master-key-limit.md).

A tenant name is 1 to 128 characters, with no control characters and no
space at either end. A tenant key's scopes are one or more of `write`,
`read`, `delete`, `legal` and `audit`, each named once.

## Audit logs

| Route | Scope | Answers |
|---|---|---|
| `GET /audit` | `audit` (a tenant key) | `200`: the key's tenant log |
| `GET /admin/audit` | `admin` (an operator key) | `200`: the admin log |

Both answer `Content-Type: application/jsonl`: one record a line, oldest
first, exactly the lines `vcon-server audit export` writes for the same
records (see [audit logs](@/docs/audit-logs.md#a-record)).

- `after` is a `seq`: the page starts after it (default `0`, the start).
- `limit` is 1 to 10000 records a page (default 1000).
- A page shorter than `limit` reaches the end of the log. Ask for the next
  page with `after` set to the last `seq` you read.
- Each read is itself recorded, as `audit_read`, in the log it read, after
  the page is read; so the next page starts with it.
- A parameter the routes do not take, one given twice, or a bad value is
  [`bad-query`](@/problems/bad-query.md) (400).

```sh
# Copy a tenant's log, 1000 records a page, resuming from the last seq read
after=0
while :; do
  page=$(curl -s "https://vcon.example.com/audit?after=$after" -H "Authorization: Bearer $KEY")
  [ -n "$page" ] || break
  printf '%s\n' "$page" >> audit.jsonl
  [ "$(printf '%s\n' "$page" | wc -l)" -lt 1000 ] && break
  after=$(printf '%s\n' "$page" | tail -n 1 | jq .seq)
done
```

## Your key

| Route | Scope | Answers |
|---|---|---|
| `GET /key` | any valid key | `200` `{"id", "tenant", "scopes"}` of the key that sent the request, with `Cache-Control: no-store`; `tenant` is `null` for an operator key |

A client, such as the [console](@/docs/console.md), uses it to learn what its
key may do. Each request is recorded as `key_info`, in the key's tenant log,
or the admin log for an operator key. A key whose tenant is being removed,
or whose tenant's log cannot be opened, gets
[`unavailable`](@/problems/unavailable.md) (503).

```sh
curl https://vcon.example.com/key -H "Authorization: Bearer $KEY"
```

## Health and description

| Route | Scope | Answers |
|---|---|---|
| `GET /openapi.json` | any valid key | `200`: this API's OpenAPI description, `application/json`; recorded as `openapi` |
| `GET /health` | any valid key | `200` `{"status": "ok"}` when `server.sqlite` answers; [`unavailable`](@/problems/unavailable.md) (503) otherwise, and for a key whose tenant is being removed |

Health requests are counted in the audit log, one record per key, status and
minute. A health check that cannot be counted is answered as unhealthy.
