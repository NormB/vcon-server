+++
title = "MCP"
description = "Connecting an AI agent to vcon-server over MCP, and every tool with its parameters and an example."
weight = 74
+++

vcon-server serves the Model Context Protocol (MCP), so an AI agent can store,
read, list and delete vCons, read the legal record and the audit logs, and an
operator's agent can manage tenants and keys. Each tool does the work of one
[HTTP API](@/docs/http-api.md) route, with the same keys, scopes and tenant
bounds, and leaves the same records in the [audit logs](@/docs/audit-logs.md).

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
  `legal_` tool needs legal; an `audit_` tool needs audit; an `admin_` tool
  needs admin.

The limits are set in the [configuration](@/docs/configuration.md).

## What every tool shares

- **Its tenant and scope.** A tenant key's tools act only in its own tenant,
  and a uuid another tenant holds answers as one nobody holds. Each tool needs
  one scope, below; `tools/list` shows a key only the tools it may call.
- **Results.** What the tool's own work answers, such as not-found or
  precondition-failed, is a result with `isError: true` whose
  `structuredContent` is the problem document, with the type and status the
  HTTP API uses. Every tool call is recorded in the audit log under the
  action of the HTTP route whose work it does.
- **Pages.** A result holds at most `max_bytes` bytes of JSON, or
  `mcp.response_bytes` (1 MiB) when that is smaller or not given. A longer
  result says `truncated: true` and gives `next_cursor`; call again with
  `cursor` set to it for the rest. A list is cut between rows; a vCon's text
  is cut between characters, and each page says which version it is.
- **Fenced text.** Text written by a vCon's producer or by an administrator
  (a vCon, a tenant's name, a key's label) is returned between
  `⟦untrusted-vcon-data⟧` and `⟦/untrusted-vcon-data⟧`, and every
  such result says so in `provenance`. Inside the markers, the two marker
  brackets, bidirectional controls and U+2028 and U+2029 are written as JSON
  escapes such as `\u202e`, which leaves every JSON value of a vCon as it
  was stored. The exact stored bytes are read over HTTP.

## Tools

