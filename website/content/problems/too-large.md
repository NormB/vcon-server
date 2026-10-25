+++
title = "too-large"
template = "problem.html"
[extra]
status = 413
problem_title = "The body is over the size limit"
+++

## When

The body is longer than `limits.body`, checked against `Content-Length` before anything is read and again as it arrives; or its gzip layers, together, inflate past `limits.inflated`.

## What to do

Send long recordings by URL rather than inline, or raise the limit. See [configuration](@/docs/configuration.md#where-the-defaults-come-from).
