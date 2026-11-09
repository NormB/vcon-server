+++
title = "What is built"
description = "Which parts of vcon-server work today, and which are being built."
weight = 5
+++

vcon-server 0.4.1 is released: the `vcon` and `vcon-server` crates are on
crates.io, and the server for Linux on x86_64 and aarch64 is on
[GitHub](https://github.com/NormB/vcon-server/releases). It is not yet for
production. This page says what works today; each other page describes only
what is built, unless it says otherwise.

## Built and tested

- **Input checks.** The three vCon forms (unsigned, signed, encrypted), plain
  or gzipped, with bounded inflation, duplicate JSON members refused, and the
  `critical` extension list honored. These checks are a separate library
  crate, [`vcon`](@/docs/library.md). Both crates, `vcon` and `vcon-server`,
  are published on crates.io.
- **Writing vCons** in the `vcon` library, released in vcon 0.3.0:
  `vcon::write` builds a vCon from the core draft's object model and writes
  it unsigned or gzipped, refusing what the draft does not allow, or signed
  (a JWS) or encrypted (a JWE) through signers and encrypters the caller
  provides; the draft's examples A.4, A.8 and A.9 are built with it.
  vcon-server itself never writes a vCon.
- **Encrypted storage.** AES-256-GCM per version under a per-version data key,
  tenant keys wrapped by the master key, SQLCipher catalogs, crash-safe writes
  and start-up recovery.
- **Objects in an S3-compatible bucket**, from 0.4.0: RustFS, MinIO or AWS S3
  keeps each version's sealed object, with path-style or virtual-hosted
  addressing, a key from a file or from the AWS credential chain, a private
  certificate authority, versioned buckets refused, and one data directory
  for each bucket and prefix. The store's conformance suite and the how-to
  run against RustFS 1.0.4 and MinIO RELEASE.2025-09-07T16-13-09Z in CI;
  AWS S3 is checked against its documented API only. See
  [keep vCons in an S3 bucket](@/docs/s3-storage.md).
- **Parquet storage in the bucket**, from 0.4.0: the
  objects packed into Parquet files by tenant and store date, a read by
  uuid fetching one row with a ranged read, and a purge that rewrites the
  file and deletes the old one, confirmed with HeadObject, before it is
  done. Versioned and Object Lock buckets are refused. Numbered manifests
  name the files and the live versions for outside readers, small files are
  merged, and a merged file stays for a reader grace that a purge overrides.
  The files hold no clear column; queryable columns come from the export.
  The store's conformance suite
  runs on it against RustFS and MinIO in CI, and the how-to against RustFS.
  See [keep vCons as Parquet files in a bucket](@/docs/parquet-storage.md).
- **Parquet export**, from 0.4.0: `vcon-server export parquet` writes a
  copy of a tenant's kept versions as Parquet files, to a directory or a
  bucket, beside a running server, with the clear columns the tenant's
  metadata level allows. See [export to Parquet](@/docs/export-parquet.md).
- **The HTTP API over TLS.** Create, read, replace and delete; the legal
  routes (reads of deleted vCons, version history, restore, legal holds); the
  admin routes (tenants and their keys); health.
- **The client API.** `GET /vcons` lists a tenant's vCons a page at a time,
  `GET /audit` and `GET /admin/audit` read the audit logs, and
  `GET /openapi.json` describes every route
  ([HTTP API](@/docs/http-api.md)). The [client examples](@/docs/clients.md),
  in curl, Python, TypeScript, Go and Rust, run against a live server in CI.
- **The console:** a web page at `/console/`, signed in with an API key
  held in page memory only; it shows the key's own id, tenant and scopes
  (`GET /key`), lists, reads, stores and deletes vCons as the key's scopes
  allow; the legal view (versions, holds, restore), the audit logs, and for
  an operator key the tenants and their keys.
- **The MCP server** for AI agents, at `/mcp` and through `vcon-server mcp`:
  tools that read, list, store, replace and delete vCons, the legal tools, the
  audit logs, and for an operator key the tenants and their keys; each passes
  one gate for rate, concurrency and scope, and leaves the records its HTTP
  route leaves. See [MCP](@/docs/mcp.md).
- **Tenants and keys.** Scoped bearer keys, revocation, tenant removal that
  never passes a legal hold.
- **Retention.** Calendar periods per tenant or server-wide, the purge sweep
  at start and on an interval, and the purge of a deleted vCon's due versions.
- **Audit logs.** One per tenant and one for administration.
- **Hardening.** Memory locked and out of swap, core dumps off, one server per
  data directory.
- **Configuration.** One TOML file; unknown keys refused; defaults measured
  from real vCons.
- **The command line** for tenants, keys (including operator keys) and
  audit export, with the rules on which commands may run beside the server.
- **Key rotation:** `rekey tenant` (data keys rewrapped and the tenant's
  catalog re-encrypted under a new tenant key) and `rekey master` (every
  tenant key rewrapped under a new master key, in a copy of `server.sqlite`
  that replaces it only once complete). A stopped run of either finishes when
  run again.
- **`check`:** every stored version opened, leftover files listed, versions
  past due and work waiting for key rotation reported; exit 1 when anything
  needs the operator.
- **Start-up refusals** for a tenant key wrapped under another key, a key
  rotation stopped part way, data under another key, or a tenant catalog
  the server may not open, with the system's error; the daily warning
  while key rotation is overdue; and a log line with the number of objects
  start-up recovery removed.
- **The operator guide.** [Getting started](@/docs/getting-started.md) takes
  a fresh host to a running server under systemd, from a release, from
  crates.io or from source; [operations](@/docs/operations.md) covers
  backups, restores and key rotation. CI runs every step of the
  [how-to guides](@/docs/how-to.md) on each change, in fresh Debian 13 and
  Fedora 44 containers with the server in the foreground, and on Debian 13
  under systemd.
- **Fuzzing.** Every parser of outside input has a
  [fuzz target](@/docs/development.md#fuzzing), run in CI and by
  ClusterFuzzLite.
- **Releases.** Each release holds the server for Linux on x86_64 and
  aarch64, built to run on glibc 2.34 or later, a CycloneDX SBOM and
  checksums, every file with a signed build provenance attestation; from
  0.3.0 also a Sigstore signature over the checksums and the provenance as a
  file. The release workflow also publishes both crates to crates.io.

## Being built

- **A PostgreSQL catalog**, a preview behind the `postgres` cargo feature:
  `[catalog.postgres]` is read and checked, and a start reads its secret
  files once, connects over TLS with the certificate and host name
  verified, logs in by SCRAM-SHA-256 with channel binding or a client
  certificate, refuses a standby, a read-only server, a superuser and a role
  with `BYPASSRLS`, and then stops, since the catalogs are not kept there
  yet. Its tests run against PostgreSQL 16 and 18 in CI. See
  [keep the catalogs in PostgreSQL](@/docs/postgres-catalog.md).
- **The draft's recommended algorithms** for `vcon::write` (RS256,
  RSA-OAEP and A256CBC-HS512) as an option of the `vcon` crate, and a check
  of its output with another JOSE library, for a release after vcon 0.4.0.
- **Review:** a review of the code against its design, a review of every
  feature against its tests, and a full fault-injection run.

The [commit history](https://github.com/NormB/vcon-server/commits/main)
records each change as it lands, with the reason in its message.
