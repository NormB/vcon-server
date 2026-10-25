+++
title = "damaged"
template = "problem.html"
[extra]
status = 500
problem_title = "A stored version is damaged"
+++

## When

A stored version's file is missing or does not decrypt and authenticate.

## What to do

Tell the operator. The audit log records the request; `vcon-server check` lists every damaged version (see [checking the data](@/docs/operations.md#checking-the-data)).
