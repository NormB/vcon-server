+++
title = "bad-path-uuid"
template = "problem.html"
[extra]
status = 400
problem_title = "The path's uuid is not in RFC 9562 text form"
+++

## When

The `{uuid}` in the path is not a uuid written as 36 characters with hyphens.

## What to do

Use the uuid as the vCon writes it.
