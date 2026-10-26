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

## What you see

After you sign in, the console shows a button for each view your key's
scopes allow. A view, or a control inside one, that the key may not use is
not shown; the API would refuse its request anyway.

| View | Shown for | What it does |
|---|---|---|
| **vCons** | `read`, `write` or `delete` | lists, reads, stores and deletes vCons, as below |
| **Legal** | `legal` | reads a vCon as a legal key sees it, and its versions |

## Work with vCons

Choose **vCons**.

- **List them** (`read`): the tenant's live vCons, newest stored first, 25
  at a time, with each one's uuid, current version, and when it was first
  stored and last changed. Choose **More** for the next 25. The list uses
  `GET /vcons`.
- **Read one** (`read`): choose **Open** beside it in the list, or type its
  uuid under **vCon uuid** and choose **Open**. The console shows its
  version (`ETag`) and the vCon as text (`GET /vcons/{uuid}`). A vCon stored
  gzipped is inflated first.
- **Store one** (`write`): under **Store a vCon from a file**, pick the file
  and choose **Store**. The console sends the file's bytes unchanged
  (`POST /vcons`) and shows the stored uuid and version, or why the server
  refused it, such as `The body is not one JSON object (422)`.
- **Delete one** (`delete`): choose **Delete** under the vCon you opened, or
  type its uuid and choose **Delete**, then confirm. The console calls
  `DELETE /vcons/{uuid}`.

## Look up a vCon as a legal key

Choose **Legal**, type the vCon's uuid and choose **Find**. The console shows
whether the vCon is `live` or `deleted`, its current version (or, once
deleted, the last kept one) as text, and every kept version with when it was
stored and hidden. Choose **Open** beside a version to read it. These use
`GET /legal/vcons/{uuid}`, `GET /legal/vcons/{uuid}/versions` and
`GET /legal/vcons/{uuid}/versions/{version}`.

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
