+++
title = "Operator listener"
description = "Give operator keys a listener of their own, on an address only operators reach, so the address tenants use serves tenant keys only: the configuration, the restart, the checks, and what to change in proxies and clients."
weight = 34

[extra]
group = "how-to"
+++

A server has one listener by default: the address in `listen` serves every
key, a tenant's and an operator's alike. This page gives operator keys a
listener of their own. With an `[operator]` table in the configuration, the
server listens on a second address, and each address serves one kind of
key:

| Address | Serves | Refuses |
|---|---|---|
| `listen` (the tenant listener) | tenant keys: vCons, legal, audit, MCP | operator keys |
| `operator.listen` (the operator listener) | operator keys: tenants, keys, the admin log, MCP | tenant keys |

A valid key that comes to the other listener is refused with `403` and the
problem type [`wrong-listener`](@/problems/wrong-listener.md), before any
route sees the request. So a leaked operator key is of no use to someone
who can reach only the address tenants use, and the operator listener can
sit on an address the internet does not reach: the loopback address, behind
an SSH tunnel, or a management network.

The operator listener is on `main` only, in no release yet: release 0.4.2
refuses a configuration with an `[operator]` table as an unknown key.

Without the table nothing changes: one listener serves both kinds of key,
as in every release so far.

CI runs every command on this page against a live server, after
[getting started](@/docs/getting-started.md).

## Before you start

- **A running server** set up as [getting started](@/docs/getting-started.md)
  leaves it: its configuration in `/etc/vcon-server/vcon-server.toml`, the
  tenant `acme`, and the certificate authority's copy for clients in
  `/usr/local/share/vcon-server/ca.pem`.
- **A shell on the server's host**, with `sudo`. The configuration changes
  and the server restarts, so every request waits or fails for the seconds
  the restart takes.
- **A port for the operator listener** that nothing else uses. This page
  uses `8444` on the loopback address `127.0.0.1`.
