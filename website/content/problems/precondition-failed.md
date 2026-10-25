+++
title = "precondition-failed"
template = "problem.html"
[extra]
status = 412
problem_title = "If-Match does not name the current version"
+++

## When

`If-Match` names a version that is no longer current, or is not one strong tag (`*`, a weak tag or a list name no version).

## What to do

Read again, merge your change into the current version, and retry with its `ETag`.
