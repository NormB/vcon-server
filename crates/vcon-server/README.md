# vcon-server

A server that stores [vCons](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/),
the IETF conversation-record format, encrypted at rest, and returns them to
authorized readers. It serves an HTTP API over TLS and keeps each tenant's
vCons under that tenant's own keys. A deleted or replaced version is kept,
hidden, until its tenant's retention period has passed since it was stored,
and then erased, unless a legal hold keeps it. Every request is recorded in
an audit log.

**Status: 0.4.3 released, not yet for production.** What works today is on
[what is built](https://vcon-server.com/docs/status/).

This crate is the `vcon-server` program. Its library exists so the tests can
drive the same code the binary runs; it is not a stable API.

## Install

From crates.io, with Rust 1.99 or later:

```sh
cargo install vcon-server --locked
vcon-server --version
```

The build compiles SQLCipher, the OpenSSL it uses, and AWS-LC, so the host
needs a C compiler with the C library headers, `make` and `perl`. On Debian or
Ubuntu:

```sh
sudo apt-get install gcc libc6-dev make perl
```

Release archives for Linux on x86_64 and aarch64, with checksums, an SBOM and
signed build provenance, are on the
[releases page](https://github.com/NormB/vcon-server/releases).

## Run it

The server needs a master key (a file of 32 random bytes), a TLS certificate
and its key, and one TOML configuration file:

```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"
```

```sh
vcon-server serve --config /etc/vcon-server/vcon-server.toml
```

It locks all its memory at start and refuses to start if it cannot. The
[getting started guide](https://vcon-server.com/docs/getting-started/) covers
each step, the systemd unit, and making the first tenant and its keys.

## For AI agents

The server serves MCP at `/mcp` on its TLS listener, and through
`vcon-server mcp` for an agent that starts its servers as commands. Each
tool does the work of one HTTP route, with the same API keys, scopes and
audit records. The [MCP page](https://vcon-server.com/docs/mcp/) lists every
tool with its parameters and an example.

## Objects in an S3 bucket

The server can keep each version's sealed object in an S3-compatible bucket
(RustFS, MinIO or AWS S3) instead of the data directory, and can pack those
objects into Parquet files in the same bucket.
[Keep vCons in an S3 bucket](https://vcon-server.com/docs/s3-storage/) sets
up the bucket, and
[keep vCons as Parquet files in a bucket](https://vcon-server.com/docs/parquet-storage/)
turns on Parquet storage. `vcon-server export parquet` writes a copy of a
tenant's vCons as Parquet files; see
[export to Parquet](https://vcon-server.com/docs/export-parquet/).

## Links

- [Documentation](https://vcon-server.com/docs/)
- [HTTP API](https://vcon-server.com/docs/http-api/)
- [Source](https://github.com/NormB/vcon-server)
- [The vcon library](https://crates.io/crates/vcon), which checks every vCon
  the server receives

## License

Licensed under either of the [Apache License, Version 2.0](https://github.com/NormB/vcon-server/blob/main/LICENSE-APACHE)
or the [MIT License](https://github.com/NormB/vcon-server/blob/main/LICENSE-MIT),
at your option.
