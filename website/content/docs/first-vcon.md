+++
title = "Store your first vCon"
description = "A first vCon end to end: store it, store it again, see a conflict refused, read it back byte for byte, list it, and delete it, each with a key that holds only the scope it needs, and every answer shown."
weight = 12
+++

This how-to stores one vCon and follows it to its deletion. Each step shows
the request and the server's whole answer, headers included. It uses three
tenant keys, one per scope, so each refusal shows what a scope allows.

CI runs every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A running server, set up as in [getting started](@/docs/getting-started.md),
  with the tenant `acme`.
- `fullchain.pem`, the certificate made there, in the current directory.
  With a certificate from a public authority, leave out `--cacert fullchain.pem`.
- One shell for every step: later steps use variables that earlier steps set.

## Make a key for each scope

The tenant's id names it in `key create`:

```sh
TENANT=$(sudo -u vcon vcon-server tenant list --config /etc/vcon-server/vcon-server.toml | jq -r 'select(.name == "acme") | .id')
```

Make three keys for the tenant: one that may only store, one that may only
read, and one that may only delete. `key create` runs beside the server:

```sh
WRITER=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope write --label first-vcon-writer \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
READER=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label first-vcon-reader \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
DELETER=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope delete --label first-vcon-deleter \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
```

Set the server's address once:

```sh
URL=https://vcon.example.com:8443
```

## Write a vCon

A vCon is a JSON object. This one has two parties and no dialog yet. Its
`uuid` is new each time you make the file, so you can run the steps again:

```sh
UUID=$(cat /proc/sys/kernel/random/uuid)
cat > call.vcon <<EOF
{"vcon":"0.4.0","uuid":"$UUID","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550101","name":"Caller Example"},{"mailto":"agent@example.com","name":"Agent Example"}],"dialog":[],"attachments":[],"analysis":[]}
EOF
jq -r .uuid call.vcon
```

```text
<uuid>
```

[Storing vCons](@/docs/storing-vcons.md) lists what the server checks in a
body before it stores it.

## Store it

Send it with the write key. `-i` prints the answer's status line and
headers:

```sh
curl -sS -i --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $WRITER" \
  -H "Content-Type: application/vcon" \
  --data-binary @call.vcon
```

```text
HTTP/2 201
location: /vcons/<uuid>
etag: "<version>"
content-length: 0
date: <date>
```

`201 Created`: the server stored the vCon. `location` is its path, and
`etag` names the version stored. A version is one stored body. Replacing
the vCon later stores a new version.

## Store the same bytes again

A producer that did not see the answer sends the vCon again. The same bytes
answer `200`, with the same `etag`, and nothing new is stored:

```sh
curl -sS -i --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $WRITER" \
  -H "Content-Type: application/vcon" \
  --data-binary @call.vcon
```

```text
HTTP/2 200
etag: "<version>"
content-length: 0
date: <date>
```

So a producer can retry any `POST /vcons` without making a duplicate.

## Store other bytes under the same uuid

A different body with a uuid already stored is refused with `409`, as the
[`conflict`](@/problems/conflict.md) problem:

```sh
sed 's/Caller Example/Caller Changed/' call.vcon > changed.vcon
curl -sS -i --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $WRITER" \
  -H "Content-Type: application/vcon" \
  --data-binary @changed.vcon
```

```text
HTTP/2 409
content-type: application/problem+json
content-length: 103
date: <date>

{"status":409,"title":"Another vCon holds the uuid","type":"https://vcon-server.com/problems/conflict"}
```

