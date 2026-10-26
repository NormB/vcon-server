+++
title = "Client examples"
description = "Programs in curl, Python, TypeScript, Go and Rust that store, list, read and delete a vCon."
weight = 72
+++

The repository's `clients/` directory holds one small program per language.
Each one stores a new vCon, finds it in [the list](@/docs/http-api.md#listing),
reads it back and checks the bytes are the ones it sent, deletes it, and checks
that the vCon then answers [`gone`](@/problems/gone.md) (410). CI runs every one
of them against a live server on each push, and checks that each fails with a
key the server does not know.

| Language | Program | Needs |
|---|---|---|
| curl | [`clients/curl/vcon-client.sh`](https://github.com/NormB/vcon-server/blob/main/clients/curl/vcon-client.sh) | bash, curl, jq |
| Python | [`clients/python/vcon_client.py`](https://github.com/NormB/vcon-server/blob/main/clients/python/vcon_client.py) | Python 3, standard library only |
| TypeScript | [`clients/typescript/vcon-client.ts`](https://github.com/NormB/vcon-server/blob/main/clients/typescript/vcon-client.ts) | Node.js 22.18 or later |
| Go | [`clients/go/vcon-client/main.go`](https://github.com/NormB/vcon-server/blob/main/clients/go/vcon-client/main.go) | Go, standard library only |
| Rust | [`clients/rust/src/main.rs`](https://github.com/NormB/vcon-server/blob/main/clients/rust/src/main.rs) | reqwest's blocking client |

## Run one

Each reads the same environment variables:

- `VCON_URL`: the server, such as `https://vcon.example.com`.
- `VCON_KEY`: a tenant key with the `write`, `read` and `delete` scopes (see
  [tenants and keys](@/docs/tenants-and-keys.md)).
- `VCON_CA`: the PEM file of the authority that signed the server's
  certificate, when your system does not trust it. Node.js reads
  `NODE_EXTRA_CA_CERTS` instead.

From the repository root:

```sh
export VCON_URL=https://vcon.example.com VCON_KEY=vcs_...
bash clients/curl/vcon-client.sh
python3 clients/python/vcon_client.py
node clients/typescript/vcon-client.ts
go -C clients/go run ./vcon-client
cargo run -p vcon-client
```

Each prints four lines and exits 0:

```text
stored 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f
listed 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f
read 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f 123 bytes
deleted 6f1c2d3e-4b5a-4c6d-8e7f-0a1b2c3d4e5f
```

Any refusal stops it with the route, the status and the problem type, and a
non-zero exit.

## Run them all against a test server

`scripts/run-clients.sh` makes a data directory, a master key, a certificate
authority and a server certificate, a tenant and a key, starts a server on a
free loopback port, and runs every client, as CI does:

```sh
cargo build -p vcon-server -p vcon-client
scripts/run-clients.sh target/debug/vcon-server target/debug/vcon-client
```

It needs curl, jq, openssl, Python 3, Go and Node.js on `PATH`, and a memory
lock limit large enough for the server (see
[getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory)).
The TypeScript client is type-checked with
`npm ci && npx tsc --noEmit` in `clients/typescript`.

To generate a client in another language, start from the
[OpenAPI description](@/docs/http-api.md#basics).
