+++
title = "What is built"
description = "Which parts of vcon-server work today, and which are being built."
weight = 60

[extra]
group = "project"
+++

vcon-server 0.4.5 is released: the `vcon` and `vcon-server` crates are on
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
- **The terminal UI:** `vcon-server tui`, in the release binaries from
  0.4.2 on, lists the vCons of one key's tenant and shows each one's uuid,
  version and times, read over the HTTP API with a `read` key. It reads
  only, and never a vCon's content. From 0.4.5 on, the last line of each
  screen names only the keys that act on it; releases 0.4.2 to 0.4.4 show
  the list's keys on the connecting screen, an empty tenant and every
  failure. See [the terminal UI](@/docs/tui.md).
- **Reveals in the audit log**, from 0.4.3:
  `GET /vcons/{uuid}?reveal=<part>`, and the same parameter on the two
  legal read routes, records the read as `reveal_<part>` or
  `legal_reveal_<part>`, and the console marks each of its reveals that
  way. Release 0.4.2 has no `reveal` parameter: it passes the query over
  and records `read` or `legal_read`. See
  [reveals](@/docs/http-api.md#reveals).
- **An operator listener**, from 0.4.3: with an `[operator]` table in the
  configuration, the server listens on a second address that serves
  operator keys only, and `listen` serves tenant keys only; a valid key at
  the other address is refused with `wrong-listener`. Release 0.4.2 has
  one listener for both kinds of key, and refuses a configuration with
  that table. See [the operator listener](@/docs/operator-listener.md).
- **A recording player in the console**, from 0.4.3: a dialog's recording
  that is inside the vCon plays in a tab of its own,
  `/console/player.html`, whose policy alone lets a blob play and lets the
  page make no request. Each Play is recorded as `reveal_audio` or
  `legal_reveal_audio`. Release 0.4.2 has no Play button and no such page.
  See [play a recording](@/docs/console.md#play-a-recording).
- **Connection limits**, from 0.4.3: a listener holds at most
  `limits.connections` connections at once, and closes a connection that
  stalls in its TLS handshake or in a request head, or that waits longer
  than `limits.timeouts.idle` for its next request; a listener that fills
  is logged and counted in the admin audit log as `listener_full`. Release
  0.4.2 has none of the five keys (`limits.connections` and
  `limits.timeouts.handshake`, `first_head`, `head` and `idle`) and refuses
  a configuration that has one: it takes every connection it is offered
  and keeps each for as long as the client does. See
  [connection limits](@/docs/configuration.md#connection-limits).
- **Reports of a broken console policy**, from 0.4.3: both console
  policies name `POST /console/csp-report` as the place a browser reports
  a page that breaks them, and each report is counted in the admin audit
  log as `csp_report`. Release 0.4.2 sends policies with no reporting
  directive and answers 404 at that path. See
  [reports of a broken policy](@/docs/console.md#reports-of-a-broken-policy).
- **Users and memberships, and schema version 9**, from 0.4.4: the
  commands `user add`, `user list`, `user deactivate`, `membership grant`
  and `membership revoke` keep a list of people and their roles in a
  tenant or as an operator, on the host, in `server.sqlite`, with each
  change in the audit logs. No sign-in method exists yet, so nobody can
  sign in as a user. The catalogs move from schema version 4 to 9, which
  release 0.4.3 refuses to open: there is no downgrade, and the only way
  back is the copy of the data directory taken before the upgrade.
  Release 0.4.3 has none of these commands and keeps schema version 4.
  See [add users and memberships](@/docs/users-and-memberships.md) and
  [schema version 9](@/docs/upgrading.md#schema-version-9-and-no-way-back-without-the-copy).
- **Roles as scopes, and a tenant's role cap**, from 0.4.4: each role of
  a membership gives one scope, and `user list` prints a membership's
  `scopes` beside its `roles`. The commands `tenant set-role-cap` and
  `tenant role-cap` set and print the roles a membership in a tenant may
  be given, recorded as `role_cap_set` and `role_cap_read`. A tenant key
  may hold the new scope `tenant_admin`, which the role `tenant-admin`
  gives. Release 0.4.3 has neither command, and refuses `tenant_admin` as
  a key's scope. See
  [roles, scopes and a tenant's role cap](@/docs/roles-and-scopes.md).
- **Routes for a tenant's administrator**, from 0.4.4: a key of a tenant
  that holds `tenant_admin` reads and changes the tenant's settings at
  `/tenant/settings`, makes, lists and revokes the tenant's keys at
  `/tenant/keys`, within the tenant's role cap, and lists its memberships
  at `/tenant/memberships`. Each call is in the tenant's audit log, as
  `settings_read`, `settings_set`, `membership_list` or one of the key
  actions, and a change outside the list of settings is refused with
  `bad-setting`. The settings (`masking.<role>`, `session.idle_seconds`
  and `session.absolute_seconds`) are stored and answered; a session takes
  the two limits when it starts, and nothing reads the masking settings
  yet. Release 0.4.3 has none of these routes. See
  [administer a tenant with a key](@/docs/tenant-administration.md).
- **Sessions, with no way to start one yet**, from 0.4.4: the server
  keeps sessions in `server.sqlite`, by the SHA-256 of their ids, and
  serves `GET /session` and `POST /session/logout` to a request with the
  cookie `__Host-vcs_session` that comes from the listener's public
  origin (`public_origin`, `operator.public_origin`) and, for the
  sign-out, carries the session's CSRF token. A change of a user's roles,
  a deactivation and a change of a tenant's role cap end the sessions
  they touch. A request with both a key and the cookie is refused with
  `key-and-session`. No sign-in method exists in 0.4.4: nothing starts a
  session, no browser holds one, nobody can sign in as a user, and the
  routes that take a key take no session. Release 0.4.3 has none of
  this. See
  [set a listener's public origin for sessions](@/docs/session-origin.md).
- **Key expiry and last-used time**, from 0.4.4: a new key expires 90
  days after it is made, or in `expires_in_days` days (1 to 365), or
  never with `"no_expiry": true`; `key create` takes `--expires-days` and
  `--no-expiry`, and `keys.no_expiry` in the configuration can refuse new
  keys with no expiry. A request by an expired key is refused with
  `key-expired`, and an expiry a key cannot have with `bad-expiry`. Every
  list of keys shows `expires_at` and `last_used_at`, the minute of the
  key's latest accepted request. A key made by release 0.4.3 or earlier
  keeps working with no expiry. Release 0.4.3 has none of this. From
  0.4.5 on, the `expires_at` of a new key is a whole second; a key made
  by 0.4.4 may have one with a fraction of a second. See
  [administer a tenant with a key](@/docs/tenant-administration.md#set-a-key-s-expiry-and-see-when-it-was-last-used).
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
  with `BYPASSRLS`, a role that can change the schema and a schema of any
  other version than the binary's, and then stops, since the catalogs are
  not kept there yet. `vcon-server migrate` makes the schema, with every
  table under row-level security that gives a query with no tenant no row and refuses it at the first row it examines. Its tests run against PostgreSQL 16 and 18 in CI, and its how-to against Debian 13's PostgreSQL 17. See
  [keep the catalogs in PostgreSQL](@/docs/postgres-catalog.md).
- **The draft's recommended algorithms** for `vcon::write` (RS256,
  RSA-OAEP and A256CBC-HS512) as an option of the `vcon` crate, and a check
  of its output with another JOSE library, for a release after vcon 0.4.0.
- **Review:** a review of the code against its design, a review of every
  feature against its tests, and a full fault-injection run.

The [commit history](https://github.com/NormB/vcon-server/commits/main)
records each change as it lands, with the reason in its message.
