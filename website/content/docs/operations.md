+++
title = "Operations"
description = "Running vcon-server day to day: the data directory, what may run beside the server, the purge sweep, logs, health checks and backups."
weight = 47

[extra]
group = "concepts"
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

## Objects in an S3 bucket

With `storage.objects = "s3"` (see
[configuration](@/docs/configuration.md#storage) and the how-to
[keep vCons in an S3 bucket](@/docs/s3-storage.md)), each
tenant's `objects/` directory is replaced by names in the bucket, under the
configured prefix:

```text
<prefix>tenants/<tenant id>/objects/<version id>
<prefix>vcon-server.claim
```

Each object holds the same sealed bytes the local file would. The
catalogs, `server.sqlite`, the lock files, `storage.claim` and the master
key stay in `data_dir`. At start, `serve` and every command but `mcp` and
`rekey master` read the secret key file, reach the bucket, read its
versioning, and check the claim; a
bucket with versioning on or suspended is refused unless
`storage.s3.allow_versioning` is set, because it keeps a removed object's
earlier version.

- Start-up recovery removes an object no catalog row names, and the objects
  of a tenant that has no row in `server.sqlite`. It does so only under a
  prefix this data directory claimed: a prefix that another data directory
  claimed, or that holds objects and no claim, is refused at start. It logs
  how many objects it removed (see [logs](#logs)).
- A purge removes the version's object from the bucket after its row is
  deleted. An object already missing does not stop the purge.
- `tenant remove` removes the tenant's objects from the bucket, a thousand
  at a time, then its directory.
- `check` reads every object, as it reads every local file: one request
  for each version.
- `backup` writes the local layout: it copies each object from the bucket
  into the backup's `objects/` directories. A server restored from it with
  no `[storage]` table keeps its objects as files. The backup holds no
  `storage.claim`, so a server restored from it with the same `[storage]`
  table refuses the prefix; its recovery would otherwise remove the objects
  of versions stored after the backup. See
  [back up and restore](@/docs/backup-and-restore.md#objects-in-an-s3-bucket).

The bucket's operator sees object names (random ids), sizes and times; no
object can be opened without the tenant key, which stays wrapped under the
master key in `server.sqlite`. The account the server uses needs to get,
put, delete and list objects in the bucket and read its versioning;
nothing else.

## One server per directory

`serve` takes `lock` exclusively and holds it while it runs. A second server
on the same directory refuses to start: two servers would each run start-up
recovery, and one could remove a write the other has under way.

The commands that change the registry wholesale run alone: `tenant create`,
`tenant remove`, `rekey tenant`, `rekey master` and `check` refuse while the
server or any other command runs. The rest (`tenant list`,
`tenant set-retention`, the `key` commands, `audit export` and `backup`) run
beside the server, but never beside a command that runs alone, so nothing they write can
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

- `purged` is the number of versions purged by this sweep, over every
  tenant. Each one is also in its tenant's audit log, as `purge` by actor
  `sweep`.
- `failed` counts tenants that could not be swept, such as one being removed,
  one whose retention period does not read back, or one whose purge failed
  part way; a failure to list the tenants counts as one. They are tried
  again at the next sweep. A `failed` that stays above zero needs a look:
  with the objects in an S3 bucket, a bucket that cannot be reached is one
  cause.

## Logs

Structured lines on standard error, so systemd's journal or any collector can
take them:

- `serving` with the address it listens on once it listens; with port 0 in
  `listen`, this names the port the system chose;
- one `request` line per request: `request_id`, `method`, `route` (the route
  template, never the path), `status`, `duration_ms`;
- `purge sweep` lines (see [the purge sweep](#the-purge-sweep));
- `start-up recovery removed objects that no catalog row names`, with
  `removed`, the number of objects removed, when a tenant's start-up
  recovery removed any (a crash's leftovers; see [stopping](#stopping)):

  ```text
  2026-11-06T03:25:31.882438Z  INFO vcon_server::store: start-up recovery removed objects that no catalog row names removed=2
  ```

- `a stored version's object could not be read`, a `WARN` line with the
  failure's `kind` and, for a file, `os_error`, the system's error number,
  when a read's object file or S3 object could not be read for a reason
  other than its absence; the request answers
  [internal](@/problems/internal.md). The line holds no text of the error,
  since a service's message may name the object:

  ```text
  2026-11-06T04:36:06.578959Z  WARN vcon_server::store: a stored version's object could not be read kind=permission denied
  ```

- a `WARN` line, at start and then daily, while key rotation is overdue,
  with counts only: `tenants_with_purges`, the tenants with purges since
  their key was last rotated; `purges`, those purges; and `removals`, the
  tenants removed since the master key was last rotated. It is overdue
  when the oldest of those is older than `rotation_warn`
  ([configuration](@/docs/configuration.md)). [Key rotation](#key-rotation)
  ends it:

  ```text
  2026-11-06T03:25:00.132913Z  WARN vcon_server::serve: key rotation overdue: purged vCons and removed tenants stay readable on old copies until it tenants_with_purges=1 purges=1 removals=0
  ```

  When the backlog cannot be read, the line is
  `WARN vcon_server::serve: the key rotation backlog could not be read`;
  run `vcon-server check` with the server stopped to see why;
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
prints one JSON line per tenant and a summary line. It reports damage,
catalogs that do not open, objects the storage did not read, leftover
files, versions past due for purge and work waiting for key rotation, and
exits 1 when anything needs the operator. The restore, upgrade and key
rotation guides each run it before the server is started again. The
[command line](@/docs/command-line.md#checking) page says what each member
of the report means.

## Key rotation

A purge erases a version from the live system, but a copy made earlier (an
old disk, freed blocks, a backup) still holds its sealed file and wrapped
key. Rotation makes those copies unreadable, by destroying the keys that
open them. Run the `vcon-server` commands as the service account, with the
server stopped.

The steps, with every command and what it prints, are on
[revoke and rotate keys](@/docs/rotate-keys.md#rotate-the-tenant-key-and-the-master-key):
stop the server; rotate the key of each tenant that `check` lists with
purges since its key was last rotated; make a new master key file and
rotate the master key to it; point `master_key` in the configuration at the
new file; back up the new master key and the data directory; start the
server; and destroy every copy of the old master key, backups included.

Once every copy of the old master key is destroyed, the data purged before
the rotation cannot be read from any copy taken before it. A step that
stops part way is finished by running it again. The test suite plays this
procedure end to end with the binary
(`crates/vcon-server/tests/rotation.rs`): it copies the data directory,
purges a vCon, rotates, destroys the old master key, and then tries every
key that still exists on the copy.

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
that does not hold the data backups, and label the copy with the key's id,
the first 16 hexadecimal digits of its SHA-256. Keep a list of every copy,
so that each one can be found and destroyed at the next rotation.
[Back up and restore](@/docs/backup-and-restore.md#back-up-the-master-key)
has the commands.

### Back up the data directory

`backup` writes a copy of the data directory to a new directory while the
server runs, as the service account, and prints one line with the numbers
of tenants, tries and versions.
[Back up and restore](@/docs/backup-and-restore.md#back-up-the-data-directory)
has the commands and what they print.

It copies `server.sqlite`, then each tenant's catalog, with SQLite's online
backup API, so each copy is its catalog as one commit left it; then it copies
every object file a copied catalog names. Requests and the purge sweep go on
meanwhile. Tenants are copied one after another, each as it was when its
catalog was copied. A version purged, or a tenant removed, while it is being
copied makes `backup` copy that part again; after 5 tries it exits 1 with
`tenants were removed or versions purged during each try; try again`. Every
directory it writes is mode `0700` and every file `0600`, whatever the
umask it runs under, and every file is synced before it exits 0. The admin log records each run as
`backup`, with its outcome. The copy holds nothing readable without the
master key, and it does not hold the master key: store it away from the
master key's copies. A copy whose last try ended with exit 1 is not whole;
remove it.

### Or copy it with the server stopped

With the server stopped, the data directory can be copied with `tar`, while
`flock` holds its two lock files so that no server or command starts during
the copy. A server that stopped cleanly leaves no `-wal` file; the archive
holds `server.sqlite`, `lock`, `lock.commands` and `tenants/`, and nothing
in it is readable without the master key.
[Back up and restore](@/docs/backup-and-restore.md#copy-it-with-the-server-stopped)
has the commands.

Do not copy a running server's files with `tar` or `cp`. Each file is read
at a different moment, and a catalog copied in the middle of a transaction
"might contain some old and some new content, and thus be corrupt"
([SQLite](https://www.sqlite.org/howtocorrupt.html#_backup_or_restore_while_a_transaction_is_active)).
`backup` reads them consistently.

### Or take a filesystem snapshot

To back up while the server runs without `backup`, take an atomic snapshot of the filesystem
that holds the data directory (LVM, ZFS, btrfs). A snapshot holds what a
power cut at that moment would leave on disk, and start-up recovery opens
that state (see [stopping](#stopping)). Copy the data directory out of the snapshot, and
leave the master key out of that copy when the key is on the same
filesystem.

### Restore the data directory

Restore it with the server stopped, under the master key that was current
when the backup was taken, then run `check` on it before starting. Copied
as root with `cp -a` or `tar -p`, the files keep their owner and modes. A
directory `backup` wrote holds no lock files; the server and the commands
make them. Under any other master key, `check` and the server both refuse,
with `the master key is not the one server.sqlite was made under`.
[Back up and restore](@/docs/backup-and-restore.md#restore-the-data-directory)
has the commands.

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
taken. Once every copy of that key is destroyed (the last step of a
[key rotation](#key-rotation)), the backup can no longer be read or
restored, and that includes every vCon purged before the rotation. So:

- take a new data backup, and back up the new master key, right after a
  rotation;
- a backup taken before the rotation cannot be restored after it.
