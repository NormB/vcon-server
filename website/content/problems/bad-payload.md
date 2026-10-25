+++
title = "bad-payload"
template = "problem.html"
[extra]
status = 422
problem_title = "The signed payload cannot be read"
+++

## When

In a signed vCon, `signatures` is not an array of objects, or `payload` is not base64url of a vCon.

## What to do

Send a JWS in general JSON serialization whose payload is the vCon.
