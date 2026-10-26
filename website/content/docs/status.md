+++
title = "What is built"
description = "Which parts of vcon-server work today, and which are being built."
weight = 5
+++

vcon-server is in development and not yet for production. This page says what
works today; each other page describes only what is built, unless it says
otherwise.

## Built and tested

- **Input checks.** The three vCon forms (unsigned, signed, encrypted), plain
  or gzipped, with bounded inflation, duplicate JSON members refused, and the
  `critical` extension list honoured. These checks are a separate library
  crate, `vcon`.
- **Encrypted storage.** AES-256-GCM per version under a per-version data key,
  tenant keys wrapped by the master key, SQLCipher catalogs, crash-safe writes
  and start-up recovery.
- **The HTTP API over TLS.** Create, read, replace and delete; the legal
  routes (reads of deleted vCons, version history, restore, legal holds); the
  admin routes (tenants and their keys); health.
- **The console:** a web page at `/console/`, signed in with an API key
  held in page memory only; it shows the key's own id, tenant and scopes
  (`GET /key`), lists, reads, stores and deletes vCons as the key's scopes
  allow, and shows a vCon's versions to a legal key.
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
  rotation stopped part way, or data under another key; and the daily
  warning while key rotation is overdue.

## Being built

- **The console's views:** legal holds and restore, the admin views, and
  the audit logs.
- **Release:** an operator guide, a review of the code against its design,
  and a full fault-injection run.

The [commit history](https://github.com/NormB/vcon-server/commits/main)
records each change as it lands, with the reason in its message.
