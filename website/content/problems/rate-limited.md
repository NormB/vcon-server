+++
title = "rate-limited"
template = "problem.html"
[extra]
status = 429
problem_title = "Too many tool calls from this key"
+++

## When

An MCP tool call came from a key that has already made `mcp.calls_per_second` calls in the current one-second window (100 unless configured). The call did not run. The refusal's `retry_after_ms` says how long until the window ends. Refusals of this kind are counted in the audit log, one record a minute, not one each.

## What to do

Wait `retry_after_ms` milliseconds and call again, or make fewer calls a second.
