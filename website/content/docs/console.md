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

`/console` without the slash is sent on to `/console/`. The page and its
files (a stylesheet and two scripts) need no key; they hold no data.

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
| **Legal** | `legal` | reads a vCon as a legal key sees it and its versions; holds, releases and restores it |
| **Audit log** | `audit`, or an operator key | the tenant's audit log, or for an operator key the admin log |
| **Tenants** | an operator key | tenants and their keys |

## Work with vCons

Choose **vCons**.

- **List them** (`read`): the tenant's live vCons, newest stored first, 25
  at a time, with each one's uuid, current version, and when it was first
  stored and last changed. Choose **More** for the next 25. The list uses
  `GET /vcons`.
- **Read one** (`read`): choose **Open** beside it in the list, or type its
  uuid under **vCon uuid** and choose **Open**. The console shows its
  version (`ETag`) and a masked summary (`GET /vcons/{uuid}`); see
  [open a vCon masked](#open-a-vcon-masked). A vCon stored gzipped is
  inflated first.
- **Store one** (`write`): under **Store a vCon from a file**, pick the file
  and choose **Store**. The console sends the file's bytes unchanged
  (`POST /vcons`) and shows the stored uuid and version, or why the server
  refused it, such as
  `The body is not one JSON object (422). Correct what was sent and try again.`
- **Delete one** (`delete`): choose **Delete** under the vCon you opened, or
  type its uuid and choose **Delete**, then confirm. The console calls
  `DELETE /vcons/{uuid}`.

## Open a vCon masked

**Purpose:** see what a vCon holds without putting names, numbers or what
was said on the screen, and leave a record each time the content itself is
shown.

**Prerequisites:** a key with the `read` scope, and a browser that reaches
the server's TLS listener. Nothing changes on any other system: masking is
part of the console page the server already serves, and needs no setting.

**Steps:**

1. Sign in, choose **vCons**, and choose **Open** beside a vCon. With a
   `legal` key, choose **Legal** and **Find** a vCon instead: the version
   found, and each kept version you **Open**, is masked the same way.
2. Read the summary. It shows:
   - the form: `unsigned`, `signed (JWS)` or `encrypted (JWE)`;
   - when the vCon says it was created, in UTC;
   - whether it has a subject (`present, hidden` or `none`);
   - the number of dialogs, their total seconds, and the number of
     attachments and analyses;
   - each dialog as a line such as
     `Dialog 1: text, 2026-10-23 12:00:00 UTC+00:00, 30 s, parties 1, 2; content hidden`,
     with its type, start, length and parties and never what was said. A
     type the vCon format does not name shows as `other type`;
   - how many links the vCon holds to places outside the server;
   - each party as a line such as
     `Party 1: number ending 0123; address at example.com; name hidden`.
     A telephone number shows its last four digits, and one of fewer than
     eight digits is hidden whole. An email address shows its domain only.
   - for a signed vCon, the `alg`, `kid` and `x5t` of its first signature's
     header, and `not verified by the console`. The summary is read from
     the signed payload. An encrypted vCon is not decoded.
3. To see one part, choose **Reveal parties**, **Reveal subject**,
   **Reveal dialog** or **Reveal analysis**. The console reads the vCon
   again, as a [reveal](@/docs/audit-logs.md#reveals) of that part, and
   shows that part alone as text. Inline media (a body with a
   `base64url` or `base64` encoding) is named by its size and not shown.
4. To see where the vCon links to, choose **Reveal links**. Each link is
   shown as text with where it is, such as `dialog 2`, and
   `points outside the server and is never loaded`. The console makes no
   request to a link and never shows one as a link, an image or a player.
   **Copy** puts the link's text on the clipboard.
5. To see the whole vCon, choose **Reveal the whole vCon** (in the Legal
   view, **Reveal this version** or **Reveal the opened version**). The
   console reads it again and shows it as text. Past 2,097,152 characters
   the text is cut there, and a line says how many more there are.
6. To keep a copy, choose **Download**. The console reads the vCon again
   and the browser saves it as `<uuid>.json`, as plain bytes
   (`application/octet-stream`), under the uuid and never under a name
   from the vCon. A vCon stored gzipped is saved inflated.

**Verify:** read the tenant's [audit log](@/docs/audit-logs.md) with an
`audit` key, in the **Audit log** view or over the API. Opening a vCon
leaves one `read` record for its uuid, and each **Download** leaves one
more. Each Reveal leaves a record that names the part it showed:
`reveal_parties`, `reveal_subject`, `reveal_dialog`, `reveal_analysis`,
`reveal_links`, or `reveal_all` for the whole vCon. In the Legal view each
find and each version opened leaves a `legal_read` record, and each Reveal
a `legal_reveal_` record with the same part names. Opening another vCon,
or the same one again, masks it again.

**Troubleshoot:**

- *The summary says `not JSON` or `not a JSON object`:* the stored bytes are
  not a vCon the console can summarize. **Reveal the whole vCon** still
  shows them as text.
- *A part shows `This vCon cannot be read in parts. Reveal the whole vCon
  instead.`:* the vCon is encrypted, or is not a JSON object.
- *Copy answers `The browser did not allow the copy.`:* the browser refused
  clipboard access. Select the link's text and copy it.
- *Reveal answers `(410)`:* the vCon was deleted after you opened it. A key
  with the `legal` scope can still find it under **Legal**.
- *Nothing happens after Reveal and a line says the server could not be
  reached:* the request got no answer. Check the connection and choose
  Reveal again.
- *Reveal answers `(401)`:* the key was revoked. Sign in with another key.
- *Reveal answers `(400)`:* the server is older than the console's page
  and does not know the part. Reload the page, so the page and the server
  are of one version.
- *The audit log shows `read` where you expect a reveal:* the page in the
  browser is from before the server was upgraded. Reload it.

Masking is what the console shows, not what the server sends: the API
answers a `read` key with the whole vCon, and the console keeps it only long
enough to make the summary. A reveal is recorded under an action of its
own because the console marks the read it makes for one; see
[reveals](@/docs/audit-logs.md#reveals) for what the marker does and does
not do.

## Lists, times and refusals

Every list says `Loading …` while its request is out and `No … to show.`
when the answer holds nothing. A list that could not be read shows why
instead, and a request that got no answer shows
`The server could not be reached. Check the connection and try again.`

Every time the console shows is in UTC, written with its offset, such as
`2026-10-23 12:00:00 UTC+00:00`. When the server refuses a request, the
console shows the problem's title, its status and what to do next, such as
`The body is not one JSON object (422). Correct what was sent and try again.`

## Look up a vCon as a legal key

Choose **Legal**, type the vCon's uuid and choose **Find**. The console shows
whether the vCon is `live` or `deleted`, its current version (or, once
deleted, the last kept one) as text, and every kept version with when it was
stored and hidden. Choose **Open** beside a version to read it. Both are
shown masked first; see [open a vCon masked](#open-a-vcon-masked). These use
`GET /legal/vcons/{uuid}`, `GET /legal/vcons/{uuid}/versions` and
`GET /legal/vcons/{uuid}/versions/{version}`.

Under the vCon found:

- **Hold** puts a legal hold on it (`PUT /legal/vcons/{uuid}/hold`).
- **Release hold** takes the hold off after you confirm
  (`DELETE /legal/vcons/{uuid}/hold`).
- **Restore** makes a deleted vCon live again with its last kept version
  (`POST /legal/vcons/{uuid}/restore`). A live vCon is refused with
  `not-deleted` (409).

## Read the audit log

Choose **Audit log**. A tenant key with the `audit` scope sees its tenant's
log (`GET /audit`); an operator key sees the admin log (`GET /admin/audit`).
Records are listed oldest first, 50 at a time, with their `seq`, time,
actor, action, target, version, status and count; choose **More** for the
next 50. Each page read is itself recorded, as `audit_read`. See
[audit logs](@/docs/audit-logs.md#a-record) for the fields.

## Manage tenants and keys

Sign in with an operator key and choose **Tenants**.

- **Create a tenant:** type its **Name** and, if it should not use the
  server default, a **Retention** period such as `7y`, `18m` or `90d`, then
  choose **Create tenant** (`POST /admin/tenants`).
- **Change its retention:** choose **Retention** in its row and type the new
  period, or leave it empty for the server default
  (`PATCH /admin/tenants/{id}`).
- **Remove it:** choose **Remove** and confirm (`DELETE /admin/tenants/{id}`).
  If it still holds vCons, the console asks again before it removes them
  too (`?force=true`). A tenant with any vCon under a legal hold is never
  removed over the API.
- **Its keys:** choose **Keys** to list them. To issue one, tick its scopes,
  type a **Label** and choose **Issue key**
  (`POST /admin/tenants/{id}/keys`). The token is shown once, under the
  form: copy it then. It is gone once you leave the view, and the server
  cannot show it again. **Revoke** a key and confirm to end it
  (`DELETE /admin/keys/{id}`).

Each of these leaves the same audit record as the same request made with
`curl` or any other client.

## Keyboard and screen readers

The console is built to WCAG 2.2 level AA, and its browser tests run
axe-core on every view, in light and dark, at 320 and 1280 pixels wide.

- The first Tab stop is **Skip to the content**.
- Every control has a label and shows a focus ring when the keyboard
  reaches it. Choosing a view moves focus to its heading, and a reveal
  moves focus to the revealed text.
- The view shown is marked for assistive technology, and in bold and
  underlined.
- Refusals are announced at once; list states and the result of a store
  are announced when the screen reader is idle.
- The page follows the system's light or dark setting, and a wide table
  scrolls inside its own region, so the page never scrolls sideways.

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
