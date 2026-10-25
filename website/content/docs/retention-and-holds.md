+++
title = "Retention and legal holds"
description = "How long a deleted vCon is kept, when it is erased, how a legal hold stops that, and what the legal routes can read and restore."
weight = 50
+++

## The rule

A version is **purged**, erased for good, when all of these hold:

- it is hidden: its uuid was deleted, or a newer version replaced it;
- its uuid is not under a legal hold;
- its tenant's retention period has passed since the version was **stored**.

The period counts from when the version was stored, not from when it was
deleted. So deleting an old vCon erases it at once (unless it is held), and
deleting a new one keeps it, hidden, until its period runs out. A live
version is never purged, however old.

A purge removes the version's catalog row, with its wrapped data key, and its
object file. The catalog is a SQLCipher database with `secure_delete` on, so
the row's bytes are overwritten. When a deleted uuid's last version goes, a
tombstone keeps the uuid, so it stays [`gone`](@/problems/gone.md) and cannot
be stored again.

## The period

Each tenant has its own period, or the server's `retention_default` (`7y`
unless configured). A period is calendar years, months or days, such as
`7y`, `18m` or `90d`, added to the stored time in UTC. Set a tenant's with
`PATCH /admin/tenants/{id}` (`{"retention": "10y"}`, or `null` for the server
default). A change applies to every version the tenant keeps, at the next
sweep.

## When purges run

- **The sweep:** at start, then `purge_interval` after each sweep ends
  (an hour unless configured), every tenant is swept. A tenant that cannot be
  swept is counted in the sweep's log line and tried again next time.
- **On delete:** `DELETE /vcons/{uuid}` purges that uuid's versions that are
  already due, before it answers. With a `0d` period a delete erases the vCon
  inside the request.

## Legal holds

A legal hold is set on a uuid, live or deleted, by a key with the `legal`
scope:

```sh
curl -X PUT    https://vcon.example.com/legal/vcons/$UUID/hold -H "Authorization: Bearer $LEGAL_KEY"   # 204
curl -X DELETE https://vcon.example.com/legal/vcons/$UUID/hold -H "Authorization: Bearer $LEGAL_KEY"   # 204
```

While it is held, none of its versions is purged, by the sweep or by a delete.
A hold does not stop a delete: the vCon is hidden from ordinary reads, and kept.
A tenant with a held uuid cannot be removed over HTTP, even with `force`.
A uuid already purged answers [`gone`](@/problems/gone.md) (410).

## Legal reads

With the `legal` scope:

| Route | Answers |
|---|---|
| `GET /legal/vcons/{uuid}` | the current version, or for a deleted uuid the latest kept one, with `Vcon-State: live` or `deleted` |
| `GET /legal/vcons/{uuid}/versions` | every kept version, oldest first: id, `stored_at`, `hidden_at` (RFC 3339, UTC) and whether it is current |
| `GET /legal/vcons/{uuid}/versions/{version}` | that kept version, as stored |
| `POST /legal/vcons/{uuid}/restore` | makes a deleted uuid live again, with its latest kept version; `200` with that version's `ETag` |

Restoring a uuid that is not deleted is [`not-deleted`](@/problems/not-deleted.md)
(409). Every legal request is recorded in the tenant's audit log, like every
other request.

## What erasure means on disk

A purge erases the version from the live system. Copies made before it
(old disks, freed blocks, backups) still hold the sealed file and the wrapped
key, readable with the tenant key and master key of that time.
[Key rotation](@/docs/operations.md#key-rotation) closes that: after a rotation, the keys that
could open those copies are destroyed.
