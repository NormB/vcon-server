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
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
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
NEW_VERSION=0.2.0
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
timeout 30 sh -c 'until curl -s -o /dev/null --cacert fullchain.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
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
curl -sS --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY" -o upgraded.vcon
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

## Clean up

Revoke the key this page made:

```sh
KEY_ID=$(sudo -u vcon vcon-server key list --tenant "$TENANT" --config $CONFIG | jq -r 'select(.label == "upgrade-check") | .id')
sudo -u vcon vcon-server key revoke "$KEY_ID" --config $CONFIG
```
