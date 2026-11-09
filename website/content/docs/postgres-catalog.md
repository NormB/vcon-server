+++
title = "Keep the catalogs in PostgreSQL"
description = "Preview: reach a PostgreSQL server over TLS for the catalogs: the database's side, the role, TLS and client certificates, the configuration, the secret files, and what each refusal at start means. This release connects, checks the server and stops."
weight = 13
+++

**This page is a preview.** The PostgreSQL catalog is being built. In this
release, a server configured for it reads its secret files, connects to
PostgreSQL over TLS, checks the server and the role, and then stops with
exit code 1: the catalogs and the registry are not kept in PostgreSQL yet,
and stay in the data directory as SQLCipher files. This page covers what
that start does, so a database can be set up and checked ahead of the
catalog itself. CI does not run this page's commands yet; the messages
below were printed by the server built from this release's source.

The release binaries and `cargo install vcon-server` do not have the
`postgres` cargo feature. Build from source with it, as
[install](@/docs/install.md#build-from-source) shows, adding the feature:

```sh
cargo build --release --locked -p vcon-server --features postgres
```

## What was tested, and what was not

- **PostgreSQL 16.15 and 18.6.** CI runs the connector's and the pool's tests
  against each on every change, in containers with TLS: a primary with one
  test role for each way of logging in, a hot standby of it, and a server
  with ssl off. PostgreSQL 16 is the oldest release supported.
- **Not tested:** PostgreSQL 15 and earlier, managed services (Amazon RDS,
  Cloud SQL, Azure), connection poolers, and a server certificate with an
  Ed25519 key (see [TLS](#tls)).

## Before you start

- PostgreSQL 16 or later, with `ssl = on` and a server certificate whose
  subject alternative names hold every host name or address the
  configuration gives in `hosts`.
- The sealed objects in an S3-compatible bucket: `storage.objects = "s3"`,
  as [keep vCons in an S3 bucket](@/docs/s3-storage.md) sets up. A
  catalog in PostgreSQL is refused with objects in the data directory,
  since every server that shares the catalog must be able to read the
  objects it names.
- On the database host, `pg_hba.conf` lines that let the server's role in
  only over TLS (`hostssl`), and only with SCRAM-SHA-256 or a client
  certificate.

## On the database host

Store passwords as SCRAM in `postgresql.conf` (the default since
PostgreSQL 14):

```text
ssl = on
ssl_cert_file = 'server.crt'
ssl_key_file = 'server.key'
password_encryption = 'scram-sha-256'
```

Make a role that may log in, is not a superuser and does not have
`BYPASSRLS`, and a database for it, as a superuser in `psql`:

```sql
CREATE ROLE vcon_app LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD 'the role password';
CREATE DATABASE vcon;
```

Then in `pg_hba.conf`, before any line that would match it with another
method:

```text
hostssl vcon vcon_app 0.0.0.0/0 scram-sha-256
hostssl vcon vcon_app ::/0 scram-sha-256
```

Narrow `0.0.0.0/0` and `::/0` to the servers' addresses. Reload with
`SELECT pg_reload_conf();`.

## TLS

Every connection is TLS. There is no setting that turns TLS off or stops
the checks:

- The server's certificate chain must lead to an authority in
  `catalog.postgres.ca_file`, or, without that key, to one of the system's.
- The certificate must hold the host name or IP address the connection was
  made to, as written in `hosts`.
- A server that answers without TLS is refused, and so is a Unix socket
  host, which has none.

With a password, the server accepts only SCRAM-SHA-256 with channel binding
(`SCRAM-SHA-256-PLUS`), which binds the password exchange to the TLS
connection, so a machine in the middle with another certificate cannot
relay it. A server that asks for `md5`, for a plain `password`, or for
nothing (`trust`) is refused before the password is sent. The channel
binding is a hash of the server's certificate, which tokio-postgres-rustls
0.14.0 computes for RSA and ECDSA certificates and, by its source, not for
Ed25519 ones; with an Ed25519 server certificate every password login is
refused (not tested here).

### Client certificates

A client certificate can replace the password, or go with it. Its subject's
common name (CN) is the role name. In `pg_hba.conf`:

```text
hostssl vcon vcon_app 0.0.0.0/0 cert
```

for the certificate alone, or, for the certificate and the password:

```text
hostssl vcon vcon_app 0.0.0.0/0 scram-sha-256 clientcert=verify-full
```

The server's `ssl_ca_file` must hold the authority that signed the client
certificate. With a certificate and no password, the server's answer to a
`cert` line and to a `trust` line is the same, so vcon-server cannot tell
them apart: check `pg_hba.conf` for a `trust` line that matches the role.

## The secret files

The password and the client certificate's key are each in a file that
gives other users no access: no bit of `0007` set. The group may read it,
for a file owned by root and readable by the service's group. Install the
password on one line:

```sh
printf %s "$PG_PASSWORD" | sudo install -m 0600 -o vcon -g vcon /dev/stdin /etc/vcon-server/pg.password
sudo install -m 0600 -o vcon -g vcon pg-client.key /etc/vcon-server/pg-client.key
```

The server reads every file `[catalog.postgres]` names once, when it
starts, before it connects, and never again: each new connection is made
from what it read. A message never holds the password or the key.

## The configuration

Add the `[catalog]` table to the configuration, beside the `[storage]`
table of the bucket. Every key, with its default, is on the
[configuration page](@/docs/configuration.md#catalog-in-postgresql).

```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"

[storage]
objects = "s3"

[storage.s3]
endpoint = "https://objects.example.com:9000"
region = "us-east-1"
bucket = "vcon"
path_style = true
access_key_id = "vcon-server"
secret_access_key_file = "/etc/vcon-server/s3.secret"

[catalog]
backend = "postgres"

[catalog.postgres]
hosts = ["db1.example.com", "db2.example.com"]
dbname = "vcon"
user = "vcon_app"
ca_file = "/etc/vcon-server/pg-ca.pem"
password_file = "/etc/vcon-server/pg.password"
client_cert_file = "/etc/vcon-server/pg-client.pem"
client_key_file = "/etc/vcon-server/pg-client.key"
```

- **Hosts.** Each is a DNS name in ASCII, an internationalized name in its
  `xn--` form, or an IP address (an IPv6 address without brackets). They
  are tried in order, each for at most `connect_timeout` seconds. A standby
  is passed over for the next host.
- **Names.** `dbname` and `user` may hold any character but NUL, up to 63
  bytes: PostgreSQL would cut a longer name and name another database or
  role.
- **Timeouts.** `statement_timeout` and `lock_timeout` are set on every
  connection, in whole seconds, up to 2147483, the longest PostgreSQL
  holds.
- **The pool.** At most `pool` connections are open at once. A request
  waits at most `connect_timeout` seconds for a free one.

## Check it

Start the server with the configuration:

```sh
sudo -u vcon vcon-server serve --config /etc/vcon-server/vcon-server.toml
```

In this release it reaches the first host that passes the checks, says
which, and stops with exit code 1:

```text
vcon-server: the storage cannot be used: catalog.postgres: connected to PostgreSQL 16.15 at db1.example.com:5432 as vcon_app over TLS; this release does not keep the catalogs in PostgreSQL yet (a preview), so it stops here: set catalog.backend = "local" to serve
```

Every command (`check`, `tenant create` and the others) does the same.
Set `catalog.backend = "local"`, or remove the `[catalog]` table, to serve
from the data directory again.

## When it does not start

The server stops with an error that names the setting or the server, and
exits 1. Each message below was printed by the server; host names, ports,
role names and paths are this page's. When several hosts are given, the message names
each with its reason, separated by `;`.

### The build has no PostgreSQL support

```text
vcon-server: the storage cannot be used: catalog.backend is "postgres", but this build has no PostgreSQL support (cargo feature postgres)
```

Build with `--features postgres`, as the top of this page shows.

### The catalog table does not hold together

The configuration is checked before anything is reached:

```text
vcon-server: configuration: catalog.backend = "postgres" needs storage.objects = "s3": a catalog in PostgreSQL names objects that every server must be able to read
vcon-server: configuration: catalog.postgres.hosts: "/var/run/postgresql" is a Unix socket, which carries no TLS; the catalog is reached over TCP with TLS only
vcon-server: configuration: set catalog.postgres.password_file, catalog.postgres.client_cert_file, or both: a server that asks for neither is not accepted
```

The first needs the bucket of [before you start](#before-you-start). For a
Unix socket, give the server's host name or address. The third needs a
way to log in. A key the server does not know, such as `sslmode`, is a
TOML error that names it.

### A secret file cannot be used

```text
vcon-server: the storage cannot be used: catalog.postgres.password_file /etc/vcon-server/pg.password: No such file or directory (os error 2)
vcon-server: the storage cannot be used: catalog.postgres.password_file /etc/vcon-server/pg.password: other users may use the file (mode 0644); make it 0600, or 0640 for a group
```

Install the file again, as [above](#the-secret-files). The same messages
name `catalog.postgres.client_key_file` for the key.

### The server's certificate is not trusted

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: error performing TLS handshake: invalid peer certificate: UnknownIssuer: the server's certificate is not signed by an authority in catalog.postgres.ca_file (or the system's, without it)
```

`ca_file` must hold the authority that signed the server's certificate.
For a certificate that does not hold the host name it was reached by, the
reason ends `the server's certificate does not hold the host name it was
reached by` (from the tests; give the name the certificate holds, or a new
certificate).

### The server does not offer TLS

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: error performing TLS handshake: server does not support TLS: turn ssl on in the server's postgresql.conf
```

### The server asks for md5, a plain password or nothing

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: authentication error: server did not use channel binding: with a password, the only login accepted is SCRAM-SHA-256 with channel binding; give the role a scram-sha-256 line under hostssl in pg_hba.conf (md5, password and trust are refused)
```

The same message is printed for an `md5`, `password` or `trust` line in
`pg_hba.conf`, and for a server certificate with no channel-binding hash.
The password was not sent. Give the role a `scram-sha-256` line, and, if
the role's password was set while `password_encryption` was `md5`, set it
again.

### The password is wrong

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the server refused: password authentication failed for user "vcon_app" (SQLSTATE 28P01)
```

### The server refused the client certificate

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: error communicating with the server: received fatal alert: UnknownCA: the server refused the client certificate (catalog.postgres.client_cert_file): the server's ssl_ca_file must hold the authority that signed it
```

### The server asks for a password, and none is configured

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: invalid configuration: password missing: the server asked for a password, and catalog.postgres has no password_file
```

The role's `pg_hba.conf` line asks for SCRAM; set `password_file`, or give
the role a `cert` line.

### The role is a superuser or has BYPASSRLS

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the role "vcon_app" is a superuser, which passes row-level security; log in as a role that is not
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the role "vcon_app" has BYPASSRLS, which passes row-level security; log in as a role without it
```

The catalog will keep each tenant's rows apart with row-level security,
which such a role passes. `ALTER ROLE vcon_app NOSUPERUSER NOBYPASSRLS;`

### The server is a standby or read-only

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db2.example.com:5432: the server is a standby (pg_is_in_recovery() is true); name the primary
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the server takes only read-only transactions (transaction_read_only is on); name a primary that takes writes
```

A standby is in recovery and takes no writes. The second is a server, or a
role or database, with `default_transaction_read_only = on`. List the
primary in `hosts`; a standby before it is passed over.

### The server cannot be reached

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: error connecting to server: Connection refused (os error 111)
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: error connecting to server: connection timed out
```

Nothing listens on the port, or no answer came within `connect_timeout`.
Check `hosts`, `port`, the server's `listen_addresses` and the firewall. A
server that takes the connection and then never answers is given up after
`connect_timeout` too, with `no answer within
catalog.postgres.connect_timeout`.
