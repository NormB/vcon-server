# Security policy

## Reporting a vulnerability

**Do not open a public issue for a security problem.**

Report it privately through GitHub: on this repository, open the Security
tab and choose "Report a vulnerability". Include:

- what the problem is,
- how to reproduce it, or a proof of concept,
- what an attacker could do with it,
- a name or handle for credit, if you want one.

## Response

| Stage | Target |
|---|---|
| Acknowledgment | 48 hours |
| Initial assessment | 7 days |
| Fix for a critical problem | 30 days |
| Public disclosure | After the fix is released |

## Supported versions

Fixes are made on `main` and ship in the next release. Only the newest
release is supported: upgrade to it to receive a fix. Reports are taken
against the newest release and `main`.

## Scope

The server's guarantees are on the [security page](https://vcon-server.com/docs/security/).
A report that one of them does not hold is in scope, for example:

- **Tenant isolation**: a key, a request or a command that reads, changes,
  lists or learns of another tenant's vCons, keys or audit records.
- **Scopes**: a key doing what its scopes do not allow; an admin action
  without an operator key; a legal read, restore or hold without the legal
  scope.
- **Encryption at rest**: a vCon body, a data key, a tenant key or a catalog
  row written to disk unencrypted, or readable with a key that should not
  open it, including after a key rotation (`rekey`).
- **Keys and tokens leaking**: the master key, a tenant or data key, or an API
  token in a log, an audit record, a problem document, a core dump, swap, or
  any response other than the one that creates the token.
- **Retention and legal holds**: a vCon purged before its retention period
  ends or while held, or still readable after it was due and purged.
- **Audit log**: a request or command that leaves no audit record, or a way
  to change or remove a record.
- **Parsing**: a vCon, JWS, JWE or gzip body that crashes the server, makes it
  panic, or makes it use memory or time far beyond the configured limits.
- **TLS and HTTP**: a way past TLS, a request smuggled between two others, or
  a header that changes what the server does in a way it should not.

Out of scope: problems that need the master key file, root on the host or
write access to the data directory, since those already hold every vCon;
denial of service by sending more traffic than the configured limits allow;
reports from automated scanners without a demonstrated impact.

## Disclosure

A fix is released first. The advisory is then published on GitHub with
credit to the reporter, unless they ask not to be named.
