+++
title = "path-uuid-mismatch"
template = "problem.html"
[extra]
status = 422
problem_title = "The body's uuid is not the one in the path"
+++

## When

A `PUT /vcons/{uuid}` body holds a vCon with another uuid.

## What to do

Send the vCon to its own uuid's path.
