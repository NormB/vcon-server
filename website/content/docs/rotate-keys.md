+++
title = "Revoke and rotate keys"
description = "Revoke an API key over HTTP or on the host, replace a key without a gap, and rotate a tenant's key and the master key, with every command and its output."
weight = 15
+++

Two kinds of key need replacing from time to time:

- **API keys**, the bearer tokens that clients send. Revoking one refuses its
  next request. Replacing one is: make the new key, move the client to it,
  revoke the old one.
- **Encryption keys**: each tenant's key and the master key above them. A
  purge erases a version from the live system, but a copy made earlier (an
  old disk, a backup) still holds it. Rotating the keys and destroying the
  old ones makes those copies unreadable.

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
OPS=$(sudo -u vcon vcon-server key create --operator --label rotate-ops --config $CONFIG | jq -r .token)
```

## Revoke a key over HTTP

Make a key to revoke, and keep its id and token. The id is the 16
hexadecimal digits after `vcs_` in the token, and is not secret:

```sh
CREATED=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label rotate-old --config $CONFIG)
OLD_ID=$(echo "$CREATED" | jq -r .id)
OLD_KEY=$(echo "$CREATED" | jq -r .token)
unset CREATED
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $OLD_KEY"
```

```text
200
```

`DELETE /admin/keys/{id}`, with the operator key, revokes it. The answer is
`204`:

```sh
curl -sS -i --cacert fullchain.pem -X DELETE "$URL/admin/keys/$OLD_ID" -H "Authorization: Bearer $OPS"
```

```text
HTTP/2 204
date: <date>
```

Its next request is refused with `401`:

```sh
curl -sS --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $OLD_KEY"
```

```text
{"status":401,"title":"A valid bearer key is needed","type":"https://vcon-server.com/problems/unauthorized"}
```

A request already under way when the key is revoked finishes. Revoking it
again answers `204` too. The key list keeps the key, with the time it was
revoked:

```sh
curl -sS --cacert fullchain.pem "$URL/admin/tenants/$TENANT/keys" -H "Authorization: Bearer $OPS" \
  | jq -c --arg id "$OLD_ID" '.[] | select(.id == $id) | {label, revoked: (.revoked_at != null)}'
```

```text
{"label":"rotate-old","revoked":true}
```

Only the operator key may revoke. A tenant key gets `403`:

```sh
SPARE=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label rotate-spare --config $CONFIG | jq -r .token)
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/admin/keys/$OLD_ID" -H "Authorization: Bearer $SPARE"
```

```text
403
```

## Revoke a key on the host

`vcon-server key revoke` does the same on the server's host, with the
server running or not. `key list` finds a key's id by its label:

```sh
SPARE_ID=$(sudo -u vcon vcon-server key list --tenant "$TENANT" --config $CONFIG | jq -r 'select(.label == "rotate-spare") | .id')
sudo -u vcon vcon-server key revoke "$SPARE_ID" --config $CONFIG
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $SPARE"
```

```text
401
```

The server sees the change at its next request. `key revoke` prints nothing
and exits 0.

## Replace a key without a gap

A client keeps working while its key is replaced, when the new key exists
before the old one goes:

1. Make the new key, with the same scopes and a label that says it replaces
   the old one:

   ```sh
   CURRENT=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label rotate-current-1 --config $CONFIG | jq -r .token)
   NEXT=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label rotate-current-2 --config $CONFIG | jq -r .token)
   ```

2. Give the client the new token, and check that the client uses it.
   `GET /key` shows which key a token is:

   ```sh
   curl -sS --cacert fullchain.pem "$URL/key" -H "Authorization: Bearer $NEXT" | jq -r .scopes[]
   ```

   ```text
   read
   ```

3. Revoke the old key. The [audit log](@/docs/audit-logs.md) records each
   request with the key that sent it, so a request still made with the old key
   shows there before you revoke it.

   ```sh
   CURRENT_ID=$(sudo -u vcon vcon-server key list --tenant "$TENANT" --config $CONFIG | jq -r 'select(.label == "rotate-current-1") | .id')
   curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem -X DELETE "$URL/admin/keys/$CURRENT_ID" -H "Authorization: Bearer $OPS"
   curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $CURRENT"
   curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $NEXT"
   ```

   ```text
   204
   401
   200
   ```

## Rotate the tenant key and the master key

A purge erases a version from the live system. A copy made earlier still
holds its sealed file and its wrapped key, readable with the tenant key and
the master key of that time. Rotation gives the tenant a new key and the
server a new master key, then the old master key is destroyed. After that,
no copy taken before the rotation can be read. [Operations](@/docs/operations.md#key-rotation)
explains when to rotate. `check` shows how many purges wait for rotation.

First store a vCon, to read back after the rotation:

```sh
WRITER=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope write --scope read --label rotate-writer --config $CONFIG | jq -r .token)
UUID=$(cat /proc/sys/kernel/random/uuid)
printf '{"vcon":"0.4.0","uuid":"%s","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550104"}],"dialog":[],"attachments":[],"analysis":[]}' \
  "$UUID" > rotate.vcon
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" \
  -H "Authorization: Bearer $WRITER" -H "Content-Type: application/vcon" --data-binary @rotate.vcon
