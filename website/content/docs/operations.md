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
for a crash's leftovers. See the [command line](@/docs/command-line.md). While
the server runs, the admin API does the tenant work instead.

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
- a `WARN` line, at start and then daily, while key rotation is overdue,
  with counts only (`tenants_with_purges`, `purges`, `removals`);
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

## Checking the data

`vcon-server check`, with the server stopped, opens every stored version and
prints one JSON line per tenant and a summary line. A tenant's line gives:

- `versions` kept, and `damaged`: the versions whose sealed file is missing or
  does not open;
- `leftovers`: files a crash left that no row names (the next start removes
  them; `check` only reports them);
- `past_due` and `oldest_past_due`: versions due for purge, and since when;
- `purges_awaiting_rotation` and `oldest_purge_awaiting_rotation`: purges
  since the tenant's key was last rotated;
- `rotation_stopped`: a `rekey tenant` that stopped part way;
- `problems`: what needs the operator.

The summary gives `removed_tenants_awaiting_master_rotation` and the total
`problems`. A problem is damage, a catalog that does not open, a stopped
rotation, or a version that has waited longer than `purge_interval` (a sweep
that is not keeping up). Leftovers and work waiting for rotation are reported
but are not problems. `check` exits 1, after printing its report, when it
found a problem.

## Key rotation

A purge erases a version from the live system, but a copy made earlier (an
old disk, freed blocks, a backup) still holds its sealed file and wrapped
key. Rotation makes those copies unreadable, by destroying the keys that
open them:

1. Stop the server.
2. Run `vcon-server check`: each tenant's line gives
   `purges_awaiting_rotation`. Run `vcon-server rekey tenant <id>` for each
   tenant where it is above zero.
3. Make a new master key file and run
   `vcon-server rekey master --new-master <file>`.
4. Point `master_key` in the configuration at the new file and start the
   server.
5. Destroy every copy of the old master key, backups included.

From step 5 on, the data purged before the rotation cannot be read from any
copy taken before it. A step that stops part way is finished by running it
again. The test suite plays this procedure end to end with the binary
(`crates/vcon-server/tests/rotation.rs`): it copies the data directory, purges
a vCon, rotates, destroys the old master key, and then tries every key that
still exists on the copy.

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
  master key. [Key rotation](#key-rotation) is what makes such copies
  unreadable: after a rotation, destroy every copy of the old master key.
