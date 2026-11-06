+++
title = "Export to Parquet"
description = "Write a copy of a tenant's vCons as Parquet files, to a directory or an S3 bucket, beside a running server: choose which clear columns the files carry, read them with DuckDB, check the export, and see what each refusal means."
weight = 12
+++

`vcon-server export parquet` writes a copy of one tenant's kept versions,
current and hidden, as [Apache Parquet](https://parquet.apache.org/) files,
for analytics tools such as DuckDB, Athena or Spark. It runs beside a
serving server and reads the store without changing it; it adds one
`export` record to the tenant's [audit log](@/docs/audit-logs.md).

Each row holds one version's sealed object, byte for byte, in the column
`sealed`. That is ciphertext: only the server, with the tenant's keys, can
open it. Beside it, the files hold clear columns, as the tenant's metadata
level says: by default only the version id and the time it was stored.

An export is a copy as of the moment it starts. A version purged later stays
in every export made before the purge. Its sealed body can no longer be
opened, because the purge destroys its data key, but its clear columns stay
readable. Remove old exports on the tenant's retention schedule, and keep
them where only the people who may read the tenant's records can reach them.

**The export is on `main` and in no release yet.** Release 0.3.0 has no
`export` command. Until the next release, build the server from `main`, as
[build from source](@/docs/install.md#build-from-source) shows.

CI runs this page from top to bottom on each change: after
[getting started](@/docs/getting-started.md) in a Debian 13 container under
systemd, and after [keep vCons in an S3 bucket](@/docs/s3-storage.md) with
RustFS, where it also runs the steps for a bucket.

## Before you start

- **A server set up as [getting started](@/docs/getting-started.md) shows.**
  The steps use its shell variables (`$TENANT`, `$RECORDER`, `$OPS`) and its
  files in `/etc/vcon-server`.
- **For a directory:** a directory on the server's host that the service
  account may write. The steps make `/var/lib/vcon-export`.
- **For a bucket:** an S3-compatible bucket whose versioning has never been
  turned on and that has no Object Lock, so an export removed from it is
  gone. The export reaches it with `[export.s3]` in the configuration, or,
  without that table, with `[storage.s3]`'s service and keys (see
  [configuration](@/docs/configuration.md#keys)). Its key needs
  `s3:PutObject`, `s3:ListBucket` and `s3:DeleteObject` on the bucket: the
  export lists the manifests to number its own, writes each file only if its
  name is free (`If-None-Match: *`), and removes its files when it fails.
- **To read the files:** the DuckDB command-line program, installed below,
  and `unzip` to unpack it. On Debian or Ubuntu:

  <!-- docs-test: only apt -->
  ```sh
  sudo apt-get install -y -qq unzip >/dev/null
  ```

## Store vCons to export

Store two vCons with the write key:

```sh
for tel in +15555550101 +15555550102; do
  n=${tel: -1}
  printf '{"vcon":"0.4.0","uuid":"%s","created_at":"2026-11-05T12:00:00Z","subject":"Export example %s","parties":[{"tel":"%s","name":"Caller Example"}],"dialog":[],"attachments":[],"analysis":[]}' \
    "$(cat /proc/sys/kernel/random/uuid)" "$n" "$tel" > "export-$n.vcon"
  curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/vcons \
    -H "Authorization: Bearer $RECORDER" -H "Content-Type: application/vcon" --data-binary "@export-$n.vcon"
done
```

```text
201
201
```

## Choose the metadata level

The tenant's metadata level says which clear columns its files carry. Each
level adds to the one before:

| Level | Columns | What they show |
|---|---|---|
| `none` (default) | `version`, `stored_at`, `sealed` | a random version id, the time the server stored the version, and the sealed body's length |
| `counts` | also `form`, `gzip`, `size`, `created_at`, `updated_at`, `party_count`, `dialog_count` | how the vCon arrived, its own times, and how many parties and dialogs it has |
| `full` | also `uuid`, `subject`, `parties` | the vCon's uuid, its subject, and each party's `tel`, `mailto` and `name`: personal data, in clear |

An encrypted vCon shows only `form`, `gzip`, `size` and, at `full`, `uuid`:
its other columns are empty (NULL). The level applies to the next export; a
file already written keeps the level it was written at, which it records in
its own metadata.

Set acme's level to `counts`:

```sh
sudo -u vcon vcon-server tenant set-metadata "$TENANT" counts --config /etc/vcon-server/vcon-server.toml | jq -c '{metadata}'
```

```text
{"metadata":"counts"}
```

An operator key sets it over HTTPS too:

```sh
curl -sS --cacert /usr/local/share/vcon-server/ca.pem -X PUT "https://vcon.example.com:8443/admin/tenants/$TENANT/metadata" \
  -H "Authorization: Bearer $OPS" -H "Content-Type: application/json" --data '{"metadata":"counts"}' | jq -c '{metadata}'
```

```text
{"metadata":"counts"}
```

Each change is recorded in the admin log, as `tenant_metadata`, and in the
tenant's log, as `metadata`.

## Export to a directory

Make the directory, owned by the service account, and export acme to it:

```sh
sudo install -d -m 0700 -o vcon -g vcon /var/lib/vcon-export
sudo -u vcon vcon-server export parquet --tenant "$TENANT" --to /var/lib/vcon-export \
  --config /etc/vcon-server/vcon-server.toml | jq -c '{metadata, seq, files, rows, skipped, unread}'
```

```text
{"metadata":"counts","seq":1,"files":1,"rows":2,"skipped":0,"unread":0}
```

The line also gives `tenant`, `to`, `manifest` (the manifest's path under
the target) and `bytes`. `skipped` counts versions purged while the export
ran, and `unread` versions whose body could not be read for the metadata
columns, which are then empty.

The files go under the tenant's id, one directory per store date, in UTC,
and the manifest names them:

```sh
sudo find /var/lib/vcon-export -type f | sort
```

```text
/var/lib/vcon-export/tenant=<tenant id>/date=<yyyy-mm-dd>/part-<file id>.parquet
/var/lib/vcon-export/tenant=<tenant id>/manifest/00000000000000000001.json
```

Each file is written under a temporary name, synced, and renamed to its own
name only once whole. The manifest is written last, once every file it
names is in place. A file under its own name is always whole; an export cut
off part way leaves no manifest, and removes the files it wrote when it can.

The manifest holds the export's number (`seq`), the level, the time it ran,
the `--since` and `--until` bounds, each file with its rows and bytes, and
the ids of the live versions (each uuid's current version, not deleted):

```sh
sudo -u vcon jq -c '{format, seq, metadata, rows, files: (.files | length), live: (.live | length)}' \
  "/var/lib/vcon-export/tenant=$TENANT/manifest/00000000000000000001.json"
```

```text
{"format":1,"seq":1,"metadata":"counts","rows":2,"files":1,"live":2}
```

Each export to the same target writes new files and the next manifest,
`…02.json` and on, and leaves the earlier ones as they were. Read through
the newest manifest, as below, to see one export and no other.

`--since` and `--until` take RFC 3339 times and export only the versions
stored at or after `--since` and before `--until`, such as one day's:
`--since 2026-11-05T00:00:00Z --until 2026-11-06T00:00:00Z`.

## Read it with DuckDB

Download the DuckDB command-line program for the host's CPU and check it
against the SHA-256 of its release asset:

```sh
DUCKDB_VERSION=1.5.6
case "$(uname -m)" in
  x86_64) DUCKDB_ARCH=amd64 DUCKDB_SHA256=6e89deac1ebbc36eed0291caf8b567b030c7b86ac35998f71854e22b3c5d5e2f ;;
  aarch64) DUCKDB_ARCH=arm64 DUCKDB_SHA256=c544e92c9b7c31fc53c2139802cabd8e2d1b2b3e3f933117f31611239c1402db ;;
esac
curl -fsSL --retry 5 --retry-all-errors -o duckdb.zip \
  "https://github.com/duckdb/duckdb/releases/download/v$DUCKDB_VERSION/duckdb_cli-linux-$DUCKDB_ARCH.zip"
echo "$DUCKDB_SHA256  duckdb.zip" | sha256sum -c
unzip -q -o duckdb.zip duckdb
sudo install -m 0755 duckdb /usr/local/bin/duckdb
duckdb --version
```

```text
duckdb.zip: OK
v1.5.6 <words>
```

Read the files the newest manifest names. A table function in DuckDB takes
no subquery, so the list goes into a variable first:

```sh
EXPORT=/var/lib/vcon-export
MANIFEST=$(sudo -u vcon ls "$EXPORT/tenant=$TENANT/manifest" | tail -n 1)
sudo -u vcon duckdb -csv -c "
  SET VARIABLE files = (SELECT list('$EXPORT/' || f.path) FROM (SELECT unnest(files) AS f FROM read_json('$EXPORT/tenant=$TENANT/manifest/$MANIFEST')));
  SELECT count(*) AS versions, count(*) FILTER (WHERE form = 'unsigned') AS unsigned, sum(party_count) AS parties
  FROM read_parquet(getvariable('files'));"
```

```text
versions,unsigned,parties
2,2,2
```

The columns and their types, as DuckDB reads them. DuckDB adds `date` and
`tenant` from the directory names, and reads the times, which the files hold
in nanoseconds, as `TIMESTAMP WITH TIME ZONE`, to the microsecond:

```sh
sudo -u vcon duckdb -csv -c "DESCRIBE SELECT * FROM read_parquet('$EXPORT/tenant=$TENANT/date=*/*.parquet')" | cut -d, -f1,2
```

```text
column_name,column_type
version,UUID
stored_at,TIMESTAMP WITH TIME ZONE
sealed,BLOB
form,VARCHAR
gzip,BOOLEAN
size,BIGINT
created_at,TIMESTAMP WITH TIME ZONE
updated_at,TIMESTAMP WITH TIME ZONE
party_count,INTEGER
dialog_count,INTEGER
date,DATE
tenant,VARCHAR
```

A directory that holds one export, and nothing else, can also be read by a
glob, with the `tenant` and `date` directories as columns:

```sh
sudo -u vcon duckdb -csv -c "
  SELECT typeof(date) AS date_type, count(*) AS versions
  FROM read_parquet('$EXPORT/tenant=*/date=*/*.parquet', hive_partitioning = true) GROUP BY ALL"
```

```text
date_type,versions
DATE,2
```

A glob reads every export in the directory, and any file an export cut off
by a crash left behind, so with more than one export it counts a version
once for each. Read through the newest manifest then.

## Check the export

The rows are the versions the tenant keeps: here the two vCons stored
above. The export is recorded in the tenant's log:

```sh
sudo -u vcon vcon-server audit export --tenant "$TENANT" --config /etc/vcon-server/vcon-server.toml \
  | jq -c 'select(.action == "export") | {actor, action, status}'
```

```text
{"actor":"cli","action":"export","status":200}
```

At the `counts` level no file holds a subject, a party or a uuid:

```sh
sudo -u vcon grep -rlF -e "Export example" -e "+15555550101" -e "Caller Example" /var/lib/vcon-export || echo "no clear vCon content"
```

```text
no clear vCon content
```

## Export to a bucket

These steps follow [keep vCons in an S3 bucket](@/docs/s3-storage.md) with
RustFS at `objects.example.com`, and sign requests with `curl` as that page
does. Its clean-up step unsets the key; set it again, and make a bucket of
its own for exports, apart from the server's:

<!-- docs-test: only rustfs -->
```sh
S3_SECRET=$(sudo cat /etc/rustfs/secret_key)
S3=(--cacert /usr/local/share/vcon-server/s3-ca.pem --aws-sigv4 aws:amz:us-east-1:s3 --user "vcon-server:$S3_SECRET")
curl -sS --fail-with-body "${S3[@]}" -X PUT https://objects.example.com:9000/vcon-exports
curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon-exports?versioning"; echo
```

```text
<?xml version="1.0" encoding="UTF-8"?><VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"></VersioningConfiguration>
```

With no `[export.s3]` table, the export reaches the bucket with
`[storage.s3]`'s service and keys. Export to the prefix `daily/`:

<!-- docs-test: only rustfs -->
```sh
sudo -u vcon vcon-server export parquet --tenant "$TENANT" --to s3://vcon-exports/daily/ \
  --config /etc/vcon-server/vcon-server.toml | jq -c '{to, seq, files, rows}'
curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon-exports?list-type=2" | grep -o '<Key>[^<]*' | cut -c6-
```

```text
{"to":"s3://vcon-exports/daily/","seq":1,"files":1,"rows":2}
daily/tenant=<tenant id>/date=<yyyy-mm-dd>/part-<file id>.parquet
daily/tenant=<tenant id>/manifest/00000000000000000001.json
```

Each object is one PutObject with `If-None-Match: *`, so it is written whole
or not at all, and never replaces an object: of two exports that race for
one manifest name, the second is refused (see
[troubleshooting](#troubleshooting)). The manifest is put last.

To read the bucket with DuckDB, load its `httpfs` extension (DuckDB fetches
it from its extension repository the first time, and refuses an extension
whose signature does not check, unless `allow_unsigned_extensions` is set),
give it the bucket's key, and trust the storage's certificate authority.
This puts the secret on DuckDB's command line, as `curl --user` does above:

<!-- docs-test: only rustfs -->
```sh
MANIFEST=s3://vcon-exports/daily/tenant=$TENANT/manifest/00000000000000000001.json
sudo -u vcon duckdb -csv -c "
  INSTALL httpfs; LOAD httpfs;
  SET ca_cert_file = '/usr/local/share/vcon-server/s3-ca.pem';
  CREATE SECRET (TYPE s3, KEY_ID 'vcon-server', SECRET '$S3_SECRET', REGION 'us-east-1',
    ENDPOINT 'objects.example.com:9000', URL_STYLE 'path');
  SET VARIABLE files = (SELECT list('s3://vcon-exports/daily/' || f.path) FROM (SELECT unnest(files) AS f FROM read_json('$MANIFEST')));
  SELECT count(*) AS versions FROM read_parquet(getvariable('files'));"
```

`CREATE SECRET` prints its own row, `Success`, before the count:

```text
Success
true
versions
2
```

## Other readers

The files are plain Parquet 2, uncompressed, without dictionary pages, with
the `version` and `uuid` columns as `FIXED_LEN_BYTE_ARRAY(16)` with the UUID
logical type, and the times as `INT64` with the logical type
`TIMESTAMP(NANOS, UTC)`. The partition directories follow the Hive layout,
`tenant=<id>/date=<yyyy-mm-dd>/`. CI reads them with DuckDB only; the
following was not run by CI and not checked for this page:

- **Athena:** a table over `s3://vcon-exports/daily/` with
  `PARTITIONED BY (tenant string, date date)` and `STORED AS PARQUET`, and
  its partitions loaded with `MSCK REPAIR TABLE`. `date` is a reserved word
  there and is quoted with backticks.
- **Spark:** `spark.read.parquet("s3a://vcon-exports/daily/")` finds the
  `tenant` and `date` partitions. Spark's support for the
  `TIMESTAMP(NANOS)` type depends on its version and settings.

Either reader sees every export under the prefix. To read one export, read
the files its manifest names, or give each export a prefix of its own, such
as `s3://vcon-exports/2026-11-05/`.

## Troubleshooting

Each refusal exits 1 and says why on standard error, and writes nothing:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server export parquet --tenant "$TENANT" --to /var/lib/vcon-export --since yesterday \
  --config /etc/vcon-server/vcon-server.toml
```

```text
vcon-server: "yesterday" is not an RFC 3339 time such as 2026-11-05T00:00:00Z
```

| Message | Why, and what to do |
|---|---|
| `"…" is not an RFC 3339 time such as 2026-11-05T00:00:00Z` | `--since` or `--until` is not a time with a date, a time and an offset. |
| `"…" is not a directory or an s3://bucket/prefix` | `--to` is empty, or an `s3://` URL with no bucket or with an empty part, such as `s3://vcon-exports//daily`. |
| `"…" is not a metadata level: none, counts or full` | `tenant set-metadata` takes only these. |
| `an s3:// target needs an [export.s3] or [storage.s3] table, in a build with the s3 feature` | Add an `[export.s3]` table (see [configuration](@/docs/configuration.md#keys)). |
| `no tenant has this id` | `--tenant` names no tenant; `vcon-server tenant list` gives the ids. |
| `the target could not be written: …` | The directory cannot be written by the service account, or the bucket refused a call; the rest of the message is the system's or the service's reason, such as `AccessDenied`. The files the export wrote are removed. |
| `another export wrote tenant=<id>/manifest/<seq>.json first; run this export again` | Two exports to one target ran at once and took one number. The refused one removed its files; run it again for the next number. |
| `a version's object is missing or does not open; run vcon-server check` | The store is damaged. [Check](@/docs/operations.md#checking-the-data) it. |
| `the tenant's data could not be read: Io` | The catalog or a version's object could not be read, which says nothing about the stored data: a disk error, a file the service account may not read, or a bucket that cannot be reached, refuses the key or does not answer within `storage.s3.timeout`. Fix the cause and run the export again. |
| `the export is written, but its audit record could not be; the manifest names its files` | The export is whole; its record in the tenant's log was lost. |
| `the data directory is in use by a command that runs alone` | A key rotation, `check` or a tenant removal is running. Run the export after it. |

A version purged while the export runs is left out and counted in
`skipped`.
