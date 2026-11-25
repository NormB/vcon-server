+++
title = "Roadmap"
description = "What vcon-server plans to do through December 2027, by target month, and what it does not plan to do."
weight = 61

[extra]
group = "project"
+++

The plan on this page runs through December 2027. It lists the milestones
by target month, the work that has no month yet, and what the project
does not plan to do. Each month is a target, not a promise: work can move,
and this page changes when it does.
[What is built](@/docs/status.md) says what works today, and
[the 1.0 checklist](@/docs/production-checklist.md) what release 1.0, the
first for production, needs.

## Milestones

- **January 2027: scale numbers at 10 million and 100 million vCons.**
  - Work: runs of the scale harness on stated hardware, with S3, Parquet
    and PostgreSQL storage, for ingest rate and read latency.
  - Status: CI runs the harness at 100,000 vCons today. The
    [scale numbers](@/docs/scale.md#results) page shows each measured run,
    and the [1.0 checklist](@/docs/production-checklist.md#published-scale-numbers)
    tracks the rest.
- **February 2027: more maintainers and more ways to install.**
  - Work: two maintainers with merge rights besides the current one,
    signed multi-arch container images on GitHub Container Registry, a
    Helm chart, a signed APT repository, and a static musl build.
  - Status: [MAINTAINERS.md](https://github.com/NormB/vcon-server/blob/main/MAINTAINERS.md)
    lists one maintainer. The release binaries need
    [glibc 2.34 or later](@/docs/install.md#what-the-host-needs), and
    [Run it in a container](@/docs/container.md) builds an image from a
    release.
- **February 2027: keys kept in a KMS or an HSM.**
  - Work: one interface for wrapping keys, with the key file as one
    backend, and backends for HashiCorp Vault Transit, OpenBao, PKCS#11
    (for HSMs), AWS KMS, Azure Key Vault and Google Cloud KMS. Each has a
    key rotation drill. CI tests the first five against a local service,
    and Google Cloud KMS, which has no local service, against recorded
    responses.
  - Status: [Keys kept in a KMS or an HSM](@/docs/production-checklist.md#keys-kept-in-a-kms-or-an-hsm),
    an item of the 1.0 checklist.
- **April 2027: a conformance suite for the vCon core draft.**
  - Work: a test for each MUST and SHOULD of the core draft, published for
    other implementations to run, and a public pass and fail matrix of runs
    against other vCon implementations.
  - Status: [Standards](@/standards.md) lists the drafts the server
    follows today, and how far.
- **April 2027: events, metrics and traces.**
  - Work: events for ingest, redaction, hold, release, purge and key
    rotation, sent to HMAC-signed webhooks, NATS JetStream and Kafka, with
    identifiers and metadata only, never content. Prometheus metrics,
    OpenTelemetry traces, and a Grafana dashboard with alert rules.
  - Status: no page yet.
- **June 2027: the companion drafts.**
  - Work, in this order: a SCITT transparency receipt at ingest, redaction
    and purge; a lawful basis stored with each vCon and checked at each
    read; the Contact Center extension's fields indexed; the World
    Transcription Format and the SIP Signaling and STIR/SHAKEN extensions
    checked and stored.
  - Status: [Standards](@/standards.md) gains a row for each draft as it
    ships.
- **June 2027: answers for compliance.**
  - Work: erasure of one party's data by crypto-shredding, refused under a
    legal hold. Redaction of card numbers and US Social Security numbers,
    recorded as a vCon redaction. Pages that map the server to GDPR, CCPA,
    HIPAA, PCI DSS, SOC 2 and the US two-party consent laws: what the
    server covers, and what is left to the operator.
  - Status: [Retention and holds](@/docs/retention-and-holds.md) describes
    erasure of a whole vCon and legal holds today.
- **August 2027: search for compliance.**
  - Work: per-tenant blind (HMAC) indexes of party phone, email and name,
    tags, subject, analysis types and extensions. An optional encrypted
    per-tenant full-text search over transcripts. An eDiscovery export with
    an audit trail and a chain of custody.
  - Status: no page yet.
- **August 2027: a demonstration with sipnab.**
  - Work: one Docker Compose stack of OpenSIPS, Asterisk, sipnab capture,
    vcon-server, PostgreSQL, RustFS, a local whisper.cpp transcription
    worker and queries over MCP. A five-minute walkthrough from a placed
    call to questions about it, and talk submissions to conferences.
  - Status: no page yet.
- **September 2027: an outside review of the cryptography, then release 1.0.**
  - Work: a review, from outside the project, of the envelope format, the
    key hierarchy, key rotation and purge, published with each finding
    fixed or tracked. Release 1.0 follows when every item of the 1.0
    checklist is done.
  - Status: [An outside review of the cryptography](@/docs/production-checklist.md#an-outside-review-of-the-cryptography)
    and the other items of the [1.0 checklist](@/docs/production-checklist.md).

## Planned, with no target month

- **A PostgreSQL catalog, the rest of it:** a command that moves a data
  directory's tenants into PostgreSQL and back. From 0.4.6 on, a server
  keeps the registry and the catalogs in PostgreSQL, as a preview behind the `postgres` cargo feature, which the
  release build names (see
  [what is built](@/docs/status.md#the-postgresql-catalog-preview) and
  [the limits of the preview](@/docs/postgres-catalog.md#the-limits-of-the-preview)).
  Several servers on one database is
  [an item of the 1.0 checklist](@/docs/production-checklist.md#several-servers-on-one-postgresql-database).
- **The draft's recommended algorithms** for `vcon::write` (RS256,
  RSA-OAEP and A256CBC-HS512), as an option of the `vcon` crate.

## Not planned

- **A new name.** The project keeps the name vcon-server, and its site
  stays at vcon-server.com.
- **A processing pipeline.** vcon-server stores vCons and returns them. It
  does not run vCons through chains of processing steps, as the
  [vCon Conserver](https://github.com/vcon-dev/vcon-server) does. A worker
  such as a transcriber reads and writes through the HTTP API.
- **Windows builds.** The server is built and tested on Linux on x86_64 and
  aarch64 only (see [what the host needs](@/docs/install.md#what-the-host-needs)).
  No Windows work is planned. Pull requests that add Windows support are
  welcome.
- **Native UTCP support.** The server serves its OpenAPI document at
  `GET /openapi.json` (see [the HTTP API](@/docs/http-api.md)), which a
  client can read.
- **A JavaScript framework or generated user interface in the console.**
  The [web console](@/docs/console.md) stays plain JavaScript under a
  strict Content-Security-Policy, and sends no data it shows to a language
  model.

A milestone moves to [What is built](@/docs/status.md) when it ships. The
[commit history](https://github.com/NormB/vcon-server/commits/main) records
each change as it lands.
