+++
title = "Producers"
description = "Sending vCons to vcon-server from a recorder, a conserver or any other producer: keys, retries, and what each answer means."
weight = 75
+++

A producer is anything that makes vCons: a call recorder, a contact-centre
platform, a conserver pipeline, a capture tool. vcon-server takes them over
one route, `POST /vcons`, with a bearer key.

## Give each producer its own key

Make a tenant key with only the `write` scope, labelled with the producer's
name:

```sh
curl https://vcon.example.com/admin/tenants/$TENANT/keys \
  -H "Authorization: Bearer $OPERATOR_KEY" -H "Content-Type: application/json" \
  -d '{"scopes": ["write"], "label": "recorder-east-1"}'
# 201 {"id":"…","token":"vcs_…"}
```

The token is shown once. A producer that also reads back what it sent needs
`read` too; one that replaces vCons as analysis arrives needs only `write`.

## Send

```sh
curl https://vcon.example.com/vcons \
  -H "Authorization: Bearer $VCON_KEY" \
  -H "Content-Type: application/vcon" \
  --data-binary @call.vcon
```

Send gzipped bodies as `application/vcon+gzip`. Send signed or encrypted vCons
as they are: they are stored byte for byte, so signatures stay valid.

## What each answer means

| Answer | Meaning | What to do |
|---|---|---|
| `201` | stored; `ETag` is its version | done |
| `200` | the same bytes were already stored | done: this is a safe retry |
| `409` [`conflict`](@/problems/conflict.md) | another vCon already has this uuid | a bug upstream: two vCons share a uuid, or this one changed and needs `PUT` |
| `410` [`gone`](@/problems/gone.md) | this uuid was deleted | do not resend; a deleted uuid stays deleted |
| `413` [`too-large`](@/problems/too-large.md) | over `limits.body`, or inflates past `limits.inflated` | send the recording by URL instead of inline |
| `422` (format problems) | the body is not a vCon the server can store | fix the producer; resending will not help |
| `401` / `403` | the key is wrong, revoked, or lacks `write` | fix the key |
| `408` [`timeout`](@/problems/timeout.md), `503`, connection errors | the request may not have been carried out | retry with backoff: a retry of the same bytes is safe |

Because the same bytes answer `200`, a producer can retry any request whose
answer it did not see, without creating a duplicate.

## Keep recordings by URL

vCons let a recording be held inline (base64url in the dialog body) or by
URL with a content hash. Of 25,471 recordings in the vCons measured to set
the defaults, all but 26 were by URL. Inline audio costs about 43 kB a second
of an 8 kHz 16-bit stereo WAV, so the 16 MiB default body holds about six
and a half minutes. Long calls belong in object storage, referenced by URL.

## Updating a vCon

When analysis arrives after the call, read the current `ETag` and replace with
`If-Match`. A `412` means someone else replaced it first: read again, merge,
and retry. Signed and encrypted vCons are never replaced: send a new vCon
instead.
