+++
title = "not-a-vcon"
template = "problem.html"
[extra]
status = 422
problem_title = "The body is not a vCon this server can read"
+++

## When

The vcon library this server is built with refused the body for a reason this server does not name with a problem of its own. Every refusal the library has today has its own problem; this one answers only for a refusal a later release of the library adds, until the server names it.

## What to do

Check the body against the vCon format; the [storing vCons](@/docs/storing-vcons.md#what-is-checked) page lists what is checked.
