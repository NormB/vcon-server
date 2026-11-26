+++
title = "On-disk formats"
description = "Every format vcon-server writes to disk or to a bucket, with its version; what each release since 0.4.0 wrote; the rule a new build keeps for the data of an older one; and how a release's data is kept in the repository and tested."
weight = 58

[extra]
group = "reference"
+++

vcon-server keeps its data in a few formats. Each one carries a version
number. This page lists them, the versions each release wrote, and the rule
every build keeps. To move a server to a newer release, see
[upgrade](@/docs/upgrading.md).

## The rule

- A build opens the data directory of every release since 0.4.0. It
  brings each catalog up to its own version the first time it opens it,
  each in one transaction, so a migration that fails leaves the old
  version whole. A PostgreSQL catalog is brought up to date by
  `vcon-server migrate`, never by a server at start.
- A build refuses a version newer than its own, and names it: `schema
  version 11 is not one this build knows` for a catalog in the data
  directory, `the database schema is newer than this binary` for
  PostgreSQL. So an older binary cannot open data that a newer one has
  opened, and the way back from an upgrade is a copy taken before it.
- A change to a format raises its version. The same change adds the
  migration, the version's row on this page and a CHANGELOG entry, and the
  next release adds its own data to the repository (see
  [the data of each release](#the-data-of-each-release)).

The rule covers 0.4.0 and later. Release 0.3.0 and earlier are not tested.

## The formats

| Format | What it is | Version | Where |
|---|---|---|---|
| `catalog` | The SQLite catalogs, encrypted with SQLCipher: `server.sqlite` (tenants, keys, users, sessions and its audit log) and each tenant's `catalog.sqlite` (its vCons, versions, tombstones and audit log). The version is SQLite's `user_version`. | 10 | the data directory |
| `postgres` | The schema of the [PostgreSQL catalog](@/docs/postgres-catalog.md), a preview, in place of the catalogs above. | 15 | the database |
| `envelope` | The first byte of every sealed blob (an object, a wrapped key, and in PostgreSQL a sealed column): AES-256-GCM with a 12-byte nonce and a 16-byte tag. | 1 | objects and catalogs |
| `key-wrap` | The first byte of a key that a key backend other than the master key file wrapped: the backend's kind, the reference of its key, and the backend's ciphertext. This release writes none: the command line moves keys to a master key file only, and the other backends come in later releases. | 1 | none yet |
| `parquet-storage` | The `format` of each manifest of [Parquet storage](@/docs/parquet-storage.md) in a bucket. | vcon-server.parquet-storage/1 | the bucket |
| `export-manifest` | The `format` of the manifest that [`export parquet`](@/docs/export-parquet.md) writes. | 1 | the export directory |
| `export-columns` | The `vcon-server.schema` value in the key-value metadata of each Parquet file `export parquet` writes: the columns. | 1 | the export directory |

The server reads back the first five. The two export formats are written
for other programs, such as DuckDB, and the server never reads them.

The key of each catalog is HKDF-SHA256 of its parent key, with no salt and
the info `vcon-server/v1 catalog`: the master key's for `server.sqlite`, the
tenant key's for a tenant's catalog. [Security](@/docs/security.md)
describes the key hierarchy.

## What each release wrote

| Release | `catalog` | `postgres` | `envelope` | `key-wrap` | `parquet-storage` | `export-manifest` | `export-columns` |
|---|---|---|---|---|---|---|---|
| 0.4.0 | 4 | none | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.1 | 4 | none | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.2 | 4 | none | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.3 | 4 | none | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.4 | 9 | none | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.5 | 9 | none | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.6 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.7 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.8 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.9 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.10 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.11 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.12 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.13 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.14 | 9 | 13 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.15 | 9 | 14 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.16 | 9 | 14 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.17 | 10 | 14 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |
| 0.4.18 | 10 | 15 | 1 | none | vcon-server.parquet-storage/1 | 1 | 1 |

`none` under `key-wrap`: the release has no such format. `none` under
`postgres`: the release binary has no PostgreSQL catalog. The `postgres`
feature, a preview, is in the release binaries from 0.4.6 on. A build from
source with that feature made schema version 1 at 0.4.2 and 0.4.3, and 6 at
0.4.4 and 0.4.5.

## The data of each release

The repository keeps a data directory written by each release's own binary,
under
[crates/vcon-server/tests/data/formats](https://github.com/NormB/vcon-server/tree/main/crates/vcon-server/tests/data/formats),
one directory per release. Each holds:

- `data/`: the data directory as the release left it;
- `master.key`: its master key, the SHA-256 of a fixed text and the release
  number. The key protects test data only;
- `manifest.json`: every version kept, with the SHA-256 of its body; every
  uuid purged; and what made the directory (the release archive's sha256,
  the binary's sha256, the container image).

The workload is the same for each release: two tenants, one with the
default retention period and one with `0d`, ten vCons each; replaced vCons
(hidden versions), deleted vCons (kept under the default period, purged in
the request under `0d`), legal holds on a live, a deleted and a replaced
vCon, a restart whose sweep purges what is due, and the audit records of
every step. From 0.4.4, which added users, it also makes two users with
memberships. The sizes measured on 2026-11-17: 246,084 bytes for each of
0.4.0 to 0.4.3, and 299,479 bytes for each of 0.4.4 to 0.4.9; on
2026-11-18, 299,480 bytes for 0.4.10; on 2026-11-19, 299,480 bytes for
0.4.11; on 2026-11-21, 299,480 bytes for each of 0.4.12 and 0.4.13; on
2026-11-24, 299,480 bytes for each of 0.4.14 and 0.4.15; on 2026-11-25,
299,480 bytes for each of 0.4.16 and 0.4.17; on 2026-11-26, 299,480 bytes
for 0.4.18.

The test
[formats.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/tests/formats.rs)
runs on every change, against a copy of each release's data. For each
release:

- `check` passes, and counts each tenant's kept versions;
- every kept version reads back byte-equal to the body the release stored,
  in the order the release kept them, and each purged uuid stays gone;
- `server.sqlite` is at this build's version after the first open, and a
  second open changes no file;
- `server.sqlite` set to one version past this build is refused by name,
  and the data is left as it was; the wrong master key changes nothing;
- for 0.4.0 to 0.4.3, a migration made to fail at its last step leaves
  version 4 whole, and opens once the cause is gone;
- every object starts with the `envelope` version this page states, and
  the `catalog` version this page states is the one the release wrote.

The test also holds this page to the list of formats in the code
([formats.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/src/formats.rs)).

## The data in PostgreSQL and Parquet storage

Each release from 0.4.6, the first built with the `postgres` feature, also
has a deployment with its catalogs in PostgreSQL and its objects in an S3
bucket with Parquet storage on, under
[crates/vcon-server/tests/data/formats-postgres](https://github.com/NormB/vcon-server/tree/main/crates/vcon-server/tests/data/formats-postgres).
The release's own binary wrote it against PostgreSQL 16.15 and RustFS
1.0.4, the images of CI's `backends-postgres` job. Each directory holds:

- `dump.sql`: `pg_dump --inserts` of the database, made by PostgreSQL
  16, the oldest major CI runs, so that it loads into 16 and 18 alike;
- `bucket/`: every object the bucket held, by its key: the Parquet files
  of the pack run at start, the versions stored after it as objects, the
  storage manifests and the claim;
- `data/`: the lock files; `master.key`, made as above;
- `manifest.json`: every version kept, with the SHA-256 of its body and
  where it is (a Parquet file or an object); every uuid purged; the schema
  version the release wrote; and what made the directory.

The workload is the one above, then a restart with Parquet storage on,
whose pack run packs every object, then two more vCons in each tenant,
stored as objects. The sizes measured on 2026-11-17: 159,490 to 159,500
bytes for each of 0.4.6 to 0.4.9; on 2026-11-18, 159,657 bytes for 0.4.10;
on 2026-11-19, 159,497 bytes for 0.4.11; on 2026-11-21, 159,489 bytes for
0.4.12 and 159,494 bytes for 0.4.13; on 2026-11-24, 159,494 bytes for
0.4.14 and 160,747 bytes for 0.4.15; on 2026-11-25, 160,906 bytes for
0.4.16 and 160,751 bytes for 0.4.17; on 2026-11-26, 163,649 bytes for
0.4.18.

On every change, the same test checks each release's files without a
database: the dump has no psql meta-command, every object starts with the
`envelope` version, every Parquet file starts and ends with Parquet's
magic bytes, and each
storage manifest states the `parquet-storage` format this page states.
In CI's `backends-postgres` job, on PostgreSQL 16 and 18, it loads each
dump into a database of its own and the objects into a bucket of its own,
and:

- `check` passes; every kept version, packed or an object, reads back
  byte-equal, in the order the release kept them, and each purged uuid
  stays gone;
- `migrate` finds the schema current and changes nothing; a schema set to
  one version past this build is refused by name; the wrong master key
  opens nothing;
- the restored deployment stores and reads a new vCon, a missing object is
  named damaged by `check`, and `serve` without the `[storage.parquet]`
  table is refused;
- held versions outlive the start-up sweep under a `0d` period, and the
  pack run at start packs the late objects, each still read back.

## Add the data of a new release

After each release, add its data, so every later build opens it.

Before you start, you need:

- Docker, and the container image the script names (`debian:trixie-slim`,
  pinned by its digest in the script);
- `openssl` and Python 3 on the host;
- the release's archives pinned in `scripts/docs-test-pins.txt`.

Steps:

1. Add the release to `RELEASES` in `scripts/make-format-fixtures.py`.
2. Write its data. The script downloads the pinned archive for this host's
   architecture through `scripts/fetch-pinned.sh`, which checks its
   sha256, and runs the binary in throw-away containers:

   ```sh
   python3 -I scripts/make-format-fixtures.py --cache ~/.cache/vcon-format-fixtures --release 0.4.9
   ```

3. Write its PostgreSQL deployment. The script starts the servers of
   `scripts/pg-test-server.sh` and a RustFS container, and removes them
   after:

   ```sh
   python3 -I scripts/make-format-fixtures.py --cache ~/.cache/vcon-format-fixtures --postgres --release 0.4.9
   ```

4. Add a row for the release to the table above, and the lines
   `release!(release_0_4_9, "0.4.9");` and
   `pg_release!(pg_release_0_4_9, "0.4.9");` to `tests/formats.rs` (with
   `older` when it wrote an older `catalog` version than the build).
5. Commit the new directories with the page and the test.

Verify: `cargo test -p vcon-server --test formats` passes; with the
servers of `scripts/pg-test-server.sh` and an S3 store named in the
`VCON_TEST_S3_*` variables, so does
`cargo test -p vcon-server --features postgres,backend-tests --test formats postgres::`.
The script prints the size of each release's data; state it in the pull
request.

When it does not work:

- `0 pins name /v0.4.9/vcon-server-0.4.9-...`: the release's archives are
  not in `scripts/docs-test-pins.txt` yet. Add them first.
- `serve exited 1`: the release refused the configuration or the data
  directory. The message ends with the server's last log lines.
- A test names the release and a version: the page's row states another
  version than the release wrote. Correct the row; never edit the data.
- `the pack run left N objects`: Parquet storage did not pack every object
  in the time a start is given. The server's log lines say why.
- `--postgres takes only ...`: the release is older than 0.4.6, which has
  no `postgres` feature.
