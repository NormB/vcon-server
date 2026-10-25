+++
title = "timeout"
template = "problem.html"
[extra]
status = 408
problem_title = "The body did not arrive in time"
+++

## When

The body took longer than `limits.timeouts.read` seconds to arrive.

## What to do

Retry. The request was not carried out; a retry of the same bytes is safe.
