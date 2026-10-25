+++
title = "bad-scopes"
template = "problem.html"
[extra]
status = 422
problem_title = "A tenant key cannot hold these scopes"
+++

## When

A key request names an unknown scope, a scope twice, no scope, or `admin`.

## What to do

Name one or more of `write`, `read`, `delete` and `legal`, each once.
