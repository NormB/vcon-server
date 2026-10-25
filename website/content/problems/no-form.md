+++
title = "no-form"
template = "problem.html"
[extra]
status = 422
problem_title = "The body is in no vCon form"
+++

## When

The object is neither signed (`payload` and `signatures`), encrypted (`ciphertext` and `recipients`) nor unsigned (any of `uuid`, `parties`, `dialog`, `analysis`, `attachments`).

## What to do

Send a vCon.
