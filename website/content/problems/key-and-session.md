+++
title = "key-and-session"
template = "problem.html"
[extra]
status = 400
problem_title = "The request carries both a key and a session cookie"
+++

## When

The request has an `Authorization` header and a cookie named `__Host-vcs_session`. A script signs its requests with a key, and a browser with a session; a request with both is refused on every route, before the key or the cookie is read, so a valid key and a live session are refused too. Nothing was read or changed. Each refusal is counted in the admin audit log as `auth_failed` with status 400 and no actor.

## What to do

Send one of the two. A script or a command-line client sends the key and no `Cookie` header: check that the HTTP client does not share a cookie store with a browser session of the same host. A page signed in by session sends the cookie and no `Authorization` header. See [sessions](@/docs/session-origin.md).
