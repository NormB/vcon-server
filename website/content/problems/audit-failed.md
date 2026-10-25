+++
title = "audit-failed"
template = "problem.html"
[extra]
status = 503
problem_title = "The audit record could not be written"
+++

## When

The request's audit record could not be written, so the request was not carried out. One case differs: a change recorded in the admin log whose copy in the tenant's log failed; the change stands.

## What to do

Retry with backoff, and tell the operator: the catalogs may be out of space or damaged.
