+++
title = "master-key-limit"
template = "problem.html"
[extra]
status = 503
problem_title = "The master key must be rotated"
+++

## When

The master key has wrapped 2^32 tenant keys, so no tenant can be made.

## What to do

The operator rotates the master key with `vcon-server rekey master --new-master <file>`, with the server stopped. See [key rotation](@/docs/operations.md#key-rotation).
