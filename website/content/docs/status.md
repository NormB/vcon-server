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
  crate, `vcon-form`.
- **Encrypted storage.** AES-256-GCM per version under a per-version data key,
  tenant keys wrapped by the master key, SQLCipher catalogs, crash-safe writes
  and start-up recovery.
- **The HTTP API over TLS.** Create, read, replace and delete; the legal
  routes (reads of deleted vCons, version history, restore, legal holds); the
  admin routes (tenants and their keys); health.
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

## Being built

- **Key rotation** (`rekey tenant`, `rekey master`), which makes purged vCons
  unreadable on old disks and backups too.
- **`check`**, which reports damaged or missing objects, versions past due,
  and work waiting for rotation.
- **The remaining start-up refusals** and the rotation warning.
- **Release:** an operator guide, a review of the code against its design,
  and a full fault-injection run.

The [commit history](https://github.com/NormB/vcon-server/commits/main)
records each change as it lands, with the reason in its message.
