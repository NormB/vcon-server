+++
title = "How-to guides"
description = "Step-by-step guides for installing and running vcon-server. Each shows every command and its output, and CI runs each against a live server."
weight = 20

[extra]
group = "how-to"
+++

Each guide gives every command to paste, in order, and what the command
prints. CI runs the commands of each guide against a live server on each
change, and fails when a command prints something other than the guide
shows. `scripts/docs_test.py` runs them. See
[development](@/docs/development.md#the-how-to-pages-are-tested).

## Install and start

- [Install](@/docs/install.md): download a release and check it, install
  with cargo, or build from source, with what each Linux distribution needs
  first.
- [Getting started](@/docs/getting-started.md): the service account, the
  master key, a certificate, the configuration and the first keys. Then the
  server is started under systemd or in the foreground.
- [Get a Let's Encrypt certificate](@/docs/lets-encrypt.md): a free,
  publicly trusted certificate in place of the self-signed one, by HTTP-01
  or DNS-01 with certbot, and its renewal with a hook that installs it and
  restarts the server. Clients then need no CA file. CI runs it with
  Pebble, Let's Encrypt's test server, in Let's Encrypt's place.

- [Keep vCons in an S3 bucket](@/docs/s3-storage.md): RustFS, MinIO or AWS
  S3 for the sealed objects, the bucket and its key on the storage side, a
  check that the objects go there and that a purge removes them, what each
  refusal at start means, and the clean-up. CI runs it with RustFS and with
  MinIO.
- [Keep vCons as Parquet files in a bucket](@/docs/parquet-storage.md): the
  objects in the bucket packed into Parquet files, a check that each vCon
  still reads back, a read with DuckDB, what a purge removes from the bucket
  and from the storage's disks, and what each refusal means. CI runs it with
  RustFS.
- [Keep the catalogs in PostgreSQL](@/docs/postgres-catalog.md), a
  preview: the database's side, the roles, the schema and
  `vcon-server migrate`, TLS and client certificates, the configuration and
  the secret files, and what each refusal at start means. In this release
  `migrate` makes the schema, and the server connects, checks the server
  and stops. CI runs it with Debian 13's PostgreSQL 17 and a server built
  with the `postgres` feature.

## Use it

- [Store your first vCon](@/docs/first-vcon.md): store, store again, see a
  conflict refused, read, list and delete a vCon, with a key for each scope.
- [Use keys and scopes](@/docs/keys-and-scopes.md): a key for each scope,
  what each may do and what it is refused, and a key with two scopes.
- [Add a second tenant](@/docs/second-tenant.md): a tenant made over the
  admin API, its key, its data kept apart, its retention period, and its
  removal.
- [Browse vCons in a terminal](@/docs/tui.md): `vcon-server tui`, its
  build, the key and client file it needs, its keys and its messages.

## Run it

- [Revoke and rotate keys](@/docs/rotate-keys.md): revoke an API key,
  replace one without a gap, and rotate the tenant key and the master key.
- [Back up and restore](@/docs/backup-and-restore.md): the master key and
  the data directory, a restore, and what it brings back.
- [Read the audit logs](@/docs/read-audit-logs.md): a tenant's log and the
  admin log, with a command on the host and over HTTPS a page at a time,
  and who revealed which part of a vCon.
- [Export to Parquet](@/docs/export-parquet.md): a tenant's vCons as
  Parquet files in a directory or a bucket, beside a running server, the
  clear columns each metadata level adds, a read with DuckDB, and what each
  refusal means.
- [Run it in a container](@/docs/container.md): an image built from a checked
  release, volumes, the memory-lock limit, Docker and Docker Compose.
- [Upgrade](@/docs/upgrading.md): a new binary over a running server, the
  data checked with it, and the way back.
- [Give operator keys their own listener](@/docs/operator-listener.md): a
  second address that serves operator keys only, so the address tenants
  use refuses them, with what to change in proxies and clients.

## Connect an AI agent

- [Connect an AI agent over MCP](@/docs/connect-mcp.md): a key for the
  agent, the handshake and the tool list over HTTPS and through
  `vcon-server mcp`, and the refusals. The tool list holds the tools the
  key's scopes allow.
- [MCP](@/docs/mcp.md), the reference: the endpoint, the stdio command, the
  gate each tool call passes, and every tool with its parameters and an
  example.
