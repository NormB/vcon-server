+++
title = "precondition-required"
template = "problem.html"
[extra]
status = 428
problem_title = "If-Match must name the current version"
+++

## When

A `PUT /vcons/{uuid}` came without `If-Match`. A replace must say which version it replaces, so that two writers cannot overwrite each other unseen.

## What to do

Read the vCon, take its `ETag`, and send it as `If-Match`.
