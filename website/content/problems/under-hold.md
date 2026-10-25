+++
title = "under-hold"
template = "problem.html"
[extra]
status = 409
problem_title = "A vCon of the tenant is under a legal hold"
+++

## When

A removal with `force=true`, and a uuid of the tenant is under a legal hold. Nothing over HTTP passes a legal hold.

## What to do

Release the hold, if it may be released. Only the command line, with the server stopped, can override holds.
