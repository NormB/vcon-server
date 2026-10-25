+++
title = "ambiguous-form"
template = "problem.html"
[extra]
status = 422
problem_title = "The body is both signed and encrypted"
+++

## When

The object has both `payload` with `signatures` and `ciphertext` with `recipients`.

## What to do

Send the vCon in one form. A signed vCon that is then encrypted is a JWE whose plaintext is the JWS.
