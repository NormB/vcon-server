+++
title = "unknown-tenant"
template = "problem.html"
[extra]
status = 404
problem_title = "No tenant has the id"
+++

## When

An admin route names a tenant id that does not exist, or is not 32 hexadecimal digits.

## What to do

Use an id from `GET /admin/tenants`.
