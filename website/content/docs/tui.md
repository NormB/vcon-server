+++
title = "Terminal UI"
description = "Look at a tenant's vCons in a terminal with vcon-server tui: set it up on one machine or two, use it task by task, and look up every key, setting and message."
weight = 32

[extra]
group = "how-to"
+++

`vcon-server tui` shows a tenant's vCons in a terminal: each vCon's uuid,
when the server first stored it and when it last changed, newest first. It
reads them from a running server over HTTPS with one API key.

{{ <tui_screen name="list" label="What the command shows after a start that works, in a terminal of 80 columns by 24 lines: the list of a tenant's vCons, one on each line with its uuid and two times, under a first line with the server's address, the tenant's id and the word connected." /> }}

Press Enter on a vCon to see its version and times. Press `?` for the keys
and `q` to leave. The command reads and never writes, and it never asks
the server for a vCon's content.

## Every screen

The command draws these screens. Each name leads to the screen as the
command draws it, what it shows, its keys and how to leave it.

| Screen | When it appears |
|---|---|
| [The list](#the-list) | After a start that works, when the server holds more vCons than those read. |
| [The list with every vCon read](#the-list-with-every-vcon-read) | When the server holds no older vCon than those on the screen. |
| [The list of one vCon](#the-list-of-one-vcon) | When the tenant holds one vCon. |
| [The list while the next page is read](#the-list-while-the-next-page-is-read) | After `n`, until the server answers. |
| [The list after the next page](#the-list-after-the-next-page) | When the page that `n` asked for has arrived. |
| [A next page that was not read](#a-next-page-that-was-not-read) | When the request that `n` made failed. |
| [A tenant with no vCons](#a-tenant-with-no-vcons) | When the server answered and the tenant holds no vCon. |
| [Connecting](#connecting) | From the start, or from `r` after no answer, until the server answers for the key. |
| [Loading](#loading) | While the first page of vCons is on its way. |
| [A server that did not answer](#a-server-that-did-not-answer) | When no answer came for the key. |
| [A key the server refused](#a-key-the-server-refused) | When the server answered 401 for the key. |
| [A certificate that was refused](#a-certificate-that-was-refused) | When the server's certificate did not pass the check. |
| [An operator key](#an-operator-key) | When the token is an operator key's. |
| [A key without the read scope](#a-key-without-the-read-scope) | When the token is a tenant key's without the `read` scope. |
| [A list the server did not give](#a-list-the-server-did-not-give) | When the server answered for the key and refused or failed the list. |
| [The detail of a vCon](#the-detail-of-a-vcon) | After Enter on a vCon in the list. |
| [The detail of the only vCon](#the-detail-of-the-only-vcon) | After Enter in the list of one vCon. |
| [The detail while the list is read again](#the-detail-while-the-list-is-read-again) | After `r` in the detail view, until the server answers. |
| [The detail of a vCon that left the list](#the-detail-of-a-vcon-that-left-the-list) | When `r` in the detail view read a list without the vCon on the screen. |
| [The detail after a list that was not read](#the-detail-after-a-list-that-was-not-read) | When `r` in the detail view failed. |
| [Help](#help) | After `?` in the list or the detail view. |
| [A terminal of 120 columns](#a-terminal-of-120-columns) | In a terminal of 114 columns or more. |
| [A terminal of 60 columns](#a-terminal-of-60-columns) | In a terminal of 59 to 79 columns. |
| [A terminal of 40 columns](#a-terminal-of-40-columns) | In a terminal of 40 to 58 columns. |
| [A terminal too small](#a-terminal-too-small) | In a terminal of fewer than 40 columns or 8 lines. |

## Is this the right tool

| To | Terminal UI | [Console](@/docs/console.md) | [HTTP API](@/docs/http-api.md) |
|---|---|---|---|
| List a tenant's vCons, newest first | yes | yes | `GET /vcons` |
| See a vCon's version and when it was stored and updated | yes | yes | `GET /vcons` |
| Read a vCon's content | no | yes, masked | `GET /vcons/{uuid}` |
| List only the vCons stored between two times | no | no | `GET /vcons` with `since` and `until` |
| Store or delete a vCon | no | yes | yes |
| Read the legal view or the audit log | no | yes | yes |
| Manage tenants and keys | no | yes | yes |

Use the terminal UI to see what a tenant holds and when it arrived, from a
shell, without a browser. It works over SSH, needs one file of settings,
and holds a key that can only read.

## Before you start

| Have | On | Notes |
|---|---|---|
| A running vcon-server that answers over HTTPS | the server | [Getting started](@/docs/getting-started.md) sets one up. |
| A tenant | the server | The command lists one tenant's vCons. Getting started makes the tenant `acme`. |
| A tenant key with the `read` scope | the server | Step 1 makes it. An operator key and a key without `read` list nothing: see [an operator key](#an-operator-key) and [a key without the read scope](#a-key-without-the-read-scope). |
| No operator configuration | the server | The command uses the tenant listener (`listen`) and the routes `GET /key` and `GET /vcons`. It needs no `[operator]` table, no operator listener and no change to the server's configuration. |
| `sudo` and `jq` | the server | The steps make a key with them. |
| The `vcon-server` binary, 0.4.2 or later | the terminal's machine | [Install](@/docs/install.md) it. This machine needs no server configuration and no data directory. |
| A memory-lock allowance for your account | the terminal's machine | Every `vcon-server` command locks its memory. Add the line `<account> - memlock unlimited` to a file in `/etc/security/limits.d/`, as [getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory) does for the `vcon` account, then log in again. |
| The server's certificate authority, as a PEM file | the terminal's machine | Getting started leaves it on the server as `/usr/local/share/vcon-server/ca.pem`. Not needed when the system already trusts the server's certificate, as with [Let's Encrypt](@/docs/lets-encrypt.md). |
| A route to the server's name and port | the terminal's machine | The command connects directly. It uses no proxy. |
| A terminal of 80 columns by 24 lines | the terminal's machine | Smaller works, down to 40 by 8, with fewer columns shown. |
| One shell for the steps | both | Later steps use variables that earlier steps set. |

The screens on this page are those of release 0.4.5. Releases 0.4.2 to
0.4.4 show another last line on some of them: the list's keys
(`j down  k up  Enter open  r refresh  n more`) on the connecting screen,
an empty tenant and every failure, and `Press r to try again, or q to
quit.` for an operator key and for a key without the `read` scope.

## Quick start on the server's own machine

These steps suit the machine [getting started](@/docs/getting-started.md)
set up: the server runs on it, the tenant is `acme`, and the terminal is a
shell on the same machine. For a terminal on another machine, follow
[the next section](#set-it-up-for-a-terminal-on-another-machine).

1. Make a key that can only read, and put its token in a file that only
   your account can read. The token goes from the command to the file. It
   is never on a command line, in a variable or on the screen:

   ```sh
   CONFIG=/etc/vcon-server/vcon-server.toml
   TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
   install -d -m 0700 ~/.config/vcon-server
   install -m 0600 /dev/null ~/.config/vcon-server/tui.token
   sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label tui-1 --config $CONFIG \
     | jq -r .token > ~/.config/vcon-server/tui.token
   stat -c '%a %s' ~/.config/vcon-server/tui.token
   ```

   The file's mode is 600 and it is not empty:

   ```text
   600 <size>
   ```

2. Write the client file. It names the server, the token file and the
   certificate authority:

   ```sh
   printf '%s\n' \
     'url = "https://vcon.example.com:8443"' \
     "key_file = \"$HOME/.config/vcon-server/tui.token\"" \
     'ca = "/usr/local/share/vcon-server/ca.pem"' \
     > ~/.config/vcon-server/tui.toml
   cat ~/.config/vcon-server/tui.toml
   ```

   ```text
   url = "https://vcon.example.com:8443"
   key_file = "<home>/.config/vcon-server/tui.token"
   ca = "/usr/local/share/vcon-server/ca.pem"
   ```

3. Start it. `--config` has no default, so name the client file each time:

   <!-- docs-test: terminal UUID -->
   ```sh
   vcon-server tui --config ~/.config/vcon-server/tui.toml
   ```

   The screen shows the tenant's vCons, with the newest one selected:

   ```text
   vcon.example.com:8443 | tenant <id> | connected
     UUID                                  STORED (UTC+00:00)   UPDATED (UTC+00:00)
   > <uuid>  <stored>  <updated>
   …
   1 of <count> read | as of <time> UTC+00:00
   j down  k up  Enter open  r refresh  ? help  q quit
   ```

   The last line names the keys that act on the screen. It has `n more`
   when the server holds more vCons than the 100 read.

4. Press `q` to leave.

## Set it up for a terminal on another machine

The key is made on the server. Everything else is on the machine with the
terminal.

1. **On the server**, make the key. The answer holds the token, and the
   server shows it this once:

   ```sh
   CONFIG=/etc/vcon-server/vcon-server.toml
   TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
   sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label tui-2 --config $CONFIG
   ```

   ```text
   {"expires_at":"<time>","id":"<key id>","token":"<token>"}
   ```

   Make one key for each person, with a label that names the person, so
   that the audit log tells them apart and one key can be revoked alone.

2. **On the terminal's machine**, check the binary and make the token file,
   empty, with a mode that keeps other users out:

   ```sh
   vcon-server --version
   install -d -m 0700 ~/.config/vcon-server
   touch ~/.config/vcon-server/tui.token
   chmod 600 ~/.config/vcon-server/tui.token
   ```

   ```text
   vcon-server <version>
   ```

   Open `~/.config/vcon-server/tui.token` in an editor and paste the token
   as its only line. Carry the token from the server over a channel you
   trust, such as the SSH session you made the key in.

3. **On the terminal's machine**, copy the server's
   `/usr/local/share/vcon-server/ca.pem` to the same path, and check that
   it holds a certificate. Skip this step when the system already trusts
   the server's certificate:

   ```sh
   openssl x509 -in /usr/local/share/vcon-server/ca.pem -noout -subject
   ```

   ```text
   subject=<name>
   ```

4. **On the terminal's machine**, check that the server's name resolves. If
   it does not, add it to DNS or to `/etc/hosts`:

   ```sh
   getent hosts vcon.example.com
   ```

   ```text
   <address> vcon.example.com
   ```

5. **On the terminal's machine**, write the client file and start the
   command, as in steps 2 and 3 of
   [the quick start](#quick-start-on-the-server-s-own-machine). Leave `ca`
   out of the file when you skipped step 3.

## Use it

### Read the screen

Every view has the same four parts.

| Part | Shows |
|---|---|
| First line | The server's address, `tenant` and the tenant's id, and the connection: `connecting`, `connected` or `not connected`. |
| Middle | The view: the list, one vCon, or help. A message takes its place when there is nothing to show. |
| Line above the keys | Which vCon is selected, how many are read, whether more are on the server, and when the list was read. A failed request shows its message here when the list stays on screen. |
| Last line | The keys that act on this screen. `?` lists every key. |

The last line names a key only where it does something:

| Screen | Last line |
|---|---|
| The list, with more on the server | `j down  k up  Enter open  r refresh  n more  ? help  q quit` |
| The list, every vCon read | `j down  k up  Enter open  r refresh  ? help  q quit` |
| The list of one vCon | `Enter open  r refresh  ? help  q quit` |
| The list while the next page is read | `j down  k up  Enter open  ? help  q quit` |
| A tenant with no vCons, or a failure that `r` can try again | `r refresh  ? help  q quit` |
| `Connecting to the server...`, `Loading vCons...`, or a key that may not list | `? help  q quit` |
| The detail view | `Esc back  j down  k up  r refresh  ? help  q quit` |
| The detail view of the only vCon | `Esc back  r refresh  ? help  q quit` |
| The detail view while the list is read again | `Esc back  ? help  q quit` |
| Help | `Esc back  q quit` |

While a message stands in place of the list, the keys that select and
open do nothing.

Every time on screen is in UTC and says so (`UTC+00:00`). There is no
setting for another time zone. Nothing depends on color: the connection is
a word, and the selected vCon has `>` in front of it.

### Find a vCon

The list is in the order the server gives it: newest first, by when each
vCon was first stored. There is no search, filter or sort. To find a vCon,
move through the list:

- `j` or Down selects the vCon below, `k` or Up the one above.
- PgDn and PgUp move one screen.
- `g` or Home selects the first vCon. `G` or End selects the last one read,
  which is the oldest on screen and not always the oldest on the server.
- `n` reads 100 older vCons when the server has more.

To find a vCon by its uuid or by a time, use `GET /vcons/{uuid}` or
`GET /vcons` with `since` and `until` ([HTTP API](@/docs/http-api.md)).

### See when a vCon was stored

The list shows the stored time and the updated time of every vCon. Press
Enter on a vCon for its version too:

<!-- docs-test: terminal UUID; keys Enter q -->
```sh
vcon-server tui --config ~/.config/vcon-server/tui.toml
```

```text
vcon.example.com:8443 | tenant <id> | connected
vCon <uuid>

Version       <version>
First stored  <stored> UTC+00:00
Last updated  <updated> UTC+00:00

Content is not shown: this view lists metadata only.
…
1 of <count> read | as of <time> UTC+00:00
Esc back  j down  k up  r refresh  ? help  q quit
```

The screen: [The detail of a vCon](#the-detail-of-a-vcon).

In this view `j` and `k` show the vCon below and above without going back
to the list. Esc goes back.

### See whether more vCons exist

The command reads 100 vCons at a time. The line above the keys says
`more on the server` when the server holds older ones than those read, as
on the screen at the top of this page (`3 of 100 read, more on the
server`). When every vCon is read, the line has no such words:

The screen: [The list with every vCon read](#the-list-with-every-vcon-read).

A tenant with no vCons shows `This tenant has no vCons.`:

The screen: [A tenant with no vCons](#a-tenant-with-no-vcons).

### Read the next page

Press `n`. The line above the keys says `Reading the next page...`, then
the count grows by up to 100 and the selection stays where it was:

The screen: [The list after the next page](#the-list-after-the-next-page).

`n` does nothing when the server has no more, and while a request is out;
the last line does not name it then.
If the next page cannot be read, the vCons already read stay on screen and
the line above the keys says why. Press `n` again, or `r`:

The screen: [A next page that was not read](#a-next-page-that-was-not-read).

### Read the list again

The list is read when the command starts, and again only when you press
`r`. Nothing refreshes it on a timer. `r` reads the newest 100 vCons and
drops the older pages that `n` added. The selection stays on the same vCon
when it is among them. While the request is out the screen says
`Loading vCons...`, or `Reading the list again...` in the detail view.

If the vCon open in the detail view is not in the new list, the view says
so:

The screen: [The detail of a vCon that left the list](#the-detail-of-a-vcon-that-left-the-list).

### Use a vCon's uuid elsewhere

Select the uuid's text with the terminal's own selection and copy it. The
mouse does nothing in the command itself. With the uuid, a key that holds
`read` reads the vCon over the [HTTP API](@/docs/http-api.md)
(`GET /vcons/{uuid}`), as [store and read a vCon](@/docs/first-vcon.md)
shows, and the [console](@/docs/console.md) opens it masked.

### Get help

Press `?` in the list or the detail view. `?` or Esc closes help:

<!-- docs-test: terminal UUID; keys ? q -->
```sh
vcon-server tui --config ~/.config/vcon-server/tui.toml
```

```text
vcon.example.com:8443 | tenant <id> | connected
Keys
  j      Select the vCon below (also Down)
  k      Select the vCon above (also Up)
  g      Select the first vCon (also Home)
  G      Select the last vCon read (also End)
  PgUp   Move up one screen
  PgDn   Move down one screen
  Enter  Open the selected vCon's metadata
  Esc    Go back to the view before (also Backspace)
  r      Read the list again from the server
  n      Read the next page of vCons
  ?      Show or close this help
  q      Quit (also Ctrl-C)

Every time shown is in UTC (UTC+00:00).
…
1 of <count> read | as of <time> UTC+00:00
Esc back  q quit
```

The screen: [Help](#help).

### Leave

Press `q` or Ctrl-C in any view. The terminal is put back as it was: the
screen you had before the command, the cursor, and the echo of what you
type. The same happens when the command ends on an error.

### Use a narrow or a wide terminal

The views follow the terminal's size when it changes. From 114 columns the
list adds each vCon's version:

The screen: [A terminal of 120 columns](#a-terminal-of-120-columns).

Below 80 columns the list drops the updated time. The first line always
ends with the connection: the address loses `https://` first, then the
tenant's id is shortened, with `...`. The line above the keys takes two
lines when it does not fit one:

The screen: [A terminal of 60 columns](#a-terminal-of-60-columns).

Below 59 columns the list drops the stored time too. A uuid is never cut.
When not even four characters of the tenant's id fit, the first line leaves
the tenant out. The last line always ends with `? help  q quit` and leaves
out other keys that do not fit. They still work:

The screen: [A terminal of 40 columns](#a-terminal-of-40-columns).

Below 40 columns or 8 lines the screen says so:

The screen: [A terminal too small](#a-terminal-too-small).

## Screens

The command draws 25 screens. Each one is shown here as the command draws
it, with what it shows, the keys that do something on it, how to leave it
and, for a failure, what to do. Every screen has the same four parts:
[read the screen](#read-the-screen) names them.

### The list

**When.** The server answered for the key and gave the first page of
vCons, and it holds more than those read. This is the screen after a
start that works.

{{ <tui_screen name="list" label="The list view in a terminal of 80 columns by 24 lines. First line: the server's address, the tenant's id and the word connected. Below it: one vCon on each line with its uuid, stored time and updated time, the third one selected. Last two lines: 3 of 100 read, more on the server, the time the list was read, and the keys." /> }}

**Shows.** First line: the server's address, `tenant` and the tenant's id,
and `connected`. Then one vCon on each line, newest first: its uuid
(`UUID`), when the server first stored it (`STORED`) and when it last
changed (`UPDATED`), in UTC. `>` and reversed colors mark the selected
vCon. The line above the keys says which vCon is selected, how many are
read, `more on the server`, and when the list was read. The last line
names the keys.

{{ <tui_keys name="list" /> }}

**Leave.** Enter opens [the detail of a vCon](#the-detail-of-a-vcon), `?`
opens [help](#help), `n` reads the next page and `q` ends the command.

### The list with every vCon read

**When.** The server gave a page and holds no older vCon: a tenant with
100 vCons or fewer, or the last page that `n` read.

{{ <tui_screen name="list-end" label="The list of a tenant with three vCons, all read. The line above the keys says 1 of 3 read and the time the list was read, with no mention of more on the server." /> }}

**Shows.** The same as [the list](#the-list). The line above the keys has
no `more on the server`, and the last line has no `n more`.

{{ <tui_keys name="list-end" /> }}

**Leave.** Enter opens the detail of the selected vCon, `?` opens help and
`q` ends the command. `n` does nothing here.

### The list of one vCon

**When.** The tenant holds one vCon.

{{ <tui_screen name="list-one" label="The list of a tenant with one vCon: the vCon is selected, the line above the keys says 1 of 1 read, and the last line says Enter open, r refresh, ? help, q quit." /> }}

**Shows.** One line with the vCon, selected, and `1 of 1 read`. The last
line names no key that moves: there is no other vCon to select.

{{ <tui_keys name="list-one" /> }}

**Leave.** Enter opens [the detail of the only
vCon](#the-detail-of-the-only-vcon), `?` opens help and `q` ends the
command.

### The list while the next page is read

**When.** You pressed `n` on [the list](#the-list) and the server has not
answered yet.

{{ <tui_screen name="more-reading" label="The list while the next page is read: the vCons read so far, Reading the next page... on the line above the keys, and a last line without r refresh and n more." /> }}

**Shows.** The vCons read so far, and `Reading the next page...` on the
line above the keys. The last line names neither `r` nor `n`: a second
request would race the one that is out, so both keys wait.

{{ <tui_keys name="more-reading" /> }}

**Leave.** The screen changes by itself when the server answers: to [the
list after the next page](#the-list-after-the-next-page), or to [a next
page that was not read](#a-next-page-that-was-not-read) after 15 seconds
with no answer. `q` ends the command at once.

### The list after the next page

**When.** The page that `n` asked for arrived.

{{ <tui_screen name="more-read" label="The list after n read the second page: the third vCon is still selected and the line above the keys says 3 of 200 read, with no mention of more on the server." /> }}

**Shows.** The list with the new vCons added below the ones read before.
The selection has not moved. The line above the keys counts all the vCons
read, here `3 of 200 read`, and says `more on the server` only when the
server holds still older ones.

{{ <tui_keys name="more-read" /> }}

**Leave.** As from [the list](#the-list). `r` reads the newest 100 vCons
again and drops the pages that `n` added.

### A next page that was not read

**When.** You pressed `n` and the request failed: the server did not
answer, or it answered with a refusal.

{{ <tui_screen name="next-page-failed" label="The list after a next page that got no answer. The first line says not connected. The vCons read before are still listed. Above the keys: The next page was not read: the server did not answer: no answer within 15 seconds." /> }}

**Shows.** The vCons read before stay on the screen. The line above the
keys says `The next page was not read:` and why. When the server did not
answer, or refused the key, the first line says `not connected` and
`tenant unknown`.

{{ <tui_keys name="next-page-failed" /> }}

**Leave.** Any key removes the message. `n` asks for the page again, `r`
reads the list from the start, and `q` ends the command.

**What to do.** Press `n` to try the page again. If the message says the
server did not answer, check that the server runs and that this machine
reaches it, then press `r`. If it says the server refused the key (401),
see [a key the server refused](#a-key-the-server-refused).

### A tenant with no vCons

**When.** The server answered and the tenant holds no vCon.

{{ <tui_screen name="empty" label="The list view of a tenant with no vCons: the first line says connected, the middle says This tenant has no vCons. and Press r to read again, or q to quit., the line above the keys has the time the list was read, and the last line says r refresh, ? help, q quit." /> }}

**Shows.** `This tenant has no vCons.` and `Press r to read again, or q to
quit.` The first line says `connected`, and the line above the keys says
when the list was read. Nothing failed.

{{ <tui_keys name="empty" /> }}

**Leave.** `r` reads the list again, for a vCon stored since. `q` ends the
command.

### Connecting

**When.** The command has started, or you pressed `r` after the server
did not answer, and the server has not answered for the key yet.

{{ <tui_screen name="connecting" label="The connecting screen: the first line ends with tenant unknown and connecting, the middle says Connecting to the server... and Press q to quit., and the last line says ? help and q quit." /> }}

**Shows.** `Connecting to the server...` and `Press q to quit.` The first
line holds the address as the client file has it, `tenant unknown` and
`connecting`.

{{ <tui_keys name="connecting" /> }}

**Leave.** The screen changes by itself: to [loading](#loading) when the
server answers for the key, or to a failure screen after 15 seconds with
no answer. `q` ends the command at once.

### Loading

**When.** The server answered for the key and the first page of vCons is
on its way: at the start, and after `r` in the list.

{{ <tui_screen name="loading" label="The loading screen: the first line names the tenant and says connected, the middle says Loading vCons... and Press q to quit., and the last line says ? help and q quit." /> }}

**Shows.** `Loading vCons...` and `Press q to quit.` The first line
already says `connected` and names the tenant.

{{ <tui_keys name="loading" /> }}

**Leave.** The screen changes by itself to the list, to [a tenant with no
vCons](#a-tenant-with-no-vcons) or to [a list the server did not
give](#a-list-the-server-did-not-give). `q` ends the command at once.

### A server that did not answer

**When.** No answer came for the key: the name does not resolve, nothing
listens on the port, the connection broke, or 15 seconds passed.

{{ <tui_screen name="not-connected" label="A server that did not answer: the first line ends with tenant unknown and not connected, and the middle says Could not list vCons: the server did not answer: no answer within 15 seconds, then the two lines about r, q and the next start." /> }}

**Shows.** `Could not list vCons:`, then `the server did not answer:` and
the reason. The first line says `tenant unknown` and `not connected`. The
last two lines of the message say that `r` tries again and that a changed
client file or key file is read at the next start.

{{ <tui_keys name="not-connected" /> }}

**Leave.** `r` asks the server again and shows
[connecting](#connecting). `q` ends the command.

**What to do.** Check the port in `url`, the route to the server and that
the server runs, then press `r`. [The first line says `not
connected`](#the-first-line-says-not-connected) lists each reason.

### A key the server refused

**When.** The server answered 401 for the key: it does not know the
token, the key is revoked or the key has expired. A key that the server
stops accepting while the command runs gives this screen at the next `r`.

{{ <tui_screen name="refused-key" label="A key the server refused: the first line ends with not connected, and the middle says Could not list vCons: the server refused the key (401). and Check that key_file holds a key the server has not revoked." /> }}

**Shows.** `Could not list vCons: the server refused the key (401).` and
`Check that key_file holds a key the server has not revoked.` The first
line says `not connected`.

{{ <tui_keys name="refused-key" /> }}

**Leave.** `r` asks again with the same token, which the server refuses
again unless the key was made valid on the server. `q` ends the command.

**What to do.** Put the token of a current tenant key with the `read`
scope in `key_file`, then press `q` and start the command again: the file
is read once, at start.

### A certificate that was refused

**When.** The server's certificate did not pass the check: no authority
this machine trusts signed it, it is not for the name in `url`, it has
expired, or it is not valid yet.

{{ <tui_screen name="refused-certificate" label="A certificate that no trusted authority signed: Could not list vCons: the server's certificate was refused: no certificate this client trusts signed it; set ca in the client file to the server's current certificate authority." /> }}

**Shows.** `Could not list vCons: the server's certificate was refused:`,
then which check failed and what to set. The first line says
`not connected`.

{{ <tui_keys name="refused-certificate" /> }}

**Leave.** `r` connects again, which helps after the clock or the
server's certificate was corrected. `q` ends the command.

**What to do.** Follow the message. [The server's certificate was
refused](#the-server-s-certificate-was-refused) has a line for each
check. A changed `ca` is read at the next start.

### An operator key

**When.** The token in `key_file` is an operator key's. Operator keys
manage tenants and read no vCon.

{{ <tui_screen name="operator-key" label="An operator key: the first line says no tenant (operator key) and connected, and the middle says Could not list vCons: this is an operator key, which reaches no vCon: use a tenant key with the read scope. Then Press q to quit., and the last line says ? help and q quit." /> }}

**Shows.** `Could not list vCons: this is an operator key, which reaches
no vCon: use a tenant key with the read scope.`, then `Press q to quit.`
The first line says `no tenant (operator key)` and `connected`: the
server did answer.

{{ <tui_keys name="operator-key" /> }}

**Leave.** `q` ends the command. `r` does nothing here: the answer
depends on the key alone, and the key file is read once, at start.

**What to do.** Make a tenant key with `--scope read`
([step 1 of the quick start](#quick-start-on-the-server-s-own-machine)),
put its token in `key_file` and start the command again.

### A key without the read scope

**When.** The token is a tenant key's and the key does not hold the
`read` scope, for example a producer's key with `write` alone.

{{ <tui_screen name="no-read-scope" label="A tenant key without the read scope: the first line names the tenant and says connected, and the middle says Could not list vCons: this key lacks the read scope: use a tenant key that holds it. Then Press q to quit., and the last line says ? help and q quit." /> }}

**Shows.** `Could not list vCons: this key lacks the read scope: use a
tenant key that holds it.`, then `Press q to quit.` The first line names
the tenant and says `connected`.

{{ <tui_keys name="no-read-scope" /> }}

**Leave.** `q` ends the command. `r` does nothing here, as for [an
operator key](#an-operator-key).

**What to do.** Make a key for the same tenant with `--scope read`, put
its token in `key_file` and start the command again. A key's scopes
cannot be changed after it is made.

### A list the server did not give

**When.** The server answered for the key and then refused the list or
failed to give it: for example 503 while it cannot read its key table.

{{ <tui_screen name="server-error" label="A list the server did not give: the first line says connected, and the middle says Could not list vCons: the server answered 503 (The service cannot answer now). The last line says r refresh, ? help, q quit." /> }}

**Shows.** `Could not list vCons:`, then `the server answered` with the
status and, in parentheses, the title of the server's problem document,
here `the server answered 503 (The service cannot answer now)`. The first
line still says `connected`.

{{ <tui_keys name="server-error" /> }}

**Leave.** `r` asks for the first page again. `q` ends the command.

**What to do.** Press `r` after a moment. [Other messages on the
screen](#other-messages-on-the-screen) and the server's
[problem pages](@/problems/_index.md) say what each status means.

### The detail of a vCon

**When.** You pressed Enter on a vCon in the list.

{{ <tui_screen name="detail" label="The detail view of one vCon: the line vCon and its uuid, then Version, First stored and Last updated, each with its value, then the sentence Content is not shown: this view lists metadata only. The keys on the last line start with Esc back." /> }}

**Shows.** `vCon` and the uuid, then `Version` (the vCon's version on the
server), `First stored` and `Last updated`, each in UTC, then `Content is
not shown: this view lists metadata only.` The line above the keys is the
list's.

{{ <tui_keys name="detail" /> }}

**Leave.** Esc or Backspace goes back to the list, with the same vCon
selected. `j` and `k` show the vCon below and the one above without
leaving the view.

### The detail of the only vCon

**When.** You pressed Enter in [the list of one
vCon](#the-list-of-one-vcon).

{{ <tui_screen name="detail-one" label="The detail view of a tenant's only vCon: its uuid, version and two times, 1 of 1 read on the line above the keys, and a last line that says Esc back, r refresh, ? help, q quit." /> }}

**Shows.** The same lines as [the detail of a
vCon](#the-detail-of-a-vcon). The last line names no key that moves:
there is no other vCon to show.

{{ <tui_keys name="detail-one" /> }}

**Leave.** Esc or Backspace goes back to the list. `q` ends the command.

### The detail while the list is read again

**When.** You pressed `r` in the detail view and the server has not
answered yet.

{{ <tui_screen name="detail-refreshing" label="The detail view while the list is read again: Reading the list again... and Press q to quit. in place of the vCon, and a last line that says Esc back, ? help, q quit." /> }}

**Shows.** `Reading the list again...` and `Press q to quit.` in place of
the vCon's lines.

{{ <tui_keys name="detail-refreshing" /> }}

**Leave.** The screen changes by itself when the server answers: to the
detail of the same vCon, to [the detail of a vCon that left the
list](#the-detail-of-a-vcon-that-left-the-list), or to [the detail after
a list that was not read](#the-detail-after-a-list-that-was-not-read).
Esc goes back to the list view, which shows [loading](#loading).

### The detail of a vCon that left the list

**When.** `r` in the detail view read a list that does not hold the vCon
on the screen: the vCon was deleted, or 100 newer ones were stored since.

{{ <tui_screen name="detail-gone" label="The detail view after r, for a vCon that the new list does not hold: This vCon is no longer in the list: it was deleted, or it is past the pages read. Press Esc to go back." /> }}

**Shows.** `This vCon is no longer in the list: it was deleted, or it is
past the pages read.` and `Press Esc to go back.` The line above the keys
describes the new list.

{{ <tui_keys name="detail-gone" /> }}

**Leave.** Esc goes back to the list. `j` or `k` shows a vCon that is in
the list.

**What to do.** To see whether the vCon still exists, read it by its uuid
over the [HTTP API](@/docs/http-api.md) (`GET /vcons/{uuid}`), or press
Esc and `n` to read older pages.

### The detail after a list that was not read

**When.** `r` in the detail view failed: the server did not answer or
refused the request.

{{ <tui_screen name="detail-failed" label="The detail view after a refresh the server answered with 503: Could not list vCons: the server answered 503 (The service cannot answer now). The last line says Esc back, r refresh, ? help, q quit." /> }}

**Shows.** The failure in place of the vCon's lines, as in the list view:
`Could not list vCons:` and why, here `the server answered 503 (The
service cannot answer now)`, then the two lines about `r` and the next
start.

{{ <tui_keys name="detail-failed" /> }}

**Leave.** `r` asks again. Esc goes back to the list view, which shows
the same failure. `q` ends the command.

**What to do.** The same as for the failure in the list view: find the
message under [when it does not work](#when-it-does-not-work), then press
`r`.

### Help

**When.** You pressed `?` in the list or the detail view, on any screen
of theirs.

{{ <tui_screen name="help" label="The help view: the heading Keys, then twelve lines, each a key and what it does, from j Select the vCon below to q Quit, then the sentence Every time shown is in UTC. The last line says Esc back and q quit." /> }}

**Shows.** The heading `Keys`, then every key with what it does, also
the keys that do nothing on the screen help was opened from, then `Every
time shown is in UTC (UTC+00:00).` The first line and the line above the
keys stay as they were.

{{ <tui_keys name="help" /> }}

**Leave.** `?`, Esc or Backspace closes help and shows the screen it was
opened from. `q` ends the command.

### A terminal of 120 columns

**When.** The terminal is 114 columns wide or wider.

{{ <tui_screen name="wide" label="The list in a terminal of 120 columns by 10 lines: a fourth column, VERSION, follows the updated time, and the first line holds the server's address with https:// in front." /> }}

**Shows.** [The list](#the-list) with a fourth column, `VERSION`, and the
server's address with `https://` in front when it fits. This screen is
drawn in 120 columns.

{{ <tui_keys name="wide" /> }}

**Leave.** As from the list. Making the terminal narrower removes the
column again.

### A terminal of 60 columns

**When.** The terminal is narrower than 80 columns and at least 59.

{{ <tui_screen name="narrow" label="The list in a terminal of 60 columns by 10 lines: each vCon has its uuid and stored time, the tenant's id on the first line ends in three dots, and the line above the keys is on two lines, 3 of 100 read, more on the server, then the time the list was read." /> }}

**Shows.** The list without the `UPDATED` column. The tenant's id on the
first line is shortened and ends in three dots, and the line above the
keys takes two lines. This screen is drawn in 60 columns.

{{ <tui_keys name="narrow" /> }}

**Leave.** As from the list. Every key works as in a terminal of 80
columns.

### A terminal of 40 columns

**When.** The terminal is narrower than 59 columns and at least 40.

{{ <tui_screen name="narrowest" label="The list in a terminal of 40 columns by 10 lines: the first line holds the server's address and connected, each vCon is its uuid alone, and the last line says j down, k up, Enter open, ? help, q quit." /> }}

**Shows.** Each vCon as its uuid alone. The first line leaves the tenant
out, and the last line keeps `? help  q quit` and leaves out the keys
that do not fit. This screen is drawn in 40 columns.

{{ <tui_keys name="narrowest" /> }}

**Leave.** As from the list. The keys the last line leaves out still
work.

### A terminal too small

**When.** The terminal has fewer than 40 columns or fewer than 8 lines.

{{ <tui_screen name="too-small" label="A terminal of 36 columns by 6 lines, too small for a view: Terminal too small: 40x8 is needed. Enlarge it, or press q to quit." /> }}

**Shows.** `Terminal too small: 40x8 is needed. Enlarge it, or press q to
quit.` and nothing else. This screen is drawn in 36 columns by 6 lines.

{{ <tui_keys name="too-small" /> }}

**Leave.** Enlarge the terminal and the view comes back at once. `q`
ends the command.

**What to do.** Make the terminal at least 40 columns by 8 lines. The
keys still act on the view while it is not drawn, so press nothing but
`q` until it is back.

## Check that it works

On the screen:

- The first line ends with `connected` and names the tenant you expect.
- The list holds the tenant's vCons, or `This tenant has no vCons.`
- The line above the keys ends with the time the list was read.

On the server, the tenant's [audit log](@/docs/audit-logs.md) has a
`key_info` record for each start and a `list` record for each page read,
under the key's id:

```sh
KEY_ID=$(sudo -u vcon vcon-server key list --tenant "$TENANT" --config $CONFIG | jq -r 'select(.label == "tui-1") | .id')
sudo -u vcon vcon-server audit export --tenant "$TENANT" --config $CONFIG \
  | jq -r --arg key "$KEY_ID" 'select(.actor == $key) | .action' | sort -u
```

```text
key_info
list
```

## Reference

### The command

```text
vcon-server tui --config <client file>
```

`--config` is required and names the [client file](#the-client-file). The
command reads the client file, the token and the certificate authority
once, at start. A change to any of them takes effect at the next start.

| Exit code | When |
|---|---|
| 0 | You quit with `q` or Ctrl-C. |
| 1 | The command could not start, or the terminal failed. One line on standard error says why; see [the messages](#when-it-does-not-work). |

### The client file

A TOML file with these keys and no others.

| Key | Required | Default | Meaning | Example |
|---|---|---|---|---|
| `url` | yes | none | The server's address: `https://`, the name and the port, with no path. A path, such as the `/mcp` of an MCP client file, goes in front of every request, and the server answers 404. | `"https://vcon.example.com:8443"` |
| `key_file` | yes | none | The file that holds the token and nothing else, by its full path. `~` is not expanded. Its mode must keep other users out: 600, or 640 for a group that only reads it. | `"/home/you/.config/vcon-server/tui.token"` |
| `ca` | no | the system's trusted roots | A PEM file of the certificates to trust for the server, in place of the system's. | `"/usr/local/share/vcon-server/ca.pem"` |

### Keys

| Key | Does | List | Detail | Help |
|---|---|---|---|---|
| `j`, Down | Select the vCon below | yes | yes | no |
| `k`, Up | Select the vCon above | yes | yes | no |
| `g`, Home | Select the first vCon | yes | yes | no |
| `G`, End | Select the last vCon read | yes | yes | no |
| PgUp | Move up one screen | yes | yes | no |
| PgDn | Move down one screen | yes | yes | no |
| Enter | Open the selected vCon's metadata | yes | no | no |
| Esc, Backspace | Go back to the view before | no | yes | yes |
| `r` | Read the list again from the server | yes | yes | no |
| `n` | Read the next page of vCons | yes | yes | no |
| `?` | Show or close this help | yes | yes | yes |
| `q`, Ctrl-C | Quit | yes | yes | yes |

In the detail view, a key that selects a vCon shows that vCon. Any other
key does nothing. A key held down repeats.

### Environment

| Variable | Effect |
|---|---|
| `NO_COLOR` | Set to anything but the empty string, it turns color off. |
| `TERM` | `dumb`, or not set, turns color off. |

With color, the connection is green, yellow or red, and bold. Without it,
the screen uses bold and reverse video only. The command reads no other
variable: `HTTPS_PROXY` and the like do not apply, and the token and the
address come from the client file alone.

### Sizes and limits

| What | Value |
|---|---|
| vCons read at a time | 100 |
| Time allowed for one request | 15 seconds |
| Smallest terminal | 40 columns by 8 lines |
| Stored time shown from | 59 columns |
| Updated time shown from | 80 columns |
| Version shown from | 114 columns |

## When it does not work

A command that cannot start writes one line to standard error, after
`vcon-server: `, and exits with code 1. Once the screen is up, a failure is
a message on the screen. In the tables, `<path>` and the like stand for
text that differs from one machine to the next.

A failure on the screen ends with these two lines. `r` asks the server
again. It does not read the client file, the token or the certificate
authority again:

```text
Press r to try again, or q to quit.
A change to the client file or the key file takes effect at the next start.
```

For a key that may not list vCons (an operator key, or a key without the
`read` scope) the first of the two lines is `Press q to quit.`: the answer
depends on the key alone, so `r` does nothing and the last line does not
name it.

### The command ends at once: the client file

| Message | Cause | Do |
|---|---|---|
| `client file: <path>: No such file or directory (os error 2)` | `--config` names a file that does not exist. | Name the client file by its path. |
| `client file: TOML parse error at line <line>, column <column>` | The file is not TOML, lacks `url` or `key_file`, or has a key that is not in [the table](#the-client-file). The lines after the message show the place. | Correct the file. |
| `client file: url: <url> is not an https:// URL` | `url` starts with `http://` or with nothing. | Start `url` with `https://`. The server speaks TLS only. |

### The command ends at once: the token file

| Message | Cause | Do |
|---|---|---|
| `key_file <path>: No such file or directory (os error 2)` | `key_file` names a file that does not exist. A `~` or a relative path counts from the directory you are in. | Give `key_file` the full path. |
| `key_file <path>: other users may read or change the file (mode <mode>)` | The file's mode lets other users read or write it. | Run the `chmod 600` that the message gives. |
| `key_file <path>: the file is owned by uid <uid>, not by the user this runs as` | Another account owns the file. | Run the `chown` that the message gives, or make the file as your own account. |
| `key_file <path>: not a regular file` | `key_file` names something that is not a file, such as a directory. | Name the file itself. |
| `key_file <path>: holds no single token` | The file is empty, or holds more than the token: a second word, or the whole answer of `key create`. | Leave the token as the file's only line. |
| `key_file <path>: the file is not UTF-8` | The file is not text. | Write the token again. |

### The command ends at once: the certificate authority

| Message | Cause | Do |
|---|---|---|
| `ca: <path>: No such file or directory (os error 2)` | `ca` names a file that does not exist. | Copy the server's `ca.pem` to that path, or correct `ca`. |
| `ca: <path>: holds no certificate` | The file is not PEM, or is empty. | Copy the file again. It starts with `-----BEGIN CERTIFICATE-----`. |

### The command ends at once: the terminal and the system

| Message | Cause | Do |
|---|---|---|
| `a terminal is required: <what>; run it in a terminal, not through a pipe, a redirect or a job without one` | Standard input or standard output is a pipe, a file or nothing. `<what>` says which. | Run the command in a terminal. Over SSH, use `ssh -t` when you give the command on the `ssh` line. |
| `memory cannot be locked: <reason>; allow it, for example LimitMEMLOCK=infinity` | Your account has no memory-lock allowance. | Add the allowance in [before you start](#before-you-start), then log in again. |
| `core dumps cannot be turned off: <reason>` | The system refused to set the core-dump limit to zero (`setrlimit`), or to mark the process as not dumpable (`prctl`). The reason is the system's error. | Run the command where the system allows both calls. |
| `this build has no terminal UI: build vcon-server with the cargo feature tui` | The binary was built with `--no-default-features`. | Use a release binary, or build with the default features. |
| `the terminal: <reason>` | The terminal could not be set up, drawn on or read. | Check `TERM`, and that the terminal is still there. |

### The first line says `not connected`

The screen: [A server that did not answer](#a-server-that-did-not-answer).

| Message | Cause | Do |
|---|---|---|
| `the server did not answer: no answer within 15 seconds` | The address is reachable and nothing answers in time: a firewall that drops packets, or a server under load. | Check the port in `url` and the route to the server, then press `r`. |
| `the server did not answer: <reason>` | The name does not resolve, nothing listens on the port, or the connection broke. The reason says which. | Check `url`, the name and that the server runs, then press `r`. |
| `the server refused the key (401)` | The server does not know the token, or the key is revoked. | Put a current token in `key_file` and start the command again. |
| `the server answered 404` | `url` holds a path, or the address is another service's. | Give `url` the server's address with no path and start again. |
| `the server answered 503 (The service cannot answer now)` | The server cannot read its key table now, or the tenant is being removed. | Press `r` after a moment. See [unavailable](@/problems/unavailable.md). |
| `the server answered 503 (The audit record could not be written)` | The server could not record the request, so it did not answer it. | See [audit-failed](@/problems/audit-failed.md), then press `r`. |
| `the server's answer could not be read: <reason>` | The address answers, and not as a vcon-server does. | Check that `url` names the vcon-server and not a proxy's own page. |

A key that the server stops accepting while the command runs gives the same
screen at the next `r` or `n`:

The screen: [A key the server refused](#a-key-the-server-refused).

### The server's certificate was refused

The first line says `not connected` and the message starts with
`the server's certificate was refused`. The command always checks the
certificate. No setting turns that off.

The screen: [A certificate that was refused](#a-certificate-that-was-refused).

| Message | Cause | Do |
|---|---|---|
| `the server's certificate was refused: no certificate this client trusts signed it` | `ca` is missing, names another certificate authority, or holds a certificate the server has since replaced. | Set `ca` to the server's current certificate authority and start again. |
| `the server's certificate was refused: it is not for the name in url` | `url` names the server by a name or an address that its certificate does not hold. | Use a name the certificate holds and start again. |
| `the server's certificate was refused: it is marked as a certificate authority (CA:TRUE)` | The server presents a certificate made as a certificate authority's, which `openssl req -x509` makes by default. | On the server, make the certificate with `basicConstraints=CA:FALSE`, as in [getting started](@/docs/getting-started.md#get-a-certificate), restart the server, then press `r`. |
| `the server's certificate was refused: it has expired` | The certificate's last day has passed, or this machine's clock is ahead. | Renew the certificate on the server, or correct the clock, then press `r`. |
| `the server's certificate was refused: it is not valid yet` | This machine's clock is behind, or the certificate starts later. | Correct the clock, then press `r`. |

### The first line says `connected` and there is no list

The screen: [An operator key](#an-operator-key).

| Message | Cause | Do |
|---|---|---|
| `this is an operator key, which reaches no vCon: use a tenant key with the read scope` | The token is an operator key's. Operator keys manage tenants and read no vCon. | Make a tenant key with `--scope read`, put its token in `key_file` and start again. |
| `this key lacks the read scope: use a tenant key that holds it` | The key holds other scopes only, such as `write`. | The same. A key's scopes do not change after it is made. |
| `the server answered <status> (<title>)` | The server refused the list. The title is the [problem's](@/problems/_index.md). | Follow the problem's page, then press `r`. |

### Other messages on the screen

| Message | Cause | Do |
|---|---|---|
| `The next page was not read: <reason>` | The request `n` made failed. The reason is one of the messages above. | Press `n` again, or `r` to read the list from the start. |
| `This vCon is no longer in the list: it was deleted, or it is past the pages read.` | After `r`, the vCon in the detail view is not among the newest 100. | Press Esc. Press `n` in the list to read older vCons. |
| `Terminal too small: 40x8 is needed. Enlarge it, or press q to quit.` | The terminal has fewer than 40 columns or 8 lines. | Enlarge the terminal. |
| `Connecting to the server...` | The first request is out. It ends within 15 seconds. | Wait, or press `q`. |

## Security

| Question | Answer |
|---|---|
| Where is the token? | In the file `key_file` names, and in the command's locked memory while it runs. The command refuses a token file that other users can read. It never draws the token, and it takes no token from a command line or the environment. |
| Is the server's certificate checked? | Always, against `ca`, or against the system's trusted roots without it. No setting turns the check off. |
| What content reaches the terminal? | None. The command asks the server for the key's own description (`GET /key`) and the list (`GET /vcons`): uuids, versions and times. It never asks for a vCon, so no party, dialog or attachment is on the screen, in the terminal's scrollback or in this machine's memory. |
| What does the audit log record? | A `key_info` record at each start and a `list` record for each page, under the key's id, as for any client ([audit logs](@/docs/audit-logs.md)). The server's scopes and tenant bounds apply: a key sees its own tenant only. |
| What can the key do if it leaks? | Read the tenant's vCons over the API until it is revoked. Make it with `read` alone, and revoke it as below. |

## Remove access

On the server, revoke the key. The server refuses it from its next request:

```sh
sudo -u vcon vcon-server key revoke "$KEY_ID" --config $CONFIG
```

A command that is running shows the refusal at its next `r` or `n`, and a
new start shows it at once:

<!-- docs-test: terminal revoked -->
```sh
vcon-server tui --config ~/.config/vcon-server/tui.toml
```

```text
https://vcon.example.com:8443 | tenant unknown | not connected
Could not list vCons: the server refused the key (401).

Check that key_file holds a key the server has not revoked.
Press r to try again, or q to quit.
A change to the client file or the key file takes effect at the next start.
…
r refresh  ? help  q quit
```

On the terminal's machine, remove the token file:

```sh
rm ~/.config/vcon-server/tui.token
```

[Revoke and rotate keys](@/docs/rotate-keys.md) shows how to replace a key
without a gap.

## Next

- [Keys and scopes](@/docs/keys-and-scopes.md): what each scope allows.
- [Console](@/docs/console.md): read a vCon's content, masked, in a browser.
- [Read the audit logs](@/docs/read-audit-logs.md): see what a key did.
