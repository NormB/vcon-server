+++
title = "Set a listener's public origin for sessions"
description = "Tell the server the origin browsers reach each listener at, so it can serve sessions: the configuration key, the restart, the checks a request by session passes, the refusals, and what to change in proxies and DNS."
weight = 39

[extra]
group = "how-to"
+++

A session is how a person's browser will be known to the server once
sign-in exists: a cookie the page's script cannot read, in place of a key
pasted into a page. The server refuses a request by session unless it
comes from a page of the server's own. To tell, it compares the request
with the listener's public origin, the scheme, host and port in the
browser's address bar. The server cannot find that origin by itself, since
a proxy or a port mapping may stand between the browser and the address in
`listen`. This how-to sets the origin and checks each refusal.

This is the fourth part of sign-in. The server now keeps sessions, serves
`GET /session` and `POST /session/logout`, and ends a user's sessions when
its roles change. No sign-in method exists yet, so no browser holds a
session: the checks below show the refusals, which are what a server
without a signed-in user can show. Passkeys and single sign-on, which a
later release adds, will start sessions. Set the origin now to have the
listener ready.

The keys and the routes are on `main` only; release 0.4.3 does not have
them. CI runs every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## What a request by session must carry

| Request | Cookie `__Host-vcs_session` | Where it comes from | CSRF token |
|---|---|---|---|
| `GET /session` | yes | `Sec-Fetch-Site: same-origin`; `Origin`, when sent, equal to the public origin | none: the answer hands it to the page |
| `POST /session/logout` | yes | `Origin` equal to the public origin | `X-CSRF-Token` |

- **The cookie** is `Secure`, `HttpOnly` and `SameSite=Strict`, with
  `Path=/` and no domain. The `__Host-` prefix has the browser refuse it
  from any other host and from plain HTTP.
- **No key:** a request that carries both an `Authorization` header and
  the cookie is refused with
  [`key-and-session`](@/problems/key-and-session.md) on every route of the
  server. A script uses a key; a browser uses a session.
- **The routes that take a key take no session yet.** A request with the
  cookie alone is `401` on them, as any request with no key is.
- **The limits:** a session ends when it has not been seen for 15 minutes,
  and 8 hours after it started whatever was seen. A tenant's administrator
  may set shorter limits for the tenant, `session.idle_seconds` and
  `session.absolute_seconds`
  ([administer a tenant with a key](@/docs/tenant-administration.md)); a
  session takes the limits as they are when it starts.
- **Ended at once:** a grant or removal of a role, or the deactivation of
  the user, ends every session of that user. A change of a tenant's role
  cap, or the removal of the tenant, ends every session in the tenant.

## Before you start

- **A running server** built from `main`, set up as
  [getting started](@/docs/getting-started.md) leaves it: its
  configuration in `/etc/vcon-server/vcon-server.toml`, the tenant `acme`,
  and the certificate authority's copy for clients in
  `/usr/local/share/vcon-server/ca.pem`.
- **A shell on the server's host**, with `sudo`. The configuration changes
  and the server restarts, so every request waits or fails for the seconds
  the restart takes.
- **The origin browsers use**, written as the address bar shows it:
  `https://`, the host in lower case, and the port unless it is 443, with
  no path and no `/` at the end. This page uses
  `https://vcon.example.com:8443`.
- `curl` and `jq` on that machine.
- **Changes on other systems:**
  - A reverse proxy in front of the server must pass the browser's
    `Cookie`, `Origin` and `Sec-Fetch-Site` request headers and the
    server's `Set-Cookie` response header without change, and must serve
    the public origin over HTTPS. The public origin is then the proxy's
    address, not the address in `listen`.
  - With an [operator listener](@/docs/operator-listener.md), give it a
    host name of its own in DNS (and in its certificate) before you set
    `operator.public_origin`: a browser keeps a cookie by host and not by
    port, so the server refuses two public origins on one host.
  - A firewall needs no new rule: sessions use the listeners' own ports.
