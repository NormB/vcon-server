# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/NormB/vcon-server/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/NormB/vcon-server/releases/tag/v0.2.0
[0.1.0]: https://github.com/NormB/vcon-server/releases/tag/v0.1.0
