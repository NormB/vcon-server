+++
title = "Storing vCons"
description = "What vcon-server accepts, what it checks, how versions, deletes and replaces work, and what it never does to a vCon."
weight = 40
+++

## What is accepted

A vCon is sent as one of three forms, as the IETF vCon draft defines them:

- **unsigned:** a JSON object with at least `uuid` and `created_at`;
- **signed:** a JWS in general JSON serialization whose payload is the vCon,
  itself possibly signed again;
- **encrypted:** a JWE in general JSON serialization.

Each may be gzipped. The request's `Content-Type` is `application/vcon`,
`application/vcon+gzip` or `application/json` (any case, any parameters), or
absent; anything else is refused with
[`unsupported-media-type`](@/problems/unsupported-media-type.md). A body is
treated as gzip when its media type says so or it starts with the gzip magic
number.

## What is checked

The checks are in the `vcon` crate, which reads a body without changing
it. In order:

1. Gzip, if any, inflates cleanly ([`bad-gzip`](@/problems/bad-gzip.md)) and
   within `limits.inflated` ([`too-large`](@/problems/too-large.md)). Every
   gzip layer, nested signed payloads included, shares that one limit.
2. The body is one JSON object ([`not-json`](@/problems/not-json.md)) in which
   no object repeats a member name ([`duplicate-member`](@/problems/duplicate-member.md))
   and no more arrays and objects are open at once than the
   [fixed limit](@/docs/configuration.md#fixed-limits) ([`too-deep`](@/problems/too-deep.md)).
   Parsers disagree on which duplicate wins, so a vCon with one could mean
   different things to different readers.
3. The form is clear: not both signed and encrypted
   ([`ambiguous-form`](@/problems/ambiguous-form.md)), and one of the three
   ([`no-form`](@/problems/no-form.md)).
4. The uuid is found: in an unsigned vCon, `uuid` in RFC 9562 text form
   ([`missing-uuid`](@/problems/missing-uuid.md), [`bad-uuid`](@/problems/bad-uuid.md))
   and `created_at` present ([`missing-created-at`](@/problems/missing-created-at.md));
   in a signed one, inside the payload ([`bad-payload`](@/problems/bad-payload.md))
   of at most the [fixed number](@/docs/configuration.md#fixed-limits) of
   signed layers ([`too-deep`](@/problems/too-deep.md)),
   and every uuid a signature header names must agree
   ([`uuid-mismatch`](@/problems/uuid-mismatch.md)); in an encrypted one,
   from its unprotected header.
5. `critical` (or `must_support`) is a list of names
   ([`bad-critical`](@/problems/bad-critical.md)), each one supported
   ([`unsupported-extension`](@/problems/unsupported-extension.md)). An
   encrypted vCon's list cannot be read, so it is not checked.

vcon-server does not verify signatures and cannot decrypt an encrypted vCon:
it holds no producer's keys. It stores them as received, so whoever does hold
the keys can verify or decrypt them later.

## Stored exactly as received

The stored body is the request body, byte for byte: never re-serialised,
re-compressed or normalised. A read returns those bytes, with
`Content-Type: application/vcon+gzip` if they were gzipped and
`application/vcon` if not.

## Versions

Every stored body is a **version**, named by a random 32-hex-digit id and
returned as a strong `ETag`, such as `"9f2c…"`. A uuid has one current
version.

- **Create** (`POST /vcons`) stores a new uuid: `201` with `Location` and
  `ETag`. The same bytes again answer `200` with the same `ETag`, so a
  producer can retry without fear. Other bytes for a uuid that exists are
  [`conflict`](@/problems/conflict.md) (409).
- **Replace** (`PUT /vcons/{uuid}`) needs `If-Match` naming the current
  version, with one strong tag; without it the answer is
  [`precondition-required`](@/problems/precondition-required.md) (428), and
  with any other tag [`precondition-failed`](@/problems/precondition-failed.md)
  (412). The body's uuid must be the path's
  ([`path-uuid-mismatch`](@/problems/path-uuid-mismatch.md)). A signed or
  encrypted current version is never replaced
  ([`immutable`](@/problems/immutable.md)): its signature would no longer hold
  for what is served. The old version is hidden and kept for legal reads until
  it is purged.
- **Delete** (`DELETE /vcons/{uuid}`) hides the uuid and all its versions at
  once: reads then answer [`gone`](@/problems/gone.md) (410), and so does a
  create with that uuid. Deleting again answers `204` again. The versions stay
  for [legal reads](@/docs/retention-and-holds.md) until they are purged.

Two requests racing on one uuid are settled in the catalog: one wins, and the
other gets the answer it would have had after it.

## On disk

Each version is a file under the tenant's `objects/` directory, named by its
version id and sealed with AES-256-GCM under its own data key. The catalog
row holds that key, wrapped by the tenant key. A write goes to a temporary
file, is synced, renamed and the directory synced, before the catalog row
commits; a crash at any point leaves either the whole version or nothing that
start-up recovery does not remove. See [security](@/docs/security.md).
