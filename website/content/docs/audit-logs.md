+++
title = "Audit logs"
description = "What vcon-server records about each request, where, and what it never records."
weight = 60
+++

## Two kinds of log

- **Each tenant's log** lives in that tenant's catalog and records every
  request made with that tenant's keys, refused ones included, the purges of
  its versions, and each [`export parquet`](@/docs/export-parquet.md) run
  (action `export`, actor `cli`, status 200 or 500).
- **The admin log** lives in `server.sqlite` and records administration:
  tenants made, listed, changed and removed, keys made, listed and revoked,
  backups, and requests that cannot be tied to a tenant.

A change that concerns a tenant, such as a key made for it, its retention
changed or its metadata level set, is recorded in the admin log and copied to
the tenant's own log.
The admin log's record commits with the change, so a failure between the two
can lose only the copy; the request then answers
[`audit-failed`](@/problems/audit-failed.md) (503), though the change stands.

A request whose record cannot be written is not carried out: it answers
[`audit-failed`](@/problems/audit-failed.md). Records are only ever added.
No route changes them.

## A record

| Field | Holds |
|---|---|
| `seq` | the record's number in its log, rising |
| `at` | when, RFC 3339 in UTC |
| `actor` | the key id; `sweep` for the purge sweep; `cli` for a command on the host; empty for a token that did not parse or names no known key |
| `action` | what was asked: one of the [actions](#every-action) below |
| `target` | the uuid, tenant id or key id acted on, if any |
| `version` | the version id involved, if any |
| `status` | the HTTP status answered |
| `count` | how many requests the record stands for |

A record holds no vCon content, no token and no client address.

## Every action

Each action, what it records, and the log it is in. "The key's log" is the
tenant log of the key's tenant, or the admin log for an operator key. An
action through MCP is recorded as its HTTP route records it.

| Action | Records | Log |
|---|---|---|
| `create` | a vCon stored (`POST /vcons`) | the tenant's |
| `replace` | a vCon replaced by a new version (`PUT /vcons/{uuid}`) | the tenant's |
| `read` | a vCon read (`GET /vcons/{uuid}`) | the tenant's |
| `delete` | a vCon deleted (`DELETE /vcons/{uuid}`) | the tenant's |
| `list` | a page of the vCon list (`GET /vcons`) | the tenant's |
| `purge` | a version erased, by the purge sweep (actor `sweep`), or at once by a delete when the period is `0d` | the tenant's |
| `legal_read` | a read through the legal routes of the latest kept version or of one version (`GET /legal/vcons/{uuid}`, `GET /legal/vcons/{uuid}/versions/{version}`) | the tenant's |
| `legal_versions` | a read of a vCon's version history | the tenant's |
| `restore` | a deleted vCon made live again | the tenant's |
| `hold` | a legal hold placed | the tenant's |
| `release` | a legal hold released | the tenant's |
| `audit_read` | a page of a log read over HTTPS or MCP (`GET /audit`, `GET /admin/audit`) | the log read |
| `retention` | the tenant's retention period changed, copied from the admin log's `tenant_retention` | the tenant's |
| `metadata` | the tenant's metadata level changed, copied from the admin log's `tenant_metadata` | the tenant's |
| `export` | a run of `vcon-server export parquet`, with its outcome (actor `cli`) | the tenant's |
| `key_info` | a key reading what it may do (`GET /key`) | the key's |
| `openapi` | a read of `GET /openapi.json` | the key's |
| `health` | a health check, [counted](#counted-records) | the key's |
| `auth_failed` | a request whose token was refused, [counted](#counted-records) | the key's tenant's when the key is known, otherwise the admin log with an empty actor |
| `mcp` | a request to `/mcp`, [counted](#counted-records) | the key's |
| `key_create` | a key made, over the admin API or by `vcon-server key create` | admin, copied to the tenant's for a tenant key |
| `key_revoke` | a key revoked | admin, copied to the tenant's for a tenant key |
| `key_list` | the keys listed | admin |
| `tenant_create` | a tenant made | admin |
| `tenant_list` | the tenants listed | admin |
| `tenant_retention` | a tenant's retention period changed | admin |
| `tenant_metadata` | a tenant's metadata level changed (`PUT /admin/tenants/{id}/metadata`, `vcon-server tenant set-metadata`) | admin |
| `tenant_remove` | a tenant removed | admin |
| `rekey_tenant` | a tenant key rotated (`vcon-server rekey tenant`) | admin |
| `rekey_master` | the master key rotated (`vcon-server rekey master`) | admin |
| `backup` | a run of `vcon-server backup`, with its outcome | admin |

## Counted records

Health checks and failed authentications could flood a log, so they are
counted instead: one record per actor, action, status and minute, whose
`count` rises. A health request whose count cannot be written is answered
as unhealthy. Failures by tokens that name no known key are counted with an
empty actor, so made-up tokens cannot grow the log without bound.

Requests to the MCP server at `/mcp` are counted the same way, as action `mcp`
with the status each was answered, in the key's tenant log or, for an
operator key, the admin log. A request whose count cannot be written is
answered 503 audit-failed.

## Reading the logs

`vcon-server audit export (--tenant <id> | --admin) [--since <seq>]` writes a
log as JSON lines, oldest first, from after `seq`. It runs beside a serving
server. An export from the command line is not itself recorded.

Over HTTPS, a tenant key with the `audit` scope reads its tenant's log with
`GET /audit`, and an operator key reads the admin log with
`GET /admin/audit`, a page at a time from after a `seq`. Each page is the
same lines `audit export` writes for those records, byte for byte. Each read
over HTTPS is recorded, as `audit_read`, in the log it read. See the
[HTTP API](@/docs/http-api.md#audit-logs).

## The request log is not the audit log

The server also writes one structured line per request to standard error:
a random request id, the method, the route template (never the path), the
status and the duration. It never holds a uuid, a tenant id, a token or any
content, so it can go to a log system that is less trusted than the data.
