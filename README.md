# vcon-server

A server that stores [vCons](https://datatracker.ietf.org/wg/vcon/about/), the
IETF conversation-record format, encrypted at rest, and returns them to
authorised readers.

**Status:** early development. It is not ready for production use.

## Building

The Rust toolchain is pinned in `rust-toolchain.toml`.

```sh
cargo build --release
cargo test --workspace
```

## Contributing

Enable the repository's git hooks once per clone:

```sh
git config core.hooksPath .githooks
```

Before each commit they run `cargo fmt --check`, `cargo clippy -D warnings`
and `cargo test` on exactly what is staged. They refuse personal data and
AI-tool attribution in staged changes and commit messages. Before each push,
every outgoing commit must carry a good signature from an allowed signer and
the pusher's own configured name and email. Test data uses
reserved values only: `example.com` addresses, 555-01xx numbers, and RFC 5737
and RFC 3849 IP addresses. The one exception is the IETF draft's own examples
in `crates/vcon-form/tests/fixtures/ietf/`, kept unchanged. `scripts/test-hooks.sh` and `scripts/test-pre-push.sh` test the hooks
themselves.

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
