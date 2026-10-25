+++
title = "bad-uuid"
template = "problem.html"
[extra]
status = 422
problem_title = "The uuid is not in RFC 9562 text form"
+++

## When

The vCon's `uuid` is not a string in RFC 9562 text form.

## What to do

Write the uuid as 36 characters with hyphens.
