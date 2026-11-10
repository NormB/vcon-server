+++
title = "Console"
description = "Open the web console the server serves, sign in with an API key, and what the console does and does not keep."
weight = 53

[extra]
group = "reference"
+++

The server serves a web console on its own TLS listener. The console is a
page that calls the [HTTP API](@/docs/http-api.md) with the key you sign in
with, so it can do exactly what that key can do over the API, and every
request it makes is checked and recorded in the
[audit log](@/docs/audit-logs.md) as any client's would be.

## Before you start

- **A running server.** The console is part of the server and is always
  served, on the same TLS listener as the HTTP API. There is no setting to
  turn it on. See [getting started](@/docs/getting-started.md).
- **An API key.** Any key signs in. What the console then shows depends on
  the key's scopes; see [tenants and keys](@/docs/tenants-and-keys.md) to
  make one.
- **A browser that reaches the server's address** and trusts its
  certificate, or in which you have accepted it.
- **Nothing on any other system.** A reverse proxy in front of the server
  must pass `/console/` and the files under it, and each answer's headers,
  unchanged.

Two things on this page are on `main` only, in no release yet. Release
0.4.2 records each Reveal as `read` or `legal_read`, not under a reveal
action, and it has no [recording player](#play-a-recording). The
[status](@/docs/status.md) page lists what no release holds.

## Open it

Open `/console/` on the server's address in a browser, for example:

```text
https://vcon.example.com:8443/console/
```

`/console` without the slash is sent on to `/console/`. The page and its
files need no key; they hold no data. They are `/console/console.css`,
`/console/console.js`, `/console/format.js` and, for the
[recording player](#play-a-recording), `/console/player.html` and
`/console/player.js`.

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
more. On `main` only, in no release yet (release 0.4.2 records each Reveal
as one more `read` or `legal_read`), each Reveal leaves a record that
names the part it showed:
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

## Play a recording

The player is on `main` only, in no release yet: release 0.4.2 has no Play
button and no player page.

**Purpose:** hear a recording that a vCon holds inside one of its dialogs,
in the browser, without saving the vCon, and leave a record that it was
played.

**Prerequisites:**

- A key with the `read` scope, or the `legal` scope for the Legal view.
- The recording is inside the vCon: the dialog has a `body` encoded as
  `base64url` (or `base64`) and a `mediatype` of `audio/wav`,
  `audio/mpeg`, `audio/ogg` or `audio/opus`, or one of the other names in
  the table below, and is at most 104,857,600 bytes. The type is read
  without its case or its parameters, so `Audio/X-WAV; codecs=1` is
  `audio/x-wav`. A recording the dialog only links to (`url`) is never played,
  because the console never loads a link from a vCon.
- The browser lets this site open a tab. The player is a tab of its own;
  a browser set to block pop-ups must allow them for the server's address.
- On other systems: a reverse proxy or firewall that passes the console's
  paths by name must also pass `/console/player.html` and
  `/console/player.js`, and must pass the `Content-Security-Policy` header
  of each answer unchanged. Nothing changes on the server: there is no
  setting.

| Declared as | Played as | Its bytes must start as |
|---|---|---|
| `audio/wav`, `audio/x-wav`, `audio/wave`, `audio/vnd.wave` | `audio/wav` | a RIFF file of the WAVE kind |
| `audio/mpeg`, `audio/mp3`, `audio/x-mp3` | `audio/mpeg` | an ID3 tag or an MPEG audio frame |
| `audio/ogg`, `audio/x-ogg` | `audio/ogg` | an Ogg stream |
| `audio/opus` | `audio/opus` | an Ogg stream whose first packet is an Opus header |

A type that is not in the table is not played. `application/ogg` is not
in it: that name does not say whether the stream is audio or video, and
the console does not read far enough into a stream to tell.

**Steps:**

1. Open the vCon, as in [open a vCon masked](#open-a-vcon-masked). Under
   the reveal buttons there is one button for each dialog with a recording
   the console plays: **Play dialog 1 recording (opens the player in a new
   tab)**.
2. Choose it. The console opens the player's tab, reads the vCon again as
   a [reveal](@/docs/audit-logs.md#reveals) of `audio`, checks that the
   recording's first bytes are the type the dialog declares, and hands the
   bytes to the tab. The console then says
   `Dialog 1's recording was handed to the player's tab.`
3. In the player, read the line that names what it holds, such as
   `Ready to play: Dialog 1 of <uuid>, audio/wav. It does not start by itself: use the controls below.`
   Focus is on the browser's own audio controls: play, pause, seek and
   volume, by mouse or keyboard.
4. When playback ends, the player lets go of the recording and says so.
   To hear it again, choose Play in the console again.
5. Choose **Close the player** when you are done. Focus returns to the
   Play button in the console, which says `The player was closed.`
   Signing out of the console closes every player it opened.

**Verify:** read the tenant's [audit log](@/docs/audit-logs.md). Each Play
leaves one `reveal_audio` record for the vCon's uuid, or
`legal_reveal_audio` from the Legal view, with the key that played it.

**Troubleshoot:**

- *There is no Play button for a dialog:* its recording is at a link, or
  its body is not declared as a type in the table above, is not encoded as
  `base64url` or `base64`, or is over the size limit. **Download** has
  the whole vCon. The IETF draft's example vCons are of the first kind:
  each keeps its recording at a link (`url`), declared `audio/x-mp3`, so
  they have no Play button as published; with the recording put inside
  the dialog, they play.
- *`The browser blocked the player's tab. Allow pop-ups for this site, then choose Play again.`:*
  allow pop-ups for the server's address in the browser's settings. Nothing
  was read, and nothing is recorded.
- *`Dialog 1's recording is declared as audio/wav, but its bytes are not audio/wav.`:*
  the bytes do not start as that kind of file does. For another name the
  line gives both, such as
  `declared as audio/x-mp3, but its bytes are not audio/mpeg`. The console hands
  nothing to the player. The read is recorded, because the console had to
  read the vCon to find out.
- *The player says `The browser could not play this recording.`:* the bytes
  start as audio does but the browser's decoder refuses them, or the
  browser has no decoder for the type. **Download** the vCon and use
  another tool.
- *The player says it is waiting and nothing arrives:* the console's tab
  was closed or reloaded, which forgets the key. Close the player, sign in
  again and choose Play.
- *The player's tab is blank or shows a refusal:* a proxy in front of the
  server does not pass `/console/player.html` or `/console/player.js`.

What the player is and is not:

- It is a page of its own because a browser applies one
  content-security policy to a whole page. The console's policy plays no
  media at all, and stays as it is. The player's policy alone has
  `media-src blob:`, and has `connect-src 'none'`: the player can play a
  blob made in its own page and can make no request of any kind.
- It never has the key. The console reads the vCon and hands the player
  the recording's bytes in a message to its own origin; the player takes a
  message only from the console that opened it, only once, and only when
  it is one recording of a listed type whose bytes carry that type's
  signature.
- It keeps the recording in memory only: nothing in browser storage, and
  nothing in its address, which is the same for every recording. It lets
  go of the recording when playback ends and when the tab closes.
- It never starts playing by itself.

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

## When something does not work

- *The browser warns about the certificate:* the server's certificate is
  not one the browser trusts. Use the address the certificate names, or
  give the server a certificate from an authority the browser trusts; see
  [getting started](@/docs/getting-started.md).
- *`The key was not accepted.`:* the key is mistyped, revoked, or of
  another server. The console says no more than this, as the API does.
  Paste the whole token, or make a new key.
- *A view is not there:* the key lacks the scope the view needs; see the
  table under [what you see](#what-you-see). Sign in with a key that has
  it.
- *`The server could not be reached. Check the connection and try again.`:*
  the request got no answer. The server is stopped, the network is down,
  or a proxy closed the connection. The key is still held; choose the
  action again.
- *A request is refused with a status:* the line shows the problem's title,
  its status and what to do next. Each problem type has a page under
  [problems](@/problems/_index.md).
- *The page is blank, or has no styles:* a proxy in front of the server
  does not pass one of the console's files, or changes the
  `Content-Security-Policy` header. The files are listed under
  [open it](#open-it).
- *After a reload the console asks for the key again:* that is how it
  works. The key is held in the page's memory only.

Each task on this page has its own troubleshooting list:
[open a vCon masked](#open-a-vcon-masked) and
[play a recording](#play-a-recording).

## Keyboard and screen readers

The console is built to WCAG 2.2 level AA, and its browser tests run
axe-core on every view, in light and dark, at 320 and 1280 pixels wide.

- The first Tab stop is **Skip to the content**.
- Every control has a label and shows a focus ring when the keyboard
  reaches it. Choosing a view moves focus to its heading, and a reveal
  moves focus to the revealed text.
- A Play button says that it opens a new tab. The player has one heading,
  a line that names the recording, the browser's own audio controls, which
  take focus when the recording arrives, and a button that closes it and
  returns focus to the Play button.
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
- A recording handed to the [player](#play-a-recording) is held in that
  tab's memory until playback ends or the tab closes, and nowhere else.
- Everything the server sends is shown as text. Markup inside a stored vCon,
  a tenant name or a key label is shown as the characters it is made of and
  never runs.

## What protects it

Every console answer but one carries these headers, so the page runs only
its own script, loads nothing from another site, cannot be shown inside
another site's frame and is not cached:

```text
Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'
X-Frame-Options: DENY
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
Cache-Control: no-store
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Resource-Policy: same-origin
```

The one other is the [recording player](#play-a-recording)'s page,
`/console/player.html`, when it is served. It carries the same headers with
this policy in place of the first, which differs in two places: the page
may play a blob it made (`media-src blob:`) and may make no request at all
(`connect-src 'none'`):

```text
Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'none'; media-src blob:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; require-trusted-types-for 'script'; trusted-types 'none'
```

The server sets no cookie and sends no cross-origin (CORS) headers, so a page
on another site cannot send your key or read an answer. The console cannot
protect a key from code that already runs inside your browser, such as a
hostile extension.

The console is always served; it adds no power that a key does not already
have.
