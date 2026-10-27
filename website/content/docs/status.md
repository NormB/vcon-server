+++
title = "What is built"
description = "Which parts of vcon-server work today, and which are being built."
weight = 5
+++

vcon-server 0.2.0 is released: the `vcon` and `vcon-server` crates are on
crates.io, and the server for Linux on x86_64 and aarch64 is on
[GitHub](https://github.com/NormB/vcon-server/releases). It is not yet for
production. This page says what works today; each other page describes only
what is built, unless it says otherwise.

## Built and tested

- **Input checks.** The three vCon forms (unsigned, signed, encrypted), plain
  or gzipped, with bounded inflation, duplicate JSON members refused, and the
  `critical` extension list honoured. These checks are a separate library
  crate, [`vcon`](@/docs/library.md). Both crates, `vcon` and `vcon-server`,
  are published on crates.io.
- **Encrypted storage.** AES-256-GCM per version under a per-version data key,
  tenant keys wrapped by the master key, SQLCipher catalogs, crash-safe writes
  and start-up recovery.
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
  the server may not open, with the system's error; and the daily warning
  while key rotation is overdue.
- **The operator guide.** [Getting started](@/docs/getting-started.md) takes
  a fresh host to a running server under systemd, from a release, from
  crates.io or from source; [operations](@/docs/operations.md) covers
  backups, restores and key rotation. Every step was run on a fresh
  Debian 13 host from the 0.1.0 release.
- **Fuzzing.** Every parser of outside input has a
  [fuzz target](@/docs/development.md#fuzzing), run in CI and by
  ClusterFuzzLite.
- **Releases.** Each release holds the server for Linux on x86_64 and
  aarch64, a CycloneDX SBOM and checksums, every file with a signed build
  provenance attestation; the release workflow also publishes both crates to
  crates.io.

## Being built

- **MCP:** a Model Context Protocol endpoint on the TLS listener, and a
  command for clients that speak it over standard input and output, using
  the same API keys and scopes as the HTTP API. It is being built on
  branches and is not on `main` yet.
- **Review:** a review of the code against its design, a review of every
  feature against its tests, and a full fault-injection run.

The [commit history](https://github.com/NormB/vcon-server/commits/main)
records each change as it lands, with the reason in its message.
