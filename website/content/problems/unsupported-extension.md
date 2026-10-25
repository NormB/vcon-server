+++
title = "unsupported-extension"
template = "problem.html"
[extra]
status = 422
problem_title = "An extension the vCon needs is not supported"
+++

## When

The vCon lists in `critical` (or `must_support`) an extension that is not in this server's `supported_critical`. The answer does not name it, since no problem repeats the request.

## What to do

Ask the operator to accept the extension, if storing it unread is right; see [configuration](@/docs/configuration.md#extensions-in-critical).
