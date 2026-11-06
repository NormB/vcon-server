# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.4.0] - 2026-11-06

### Added

- `vcon-server storage move-objects` moves every tenant's sealed objects from
  the data directory to the bucket `storage.objects = "s3"` names, while no
  server runs: each object is written, read back and compared byte for byte
  before its file is removed, so a run that stops loses nothing and the
  next one moves the rest. It prints, per tenant, how many moved and how
  many versions `check` then finds without an object. A server that kept
  its objects locally moves to Parquet storage with it, then with
  `[storage.parquet]`.

- Parquet storage: with `[storage.parquet]` beside `storage.objects = "s3"`,
  the server packs each tenant's objects in the bucket into Parquet files,
  one store date at a time, at
  `<prefix>tenants/<tenant id>/parquet/date=<yyyy-mm-dd>/part-<file id>.parquet`,
  in a pack run at start and every `pack_interval` seconds (300 unless set);
  a sealed object over `pack_limit` (16 MiB unless set) stays an object. A
  read fetches a packed version's row with one ranged read. A purge lists
  the version's file in the purge's own transaction, rewrites the file
  without the row, deletes the old file and checks with HeadObject that it
  is gone, before `purge_due` returns; start-up recovery and the next sweep
  finish one cut off. A bucket with versioning on or suspended, whatever
  `allow_versioning` says, or with Object Lock on, is refused for Parquet
  storage at start. Start-up recovery removes a pack file no catalog row
  names, and the object of a packed version. A backup writes each packed
  version as an object file, so it restores as before. Removing a tenant
  removes its pack files. Each change of which files hold which rows, and
  each pack run that finds the live versions changed, writes the next
  numbered manifest (`<prefix>tenants/<tenant id>/manifest/<20 digits>.json`,
  `If-None-Match: *`) naming the files and the live versions; start-up
  recovery writes one left unwritten. Each pack run merges a date's small
  files into one; a file merged away and an older manifest stay for
  `reader_grace` seconds (900 unless set), and a purge deletes them at once.
  The files hold a version's id, store time and sealed bytes only, whatever
  the tenant's metadata level; queryable columns come from `export
  parquet`. On `main` only. The how-to [keep vCons as Parquet files
  in a bucket](https://vcon-server.com/docs/parquet-storage/) turns it on
  over an existing bucket, and CI runs the store's conformance suite on it
  against RustFS and MinIO.

- vcon-server can keep its sealed vCon objects in a bucket of an
  S3-compatible service (RustFS, MinIO, AWS S3): `storage.objects = "s3"`
  and the `[storage.s3]` table, with the endpoint, region, bucket, prefix,
  path style (RustFS, MinIO) or virtual-hosted style (AWS S3), an access
  key id with its secret in a file or the AWS default credential chain,
  `ca_file` for a service whose certificate a private certificate authority
  signed, and a request timeout. Each object holds the same sealed bytes as
  the local file, at `<prefix>tenants/<tenant id>/objects/<version id>`;
  the catalogs, `server.sqlite`, the locks and the master key stay in
  `data_dir`. A purge removes the object from the bucket. A bucket with
  versioning on is refused unless `allow_versioning` is set. The first data
  directory to start on an empty bucket and prefix claims it
  (`storage.claim`, `<prefix>vcon-server.claim`), and any other is refused
  there, since start-up recovery removes every object no catalog of its own
  names. A start that cannot use the bucket names the cause: the service's
  error code, a refused connection or an untrusted certificate. Without a
  `[storage]` table nothing changes. Built with the `s3` cargo feature, on
  by default. The how-to [keep vCons in an S3
  bucket](https://vcon-server.com/docs/s3-storage/) sets up RustFS step by
  step, and CI runs the store's conformance suite against RustFS.
- CI runs the store's conformance suite, the S3 backend's tests and `serve`
  against MinIO RELEASE.2025-09-07T16-13-09Z as well (job "storage backends
  (MinIO)"), including a check that the service refuses a second write with
  `If-None-Match: *`, and runs the S3 how-to with MinIO as well as RustFS.
- Start-up recovery logs how many objects it removed (`start-up recovery
  removed objects that no catalog row names removed=<n>`), for local files
  and for objects in a bucket. Before, it removed them without a word.
- Documentation: the S3 how-to gives MinIO and AWS S3 their own setup,
  checks and troubleshooting, the services' required support for
  conditional writes, every start-up refusal with the server's message, and
  a clean-up; a test reads its AWS configuration and IAM policies. The
  audit-logs page lists every action; the command line page has `serve`;
  the client examples show gzip, `If-Match`, paging, a legal hold, an audit
  read, an admin call and MCP; the library page shows `vcon::write`. Tests
  hold the HTTP API page to the router and the command line page to the
  binary, both ways.
- `vcon-server export parquet --tenant <id> --to <dir | s3://bucket/prefix>
  [--since <time>] [--until <time>]` writes a copy of a tenant's kept
  versions, current and hidden, as Parquet files under
  `tenant=<id>/date=<yyyy-mm-dd>/`, then a numbered manifest,
  `tenant=<id>/manifest/<seq>.json`, naming them. It runs beside a serving
  server and changes nothing in the store but one `export` record in the
  tenant's audit log. Each row holds the version's sealed object byte for
  byte. Each file is written whole under its own name (a temporary name and
  a rename in a directory; one PutObject with `If-None-Match: *` in a
  bucket), the manifest last, and a run that fails removes its files. A
  bucket target is reached with the new `[export.s3]` table, or with
  `[storage.s3]`'s service and keys. Built with the `parquet` crate's
  column writer, without Arrow and without a compression codec. The how-to
  [export to Parquet](https://vcon-server.com/docs/export-parquet/) reads
  an export with DuckDB, from a directory and from a bucket.
- Each tenant has a metadata level for its Parquet exports: `none` (the
  default: only the version id, the store time and the sealed body),
  `counts` (also the form, gzip, size, the vCon's times, and its party and
  dialog counts) or `full` (also the uuid, the subject and the parties'
  `tel`, `mailto` and `name`). It is set with `vcon-server tenant
  set-metadata` or `PUT /admin/tenants/{id}/metadata`, recorded in both
  logs, and shown by the tenant list (`metadata`). Statistics are written
  only for the time and count columns, and no column has a dictionary page.
- The `vcon` crate reads, in `vcon::read::fields`, the members an export
  copies from an unsigned vCon or the vCon inside a signed one: `created_at`,
  `updated_at`, `subject`, each party's `tel`, `mailto` and `name`, and the
  number of dialogs.

### Changed

- The tests run under cargo-nextest, pinned to 0.9.148 by
  `scripts/nextest.sh`, in the commit hook and in CI: a test still running
  after 60 seconds is reported by name, and after 3 minutes it is ended and
  fails, where one hung test used to hang the hook. The doc tests still run
  under `cargo test --doc`. Test builds on aarch64 Linux link with the Rust
  toolchain's own lld (`scripts/cc-lld.sh`), and fall back to the system
  linker when the toolchain has none; release binaries link as before.

- Every catalog is at schema version 4: `server.sqlite` has each tenant's
  metadata level, and each tenant's catalog has a place for each version
  in a Parquet pack file and the tables of its pack files, all empty until
  a later change packs versions. A server opens an older catalog and brings
  it up to date. Release 0.3.0 does not open a data directory at version 3
  or 4: go back to it only with the copy taken before the upgrade, as
  [upgrade](https://vcon-server.com/docs/upgrading/) describes.
- Once a tenant has a version packed into a Parquet file, `serve` refuses
  to start with a configuration that has no `[storage.parquet]`, naming the
  table: without it the server packed no more, and its start no longer
  refused a bucket with Object Lock or versioning, either of which keeps the
  bytes of a pack file that a purge rewrote. `vcon-server check` gives each
  tenant's `packed` versions and reports packed versions without the table
  as a problem; the other commands run as before.

### Fixed

- A purge sweep whose version had already lost its object file (removed by
  hand, or by a purge cut off after its commit) stopped with an I/O error
  after deleting the version's row, so the rest of the sweep and the WAL
  checkpoint that follows it did not run. A missing object now counts as
  removed.
- A read whose object the storage failed to give answered `500 damaged`,
  and `vcon-server check` listed the version as damaged, though the stored
  data may be sound: an object file the process may not read or that the
  disk fails to read, a key page the system would not lock, and with the
  objects in an S3 bucket a service that cannot be reached, refuses the key
  or does not answer in time. These now answer `500 internal`, and the
  server logs `a stored version's object could not be read` with the
  failure's `kind` (such as `permission denied` or `timed out`), never its
  text, a key or an id. `damaged` is now only an object that is missing
  while its row is kept, or one that does not decrypt and authenticate.
