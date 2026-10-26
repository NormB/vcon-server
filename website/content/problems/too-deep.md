+++
title = "too-deep"
template = "problem.html"
[extra]
status = 422
problem_title = "The body nests too deeply"
+++

## When

The body has more signed (JWS) layers, one inside the other, than the server reads; or a JSON text in it, the body itself or a signed payload, has more arrays and objects open at once than the server reads. Both limits are fixed; see [fixed limits](@/docs/configuration.md#fixed-limits).

## What to do

Send the vCon with fewer layers: a vCon signed again for long-term archiving keeps every earlier layer, so verify it and sign the innermost vCon afresh. Keep the vCon's own JSON shallower.
