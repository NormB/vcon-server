+++
title = "vcon-server"
description = "vcon-server stores vCons, the IETF format for conversation records, and returns them only to the keys allowed to see them. Each vCon is encrypted at rest under its own key, kept for its tenant's retention period, erased after it, and held past it when a legal hold says so. Every request is recorded."
[extra]
tagline = "encrypted, multi-tenant, audited storage for vCons"
headline = "Encrypted, multi-tenant, audited storage for vCons"
+++

> **Status: in development, not yet for production.** The store, the HTTP API,
> tenants and their keys, retention and the purge sweep, legal holds, the audit
> logs, the command line, key rotation and `check` are built and tested. The
> last start-up checks and the release work are next. See
> [what is built](@/docs/status.md).

## What it is for

A vCon holds a conversation: who took part, the dialog itself (text, or a
recording inline or by URL), and what was learned from it (transcripts,
summaries, analysis). That is some of the most sensitive data a business keeps,
and much of it must be kept for a fixed time and then destroyed, unless a
court says otherwise.

vcon-server is the place a producer sends vCons to and the place they are read
back from: an HTTP API over an encrypted store, with tenants kept apart, every
request recorded, and retention periods and legal holds applied to each vCon.

## What it guarantees

- **Encrypted at rest, key by key.** Each stored version is sealed with
  AES-256-GCM under its own data key, wrapped by its tenant's key, which is
  wrapped by the master key. The catalogs are SQLCipher databases. See
  [security](@/docs/security.md).
- **Stored exactly as received.** Unsigned, signed (JWS) and encrypted (JWE)
  vCons, plain or gzipped, are checked and stored byte for byte. Nothing is
  rewritten, so a signature stays valid. See [storing vCons](@/docs/storing-vcons.md).
- **Tenants kept apart.** Each tenant has its own key, catalog, files, retention
  period and audit log. A key belongs to one tenant and reaches nothing else.
  See [tenants and keys](@/docs/tenants-and-keys.md).
- **Kept as long as required, then erased.** A deleted vCon is hidden at once
  and purged once its tenant's retention period has passed since it was stored.
  A legal hold stops the purge. See [retention and legal holds](@/docs/retention-and-holds.md).
- **Every request recorded.** Each tenant has an append-only audit log, and the
  server keeps one for its own administration. No log holds vCon content or a
  token. See [audit logs](@/docs/audit-logs.md).
- **Errors you can act on.** Every refusal is an RFC 9457 problem document
  whose `type` is a page on this site: [problem types](@/problems/_index.md).

## A first look

Send a vCon with a key that holds the `write` scope, then read it back:

```sh
curl --fail-with-body https://vcon.example.com/vcons \
  -H "Authorization: Bearer $VCON_KEY" \
  -H "Content-Type: application/vcon" \
  --data-binary @call.vcon
# 201 Created
# Location: /vcons/01a07da8-c2bb-83e5-b9a2-279e0d16bc46
# ETag: "…version id…"

curl --fail-with-body https://vcon.example.com/vcons/01a07da8-c2bb-83e5-b9a2-279e0d16bc46 \
  -H "Authorization: Bearer $VCON_KEY"
```

Sending the same bytes again answers `200` with the same version, so a producer
can retry safely. The [HTTP API](@/docs/http-api.md) lists every route.

## Where to start

- **Running a server:** [getting started](@/docs/getting-started.md), then
  [configuration](@/docs/configuration.md) and [operations](@/docs/operations.md).
- **Sending vCons from a producer:** [producers](@/docs/producers.md) and the
  [HTTP API](@/docs/http-api.md).
- **Working on the code:** [development](@/docs/development.md).
