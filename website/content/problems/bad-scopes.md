+++
title = "bad-scopes"
template = "problem.html"
[extra]
status = 422
problem_title = "A tenant key cannot hold these scopes"
+++

## When

A key request names an unknown scope, a scope twice, no scope, or `admin`.

At `POST /tenant/keys`, where a tenant's administrator makes a key, it is
also the answer for `tenant_admin` and for a scope outside the tenant's
role cap.

## What to do

Name one or more of `write`, `read`, `delete`, `legal`, `audit` and
`tenant_admin`, each once.

At `POST /tenant/keys`, name one or more of `write`, `read`, `delete`,
`legal` and `audit` that the tenant's role cap allows. Only the operator
makes a key that holds `tenant_admin`, and only the operator changes the
cap: see [roles, scopes and a tenant's role cap](@/docs/roles-and-scopes.md).
