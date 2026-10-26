# vcon

A Rust library for [vCons](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/),
the IETF conversation-record format.

Its current scope is reading and checking a vCon: which of the three forms it
is in (unsigned, signed or encrypted), whether it is gzipped, its uuid, and
whether it needs extensions the caller does not support. If the vCon is
refused, it says why. It does no I/O and returns no bytes, so the caller can
keep the body exactly as received.

**Status:** in development, not yet for production. The API may change in any
release before 1.0.

[vcon-server](https://crates.io/crates/vcon-server) uses this crate to check
every vCon it receives.

## Example

```rust
use std::collections::BTreeSet;

use vcon::{Form, Limits, Rejection, inspect};

// Gzip inflation in one body may produce at most this many bytes.
let limits = Limits { inflated: 1 << 20 };
// The extensions this caller supports in `critical`: none.
let supported = BTreeSet::new();

let body = br#"{
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
  "created_at": "2026-10-23T00:00:00Z",
  "parties": []
}"#;
let found = inspect(body, Some("application/vcon+json"), &limits, &supported)
    .expect("the body is a vCon");
assert_eq!(found.form, Form::Unsigned);
assert!(!found.gzip);
assert_eq!(found.uuid.to_string(), "01a07da8-c2bb-83e5-b9a2-279e0d16bc46");

// A vCon that needs an extension the caller does not support is refused,
// with the names it lists.
let needs = br#"{
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
  "created_at": "2026-10-23T00:00:00Z",
  "critical": ["example-extension"]
}"#;
assert_eq!(
    inspect(needs, None, &limits, &supported),
    Err(Rejection::UnsupportedCritical(vec!["example-extension".to_string()]))
);

// A body that is not a vCon is refused too.
assert_eq!(inspect(b"[]", None, &limits, &supported), Err(Rejection::NotJson));
```

## What it checks

- The body is gzip when its media type is `application/vcon+gzip` or it
  starts with the gzip magic number; inflation is bounded by `Limits`, and
  every gzipped layer inside a signed vCon shares the same bound.
- The body is exactly one JSON object, no object in it repeats a member
  name, and no JSON text read has more than `MAX_JSON_DEPTH` (64) arrays and
  objects open at once.
- The form, told from the members present: `payload` and `signatures` for a
  signed vCon (a JWS), `ciphertext` and `recipients` for an encrypted one (a
  JWE), otherwise the members of an unsigned vCon.
- The uuid, in RFC 9562 text form: from the vCon itself, from the innermost
  signed payload, or from the unprotected header of an encrypted vCon. A
  `uuid` in a JWS header must match the signed vCon's. Nested signed layers
  are read in a loop, at most `MAX_SIGNED_LAYERS` (64) of them; deeper
  nesting is `Rejection::TooDeep`.
- `created_at` is present.
- `critical` (and `must_support`, its earlier name) lists only extensions the
  caller supports. An encrypted vCon's `critical` cannot be read and is not
  checked.

It does not verify signatures or decrypt.

## Links

- [Documentation](https://docs.rs/vcon)
- [Source](https://github.com/NormB/vcon-server/tree/main/crates/vcon)
- [vcon-server.com](https://vcon-server.com/)

## License

Licensed under either of the [Apache License, Version 2.0](https://github.com/NormB/vcon-server/blob/main/LICENSE-APACHE)
or the [MIT License](https://github.com/NormB/vcon-server/blob/main/LICENSE-MIT),
at your option.
