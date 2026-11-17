+++
title = "The 1.0 checklist"
description = "What vcon-server needs before release 1.0, the first release for production: each item's state, what is left, and the evidence."
weight = 63

[extra]
group = "project"
+++

Release 1.0 is the first release of vcon-server for production. These are
the things it needs. Each item states what is done and what is left, and
links its evidence: a page of this site, a file in the repository, a test
or a CI job. An item is stated as done only when a CI job runs its
evidence. [What is built](@/docs/status.md) says what works today, and the
[roadmap](@/docs/roadmap.md) lists the next milestones.

A test in the repository holds this page to the code: each link must lead
to a page and heading of the site, a file in the repository, a test its
file defines or a job its workflow defines.

## Several servers on one PostgreSQL database

**State:** Partly done.

The catalogs can be kept in PostgreSQL, a preview behind the `postgres`
cargo feature, which the release binaries have. CI runs the store's
conformance suite and a server on PostgreSQL 16 and 18. One server serves
a database: a second server, on any host, is refused at start.

Left: a command that moves a data directory's tenants into PostgreSQL and
back; several servers serving one database; and tests in CI that cut the
connection to PostgreSQL at each step of a write and fail over from a
primary to a standby, with every acknowledged write read back once.

Evidence:

- [the limits of the preview](@/docs/postgres-catalog.md#the-limits-of-the-preview)
- CI job `backends-postgres` in
  [ci.yml](https://github.com/NormB/vcon-server/blob/main/.github/workflows/ci.yml)
- test `a_second_server_on_the_database_is_refused` in
  [serve.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/tests/serve.rs)

## A backup restored in CI

**State:** Partly done.

CI follows the backup how-to on a live server whose objects are in the
data directory: it backs the data directory up, changes it, restores the
backup, checks the restored copy and serves it. A test takes a backup while
vCons are being written, checks it and serves it.

Left: the same in CI with the objects in an S3 bucket, with Parquet
storage and with the catalogs in PostgreSQL; a backup taken before a
master key rotation; a backup made by the previous release; and, in each,
a comparison of every restored vCon with what was stored.

Evidence:

- [restore the data directory](@/docs/backup-and-restore.md#restore-the-data-directory)
- [objects in an S3 bucket](@/docs/backup-and-restore.md#objects-in-an-s3-bucket),
  which CI does not run yet
- CI job `docs` in
  [ci.yml](https://github.com/NormB/vcon-server/blob/main/.github/workflows/ci.yml),
  which runs each how-to against a live server
- the how-tos each CI run follows, in
  [docs_test.py](https://github.com/NormB/vcon-server/blob/main/scripts/docs_test.py)
- test `a_backup_taken_under_write_load_checks_whole_and_serves` in
  [serve.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/tests/serve.rs)

## Keys kept in a KMS or an HSM

**State:** Not started.

The master key is a file the server reads at start and keeps in locked
memory. The key of the registry is derived from the master key's bytes,
which a key kept in a KMS or an HSM cannot give, so the registry needs a
key of its own first: a change of the on-disk format.

Left: one interface for wrapping keys, with the key file as one backend;
backends for HashiCorp Vault Transit, OpenBao, PKCS#11 (for HSMs) and AWS
KMS, each tested in CI against a local service; and a key rotation drill
for each. Google Cloud KMS and Azure Key Vault have no local service to
test against; whether to add them is not decided.

Evidence:

- [keys](@/docs/security.md#keys)
- the key file and the derivation, in
  [envelope.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/src/envelope.rs)

## An outside review of the cryptography

**State:** Not started.

The threat model, the trust boundaries and the evidence for them are in
the assurance case. No reviewer from outside the project has read the
envelope format, the key hierarchy, key rotation or purge.

Left: a specification of the envelope format and the key hierarchy, with
test vectors that a second implementation checks in CI; the review itself;
and its report, published with each finding fixed or tracked.

Evidence:

- [the threat model](@/docs/assurance-case.md#threat-model)
- the security policy, in
  [SECURITY.md](https://github.com/NormB/vcon-server/blob/main/SECURITY.md)

## Published scale numbers

**State:** Not started.

No ingest rate, read latency or largest number of stored vCons has been
measured. The one performance figure CI tests is the number of round trips
each call of the store makes to PostgreSQL.

Left: a harness that measures the ingest rate and the p99 read latency, at
100,000 vCons in CI and at 10 million and 100 million vCons on stated
hardware, and a page of its results that names the hardware and the
commit.

Evidence:

- test `a_store_call_costs_a_fixed_number_of_round_trips` in
  [catalog_tests.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/src/backend/postgres/tests/catalog_tests.rs)

## A frozen HTTP API, configuration and on-disk format

**State:** Partly done.

The HTTP API is described in OpenAPI, and a test holds every route to the
HTTP API page. Every table of the configuration file refuses a key it does
not know. [On-disk formats](@/docs/on-disk-format.md) lists every on-disk
format with its version and what each release wrote, and a test holds the
page to the code. The repository keeps a data directory written by each
released binary from 0.4.0 to 0.4.9; on every change, CI opens each one
with the new build, checks it, reads every kept version back byte-equal,
and refuses a catalog one version past the build. From 0.4.6 to 0.4.9 it
also keeps each release's deployment with its catalogs in PostgreSQL and
its objects in Parquet storage, which CI restores and opens the same way on
PostgreSQL 16 and 18. CI also upgrades the previous release to this one and
back.

Left: a written rule for what 1.0 keeps unchanged; and checks in CI that
refuse a change that breaks the HTTP API or the configuration, against a
frozen copy of each.

Evidence:

- the OpenAPI description, in
  [openapi.json](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/openapi.json)
- test `every_route_is_on_the_http_api_page_and_every_row_is_a_route` in
  [docs_pages.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/tests/docs_pages.rs)
- [on-disk formats](@/docs/on-disk-format.md#what-each-release-wrote)
- test `every_format_of_this_build_is_on_the_page_with_its_version` in
  [formats.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/tests/formats.rs)
- the data of each release, in
  [manifest.json](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/tests/data/formats/0.4.0/manifest.json)
  and the directories beside it
- [the data in PostgreSQL and Parquet storage](@/docs/on-disk-format.md#the-data-in-postgresql-and-parquet-storage)
- the PostgreSQL data of each release, in
  [manifest.json](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/tests/data/formats-postgres/0.4.6/manifest.json)
  and the directories beside it
- test `every_fixture_has_its_test_module_and_every_module_a_fixture` in
  [formats.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/tests/formats.rs)
- CI job `backends-postgres` in
  [ci.yml](https://github.com/NormB/vcon-server/blob/main/.github/workflows/ci.yml),
  which restores each release's PostgreSQL data on PostgreSQL 16 and 18
- [before you upgrade](@/docs/upgrading.md#before-you-upgrade)
- test `a_failed_upgrade_from_0_4_2_keeps_none_of_its_steps` in
  [v6_tests.rs](https://github.com/NormB/vcon-server/blob/main/crates/vcon-server/src/catalog/v6_tests.rs)
- CI job `docs` in
  [ci.yml](https://github.com/NormB/vcon-server/blob/main/.github/workflows/ci.yml),
  which runs the upgrade how-to from the previous release
