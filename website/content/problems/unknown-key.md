+++
title = "unknown-key"
template = "problem.html"
[extra]
status = 404
problem_title = "No key has the id"
+++

## When

`DELETE /admin/keys/{id}` names a key id that does not exist, or is not 16 hexadecimal digits.

## What to do

Use an id from `GET /admin/tenants/{id}/keys`.