- For a tenant whose objects the storage did not read, `vcon-server check`
  reported `its catalog cannot be opened: Io`, though the catalog had
  opened. It now reports `its objects cannot be read:` with the failure's
  kind and, when the system gives one, its error number, such as
  `permission denied (os error 13)`, as the server's log line gives them,
  never an object's key; any other failure of the examination is
  `it could not be examined:` with its error.
- The vcon-server crate's description and the OpenAPI description spelled
  "authorised", and four site pages and llms.txt spelled "behaviour",
  "licence" and "honour(ed)"; each is now in US spelling, and a test holds
  every `Cargo.toml` description, every site page, llms.txt and the OpenAPI
  description to the README's US spelling. The development page's
  "Licence" heading is now "License".

## [0.3.0] - 2026-11-04

### Added

- The MCP server for AI agents: `POST /mcp` on the TLS listener, and
  `vcon-server mcp --config <client file>`, which speaks MCP on stdin and
  stdout and forwards to a running server with one key. Every request is
  authenticated with the bearer key as every route is, and counted in the
  key's tenant log or the admin log as action `mcp`. 19 tools: `vcon_get`,
  `vcon_list`, `vcon_store`, `vcon_replace` and `vcon_delete`; `legal_get`,
  `legal_versions`, `legal_restore`, `legal_hold` and `legal_release`;
  `audit_read`; and for operator keys `admin_tenant_create`,
  `admin_tenant_list`, `admin_tenant_retention`, `admin_tenant_remove`,
  `admin_key_create`, `admin_key_list`, `admin_key_revoke` and
  `admin_audit_read`. Each tool calls the function its HTTP route calls and
  leaves the same audit records. Every call passes one gate, in order: the
  key's rate (`mcp.calls_per_second`), a free slot (`mcp.concurrent`), a
  known tool, the tool's scope, and arguments that fit the tool. The
  problems `rate-limited`, `at-capacity`, `unknown-tool` and
  `invalid-arguments` name the refusals. A result holds at most
  `mcp.response_bytes` (1 MiB by default), with `next_cursor` to resume, and
  a vCon's text comes back between untrusted-data markers it cannot close.
