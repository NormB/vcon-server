+++
title = "unauthorized"
template = "problem.html"
[extra]
status = 401
problem_title = "A valid bearer key is needed"
+++

## When

The request had no `Authorization: Bearer` header, or its token is malformed, names no key, or names a revoked key, or a key whose tenant has been removed.

## What to do

Send a valid, current key. Failed attempts are counted in the audit log.
