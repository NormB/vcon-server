# vcon-server

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/lockup-dark.svg">
  <img src="assets/brand/lockup-light.svg" alt="vcon-server" height="48">
</picture>

[![CI](https://github.com/NormB/vcon-server/actions/workflows/ci.yml/badge.svg)](https://github.com/NormB/vcon-server/actions/workflows/ci.yml)
[![OpenSSF Scorecard](https://api.securityscorecards.dev/projects/github.com/NormB/vcon-server/badge)](https://scorecard.dev/viewer/?uri=github.com/NormB/vcon-server)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/15941/badge)](https://www.bestpractices.dev/projects/15941)
[![codecov](https://codecov.io/gh/NormB/vcon-server/graph/badge.svg)](https://codecov.io/gh/NormB/vcon-server)
[![XERJ Corpus Hub contributor](https://hub.xerj.org/contributors/NormB/badge.svg)](https://hub.xerj.org/contributors/NormB)

A server that stores [vCons](https://datatracker.ietf.org/wg/vcon/about/), the
IETF conversation-record format, encrypted at rest, and returns them only to
the keys allowed to see them.

**Status: 0.4.1 released, not yet for production.** The `vcon` and
`vcon-server` crates are on crates.io, and the server for Linux on x86_64 and
aarch64 is on [GitHub](https://github.com/NormB/vcon-server/releases).
[What is built](https://vcon-server.com/docs/status/) says what works today.
The documentation is at [vcon-server.com](https://vcon-server.com/docs/).

## What is built

The same list as [what is built](https://vcon-server.com/docs/status/),
where each item is described in full:

- **Input checks:** the three vCon forms, plain or gzipped, checked by the
  `vcon` library crate.
- **Writing vCons:** `vcon::write` builds a vCon and writes it unsigned,
  gzipped, signed or encrypted.
- **Encrypted storage:** AES-256-GCM per version, tenant keys wrapped by the
  master key, SQLCipher catalogs, crash-safe writes and start-up recovery.
- **Objects in an S3-compatible bucket:** RustFS, MinIO or AWS S3 keeps the
  sealed objects; see
  [keep vCons in an S3 bucket](https://vcon-server.com/docs/s3-storage/).
- **Parquet storage in the bucket:** the objects packed into Parquet files,
  each vCon read back by one ranged read, and a purge that deletes the old
  file before it is done, with manifests for outside readers and small
  files merged; see
  [keep vCons as Parquet files in a bucket](https://vcon-server.com/docs/parquet-storage/).
- **Parquet export:** a copy of a tenant's vCons as Parquet files, in a
  directory or a bucket, written beside a running server.
- **The HTTP API over TLS:** create, read, replace and delete, the legal
  routes, the admin routes and health.
- **The client API:** paged lists, the audit logs over HTTPS, the OpenAPI
  description, and client examples in five languages.
- **The console:** a web page at `/console/`.
- **The MCP server:** tools for AI agents, at `/mcp` and through
  `vcon-server mcp`.
- **Tenants and keys:** scoped bearer keys, revocation and tenant removal.
- **Retention:** periods per tenant or server-wide, and the purge sweep.
- **Audit logs:** one per tenant and one for administration.
- **Hardening:** memory locked, core dumps off, one server per data
  directory.
- **Configuration:** one TOML file, unknown keys refused.
- **The command line:** tenants, keys, audit export, backup and `check`.
- **Key rotation:** `rekey tenant` and `rekey master`.
- **`check`:** every stored version opened and every problem reported.
- **Start-up refusals:** for keys, rotations and catalogs that do not hold
  together.
- **The operator guide:** how-to guides that CI runs on each change.
- **Fuzzing:** every parser of outside input.
- **Releases:** signed Linux binaries, an SBOM and build provenance.

## Install

A release holds the server as a `.tar.gz` archive for each CPU,
`SHA256SUMS.txt` with the SHA-256 of each file, and
`SHA256SUMS.txt.sigstore.json`, a keyless Sigstore signature over
`SHA256SUMS.txt`. Check the archive with `sha256sum` and the checksums with
`cosign verify-blob`. The
[install page](https://vcon-server.com/docs/install/#download-a-release) gives
each command and what it prints, and the other ways to install: cargo, a build
from source and a container.

## For AI agents

vcon-server serves MCP at `/mcp` on its TLS listener, and through
`vcon-server mcp` for an agent that starts its servers as commands. Each tool
does the work of one HTTP route, with the same API keys, scopes and audit
records. The [MCP page](https://vcon-server.com/docs/mcp/) lists every tool
with its parameters and an example.

## Building

The Rust toolchain is pinned in `rust-toolchain.toml`.

```sh
cargo build --release
scripts/nextest.sh install
scripts/nextest.sh run --workspace
cargo test --workspace --doc
```

The tests run under the pinned [cargo-nextest](https://nexte.st/), which ends
a test still running after 3 minutes and fails it by name; it does not run
doc tests, so `cargo test --doc` runs those.

[S3 object storage](https://vcon-server.com/docs/s3-storage/) and
[Parquet storage](https://vcon-server.com/docs/parquet-storage/), which packs
those objects into Parquet files in the same bucket, are in the release
binaries and in `cargo install`. Both need the `s3` cargo feature, which is
on by default; `cargo build --release -p vcon-server --no-default-features`
leaves it out.

The `postgres` cargo feature, which is not on by default and not in the
release binaries, adds a preview of the
[PostgreSQL catalog](https://vcon-server.com/docs/postgres-catalog/): the
server connects to PostgreSQL over TLS, checks it, and stops, since the
catalogs are not kept there yet.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Every contributor signs the
[Contributor License Agreement](CLA.md) once, before a first pull request
merges.

Enable the repository's git hooks once per clone:

```sh
git config core.hooksPath .githooks
```

Before each commit, `.githooks/pre-commit` refuses personal data and AI-tool
attribution in the staged changes (`scripts/scan_forbidden.py`) and checks
that git ignores every export of the private design documents and every
private spec or plan file (a name ending in .local.md), and that none is staged
(`scripts/check-ignored-exports.sh`). Then, on a copy of exactly what is
staged, it checks that every GitHub Action is pinned by commit
(`scripts/check_action_pins.py`) and runs `scripts/rustfmt.sh --check`,
`scripts/check_style.py --walk`,
`cargo clippy --workspace --all-targets --locked --quiet -- -D warnings`, the
tests with `scripts/nextest.sh run --workspace --locked` and the doc tests with
`cargo test --workspace --locked --quiet --doc` (on aarch64, linked by
`scripts/cc-lld.sh` with the toolchain's lld), the same for the fuzz package, the
tests of the scripts, and `scripts/check_package.py`, which checks what each
crate would publish. `.githooks/commit-msg` runs the same scan over the commit
message. Before each push, `.githooks/pre-push` refuses any outgoing commit
that has no good signature from an allowed signer, whose author or committer
is not the pusher's configured name and email, or whose message
`scripts/scan-commit-messages.sh` refuses. The
[development page](https://vcon-server.com/docs/development/#gates) describes
each gate.

Test data uses reserved values only: `example.com` addresses, 555-01xx
numbers, and RFC 5737 and RFC 3849 IP addresses. The one exception is the IETF
draft's own examples in `crates/vcon/tests/fixtures/ietf/`, kept unchanged.
`scripts/test-hooks.sh` and `scripts/test-pre-push.sh` test the hooks
themselves.

## Support the project

[![Patreon](https://img.shields.io/badge/Patreon-support-f96854?logo=patreon&logoColor=white)](https://www.patreon.com/c/NormB975)
[![Sponsor](https://img.shields.io/badge/Sponsor-%E2%9D%A4-db61a2?logo=githubsponsors&logoColor=white)](https://github.com/sponsors/NormB)

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option. The same two texts are in [`LICENSES/`](LICENSES), named by
their SPDX identifiers.

Copyright 2026 [Norm Brandinger](https://www.linkedin.com/in/nbrandinger)