- `vcon::write` in the vcon library: an object model of the vCon core
  draft (parties, dialogs of each type, attachments, analysis, extension
  members) and its unsigned and gzip forms. The output is compact JSON
  with members in the draft's order, so the same model gives the same
  bytes, and `read::inspect` reads every output back with the same form,
  gzip flag and uuid. It refuses what core-04 does not allow and names the
  JSON path: an index to nothing, a member Table 1 forbids or a missing one
  it requires, content without an https URL and a hash, an inline
  attachment without a mediatype, both `amended` and `redacted`, an
  extension member with a reserved name, and output deeper than
  `read::MAX_JSON_DEPTH`. `Vcon::amend` and `Vcon::redact` copy a vCon
  under a new uuid and link the prior one; a removed entry leaves a
  placeholder so no index moves. `uuid_v8` makes the version 8 uuid the
  draft recommends. The draft's examples A.4, A.8 and A.9 are built with it
  and equal the draft's files. `WriteError` implements `std::error::Error`.
- The signed and encrypted forms in `vcon::write`: `sign` writes a General
  JWS JSON Serialization with one signature per `Signer` and `updated_at`
  set to the signing time; `sign_again` signs a signed vCon again for a
  long-term archive, at most `read::MAX_SIGNED_LAYERS` layers; `encrypt`
  writes a General JWE JSON Serialization of a signed vCon, its content key
  from a `ContentEncrypter` and wrapped by each `KeyEncrypter`. The
  cryptography is the caller's, through those three traits; a payload or
  plaintext can be gzipped first (`Compression`). The input to sign again
  or encrypt is read under the caller's `read::Limits`, and every output
  reads back with `read::inspect`. The draft's recommended algorithms
  follow as an option in a later release.
