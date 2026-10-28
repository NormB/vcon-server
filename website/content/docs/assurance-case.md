+++
title = "Assurance case"
description = "Why vcon-server's security requirements are met: the threat model, the trust boundaries, how the design principles are applied, and how the common weaknesses are countered."
weight = 66
+++

The claim: a vCon stored in vcon-server is readable only through a key
allowed to read it, and no copy of the data at rest, alone, gives anyone its
contents. This page sets out why that holds. The mechanisms are described on
the [security page](@/docs/security.md); this page argues from them.

## Threat model

What is protected:

- the vCons each tenant stores, and their earlier versions,
- the keys that open them: the master key, each tenant key and each data key,
- the API tokens that grant access, and the audit records of that access.

Who the server defends against:

- a client on the network without a valid key, or with a key used beyond its
  scopes;
- a tenant reaching for another tenant's data;
- an operator with an operator key, who manages tenants and keys but has no
  reason to read a vCon;
- someone who obtains the data directory, a backup of it, or the request log,
  without the master key;
- crafted input: a malformed vCon, an oversized or deeply nested body, or a
  hostile request.

Out of scope: an attacker who holds both the data and the master key at the
same time, or who controls the host while the server runs. The table in
[What each theft gives](@/docs/security.md#what-each-theft-gives) says what
each stolen part gives.

## Trust boundaries

| Boundary | What crosses it | What guards it |
|---|---|---|
| Network to server | every HTTP request | TLS only (rustls, TLS 1.2 and 1.3); a bearer key on every request, checked against its scopes |
| Tenant to tenant | nothing | each key belongs to one tenant; each tenant has its own tenant key and catalog |
| Operator key to vCons | nothing | an operator key holds only `admin` and can never reach a vCon |
| Server to disk | sealed bodies, wrapped keys, catalogs | AES-256-GCM bound to tenant, uuid and version; SQLCipher catalogs; the master key is never stored by the server |
| Server to swap and core dumps | nothing | all memory locked (`mlockall`), core dumps off, each key in its own locked page |
| Server to browser | the console's pages and the data it shows | a content-security policy allowing only the console's own script; data shown as text |
| Host operator to data | commands run as the service account | the master key file (mode 0400) and the data directory (mode 0700), owned by that account, as [Getting started](@/docs/getting-started.md) sets them up |

## Secure design principles

- **Deny by default, complete mediation:** every request needs a key, and
  every route checks the key's scopes; a request without the scope is
  refused with [`forbidden`](@/problems/forbidden.md) and recorded.
- **Least privilege:** five tenant scopes (`write`, `read`, `delete`,
  `legal`, `audit`), each given only where needed; an operator key cannot
  read a vCon or make another operator key
  ([Tenants and keys](@/docs/tenants-and-keys.md)).
- **Separation:** each tenant's data is sealed under its own keys, and a
  sealed body moved to another tenant, uuid or version does not open.
- **Economy of mechanism:** one cryptographic library (AWS-LC, through
  `aws-lc-rs`) and one construction for every encryption (AES-256-GCM with a
  random nonce and the context bound in).
- **Fail safe:** the server refuses to start if it cannot lock its memory,
  and refuses writes once a wrapping key nears the NIST nonce limit, until
  the key is rotated.
- **Accountability:** every request is recorded in the audit log
  ([Audit logs](@/docs/audit-logs.md)); the request log holds only method,
  route template, status and timing.

## Common weaknesses countered

| Weakness | Countered by |
|---|---|
| Missing or broken authentication (CWE-287, CWE-306) | a bearer key on every request; tokens stored only as SHA-256 and compared by it |
| Missing authorization (CWE-862, CWE-863) | scopes checked on every route; tenant keys cannot hold `admin`; operator keys cannot reach vCons |
| Cleartext storage or transmission (CWE-312, CWE-319) | sealed bodies and encrypted catalogs on disk; TLS only on the wire |
| Weak or misused cryptography (CWE-327, CWE-330) | AES-256-GCM, HKDF-SHA256 and SHA-256 from AWS-LC; keys and nonces from its random generator; no MD5, SHA-1, RC4 or DES |
| Sensitive data in logs (CWE-532) | an audit record holds no vCon content, no token and no client address; the request log holds the route template, never the path ([Audit logs](@/docs/audit-logs.md)) |
| SQL injection (CWE-89) | catalog statements bind their values as parameters; the one statement built as text is a PRAGMA given a version number the catalog holds |
| Cross-site scripting (CWE-79) | the console shows server data as text, under a content-security policy with no inline code |
| Memory-safety errors (CWE-119, CWE-416) | Rust; the unsafe code is confined to system calls (memory locking, core-dump limits, file locking, an access check) and SQLCipher's key calls, and the tests run under AddressSanitizer, the locked-memory code under Miri |
| Uncontrolled resource use (CWE-400, CWE-674) | request bodies bounded by `limits.body`, gzip inflation by `limits.inflated`, JSON nesting and signed layers to 64 deep |
| Malformed input (CWE-20) | the vCon reader refuses anything outside the format, with a problem document naming why |

## Evidence

- The tests, run on every push and pull request
  ([Development](@/docs/development.md)), with the test for each change
  written first.
- AddressSanitizer over the tests and Miri over the locked-memory code
  ([`sanitizers.yml`](https://github.com/NormB/vcon-server/blob/main/.github/workflows/sanitizers.yml)).
- Twelve fuzz targets, over the vCon reader, the request and query parsers,
  the configuration, tokens, ids, retention and the sealed format, in CI and in
  ClusterFuzzLite
  ([`fuzz/fuzz_targets`](https://github.com/NormB/vcon-server/tree/main/fuzz/fuzz_targets)).
- CodeQL's security queries and clippy with warnings as errors, on every
  push and pull request.
- The [security policy](https://github.com/NormB/vcon-server/blob/main/SECURITY.md)
  for reporting what this page has missed.
