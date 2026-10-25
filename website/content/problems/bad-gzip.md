+++
title = "bad-gzip"
template = "problem.html"
[extra]
status = 422
problem_title = "The body is marked gzip but is not"
+++

## When

The body says it is gzip, by its media type or its first two bytes, and does not inflate.

## What to do

Send the vCon plain, or gzipped correctly.
