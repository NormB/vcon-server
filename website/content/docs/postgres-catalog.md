+++
title = "Keep the catalogs in PostgreSQL"
description = "Preview: reach a PostgreSQL server over TLS for the catalogs: the database's side, the roles, the schema and `migrate`, TLS and client certificates, the configuration, the secret files, and what each refusal at start means. This release migrates the schema, connects, checks the server and stops."
weight = 30

[extra]
group = "how-to"
+++

**This page is a preview.** The PostgreSQL catalog is being built. In this
release, `vcon-server migrate` makes the schema in the database, and a
server configured for it reads its secret files, connects to PostgreSQL
over TLS, checks the server, the roles and the schema's version, and then
stops with exit code 1: the catalogs and the registry are not kept in
PostgreSQL yet, and stay in the data directory as SQLCipher files. This
page covers what those two do, so a database can be set up and checked
ahead of the catalog itself.

CI runs every command on this page on each change, after
[getting started](@/docs/getting-started.md) and
[keep vCons in an S3 bucket](@/docs/s3-storage.md), on Debian 13 with the
PostgreSQL 17 it packages, the database on the server's own host, and a
server built with the `postgres` feature. The messages under
[when it does not start](#when-it-does-not-start) that no step here prints
were printed by the server built from this release's source.

The release binaries and `cargo install vcon-server` do not have the
`postgres` cargo feature. Build from source with it, as
[install](@/docs/install.md#build-from-source) shows, adding the feature,
and install that binary:

<!-- docs-test: skip the run is given a server built with the postgres feature -->
```sh
cargo build --release --locked -p vcon-server --features postgres
sudo install -m 0755 target/release/vcon-server /usr/local/bin/vcon-server
```

## What was tested, and what was not

- **PostgreSQL 16.15 and 18.6.** CI runs the connector's and the pool's tests
  against each on every change, in containers with TLS: a primary with one
  test role for each way of logging in, a hot standby of it, and a server
  with ssl off. PostgreSQL 16 is the oldest release supported.
- **PostgreSQL 17, as Debian 13 packages it.** CI runs this page's steps
  against it: one server, logged in to with a password.
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
- A superuser on the database host, to make the roles and the database.
  The steps below run on one host that holds PostgreSQL and vcon-server
  both; with two hosts, each heading says which host its steps are for.
- One shell for the steps: later steps use variables that earlier steps
  set, and `$MONITOR` of [getting started](@/docs/getting-started.md).

## On the database host

Install PostgreSQL 16 or later. On Debian 13, as packaged (PostgreSQL 17):

<!-- docs-test: only apt -->
```sh
sudo apt-get install -y -qq postgresql >/dev/null
```

The package starts the server on a host. Where it does not, as in a
container whose image stops packages from starting services, start it; the
second command waits until it answers:

```sh
sudo systemctl start postgresql
timeout 30 sh -c 'until sudo -u postgres psql -Atc "SELECT 1" 2>/dev/null; do sleep 1; done'
```

```text
1
```

Find where this server keeps its configuration and its data:

```sh
PGETC=$(dirname "$(sudo -u postgres psql -Atc 'SHOW config_file')")
PGDATA=$(sudo -u postgres psql -Atc 'SHOW data_directory')
sudo -u postgres psql -Atc 'SHOW server_version_num' | cut -c1-2
```

```text
17
```

Give the server a certificate that holds the name vcon-server reaches it
by, here `db1.example.com`. With a certificate from your certificate
authority, install it and its key as `server.crt` and `server.key` in the
data directory. To try it without one, make a self-signed certificate; the
`-addext` options mark it as a TLS server's and not an authority's, which
vcon-server requires:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 365 \
  -subj /CN=db1.example.com -addext subjectAltName=DNS:db1.example.com \
  -addext basicConstraints=critical,CA:FALSE -addext keyUsage=critical,digitalSignature -addext extendedKeyUsage=serverAuth \
  -keyout pg-server.key -out pg-server.pem 2>/dev/null
sudo install -m 0600 -o postgres -g postgres pg-server.key "$PGDATA/server.key"
sudo install -m 0644 -o postgres -g postgres pg-server.pem "$PGDATA/server.crt"
```

Turn TLS on with that certificate, and store passwords as SCRAM (the
default since PostgreSQL 14). Debian's `postgresql.conf` reads every file in
`conf.d`; elsewhere, put the four lines in `postgresql.conf` itself. Then
restart PostgreSQL:

```sh
printf '%s\n' "ssl = on" "ssl_cert_file = 'server.crt'" "ssl_key_file = 'server.key'" "password_encryption = 'scram-sha-256'" \
  | sudo tee "$PGETC/conf.d/vcon.conf" >/dev/null
sudo systemctl restart postgresql
timeout 30 sh -c 'until sudo -u postgres psql -Atc "SHOW ssl" 2>/dev/null; do sleep 1; done'
```

```text
on
```

Make three roles and a database, as a superuser. Each login role gets a
random password, kept in a shell variable until it is installed in a file:

```sh
PG_PASSWORD=$(openssl rand -hex 24)
PG_MIGRATE_PASSWORD=$(openssl rand -hex 24)
sudo -u postgres psql -q -v ON_ERROR_STOP=1 \
  -c "CREATE ROLE vcon_owner NOLOGIN NOSUPERUSER NOBYPASSRLS" \
  -c "CREATE ROLE vcon_app LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD '$PG_PASSWORD'" \
  -c "CREATE ROLE vcon_migrate LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD '$PG_MIGRATE_PASSWORD'" \
  -c "GRANT vcon_owner TO vcon_migrate" \
  -c "CREATE DATABASE vcon OWNER vcon_owner"
sudo -u postgres psql -Atc "SELECT rolname FROM pg_roles WHERE rolname LIKE 'vcon\_%' ORDER BY 1"
```

```text
vcon_app
vcon_migrate
vcon_owner
```

- `vcon_owner` owns the schema; nobody logs in as it.
- `vcon_app` is the role the server logs in as. It reads and writes rows,
  never the schema, and a query from it with no tenant set is refused by
  row-level security.
- `vcon_migrate` is the role `vcon-server migrate` logs in as. Keep its
  password apart from the server's: a server that logs in with a role that
  owns the tables, or can act as `vcon_owner`, is refused. If your server
  must log in as another role name, make that role a member of `vcon_app`.

Then let the two roles in, over TLS and with SCRAM only. The lines go at
the top of `pg_hba.conf`, before any line that would match the roles with
another method:

```sh
sudo sed -i '1i hostssl vcon vcon_app 0.0.0.0/0 scram-sha-256\nhostssl vcon vcon_app ::/0 scram-sha-256\nhostssl vcon vcon_migrate 0.0.0.0/0 scram-sha-256\nhostssl vcon vcon_migrate ::/0 scram-sha-256' "$PGETC/pg_hba.conf"
sudo -u postgres psql -Atc 'SELECT pg_reload_conf()'
sudo -u postgres psql -Atc "SELECT type, user_name, auth_method FROM pg_hba_file_rules WHERE database = '{vcon}' AND error IS NULL"
```

```text
t
hostssl|{vcon_app}|scram-sha-256
hostssl|{vcon_app}|scram-sha-256
hostssl|{vcon_migrate}|scram-sha-256
hostssl|{vcon_migrate}|scram-sha-256
```

Narrow `0.0.0.0/0` and `::/0` to the servers' addresses. With vcon-server
on another host, also set `listen_addresses` in `postgresql.conf` to an
address that host reaches, and restart PostgreSQL.

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

These steps, and the rest of the page, are for the vcon-server host.

The password and the client certificate's key are each in a file that
gives other users no access: no bit of `0007` set. The group may read it,
for a file owned by root and readable by the service's group. Install each
password on one line, and the certificate vcon-server is to trust for the
database (your authority's, or the self-signed one from above):

```sh
printf %s "$PG_PASSWORD" | sudo install -m 0600 -o vcon -g vcon /dev/stdin /etc/vcon-server/pg.password
printf %s "$PG_MIGRATE_PASSWORD" | sudo install -m 0600 -o vcon -g vcon /dev/stdin /etc/vcon-server/pg-migrate.password
sudo install -m 0644 -o vcon -g vcon pg-server.pem /etc/vcon-server/pg-ca.pem
unset PG_PASSWORD PG_MIGRATE_PASSWORD
sudo stat -c '%a %U %n' /etc/vcon-server/pg.password /etc/vcon-server/pg-migrate.password
```

```text
600 vcon /etc/vcon-server/pg.password
600 vcon /etc/vcon-server/pg-migrate.password
```

A client certificate's key is installed the same way, as the file
`client_key_file` names:

<!-- docs-test: skip the run logs in with a password, not a client certificate -->
```sh
sudo install -m 0600 -o vcon -g vcon pg-client.key /etc/vcon-server/pg-client.key
```

The server reads every file `[catalog.postgres]` names once, when it
starts, before it connects, and never again: each new connection is made
from what it read. A message never holds the password or the key.

The certificate names `db1.example.com`. Until DNS gives that name to the
database host, add it to `/etc/hosts` on the vcon-server host:

```sh
echo '127.0.0.1 db1.example.com' | sudo tee -a /etc/hosts
```

```text
127.0.0.1 db1.example.com
```

## The configuration

Add the `[catalog]` table to the configuration, beside the `[storage]`
table of the bucket. Every key, with its default, is on the
[configuration page](@/docs/configuration.md#catalog-in-postgresql). A
whole configuration, with two database hosts and a client certificate:

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

The steps here add the two tables to the configuration that
[keep vCons in an S3 bucket](@/docs/s3-storage.md) left, with one host and
a password. Stop the server first: with these tables it does not serve.

```sh
sudo systemctl stop vcon-server
printf '%s\n' '' '[catalog]' 'backend = "postgres"' '' '[catalog.postgres]' 'hosts = ["db1.example.com"]' \
  'dbname = "vcon"' 'user = "vcon_app"' 'ca_file = "/etc/vcon-server/pg-ca.pem"' \
  'password_file = "/etc/vcon-server/pg.password"' \
  | sudo tee -a /etc/vcon-server/vcon-server.toml >/dev/null
sudo tail -n 9 /etc/vcon-server/vcon-server.toml
```

```text
[catalog]
backend = "postgres"

[catalog.postgres]
hosts = ["db1.example.com"]
dbname = "vcon"
user = "vcon_app"
ca_file = "/etc/vcon-server/pg-ca.pem"
password_file = "/etc/vcon-server/pg.password"
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

## Make the schema

Until the schema is made, the server and every command refuse the
database, and say what to run:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server check --config /etc/vcon-server/vcon-server.toml
```

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the database has no vCon schema: run vcon-server migrate
```

`vcon-server migrate` makes the catalog's schema in the database, as
`vcon_owner`, one transaction per schema version. It needs no data
directory or master key, only `[catalog.postgres]`, so run it with a copy
of the configuration whose `user` and `password_file` are `vcon_migrate`'s:

```sh
sudo sed -e 's/^user = "vcon_app"/user = "vcon_migrate"/' -e 's|/pg\.password"|/pg-migrate.password"|' /etc/vcon-server/vcon-server.toml \
  | sudo install -m 0400 -o vcon -g vcon /dev/stdin /etc/vcon-server/vcon-migrate.toml
sudo -u vcon vcon-server migrate --config /etc/vcon-server/vcon-migrate.toml
```

It takes the two locks every server and command shares and refuses if
another holds them, so no server may run during a migration. It says what
it found and what it left:

```text
{"from":0,"message":"schema version 0 to schema version <version>","migrated":true,"to":<version>}
```

Run it again after an upgrade of the binary; a schema already at the
binary's version is left alone:

```sh
sudo -u vcon vcon-server migrate --config /etc/vcon-server/vcon-migrate.toml | jq -c '{migrated, same: (.from == .to)}'
```

```text
{"migrated":false,"same":true}
```

The server never migrates at start: with several instances, one starting
up would change the schema under the others. After each migration, every
new connection reads the schema's version and refuses any other than the
binary's.

## Check it

Run a command with the server's configuration, or start the server:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server check --config /etc/vcon-server/vcon-server.toml
```

In this release it reaches the first host that passes the checks, says
which, and stops with exit code 1:

```text
vcon-server: the storage cannot be used: catalog.postgres: connected to PostgreSQL <version> at db1.example.com:5432 as vcon_app over TLS; this release does not keep the catalogs in PostgreSQL yet (a preview), so it stops here: remove the [catalog] and [catalog.postgres] tables to serve
```

`serve` and every other command (`tenant create` and the rest) do the
same. The next two steps make two of the refusals of
[when it does not start](#when-it-does-not-start) on purpose. A server
that logs in as the role that can change the schema:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server check --config /etc/vcon-server/vcon-migrate.toml
```

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the role "vcon_migrate" owns the catalog's tables or is a member of the role that does; a server logs in as vcon_app
```

And `migrate` run with the server's role, which may not act as the owner:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server migrate --config /etc/vcon-server/vcon-server.toml
```

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the role "vcon_app" cannot act as vcon_owner, which migrate needs; run GRANT vcon_owner TO "vcon_app" as a superuser, for a role the server does not use
```

## Serve from the data directory again

Remove the `[catalog]` and `[catalog.postgres]` tables to serve from the
data directory again. Both must go: `backend = "local"` beside a
`[catalog.postgres]` table is refused, with
`catalog.postgres is set but catalog.backend is not "postgres"`, so that a
table left behind is never silently unused. The steps above put the two
tables at the end of the file, so this removes them. Then start the server,
and ask for its health:

```sh
sudo sed -i '/^\[catalog\]$/,$d' /etc/vcon-server/vcon-server.toml
sudo systemctl start vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done'
curl -sS --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health -H "Authorization: Bearer $MONITOR"
```

```text
{"status":"ok"}
```

## Clean up

The working directory holds the server certificate's key. PostgreSQL has
its own copy; remove this one:

```sh
rm -f pg-server.key pg-server.pem
ls pg-server.key 2>/dev/null || echo "pg-server.key is gone"
```

```text
pg-server.key is gone
```

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
vcon-server: the storage cannot be used: catalog.postgres.password_file /etc/vcon-server/pg.password: other users may read or change the file (mode 0644); run `chmod 600 /etc/vcon-server/pg.password`, or `chmod 640 /etc/vcon-server/pg.password` for a group that only reads it
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

### The schema is missing or the wrong version

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the database has no vCon schema: run vcon-server migrate
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the database schema is version 1, older than this binary's 4: run vcon-server migrate
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the database schema is newer than this binary (version 5, this binary 4): upgrade this server
```

Run `vcon-server migrate` with `vcon_migrate`'s configuration, or upgrade
the server.

### The role owns the catalog or may not use it

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the role "vcon_migrate" owns the catalog's tables or is a member of the role that does; a server logs in as vcon_app
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the role "vcon_x" may not use the vCon schema; make it a member of vcon_app, or log in as vcon_app
```

A server must not hold the role that can change the schema. Set `user` to
`vcon_app`, or to a role that is a member of `vcon_app` and not of
`vcon_owner`.

### migrate cannot act as the owner

```text
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the role "vcon_app" cannot act as vcon_owner, which migrate needs; run GRANT vcon_owner TO "vcon_app" as a superuser, for a role the server does not use
vcon-server: the storage cannot be used: catalog.postgres: another server or command holds the database's locks; stop every server and command and run migrate again
```

Run `migrate` with the login role that is a member of `vcon_owner` (the
first), with every server and command stopped (the second). A message
that a schema exists that vcon-server did not make means a `vcon` schema
is already there without its version table: remove it or use another
database. A migration that fails leaves the database at the version
before it; its message names the statement's error.

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
