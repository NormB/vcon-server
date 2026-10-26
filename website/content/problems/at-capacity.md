+++
title = "at-capacity"
template = "problem.html"
[extra]
status = 503
problem_title = "Too many tool calls are running"
+++

## When

An MCP tool call arrived while `mcp.concurrent` tool calls were already running on the server (100 unless configured). The call did not run; it is refused at once, never queued. Refusals of this kind are counted in the audit log, one record a minute, not one each.

## What to do

Call again shortly, with backoff.
