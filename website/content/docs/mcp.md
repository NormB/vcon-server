+++
title = "MCP"
description = "Connecting an AI agent to vcon-server over MCP, through HTTPS or a stdio command."
weight = 74
+++

vcon-server serves the Model Context Protocol (MCP), the protocol AI agents
use to find and call a server's tools. An agent connects over HTTPS to the
server's TLS listener, or starts `vcon-server mcp`, a command that forwards to
a running server. Either way it authenticates with an API key, the same keys
the [HTTP API](@/docs/http-api.md) takes.

The server has no tools yet: an agent can connect and complete the handshake,
and `tools/list` answers an empty list. The tools are being built (see
[what is built](@/docs/status.md)).

## Connect over HTTPS

The server answers MCP at `POST /mcp` on its TLS listener, the same address as
the HTTP API. Send the API key as `Authorization: Bearer <token>`, as on every
route. Each request carries one JSON-RPC message and gets one JSON answer;
there is no session. The server takes protocol versions 2024-11-05 to
2025-11-25 through `initialize`, and 2026-07-28 through `server/discover`. It
names itself `vcon-server` with its version.

- No key, or a malformed, unknown or revoked one: 401 with
  `WWW-Authenticate: Bearer`.
- A request with an `Origin` header, as a web page sends: 403.
- `GET /mcp` and `DELETE /mcp`: 405. There is no event stream and no session
  to end.
- A body over `limits.body`: 413 [too-large](@/problems/too-large.md).

Every request to `/mcp` is counted in the key's audit log under the action
`mcp`, with its status; an operator key's requests are counted in the admin
log. See [audit logs](@/docs/audit-logs.md).

## Connect over stdio

An agent that starts its MCP servers as commands runs
`vcon-server mcp --config <client file>`. The command reads MCP messages on
standard input, sends each to a running server's `/mcp` with the one key its
client file names, and writes the answers to standard output. It opens no data
directory and no master key, so it can run on another machine. The
[command line](@/docs/command-line.md#mcp-for-an-ai-agent) page describes the
client file and the exit codes.

## Client configuration

These blocks are for a client that reads its servers from an `mcpServers`
object in a JSON file. For a client that starts its servers as commands:

```json
{
  "mcpServers": {
    "vcon-server": {
      "command": "vcon-server",
      "args": ["mcp", "--config", "/home/agent/.config/vcon-server/mcp.toml"]
    }
  }
}
```

`mcp.toml` is the client file. The key stays in the key file it names, never
in the client's configuration.

For a client that connects over HTTP itself:

```json
{
  "mcpServers": {
    "vcon-server": {
      "type": "http",
      "url": "https://vcon.example.com:8443/mcp",
      "headers": {
        "Authorization": "Bearer ${VCON_SERVER_TOKEN}"
      }
    }
  }
}
```

A client that expands `${VAR}` in its configuration replaces
`${VCON_SERVER_TOKEN}` with that environment variable's value; otherwise
write the token there, and keep the file readable only by its owner. When the
server's certificate comes from a private CA, a client that runs on Node.js
trusts it when `NODE_EXTRA_CA_CERTS` names the CA's PEM file.

## Tools

There are no tools yet.
