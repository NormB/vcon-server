+++
title = "not-json"
template = "problem.html"
[extra]
status = 422
problem_title = "The body is not one JSON object"
+++

## When

The body (once inflated) is not valid JSON, or is JSON but not an object.

## What to do

Send one vCon as one JSON object.
