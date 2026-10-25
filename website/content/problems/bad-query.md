+++
title = "bad-query"
template = "problem.html"
[extra]
status = 400
problem_title = "The query is not one this route takes"
+++

## When

`DELETE /admin/tenants/{id}` with a query other than `force=true` or `force=false`.

## What to do

Send no query, or one of those two.
