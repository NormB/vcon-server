+++
title = "Audit logs"
description = "What vcon-server records about each request, where, and what it never records."
weight = 60
+++

## Two kinds of log

- **Each tenant's log** lives in that tenant's catalog and records every
  request made with that tenant's keys, refused ones included, and the
  purges of its versions.
- **The admin log** lives in `server.sqlite` and records administration:
  tenants made, listed, changed and removed, keys made, listed and revoked,
  and requests that cannot be tied to a tenant.

A change that concerns a tenant, such as a key made for it or its retention
changed, is recorded in the admin log and copied to the tenant's own log.
The admin log's record commits with the change, so a failure between the two
can lose only the copy; the request then answers
[`audit-failed`](@/problems/audit-failed.md) (503), though the change stands.

A request whose record cannot be written is not carried out: it answers
[`audit-failed`](@/problems/audit-failed.md). Records are only ever added.
No route reads or changes them.

## A record

| Field | Holds |
|---|---|
| `seq` | the record's number in its log, rising |
| `at` | when, RFC 3339 in UTC |
| `actor` | the key id; `sweep` for the purge sweep; `cli` for a command on the host; empty for a token that did not parse or names no known key |
| `action` | what was asked, such as `create`, `read`, `legal_read`, `hold`, `purge`, `key_create` or `auth_failed` |
| `target` | the uuid, tenant id or key id acted on, if any |
| `version` | the version id involved, if any |
| `status` | the HTTP status answered |
| `count` | how many requests the record stands for |

A record holds no vCon content, no token and no client address.

## Counted records

Health checks and failed authentications could flood a log, so they are
counted instead: one record per actor, action, status and minute, whose
`count` rises. A health request whose count cannot be written is answered
as unhealthy. Failures by tokens that name no known key are counted with an
empty actor, so made-up tokens cannot grow the log without bound.

## Reading the logs

`vcon-server audit export (--tenant <id> | --admin) [--since <seq>]` writes a
log as JSON lines, oldest first, from after `seq`. It runs beside a serving
server. The command is [being built](@/docs/status.md).

## The request log is not the audit log

The server also writes one structured line per request to standard error:
a random request id, the method, the route template (never the path), the
status and the duration. It never holds a uuid, a tenant id, a token or any
content, so it can go to a log system that is less trusted than the data.
