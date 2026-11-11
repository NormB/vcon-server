+++
title = "Audit logs"
description = "What vcon-server records about each request, where, and what it never records."
weight = 43

[extra]
group = "concepts"
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

A record is one JSON object with eight members: its number in the log,
when, who asked, what was asked, on what, which version, the status
answered, and how many requests it stands for. It holds no vCon content, no
token and no client address.
[Audit log records](@/docs/audit-log-records.md#a-record) describes each
member.

## Every action

Every route, every command that changes something and the purge sweep
write a record under an action of their own, such as `create`, `read`,
`purge` or `key_revoke`.
[Audit log records](@/docs/audit-log-records.md#every-action) lists every
action, what it records and which log holds it.

## Reveals

Reveals are new in 0.4.3: release 0.4.2 records each of these reads as
`read` or `legal_read`, and never writes a `reveal_` or `legal_reveal_`
action.

A client that shows a vCon masked, as the [console](@/docs/console.md)
does, reads it once for its summary and again each time a person chooses
to see a part of it. That second read carries a marker, `?reveal=<part>`,
and is recorded as `reveal_<part>` in place of `read`, or as
`legal_reveal_<part>` in place of `legal_read` on the legal read routes.
The parts are `parties`, `subject`, `dialog`, `analysis`, `links`, `all`
(the whole vCon) and `audio` (one dialog's recording, to play it).

The marker changes the record's action and nothing else. The route, the
scope the key needs and the answer are those of the read without it: the
answer is the whole vCon either way, so a marker gives a key nothing it
could not already read, and a read without a marker is recorded as `read`,
as before. A record still holds no content: it names the part, never what
the part held. The server records what the client says it showed; a client
that reads without the marker and shows everything is recorded as a `read`.

### See who revealed what

The steps that list the reveals of one vCon, with the key that made each
and the part it showed, are on
[read the audit logs](@/docs/read-audit-logs.md#see-who-revealed-what).

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

On a server with an [operator listener](@/docs/operator-listener.md), a
valid key refused because it came to the listener that serves the other
kind of key is counted as action `wrong_listener`, with status 403, under
the key's id. The request is refused whether or not the count could be
written. The action is new in 0.4.3.

Each time a listener fills, that is, holds
[`limits.connections`](@/docs/configuration.md#the-most-connections-at-once)
connections and takes no more until one ends, is counted in the admin log
as action `listener_full`, with an empty actor and the listener's
configuration key (`listen` or `operator.listen`) as the target: one
record for each listener and minute. The record describes a state of the
listener and answers no request, so its `status` is `null`, where every
other record has a number. Its `count` is the times the listener became
full, not the connections that waited: the listener cannot see how many
wait in the system's queue. No caller is recorded, since the connections
that wait have not been taken. The listener goes on whether or not the
count could be written. The action is new in 0.4.3.

A browser's report that a [console](@/docs/console.md) page broke the
page's content-security policy, sent to `POST /console/csp-report`, is
counted in the admin log as action `csp_report`, with an empty actor and
no target, under the status that answered it: 204 for a report taken,
429 for the reports refused in a minute past `limits.csp_reports` or
`limits.csp_reports_per_address` (written once for the minute, not for
each request), or
400, 408, 413 or 415 for a request refused. A browser sends a report with
no key, so the route takes none; one record for each status and minute is
all a sender can add. Nothing a report says is stored. The request is
answered whether or not the count could be written. See
[reports of a broken policy](@/docs/console.md#reports-of-a-broken-policy).
The action is new in 0.4.3.

## Reading the logs

`vcon-server audit export (--tenant <id> | --admin) [--since <seq>]` writes a
log as JSON lines, oldest first, from after `seq`. It runs beside a serving
server. An export from the command line is not itself recorded.

Over HTTPS, a tenant key with the `audit` scope reads its tenant's log with
`GET /audit`, and an operator key reads the admin log with
`GET /admin/audit`, a page at a time from after a `seq`. Each page is the
same lines `audit export` writes for those records, byte for byte. Each read
over HTTPS is recorded, as `audit_read`, in the log it read. See the
[HTTP API](@/docs/http-api.md#audit-logs). [Read the audit logs](@/docs/read-audit-logs.md) shows both
ways, with every command and what it prints.

## The request log is not the audit log

The server also writes one structured line per request to standard error:
a random request id, the method, the route template (never the path), the
status and the duration. It never holds a uuid, a tenant id, a token or any
content, so it can go to a log system that is less trusted than the data.
