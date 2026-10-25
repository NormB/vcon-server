+++
title = "bad-critical"
template = "problem.html"
[extra]
status = 422
problem_title = "critical or must_support is not a list of names"
+++

## When

`critical` (or `must_support`) is present but is not an array of strings.

## What to do

Write the extensions a reader must support as an array of names.
