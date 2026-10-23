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

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
