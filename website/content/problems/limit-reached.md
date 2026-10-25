+++
title = "limit-reached"
template = "problem.html"
[extra]
status = 503
problem_title = "The tenant key must be rotated"
+++

## When

The tenant key has wrapped 2^32 data keys, the most AES-GCM with random nonces allows under one key, so new writes are refused.

## What to do

The operator rotates the tenant key with `vcon-server rekey tenant <id>`, with the server stopped. See the [command line](@/docs/command-line.md#key-rotation).
