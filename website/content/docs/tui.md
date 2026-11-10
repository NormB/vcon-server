+++
title = "Terminal UI"
description = "Browse a tenant's vCons from a terminal with vcon-server tui: the build, the key and client file it needs, the keys, and what each message means."
weight = 73
+++

`vcon-server tui` shows a tenant's vCons in a terminal. It is a client of a
running server's [HTTP API](@/docs/http-api.md) with one API key, like any
other client: it opens no data directory, catalog or master key, so it runs
on any machine that reaches the server, and the server's scopes, tenant
bounds and [audit log](@/docs/audit-logs.md) apply to everything it shows.

This release reads only. It has three views:

- **The list:** the key's tenant's vCons, newest first, with each uuid and
  the times it was first stored and last updated.
- **The detail:** one vCon's uuid, version and times.
- **Help:** every key.

It shows what `GET /vcons` lists and nothing else. It never asks the server
for a vCon's content, so no party, dialog or attachment reaches the
terminal.

## Before you start

- **A build with the terminal UI.** It is behind the cargo feature `tui`,
  which is off by default, so the server's own binary does not carry it.
  From a checkout ([build from source](@/docs/install.md#build-from-source)):

  ```sh
  cargo build --release --locked -p vcon-server --features tui
  ```

  A build without the feature has the command, and it answers `this build
  has no terminal UI: build vcon-server with the cargo feature tui`.
- **On the server: a tenant key with the `read` scope.** Make one for each
  person, with a label that names them
  ([keys and scopes](@/docs/keys-and-scopes.md)). An operator key reaches no
  vCon and lists nothing.

  ```sh
  sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label tui-1 \
    --config /etc/vcon-server/vcon-server.toml | jq -r .token
  ```
- **The server's address**, reachable over HTTPS from the machine with the
  terminal. The command connects directly, through no proxy.
- **The server's certificate authority**, if the system does not already
  trust it: the certificate's copy for clients, such as
  `/usr/local/share/vcon-server/ca.pem` from
  [getting started](@/docs/getting-started.md). The certificate is always
  verified; there is no setting that turns that off.
- **A memory-lock allowance** for the account that runs it, as for every
  command ([getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory)).

## Steps

1. Put the token in a file only you can read. The token is never on the
   command line or in the environment:

   ```sh
   mkdir -p ~/.config/vcon-server
   install -m 0600 /dev/null ~/.config/vcon-server/tui.token
   $EDITOR ~/.config/vcon-server/tui.token
   ```

2. Write the client file, `~/.config/vcon-server/tui.toml`:

   ```toml
   url = "https://vcon.example.com:8443"
   key_file = "/home/reader/.config/vcon-server/tui.token"
   ca = "/usr/local/share/vcon-server/ca.pem"
   ```

   - `url`: the server's address. It must start with `https://`.
   - `key_file`: the file holding the token and nothing else.
   - `ca`: optional. PEM certificates to trust for the server; without it,
     the system's trusted roots are used.

3. Start it:

   ```sh
   vcon-server tui --config ~/.config/vcon-server/tui.toml
   ```

## Keys

The last line of every view shows its keys, and `?` lists them all.

| Key | Does |
|---|---|
| `j`, Down | Select the vCon below |
| `k`, Up | Select the vCon above |
| `g`, Home | Select the first vCon |
| `G`, End | Select the last vCon read |
| PgUp, PgDn | Move one screen |
| Enter | Open the selected vCon's metadata |
| Esc, Backspace | Go back to the view before |
| `r` | Read the list again from the server |
| `n` | Read the next page of vCons (100 a page) |
| `?` | Show or close help |
| `q`, Ctrl-C | Quit |

## Check that it works

- The first line names the server, `tenant` and the tenant's id, and
  `connected`.
- The list shows the tenant's vCons, or `This tenant has no vCons.`
- The line above the keys says how many vCons are read, whether more are on
  the server, and when the list was read. Every time on screen is in UTC,
  and says so (`UTC+00:00`).
- On the server, the tenant's audit log has a `key_info` record and a `list`
  record for the key ([audit logs](@/docs/audit-logs.md)).

The terminal is put back as it was when the command ends, whether by `q`, by
an error or by a failure of the program itself.

## Display

- Color is never the only sign of anything: the connection is a word and the
  selected row has a `>` in front of it.
- With `NO_COLOR` set, `TERM=dumb` or no `TERM`, no color is used at all.
- The views are made for 80 columns by 24 lines and follow a resize. Below
  80 columns the list drops the update time; from 114 it adds the version.
  Below 40 by 8 the screen says the terminal is too small.

## When it does not work

| Message | Cause | Do |
|---|---|---|
| `client file:` and a reason, at start | The client file is missing, is not TOML, has a key it does not take, or its `url` is not `https://` | Correct the file |
| `key_file` and a reason, at start | The token file is missing, can be read by other users, or holds more than one token | `chmod 600` the file; keep only the token in it |
| `ca:` and a reason, at start | The CA file is missing or holds no certificate | Name the certificate's PEM copy |
| `memory cannot be locked`, at start | The account has no memory-lock allowance | Allow it, as in getting started |
| `this build has no terminal UI` | The binary was built without the `tui` feature | Build with `--features tui` |
| `not connected`, with `the server did not answer` | The address, the network, or a certificate the client does not trust | Check `url` and `ca`, then press `r` |
| `the server refused the key (401)` | The token is not a key the server knows, or the key is revoked | Put a current token in `key_file`, then start again |
| `this is an operator key` | Operator keys reach no vCon | Use a tenant key with `read` |
| `this key lacks the read scope` | The key holds other scopes only | Use a key that holds `read` |
| `the server answered 503 (unavailable)` | The tenant is being removed, or its log cannot be written | See [operations](@/docs/operations.md), then press `r` |
| `Terminal too small` | Fewer than 40 columns or 8 lines | Enlarge the terminal |

## What it does not do yet

It does not show a vCon's content, search, store, delete, show the legal
views or the audit logs, or manage tenants and keys. The
[console](@/docs/console.md) and the [HTTP API](@/docs/http-api.md) do.