Every refusal is an RFC 9457 problem document, and its `type` is a page on
this site. To change a stored vCon, replace it with `PUT /vcons/{uuid}` and
`If-Match` (see the [HTTP API](@/docs/http-api.md#vcons)).

## Read it back

Reading needs the `read` scope. The write key is refused with `403`, as
[`forbidden`](@/problems/forbidden.md):

```sh
curl -sS -i --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $WRITER"
```

```text
HTTP/2 403
content-type: application/problem+json
content-length: 104
date: <date>

{"status":403,"title":"The key does not allow this","type":"https://vcon-server.com/problems/forbidden"}
```

The read key gets the body as it was stored:

```sh
curl -sS -i --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $READER"
```

```text
HTTP/2 200
content-type: application/vcon
etag: "<version>"
content-length: 255
date: <date>

{"vcon":"0.4.0","uuid":"<uuid>","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550101","name":"Caller Example"},{"mailto":"agent@example.com","name":"Agent Example"}],"dialog":[],"attachments":[],"analysis":[]}
```

The server stores a body byte for byte and never rewrites it, so a signed
vCon keeps its signature. Compare the bytes:

```sh
curl -sS --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $READER" -o read.vcon
cmp read.vcon call.vcon && echo "the same bytes"
```

```text
the same bytes
```

## List the tenant's vCons

`GET /vcons` lists the key's tenant's vCons, newest first, a page at a time:

```sh
curl -sS --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $READER" | jq .
```

```text
{
  "next": null,
  "vcons": [
    {
      "stored_at": "<time>",
      "updated_at": "<time>",
      "uuid": "<uuid>",
      "version": "<version>"
    }
  ]
}
```

`next` is `null` on the last page. [Listing](@/docs/http-api.md#listing)
describes the paging and the time bounds.

## Delete it

Deleting needs the `delete` scope. Neither the write key nor the read key
holds it:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/vcons/$UUID" -H "Authorization: Bearer $WRITER"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/vcons/$UUID" -H "Authorization: Bearer $READER"
```

```text
403
403
```

The delete key deletes it. The answer is `204`, with no body:

```sh
curl -sS -i --cacert fullchain.pem -X DELETE "$URL/vcons/$UUID" -H "Authorization: Bearer $DELETER"
```

```text
HTTP/2 204
date: <date>
```

## After the delete

The vCon is hidden at once. A read answers `410`, as
[`gone`](@/problems/gone.md):

```sh
curl -sS -i --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $READER"
```

```text
HTTP/2 410
content-type: application/problem+json
content-length: 102
date: <date>

{"status":410,"title":"The vCon was deleted or purged","type":"https://vcon-server.com/problems/gone"}
```

The list leaves it out:

```sh
curl -sS --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $READER" | jq -c .
```

```text
{"next":null,"vcons":[]}
```

A deleted uuid stays deleted: storing it again answers `410` too, even with
the same bytes:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $WRITER" -H "Content-Type: application/vcon" --data-binary @call.vcon
```

```text
410
```

The server keeps the deleted version, hidden, until the tenant's retention
period has passed since it was stored, then purges it. A key with the
`legal` scope can still read it, and restore it, until then. See
[retention and legal holds](@/docs/retention-and-holds.md).

## A request without a key

Every route needs a key. Without one, or with a token the server does not
know, the answer is `401`, as [`unauthorized`](@/problems/unauthorized.md):

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer vcs_0000000000000000_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
```

```text
401
401
```

## Clean up

The three keys were for this how-to. Revoke them, with the commands that run
beside the server:

```sh
for label in first-vcon-writer first-vcon-reader first-vcon-deleter; do
  id=$(sudo -u vcon vcon-server key list --tenant "$TENANT" --config /etc/vcon-server/vcon-server.toml \
    | jq -r --arg l "$label" 'select(.label == $l) | .id')
  sudo -u vcon vcon-server key revoke "$id" --config /etc/vcon-server/vcon-server.toml
done
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $READER"
```

```text
401
```

A revoked key is refused at its next request.

## Next

- [Tenants and keys](@/docs/tenants-and-keys.md): every scope, and what an
  operator key may do.
- [Producers](@/docs/producers.md): what a producer does with each answer.
- [The HTTP API](@/docs/http-api.md): every route.
