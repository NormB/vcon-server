# Changelog

All notable changes to this project are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Cargo workspace with the `vcon-form` library and the `vcon-server` binary.
- `vcon-server --version`.
- `vcon_form::inspect`: reads a received vCon without changing it and reports
  its form (unsigned, signed or encrypted), whether it is gzipped, and its
  uuid, or why it cannot be stored.
- Licence files (MIT OR Apache-2.0), README and this changelog.
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
