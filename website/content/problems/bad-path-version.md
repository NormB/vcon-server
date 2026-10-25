+++
title = "bad-path-version"
template = "problem.html"
[extra]
status = 400
problem_title = "The path's version is not a version id"
+++

## When

The `{version}` in a legal route's path is not a version id: 32 lowercase hexadecimal digits.

## What to do

Use an id from `GET /legal/vcons/{uuid}/versions`, or an `ETag` without its quotes.
