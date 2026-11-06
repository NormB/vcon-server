+++
title = "Keep vCons as Parquet files in a bucket"
description = "Turn on Parquet storage over an S3 bucket: what the bucket must be, the configuration, a check that the server packs its objects into Parquet files and still reads each vCon back, a read of the files with DuckDB, what a purge removes from the bucket and from the storage's disks, and what each refusal means."
weight = 11
+++

<!-- docs-test: skip the run uses the server built from the commit under test -->
```sh
sudo install -m 0755 target/release/vcon-server /usr/local/bin/vcon-server
```

Parquet storage is on `main` and in no release yet: release 0.3.0 refuses
the `[storage.parquet]` table as an unknown key. Until the next release,
build the server from `main` to use it, as
[build from source](@/docs/install.md#build-from-source) shows.

With [S3 object storage](@/docs/s3-storage.md), the server keeps one sealed
object per stored version in a bucket. With Parquet storage on as well, it
packs those objects into [Apache Parquet](https://parquet.apache.org/)
files: one file for each tenant and each date a version was stored, at
`<prefix>tenants/<tenant id>/parquet/date=<yyyy-mm-dd>/part-<file id>.parquet`.
Each row of a file is one version's sealed object, byte for byte, beside
its version id and the time it was stored, and nothing else: the files
hold no clear part of any vCon.

A new version is still written first as its own object. A pack run, at
start and then every `pack_interval` seconds, packs the objects into files
and removes them. A read finds a packed version by its row and fetches
that row's bytes with one ranged read. A purge rewrites the file without
the purged row, deletes the old file, and checks that the bucket no longer
has it, before the purge is done.

- **The files only grow, for now.** A replaced or deleted version stays in
  its file, hidden, until retention purges it, and each pack run writes new
  files. Only a purge rewrites a file. Nothing yet merges small files or
  rewrites a file whose rows are mostly hidden: that is a later change.
- **There is no way back.** No command turns packed versions back into
  objects. A server whose configuration no longer has `[storage.parquet]`
  still reads and purges its packed versions, but packs no more.

This page was run on Debian 13 under systemd, after
[keep vCons in an S3 bucket](@/docs/s3-storage.md) with RustFS 1.0.4 and
[export to Parquet](@/docs/export-parquet.md). MinIO and AWS S3 were not
run with these steps. CI runs the store's conformance suite on Parquet
storage against RustFS and against MinIO on each change.

## Before you start

- **A server keeping its objects in a bucket, as
  [keep vCons in an S3 bucket](@/docs/s3-storage.md) sets it up.** The
  steps use that page's files in `/etc/vcon-server` and its shell variables
  from [getting started](@/docs/getting-started.md) (`$TENANT`,
  `$RECORDER` to write, `$MONITOR` to read).
- **A bucket whose versioning was never turned on, with no Object Lock.**
  A versioned bucket keeps a deleted file's bytes as an earlier version, and
  Object Lock can refuse to delete a file, so a purge could not remove the
  bytes. The server refuses such a bucket for Parquet storage at start,
  whatever `storage.s3.allow_versioning` says.
- **The key's rights.** The pack run and the purge need `s3:GetObject`
  (with ranges), `s3:PutObject`, `s3:DeleteObject` and `s3:ListBucket` on
  the bucket, as the objects already do, and the start reads the bucket's
  Object Lock setting (`s3:GetBucketObjectLockConfiguration`).
- **DuckDB, to read the files.** The [export page](@/docs/export-parquet.md)
  installs it.

## Check the bucket

The steps sign requests with `curl`, as the S3 page does. Its clean-up
unset the key; set it again:

<!-- docs-test: only rustfs -->
```sh
S3_SECRET=$(sudo cat /etc/rustfs/secret_key)
S3=(--cacert /usr/local/share/vcon-server/s3-ca.pem --aws-sigv4 aws:amz:us-east-1:s3 --user "vcon-server:$S3_SECRET")
```

The S3 page checked that the bucket was never versioned. Check that it has
no Object Lock: a bucket without it answers 404 for its lock configuration:

<!-- docs-test: only rustfs -->
```sh
curl -sS -o /dev/null -w '%{http_code}\n' "${S3[@]}" "https://objects.example.com:9000/vcon?object-lock"
```

```text
404
```

## Turn Parquet storage on

The bucket already holds an object for each version stored on the earlier
pages. Turning Parquet storage on over the same bucket and prefix packs
those too, oldest date first, as the first pack runs go: this is how a
server moves from S3 objects to Parquet storage. Reads work all through,
since a version is either an object or a packed row.

Save the configuration again with a `[storage.parquet]` table. Here the
pack run comes every 10 seconds, so the check below is quick; leave
`pack_interval` out for the default of 300:

<!-- docs-test: file vcon-server.toml -->
```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"

[storage]
objects = "s3"

[storage.s3]
endpoint = "https://objects.example.com:9000"
region = "us-east-1"
bucket = "vcon"
path_style = true
access_key_id = "vcon-server"
secret_access_key_file = "/etc/vcon-server/s3.secret"
ca_file = "/usr/local/share/vcon-server/s3-ca.pem"

[storage.parquet]
pack_interval = 10
```

`pack_limit`, 16 MiB unless set, is the largest sealed object packed; a
larger one, such as a vCon with a long recording inline, stays an object.
Every key is on the [configuration page](@/docs/configuration.md#parquet-storage).

Install it and restart the server. At start, it reads the bucket's
versioning and Object Lock, and stops with an error naming the bucket if
either is on:

```sh
sudo install -m 0400 -o vcon -g vcon vcon-server.toml /etc/vcon-server/vcon-server.toml
sudo systemctl restart vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
```

```text
answering
```

## Check that the server packs its objects

The pack run at start packs the objects the earlier pages stored. Wait
until no object is left under the tenant's name, and note the files:

<!-- docs-test: only rustfs -->
```sh
LIST="https://objects.example.com:9000/vcon?list-type=2&prefix=tenants/"
for i in $(seq 30); do
  curl -sS --fail-with-body "${S3[@]}" "$LIST" | grep -q '/objects/' || break
  sleep 2
done
BEFORE=$(curl -sS --fail-with-body "${S3[@]}" "$LIST" | grep -o '<Key>[^<]*' | cut -c6-)
echo "$BEFORE"
```

```text
tenants/<tenant id>/parquet/date=<yyyy-mm-dd>/part-<file id>.parquet
```

Store a vCon with the write key:

```sh
UUID=$(cat /proc/sys/kernel/random/uuid)
cat > packed.vcon <<EOF
{"vcon":"0.4.0","uuid":"$UUID","created_at":"2026-11-06T12:00:00Z","subject":"Packed example","parties":[{"tel":"+15555550102","name":"Packed Example"}],"dialog":[],"attachments":[],"analysis":[]}
EOF
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/vcons \
  -H "Authorization: Bearer $RECORDER" -H "Content-Type: application/vcon" --data-binary @packed.vcon
```

```text
201
```

Its object is in the bucket until the next pack run takes it. Wait for
that, then list the tenant's part of the bucket: the file from before, a
new file for the new version, and no object:

<!-- docs-test: only rustfs -->
```sh
for i in $(seq 30); do
  curl -sS --fail-with-body "${S3[@]}" "$LIST" | grep -q '/objects/' || break
  sleep 2
done
AFTER=$(curl -sS --fail-with-body "${S3[@]}" "$LIST" | grep -o '<Key>[^<]*' | cut -c6-)
echo "$AFTER"
NEW=$(comm -13 <(echo "$BEFORE") <(echo "$AFTER"))
```

```text
tenants/<tenant id>/parquet/date=<yyyy-mm-dd>/part-<file id>.parquet
tenants/<tenant id>/parquet/date=<yyyy-mm-dd>/part-<file id>.parquet
```

Each pack run is logged with counts only, never a tenant:

```sh
sudo journalctl -u vcon-server --since -2min -o cat | grep 'pack run' | grep -v 'versions=0' | tail -n 1
```

```text
<time>  INFO vcon_server::serve: pack run files=1 versions=1 failed=0
```

The vCon reads back byte for byte, from its row:

```sh
curl -sS --fail-with-body --cacert /usr/local/share/vcon-server/ca.pem "https://vcon.example.com:8443/vcons/$UUID" \
  -H "Authorization: Bearer $MONITOR" -o read.vcon
cmp read.vcon packed.vcon && echo "the same bytes"
```

```text
the same bytes
```

`vcon-server check` finds every packed version's file in the bucket, at the
length the catalog records:

```sh
sudo systemctl stop vcon-server
sudo -u vcon vcon-server check --config /etc/vcon-server/vcon-server.toml | jq -c 'select(.name != null) | {name, damaged}'
sudo systemctl start vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
```

```text
{"name":"acme","damaged":[]}
answering
```

## Read the files with DuckDB

The files are plain Parquet. DuckDB reads the `date=` directories as a
DATE column, and the version id as a UUID; `sealed` is ciphertext that
only the server can open:

<!-- docs-test: only rustfs -->
```sh
sudo -u vcon duckdb -csv -c "
  INSTALL httpfs; LOAD httpfs;
  SET ca_cert_file = '/usr/local/share/vcon-server/s3-ca.pem';
  CREATE SECRET (TYPE s3, KEY_ID 'vcon-server', SECRET '$S3_SECRET', REGION 'us-east-1',
    ENDPOINT 'objects.example.com:9000', URL_STYLE 'path');
  SELECT typeof(version) AS version, typeof(date) AS date, count(*) > 0 AS rows
  FROM read_parquet('s3://vcon/tenants/$TENANT/parquet/*/*.parquet', hive_partitioning = true)
  GROUP BY ALL;"
```

```text
Success
true
version,date,rows
UUID,DATE,true
```

A glob reads every file in the bucket. While a purge rewrites a file, the
old file and the new one are both there for a moment, so a glob can count a
row twice, or read a file just deleted and fail; read again.

## Check what a purge removes

A purge removes the row's data key in its own transaction, so from then on
nobody can open the purged version, even with a copy of the file. Then it
writes the file again without the row, deletes the old file, and checks
with HeadObject that the bucket no longer has it. Only then is the purge
done. The new file holds only the new vCon. With a retention period of
`0d`, a delete purges at once:

<!-- docs-test: only rustfs -->
```sh
DELETER=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope delete --label parquet-check \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
sudo -u vcon vcon-server tenant set-retention "$TENANT" 0d --config /etc/vcon-server/vcon-server.toml | jq -c '{retention}'
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem -X DELETE \
  "https://vcon.example.com:8443/vcons/$UUID" -H "Authorization: Bearer $DELETER"
curl -sS -o /dev/null -w '%{http_code}\n' "${S3[@]}" -I "https://objects.example.com:9000/vcon/$NEW"
sudo -u vcon vcon-server tenant set-retention "$TENANT" default --config /etc/vcon-server/vcon-server.toml | jq -c '{retention}'
```

```text
{"retention":"0d"}
204
404
{"retention":null}
```

HeadObject on the new file answers 404: it is gone from the bucket. Since
it held no other row, no file replaces it; a file with other rows is
written again with those rows under a new name. The file from before is
still there, and no object in the bucket holds the purged vCon's uuid:

<!-- docs-test: only rustfs -->
```sh
curl -sS --fail-with-body "${S3[@]}" "$LIST" | grep -o '<Key>[^<]*' | cut -c6- | cmp - <(echo "$BEFORE") && echo "the file from before"
for key in $(curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon?list-type=2" | grep -o '<Key>[^<]*' | cut -c6-); do
  if curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon/$key" | grep -q -a "$UUID"; then
    echo "$key holds it"
  fi
done
echo "scanned"
```

```text
the file from before
scanned
```

### What a delete leaves on the storage's disks

An S3 DELETE removes the bytes from what any S3 call can read, at once. The
storage system removes them from its disks later, on its own schedule:

- **RustFS.** Measured on RustFS 1.0.1 on 2026-11-05: a deleted object's
  bytes were moved to `.rustfs.sys/tmp/.trash/` under the data directory.
  They were still on disk about a minute after the DELETE, and gone about
  3.6 minutes after it. RustFS removes them with a background task whose
  interval was not measured.
- **MinIO and AWS S3.** When they remove a deleted object's bytes from their
  disks was not measured, and AWS does not document it.

Until then, the purged row's sealed bytes are on the disk, but nobody can
open them: the data key that opened them was in the catalog row the purge
deleted. The files hold no clear column. A bucket's lifecycle rules, and
backups of the storage host's disks, are the storage operator's: a copy
taken there keeps a file after the server deletes it.

## Clean up

```sh
rm -f packed.vcon read.vcon vcon-server.toml
unset S3_SECRET S3 LIST BEFORE AFTER NEW DELETER
```

## When it does not start

### The storage table does not hold together

`[storage.parquet]` without `storage.objects = "s3"`:

```text
vcon-server: configuration: storage.parquet packs the objects of a bucket, so it needs storage.objects = "s3"
```

A `pack_interval` or `pack_limit` of 0 is refused by name, such as
`storage.parquet.pack_interval must be more than 0`.

### The bucket has versioning on

```text
vcon-server: the storage cannot be used: bucket "vcon" has versioning Enabled, which keeps a deleted file's bytes as an earlier version; Parquet storage (storage.parquet) needs a bucket that was never versioned, and storage.s3.allow_versioning does not apply
```

A bucket that ever had versioning on reports `Suspended` once it is turned
off, and is refused too. Use a new bucket, and move the objects to it while
the server is stopped, with Parquet storage off.

### The bucket has Object Lock on

```text
vcon-server: the storage cannot be used: bucket "vcon" has Object Lock on, which can refuse to delete a file a purge rewrote; Parquet storage (storage.parquet) needs a bucket without Object Lock
```

Object Lock can only be set when a bucket is made. Use a bucket made
without it. A legal hold on a vCon is kept in the catalog, not with Object
Lock: see [retention and holds](@/docs/retention-and-holds.md).

## When a pack run or a purge fails

A pack run that fails leaves every version readable, as an object or as a
row, and the next run, or start-up recovery, finishes or removes what it
left. A purge that fails after it removed the data key reports a failure,
and the purged version is gone at once; the next purge sweep or start
finishes the file. Each of these is logged without a tenant or a name:

- `pack run files=<n> versions=<n> failed=<n>` with `failed` above 0: a
  tenant's run failed, as when the bucket cannot be reached.
- `a pack file's new name was already taken; the pack run stops`: a write
  with `If-None-Match: *` was refused, so another writer used a name this
  server chose. Check that no second server uses the same bucket and prefix
  (see [one server per bucket and prefix](@/docs/s3-storage.md#one-server-per-bucket-and-prefix)).
- `the storage still holds a pack file it was told to delete`: the bucket
  answered a DELETE with success but still has the file, which a versioned
  bucket does. The purge does not report itself done.
- `purges left pack files to rewrite after 4 rounds`: another purge kept
  rewriting the same file; the next sweep goes on.
- `start-up recovery removed pack files that no catalog row names removed=<n>`:
  a pack run or a rewrite was cut off after it wrote a file and before the
  catalog recorded it.