- Each release also holds `SHA256SUMS.txt.sigstore.json`, a keyless cosign
  signature over `SHA256SUMS.txt`, and `vcon-server-<version>.intoto.jsonl`,
  the build provenance as a file, both made from the release workflow's
  OIDC identity with no stored key and verified before they are released.
  The install page shows how to check the signature with cosign; 0.2.0 and
  earlier have neither file.

### Changed

- The release binaries are built on glibc 2.34 (AlmaLinux 9), and the
  release workflow refuses a binary that needs a newer glibc symbol. They
  run on Debian 12, Ubuntu 22.04 and AlmaLinux 9, where the 0.2.0
  binaries, which needed glibc 2.38, did not start. Each binary is run
  on Debian 12, Ubuntu 22.04 and AlmaLinux 9 before a release, and the
  release build is reproducible: CI builds it twice and compares the bytes.
- A failure to open `server.sqlite` at start says which step failed, on
  which file, and the error underneath: a wrong key, an unfinished
  recovery, or the system's error. It said only `Io` before.

### Fixed

- Every file the server and its commands write into the data directory or a
  backup is made mode `0600` and every directory `0700`, whatever the umask
  of the process. They followed the umask before: `backup` under umask 0002
  wrote its files `0664` and its `tenants` directory `0775`, inside its own
  `0700` directory, and the catalogs were made `0644`.
- Storing or replacing a vCon, or creating a tenant, removed the new object
  file or tenant directory whenever the commit returned an error. When
  SQLite cannot say whether a failed COMMIT was kept, the file now stays,
  and start-up recovery removes it only if no row names it. Before, a commit
  kept after such an error left a row without its object, read back as
  damage, or a tenant without its catalog.

## [0.2.0] - 2026-10-27

### Added

- The `not-a-vcon` problem (422): the answer to a refusal from the vcon
  library that this server does not name with a problem of its own.
- `vcon-server backup <dir>`: a consistent copy of the data directory,
  taken while the server runs. Each catalog is copied with SQLite's online
  backup API, then every object file a copied catalog names; a version
  purged or a tenant removed during the copy makes it try again, at most 5
  times. The copy is a data directory that opens under the same master key,
  and the admin log records each run as `backup`.
- The release workflow publishes `vcon` and `vcon-server` on crates.io by
  trusted publishing: on a `v*` tag, after the GitHub release, each crate
  whose version crates.io does not have yet, with a token the job gets from
  its OIDC identity and that is revoked when the job ends. A run started by
  hand makes the same token exchange and dry-runs both crates, uploading
  nothing. 0.1.0 was published by hand.

### Changed

- vcon 0.2.0: the reading API is in `vcon::read` (`inspect`, `Inspection`,
  `Limits`, `Rejection`, `MAX_SIGNED_LAYERS` and `MAX_JSON_DEPTH`); `Form`
  and `parse_uuid` stay at the crate root, for what writing will share.
  `Inspection`, `Limits` and `Rejection` are `#[non_exhaustive]`, so a later
  release can add a field or a refusal without a breaking change; limits are
  made with `Limits::new`. `CRATE_NAME` is gone.

### Fixed

- The release workflow's crates.io job read one version, the workspace's,
  for both crates, so it would have skipped a vcon version that differs from
  the server's. It looks each crate up at its own version, and a run by hand
  dry-runs both crates in one `cargo publish`, so a new vcon version the
  server needs is verified before it is on crates.io.
- The commit hook's package check could build the packaged server against
  an older copy of `vcon` 0.1.0 left in the same target directory by an
  earlier run, because cargo's own verify step reuses a registry crate it
  has built at the same version. The check now packages with `--no-verify`
  in a directory made for each run, and builds each package from its
  `.crate` with the published crates patched in by path, which cargo
  checks by content. `scripts/test-check-package.sh`, run in CI, checks a
  changed `vcon` 0.1.0 in the same target directory and two trees at once.

