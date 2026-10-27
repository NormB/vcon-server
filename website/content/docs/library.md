+++
title = "The vcon library"
description = "Read and check a vCon in your own Rust code with the vcon crate."
weight = 74
+++

`vcon` is the Rust library vcon-server uses to check every vCon it receives.
Its current scope is reading and checking a vCon: which of the three forms it
is in (unsigned, signed or encrypted), whether it is gzipped, its uuid, and
whether it needs extensions the caller does not support. It does no I/O and
returns no bytes. The API may change in any release before 1.0.

This page describes vcon 0.2.0. In 0.1.0, the first release,
`inspect`, `Inspection`, `Limits`, `Rejection`,
`MAX_SIGNED_LAYERS` and `MAX_JSON_DEPTH` are at the crate root rather than in
`vcon::read`, and limits are written `Limits { inflated: 1 << 20 }`.

## Add it

```sh
cargo add vcon
```

## Read and check a vCon

`vcon::read::inspect` takes the body, its media type if one came with it, a bound on gzip
inflation, and the extension names the caller supports in `critical`. It
returns the form, whether the body was gzipped and the uuid, or a `Rejection`
that says why the vCon is refused.

```rust
use std::collections::BTreeSet;

use vcon::Form;
use vcon::read::{Limits, Rejection, inspect};

// Gzip inflation in one body may produce at most this many bytes.
let limits = Limits::new(1 << 20);
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

The whole API, with each `Rejection` and what causes it, is on
[docs.rs](https://docs.rs/vcon). The crate's page is
[crates.io/crates/vcon](https://crates.io/crates/vcon).
