+++
title = "Console"
description = "Open the web console the server serves, sign in with an API key, and what the console does and does not keep."
weight = 72
+++

The server serves a web console on its own TLS listener. The console is a
page that calls the [HTTP API](@/docs/http-api.md) with the key you sign in
with, so it can do exactly what that key can do over the API, and every
request it makes is checked and recorded in the
[audit log](@/docs/audit-logs.md) as any client's would be.

## Open it

Open `/console/` on the server's address in a browser, for example:

```text
https://vcon.example.com:8443/console/
```

`/console` without the slash is sent on to `/console/`. The page and its two
files need no key; they hold no data.

## Sign in

1. Paste an API key into **API key**. Any key works: a tenant key with any
   of its scopes, or an operator key. See
   [tenants and keys](@/docs/tenants-and-keys.md) to make one.
2. Choose **Sign in**. The console asks the server about the key with
   `GET /key`. If the server accepts it, the console shows the key's id,
   tenant and scopes. If not, it shows
   "The key was not accepted." and nothing else.
3. Choose **Sign out** when you are done.

## What the console keeps

- The key is held in the page's memory only. It is never written to the
  browser's storage, a cookie or the address bar. Reloading the page, closing
  the tab or signing out forgets it, and you sign in again.
- The console asks the browser not to cache any answer, and writes nothing
  the server sends to browser storage.
- Everything the server sends is shown as text. Markup inside a stored vCon,
  a tenant name or a key label is shown as the characters it is made of and
  never runs.

## What protects it

Every console answer carries these headers, so the page runs only its own
script, loads nothing from another site, cannot be shown inside another
site's frame and is not cached:

```text
Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'
X-Frame-Options: DENY
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
Cache-Control: no-store
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Resource-Policy: same-origin
```

The server sets no cookie and sends no cross-origin (CORS) headers, so a page
on another site cannot send your key or read an answer. The console cannot
protect a key from code that already runs inside your browser, such as a
hostile extension.

The console is always served; it adds no power that a key does not already
have.
