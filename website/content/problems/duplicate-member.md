+++
title = "duplicate-member"
template = "problem.html"
[extra]
status = 422
problem_title = "An object repeats a member"
+++

## When

Some JSON object in the body, at any depth, names the same member twice. JSON parsers disagree on which one wins, so such a vCon can mean different things to different readers.

## What to do

Fix the producer's serialiser.
