+++
title = "forbidden"
template = "problem.html"
[extra]
status = 403
problem_title = "The key does not allow this"
+++

## When

The key is valid but lacks the route's scope: `write`, `read`, `delete` or `legal` for tenant routes, `admin` for the admin routes. A tenant key never reaches the admin routes, and an operator key never reaches a tenant's vCons. Asking over HTTP for what only the command line may do, such as making an operator key, is refused the same way.

## What to do

Use a key with the scope the route needs. See [tenants and keys](@/docs/tenants-and-keys.md).
