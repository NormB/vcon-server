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

## Tool calls

Each `tools/call` passes one gate before the tool runs. The gate checks, in
this order:

| Check | Refusal | Status | JSON-RPC code |
|---|---|---|---|
| More than `mcp.calls_per_second` calls by one key in one second (default 100) | [rate-limited](@/problems/rate-limited.md) | 429 | -32000 |
| More than `mcp.concurrent` calls running on the server (default 100) | [at-capacity](@/problems/at-capacity.md) | 503 | -32000 |
| A tool the server does not have | [unknown-tool](@/problems/unknown-tool.md) | 404 | -32602 |
| A tool the key's scopes do not allow | [forbidden](@/problems/forbidden.md) | 403 | -32001 |
| An argument of the wrong type, or one the tool does not take | [invalid-arguments](@/problems/invalid-arguments.md) | 422 | -32602 |

- **A refusal** is a JSON-RPC error whose `data` is the
  [problem document](@/problems/_index.md) the HTTP API answers with, its
  status included. A rate-limited one also gives `retry_after_ms`, the wait
  until the key's one-second window ends. A call past either limit is refused
  at once, never queued.
- **Records.** Every refusal is written to the key's audit log under the
  tool's action, or under `mcp` for a tool the server does not have:
  rate-limited and at-capacity are counted, the others recorded one row each.
  A refusal that cannot be written answers
  [audit-failed](@/problems/audit-failed.md) instead.
- **Scopes.** `tools/list` shows a key only the tools it may call. A tool's
  scope follows from its name and its hints: a `vcon_` tool needs read when
  it is read-only, delete when it is destructive, and write otherwise; a
  `legal_` tool needs legal; an `admin_` tool needs admin.

The limits are set in the [configuration](@/docs/configuration.md). While
there are no tools, every call is refused as unknown-tool.

## Tools

There are no tools yet.
