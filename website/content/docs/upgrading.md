+++
title = "Upgrade"
description = "Move a running server to a newer release: check the new binary, stop the server, copy the data directory, install the binary, check the data with it, start it, and go back to the old version if you need to."
weight = 25

[extra]
group = "how-to"
+++

An upgrade replaces the binary. The data directory, the configuration and
the keys stay as they are. A newer binary opens the data of an older one,
and brings its catalogs up to its own schema version the first time it
opens them, each in one transaction. An older binary refuses a catalog with
a newer schema version than its own. So the way back from an upgrade is the
copy of the data directory taken just before it.

CI runs every command on this page on each change. It installs the previous
release, sets it up as [getting started](@/docs/getting-started.md) does,
upgrades it to the server built from the change, and goes back again.

## From 0.4.4 to 0.4.5

The upgrade is the binary alone. No stored data and no schema changes: the
catalogs keep schema version 9, and the
[PostgreSQL preview](@/docs/postgres-catalog.md) keeps schema version 6. A
0.4.4 configuration is read as it was. A downgrade is possible: 0.4.4
opens a data directory that 0.4.5 has served. Copy the data directory
before the upgrade all the same, as in
[copy the data directory](#copy-the-data-directory). Three things look
different after the upgrade:

- **The terminal UI's last line names only the keys that act.** On each
  screen of [`vcon-server tui`](@/docs/tui.md) the last line lists the
  keys that do something in the state shown: `? help  q quit` while
  connecting or loading, `r refresh  ? help  q quit` for an empty tenant
  or a failure, no `n more` when every vCon is read, and no
  `j down  k up` for a list of one. Release 0.4.4 showed the list's keys
  on each of these screens.
- **Two failure screens say what can be done.** For an operator key, and
  for a key without the `read` scope, the screen says `Press q to quit.`
  where 0.4.4 said `Press r to try again, or q to quit.`: `r` cannot read
  another key. While a message stands in place of the list, the keys that
  select and open do nothing.
- **`expires_at` of a new key is a whole second.** A key made by 0.4.5
  has an expiry with no fraction of a second, such as
  `2027-02-10T12:00:00Z`, and the answer that makes it is 136 bytes. A
  key made by 0.4.4 keeps the expiry it has, which may carry a fraction
  of a second, so a client must still read both forms. See
  [set a key's expiry](@/docs/tenant-administration.md#set-a-key-s-expiry-and-see-when-it-was-last-used).

## From 0.4.3 to 0.4.4

There is no downgrade from 0.4.4: release 0.4.3 refuses a data directory
that 0.4.4 has opened. Copy the data directory before the upgrade, as in
[copy the data directory](#copy-the-data-directory); that copy is the
only way back. A 0.4.3 configuration is read as it was. Four things need
a look before the upgrade:

- **The catalogs change.** See
  [schema version 9](#schema-version-9-and-no-way-back-without-the-copy)
  first.
- **New keys expire.** A key made by 0.4.4 expires 90 days after it is
  made, unless its request names 1 to 365 days or asks for no expiry.
  Keys that exist keep working with no expiry. See
  [keys made before the upgrade have no expiry](#keys-made-before-the-upgrade-have-no-expiry).
- **A request that carries both a key and the session cookie is
  refused.** Every route answers 400
  [`key-and-session`](@/problems/key-and-session.md) to a request with an
  `Authorization` header and a cookie named `__Host-vcs_session`. No
  release before 0.4.4 set that cookie, so a client is affected only if
  it sends a cookie of that name itself.
- **Sessions and users have no sign-in method.** 0.4.4 keeps users,
  memberships and sessions, and nothing in it starts a session, so nobody
  can sign in as a user; API keys work as before. The
  [status](@/docs/status.md) page says what is built.

### Schema version 9, and no way back without the copy

The new binary brings every catalog from schema version 4 to schema version
9, each in one transaction: `server.sqlite` the first time `serve`, `check`
or any other command opens it, and each tenant's catalog the first time
that tenant's data is opened after it. A catalog whose change fails stays
whole at version 4, and the command says why.

- `server.sqlite` gains two tables, `user_account` and `membership`, for
  [users and memberships](@/docs/users-and-memberships.md). Both start
  empty.
- The `tenant` table of `server.sqlite` gains the column `role_cap`, for a
  tenant's [role cap](@/docs/roles-and-scopes.md). It starts empty for every
  tenant: no cap, so every tenant role may be granted, as before.
- `server.sqlite` gains the table `tenant_setting`, for the settings a
  [tenant's administrator](@/docs/tenant-administration.md) sets. It starts
  empty: every tenant has every default.
- `server.sqlite` gains the table `session`, for the
  [sessions](@/docs/session-origin.md) of users who signed in. It starts
  empty, and holds the SHA-256 of each session's id and of its CSRF
  token, never the id or the token.
- The `api_key` table of `server.sqlite` gains the columns `expires_at`
  and `last_used_at`. Both start empty for every key there is: no expiry,
  and no last-used time until the key's next request.
- Every audit log, the admin log and each tenant's, can hold a record
  with no status. A log at schema version 4 could not. Release 0.4.3
  writes one for a listener that filled (`listener_full`), and at version 4
  keeps the number 0 in the place of its status. The upgrade writes each log again,
  with no status where a 0 was, under the same sequence numbers, whether or
  not the log holds such a record. `audit export` and `GET /audit` showed
  `null` for such a record before and show `null` after. The copy takes
  time in proportion to the number of records in the log, and while it runs
  the catalog file and its `-wal` file hold both copies: before you start,
  have free space of at least twice the size of the largest catalog file.

Release 0.4.3, and every release before it, refuses a catalog at schema
version 9. Once the new binary has opened a data directory, 0.4.3's
`check`, `serve` and every other command stop with `vcon-server: the
registry cannot be opened: opening /var/lib/vcon-server/server.sqlite
failed: schema version 9 is not one this build knows`. There is no
downgrade. The only way back is the copy of the data directory taken
before the upgrade, as in
[go back to the old version](#go-back-to-the-old-version); everything
stored after the copy is lost. Before you install the new binary, keep:

- a copy of the data directory, taken with the server stopped, as in
  [copy the data directory](#copy-the-data-directory);
- the master key, apart from that copy;
- the configuration file, and the TLS certificate and its key;
- the 0.4.3 binary, to go back to.

With the [PostgreSQL catalog preview](@/docs/postgres-catalog.md), the
database's schema goes from version 1 to version 6, and only
`vcon-server migrate` changes it: run it after you install the new binary
and before you start a server. Until then a server stops with `the database
schema is version 1, older than this binary's 6: run vcon-server migrate`.
Version 2 adds the same two tables and lets both audit tables hold no
status, version 3 adds the role cap's column to the tenant table, version
4 adds the table of the settings a tenant's administrator sets, version
5 adds the table of sessions, and version 6 adds the two columns of a
key's expiry and last-used time. A 0.4.3
binary refuses a database at version 2, 3, 4, 5 or 6 as `newer than this
binary`; the way back is a copy of the database taken before `migrate`.

### Keys made before the upgrade have no expiry

From this version a new key expires. `key create`, `POST
/admin/tenants/{id}/keys`, `POST /tenant/keys` and the MCP tool
`admin_key_create` make a key that expires 90 days after it is made,
unless the request names 1 to 365 days or asks for no expiry. A request
by an expired key is answered `401`
[`key-expired`](@/problems/key-expired.md).

The upgrade changes no key that exists. Every key made by 0.4.3 or
earlier keeps working after the upgrade with no expiry, and `key list`
shows its `expires_at` as `null`. No client is refused because of the
upgrade. To put such a key under an expiry, make a new key, move the
client to it and revoke the old one, as in
[rotate keys](@/docs/rotate-keys.md).

A script that makes a key and never replaces it stops working 90 days
later. Either replace its key on a schedule shorter than the key's life,
or make its key with `--no-expiry` (`"no_expiry": true` over HTTP). To
refuse keys with no expiry, set `no_expiry = false` in the `[keys]` table
of the [configuration](@/docs/configuration.md); keys made earlier are
not changed by that setting.

## From 0.4.2 to 0.4.3

The upgrade is the binary alone: the catalogs keep their schema version, so
0.4.2 opens a data directory that 0.4.3 has served, and a 0.4.2
configuration is read as it was. Four things need a look before it, and a
fifth is new and optional.

- **Readers of the audit log.** The console marks each Reveal, so a
  tenant's log holds `reveal_parties`, `reveal_subject`, `reveal_dialog`,
  `reveal_analysis`, `reveal_links`, `reveal_all` and `reveal_audio`
  where 0.4.2 wrote `read`, and the `legal_reveal_` actions with the same
  seven endings where it wrote `legal_read`. Before you install the new
  binary, add the fourteen names to any rule, parser or report that lists
  actions; see [reveals](@/docs/audit-logs.md#reveals).
- **A proxy that passes the console's paths by name.** The console has two
  more files, `/console/player.html` and `/console/player.js`, for its
  [recording player](@/docs/console.md#play-a-recording). Pass both, and
  pass each answer's `Content-Security-Policy` header unchanged: the
  player's page has a policy of its own.
- **Clients that hold many connections, or hold them long.** Release 0.4.2
  kept every connection for as long as the client did, and took as many as
  it was offered. The server now holds at most 512 connections on a
  listener, closes one that has waited 120 seconds for its next request
  (an HTTP/2 connection after a GOAWAY frame), and closes one whose TLS
  handshake takes more than 10 seconds or whose request head more than 30.
  An HTTP client opens a new connection when it finds one closed, so most
  need no change. Before you install the new binary, count the connections
  your clients and proxies hold open at once: for more than 512, set
  `limits.connections`, and keep the process's limit on open files above
  it. A proxy that keeps idle connections to the server should keep them
  for less than 120 seconds, or `limits.timeouts.idle` should be raised
  above the proxy's own time. An idle HTTP/1.1 connection is kept up to
  120 seconds. Release 0.4.2 set no time on it. To close idle connections
  sooner, for example after 30 seconds, set `idle = 30` in
  `[limits.timeouts]`; to keep them as long as the key allows, set
  `idle = 3600`. `limits.timeouts.head` covers only a request head that
  has begun. A reader of the audit
  log that knows a fixed list of actions must add `listener_full`, and
  must take `null` as that action's `status`. See
  [connection limits](@/docs/configuration.md#connection-limits).
- **A proxy or a monitor that holds the console's headers, and readers of
  the audit log.** Both `Content-Security-Policy` values of the console
  now end with `; report-uri /console/csp-report`. A check that compares
  either policy byte for byte needs the new value; a proxy that passes only
  listed paths must pass `POST /console/csp-report`, which takes no key. A
  reader of the audit log that knows a fixed list of actions must add
  `csp_report`. See
  [reports of a broken policy](@/docs/console.md#reports-of-a-broken-policy).
- **The operator listener, if you turn it on.** A configuration without an
  `[operator]` table serves as before: one listener, both kinds of key.
  With the table, operator keys work only at the new address, so every
  script, client file and console bookmark that uses one must move there,
  and a proxy rule that passes `/admin/` on the public name can go. The
  audit log gains the action `wrong_listener`. See
  [the operator listener](@/docs/operator-listener.md).

## From 0.4.1 to 0.4.2

Nothing in 0.4.2 needs a step of its own: the steps below, with the new
binary, are the whole upgrade. The catalogs keep their schema version, so
0.4.1 opens a data directory that 0.4.2 has served, and the configuration
and the HTTP API's routes are as they were. `vcon-server migrate` is new and
is for the [PostgreSQL catalog preview](@/docs/postgres-catalog.md) only; a
server with the default catalogs does not run it.

Two things look different after the upgrade. The
[console](@/docs/console.md) opens a vCon masked, and shows its text only
after a reveal, which the audit log records as a read. Its page loads one
more file, `/console/format.js`, so a proxy that passes the console's paths
by name must pass that one too. And the binary has a new command,
[`vcon-server tui`](@/docs/tui.md).

## From 0.4.0 to 0.4.1: the secret-file rule

From 0.4.1, every file that holds a secret is held to one rule: the master
key, the TLS key, the S3 and export secret access key files, the PostgreSQL
password and client key, and the MCP stdio key file (see
[secret files](@/docs/configuration.md#secret-files)). A file that other
users may read, or that its group may change, or that another user owns, now
stops the server at start, and stops each command that reads it. 0.4.0
used such a file without a word. **This is a breaking change:** a host whose
master key is `0644` starts under 0.4.0 and does not start under 0.4.1.

Before you install the new binary, list the mode and the owner of each
secret file the configuration names. The master key and the TLS key, as the
configuration of [getting started](@/docs/getting-started.md) names them:

```sh
SECRET_FILES=$(sudo sed -n -e 's/^master_key = "\(.*\)"$/\1/p' -e 's/^key = "\(.*\)"$/\1/p' /etc/vcon-server/vcon-server.toml)
sudo stat -c '%a %U' $SECRET_FILES
```

```text
400 vcon
400 vcon
```

Add each `secret_access_key_file`, `password_file` and `client_key_file`
the configuration sets. Each file passes when its mode is `600`, `400` or
`640` and its owner is the server's account or root. For one that does not,
run the command its error names, for example:

- `sudo chmod 600 /etc/vcon-server/master.key` for a mode such as `644`;
- `sudo chown vcon /etc/vcon-server/master.key` for a file another account
  owns.

## From 0.3.0 to 0.4.0

0.4.0 brings every catalog, `server.sqlite` and each tenant's, from schema
version 2 to schema version 4 the first time it opens it. 0.3.0 refuses a
catalog at schema version 3 or 4. So once 0.4.0 has opened a data directory,
as `serve` and `check` do, 0.3.0 cannot open that directory again: there is
no downgrade. 0.3.0's `check` and `tenant list`, for example, stop with
`vcon-server: the registry cannot be opened: opening
/var/lib/vcon-server/server.sqlite failed: schema version 4 is not one this
build knows`. The only way back to 0.3.0 is the copy of the data directory
taken before the upgrade, as in [go back to the old
version](#go-back-to-the-old-version); everything stored after the copy is
lost.

Before you install 0.4.0, keep:

- a copy of the data directory, taken with the server stopped, as in
  [copy the data directory](#copy-the-data-directory);
- the master key, apart from that copy;
- the configuration file, and the TLS certificate and its key;
- the 0.3.0 binary, to go back to.

## Before you upgrade

- Read the [changelog](https://github.com/NormB/vcon-server/blob/main/CHANGELOG.md)
  for every version after yours. Each version lists what it adds, changes
  and fixes.
- Know the version you run:

  ```sh
  vcon-server --version
  ```

  ```text
  vcon-server <version>
  ```

- One shell for every step: later steps use variables that earlier steps set.

```sh
URL=https://vcon.example.com:8443
CONFIG=/etc/vcon-server/vcon-server.toml
TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
KEY=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope write --scope read --label upgrade-check \
  --config $CONFIG | jq -r .token)
```

Store a vCon to read back after the upgrade:

```sh
UUID=$(cat /proc/sys/kernel/random/uuid)
printf '{"vcon":"0.4.0","uuid":"%s","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550106"}],"dialog":[],"attachments":[],"analysis":[]}' \
  "$UUID" > upgrade.vcon
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem "$URL/vcons" \
  -H "Authorization: Bearer $KEY" -H "Content-Type: application/vcon" --data-binary @upgrade.vcon
```

```text
201
```

## Get the new binary

Download the new release and check it, as on [install](@/docs/install.md#download-a-release),
but do not install it yet. The commands use their own variable names, so
they do not change the ones from install:

```sh
NEW_VERSION=0.4.5
NEW_ARCHIVE=vcon-server-$NEW_VERSION-$(uname -m)-unknown-linux-gnu.tar.gz
curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$NEW_VERSION/$NEW_ARCHIVE
curl -fsSL -o SHA256SUMS-$NEW_VERSION.txt https://github.com/NormB/vcon-server/releases/download/v$NEW_VERSION/SHA256SUMS.txt
sha256sum -c SHA256SUMS-$NEW_VERSION.txt --ignore-missing
tar -xzf "$NEW_ARCHIVE"
```

```text
vcon-server-<version>-<cpu>-unknown-linux-gnu.tar.gz: OK
```

The new binary is in the archive's directory. With a binary from
[cargo install](@/docs/install.md#install-with-cargo) or a
[source build](@/docs/install.md#build-from-source), set `NEW` to its path
instead:

```sh
NEW=${NEW_ARCHIVE%.tar.gz}/vcon-server
"$NEW" --version
```

```text
vcon-server <version>
```

## Stop the server

Under systemd:

<!-- docs-test: only systemd -->
```sh
sudo systemctl stop vcon-server
```

Or, without systemd:

<!-- docs-test: only no-systemd -->
```sh
sudo kill -TERM "$(pidof vcon-server)"
while pidof vcon-server >/dev/null; do sleep 0.2; done
```

## Copy the data directory

Copy the data directory with the server stopped. The copy holds the data
directory's two lock files, `lock` and `lock.commands`, as a command that
runs alone does, so no server or command can start while it runs. It works
with every version, including those without `vcon-server backup`.
`/backup` stands for your backup storage. The copy goes in a directory of
its own there, for root alone: `/backup` itself stays open to the service
account, which writes its [backups](@/docs/backup-and-restore.md) under it:

```sh
sudo install -d -m 0700 /backup/stopped
BEFORE=/backup/stopped/vcon-server-before-$NEW_VERSION.tar
sudo flock --nonblock /var/lib/vcon-server/lock flock --nonblock /var/lib/vcon-server/lock.commands \
  tar -C /var/lib -cf "$BEFORE" vcon-server
sudo tar -tf "$BEFORE" | sed -n 's|^vcon-server/\([^/]*\)$|\1|p' | sort
```

```text
lock
lock.commands
server.sqlite
```

`flock` exits 1, and copies nothing, if a server or a command still holds
the directory. The copy holds nothing readable without the master key. Keep
it away from the master key's copies. Keep the old binary too:

```sh
sudo cp -p /usr/local/bin/vcon-server /usr/local/bin/vcon-server.previous
```

## Install the new binary and check the data with it

```sh
sudo install -m 0755 "$NEW" /usr/local/bin/vcon-server
vcon-server --version
```

```text
vcon-server <version>
```

`check` opens every catalog and every stored version with the new binary.
A catalog of an older schema is brought up to date as it is opened.
Nothing else changes:

```sh
sudo -u vcon vcon-server check --config $CONFIG | jq -c 'select(.name == null) | {problems}'
```

```text
{"problems":0}
```

## Start it

Under systemd:

<!-- docs-test: only systemd -->
```sh
sudo systemctl start vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
```

```text
answering
```

Or, without systemd:

<!-- docs-test: only no-systemd; background serving -->
```sh
sudo -u vcon vcon-server serve --config /etc/vcon-server/vcon-server.toml
```

```text
<time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443
…
```

The vCon stored before the upgrade reads back the same:

```sh
curl -sS --cacert /usr/local/share/vcon-server/ca.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY" -o upgraded.vcon
cmp upgraded.vcon upgrade.vcon && echo "the same bytes"
```

```text
the same bytes
```

## Go back to the old version

Go back only with the copy taken before the upgrade: the new binary may have
brought the catalogs to a schema the old one refuses. Everything stored
since the copy is lost. Stop the server:

<!-- docs-test: only systemd -->
```sh
sudo systemctl stop vcon-server
```

<!-- docs-test: only no-systemd -->
```sh
sudo kill -TERM "$(pidof vcon-server)"
while pidof vcon-server >/dev/null; do sleep 0.2; done
```

Put back the data directory and the old binary, and check:

```sh
sudo mv /var/lib/vcon-server /var/lib/vcon-server.upgraded
sudo tar -C /var/lib -xpf "$BEFORE"
sudo install -m 0755 /usr/local/bin/vcon-server.previous /usr/local/bin/vcon-server
sudo -u vcon vcon-server check --config $CONFIG | jq -c 'select(.name == null) | {problems}'
```

```text
{"problems":0}
```

`tar -p`, run as root, keeps the files' owner and modes. Start the server
as above. Once it runs as it should, remove `/var/lib/vcon-server.upgraded`.

## In a container

Build an image for the new version, as in
[running it in a container](@/docs/container.md#build-the-image), then
replace the container. The volumes keep the data and the configuration.
Copy the data volume first, with the container stopped:

<!-- docs-test: skip the container scenario builds one version only -->
```sh
docker build -q --build-arg VERSION=$NEW_VERSION -t vcon-server:$NEW_VERSION .
docker stop vcon-server
docker run --rm -v vcon-data:/var/lib/vcon-server -v vcon-backup:/backup debian:trixie-slim \
  tar -C /var/lib -cf /backup/before-$NEW_VERSION.tar vcon-server
docker rm vcon-server
docker run -d --name vcon-server --restart unless-stopped --ulimit memlock=-1 -p 8443:8443 \
  -v vcon-etc:/etc/vcon-server -v vcon-data:/var/lib/vcon-server -v vcon-backup:/backup \
  vcon-server:$NEW_VERSION
```

## With the objects in an S3 bucket

S3 storage is in release 0.4.0 and later (see
[keep vCons in an S3 bucket](@/docs/s3-storage.md)). For a server that keeps
its objects in a bucket:

- **An upgrade does not move objects.** Adding a `[storage]` table to a
  server that has stored vCons is a change of storage, not an upgrade: its
  stored versions stay in the data directory and answer `500` until
  `vcon-server storage move-objects`, run with the server stopped, moves
  them (see [move objects stored in the data directory](@/docs/s3-storage.md#move-objects-stored-in-the-data-directory)).
- **The copy of the data directory does not hold the objects.** The copy
  taken before the upgrade holds the catalogs and `storage.claim`; the
  objects stay in the bucket. Going back to that copy keeps the claim, so
  the old binary starts on the same prefix. Its start-up recovery then
  removes every object stored after the copy, since no row of the copy
  names it, and logs how many it removed.
- **Release 0.3.0 cannot go back to a bucket.** It refuses the `[storage]`
  table (see [the build has no S3 support](@/docs/s3-storage.md#the-build-has-no-s3-support)),
  so a server whose objects are in a bucket cannot go back to 0.3.0.

CI runs this page with the objects in the data directory only.

## When a step fails

- **`flock` exits 1 and the copy is not made:** the server, or a command,
  still holds the data directory. [Stop the server](#stop-the-server) and
  run the copy again.
- **`check` with the new binary exits 1:** its report names each problem.
  Do not start the server on that data. Install the old binary again and
  put back the copy, as in
  [go back to the old version](#go-back-to-the-old-version).
- **The new server refuses to start, and names a secret file:** from 0.4.1
  a secret file that other users may read, that its group may change, or
  that another user owns stops the server; see
  [from 0.4.0 to 0.4.1](#from-0-4-0-to-0-4-1-the-secret-file-rule).
- **The old binary stops with `schema version 9 is not one this build knows`
  (or `schema version 4` or `3`, for a binary older than 0.4.0):**
  a newer binary has opened the data directory. The old one reads only the
  copy taken before the upgrade; see
  [go back to the old version](#go-back-to-the-old-version).
- **A failed request after the start:** see [when a request fails](@/docs/getting-started.md#when-a-request-fails).

## Clean up

Revoke the key this page made:

```sh
KEY_ID=$(sudo -u vcon vcon-server key list --tenant "$TENANT" --config $CONFIG | jq -r 'select(.label == "upgrade-check") | .id')
sudo -u vcon vcon-server key revoke "$KEY_ID" --config $CONFIG
```