## [0.1.0] - 2026-10-26

### Added

- Cargo workspace with the `vcon` library and the `vcon-server` binary.
- `vcon-server --version`.
- `vcon::inspect`: reads a received vCon without changing it and reports
  its form (unsigned, signed or encrypted), whether it is gzipped, and its
  uuid, or why it cannot be stored.
- Licence files (MIT OR Apache-2.0), README and this changelog.
- `LICENSES/`: the two licence texts again, named by SPDX identifier
  (`Apache-2.0.txt`, `MIT.txt`), kept identical to the root files by a test.
- Git hooks: fmt, clippy and tests on the staged tree, and refusal of personal
  data and AI-tool attribution in changes and commit messages.
- Pre-push hook: every outgoing commit must be signed by an allowed signer and
  carry the pusher's configured identity.
- `GET /vcons`: a tenant's live vCons, newest first by when each was first
  stored, a page at a time with a cursor, bounded by `since` and `until`.
  A tenant catalog made by an earlier build gains the first-store time when
  it is next opened.
- `GET /audit` and `GET /admin/audit`: a tenant's audit log, for a tenant key
  with the new `audit` scope, and the admin log, for an operator key, a page
  at a time, as the same lines `audit export` writes. Each read is recorded
  as `audit_read`.
- The console: a web page at `/console/` that signs in with an API key held
  in page memory only, served with a strict content-security policy and no
  framing; browser tests run it against the real server.
- `GET /key`: the calling key's own id, tenant and scopes, recorded as
  `key_info`.
- An OpenAPI 3.1 description of the HTTP API, served at `GET /openapi.json`
  to any valid key and published at <https://vcon-server.com/openapi.json>.
- The console's vCons and legal views: list, read (as text, gzip inflated),
  store from a file and delete, and a vCon's versions for a legal key, each
  shown only to a key whose scopes allow it.
- Both crates are ready for crates.io: each has its crates.io README, its
  page metadata and the two licence texts, and packages only its sources.
  `scripts/check_package.py`, run by the commit hook and CI, packages and
  builds each crate, then refuses any file in a `.crate` other than the
  sources, the files they include, the README and the licences; a missing
  README or licence; a git or path dependency; a relative README link; and
  anything the commit scan refuses.
- Client examples in `clients/` (curl, Python, TypeScript, Go and Rust), each
  storing, listing, reading and deleting a vCon; CI runs every one against a
  live server (`scripts/run-clients.sh`).
- The console's legal actions (hold, release, restore), audit log view
  (tenant log or admin log, a page at a time), and tenants and keys views
  for an operator key, each leaving the same audit record as the API.

### Changed

- The tests use a fork of rusqlite (`github.com/NormB/rusqlite`) as a
  dev-dependency, to check SQLCipher's settings through rusqlite's pragma
  helpers, which rusqlite 0.41.0 refuses for SQLCipher's pragma names. The
  server still builds with crates.io rusqlite, but a build of the repository
  now fetches the fork from github.com; `cargo vendor` copies it for offline
  builds.

### Security

- A signed vCon's nested JWS layers are read in a loop, not by recursion, and
  at most 64 are read; JSON is read at most 64 arrays and objects deep. A body
  past either limit is refused with the `too-deep` problem (422). Before,
  each layer took a stack frame, so a crafted body under a high
  `limits.inflated`, or any such body in a debug build, could overflow the
  stack and abort the server. `vcon::MAX_SIGNED_LAYERS`,
  `vcon::MAX_JSON_DEPTH` and `vcon::Rejection::TooDeep` name the limits.

[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/NormB/vcon-server/releases/tag/v0.4.0
[0.3.0]: https://github.com/NormB/vcon-server/releases/tag/v0.3.0
[0.2.0]: https://github.com/NormB/vcon-server/releases/tag/v0.2.0
[0.1.0]: https://github.com/NormB/vcon-server/releases/tag/v0.1.0
