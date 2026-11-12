+++
title = "no-session"
template = "problem.html"
[extra]
status = 401
problem_title = "A live session is needed"
+++

## When

`GET /session` or `POST /session/logout` was asked with no cookie named `__Host-vcs_session`, with more than one, or with one that names no live session. A session stops being live when it has not been seen for its idle limit (15 minutes unless the tenant set less), at its absolute limit (8 hours unless the tenant set less), when its user signs out, when the roles of its user change or the user is deactivated, when the role cap of its tenant changes, and when its tenant is removed. The answer is the same in each case, so a cookie that was never a session learns nothing. When the request sent a cookie, the answer has a `Set-Cookie` header that removes it from the browser. A key in an `Authorization` header is not a session: these two routes take no key. Each refusal is counted in the admin audit log as `auth_failed` with status 401 and no actor.

## What to do

Sign in again. This release has no sign-in method yet, so no browser holds a session; see [the status page](@/docs/status.md). A script uses a key and the routes that take one; see [the HTTP API](@/docs/http-api.md).