- One shell for the steps: later steps use variables that earlier steps set.

## Steps

1. Set the variables the steps use, and make an operator key and a tenant
   key to check with. `key create` runs beside the server. `COOKIE` is a
   session cookie as a browser sends it, with an id no session has:

   ```sh
   CONFIG=/etc/vcon-server/vcon-server.toml
   CA=/usr/local/share/vcon-server/ca.pem
   URL=https://vcon.example.com:8443
   TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
   KEY_OPS=$(sudo -u vcon vcon-server key create --operator --label origin-ops --config $CONFIG | jq -r .token)
   KEY_READ=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label origin-read --config $CONFIG | jq -r .token)
   COOKIE=__Host-vcs_session=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
   ```

2. Before the change, the listener has no public origin and refuses every
   request by session, whatever it carries:

   ```sh
   curl -sS --cacert $CA "$URL/session" -H "Cookie: $COOKIE" -H 'Sec-Fetch-Site: same-origin'; echo
   ```

   ```text
   {"status":403,"title":"The request does not come from this listener's own pages","type":"https://vcon-server.com/problems/bad-origin"}
   ```

3. Add the key as the first line of the configuration. It is not in a
   table, so it must come before the first `[table]` line:

   ```sh
   sudo sed -i '1i public_origin = "https://vcon.example.com:8443"' $CONFIG
   ```

4. Restart the server. Under systemd:

   <!-- docs-test: only systemd -->
   ```sh
   sudo systemctl restart vcon-server
   timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
   ```

   ```text
   answering
   ```

   Or, without systemd, stop it and start it again:

   <!-- docs-test: only no-systemd -->
   ```sh
   sudo kill -TERM "$(pidof vcon-server)"
   while pidof vcon-server >/dev/null; do sleep 0.2; done
   ```

   <!-- docs-test: only no-systemd; background serving -->
   ```sh
   sudo -u vcon vcon-server serve --config /etc/vcon-server/vcon-server.toml
   ```

   ```text
   <time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443
   …
   ```

## Check it

A request as a page of the listener makes it now passes the origin check.
Its cookie names no session, so the answer is `401`, with the header that
removes the cookie from the browser:

```sh
curl -sS --cacert $CA "$URL/session" -H "Cookie: $COOKIE" -H 'Sec-Fetch-Site: same-origin'; echo
curl -sS -o /dev/null -D - --cacert $CA "$URL/session" -H "Cookie: $COOKIE" -H 'Sec-Fetch-Site: same-origin' | grep -i '^set-cookie' | tr -d '\r'
```

```text
{"status":401,"title":"A live session is needed","type":"https://vcon-server.com/problems/no-session"}
set-cookie: __Host-vcs_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0
```

The same request from another site, or from a script that sets no
`Sec-Fetch-Site`, is refused before the cookie is looked at:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/session" -H "Cookie: $COOKIE" -H 'Sec-Fetch-Site: cross-site'
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/session" -H "Cookie: $COOKIE"
```

```text
403
403
```

Signing out needs the page's `Origin`. Another origin is refused; the
listener's own passes, and the answer is again that of a cookie with no
session:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA -X POST "$URL/session/logout" -H "Cookie: $COOKIE" -H 'Origin: https://other.example.net'
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA -X POST "$URL/session/logout" -H "Cookie: $COOKIE" -H "Origin: $URL"
```

```text
403
401
```

A key and the cookie together are refused on every route, with a valid key
too. The key alone still works:

```sh
curl -sS --cacert $CA "$URL/key" -H "Authorization: Bearer $KEY_READ" -H "Cookie: $COOKIE"; echo
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/key" -H "Authorization: Bearer $KEY_READ"
```

```text
{"status":400,"title":"The request carries both a key and a session cookie","type":"https://vcon-server.com/problems/key-and-session"}
200
```

