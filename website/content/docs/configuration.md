+++
title = "Configuration"
description = "Every key of the configuration file, its default, and why the default is what it is."
weight = 20
+++

`vcon-server serve --config <file>` reads one TOML file. A wrong file stops the
start with an error that names the key. A key the server does not know is an
error too, so a misspelt key is never silently ignored.

Durations are whole seconds and sizes are bytes; each must be more than zero.

## Keys

| Key | Meaning | Default |
|---|---|---|
| `listen` | Address and port to serve on, such as `0.0.0.0:8443` | required |
| `data_dir` | Directory holding `server.sqlite` and the tenant directories | required |
| `master_key` | File holding the 32-byte master key | required |
| `tls.cert` | PEM certificate chain | required |
| `tls.key` | PEM private key | required |
| `retention_default` | Retention period for tenants that set none: years, months or days, written `7y`, `18m` or `90d` | `7y` |
| `purge_interval` | Seconds between purge sweeps | `3600` (1 hour) |
| `rotation_warn` | Seconds the oldest work may wait for key rotation before a warning is logged | `2592000` (30 days) |
| `limits.body` | Largest request body, in bytes | `16777216` (16 MiB) |
| `limits.inflated` | Most bytes gzip inflation may produce while one body is checked | `67108864` (64 MiB) |
| `limits.timeouts.read` | Seconds a request body may take to arrive | `60` |
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
| `export.s3.region` | Region of the S3-compatible service that [`export parquet`](@/docs/export-parquet.md) writes an `s3://` target to; without an `export.s3` table the export uses `storage.s3`'s service and keys | required with `export.s3` |
| `export.s3.endpoint` | As `storage.s3.endpoint`, for the export's service | none |
| `export.s3.path_style` | As `storage.s3.path_style` | `false` |
| `export.s3.access_key_id` | As `storage.s3.access_key_id` | none |
| `export.s3.secret_access_key_file` | As `storage.s3.secret_access_key_file` | none |
| `export.s3.allow_http` | As `storage.s3.allow_http` | `false` |
| `export.s3.ca_file` | As `storage.s3.ca_file` | none |
| `export.s3.timeout` | Seconds one request to the export's service may take | `30` |

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

[limits.timeouts]
read = 60

[mcp]
calls_per_second = 100
concurrent = 100
response_bytes = 1048576
```

A key left out of a table that is present takes its default; the others keep
their written values.

## Storage

Without a `[storage]` table, everything is kept in `data_dir`, as before.
The `[storage]` table is on `main` and in no release yet: release 0.3.0
refuses it as an unknown key, so it needs a
[build from source](@/docs/install.md#build-from-source) until the next
release. With `storage.objects = "s3"`, each version's sealed object goes to a
bucket in an S3-compatible service (RustFS, MinIO or AWS S3) instead, at
`<prefix>tenants/<tenant id>/objects/<version id>`; the catalogs, the
registry, the lock files and the master key stay on the host. An object is
the same sealed bytes the server writes to a file: the bucket's operator
sees random names, sizes and times, and no vCon. The how-to
[keep vCons in an S3 bucket](@/docs/s3-storage.md) sets one up step by
step. A file for a server keeping its objects in a RustFS or MinIO bucket:

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
