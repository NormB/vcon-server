+++
title = "bad-expiry"
template = "problem.html"
[extra]
status = 422
problem_title = "A key cannot have this expiry"
+++

## When

A request to make a key names `expires_in_days` outside 1 to 365, names `expires_in_days` together with `"no_expiry": true`, or asks for `"no_expiry": true` on a server whose configuration sets `keys.no_expiry` to `false`. The scopes of the request are checked first: a request with bad scopes and a bad expiry is answered [`bad-scopes`](@/problems/bad-scopes.md). No key is made, and the refusal is recorded in the audit log as `key_create` with status 422.

## What to do

Leave both fields out for a key that expires in 90 days, or name `expires_in_days` from 1 to 365. For a key that never expires, send `"no_expiry": true` alone; when the server refuses it, the operator has turned such keys off in the [configuration](@/docs/configuration.md), and the longest life is 365 days.