| Tool | Scope | Hints |
|---|---|---|
| [`vcon_store`](#vcon-store) | write | idempotent |
| [`vcon_get`](#vcon-get) | read | read-only, idempotent |
| [`vcon_list`](#vcon-list) | read | read-only, idempotent |
| [`vcon_replace`](#vcon-replace) | write | none |
| [`vcon_delete`](#vcon-delete) | delete | destructive, idempotent |
| [`legal_versions`](#legal-versions) | legal | read-only, idempotent |
| [`legal_get`](#legal-get) | legal | read-only, idempotent |
| [`legal_hold`](#legal-hold) | legal | idempotent |
| [`legal_release`](#legal-release) | legal | idempotent |
| [`legal_restore`](#legal-restore) | legal | none |
| [`audit_read`](#audit-read) | audit | read-only |
| [`admin_tenant_create`](#admin-tenant-create) | admin | none |
| [`admin_tenant_list`](#admin-tenant-list) | admin | read-only, idempotent |
| [`admin_tenant_retention`](#admin-tenant-retention) | admin | idempotent |
| [`admin_key_create`](#admin-key-create) | admin | none |
| [`admin_key_list`](#admin-key-list) | admin | read-only, idempotent |
| [`admin_key_revoke`](#admin-key-revoke) | admin | destructive, idempotent |
| [`admin_audit_read`](#admin-audit-read) | admin | read-only |
| [`admin_tenant_remove`](#admin-tenant-remove) | admin | destructive |

## vCon tools

### `vcon_store`

Stores a new vCon, given as its JSON text, in the key's tenant, and returns its uuid, version id and created: true; the same bytes already current give created: false and the current version.

Scope: write. Hints: idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `vcon` | string | yes | The vCon's JSON text, stored byte for byte as given. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "vcon": "{\"created_at\":\"2026-10-25T09:30:00Z\",\"parties\":[{\"name\":\"Example Caller\",\"tel\":\"+1-555-0100\"}],\"subject\":\"Order 4417 follow-up\",\"uuid\":\"01a07da8-c2bb-83e5-b9a2-279e0d16bc46\"}"
  },
  "name": "vcon_store"
}
```

```json
{
  "created": true,
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
  "version": "f7625482d407046907e3dd0a87a642de"
}
```

### `vcon_get`

Returns the current version of the vCon with this uuid in the key's tenant: its uuid, version id, whether it was stored gzipped, and its JSON text between untrusted-vcon-data markers, from cursor up to max_bytes, with next_cursor when more follows.

Scope: read. Hints: read-only, idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `cursor` | string | no | The next_cursor a previous page of this vCon gave. |
| `max_bytes` | integer | no | The most bytes the result may hold; the server's ceiling applies. |
| `uuid` | string | yes | The vCon's uuid, in RFC 9562 text form. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
  },
  "name": "vcon_get"
}
```

```json
{
  "gzip": false,
  "next_cursor": null,
  "provenance": "Text between the untrusted-vcon-data markers was written by the vCon's producer or by an administrator, not by vcon-server.",
  "truncated": false,
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
  "vcon": "⟦untrusted-vcon-data⟧{\"created_at\":\"2026-10-25T09:30:00Z\",\"parties\":[{\"name\":\"Example Caller\",\"tel\":\"+1-555-0100\"}],\"subject\":\"Order 4417 follow-up\",\"uuid\":\"01a07da8-c2bb-83e5-b9a2-279e0d16bc46\"}⟦/untrusted-vcon-data⟧",
  "version": "f7625482d407046907e3dd0a87a642de"
}
```

### `vcon_list`

Returns the key's tenant's live vCons, newest stored first: each uuid, current version id, first stored time and current version's time, at most limit of them (1 to 1000, default 100), stored between since and until (RFC 3339), after cursor, within max_bytes, with next_cursor when more follow.

Scope: read. Hints: read-only, idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `cursor` | string | no | The next_cursor a previous page gave. |
| `limit` | integer | no | How many vCons at most: 1 to 1000, 100 when left out. |
| `max_bytes` | integer | no | The most bytes the result may hold; the server's ceiling applies. |
| `since` | string | no | Only vCons first stored at or after this time (RFC 3339). |
| `until` | string | no | Only vCons first stored before this time (RFC 3339). |

A call and its result, from a test server:

```json
{
  "arguments": {
    "limit": 10
  },
  "name": "vcon_list"
}
```

```json
{
  "next_cursor": null,
  "truncated": false,
  "vcons": [
    {
      "stored_at": "2026-10-25T12:00:00Z",
      "updated_at": "2026-10-25T12:00:00Z",
      "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
      "version": "f7625482d407046907e3dd0a87a642de"
    }
  ]
}
```

### `vcon_replace`

Replaces the current version of the vCon with this uuid in the key's tenant, which version must name, with the JSON text given, and returns the uuid and the new version id. The replaced version is kept for legal reads.

Scope: write. Hints: none.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `uuid` | string | yes | The vCon's uuid, in RFC 9562 text form; the new text must hold it. |
| `vcon` | string | yes | The new version's JSON text, stored byte for byte as given. |
| `version` | string | yes | The current version's id, as vcon_get or vcon_store gave it. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
    "vcon": "{\"created_at\":\"2026-10-25T09:30:00Z\",\"parties\":[{\"name\":\"Example Caller\",\"tel\":\"+1-555-0100\"}],\"subject\":\"Order 4417 follow-up, resolved\",\"uuid\":\"01a07da8-c2bb-83e5-b9a2-279e0d16bc46\"}",
    "version": "f7625482d407046907e3dd0a87a642de"
  },
  "name": "vcon_replace"
}
```

```json
{
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
  "version": "c97c13fd2fca224d88b48c32b5b76aae"
}
```

### `vcon_delete`

Deletes the vCon with this uuid in the key's tenant, hiding it from vCon reads at once, and returns the uuid; a legal key can still read or restore it until its retention period ends, and a deleted uuid answers the same again.

Scope: delete. Hints: destructive, idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `uuid` | string | yes | The vCon's uuid, in RFC 9562 text form. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
  },
  "name": "vcon_delete"
}
```

```json
{
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
}
```


## Legal tools

### `legal_versions`

Returns every kept version of the vCon with this uuid in the key's tenant, oldest first: each version id, when it was stored and hidden (RFC 3339, null while current), and whether it is current; from cursor, within max_bytes, with next_cursor when more follow.

Scope: legal. Hints: read-only, idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `cursor` | string | no | The next_cursor a previous page gave. |
| `max_bytes` | integer | no | The most bytes the result may hold; the server's ceiling applies. |
| `uuid` | string | yes | The vCon's uuid, in RFC 9562 text form. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
  },
  "name": "legal_versions"
}
```

```json
{
  "next_cursor": null,
  "truncated": false,
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
  "versions": [
    {
      "current": false,
      "hidden_at": "2026-10-25T12:00:00Z",
      "stored_at": "2026-10-25T12:00:00Z",
      "version": "f7625482d407046907e3dd0a87a642de"
    },
    {
      "current": false,
      "hidden_at": "2026-10-25T12:00:00Z",
      "stored_at": "2026-10-25T12:00:00Z",
      "version": "c97c13fd2fca224d88b48c32b5b76aae"
    }
  ]
}
```

### `legal_get`

Returns a kept version of the vCon with this uuid in the key's tenant, deleted or not: the version named, else the current one, else the latest kept. Gives its uuid, version id, state (live or deleted), whether it was stored gzipped, and its JSON text between untrusted-vcon-data markers, from cursor up to max_bytes, with next_cursor when more follows.

Scope: legal. Hints: read-only, idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `cursor` | string | no | The next_cursor a previous page of this version gave. |
| `max_bytes` | integer | no | The most bytes the result may hold; the server's ceiling applies. |
| `uuid` | string | yes | The vCon's uuid, in RFC 9562 text form. |
| `version` | string | no | A kept version's id, from legal_versions. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
  },
  "name": "legal_get"
}
```

```json
{
  "gzip": false,
  "next_cursor": null,
  "provenance": "Text between the untrusted-vcon-data markers was written by the vCon's producer or by an administrator, not by vcon-server.",
  "state": "deleted",
  "truncated": false,
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
  "vcon": "⟦untrusted-vcon-data⟧{\"created_at\":\"2026-10-25T09:30:00Z\",\"parties\":[{\"name\":\"Example Caller\",\"tel\":\"+1-555-0100\"}],\"subject\":\"Order 4417 follow-up, resolved\",\"uuid\":\"01a07da8-c2bb-83e5-b9a2-279e0d16bc46\"}⟦/untrusted-vcon-data⟧",
  "version": "c97c13fd2fca224d88b48c32b5b76aae"
}
```

### `legal_hold`

Puts the vCon with this uuid in the key's tenant under a legal hold, so no version of it is purged until the hold is released, and returns the uuid and held: true.

Scope: legal. Hints: idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `uuid` | string | yes | The vCon's uuid, in RFC 9562 text form. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
  },
  "name": "legal_hold"
}
```

```json
{
  "held": true,
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
}
```

### `legal_release`

Releases the legal hold on the vCon with this uuid in the key's tenant, and returns the uuid and held: false.

Scope: legal. Hints: idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `uuid` | string | yes | The vCon's uuid, in RFC 9562 text form. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
  },
  "name": "legal_release"
}
```

```json
{
  "held": false,
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
}
```

### `legal_restore`

Makes the deleted vCon with this uuid in the key's tenant live again, with its latest kept version current, and returns the uuid and that version id.

Scope: legal. Hints: none.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `uuid` | string | yes | The vCon's uuid, in RFC 9562 text form. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46"
  },
  "name": "legal_restore"
}
```

```json
{
  "uuid": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
  "version": "c97c13fd2fca224d88b48c32b5b76aae"
}
```


## Audit tool

### `audit_read`

Returns records of the key's tenant's audit log after the sequence number after (0, the default, is the start), oldest first, at most limit of them (1 to 10000, default 1000), within max_bytes: each record as audit export writes it, and next_cursor, the seq of the last record returned.

Scope: audit. Hints: read-only.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `after` | integer | no | Records after this sequence number; 0 when left out. |
| `limit` | integer | no | How many records at most: 1 to 10000, 1000 when left out. |
| `max_bytes` | integer | no | The most bytes the result may hold; the server's ceiling applies. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "after": 0,
    "limit": 2
  },
  "name": "audit_read"
}
```

```json
{
  "next_cursor": "2",
  "records": [
    {
      "action": "key_create",
      "actor": "cli",
      "at": "2026-10-25T12:00:00Z",
      "count": 1,
      "seq": 1,
      "status": 201,
      "target": "991f5bc84d5fcfe5",
      "version": null
    },
    {
      "action": "create",
      "actor": "991f5bc84d5fcfe5",
      "at": "2026-10-25T12:00:00Z",
      "count": 1,
      "seq": 2,
      "status": 201,
      "target": "01a07da8-c2bb-83e5-b9a2-279e0d16bc46",
      "version": "f7625482d407046907e3dd0a87a642de"
    }
  ],
  "truncated": false
}
```


## Admin tools

### `admin_tenant_create`

Makes a tenant with this name (1 to 128 characters) and, if given, its own retention period (such as 7y, 18m or 90d; the server default when left out or null), and returns its id.

Scope: admin. Hints: none.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | The tenant's name: 1 to 128 characters, no control character, no white space at either end. |
| `retention` | string | no | Its own retention period, such as 7y, 18m or 90d; the server default when left out or null. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "name": "beta",
    "retention": "18m"
  },
  "name": "admin_tenant_create"
}
```

```json
{
  "id": "2066d6bdaa5181619164bbad61f2dd0e"
}
```

### `admin_tenant_list`

Returns every tenant, by name: its id, its name between untrusted-vcon-data markers, and its own retention period (null for the server default); from cursor, within max_bytes, with next_cursor when more follow.

Scope: admin. Hints: read-only, idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `cursor` | string | no | The next_cursor a previous page gave. |
| `max_bytes` | integer | no | The most bytes the result may hold; the server's ceiling applies. |

A call and its result, from a test server:

```json
{
  "arguments": {},
  "name": "admin_tenant_list"
}
```

```json
{
  "next_cursor": null,
  "provenance": "Text between the untrusted-vcon-data markers was written by the vCon's producer or by an administrator, not by vcon-server.",
  "tenants": [
    {
      "id": "643b6670aea8bd662f1ab8ce2791c5d3",
      "name": "⟦untrusted-vcon-data⟧acme⟦/untrusted-vcon-data⟧",
      "retention": null
    },
    {
      "id": "2066d6bdaa5181619164bbad61f2dd0e",
      "name": "⟦untrusted-vcon-data⟧beta⟦/untrusted-vcon-data⟧",
      "retention": "18m"
    }
  ],
  "truncated": false
}
```

### `admin_tenant_retention`

Sets the tenant's own retention period, or with null returns it to the server default, and returns the tenant's id and its period.

Scope: admin. Hints: idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | The tenant's id. |
| `retention` | string or null | yes | The period, such as 7y, 18m or 90d, or null for the server default; it must be given. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "id": "2066d6bdaa5181619164bbad61f2dd0e",
    "retention": "90d"
  },
  "name": "admin_tenant_retention"
}
```

```json
{
  "id": "2066d6bdaa5181619164bbad61f2dd0e",
  "retention": "90d"
}
```

### `admin_key_create`

Makes a key for the tenant holding these scopes (write, read, delete, legal, audit) with this label, and returns its id and its token. The token is returned this once; only its hash is kept.

Scope: admin. Hints: none.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `label` | string | yes | Who holds the key. |
| `scopes` | array | yes | The scopes it holds: write, read, delete, legal, audit. |
| `tenant` | string | yes | The tenant the key belongs to. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "label": "transcriber",
    "scopes": [
      "read"
    ],
    "tenant": "2066d6bdaa5181619164bbad61f2dd0e"
  },
  "name": "admin_key_create"
}
```

```json
{
  "id": "9c55ce1f43c2da8c",
  "token": "vcs_9c55ce1f43c2da8c_aShPX8ohqSmZ7iPf_8mmbctooWO5xQZSxSQeUc7T0IE"
}
```

### `admin_key_list`

Returns the tenant's keys, oldest first: each id, tenant, scopes, label between untrusted-vcon-data markers, and times made and revoked (RFC 3339); never a token. From cursor, within max_bytes, with next_cursor when more follow.

Scope: admin. Hints: read-only, idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `cursor` | string | no | The next_cursor a previous page gave. |
| `max_bytes` | integer | no | The most bytes the result may hold; the server's ceiling applies. |
| `tenant` | string | yes | The tenant whose keys to list. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "tenant": "2066d6bdaa5181619164bbad61f2dd0e"
  },
  "name": "admin_key_list"
}
```

```json
{
  "keys": [
    {
      "created_at": "2026-10-25T12:00:00Z",
      "id": "9c55ce1f43c2da8c",
      "label": "⟦untrusted-vcon-data⟧transcriber⟦/untrusted-vcon-data⟧",
      "revoked_at": null,
      "scopes": [
        "read"
      ],
      "tenant": "2066d6bdaa5181619164bbad61f2dd0e"
    }
  ],
  "next_cursor": null,
  "provenance": "Text between the untrusted-vcon-data markers was written by the vCon's producer or by an administrator, not by vcon-server.",
  "truncated": false
}
```

### `admin_key_revoke`

Revokes the key with this id, so its next request is refused, and returns the id; a key already revoked answers the same.

Scope: admin. Hints: destructive, idempotent.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `id` | string | yes | The key's id. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "id": "9c55ce1f43c2da8c"
  },
  "name": "admin_key_revoke"
}
```

```json
{
  "id": "9c55ce1f43c2da8c"
}
```

### `admin_audit_read`

Returns records of the admin log after the sequence number after (0, the default, is the start), oldest first, at most limit of them (1 to 10000, default 1000), within max_bytes: each record as audit export --admin writes it, and next_cursor, the seq of the last record returned.

Scope: admin. Hints: read-only.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `after` | integer | no | Records after this sequence number; 0 when left out. |
| `limit` | integer | no | How many records at most: 1 to 10000, 1000 when left out. |
| `max_bytes` | integer | no | The most bytes the result may hold; the server's ceiling applies. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "after": 0,
    "limit": 2
  },
  "name": "admin_audit_read"
}
```

```json
{
  "next_cursor": "2",
  "records": [
    {
      "action": "tenant_create",
      "actor": "cli",
      "at": "2026-10-25T12:00:00Z",
      "count": 1,
      "seq": 1,
      "status": 201,
      "target": "643b6670aea8bd662f1ab8ce2791c5d3",
      "version": null
    },
    {
      "action": "key_create",
      "actor": "cli",
      "at": "2026-10-25T12:00:00Z",
      "count": 1,
      "seq": 2,
      "status": 201,
      "target": "8dbf7e34c5d7be9c",
      "version": null
    }
  ],
  "truncated": false
}
```

### `admin_tenant_remove`

Removes a tenant that keeps no vCon, or with force one that does unless a vCon of it is under a legal hold, destroying its keys and data, and returns its id.

Scope: admin. Hints: destructive.

| Parameter | Type | Required | Meaning |
|---|---|---|---|
| `force` | boolean | no | Remove it even while it keeps vCons; a legal hold still refuses. |
| `id` | string | yes | The tenant's id. |

A call and its result, from a test server:

```json
{
  "arguments": {
    "id": "2066d6bdaa5181619164bbad61f2dd0e"
  },
  "name": "admin_tenant_remove"
}
```

```json
{
  "id": "2066d6bdaa5181619164bbad61f2dd0e"
}
```
