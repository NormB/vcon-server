+++
title = "Durability"
description = "When an answered request is on disk: how the SQLite catalog syncs its commits, what a killed process and a power cut keep, what happens when a sync fails, and how to run the crash test."
weight = 48

[extra]
group = "concepts"
+++

A request that vcon-server answers is on disk before the answer leaves:
the change and its audit record are in one commit, and that commit is
synced to the disk. This page says how the default catalog, SQLite, does
that, what a crash keeps, and how to test it. The
[PostgreSQL catalog](@/docs/postgres-catalog.md) leaves the sync to
PostgreSQL's own commit.

## How a commit is synced

Each tenant has a catalog file, `catalog.sqlite`, in write-ahead log
(WAL) mode, and the server serves each tenant on one connection to it.

- **The commit.** A transaction's COMMIT writes its pages to the WAL file,
  `catalog.sqlite-wal`, without syncing it, and lets the connection go,
  so the tenant's next request can start.
- **The sync.** The commit then waits for one `fdatasync` of the WAL that
  began after its pages were written. One thread at a time calls it, for
  every commit written so far, and wakes each commit it covers: when
  several requests of a tenant commit at once, one sync covers them all.
- **The answer.** A request is answered only after that sync has
  returned. So is a request that only read: it waits until every commit
  it may have seen is synced, so it never answers from a commit that a
  power cut could still undo.
- **Files deleted after a commit.** An object file, a Parquet pack file
  or a manifest is deleted only after the commit that no longer needs it
  is synced, or after a read of that commit has waited for its sync.
- **Object files.** A vCon's object is written to a temporary file,
  synced, renamed and its directory synced before the commit that records
  it, as before.

The server catalog, `server.sqlite`, syncs inside each COMMIT
(SQLite's `synchronous = FULL`): its writes (tenants, keys, users) are
few.

Up to 0.4.16, each tenant commit synced the WAL inside COMMIT, holding
the tenant's connection: every request of the tenant waited for every
other request's sync, one at a time. The data on disk is the same: no
file, table or format changed, and a data directory moves between the
two in either direction.

Nothing configures this; there is no setting to turn the sync off.

## Commands beside the server

[Commands that run beside the server](@/docs/command-line.md), such as
`audit export` and `backup`, open the tenant's catalog with their own
connection, and can read a commit of the server before the server has
synced it. Each such command syncs the WAL itself after it reads and
before it writes any output, so it never writes out a commit that a power
cut could still undo. Its own commits sync inside COMMIT, so the server
never reads one of them unsynced.

## When a sync fails

A failed `fdatasync` means the kernel could not write some of the WAL's
pages: the commits it covered may or may not be on disk. Then:

- the requests waiting for that sync are answered
  [`internal`](@/problems/internal.md), as a commit whose outcome is not
  known is;
- every later request to that tenant's data is answered `internal` too,
  until the server is restarted;
- at the restart SQLite's recovery reads the WAL as the disk holds it,
  and keeps every commit whose pages are whole on disk.

The WAL file is opened once, when the tenant's catalog is opened, and
kept open, because Linux reports a failed writeback once to each file
open at the time: a file opened later is not told. A command beside the
server opens a WAL of its own, so a writeback error that the server's
sync was told of is not reported to it; the server's tenant is the one
that refuses requests.

## What a crash keeps

**A killed process** (SIGKILL, a crash, an out-of-memory kill) keeps
everything it had written to the kernel, synced or not: the kernel's page
cache survives the process. Every answered request is there after the
restart, and so is its audit record. The crash test below checks this.
On 2026-11-24, on the development host (an NVIDIA Jetson AGX Thor, NVMe
disk), one run of 200 kills, with 2 writers and 2 readers, took 306 s:
7,741 writes and 16,020 reads were answered, and after every kill each of
them was there with its record, and `check` reported no problem.

Since a killed process keeps the page cache, the crash test passes
whether or not the WAL is synced: it tests recovery, and that a change
and its record are kept or lost together. The order of the syncs is
tested in the server's own tests, through a model of the filesystem that
holds, fails and counts each sync.

**A power cut** keeps only what was synced. This cannot be tested on the
development host: a killed process keeps the page cache, and nothing
there drops it the way a power cut does. The argument instead, from
SQLite's source as vcon-server builds it (the SQLCipher copy in the
`libsqlite3-sys` 0.38.2 crate, `sqlcipher/sqlite3.c`):

1. With `synchronous = NORMAL`, a commit does not sync the WAL: the
   commit's sync is made only when `WAL_SYNC_FLAGS(sync_flags)` is not 0
   (line 71270), and NORMAL sets only the checkpoint's sync flags
   (`walSyncFlags = syncFlags<<2`, lines 62801 to 62803). vcon-server
   syncs the WAL itself before it answers.
2. SQLite syncs the WAL before a checkpoint copies its pages into the
   catalog file (line 69347), and syncs the WAL's header when the WAL
   starts over from its beginning (line 71193), both with the checkpoint's
   sync flags, which NORMAL keeps. So no page that a sync has not covered
   is copied into the catalog file, and no synced page is written over
   before the catalog file is synced.
3. At recovery, a WAL frame is valid only if its checksum covers the WAL
   header and every frame before it (line 68102): recovery keeps a prefix
   of the commits, in order.

So after a power cut the catalog holds a prefix of the commits that
includes every commit whose sync returned, which is every commit whose
request was answered.

## Run the crash test

`scripts/crash_run.py` starts a server on a data directory of its own,
runs writers and readers of one tenant, kills the server with SIGKILL at
a random moment, and checks what the restarted server has.

What you need first:

- a release build: `cargo build --release --locked -p vcon-server`;
- `python3` and `openssl` on the PATH;
- a directory for the run's data directory, given as `--parent` (the
  default is the system's temporary directory).

Steps:

1. Run 40 kills:

   ```sh
   python3 -I scripts/crash_run.py --server target/release/vcon-server \
     --kills 40 --out crash.json
   ```

2. Repeat it, or raise `--kills`, for more kills. `--seed` repeats a run's
   order of kills and writes; `--writers`, `--readers` and `--longest`
   (the most seconds a round runs before its kill) change the load.

To verify it, read the last line it prints and `crash.json`: it exits 0
and `problems` is empty when, after every kill:

- every write it was answered is there: each vCon's current body is the
  last answered one, or one sent after it that had no answer (that
  request may have committed before the kill), and a body never goes back
  to an older one;
- every version there has its `create` or `replace` audit record, and
  every read answered has a `read` record;
- [`vcon-server check`](@/docs/operations.md#checking-the-data) reports no
  problem.

It deletes every file it made, the data directory included, whatever the
outcome.

When it fails:

- **Exit 2** (`crash_run: ...` on standard error): the set-up or the
  server failed, not a check. `the server stopped` prints the server's
  last log lines; a missing `openssl` or a wrong `--server` path fails
  here too.
- **Exit 1**: each `PROBLEM` line names the uuid and what was lost or
  missing. Rerun with the same `--seed` to repeat the run, and keep the
  data directory by copying it from `--parent` while the run is going:
  the program deletes it at the end.