```

```text
201
```

The rotation commands run only with the server stopped. Stop it, under
systemd:

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

A rotation command refuses while the server runs, with
`the data directory is in use; stop the server and any other command first`.

`check` reports, for each tenant, the purges made since its key was last
rotated:

```sh
sudo -u vcon vcon-server check --config $CONFIG | jq -c 'select(.name != null) | {name, purges_awaiting_rotation}'
```

```text
{"name":"acme","purges_awaiting_rotation":0}
```

Rotate the key of each tenant with purges waiting. This page rotates acme's
to show the step. `rekey tenant` prints nothing:

```sh
sudo -u vcon vcon-server rekey tenant "$TENANT" --config $CONFIG
```

Make a new master key file, under a name not used before, and rotate the
master key to it. It prints the new key's id and what to do next:

```sh
OLD_MASTER=$(sudo sed -n 's/^master_key = "\(.*\)"$/\1/p' $CONFIG)
NEW_MASTER=/etc/vcon-server/master-$(date -u +%Y%m%dT%H%M%SZ).key
head -c 32 /dev/urandom | sudo -u vcon tee "$NEW_MASTER" >/dev/null
sudo chmod 0400 "$NEW_MASTER"
sudo -u vcon vcon-server rekey master --new-master "$NEW_MASTER" --config $CONFIG
```

```text
{"master_key_id":"<new master key id>","next":"point master_key in the configuration at the new key file, start the server, then destroy every copy of the old master key"}
```

Until the configuration names the new key, the server and every command
refuse, because the data is now under the new key:

<!-- docs-test: exit 1 -->
```sh
sudo -u vcon vcon-server check --config $CONFIG
```

```text
vcon-server: the master key is not the one server.sqlite was made under
```

Point `master_key` at the new file:

```sh
sudo sed -i "s|^master_key = .*|master_key = \"$NEW_MASTER\"|" $CONFIG
sudo -u vcon vcon-server check --config $CONFIG | jq -c 'select(.name == null) | {problems}'
```

```text
{"problems":0}
```

While the server is still stopped, back up the new master key and the data
directory (see [back up and restore](@/docs/backup-and-restore.md)). Then
start the server, under systemd:

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

The vCon stored before the rotation reads back the same:

```sh
curl -sS --cacert fullchain.pem "$URL/vcons/$UUID" -H "Authorization: Bearer $WRITER" -o rotated.vcon
cmp rotated.vcon rotate.vcon && echo "the same bytes"
```

```text
the same bytes
```

Destroy every copy of the old master key, backups included:

```sh
sudo rm "$OLD_MASTER"
sudo ls /etc/vcon-server
```

```text
cert.pem
key.pem
<new master key file>
vcon-server.toml
```

From this step on, data purged before the rotation cannot be read from any
copy taken before it. A rotation step that stops part way is finished by
running it again. The test suite also plays this procedure end to end with
the binary (`crates/vcon-server/tests/rotation.rs`).

## Clean up

Revoke the keys this page made:

```sh
sudo -u vcon vcon-server key list --config $CONFIG | jq -r 'select(.label | startswith("rotate-")) | select(.revoked_at == null) | .id' \
  | while read -r id; do sudo -u vcon vcon-server key revoke "$id" --config $CONFIG; done
curl -sS -o /dev/null -w '%{http_code}\n' --cacert fullchain.pem "$URL/vcons" -H "Authorization: Bearer $NEXT"
```

```text
401
```
