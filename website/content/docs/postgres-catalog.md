+++
title = "Keep the catalogs in PostgreSQL"
description = "Preview: keep the registry and every tenant's catalog in a PostgreSQL database, reached over TLS, with one server per database: the database's side, the roles, the schema and `migrate`, TLS and client certificates, the configuration, the secret files, the first start, the commands, backups, key rotation, and what each refusal means."
weight = 30

[extra]
group = "how-to"
+++

**This page is a preview.** With `catalog.backend = "postgres"` the server
keeps its registry (tenants, keys, users, the admin log) and every
tenant's catalog (which vCons and versions it holds, their keys, its audit
log) in a PostgreSQL database, in place of the SQLCipher files in the data
directory. The sealed objects stay in the S3 bucket, and what the database
holds of a vCon is sealed or keyed as the files were.

**Purpose.** The catalogs are then backed up, replicated and failed over
with PostgreSQL's own tools, on a host other than the server's. What it
does not change: the API, the keys, the bucket's contents, and the master
key file, which never leaves the server's host.

What this release does and does not do:

- **One server serves a database.** A second server on the same database
  is refused at start. Several servers on one database are the next step
  of the [roadmap](@/docs/roadmap.md).
- **A database starts with no tenant.** No command moves the tenants of a
  data directory into PostgreSQL yet, so this page sets up a new
  deployment beside the data directory's, on a bucket prefix of its own.