The cookie alone opens no route that takes a key:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/vcons" -H "Cookie: $COOKIE" -H "Origin: $URL"
```

```text
401
```

Each refusal above is counted in the admin
[audit log](@/docs/audit-logs.md) as `auth_failed` with no actor, one
record for each status and minute:

```sh
curl -sS --cacert $CA "$URL/admin/audit?limit=1000" -H "Authorization: Bearer $KEY_OPS" \
  | jq -c 'select(.action == "auth_failed" and .actor == "" and .status != 401) | {action, status}' | sort -u
```

```text
{"action":"auth_failed","status":400}
{"action":"auth_failed","status":403}
```

Once a sign-in method exists, a session's own records name its user as
`user:<id>`: `session_start`, `session_end` and the counted
`session_read` ([audit log records](@/docs/audit-log-records.md)).

## With an operator listener

A server with an `[operator]` table has two listeners, and each has its
own public origin: `public_origin` for the address in `listen`, and
`operator.public_origin` for the address in `operator.listen`. The two
must have different hosts:

```toml
public_origin = "https://vcon.example.com"

[operator]
listen = "127.0.0.1:8444"
public_origin = "https://ops.example.com:8444"
```

A session of a tenant's user is served on the tenant listener only, and an
operator's on the operator listener only. A session that reaches the other
listener is refused with
[`wrong-listener`](@/problems/wrong-listener.md) and counted as
`wrong_listener` under its user.

## Go back to no sessions

Take the line out and restart. The listener then refuses every request by
session again, and sessions that were live are not served until the key is
back or they end:

```sh
sudo sed -i '/^public_origin = /d' $CONFIG
```

Restart, under systemd:

<!-- docs-test: only systemd -->
```sh
sudo systemctl restart vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
```

```text
answering
```

Or, without systemd:

<!-- docs-test: only no-systemd -->
```sh
sudo kill -TERM "$(pidof vcon-server)"
while pidof vcon-server >/dev/null; do sleep 0.2; done
```

<!-- docs-test: only no-systemd; background serving -->
```sh
sudo -u vcon vcon-server serve --config /etc/vcon-server/vcon-server.toml
```

```text
<time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443
…
```

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/session" -H "Cookie: $COOKIE" -H 'Sec-Fetch-Site: same-origin'
```

```text
403
```

## When a step fails

- **`403`, [`bad-origin`](@/problems/bad-origin.md), after the restart:**
  the value of `public_origin` is not the origin the request names. Compare
  it with the browser's address bar, character by character: scheme, host
  in lower case, port unless it is 443, nothing after. Behind a reverse
  proxy it is the proxy's public address, and the proxy must pass `Origin`
  and `Sec-Fetch-Site` without change.
- **`401`, [`no-session`](@/problems/no-session.md):** the origin check
  passed and the cookie names no live session. With no sign-in method that
  is every cookie.
- **`403`, [`bad-csrf`](@/problems/bad-csrf.md):** the session is live and
  the `X-CSRF-Token` header is missing or wrong. Ask `GET /session` for
  the token.
- **`400`, [`key-and-session`](@/problems/key-and-session.md):** the
  request has an `Authorization` header and the session cookie. Send one.
- **`vcon-server: configuration: public_origin: … is not an origin`:** the
  value has a path, a `/` at the end, `http://`, an upper-case letter or
  no scheme. Write `https://`, the host and the port alone.
- **`vcon-server: configuration: operator.public_origin: … has the host of
  `public_origin``:** the two listeners' origins name one host. Give the
  operator listener a host name of its own.
- **`unknown field `public_origin``:** release 0.4.3 and earlier do not
  have the key. Take the line out, or run a build of `main`.
- **`duplicate key` or the key seems to be ignored:** the line is under a
  `[table]`. Move it above the first table, or, for the operator
  listener's, into `[operator]`.

## Clean up

Revoke the two keys this page made:

```sh
for id in $(sudo -u vcon vcon-server key list --config $CONFIG \
  | jq -r 'select(.label == "origin-ops" or .label == "origin-read") | .id'); do
  sudo -u vcon vcon-server key revoke "$id" --config $CONFIG
done
```