- **A way for operators to reach that address.** On the loopback address,
  that is a shell on the host or an SSH tunnel to it
  ([from another machine](#reach-it-from-another-machine)). On a management
  network's address, it is that network, and a firewall rule that lets only
  operators' machines reach the port.
- **A list of everything that uses an operator key over HTTP today**,
  because each must move to the new address:
  - scripts and jobs that call the `/admin/` routes;
  - an MCP client or `vcon-server mcp` client file that holds an operator
    key ([connect an AI agent](@/docs/connect-mcp.md));
  - bookmarks of the [console](@/docs/console.md) that operators sign in
    at;
  - a monitor that asks `/health` with an operator key. Give it a tenant
    key instead, as getting started does, or point it at the operator
    listener.
- **Changes on other systems:**
  - A reverse proxy in front of the server no longer needs to pass
    `/admin/` on the public name. Take that rule out, and do not add the
    operator listener as an upstream of the public name.
  - A firewall needs no new rule for an operator listener on the loopback
    address. For one on another address, allow the port from operators'
    machines only.
- One shell for the steps: later steps use variables that earlier steps set.

## Steps

1. Set the variables the steps use, and make an operator key and a tenant
   key to check with. `key create` runs beside the server:

   ```sh
   CONFIG=/etc/vcon-server/vcon-server.toml
   CA=/usr/local/share/vcon-server/ca.pem
   URL=https://vcon.example.com:8443
   OPS_URL=https://vcon.example.com:8444
   TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
   KEY_OPS=$(sudo -u vcon vcon-server key create --operator --label listener-ops --config $CONFIG | jq -r .token)
   KEY_READ=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label listener-read --config $CONFIG | jq -r .token)
   ```

2. Before the change, the one listener serves the operator key:

   ```sh
   curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/admin/tenants" -H "Authorization: Bearer $KEY_OPS"
   ```

   ```text
   200
   ```

3. Add the table to the end of the configuration:

   ```sh
   printf '\n[operator]\nlisten = "127.0.0.1:8444"\n' | sudo tee -a $CONFIG >/dev/null
   ```

   The operator listener presents the certificate in `tls.cert`, like the
   tenant listener. To give it one of its own, see
   [a certificate of its own](#a-certificate-of-its-own).

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
   <time>  INFO vcon_server::serve: the operator listener is bound operator_listen=127.0.0.1:8444
   <time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443
   …
   ```

   The server binds both addresses before it serves either. The line that
   names the operator listener comes first, so when the `serving` line is
   there, both listen.

## Check it

The operator key works on the operator listener:

```sh
curl -sS --cacert $CA "$OPS_URL/admin/tenants" -H "Authorization: Bearer $KEY_OPS" | jq -r '.[].name'
```

```text
acme
```

The tenant listener refuses it, on an operator route and on every other:

```sh
curl -sS --cacert $CA "$URL/admin/tenants" -H "Authorization: Bearer $KEY_OPS"; echo
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/key" -H "Authorization: Bearer $KEY_OPS"
```

```text
{"status":403,"title":"This listener does not serve this kind of key","type":"https://vcon-server.com/problems/wrong-listener"}
403
```

The tenant key works on the tenant listener, and the operator listener
refuses it:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/vcons" -H "Authorization: Bearer $KEY_READ"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$OPS_URL/vcons" -H "Authorization: Bearer $KEY_READ"
```

```text
200
403
```

A request with no key, or with a key the server does not know, is `401` on
both, as before: only a valid key learns that it came to the wrong
listener.

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$OPS_URL/admin/tenants"
```

```text
401
```

Each refusal is counted in the [audit log](@/docs/audit-logs.md) as
`wrong_listener`, one record for each key and minute: in the admin log for
an operator key, and in the tenant's log for a tenant key. The operator
key's two refusals above are in the admin log, read here on the operator
listener:

```sh
curl -sS --cacert $CA "$OPS_URL/admin/audit?limit=1000" -H "Authorization: Bearer $KEY_OPS" \
  | jq -c 'select(.action == "wrong_listener") | {action, status}' | sort -u
```

```text
{"action":"wrong_listener","status":403}
```

A `wrong_listener` record for an operator key that no operator sent is a
sign that the key has leaked: revoke it
([keys and scopes](@/docs/keys-and-scopes.md)).

## Reach it from another machine

An operator listener on `127.0.0.1` takes connections from the host
itself only. From another machine, carry the port over SSH and keep the
name the certificate holds:

<!-- docs-test: skip it needs a second machine and an SSH login on the host -->
```sh
ssh -N -L 8444:127.0.0.1:8444 you@vcon.example.com &
curl -sS --cacert ca.pem --resolve vcon.example.com:8444:127.0.0.1 \
  https://vcon.example.com:8444/admin/tenants -H "Authorization: Bearer $KEY_OPS"
```

`--resolve` sends the request for `vcon.example.com:8444` to the tunnel's
end on this machine, so the certificate's name still matches. For a
browser, add `127.0.0.1 vcon.example.com` to the machine's hosts file
while the tunnel is open, and open `https://vcon.example.com:8444/console/`.

Both listeners serve the [console](@/docs/console.md). An operator signs
in at the operator listener's address, and a tenant's people at the tenant
listener's. A key of the other kind is told to use the other address.

## A certificate of its own

The two listeners are two origins to a browser, and may be two names. To
have the operator listener present its own certificate, name the pair in
the table:

```toml
[operator]
listen = "127.0.0.1:8444"

[operator.tls]
cert = "/etc/vcon-server/operator-cert.pem"
key = "/etc/vcon-server/operator-key.pem"
```

Both keys of `[operator.tls]` are needed together. The key file follows the
same rule as `tls.key`: owned by the server's account, and not readable by
other users.

## Go back to one listener

Take the table out and restart. The one listener serves both kinds of key
again:

```sh
sudo sed -i '/^\[operator\]$/,$d' $CONFIG
```

That removes the table when it is the last thing in the file, as step 3
wrote it. Restart, under systemd:

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
curl -sS -o /dev/null -w '%{http_code}\n' --cacert $CA "$URL/admin/tenants" -H "Authorization: Bearer $KEY_OPS"
```

```text
200
```

## When a step fails

- **`403`, [`wrong-listener`](@/problems/wrong-listener.md):** the key is
  valid and came to the listener that serves the other kind. Send an
  operator key to the address in `operator.listen` and a tenant key to the
  address in `listen`.
- **`vcon-server: the operator listening address cannot be bound: Address
  already in use`:** another program, or another vcon-server, has the
  port. Choose another port, or stop that program. Neither listener serves
  until both can be bound.
- **`vcon-server: configuration: operator.listen: … is the socket
  `listen` takes`:** the two addresses are the same socket, or one is the
  unspecified address (`0.0.0.0`) on the other's port. Give the operator
  listener a port of its own.
- **`vcon-server: the operator listener's TLS certificate or key cannot be
  used`:** the message names the key and the file; the fix is the one for
  `tls.key` in [getting started](@/docs/getting-started.md).
- **`unknown field `operator``, from a release binary:** release 0.4.2 and
  earlier do not have the operator listener. Take the table out, or
  install a build from `main`.
- **Connection refused from another machine:** the operator listener is on
  the loopback address, which only the host reaches. Use a tunnel
  ([from another machine](#reach-it-from-another-machine)), or put the
  listener on an address of a network operators are on.
- **A certificate error through a tunnel:** the request went to
  `127.0.0.1` or `localhost`, a name the certificate does not hold. Keep
  the certificate's name, with `--resolve` or a hosts entry.
- **The console's Tenants view no longer opens at the public address:**
  operators now sign in at the operator listener's address.
- **A script that used an operator key stopped working with `403`:** point
  it at the operator listener's address.

## Clean up

Revoke the two keys this page made:

```sh
for id in $(sudo -u vcon vcon-server key list --config $CONFIG \
  | jq -r 'select(.label == "listener-ops" or .label == "listener-read") | .id'); do
  sudo -u vcon vcon-server key revoke "$id" --config $CONFIG
done
```
