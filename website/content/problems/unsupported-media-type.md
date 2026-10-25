+++
title = "unsupported-media-type"
template = "problem.html"
[extra]
status = 415
problem_title = "This media type is not taken"
+++

## When

The `Content-Type` is not `application/vcon`, `application/vcon+gzip` or `application/json` (any case, any parameters), and not absent.

## What to do

Send one of those media types.
