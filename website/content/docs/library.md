+++
title = "The vcon library"
description = "Read, check and write vCons in your own Rust code with the vcon crate."
weight = 74
+++

`vcon` is the Rust library vcon-server uses to check every vCon it receives.
It reads and checks a vCon: which of the three forms it is in (unsigned,
signed or encrypted), whether it is gzipped, its uuid, and whether it needs
extensions the caller does not support. Reading does no I/O and returns no
bytes. The API may change in any release before 1.0.

This page describes vcon 0.3.0. In 0.1.0, the first release,
`inspect`, `Inspection`, `Limits`, `Rejection`,
`MAX_SIGNED_LAYERS` and `MAX_JSON_DEPTH` are at the crate root rather than in
`vcon::read`, and limits are written `Limits { inflated: 1 << 20 }`.

New in 0.3.0, `vcon::write` writes vCons: an object model of the vCon core
draft, written unsigned, gzipped, signed (a JWS) or encrypted (a JWE). The cryptography is the caller's, through the `Signer`,
`ContentEncrypter` and `KeyEncrypter` traits. What the draft does not allow
is refused, with the JSON path at fault, and `read::inspect` reads every
output back. The examples are in the
[`vcon::write` documentation](https://docs.rs/vcon/latest/vcon/write/).

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

## Write a vCon

`vcon::write` builds a vCon from the core draft's object model: the
constructor takes what the draft requires, and the rest is checked when the
vCon is written. `to_json` writes it unsigned and `to_gzip` gzipped;
`vcon::read::inspect` reads back either, with the same uuid. The caller
passes every uuid and time, so the output is the same on every run. This
example is the one in the module's documentation, which runs as a doc test:

```rust
use vcon::write::{Channel, Content, ContentHash, Date, Dialog, Parties, Party, Vcon};

let uuid = vcon::write::uuid_v8(1_655_833_980_000, 0x3e5, "vcon.example.com");
let created: Date = "2022-06-21T13:53:00-04:00".parse().expect("an RFC 3339 date");
let mut v = Vcon::new(uuid, created);
let alice = v.add_party(Party::new().name("Alice"));
let bob = v.add_party(Party::new().name("Bob"));
let hash = ContentHash::try_new("sha512", &[1, 2, 3]).expect("a hash");
v.add_dialog(
    Dialog::recording(Content::External {
        url: "https://example.com/call.mp3".to_owned(),
        hashes: vec![hash],
    })
    .parties(Parties::Channels(vec![Channel::One(alice), Channel::One(bob)])),
);
let json = v.to_json().expect("the vCon is valid");
assert!(
    json.starts_with(br#"{"uuid":""#),
    "the uuid is the first member"
);

// The same vCon gzipped, read back as vcon-server reads it.
let gzipped = v.to_gzip().expect("the vCon is valid");
let limits = vcon::read::Limits::new(1 << 20);
let found = vcon::read::inspect(&gzipped, None, &limits, &Default::default())
    .expect("what vcon::write writes, vcon::read reads");
assert!(found.gzip, "the body is gzipped");
assert_eq!(found.uuid, uuid, "with the uuid it was written with");
```

Signing (a JWS) and encrypting (a JWE) take a `Signer`, or a
`ContentEncrypter` and a `KeyEncrypter`, that the caller provides; their
examples are in the
[`vcon::write` documentation](https://docs.rs/vcon/latest/vcon/write/).

The whole API, with each `Rejection` and what causes it, is on
[docs.rs](https://docs.rs/vcon). The crate's page is
[crates.io/crates/vcon](https://crates.io/crates/vcon).
