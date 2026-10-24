+++
title = "vcon-server"
[extra]
tagline = "store vCons, encrypted, for as long as they must be kept"
+++

# Store vCons, encrypted, for as long as they must be kept

vcon-server stores [vCons](https://datatracker.ietf.org/wg/vcon/about/), the IETF
format for conversation records, and returns them only to readers allowed to see
them. Every vCon is encrypted at rest under its tenant's keys.

**Status:** early development. It is not ready for production use.

## What it does today

- Accepts vCons in the three forms the IETF drafts define: unsigned JSON, signed
  (JWS) and encrypted (JWE), plain or gzipped.
- Serves an HTTP API over TLS only: `POST /vcons`, `GET`, `PUT` and `DELETE
  /vcons/{uuid}`, and `GET /health`.
- Keeps tenants apart: each has its own keys, data and retention period.
- Records each request in its tenant's audit log.
- Erases a deleted vCon once its tenant's retention period has passed since it
  was stored.

More is being built; the [repository](https://github.com/NormB/vcon-server) has
the details.

## Build it

The Rust toolchain is pinned in `rust-toolchain.toml`.

```sh
cargo build --release
cargo test --workspace
```

## Licence

MIT OR Apache-2.0, at your option.
