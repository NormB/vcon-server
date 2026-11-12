+++
title = "wrong-listener"
template = "problem.html"
[extra]
status = 403
problem_title = "This listener does not serve this kind of key"
+++

## When

The server has an operator listener (`[operator]` in its configuration), the key is valid, and the request came to the listener that serves the other kind of key. The address in `listen` serves tenant keys only. The address in `operator.listen` serves operator keys only. The key is refused before any route sees the request, so nothing was read or changed. Each refusal is counted in the audit log as `wrong_listener`: in the admin log for an operator key, and in the tenant's log for a tenant key. A server without an operator listener never answers this. A live session is held to the same rule on the two [session routes](@/docs/session-origin.md): a tenant user's session is served on the address in `listen` and an operator's on the address in `operator.listen`, and one that reaches the other listener is refused and counted as `wrong_listener` under `user:` and the user id.

## What to do

Send the request to the other listener: an operator key to the address in `operator.listen`, a tenant key to the address in `listen`. The server's `serve` log names both addresses when it starts. See [the operator listener](@/docs/operator-listener.md).
