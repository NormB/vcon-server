+++
title = "unknown-tool"
template = "problem.html"
[extra]
status = 404
problem_title = "No tool has the name"
+++

## When

An MCP `tools/call` named a tool this server does not have. The refusal is recorded in the audit log under the action `mcp`.

## What to do

Call `tools/list` for the tools the key may call, and use one of their names.
