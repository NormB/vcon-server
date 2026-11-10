+++
title = "Client examples"
description = "Programs in curl, Python, TypeScript, Go and Rust that work through the HTTP API and MCP: store, gzip, list with a cursor, replace, legal hold, audit, admin and MCP calls, and delete."
weight = 51

[extra]
group = "reference"
+++

The repository's `clients/` directory holds one small program per language.
All five take the same steps, in the same order, and print the same lines.
CI runs every one of them against a live server on each push, and checks that
each fails with a key the server does not know.

| Language | Program | Needs |
|---|---|---|
| curl | [`clients/curl/vcon-client.sh`](https://github.com/NormB/vcon-server/blob/main/clients/curl/vcon-client.sh) | bash, curl, jq, Python 3 (to gzip a body) |
| Python | [`clients/python/vcon_client.py`](https://github.com/NormB/vcon-server/blob/main/clients/python/vcon_client.py) | Python 3, standard library only |
| TypeScript | [`clients/typescript/vcon-client.ts`](https://github.com/NormB/vcon-server/blob/main/clients/typescript/vcon-client.ts) | Node.js 22.18 or later, built-in modules only |
| Go | [`clients/go/vcon-client/main.go`](https://github.com/NormB/vcon-server/blob/main/clients/go/vcon-client/main.go) | Go, standard library only |
| Rust | [`clients/rust/src/main.rs`](https://github.com/NormB/vcon-server/blob/main/clients/rust/src/main.rs) | reqwest's blocking client, serde_json, flate2 |

## What each one does

1. **Store.** `POST /vcons` with a new vCon, `Content-Type: application/vcon`:
   `201`, with the version in `ETag`.
2. **List.** `GET /vcons?limit=10`: the new vCon is in the page. Its
   `stored_at` is kept for step 5.
3. **Read.** `GET /vcons/{uuid}`: the body is the bytes that were sent.
4. **Gzip.** A second vCon, gzipped, sent with
   `Content-Type: application/vcon+gzip`. Reading it back answers the same
   gzipped bytes, with `Content-Type: application/vcon+gzip`. See
   [storing vCons](@/docs/storing-vcons.md#stored-exactly-as-received).
5. **Page.** `GET /vcons?limit=1&since=<stored_at>`, then again with
   `cursor` set to each answer's `next`, until `next` is `null`. Both vCons
   must be listed exactly once. See [listing](@/docs/http-api.md#listing).
6. **Replace.** `PUT /vcons/{uuid}` with `If-Match` naming the version from
   step 1: `200`, with a new `ETag`. The same request again, whose
   `If-Match` now names an old version, is refused with
   [`precondition-failed`](@/problems/precondition-failed.md) (412). See
   [versions](@/docs/storing-vcons.md#versions).
7. **Legal hold.** `PUT` and then `DELETE` `/legal/vcons/{uuid}/hold` with
   the legal key: `204` each. See
   [retention and legal holds](@/docs/retention-and-holds.md#legal-holds).
8. **Audit.** `GET /key` gives the id and tenant of `VCON_KEY`. Then
   `GET /audit` with the audit key, page after page, until it finds the
   `create` record of step 1, made by that key id, with status `201`. See
   [audit logs](@/docs/audit-logs.md).
9. **Admin.** `GET /admin/tenants` with the operator key: the list holds
   the tenant of `VCON_KEY`. See [admin](@/docs/http-api.md#admin).
10. **MCP.** `POST /mcp` with `VCON_KEY`: `initialize` (protocol version
    2025-06-18), the `notifications/initialized` notification, `tools/list`,
    and `tools/call` of `vcon_get` for the replaced vCon. The answer must name
    the version from step 6 and hold the replaced body between the
    untrusted-data markers. See [MCP](@/docs/mcp.md).
11. **Delete.** `DELETE /vcons/{uuid}` for both vCons: `204` each, and each
    then answers [`gone`](@/problems/gone.md) (410).

## Run one

Each reads the same environment variables. Each key is a separate key with
only the scopes its steps need (see
[tenants and keys](@/docs/tenants-and-keys.md)):

- `VCON_URL`: the server, such as `https://vcon.example.com`.
- `VCON_KEY`: a tenant key with the `write`, `read` and `delete` scopes, for
  steps 1 to 6, 8, 10 and 11.
- `VCON_LEGAL_KEY`: a key of the same tenant with the `legal` scope, for
  step 7.
- `VCON_AUDIT_KEY`: a key of the same tenant with the `audit` scope, for
  step 8.
- `VCON_OPERATOR_KEY`: an operator key, for step 9. Only
  `vcon-server key create --operator` on the server's host makes one (see the
  [command line](@/docs/command-line.md#keys)).
- `VCON_CA`: the PEM file of the authority that signed the server's
  certificate, when your system does not trust it, by its absolute path.
  For the self-signed certificate of [getting started](@/docs/getting-started.md),
  that is `/usr/local/share/vcon-server/ca.pem`. Node.js reads
  `NODE_EXTRA_CA_CERTS` instead.

Make the keys on the server's host, with the tenant's id in `$TENANT`:

```sh
CONFIG=/etc/vcon-server/vcon-server.toml
sudo -u vcon vcon-server key create --tenant "$TENANT" --scope write --scope read --scope delete \
  --label client-example --config $CONFIG
sudo -u vcon vcon-server key create --tenant "$TENANT" --scope legal --label client-example-legal --config $CONFIG
sudo -u vcon vcon-server key create --tenant "$TENANT" --scope audit --label client-example-audit --config $CONFIG
sudo -u vcon vcon-server key create --operator --label client-example-operator --config $CONFIG
```

Each prints `{"id", "token"}`; the token is the key. From the repository root:

```sh
export VCON_URL=https://vcon.example.com VCON_KEY=vcs_... VCON_LEGAL_KEY=vcs_... \
  VCON_AUDIT_KEY=vcs_... VCON_OPERATOR_KEY=vcs_...
bash clients/curl/vcon-client.sh
python3 clients/python/vcon_client.py
node clients/typescript/vcon-client.ts
go -C clients/go run ./vcon-client
cargo run -p vcon-client
```

Each prints one line a step and exits 0. The first vCon's uuid, the gzipped
one's, the id of `VCON_KEY` and its tenant's id differ from run to run:

```text
stored 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f
listed 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f
read 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f 123 bytes
gzip 0b6a5c4d-3e2f-4a1b-9c8d-7e6f5a4b3c2d read back as application/vcon+gzip
paged 2 vCons in 2 pages of 1
replaced 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f, a stale If-Match refused with 412
held 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f and released
audit create 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f by 0923ad90eb9c4ba8
admin tenant 5d5f464cc48c93ed53b923eb1d76ac9a listed
mcp vcon_get 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f from vcon-server, 5 tools listed
deleted 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f 0b6a5c4d-3e2f-4a1b-9c8d-7e6f5a4b3c2d
```

The `paged` line counts every live vCon stored since the first one, so on a
server that other producers write to at the same time it can be larger. The
`mcp` line counts the tools the key's scopes allow: five for `write`, `read`
and `delete`.

Any refusal stops it with the route, the status and the problem type, and a
non-zero exit.

## Run them all against a test server

`scripts/run-clients.sh` makes a data directory, a master key, a certificate
authority and a server certificate, a tenant and the four keys above, starts a
server on a free loopback port, and runs every client, as CI does. It checks
each client's output line by line, and that each client exits non-zero, naming
`POST /vcons` and `401`, with a key the server does not know:

```sh
cargo build -p vcon-server -p vcon-client
scripts/run-clients.sh target/debug/vcon-server target/debug/vcon-client
```

`RUN_CLIENTS`, a space-separated list of names (`curl python go typescript
rust`), runs only those; with `curl` alone it needs only curl, jq, openssl and
Python 3, and the Rust client binary may be given as `-`.

It needs curl, jq, openssl, Python 3, Go and Node.js on `PATH`, and a memory
lock limit large enough for the server (see
[getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory)).
The TypeScript client is type-checked with
`npm ci && npx tsc --noEmit` in `clients/typescript`.

To generate a client in another language, start from the
[OpenAPI description](@/docs/http-api.md#basics).