- **The data directory stays.** It holds the two lock files of
  [the commands](#commands-beside-a-server-and-alone) and nothing else.
- **`vcon-server backup` and `storage move-objects` are refused**, each
  with what to use (see [back it up](#back-it-up)).

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
  never the schema. Row-level security gives a statement from it with no
  tenant set no row, to read or to change, and refuses the statement at the
  first row it examines. A statement that examines no row, such as a
  prepared statement on a table with no rows, can end with nothing in place
  of the refusal.
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
a password. Stop the server first: the configuration is read at start.

The bucket's prefix changes too. The data directory claimed the prefix it
serves, and a database is refused a prefix that a data directory or
another database claimed, so the database gets the prefix `pg/` of the
same bucket (`storage.s3.prefix`), which nothing has written to:

```sh
sudo systemctl stop vcon-server
sudo sed -i '/^bucket = /a prefix = "pg/"' /etc/vcon-server/vcon-server.toml
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

### After an upgrade from 0.4.5

Release 0.4.5 and the releases before it leave the schema at version 6.
A build from `main` makes version 13, so a database that 0.4.5 migrated
needs the command once more, with the new binary and before any server
starts:

<!-- docs-test: skip the run starts from a new database, which the step above brought to this build's version -->
```sh
sudo -u vcon vcon-server migrate --config /etc/vcon-server/vcon-migrate.toml
```

```text
{"from":6,"message":"schema version 6 to schema version 13","migrated":true,"to":13}
```

Until then a server stops with `the database schema is version 6, older
than this binary's 13: run vcon-server migrate`. Versions 7 to 13
change no row there is. Version 7 changes two things:

- The `version` table gains the column `seq`, a number PostgreSQL gives
  each version as it is stored. Two versions of one vCon can hold the same
  time of storing, and the order of a vCon's versions is read by that time
  and then by `seq`.
- On PostgreSQL 17 and later, `vcon_app` is granted `MAINTAIN` on the
  tables `version`, `vcon` and `tombstone`. After a purge the server will
  run `VACUUM` on those three, so the bytes of the deleted rows leave the
  table files sooner. PostgreSQL 16 has no such privilege: there the
  server's `VACUUM` is skipped with a warning in PostgreSQL's log, and
  autovacuum does the same work later. To check the grant on 17 or later:

<!-- docs-test: skip the scenario runs migrate and does not ask PostgreSQL for the grant -->
```sh
sudo -u postgres psql -At -d vcon -c "SELECT has_table_privilege('vcon_app', 'vcon.version', 'MAINTAIN')"
```

```text
t
```

Version 8 adds triggers and grants nothing:

- PostgreSQL numbers each row of the tables `version`, `manifest`, `audit`
  and `admin_audit` in their column `seq`, whatever the statement that
  stores the row says, and keeps the number when the row is updated.
  Before version 8 an `INSERT` with `OVERRIDING SYSTEM VALUE` chose the
  number, and the newest version of a vCon is read by it. The numbers of a
  table rise by two for each row: they give an order, not a count.
- The audit logs, the tables `audit` and `admin_audit`, take new records.
  A record there is changes in one way: the `count` of a record the server
  counts requests in goes up. Every other `UPDATE`, every `DELETE`
  and every `TRUNCATE` is refused, for `vcon_owner` as for `vcon_app`. The
  owner of a table can drop its triggers, which is one reason no server
  logs in as `vcon_owner`.

Version 9 replaces the function those triggers run, and makes no table
and no trigger. Under version 8 an `UPDATE` passed when it added exactly
one to a count. Under version 9 it passes when the record is one the
server counts requests in, the new `count` is larger than the old one,
and no other column is different, so the server writes the refused
requests of a minute with one statement. The count only grows: an
`UPDATE` that sets it to the same number or to a smaller one is refused.

Version 10 changes who reads the table `tenant_lock`, which holds one
row for each tenant that has a catalog. A transaction of the registry
now reads every row of it, and writes none: the server lists with it
the tenants that have a catalog, to remove one that a cut-off create or
removal left with no row in the registry. A tenant's transaction reads
and writes its own row and no other, as before. No other table changes.

Version 11 gives each tenant's audit log a key of its own. The table
`audit_key` holds that key, sealed under the tenant's catalog key, and
each record in `audit` names the key that sealed its target, in the new
column `key_id`. Before version 11 a record's target was sealed under
the catalog key itself, and a rotation of the tenant key would have had
to write every record again, which version 8 refuses. `vcon_app` reads
an audit key, makes one and seals one again; no role is granted a
`DELETE` there, a record cannot name a key its tenant does not have, and
a key that a record names cannot be deleted. The registry's log,
`admin_audit`, is not changed. A database of the preview holds no audit
record, since its server stops after it connects, so version 11 converts
none.

Version 12 makes the table `registry_key`, whose one row names the
catalog key the registry's columns are sealed under, and gives the
registry's audit log a key of its own. The registry is the part of the
database that belongs to no tenant: the tenants, the API keys, the users
and their sessions. The id in `registry_key` is the id of a key derived
from the master key: a server that opens the registry under another
master key is refused with what it finds there, and a rotation of the
master key moves the id with the columns. The table `admin_audit_key`
holds the key of the registry's log, sealed under the registry's keys,
and each record in `admin_audit` names the key that sealed its target,
in the new column `key_id`, as a tenant's record does since version 11.
`vcon_app` reads the id, writes it once and changes it; it reads an
audit key, makes one and seals one again; no role is granted a `DELETE`
on either table, a record cannot name a key the log does not have, and a
key that a record names cannot be deleted. A database of the preview
holds no row of the registry, so version 12 converts none.

The statements of a rotation of the master key move the registry in one
transaction, with the registry's lock held. Every tenant key, a pending
one too, is wrapped again under the new master key. The sealed columns
and the lookup values of the tenants, API keys, users, memberships,
settings and sessions are made again under the new key, and the key of
the registry's log is sealed again. The rows of `master_key_use` give
way to one row for the new key, the id in `registry_key` is moved, and
the rotation is recorded in `admin_audit`. The transaction changes all
of them or none, and no record of the log is changed. A registry that is
under the new key already is left as it is, so a rotation can be asked
for again; a registry under neither key is refused. No command runs
those statements yet: `rekey master` works on the local registry.

Version 13 makes the table `storage_claim`, whose one row holds the
claim of the bucket prefix: the random id that binds a prefix to one set
of catalogs, as [the bucket's claim](@/docs/s3-storage.md#one-server-per-bucket-and-prefix)
describes. Catalogs in a data directory keep the id in the file
`storage.claim`; catalogs in PostgreSQL keep it in this row, so every
server on the database reads the same claim, and a server on another
database is refused the prefix. The table takes one row, and an id is 32
lowercase hexadecimal digits. `vcon_app` reads the claim and writes it
once, in a transaction of the registry; no role is granted an `UPDATE`
or a `DELETE` there. A database of the preview holds no claim, so the
table starts empty.

The audit key does not rotate with the tenant key: a rotation seals the
same audit key under the new catalog key and changes no audit record.
The statements of that rotation move a tenant's catalog to the new key
in one transaction: the sealed columns and the lookup values of its
vCons, versions and purge records are made again under the new key, and
the transaction changes either all of them or none. No server runs
those statements yet. Giving a log a new audit key is not written: the
column `key_id` is there so that the records before such a change stay
readable under the key they name.

A tenant's audit records are never deleted: the statements that remove a
tenant delete its rows from every table but `audit` and `audit_key`, and
the database refuses a `DELETE` of a record from every role. No server
runs those statements yet. A kept record shows its time, actor, action,
status and count to a transaction that sets `vcon.tenant` to the removed
tenant's id. Its target is sealed under the log's audit key, the audit
key is sealed under a key made from the tenant's key, and a removal
deletes the tenant's key with the tenant's row in the registry.

To check the second, try to empty a log as the owner:

<!-- docs-test: skip the scenario runs migrate and does not try to empty a log -->
```sh
sudo -u postgres psql -d vcon -c "SET ROLE vcon_owner" -c "TRUNCATE vcon.audit"
```

```text
SET
ERROR:  an audit log is not emptied
```

`migrate` gives up its two locks before it ends, so a server can be
started as soon as the command returns.

## Check it

Run `check` with the server's configuration. It connects as the server
does, takes the locks of a command that runs alone, opens the registry
under the master key, and examines every tenant. A new database has none:

```sh
sudo -u vcon vcon-server check --config /etc/vcon-server/vcon-server.toml | jq -c '{tenants, problems}'
```

```text
{"tenants":0,"problems":0}
```

That first open wrote the id of the master key's catalog key into the
database. From here on the database opens under this master key only.

The next two steps make two of the refusals of
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

## Start the server

Make the first tenant and a key for it. `tenant create` runs alone, so it
comes before the server starts, as on a data directory:

```sh
sudo -u vcon vcon-server tenant create acme --config /etc/vcon-server/vcon-server.toml >/dev/null
PG_TENANT=$(sudo -u vcon vcon-server tenant list --config /etc/vcon-server/vcon-server.toml | jq -r 'select(.name == "acme") | .id')
PG_KEY=$(sudo -u vcon vcon-server key create --tenant "$PG_TENANT" --scope write --scope read --label pg-check \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
sudo -u vcon vcon-server tenant list --config /etc/vcon-server/vcon-server.toml | jq -c '{name}'
```

```text
{"name":"acme"}
```

Start the server and wait for it to answer. Before its `serving` line it
logs the database server it reached and the role it logged in as:

```sh
sudo systemctl start vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done'
sudo journalctl -u vcon-server --no-pager -o cat | grep -o 'the catalogs are in PostgreSQL catalog=.* over TLS' | tail -n 1
```

```text
the catalogs are in PostgreSQL catalog=PostgreSQL <version> at db1.example.com:5432 as vcon_app over TLS
```

## Verify

Store a vCon with the new key and read it back:

```sh
PG_UUID=$(cat /proc/sys/kernel/random/uuid)
cat > pg-call.vcon <<EOF
{"vcon":"0.4.0","uuid":"$PG_UUID","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550101","name":"Caller Example"}],"dialog":[],"attachments":[],"analysis":[]}
EOF
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/vcons \
  -H "Authorization: Bearer $PG_KEY" -H "Content-Type: application/vcon" --data-binary @pg-call.vcon
curl -sS --cacert /usr/local/share/vcon-server/ca.pem "https://vcon.example.com:8443/vcons/$PG_UUID" \
  -H "Authorization: Bearer $PG_KEY" | jq -c '{parties: (.parties | length)}'
```

```text
201
{"parties":1}
```

The version is a row of the database. A superuser reads the count; the
server's role sees a tenant's rows only inside a transaction that names
the tenant:

```sh
sudo -u postgres psql -At -d vcon -c "SELECT count(*) FROM vcon.version"
```

```text
1
```

A command that runs beside a server reads the same registry while the
server runs:

```sh
sudo -u vcon vcon-server key list --config /etc/vcon-server/vcon-server.toml | jq -r .label
```

```text
pg-check
```

## Commands beside a server, and alone

The commands keep the rule they have on a data directory, and hold it in
two places: the lock files `lock` and `lock.commands` in `data_dir`, and
two advisory locks in the database, held on one connection for as long as
the process runs. The database's locks are what stop a second host.

| Work | Runs | Refused while |
|---|---|---|
| `serve` | one per database | another server, a command that runs alone, or `migrate` holds the database |
| `tenant list`, the `tenant set-` commands, `tenant role-cap`, `key create`, `key list`, `key revoke`, the `user` and `membership` commands, `audit export`, `export parquet` | beside a server and beside each other | a command that runs alone, or `migrate`, holds the database |
| `tenant create`, `tenant remove`, `rekey tenant`, `rekey master`, `check` | alone | a server or any other command holds the database |
| `migrate` | alone | a server or any command holds the database |

A lock is tried once and never waited for: the refused process says what
may hold the database and exits 1. A server that stops gives its lock up
before it ends, so the next start finds the database free. PostgreSQL
frees the lock of a process that was killed when it closes that process's
connection.

A connection pooler in transaction mode (PgBouncer's `pool_mode =
transaction`) gives each transaction another server connection, and a
session's advisory lock would then be held by none of them: connect the
server to PostgreSQL directly, or through a pooler in session mode. No
pooler was tested.

## Back it up

Three things together are a backup: the catalog database, the bucket, and
the master key file.

- **The database.** Use PostgreSQL's tools: `pg_dump` of the database
  `vcon`, or a base backup with WAL archiving for a restore to a point in
  time. They are the operator's to run and to test.
- **The bucket.** As [keep vCons in an S3 bucket](@/docs/s3-storage.md)
  says. A dump taken at one time and a bucket copied at another may
  disagree by the writes between them: `vcon-server check` on the restored
  pair names each version whose object is missing.
- **The master key file**, kept apart from both, as
  [backup and restore](@/docs/backup-and-restore.md) says.

`vcon-server backup` copies a data directory's files and is refused here,
before anything is left at the path it was given:

```text
vcon-server: the catalogs are in PostgreSQL, which backup does not copy into a directory: back up the catalog database with pg_dump or a base backup with WAL archiving, and keep the bucket and the master key file with it
```

A purged vCon is unreadable from an old dump once its tenant's key and the
master key have been rotated and the old master key destroyed, as with a
copy of a data directory: keep no dump longer than `rotation_warn`.

## Rotate the keys

[Rotate the keys](@/docs/rotate-keys.md) applies as it stands: stop the
server, `rekey tenant` for each tenant, `rekey master --new-master`, point
`master_key` at the new file, start the server, destroy the old key.

`rekey master` makes every row of the registry again under the new key in
one transaction, with the database's locks held: it either moved the whole
registry or changed nothing. Run again after it succeeded, with the same
two key files, it changes nothing and exits 0. Until `master_key` names
the new file, the server and every command stop with:

```text
vcon-server: the master key is not the one the registry in the catalog database was made under
```

## Serve from the data directory again

The data directory's own catalogs were not touched by the steps above.
Remove the `[catalog]` and `[catalog.postgres]` tables and the `prefix`
line to serve them again. Both tables must go: `backend = "local"` beside
a `[catalog.postgres]` table is refused, with
`catalog.postgres is set but catalog.backend is not "postgres"`, so that a
table left behind is never silently unused. The steps above put the two
tables at the end of the file, so this removes them. The tenants and the
vCons of the database stay in the database and under the prefix `pg/`:
the data directory's server does not see them. Then start the server, and
ask for its health:

```sh
sudo systemctl stop vcon-server
sudo sed -i -e '/^prefix = "pg\/"$/d' -e '/^\[catalog\]$/,$d' /etc/vcon-server/vcon-server.toml
sudo systemctl start vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done'
curl -sS --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health -H "Authorization: Bearer $MONITOR"
```

```text
{"status":"ok"}
```

## Clean up

The working directory holds the server certificate's key and the vCon of
[verify](#verify). PostgreSQL has its own copy of the key; remove these:

```sh
rm -f pg-server.key pg-server.pem pg-call.vcon
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
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the database schema is version 6, older than this binary's 13: run vcon-server migrate
vcon-server: the storage cannot be used: catalog.postgres: no server could be used: db1.example.com:5432: the database schema is newer than this binary (version 14, this binary 13): upgrade this server
```

Run `vcon-server migrate` with `vcon_migrate`'s configuration, or upgrade
the server.

### A statement on an audit log is refused

```text
ERROR:  an audit record is not deleted
ERROR:  an audit log is not emptied
ERROR:  an audit record is not changed, but for a larger count of a counted record
```

From schema version 8 the tables `audit` and `admin_audit` take new
records and nothing else, from every role, but for the `count` of a
record the server counts requests in. The count only grows: from schema
version 9 an `UPDATE` may make it larger, by any number, and change no
other column. A statement run by hand that deletes or rewrites a record,
or sets a count to the same number or a smaller one, is refused with one
of these; the server sends no such statement. Reading the log with `SELECT` is not refused.

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

### Another process holds the database

```text
vcon-server: the storage cannot be used: catalog.postgres: another server, a command that runs alone, or a migration holds this database's lock; one server serves a database, and those commands run while none does
vcon-server: the storage cannot be used: catalog.postgres: a server or another command holds this database's locks; this command runs alone: stop every server, wait for the other commands to end, and run it again
vcon-server: the storage cannot be used: catalog.postgres: a command that runs alone, or a migration, holds this database's lock; run this command again when it has ended
vcon-server: the storage cannot be used: catalog.postgres: another server or command holds the database's locks; stop every server and command and run migrate again
```

The first is a server's, the second that of a command that runs alone,
the third that of a command beside a server, the fourth `migrate`'s. See
[commands beside a server, and alone](#commands-beside-a-server-and-alone).
To see which connection holds a lock, as a superuser on the database host:
`SELECT a.pid, a.client_addr, a.backend_start FROM pg_locks l JOIN
pg_stat_activity a USING (pid) WHERE l.locktype = 'advisory'`.

A second server on the same host and data directory is stopped by the
data directory's lock first, with `the data directory is in use by
another vcon-server or a command that runs alone`.

### The master key is not the database's

```text
vcon-server: the master key is not the one the registry in the catalog database was made under
```

The database names the key its registry was first opened under, or last
rotated to. Point `master_key` at that key's file. After `rekey master`
that is the new file.

### The bucket prefix belongs to something else

```text
vcon-server: the storage cannot be used: bucket "vcon", prefix "" belongs to another database (its claim is <claim>); start-up recovery here would remove that server's vCons. Give this server a prefix of its own, or point it at the database that claimed this one
```

The prefix holds the claim of other catalogs: a data directory's, or
another database's. The message names the kind of catalog that asked, here
a database. A database keeps its claim in its table `vcon.storage_claim`,
so every host that serves the database shares one claim. Give the database
a `storage.s3.prefix` of its own, as
[the configuration](#the-configuration) does.

### The pool is too small

```text
vcon-server: the storage cannot be used: catalog.postgres.pool is 1: a server needs at least 2 connections, one for the lock every server holds and one for a transaction
```

Set `catalog.postgres.pool` to 2 or more, or remove it for the default.

### backup or storage move-objects is refused

```text
vcon-server: the catalogs are in PostgreSQL, which backup does not copy into a directory: back up the catalog database with pg_dump or a base backup with WAL archiving, and keep the bucket and the master key file with it
vcon-server: the storage cannot be used: storage move-objects moves the objects of a data directory's catalogs to the bucket; with catalog.backend = "postgres" every object is in the bucket from its first write and there is nothing to move: to move a data directory's objects, run it with the [catalog] and [catalog.postgres] tables removed
```

See [back it up](#back-it-up). A PostgreSQL catalog never has objects in a
data directory.
