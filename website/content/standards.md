+++
title = "Standards"
description = "The specifications vcon-server follows, and how far."
template = "page.html"
+++

| Specification | Where it applies |
|---|---|
| [draft-ietf-vcon-vcon-core-04](https://datatracker.ietf.org/doc/draft-ietf-vcon-vcon-core/04/) | The vCon format: the three forms, `uuid`, `created_at`, `critical`. The draft's own examples are the `vcon` crate's test fixtures. `must_support`, `critical`'s name before syntax 0.4.0, is read too. |
| [RFC 7515](https://www.rfc-editor.org/rfc/rfc7515) JWS | Signed vCons, in general JSON serialization: the payload is read for the vCon's uuid; signatures are not verified. |
| [RFC 7516](https://www.rfc-editor.org/rfc/rfc7516) JWE | Encrypted vCons, in general JSON serialization: stored as received; the uuid is read from the unprotected header. |
| [RFC 1952](https://www.rfc-editor.org/rfc/rfc1952) gzip | Gzipped bodies, inflated within `limits.inflated` for checking only; stored as received. |
| [RFC 8259](https://www.rfc-editor.org/rfc/rfc8259) JSON | Bodies; an object that repeats a member name is refused, since section 4 leaves its meaning to each parser. |
| [RFC 9562](https://www.rfc-editor.org/rfc/rfc9562) UUIDs | A vCon's `uuid`, in text form, of any version. |
| [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110) HTTP semantics | Strong `ETag`s for versions; `If-Match` on replace (412 when it names another version). |
| [RFC 6585](https://www.rfc-editor.org/rfc/rfc6585) | 428 Precondition Required for a replace without `If-Match`. |
| [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457) problem details | Every refusal; each `type` is a page under [/problems/](@/problems/_index.md). |
| [RFC 6750](https://www.rfc-editor.org/rfc/rfc6750) bearer tokens | `Authorization: Bearer` on every request. |
| [RFC 3339](https://www.rfc-editor.org/rfc/rfc3339) | Every time the server writes, in UTC. |
| [RFC 8446](https://www.rfc-editor.org/rfc/rfc8446) TLS 1.3, [RFC 5246](https://www.rfc-editor.org/rfc/rfc5246) TLS 1.2 | The only transport. |
| [NIST SP 800-38D](https://csrc.nist.gov/pubs/sp/800/38/d/final) AES-GCM | Sealing bodies and wrapping keys, AES-256 with random 96-bit nonces; a key is retired before 2^32 uses (section 8.3). |
| [RFC 5869](https://www.rfc-editor.org/rfc/rfc5869) HKDF | Catalog keys, HKDF-SHA256 of the parent key. |
