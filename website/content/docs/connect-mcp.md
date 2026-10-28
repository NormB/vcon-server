+++
title = "Connect an AI agent over MCP"
description = "Give an AI agent its own key, then check the MCP endpoint by hand: the handshake and the tool list over HTTPS, the refusals, and the same through vcon-server mcp."
weight = 19
+++

An AI agent reaches vcon-server through the Model Context Protocol (MCP):
over HTTPS at `/mcp` on the server's TLS listener, or by starting
`vcon-server mcp`, which forwards to a running server. [MCP](@/docs/mcp.md)
describes both, and the configuration an agent's client reads. This how-to
makes a key for an agent and checks each path by hand, with `curl` and with
`vcon-server mcp`, before an agent is pointed at it.

The server has no MCP tools yet: an agent can connect and complete the
handshake, and the tool list is empty. The tools are being built (see
[what is built](@/docs/status.md)).

CI runs every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A running server, set up as in [getting started](@/docs/getting-started.md),
  with the tenant `acme`, and `fullchain.pem` in the current directory.
- One shell for every step: later steps use variables that earlier steps set.

```sh
URL=https://vcon.example.com:8443
CONFIG=/etc/vcon-server/vcon-server.toml
TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
```

## Give the agent its own key

An agent is a client like any other: give it a key of its own, with only the
scopes its work needs and a label that names it. Over MCP the key reaches
what it reaches over HTTP, no more:

```sh
AGENT=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label agent-1 \
  --config $CONFIG | jq -r .token)
```

## The handshake over HTTPS

Each request to `POST /mcp` carries one JSON-RPC message, with the key as a
bearer token, and gets one JSON answer. `initialize` names the protocol
version the client speaks. The server answers with its own name and
version:

```sh
curl -sS --cacert fullchain.pem "$URL/mcp" \
  -H "Authorization: Bearer $AGENT" \
  -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"1"}}}' \
  | jq -c '{protocol: .result.protocolVersion, server: .result.serverInfo.name}'
```

```text
{"protocol":"2025-06-18","server":"vcon-server"}
```

The tool list is empty:

```sh
curl -sS --cacert fullchain.pem "$URL/mcp" \
  -H "Authorization: Bearer $AGENT" \
  -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | jq -c .result
```

```text
{"tools":[]}
```

## What the endpoint refuses

Without a key, or with a revoked one, the answer is `401`. A request with an
`Origin` header, as a web page sends, is refused with `403`. `GET /mcp` is
`405`: there is no event stream:

```sh
BODY='{"jsonrpc":"2.0","id":3,"method":"tools/list"}'
H=(-H "Content-Type: application/json" -H "Accept: application/json, text/event-stream")
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/mcp" "${H[@]}" -d "$BODY"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/mcp" "${H[@]}" -d "$BODY" \
  -H "Authorization: Bearer $AGENT" -H "Origin: https://example.com"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/mcp" -H "Authorization: Bearer $AGENT"
```

```text
401
403
405
```

## The same through vcon-server mcp

An agent that starts its MCP servers as commands runs `vcon-server mcp`. It
reads a client file that names the server's `/mcp` address, a file holding
the key, and the certificate to trust. Make them in a directory readable by
the agent's account only:

```sh
install -d -m 0700 agent
printf '%s\n' "$AGENT" > agent/agent.token
chmod 0600 agent/agent.token
cp fullchain.pem agent/ca.pem
cat > agent/mcp.toml <<EOF
url = "$URL/mcp"
key_file = "$PWD/agent/agent.token"
ca = "$PWD/agent/ca.pem"
EOF
```

The command reads one JSON-RPC message a line on standard input and writes
each answer on standard output, one a line. It stops when standard input
ends. Send the handshake and the tool list:

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"shell","version":"1"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | vcon-server mcp --config agent/mcp.toml | jq -c '.result | {server: .serverInfo.name, tools}'
```

```text
{"server":"vcon-server","tools":null}
{"server":null,"tools":[]}
```

Like every `vcon-server` command, `mcp` locks its memory, so the account that
runs it needs the memory-lock allowance (see
[getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory)).
A key the server refuses stops it with exit code 1:

<!-- docs-test: exit 1 -->
```sh
printf 'vcs_0000000000000000_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n' > agent/agent.token
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"shell","version":"1"}}}' \
  | vcon-server mcp --config agent/mcp.toml
```

```text
vcon-server: the server refused the key (401)
```

## Every request is in the audit log

Each request to `/mcp` is counted in the key's tenant log under the action
`mcp`, with its status. Read the statuses with a key that holds `audit`.
Here they are `200` for the answers, `202` for the notification, and `403`
for the request with an `Origin` header:

```sh
AUDITOR=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope audit --label agent-audit \
  --config $CONFIG | jq -r .token)
curl -sS --cacert fullchain.pem "$URL/audit?limit=10000" -H "Authorization: Bearer $AUDITOR" \
  | jq -r 'select(.action == "mcp") | .status' | sort -u
```

```text
200
202
403
```

## Point the agent at it

[MCP](@/docs/mcp.md#client-configuration) gives the configuration blocks for
a client that starts `vcon-server mcp` and for one that connects over HTTP
itself. Copy `agent/mcp.toml`, the key file and `ca.pem` to the agent's
account, and point the client file's paths at them.

## Clean up

Revoke the keys this page made:

```sh
sudo -u vcon vcon-server key list --config $CONFIG | jq -r 'select(.label | startswith("agent-")) | .id' \
  | while read -r id; do sudo -u vcon vcon-server key revoke "$id" --config $CONFIG; done
rm -r agent
```
