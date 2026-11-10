+++
title = "bad-query"
template = "problem.html"
[extra]
status = 400
problem_title = "The query is not one this route takes"
+++

## When

A query the route does not take:

- `DELETE /admin/tenants/{id}` with a query other than `force=true` or
  `force=false`;
- `GET /vcons`, `GET /audit` or `GET /admin/audit` with a parameter the
  route does not name, a parameter given twice, or a value it cannot read;
- a read of a vCon (`GET /vcons/{uuid}`, `GET /legal/vcons/{uuid}`,
  `GET /legal/vcons/{uuid}/versions/{version}`) whose `reveal` parameter is
  not the only parameter, is given twice, or names no part.

## What to do

Send no query, or one the route's row on the
[HTTP API](@/docs/http-api.md) page names. For `reveal`, send it alone,
once, with one of `parties`, `subject`, `dialog`, `analysis`, `links` or
`all`.
