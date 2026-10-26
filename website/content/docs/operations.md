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
open them. Run the `vcon-server` commands as the service account, with the
server stopped.

1. Stop the server:

   ```sh
   sudo systemctl stop vcon-server
   ```

2. List the tenants with purges since their key was last rotated:

   ```sh
   sudo -u vcon vcon-server check --config /etc/vcon-server/vcon-server.toml | jq -r 'select(.purges_awaiting_rotation > 0) | .tenant'
   ```

3. Rotate the key of each tenant it lists:

   ```sh
   sudo -u vcon vcon-server rekey tenant <tenant id> --config /etc/vcon-server/vcon-server.toml
   ```

4. Make a new master key file, under a name not used before, and rotate
   the master key to it:

   ```sh
   head -c 32 /dev/urandom | sudo -u vcon tee /etc/vcon-server/master-2.key >/dev/null
   sudo chmod 0400 /etc/vcon-server/master-2.key
   sudo -u vcon vcon-server rekey master --new-master /etc/vcon-server/master-2.key \
     --config /etc/vcon-server/vcon-server.toml
   ```

   It prints the new key's id and the next steps:

   ```text
   {"master_key_id":"2a21883e94360e22","next":"point master_key in the configuration at the new key file, start the server, then destroy every copy of the old master key"}
   ```

5. Point `master_key` in the configuration at the new file:

   ```sh
   sudo sed -i 's|^master_key = .*|master_key = "/etc/vcon-server/master-2.key"|' /etc/vcon-server/vcon-server.toml
   ```

6. While the server is still stopped, back up the new master key and the
   data directory (see [back up and restore](#back-up-and-restore)).
7. Start the server:

   ```sh
   sudo systemctl start vcon-server
   ```

8. Destroy every copy of the old master key, backups included:

   ```sh
   sudo rm /etc/vcon-server/master.key
   ```

From step 8 on, the data purged before the rotation cannot be read from any
copy taken before it. A step that stops part way is finished by running it
again. The test suite plays this procedure end to end with the binary
(`crates/vcon-server/tests/rotation.rs`): it copies the data directory, purges
a vCon, rotates, destroys the old master key, and then tries every key that
still exists on the copy.

## Back up and restore

A backup is two things, kept in different places:

| What | Where | Changes |
|---|---|---|
| The data directory | `data_dir`, such as `/var/lib/vcon-server` | with every request |
| The master key file | `master_key`, such as `/etc/vcon-server/master.key` | only at a [key rotation](#key-rotation) |

Data without its master key is unreadable. Kept apart, the copies of an old
master key can be destroyed at a rotation while the data backups stay where
they are, and those backups become unreadable. A backup that holds both
opens everything it holds until it is destroyed.

### Back up the master key

Back it up when it is made and after each rotation. Copy the file to storage
that does not hold the data backups, and label the copy with the key's id:

```sh
sudo sha256sum /etc/vcon-server/master.key | cut -c1-16
```

Keep a list of every copy, so that each one can be found and destroyed at
the next rotation.

### Back up the data directory

Copy it with the server stopped. The copy below holds the data directory's
two lock files, `lock` and `lock.commands`, as a command that runs alone does,
so no server or other command can start while it runs:

```sh
sudo systemctl stop vcon-server
sudo flock --nonblock /var/lib/vcon-server/lock flock --nonblock /var/lib/vcon-server/lock.commands \
  tar -C /var/lib -cf /backup/vcon-server-data-$(date -u +%Y%m%dT%H%M%SZ).tar vcon-server
sudo systemctl start vcon-server
```

If a server or a command still holds the directory, `flock` exits 1 and
nothing is copied. A server that stopped cleanly leaves no `-wal` file; the
archive holds `server.sqlite`, `lock`, `lock.commands` and `tenants/`, and
nothing in it is readable without the master key. Store it away from the
master key's copies.

Do not copy the files of a running server. Each file is read at a different
moment, and a catalog copied in the middle of a transaction "might contain
some old and some new content, and thus be corrupt"
([SQLite](https://www.sqlite.org/howtocorrupt.html#_backup_or_restore_while_a_transaction_is_active)).
vcon-server has no command that copies a running server's data
consistently.

To back up while the server runs, take an atomic snapshot of the filesystem
that holds the data directory (LVM, ZFS, btrfs). A snapshot holds what a
power cut at that moment would leave on disk, and start-up recovery opens
that state (see [stopping](#stopping)). Copy the data directory out of the snapshot, and
leave the master key out of that copy when the key is on the same
filesystem.

### Restore the data directory

Restore it with the server stopped, under the master key that was current
when the backup was taken, then check it before starting:

```sh
sudo systemctl stop vcon-server
sudo mv /var/lib/vcon-server /var/lib/vcon-server.old
sudo tar -C /var/lib -xpf /backup/vcon-server-data-<time>.tar
sudo -u vcon vcon-server check --config /etc/vcon-server/vcon-server.toml
sudo systemctl start vcon-server
```

`tar -p`, run as root, restores the files' owner and modes. Under any other
master key, `check` and the server both refuse:

```text
vcon-server: the master key is not the one server.sqlite was made under
```

The data directory is then as it was when the backup was taken:

- vCons, tenants and keys made since are gone, with their audit records;
- a vCon deleted or purged since is back;
- **a key revoked since is valid again.** Revoke it again with
  `vcon-server key revoke <id>`. The admin log records each revocation as
  `key_revoke`; an `audit export --admin` taken after the backup lists them.

Versions in the backup that are due for purge are purged again by the sweep
at start.

### What a key rotation does to backups

A data backup opens only under the master key that was current when it was
taken. Once every copy of that key is destroyed (step 8 of
[key rotation](#key-rotation)), the backup can no longer be read or
restored, and that includes every vCon purged before the rotation. So:

- take a new data backup, and back up the new master key, right after a
  rotation;
- a backup taken before the rotation cannot be restored after it.
