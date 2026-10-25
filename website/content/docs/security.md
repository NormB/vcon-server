+++
title = "Security"
description = "The key hierarchy, what is encrypted with what, how the process protects keys in memory, and what an attacker gets from each thing they might steal."
weight = 65
+++

## Keys

```text
master key (32-byte file, never stored by the server)
├── wraps each tenant key            AES-256-GCM, bound to the tenant id
│   ├── wraps each version's data key AES-256-GCM, bound to tenant, uuid and version
│   │   └── seals that version's body AES-256-GCM, bound to tenant, uuid and version
│   └── derives the tenant catalog key HKDF-SHA256
└── derives the server catalog key     HKDF-SHA256 → server.sqlite
```

- Every key is 256 bits from the system's random source.
- Every encryption uses AES-256-GCM with a random 96-bit nonce, and binds its
  context in the associated data: a sealed body moved to another tenant,
  uuid or version does not open.
- A wrapping key is retired before it can wrap 2^32 keys, the limit NIST SP
  800-38D sets for random nonces: writes are refused with
  [`limit-reached`](@/problems/limit-reached.md) (or
  [`master-key-limit`](@/problems/master-key-limit.md)) until the key is
  rotated.
- Catalogs are SQLCipher 4 databases keyed with HKDF-SHA256 of their parent
  key (no salt, info `vcon-server/v1 catalog`), with `secure_delete` on.
- The cryptography is AWS-LC, through `aws-lc-rs`.

## In memory

- At start the process locks all its memory (`mlockall`), so no key or body
  is ever written to swap, and refuses to start if it cannot.
- It turns core dumps off (`RLIMIT_CORE` 0 and not dumpable), so a crash
  writes no memory to disk.
- Each key lives in its own locked page, excluded from core dumps, and is
  zeroed when dropped.

## On the wire

TLS only, via rustls with its safe default protocol versions (TLS 1.2 and
1.3), HTTP/2 and HTTP/1.1. Tokens are compared by their SHA-256; the server
never stores a token.

## What each theft gives

| Stolen | Gives |
|---|---|
| The data directory alone | nothing readable: sealed files and encrypted catalogs |
| A backup of the data directory | the same, until someone also has the master key of that time |
| The master key alone | nothing without the data |
| Both | everything they cover, as of when they were taken |
| A tenant key | its scopes, for its tenant, until revoked |
| An operator key | tenant and key management over HTTP; never any vCon, and never a new operator key |
| The request log | method, route template, status and timing only |

[Key rotation](@/docs/operations.md#key-rotation) shortens "as of when they
were taken": after it, old copies of the data cannot be opened with the new
keys, and the old ones are destroyed.

## Reporting a vulnerability

Please report it privately through
[GitHub's security advisories](https://github.com/NormB/vcon-server/security/advisories/new)
for the repository, not in a public issue.
