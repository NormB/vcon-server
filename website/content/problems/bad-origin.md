+++
title = "bad-origin"
template = "problem.html"
[extra]
status = 403
problem_title = "The request does not come from this listener's own pages"
+++

## When

A request by session did not show that a page of this listener made it. `POST /session/logout` needs one `Origin` header equal to the listener's public origin, the value of `public_origin` or `operator.public_origin` in the configuration. `GET /session` needs one `Sec-Fetch-Site: same-origin` header, which a browser sets on a request a page makes to its own origin, and an `Origin` header, when it has one, equal to the public origin. A listener with no public origin configured answers this to every request by session. The check is made before the session is looked up, so the request learns nothing about its cookie, and the session is not counted as seen. Each refusal is counted in the admin audit log as `auth_failed` with status 403 and no actor.

## What to do

- **The listener has no public origin:** set `public_origin` (and `operator.public_origin` for the operator listener) and restart; see [sessions](@/docs/session-origin.md).
- **The public origin is not what the browser's address bar shows:** the scheme, host and port must be equal, in lower case, with no path and no `/` at the end. Port 443 is left out.
- **A reverse proxy is in front of the server:** it must pass the `Origin` and `Sec-Fetch-Site` headers of the browser's request without change.
- **The request comes from a script:** a script uses a key, not a session.
