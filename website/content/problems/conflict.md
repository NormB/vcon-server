+++
title = "conflict"
template = "problem.html"
[extra]
status = 409
problem_title = "Another vCon holds the uuid"
+++

## When

`POST /vcons` with a uuid this tenant already holds, with different bytes. (The same bytes answer `200`.)

## What to do

Two vCons share a uuid, or this one changed and should be sent with `PUT` and `If-Match`.
