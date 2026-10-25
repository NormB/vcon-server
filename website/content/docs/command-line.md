+++
title = "Command line"
description = "Every vcon-server command: what it does, what it prints, its exit codes, and which may run beside a serving server."
weight = 85
+++

Every command takes `--config <file>`, the same file `serve` reads, and acts
on the data directory it names. Like `serve`, each turns core dumps off and
locks its memory before it reads a key, so it needs the same memory-lock
allowance (see [getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory)).

## Output and exit codes

- Each command prints what the matching [admin route](@/docs/http-api.md#admin)
  answers: one JSON object per line, so the output can be piped to `jq` or
  read line by line. A command with nothing to report prints nothing.
- **0:** done.
- **1:** refused or failed. Nothing is printed on standard output; standard
  error says why, as `vcon-server: <reason>`.
- **2:** the command line does not parse, such as a missing `--config` or
  `--override-holds` without `--force`.

## Tenants

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `tenant create <name> [--retention <period>]` | `{"id"}` | no |
| `tenant list` | `{"id", "name", "retention"}` per tenant, by name | yes |
| `tenant set-retention <id> <period or default>` | `{"id", "retention"}` | yes |
| `tenant remove <id> [--force [--override-holds]]` | nothing | no |

`tenant remove` without `--force` removes only a tenant that keeps no
version. `--force` also removes one that does, unless a uuid of it is under a
legal hold. `--override-holds` destroys even held vCons; it exists only here,
never over HTTP, and only with the server stopped.

## Keys

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `key create --tenant <id> --scope <s> [--scope <s>...] --label <label>` | `{"id", "token"}` | yes |
| `key create --operator --label <label>` | `{"id", "token"}` | yes |
| `key list [--tenant <id>]` | `{"id", "tenant", "scopes", "label", "created_at", "revoked_at"}` per key, oldest first | yes |
| `key revoke <id>` | nothing | yes |

A token is printed once and never again; the server keeps only its hash.
`key create --operator` is the only way to make an operator key, which is
what the admin routes need. `key list` without `--tenant` lists every key,
operator keys included (their `tenant` is `null`).

## Audit logs

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `audit export --admin [--since <seq>]` | the admin log's records | yes |
| `audit export --tenant <id> [--since <seq>]` | the tenant's log's records | yes |

Records are written oldest first, after `--since` if given; keep the last
`seq` you read and pass it next time. See [audit logs](@/docs/audit-logs.md)
for the fields.

## Key rotation

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `rekey tenant <id>` | nothing | no |

`rekey tenant` gives the tenant a new key: every stored version's data key is
rewrapped under it, in batches, and the tenant's catalog is re-encrypted under
it, so the old key opens nothing the tenant keeps. Bodies are not
re-encrypted. A run that stops, for any reason, is finished by running it
again. `rekey master`, which rotates the master key, is
[being built](@/docs/status.md).

## Running beside the server

Commands marked "no" refuse while `serve`, or any other command, holds the
data directory, with exit code 1 and `the data directory is in use; stop the
server and any other command first`. The others run beside the server, and
the server sees their changes at its next request: a revoked key is refused at
once. See [operations](@/docs/operations.md#one-server-per-directory).

## A first tenant, from nothing

```sh
vcon-server tenant create acme --config /etc/vcon-server/vcon-server.toml
# {"id":"3f0c…"}
vcon-server key create --operator --label ops --config /etc/vcon-server/vcon-server.toml
# {"id":"…","token":"vcs_…"}   keep this token: it manages tenants over HTTP
vcon-server key create --tenant 3f0c… --scope write --label recorder-1 \
  --config /etc/vcon-server/vcon-server.toml
# {"id":"…","token":"vcs_…"}   give this one to the producer
```
