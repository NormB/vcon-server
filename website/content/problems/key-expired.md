+++
title = "key-expired"
template = "problem.html"
[extra]
status = 401
problem_title = "The key has expired"
+++

## When

The request's `Authorization: Bearer` header carries a key whose id and secret are right, and the time is at or past the key's `expires_at`. A key made without an expiry in its request expires 90 days after it was made; a request can ask for 1 to 365 days, or for no expiry where the configuration allows one. A wrong secret for an expired key is answered [`unauthorized`](@/problems/unauthorized.md), and so is a key that is both revoked and expired: only the holder of a key that was not revoked learns that it expired. Each refusal is counted in the audit log as `auth_failed` with status 401, under the key's id.

## What to do

Make a new key and give it to the client that held the old one: `vcon-server key create` on the host, `POST /admin/tenants/{id}/keys` with an operator key, or `POST /tenant/keys` with a key of the tenant's administrator. Then revoke the expired key, so the list of keys shows it as ended. An expired key cannot make its replacement. `key list` and both key lists show each key's `expires_at`; see [rotate keys](@/docs/rotate-keys.md) and [administer a tenant with a key](@/docs/tenant-administration.md#set-a-key-s-expiry-and-see-when-it-was-last-used).
