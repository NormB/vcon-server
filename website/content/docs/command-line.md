+++
title = "Command line"
description = "Every vcon-server command: what it does, what it prints, its exit codes, and which may run beside a serving server."
weight = 54

[extra]
group = "reference"
+++

Every command but `mcp` and `tui` takes `--config <file>`, the same file `serve` reads,
and acts on the data directory it names. Like `serve`, each turns core dumps off and
locks its memory before it reads a key, so it needs the same memory-lock
allowance (see [getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory)).
Run each as the service account that owns the data directory, never as root.

With the objects in an S3 bucket (`storage.objects = "s3"`; see
[keep vCons in an S3 bucket](@/docs/s3-storage.md)),
every command but `mcp`, `tui` and `rekey master` (which touches only
`server.sqlite`) also reads the secret key file, reaches the bucket,
reads its versioning and checks the claim on its prefix before it does
anything else, and exits 1 with `the storage cannot be used:` and the reason
when one of these fails. So the bucket must be reachable from the host
where the commands run, as from the server. `check` reads every object, one
request for each version, and `backup` copies every object from the bucket
into files.

## Serving

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `serve --config <file>` | log lines on standard error; nothing on standard output | no |

`serve` runs the server until SIGTERM or SIGINT, then exits 0. It refuses to
start, with exit code 1 and the reason, when the configuration, the master
key, the data directory, the certificate or the storage cannot be used.
[Getting started](@/docs/getting-started.md) runs it under systemd, and
[operations](@/docs/operations.md#logs) lists the lines it logs.

## Output and exit codes

- Each command prints what the matching [admin route](@/docs/http-api.md#admin)
  answers: one JSON object per line, so the output can be piped to `jq` or
  read line by line. A command with nothing to report prints nothing.
- **0:** done.
- **1:** refused or failed. Nothing is printed on standard output (except by
  `check`, which prints its report first); standard error says why, as
  `vcon-server: <reason>`.
- **2:** the command line does not parse, such as a missing `--config` or
  `--override-holds` without `--force`.

## Tenants

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `tenant create <name> [--retention <period>]` | `{"id"}` | no |
| `tenant list` | `{"id", "name", "retention", "metadata"}` per tenant, by name | yes |
| `tenant set-retention <id> <period or default>` | `{"id", "retention"}` | yes |
| `tenant set-metadata <id> <none, counts or full>` | `{"id", "metadata"}` | yes |
| `tenant set-role-cap <id> --role <r> [--role <r>...]` | `{"id", "role_cap"}` | yes |
| `tenant set-role-cap <id> --all` | `{"id", "role_cap"}` | yes |
| `tenant role-cap <id>` | `{"id", "role_cap"}` | yes |
| `tenant remove <id> [--force [--override-holds]]` | nothing | no |

`tenant remove` without `--force` removes only a tenant that keeps no
version. `--force` also removes one that does, unless a uuid of it is under a
legal hold. `--override-holds` destroys even held vCons; it exists only here,
never over HTTP, and only with the server stopped.

`tenant set-metadata` sets which clear columns the tenant's Parquet exports
carry: `none` (the default), `counts` or `full`. See
[export to Parquet](@/docs/export-parquet.md#choose-the-metadata-level).

`tenant set-role-cap` sets the roles a membership in the tenant may be
given, and `--all` removes the cap; `tenant role-cap` prints it. `role_cap`
is the cap's roles, or `null` for a tenant with no cap. See
[roles, scopes and a tenant's role cap](@/docs/roles-and-scopes.md).

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

## Users and memberships

On `main` only; release 0.4.3 does not have these commands. A user is a
person who will sign in; a membership gives a user roles in a tenant, or
the operator role. These commands are the only way to make either, and no
request reads them yet. See
[add users and memberships](@/docs/users-and-memberships.md).

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `user add <label> [--issuer <url> --subject <subject>]` | `{"id"}` | yes |
| `user list` | `{"id", "label", "issuer", "subject", "created_at", "deactivated_at", "memberships"}` per user, oldest first | yes |
| `user deactivate <id>` | nothing | yes |
| `membership grant <user> --tenant <id> --role <r> [--role <r>...]` | `{"user", "tenant", "roles"}` | yes |
| `membership grant <user> --operator` | `{"user", "tenant", "roles"}` | yes |
| `membership revoke <user> --tenant <id> [--role <r>...]` | `{"user", "tenant", "roles"}` | yes |
| `membership revoke <user> --operator` | `{"user", "tenant", "roles"}` | yes |

`--issuer` and `--subject` go together: the issuer of an identity provider
and the subject it gives the user. Without them the user is a local
account. A tenant role is `reader`, `producer`, `records`, `legal`,
`auditor` or `tenant-admin`; an operator membership holds `operator` alone.
`membership grant` and `membership revoke` print the roles the membership
holds afterwards, and `tenant` is `null` for the operator membership.
`membership revoke` without `--role` takes every role. A membership with no
role left is removed. `user deactivate` removes the user's memberships;
the user stays in `user list`, and its issuer and subject may be given to
a new user. Each entry of `memberships` is `{"tenant", "roles", "scopes",
"granted_at"}`: `scopes` are what the membership lets its user do, the scopes
of those of its roles that the tenant's role cap allows.

## Audit logs

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `audit export --admin [--since <seq>]` | the admin log's records | yes |
| `audit export --tenant <id> [--since <seq>]` | the tenant's log's records | yes |

Records are written oldest first, after `--since` if given; keep the last
`seq` you read and pass it next time. See
[audit log records](@/docs/audit-log-records.md) for the fields, and
[read the audit logs](@/docs/read-audit-logs.md) for the steps.

## Key rotation

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `rekey tenant <id>` | nothing | no |
| `rekey master --new-master <file>` | `{"master_key_id", "next"}` | no |

`rekey tenant` gives the tenant a new key: every stored version's data key is
rewrapped under it, in batches, and the tenant's catalog is re-encrypted under
it, so the old key opens nothing the tenant keeps. Bodies are not
re-encrypted. A run that stops, for any reason, is finished by running it
again.

`rekey master` rewraps every tenant key, and every key a stopped
`rekey tenant` left pending, under the master key in `--new-master`. It works
on a copy of `server.sqlite` under the new key and renames the copy over the
original only once it is complete, so until then the original is untouched. A
run stopped before the rename starts again; one stopped after it has nothing
left to do. Afterwards, point `master_key` in the configuration at the new
file; the old key no longer opens anything. The full procedure is in
[operations](@/docs/operations.md#key-rotation).

## Checking

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `check` | one line per tenant, then a summary | no |

`check` is the one command that prints its report and still exits 1, when it
found a problem: the report says what. It opens every stored version and
prints one JSON line per tenant and a summary line. A tenant's line gives:

- `tenant` and `name`: the tenant's id and its name;
- `versions` kept, and `damaged`: the versions whose sealed file is missing or
  does not open;
- `packed`: the versions packed into Parquet files (see
  [Parquet storage](@/docs/parquet-storage.md));
- `leftovers`: files a crash left that no row names (the next start removes
  them; `check` only reports them);
- `past_due` and `oldest_past_due`: versions due for purge, and since when;
- `purges_awaiting_rotation` and `oldest_purge_awaiting_rotation`: purges
  since the tenant's key was last rotated;
- `rotation_stopped`: a `rekey tenant` that stopped part way;
- `problems`: what needs the operator.

The summary gives `tenants`, the number of tenants;
`removed_tenants_awaiting_master_rotation` and
`oldest_removal_awaiting_master_rotation`, the tenants removed since the
master key was last rotated, and since when; and the total `problems`. A problem is damage, a catalog that does not open (under its
tenant key, or at all, with the path and the system's error, such as
`Permission denied (os error 13)`), objects that the storage did not read,
packed versions with no `[storage.parquet]` in the configuration (`serve`
refuses to start then), a stopped rotation, or a version that has waited
longer than `purge_interval` (a sweep that is not keeping up). Leftovers and work waiting for rotation are
reported but are not problems.

Objects that cannot be read, such as files the service account may not read
or a bucket key refused `s3:GetObject`, are reported with the error's kind
and, when the system gives one, its error number, as the server's
`a stored version's object could not be read` log line gives them, and
never with an object's key:

```text
its objects cannot be read: permission denied (os error 13)
```

A service that does not answer gives a kind alone, such as
`its objects cannot be read: timed out`. `check` exits 1, after printing its report, when it
found a problem.

## Backup

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `backup <dir>` | `{"tenants", "tries", "versions"}` | yes |

`backup` writes a consistent copy of the data directory to `<dir>`, which
must not exist yet, while the server runs. See
[back up and restore](@/docs/operations.md#back-up-and-restore) for what it
copies, how to restore the copy, and what to keep apart from it.

## Export

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `export parquet --tenant <id> --to <dir or s3://bucket/prefix> [--since <time>] [--until <time>]` | `{"tenant", "metadata", "to", "manifest", "seq", "files", "rows", "bytes", "skipped", "unread"}` | yes |

`export parquet` writes a copy of the tenant's kept versions, current and
hidden, as Parquet files under `tenant=<id>/date=<yyyy-mm-dd>/`, and then a
manifest, `tenant=<id>/manifest/<seq>.json`, that names them. It reads the
store and changes nothing in it but one `export` record in the tenant's audit
log. `--since` and `--until` are RFC 3339 times; a version stored at or after
`--since` and before `--until` is exported. See
[export to Parquet](@/docs/export-parquet.md).

## Storage

| Command | Prints | Beside a server |
|---|---|---|
| `storage move-objects` | `{"tenant", "moved", "missing"}` per tenant, then `{"moved"}` | no |

`storage move-objects` moves every tenant's sealed objects from the data
directory to the bucket that `storage.objects = "s3"` names (see
[keep vCons in an S3 bucket](@/docs/s3-storage.md#move-objects-stored-in-the-data-directory)).
Each object is written to the bucket, read back and compared byte for byte
before its file is removed, so a run that stops loses nothing: run it again
to move the rest. `missing` counts the versions `check` finds without an
object afterwards. It is refused when the configuration keeps the objects in
the data directory.

| Command | Prints | Beside a server |
|---|---|---|
| `migrate` | `{"from", "message", "migrated", "to"}` | no |

`migrate` applies the migrations a PostgreSQL catalog's database has not
yet, as `vcon_owner`, one transaction per schema version (see
[keep the catalogs in PostgreSQL](@/docs/postgres-catalog.md#make-the-schema)).
It needs no data directory or master key, and takes a configuration whose
`[catalog.postgres]` names a role that may act as `vcon_owner`. A server
never migrates at start; every new connection refuses a schema other than
the binary's. It is refused when the catalog is local.

## MCP for an AI agent

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `mcp --config <client file>` | MCP messages on standard output | yes |

`mcp` is for an AI agent that starts its MCP servers as commands. It reads MCP
messages on standard input, sends each to a running server's `/mcp` with one
API key, and writes the server's answers to standard output, so the agent
reaches the same tools, scopes and audit log as over HTTPS. It reads a client
file instead of the server's configuration, and it opens no data directory
and no master key, so it can run on another machine. It stops with exit code
0 when standard input ends.

```toml
url = "https://vcon.example.com:8443/mcp"
key_file = "/home/agent/.config/vcon-server/agent.token"
ca = "/home/agent/.config/vcon-server/ca.pem"
```

- `url`: the server's MCP address. It must start with `https://`.
- `key_file`: a file holding one API token and nothing else. The token is
  never on the command line or in the environment.
- `ca`: optional. PEM certificates to trust for the server, such as a private
  CA's; without it, the system's trusted roots are used.

The command connects directly, through no proxy. It exits with code 1 when the
client file or the key file cannot be read, when the server cannot be reached,
or when the server refuses the key: `vcon-server: the server refused the key
(401)`. Like the other commands it locks its memory, so the account that runs
it needs the same memory-lock allowance.

## The terminal UI

| Command | Prints | Runs beside `serve` |
|---|---|---|
| `tui --config <client file>` | a full-screen view of a tenant's vCons | yes |

`tui` shows the vCons of one key's tenant in the terminal, read from a
running server over its HTTP API: the list, and each vCon's uuid, version
and times, never its content. Like `mcp` it reads a client file (`url`, the
server's address; `key_file`; and the optional `ca`) instead of the server's
configuration, opens no data directory and no master key, and locks its
memory. The release binaries, `cargo install` and a build from source carry
it (the cargo feature `tui`, on by default); a build made with
`--no-default-features` exits with code 1 and
`this build has no terminal UI`. It exits with code 0
when the user quits, and code 1 when the client file or the key file cannot
be used, or when standard input or standard output is not a terminal
(`a terminal is required`). See [the terminal UI](@/docs/tui.md).

## Running beside the server

Commands marked "no" refuse while `serve`, or any other command, holds the
data directory, with exit code 1 and `the data directory is in use; stop the
server and any other command first`. The others run beside the server, and
the server sees their changes at its next request: a revoked key is refused at
once. See [operations](@/docs/operations.md#one-server-per-directory).

## A first tenant, from nothing

```sh
sudo -u vcon vcon-server tenant create acme --config /etc/vcon-server/vcon-server.toml
# {"id":"3f0c…"}
sudo -u vcon vcon-server key create --operator --label ops --config /etc/vcon-server/vcon-server.toml
# {"id":"…","token":"vcs_…"}   keep this token: it manages tenants over HTTP
sudo -u vcon vcon-server key create --tenant 3f0c… --scope write --label recorder-1 \
  --config /etc/vcon-server/vcon-server.toml
# {"id":"…","token":"vcs_…"}   give this one to the producer
```
