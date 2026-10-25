+++
title = "Operations"
description = "Running vcon-server day to day: the data directory, what may run beside the server, the purge sweep, logs, health checks and backups."
weight = 80
+++

## The data directory

```text
data_dir/
├── lock                  held by the server (and by commands that run alone)
├── lock.commands         shared by commands that run beside the server
├── server.sqlite         tenants, keys, wrapped tenant keys, the admin log
└── tenants/
    └── <tenant id>/
        ├── catalog.sqlite    versions, wrapped data keys, the tenant's audit log
        └── objects/
            └── <version id>  one sealed file per stored version
```

The catalogs are SQLCipher databases in WAL mode, so `-wal` and `-shm` files
sit beside them while the server runs. Nothing in the directory is readable
without the master key. Do not edit or move files by hand: start-up recovery
treats an object file with no catalog row as a crash's leftover and removes
it.

## One server per directory

`serve` takes `lock` exclusively and holds it while it runs. A second server
on the same directory refuses to start: two servers would each run start-up
recovery, and one could remove a write the other has under way.

The commands that change the registry wholesale run alone: `tenant create`,
`tenant remove`, `rekey tenant`, `rekey master` and `check` refuse while the
server or any other command runs. The rest (`tenant list`,
`tenant set-retention`, the `key` commands and `audit export`) run beside the
server, but never beside a command that runs alone, so nothing they write can
be lost to a rewrite. Commands that run beside the server open the data
without start-up recovery, so they never mistake the server's work in progress
for a crash's leftovers. The lock rules are built; the commands are
[being built](@/docs/status.md). While the server runs, the admin API does the
tenant work instead.

## The purge sweep

At start, then `purge_interval` seconds after each sweep ends, every tenant's
due versions are purged (see [retention](@/docs/retention-and-holds.md)). Each
sweep logs one line with counts only:

```text
2026-10-25T15:44:58.372109Z  INFO vcon_server::serve: purge sweep purged=0 failed=0
```

`failed` counts tenants that could not be swept, such as one being removed;
they are tried again at the next sweep. A `failed` that stays above zero
needs a look.

## Logs

Structured lines on standard error, so systemd's journal or any collector can
take them:

- `serving` with the listen address once the server listens;
- one `request` line per request: `request_id`, `method`, `route` (the route
  template, never the path), `status`, `duration_ms`;
- `purge sweep` lines;
- `stopped` after SIGTERM or SIGINT.

A server started, asked for its health with no key, and stopped:

```text
2026-10-25T15:44:58.370371Z  INFO vcon_server::serve: serving listen=127.0.0.1:18443
2026-10-25T15:44:58.372109Z  INFO vcon_server::serve: purge sweep purged=0 failed=0
2026-10-25T15:45:00.315964Z  INFO vcon_server::api: request request_id=15280239425010719166 method=GET route=/health status=401 duration_ms=5
2026-10-25T15:45:00.372646Z  INFO vcon_server::serve: stopped
```

No line holds a uuid, a tenant id, a token or any content. Who did what is in
the [audit logs](@/docs/audit-logs.md), inside the encrypted catalogs.

## Health checks

`GET /health` with any valid key answers `200` while `server.sqlite` answers,
and `503` otherwise. Give the monitor its own tenant key with no use beyond
that (a key needs at least one scope; give it `read` on a tenant that holds
nothing). Health requests are counted in the audit log once per minute per
key and status, not one record each.

## Stopping

SIGTERM or SIGINT stops the server, which logs `stopped` and exits.
Every write is crash-safe, so a kill or a power cut loses at most the requests
not yet answered; start-up recovery removes what they left.

## Backups

- **Back up the master key separately** from the data, and keep it as
  carefully as the key itself. Data without its master key is unreadable;
  that is the point.
- **Back up the data directory consistently:** stop the server and copy it, or
  take an atomic snapshot of the whole filesystem it lives on (LVM, ZFS,
  btrfs). Copying the files of a running server can catch a catalog in the
  middle of a write.
- **A restored copy needs the master key it was made under**; the server
  refuses any other.
- **A backup keeps what was purged after it was taken**, readable with that
  master key. Key rotation ([being built](@/docs/status.md)) is what makes
  such copies unreadable: after a rotation, destroy every copy of the old
  master key.
