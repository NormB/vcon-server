+++
title = "invalid-body"
template = "problem.html"
[extra]
status = 422
problem_title = "The body is not one this route takes"
+++

## When

An admin route's body is not JSON, misses a required member, has a member of the wrong type or one the route does not take, or names a tenant badly (1 to 128 characters, no control characters, no space at either end).

## What to do

See the route's body on the [HTTP API](@/docs/http-api.md#admin) page.
