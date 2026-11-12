+++
title = "Configuration"
description = "Every key of the configuration file, its default, and why the default is what it is."
weight = 55

[extra]
group = "reference"
+++

`vcon-server serve --config <file>` reads one TOML file. A wrong file stops the
start with an error that names the key. A key the server does not know is an
error too, so a misspelled key is never silently ignored.

Durations are whole seconds and sizes are bytes; each must be more than zero.

## Secret files

Every file that holds a secret is held to one rule: `master_key`, `tls.key`,
`storage.s3.secret_access_key_file`, `export.s3.secret_access_key_file`,
`catalog.postgres.password_file` and `catalog.postgres.client_key_file`, the
key file `rekey master --new-master` names, and the MCP stdio
configuration's `key_file`. The server, or the command, stops before it uses
a file that is:

- not a regular file, such as a directory or a FIFO;
- owned by a user other than the one it runs as, or root (a process that
  runs as root may read any owner's file);
- readable, writable or executable by other users, or writable by its
  group: any bit of `0027` in its mode.

`0600`, `0400` and `0640` pass. `0640` is for a file that root owns and the
server's group reads, such as `0640 root:vcon`. `0644`, `0604` and `0660` are
refused. A symbolic link is followed, and the file it names is the one
checked. The error names the key, the file, its mode and the command that
fixes it:

```text
vcon-server: the master key cannot be read: master_key /etc/vcon-server/master.key: other users may read or change the file (mode 0644); run `chmod 600 /etc/vcon-server/master.key`, or `chmod 640 /etc/vcon-server/master.key` for a group that only reads it
vcon-server: the TLS certificate or key cannot be used: tls.key /etc/vcon-server/key.pem: the file is owned by uid 1001, not by the user this runs as (uid 998) or root; run `chown 998 /etc/vcon-server/key.pem` (or `chown root`)
```

Releases up to 0.4.0 checked only the two `catalog.postgres` files, and
allowed a group to write them. To check a host before an upgrade, see
[upgrade](@/docs/upgrading.md#from-0-4-0-to-0-4-1-the-secret-file-rule).

## Keys

| Key | Meaning | Default |
|---|---|---|
| `listen` | Address and port to serve on, such as `0.0.0.0:8443` | required |
| `data_dir` | Directory holding `server.sqlite` and the tenant directories | required |
| `master_key` | File holding the 32-byte master key | required |
| `tls.cert` | PEM certificate chain | required |
| `tls.key` | PEM private key | required |
| `operator.listen` | Address and port of the operator listener, such as `127.0.0.1:8444`. With it, this address serves operator keys only and `listen` serves tenant keys only (see [the operator listener](@/docs/operator-listener.md)) | none: `listen` serves both |
| `operator.tls.cert` | PEM certificate chain the operator listener presents; set together with `operator.tls.key` | `tls.cert` |
| `operator.tls.key` | PEM private key of the operator listener; set together with `operator.tls.cert` | `tls.key` |
| `public_origin` | The origin a browser reaches `listen` at: `https://`, a host in lower case and, unless it is 443, a port, with nothing after, such as `https://vcon.example.com`. A request by session must come from it (see [sessions](@/docs/session-origin.md)) | none: the listener serves no session |
| `operator.public_origin` | The origin a browser reaches the operator listener at, written as `public_origin` is. Its host must differ from the host of `public_origin`: a browser keeps a session cookie by host and not by port | none: the listener serves no session |
| `retention_default` | Retention period for tenants that set none: years, months or days, written `7y`, `18m` or `90d` | `7y` |
| `purge_interval` | Seconds between purge sweeps | `3600` (1 hour) |
| `rotation_warn` | Seconds the oldest work may wait for key rotation before a warning is logged | `2592000` (30 days) |
| `keys.no_expiry` | Whether a key may be made with no expiry (`"no_expiry": true`, or `key create --no-expiry`). With `false`, such a request is refused with [`bad-expiry`](@/problems/bad-expiry.md), and every new key expires within 365 days. Keys made earlier are not changed | `true` |
| `limits.body` | Largest request body, in bytes | `16777216` (16 MiB) |
| `limits.inflated` | Most bytes gzip inflation may produce while one body is checked | `67108864` (64 MiB) |
| `limits.connections` | Most connections a listener holds at once, from 1 to 65536. At that number the listener takes no more until one ends | `512` |
| `limits.csp_reports` | Most [reports of a broken console policy](@/docs/console.md#reports-of-a-broken-policy) a listener takes in one minute, from 1 to 6000; a report past it is refused with 429 | `120` |
| `limits.csp_reports_per_address` | Most such reports a listener takes in one minute from one source address, from 1 to `limits.csp_reports`. Unset, it is 20, or `limits.csp_reports` when that is less | `20` |
| `limits.timeouts.read` | Seconds a request body may take to arrive | `60` |
| `limits.timeouts.handshake` | Seconds a connection may take over its TLS handshake, from 1 to 3600; a connection that takes longer is closed | `10` |
| `limits.timeouts.first_head` | Seconds from the end of the handshake to the whole head of the connection's first request, on HTTP/1.1 and HTTP/2, from 1 to 3600; a connection that has sent no request by then is closed | `30` |
| `limits.timeouts.head` | Seconds an HTTP/1.1 request head may take to arrive, counted from its first bytes, from 1 to 3600; a connection whose head takes longer is closed. The wait for a request is not counted here: it is under `limits.timeouts.idle` | `30` |
| `limits.timeouts.idle` | Seconds a connection that has had a request may wait for the next, from 1 to 3600: an HTTP/1.1 connection between requests, an HTTP/2 connection with no stream open. The connection is then closed, an HTTP/2 connection after a GOAWAY frame | `120` |
| `supported_critical` | Extension names accepted in a vCon's `critical` list, for all tenants | none |
| `mcp.calls_per_second` | MCP tool calls one key may make in each one-second window | `100` |
| `mcp.concurrent` | MCP tool calls running at once on the server | `100` |
| `mcp.response_bytes` | Largest MCP tool result, in bytes; a longer list or vCon comes in pages | `1048576` (1 MiB) |
| `storage.objects` | Where the sealed vCon objects are kept: `local`, beside each tenant's catalog in `data_dir`, or `s3`, in an S3-compatible bucket (see [Storage](#storage)) | `local` |
| `storage.s3.endpoint` | URL of the S3-compatible service, `https://`; absent means AWS S3 for the region | none |
| `storage.s3.region` | Region name, used in request signing | required with `s3` |
| `storage.s3.bucket` | Bucket the objects go in; it must exist | required with `s3` |
| `storage.s3.prefix` | Written before `tenants/` in every object's name, so one bucket can hold more than one server's objects; empty, or ending in `/` and not starting with one | empty |
| `storage.s3.path_style` | Name the bucket in the request path, not in the host name, as RustFS and MinIO are usually set up | `false` |
| `storage.s3.access_key_id` | Access key id; absent means the AWS default credential chain (environment, profile, instance or web identity) | none |
| `storage.s3.secret_access_key_file` | File holding the secret access key; required with `access_key_id` | none |
| `storage.s3.allow_http` | Accept an `http://` endpoint, for a test service on the same host | `false` |
| `storage.s3.ca_file` | PEM file of the certificate authorities trusted for an `https://` endpoint, in place of the system's, for a service whose certificate a private CA signed | none |
| `storage.s3.allow_versioning` | Start even though the bucket has versioning on or suspended | `false` |
| `storage.s3.timeout` | Seconds one request to the bucket may take | `30` |
| `storage.parquet.pack_interval` | Seconds from the end of one pack run to the start of the next; the table `[storage.parquet]` turns [Parquet storage](#parquet-storage) on | `300` (5 minutes) |
| `storage.parquet.pack_limit` | Largest sealed object packed into a Parquet file, in bytes; a larger one stays an object | `16777216` (16 MiB) |
| `storage.parquet.reader_grace` | Seconds an older manifest, and a file a compaction merged away, stays after a newer manifest is written, for readers outside the server; a purge deletes them at once | `900` (15 minutes) |
| `export.s3.region` | Region of the S3-compatible service that [`export parquet`](@/docs/export-parquet.md) writes an `s3://` target to; without an `export.s3` table the export uses `storage.s3`'s service and keys | required with `export.s3` |
| `export.s3.endpoint` | As `storage.s3.endpoint`, for the export's service | none |
| `export.s3.path_style` | As `storage.s3.path_style` | `false` |
| `export.s3.access_key_id` | As `storage.s3.access_key_id` | none |
| `export.s3.secret_access_key_file` | As `storage.s3.secret_access_key_file` | none |
| `export.s3.allow_http` | As `storage.s3.allow_http` | `false` |
| `export.s3.ca_file` | As `storage.s3.ca_file` | none |
| `export.s3.timeout` | Seconds one request to the export's service may take | `30` |
| `catalog.backend` | Where the catalogs and the registry are kept: `local`, as SQLCipher files in `data_dir`, or `postgres`, a preview that needs `storage.objects = "s3"` (see [Catalog in PostgreSQL](#catalog-in-postgresql)) | `local` |
| `catalog.postgres.hosts` | Host names or IP addresses of the PostgreSQL servers, tried in order; a DNS name in ASCII (an internationalized name in its `xn--` form) or an address, never a Unix socket | required with `postgres` |
| `catalog.postgres.port` | TCP port of every host | `5432` |
| `catalog.postgres.dbname` | Database name, at most 63 bytes | required with `postgres` |
| `catalog.postgres.user` | Role the server logs in as, at most 63 bytes; not a superuser, without `BYPASSRLS`, not the catalog's owner or a member of it, and a member of `vcon_app` or `vcon_app` itself. `vcon-server migrate` logs in with a copy of the configuration whose role may act as `vcon_owner` | required with `postgres` |
| `catalog.postgres.ca_file` | PEM file of the certificate authorities trusted for the servers' certificates, in place of the system's | none |
| `catalog.postgres.password_file` | File holding the role's password, used only with SCRAM-SHA-256 and channel binding; one line | none |
| `catalog.postgres.client_cert_file` | PEM client certificate chain; required with `client_key_file` | none |
| `catalog.postgres.client_key_file` | PEM private key of the client certificate | none |
| `catalog.postgres.pool` | Most connections open at once | `16` |
| `catalog.postgres.connect_timeout` | Seconds a connection, or a wait for a free one, may take | `5` |
| `catalog.postgres.statement_timeout` | Seconds a statement may run (PostgreSQL's `statement_timeout`), at most `2147483` | `30` |
| `catalog.postgres.lock_timeout` | Seconds a statement may wait for a lock (PostgreSQL's `lock_timeout`), at most `2147483` | `10` |

### Connection limits

The four connection limits close a connection that stalls or sits unused,
so that it does not hold a file descriptor for as long as the other side
likes. They and `limits.connections` are new in 0.4.3: release 0.4.2
refuses a configuration that has one of the five keys.
Each limit applies to one part of a connection's life:

1. `handshake`: from the moment the listener takes the connection to the
   end of its TLS handshake.
2. `first_head`: from the end of the handshake to the whole head of the
   first request, on HTTP/1.1 and HTTP/2.
3. `head`: from the first bytes of an HTTP/1.1 request head to its end.
   The first request of an HTTP/1.1 connection is under both `first_head`
   and `head`, so the lesser applies to it.
4. `idle`: from the end of one answer to the next request, on a connection
   that has had at least one request. On HTTP/2 that is the time with no
   stream open; a ping or a settings frame does not start it again.

A request whose head arrived in time may take as long as it needs: no
connection limit runs while a request is being answered, and the request's
body is under `limits.timeouts.read`. The default of 30 seconds for a head
is the default header timeout of hyper, the HTTP library the server is
built on.

A connection that reaches `idle` is closed in order. An HTTP/2 connection
is sent a GOAWAY frame with the code `NO_ERROR`, a request that arrives at
that moment is still answered, and the connection is closed five seconds
later if the client has not ended it; that wait has no key. An HTTP/1.1
connection is closed between requests. A client finds the connection
closed and opens another for its next request, as HTTP clients do without
being told. Raise `idle` for clients that keep connections longer between
requests, and lower it to free places sooner.

### The most connections at once

`limits.connections` bounds how many connections a listener holds at once,
so that connections alone cannot use up the process's file descriptors.
Each listener has that many places of its own: with an
[operator listener](@/docs/operator-listener.md), connections to the
tenant listener take no place from it. Keep the number below the process's
limit on open files (`ulimit -n`, or `LimitNOFILE` in a systemd unit), with
room for the files the server itself opens.

At the limit the listener takes no more connections until one that is held
ends or is closed by a limit above. A connection that arrives then waits in
the system's queue for the listening socket. The server asks the system
for a queue of 128 connections, the number the Rust standard library uses,
and Linux uses the lesser of that and `net.core.somaxconn`. A client in
that queue has a TCP connection and no answer to its TLS hello, so it sees
a connection that is slow to start; `limits.timeouts.handshake` starts for
it only when the listener takes it. When the queue is full as well, Linux
leaves further connection attempts unanswered: on a Linux host with a
queue of 128, 129 connections completed and the rest ended at the client's
own connect timeout. A client that gives up before it is taken costs the
server nothing.

A listener that is full shows in two places:

- The log has a warning, at most one a minute for each listener, with the
  listener's address and the limit:

  ```text
  <time>  WARN vcon_server::api::listen: the listener is full: it holds limits.connections connections and takes no more until one ends listener=0.0.0.0:8443 connections=512
  ```

- The admin audit log counts each time a listener filled, as action
  [`listener_full`](@/docs/audit-log-records.md#every-action): one record for
  each listener and minute, whose `target` is the listener's key (`listen`
  or `operator.listen`) and whose `status` is `null`, since the record
  answers no request. Its `count` is a count of the times the listener
  became full in that minute, not of the connections that waited: a
  listener that takes its last free place is full whether or not another
  connection is waiting behind it. To list them:

  ```sh
  vcon-server audit export --admin --config /etc/vcon-server/config.toml \
    | jq -c 'select(.action == "listener_full") | {at, target, count}'
  ```

A listener that fills now and then under a burst of clients needs nothing.
One that is full minute after minute is either too small for its clients,
in which case raise `limits.connections` and the limit on open files with
it, or is held by connections that do nothing, in which case lower
`limits.timeouts.idle`.

When the system cannot hand the server a connection, such as when the
process is out of file descriptors, the server logs a warning, waits one
second and goes on. That wait has no key.

At start and then each day, `serve` logs a warning while the oldest purge or
tenant removal waiting for [key rotation](@/docs/operations.md#key-rotation)
is older than `rotation_warn`.

## A full file

```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"
retention_default = "7y"
purge_interval = 3600
rotation_warn = 2592000
supported_critical = []

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"

[limits]
body = 16777216
inflated = 67108864
connections = 512
csp_reports = 120
csp_reports_per_address = 20

[limits.timeouts]
read = 60
handshake = 10
first_head = 30
head = 30
idle = 120

[mcp]
calls_per_second = 100
concurrent = 100
response_bytes = 1048576
```

A key left out of a table that is present takes its default; the others keep
their written values.

## Storage

Without a `[storage]` table, everything is kept in `data_dir`, as before.
The `[storage]` table is in release 0.4.0 and later; release 0.3.0 refuses
it as an unknown key. With `storage.objects = "s3"`, each version's sealed
object goes to a bucket in an S3-compatible service (RustFS, MinIO or AWS
S3) instead, at `<prefix>tenants/<tenant id>/objects/<version id>`; the
catalogs, the registry, the lock files and the master key stay on the host.
An object is the same sealed bytes the server writes to a file: the bucket's
operator sees random names, sizes and times, and no vCon. The how-to [keep
vCons in an S3 bucket](@/docs/s3-storage.md) sets one up step by step. A
file for a server keeping its objects in a RustFS or MinIO bucket:

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
```

- **Addressing.** RustFS and MinIO name the bucket in the request path, so
  they need `path_style = true`. AWS S3 names it in the host name
  (virtual-hosted style), which is the default; with AWS S3, leave
  `endpoint` out and set `region` to the bucket's region.
- **Keys.** With `access_key_id`, the secret is read from
  `secret_access_key_file` at start: one line, its line ending left out. An
  empty file, or one of more than one line, is refused. Without
  `access_key_id`, the keys come from the AWS default credential chain: the
  `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` variables, the shared
  credentials file, web identity, or the instance's or task's role. No
  secret is logged or put in an error.
- **TLS.** The endpoint must be `https://`. Its certificate is checked
  against the system's certificate authorities, or, with `ca_file`, against
  those in that file only. `allow_http = true` lets an `http://` endpoint
  through, for a test service on the same host; `ca_file` is refused with
  one.
- **Versioning.** The server refuses to start on a bucket with versioning on
  or suspended unless `allow_versioning` is set: such a bucket keeps a
  purged object's earlier version. That version cannot be opened once its
  row is purged, since the key that opens it was kept only in that row, but
  it stays in the bucket until the bucket's own rules remove it.
- **One server per prefix.** The first server to start on an empty bucket
  and prefix claims it, in `storage.claim` in its data directory and
  `<prefix>vcon-server.claim` in the bucket. Another data directory is
  refused on that prefix, as is any data directory on a prefix that holds
  objects but no claim, because start-up recovery removes every object no
  catalog of its own names. Two servers can share a bucket with a `prefix`
  each.

### Parquet storage

With a `[storage.parquet]` table as well (it needs `storage.objects =
"s3"`), the server packs the objects in the bucket into Parquet files, one
date of one tenant at a time, at
`<prefix>tenants/<tenant id>/parquet/date=<yyyy-mm-dd>/part-<file id>.parquet`.
Each new version is still written first as its own object, then packed by
the next pack run, which removes the object. Reads by uuid fetch one row of
a file with a ranged read. A purge rewrites the file without the purged
row, deletes the old file and checks that the bucket no longer has it,
before the purge returns. Parquet storage is in release 0.4.0 and later.
The file above, with Parquet storage on:

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
pack_interval = 300
pack_limit = 16777216
```

- **The bucket.** The server refuses to start with `[storage.parquet]` on
  a bucket with versioning on or suspended, whatever `allow_versioning`
  says, or with Object Lock on: either keeps a deleted file's bytes,
  so a purge could not remove them.
- **What the files hold.** Each row is one version's id, the time it was
  stored and its sealed bytes, and nothing else: no clear column, whatever
  the tenant's metadata level. Queryable columns come from
  [`export parquet`](@/docs/export-parquet.md), which writes them at the
  tenant's level.
- **Manifests and compaction.** Each change of which files hold which rows,
  and each pack run that finds the live versions changed, writes the next
  numbered manifest, `<prefix>tenants/<tenant id>/manifest/<20 digits>.json`,
  naming the files and the live versions. Each pack run merges a date's
  small files into one. A file merged away, and an older manifest, stay for
  `reader_grace` seconds for readers on an older manifest; a purge deletes
  them at once. A replaced or deleted version stays in its file, hidden,
  until retention purges it.
- **No way back.** Once versions are packed, keep `[storage.parquet]`:
  there is no command that turns packed versions back into objects. Without
  the table, `serve` refuses to start while any tenant has a packed version,
  since it would pack no more and its start would no longer refuse a bucket
  with Object Lock or versioning; `check` names each such tenant as a
  problem. See
  [packed versions without the table](@/docs/parquet-storage.md#packed-versions-without-the-table).

## Catalog in PostgreSQL

Preview: with `catalog.backend = "postgres"`, `serve` and every command
read the `[catalog.postgres]` secret files, connect to PostgreSQL over TLS,
check the server, and then stop with exit code 1, because this release
does not yet keep the catalogs there. Run `vcon-server migrate` first, with
a role that may act as `vcon_owner`, or a connection refuses the missing
schema. The catalogs and the registry stay in `data_dir`. It needs the `postgres` cargo feature, which the release
binaries do not have yet. The how-to
[keep the catalogs in PostgreSQL](@/docs/postgres-catalog.md) gives the
database's side, each refusal and what it means.

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

[catalog]
backend = "postgres"

[catalog.postgres]
hosts = ["db1.example.com"]
dbname = "vcon"
user = "vcon_app"
ca_file = "/etc/vcon-server/pg-ca.pem"
password_file = "/etc/vcon-server/pg.password"
```

- **TLS only.** There is no `sslmode` key. Every connection is TLS, with
  the server's certificate checked against `ca_file` (or the system's
  certificate authorities without it) and against the host name it was
  reached by. A server that does not offer TLS is refused, and so is a Unix
  socket host.
- **Logging in.** With `password_file`, the server accepts only SCRAM-SHA-256
  with channel binding (`SCRAM-SHA-256-PLUS`), which ties the password
  exchange to this TLS connection; a server that asks for `md5`, `password`
  or no password at all (`trust`) is refused before the password is sent.
  With a client certificate and no `password_file`, a server's `trust` line
  cannot be told from its `cert` line, so check `pg_hba.conf`.
- **Secret files.** `password_file` and `client_key_file` are read once, at
  start, before any connection, and never again: the pool reconnects from
  memory. Each is held to [the secret-file rule](#secret-files).
- **The role.** A superuser, a role with `BYPASSRLS`, and a server in
  recovery (a standby) or with `transaction_read_only` on are refused.

## Where the defaults come from

The size limits were set from measurement, not guessed. Of 34,885 vCon files
measured (public datasets and recorded test calls), the largest was 5,141,813
bytes: a two-minute call with its recording held inline, as an 8 kHz 16-bit
stereo WAV (42,671 encoded bytes a second). Only 26 of their 25,471 recordings
were inline; the other 25,445 were referenced by URL.

- `limits.body` at 16 MiB holds that file three times over, or about six and a
  half minutes of that recording format inline. Longer recordings should be
  sent by URL, or the limit raised.
- `limits.inflated` is four times the body limit, so a vCon accepted plain is
  accepted gzipped too, with room left for gzipped payloads inside signed
  layers. It bounds the work a small, highly compressed body can cause.
- A 16 MiB body arriving within 60 seconds needs about 2.2 Mbit/s.

The server holds each request's body in locked memory while it works on it. On
a host short of memory, lower `limits.body`.

The three `mcp` limits are sipnab's defaults for its own MCP server. A tool call
past `mcp.calls_per_second` is refused with
[`rate-limited`](@/problems/rate-limited.md), and one past `mcp.concurrent`
with [`at-capacity`](@/problems/at-capacity.md), at once and never queued.

## Fixed limits

Two limits on how deeply a body nests are fixed: no key changes them. A body
past either is refused with [`too-deep`](@/problems/too-deep.md) before
anything deeper is read.

| Limit | Value | Counted |
|---|---|---|
| Signed layers in one body | `64` | JWS objects from the body to the unsigned vCon: `1` for a vCon signed once |
| Arrays and objects open at once | `64` | in each JSON text read on its own: the body, each signed payload; the outer object counts as `1` |

- **Signed layers.** The vCon core draft (section 5.2.3, Long Term Archiving
  of Signed vCons) signs an archived vCon again, as the payload of a new JWS,
  before each signing certificate expires, for archives kept "as long as 10
  years or more" with certificates that last "a few years at best". A
  signed vCon encrypted (a JWE) is not opened, so it adds no layer. `64`
  allows a new signature every year for 63 years. Of 51,642 vCons in the
  public datasets measured, none was signed; the draft's own signed example
  has one layer.
- **Arrays and objects.** The deepest of those vCons nests 5 deep, and the
  draft's examples 6. `64` is ten times that, and below the JSON parser's own
  bound, so a body this deep is told apart from one that is not JSON.

Each layer is read in turn, not by recursion, so the stack a request needs
does not grow with the layers.

## Retention periods

A period is a whole number, with no leading zero, followed by `y`, `m` or `d`. It is counted in
calendar units from the moment a version was stored, in UTC; a day past the
end of a month clamps, so a version stored on 29 February with a period of
`1y` falls due on 28 February. `0d` is allowed: a deleted vCon is then
purged as soon as it is deleted, unless it is under a legal hold. See [retention and legal holds](@/docs/retention-and-holds.md).

## Extensions in `critical`

A vCon may list, in `critical` (or `must_support`, its older name), extensions
a reader must understand. vcon-server refuses such a vCon with
[`unsupported-extension`](@/problems/unsupported-extension.md) unless every
name is in `supported_critical`. Name an extension there only if storing it
unread is right for you: the server never interprets vCon content.
