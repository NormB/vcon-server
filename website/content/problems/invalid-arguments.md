+++
title = "invalid-arguments"
template = "problem.html"
[extra]
status = 422
problem_title = "The arguments do not fit the tool"
+++

## When

An MCP tool call's arguments do not fit the tool's input schema: a member is missing or of the wrong type, or a member the tool does not take was sent. The refusal does not repeat the arguments. It is recorded in the audit log under the tool's action.

## What to do

Send the arguments the tool's `inputSchema` in `tools/list` describes, and nothing else.
