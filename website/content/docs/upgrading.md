+++
title = "Upgrade"
description = "Move a running server to a newer release: check the new binary, stop the server, copy the data directory, install the binary, check the data with it, start it, and go back to the old version if you need to."
weight = 18
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
NEW_VERSION=0.4.1
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
`/backup` stands for your backup storage:

```sh
sudo install -d -m 0700 /backup
BEFORE=/backup/vcon-server-before-$NEW_VERSION.tar
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

## Clean up

Revoke the key this page made:

```sh
KEY_ID=$(sudo -u vcon vcon-server key list --tenant "$TENANT" --config $CONFIG | jq -r 'select(.label == "upgrade-check") | .id')
sudo -u vcon vcon-server key revoke "$KEY_ID" --config $CONFIG
```
