+++
title = "uuid-mismatch"
template = "problem.html"
[extra]
status = 422
problem_title = "A signature names another uuid"
+++

## When

A signature's header names a `uuid` other than the one in the signed vCon.

## What to do

Sign the vCon with headers that name its own uuid.
