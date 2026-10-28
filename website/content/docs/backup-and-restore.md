+++
title = "Back up and restore"
description = "Back up the master key and the data directory while the server runs, restore the data directory, check it, and see what a restore brings back, revoked keys included."
weight = 16
+++

A backup is two things, kept in different places: the data directory, which
changes with every request, and the master key file, which changes only at
a [key rotation](@/docs/rotate-keys.md). Data without its master key is
unreadable. This how-to backs up both while the server runs, then restores
the data directory and shows what the restore brings back.
[Operations](@/docs/operations.md#back-up-and-restore) describes how
`backup` copies, the other ways to copy the data directory, and what a key
rotation does to old backups.

CI runs every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A running server, set up as in [getting started](@/docs/getting-started.md),
  with the tenant `acme`, and `fullchain.pem` in the current directory.
- One shell for every step: later steps use variables that earlier steps set.

```sh
URL=https://vcon.example.com:8443
CONFIG=/etc/vcon-server/vcon-server.toml
TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
KEY=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope write --scope read --scope delete --label backup-app \
  --config $CONFIG | jq -r .token)
```

Store a vCon, so that the backup holds one this page can look for:

```sh
UUID=$(cat /proc/sys/kernel/random/uuid)
printf '{"vcon":"0.4.0","uuid":"%s","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550105"}],"dialog":[],"attachments":[],"analysis":[]}' \
  "$UUID" > backup.vcon
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $KEY" -H "Content-Type: application/vcon" --data-binary @backup.vcon
```

```text
201
```

## Back up the master key

Back up the master key when it is made and after each rotation, to storage
that does not hold the data backups. The configuration names the file. Label
the copy with the key's id, the first 16 hexadecimal digits of its SHA-256:

```sh
MASTER=$(sudo sed -n 's/^master_key = "\(.*\)"$/\1/p' $CONFIG)
MASTER_ID=$(sudo sha256sum "$MASTER" | cut -c1-16)
echo "$MASTER_ID"
```

```text
<master key id>
```

Copy it there. `/srv/key-copies` stands for that storage, such as a drive
kept offline:

```sh
sudo install -D -m 0400 "$MASTER" "/srv/key-copies/vcon-server-master-$MASTER_ID.key"
sudo sha256sum "/srv/key-copies/vcon-server-master-$MASTER_ID.key" | cut -c1-16
```

```text
<master key id>
```

The two ids match. Keep a list of every copy, so that each one can be found
and destroyed at the next rotation.

## Back up the data directory

`vcon-server backup` writes a copy of the data directory to a new directory
while the server runs. Run it as the service account. The directory it
names must not exist yet, and the service account must be able to write the
directory that holds it. `/backup` stands for your backup storage:

```sh
sudo install -d -m 0700 -o vcon -g vcon /backup/vcon-server
BACKUP=/backup/vcon-server/data-$(date -u +%Y%m%dT%H%M%SZ)
sudo -u vcon vcon-server backup "$BACKUP" --config $CONFIG
```

```text
{"tenants":<tenants>,"tries":1,"versions":<versions>}
```

It copies each catalog as one commit left it, then every file the copy names.
Requests and the purge sweep go on meanwhile. The copy is mode `0700`, holds
nothing readable without the master key, and does not hold the master key:

```sh
sudo stat -c '%a %U' "$BACKUP"
sudo ls "$BACKUP"
```

```text
700 vcon
server.sqlite
tenants
```

A target the service account cannot write is refused, and nothing is
copied:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server backup /root/data-copy --config $CONFIG
```

```text
vcon-server: the backup could not be written, or the data could not be read
```

## Change something after the backup

Delete the vCon, and revoke the key, so the restore has something to bring
back:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY"
KEY_ID=$(sudo -u vcon vcon-server key list --tenant "$TENANT" --config $CONFIG | jq -r 'select(.label == "backup-app") | .id')
sudo -u vcon vcon-server key revoke "$KEY_ID" --config $CONFIG
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY"
```

```text
204
401
```

## Restore the data directory

Restore with the server stopped, under the master key that was current when
the backup was taken. Stop it, under systemd:

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

Move the current data directory aside, copy the backup into its place, and
check it before starting. `cp -a`, run as root, keeps the files' owner and
modes:

```sh
sudo mv /var/lib/vcon-server /var/lib/vcon-server.old
sudo cp -a "$BACKUP" /var/lib/vcon-server
sudo -u vcon vcon-server check --config $CONFIG | jq -c 'select(.name == null) | {problems}'
```

```text
{"problems":0}
```

`check` opens every stored version. It exits 1 when it finds a problem, and
its report says which. Under any master key other than the backup's, it
refuses with
`vcon-server: the master key is not the one server.sqlite was made under`.

Start the server, under systemd:

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

## What the restore brings back

The data directory is as it was when the backup was taken. The vCon deleted
after the backup is back, and **the key revoked after the backup is valid
again**:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY"
```

```text
200
```

Revoke such a key again at once. The admin log records each revocation as
`key_revoke`, so an `audit export --admin` lists the keys to revoke:

```sh
sudo -u vcon vcon-server audit export --admin --config $CONFIG | jq -r 'select(.action == "key_revoke") | .target' | sort -u
```

```text
…
```

That export is read from the restored data, so it lists only the
revocations made before the backup. The keys revoked after it are in an
export taken before the restore, or in your own records. Here it is the key
this page revoked:

```sh
sudo -u vcon vcon-server key revoke "$KEY_ID" --config $CONFIG
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $KEY"
```

```text
401
```

Also gone after a restore: the vCons, tenants and keys made after the
backup, with their audit records. Versions in the backup that are due for
purge are purged again by the sweep at start.

Once the restored server is checked, remove the old data directory:

```sh
sudo rm -rf /var/lib/vcon-server.old
```
