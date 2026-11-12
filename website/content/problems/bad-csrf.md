+++
title = "bad-csrf"
template = "problem.html"
[extra]
status = 403
problem_title = "The session's CSRF token is missing or wrong"
+++

## When

A request by session other than `GET /session` came with a live session's cookie and the listener's own origin, and its `X-CSRF-Token` header is missing or does not hold the session's CSRF token. The token is compared in constant time. The session stays live and is not counted as seen. Each refusal is counted as `auth_failed` with status 403 under the user (`user:<id>`), in the tenant's audit log for a session in a tenant and in the admin log for an operator's.

## What to do

Ask `GET /session` for the token, the member `csrf_token` of its answer, and send it in the header `X-CSRF-Token` of every other request by that session. A session given a new id, as a step-up does, has a new token: ask again. See [sessions](@/docs/session-origin.md).
