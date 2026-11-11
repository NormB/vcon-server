+++
title = "Read the audit logs"
description = "Read a tenant's audit log and the admin log, with a command on the host and over HTTPS a page at a time, and list who revealed which part of a vCon. Every command and what it prints is shown."
weight = 33

[extra]
group = "how-to"
+++

Every request leaves a record in an audit log: each tenant has one, and the
server has one for its own administration. This how-to reads both, on the
server's host and over HTTPS, and lists the reveals of one vCon.
[Audit logs](@/docs/audit-logs.md) explains what is recorded, and
[audit log records](@/docs/audit-log-records.md) lists a record's members
and every action.

CI runs every command on this page against a live server, after
[getting started](@/docs/getting-started.md), and checks each answer.

## Before you start

- A running server, set up as in [getting started](@/docs/getting-started.md),
  with the tenant `acme`, and the certificate's copy for clients at
  `/usr/local/share/vcon-server/ca.pem`.
- `curl` and `jq` on the machine you read from. Reading a log changes
  nothing on any other system.
- One shell for every step: later steps use variables that earlier steps set.

## Make a key that may read the log

A tenant's log is read over HTTPS with a key of that tenant that holds the
`audit` scope. Make one, and a second key that stores and reads vCons, so
that the log has records to find. Keep each key's id: the log names a key
by it.

```sh
CONFIG=/etc/vcon-server/vcon-server.toml
URL=https://vcon.example.com:8443
TENANT=$(sudo -u vcon vcon-server tenant list --config $CONFIG | jq -r 'select(.name == "acme") | .id')
CREATED=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope audit --label audit-reader --config $CONFIG)
AUDITOR=$(echo "$CREATED" | jq -r .token)
AUDITOR_ID=$(echo "$CREATED" | jq -r .id)
CREATED=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope write --scope read --label audit-worker --config $CONFIG)
WORKER=$(echo "$CREATED" | jq -r .token)
WORKER_ID=$(echo "$CREATED" | jq -r .id)
unset CREATED
```

## Make records to find

Store a vCon with the second key, read it, and read it again as a
[reveal](@/docs/audit-logs.md#reveals) of its parties:

```sh
UUID=$(cat /proc/sys/kernel/random/uuid)
cat > audit.vcon <<EOF
{"vcon":"0.4.0","uuid":"$UUID","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550101","name":"Caller Example"}]}
EOF
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem "$URL/vcons" \
  -H "Authorization: Bearer $WORKER" -H "Content-Type: application/vcon" --data-binary @audit.vcon
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem "$URL/vcons/$UUID" \
  -H "Authorization: Bearer $WORKER"
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem "$URL/vcons/$UUID?reveal=parties" \
  -H "Authorization: Bearer $WORKER"
```

```text
201
200
200
```

The `reveal` parameter is new in 0.4.3. Release 0.4.2 answers the third
request the same way and records it as a second `read`, so with 0.4.2 each
`reveal_parties` below is a `read`.

## Export a log on the host

`vcon-server audit export` writes a log as JSON lines, oldest first. It runs
beside the server, as the service account, and is not itself recorded. The
tenant's log holds the three requests:

```sh
sudo -u vcon vcon-server audit export --tenant "$TENANT" --config $CONFIG \
  | jq -c --arg uuid "$UUID" 'select(.target == $uuid) | {action, status}'
```

```text
{"action":"create","status":201}
{"action":"read","status":200}
{"action":"reveal_parties","status":200}
```

A whole record names the key that asked, by its id, and never holds the
vCon's content or a token:

```sh
sudo -u vcon vcon-server audit export --tenant "$TENANT" --config $CONFIG \
  | jq -c --arg uuid "$UUID" 'select(.target == $uuid and .action == "create")'
```

```text
{"action":"create","actor":"<key id>","at":"<time>","count":1,"seq":<seq>,"status":201,"target":"<uuid>","version":"<version>"}
```

`--since <seq>` writes only the records after that one. Keep the last `seq`
you read, and pass it the next time. Nothing was recorded since the export
above, so this prints no record:

```sh
LAST=$(sudo -u vcon vcon-server audit export --tenant "$TENANT" --config $CONFIG | jq -s '.[-1].seq')
sudo -u vcon vcon-server audit export --tenant "$TENANT" --since "$LAST" --config $CONFIG | jq -s length
```

```text
0
```

`--admin` writes the admin log in place of a tenant's. It holds the making
of the first key, by a command on the host (actor `cli`):

```sh
sudo -u vcon vcon-server audit export --admin --config $CONFIG \
  | jq -c --arg id "$AUDITOR_ID" 'select(.target == $id) | {action, actor, status}'
```

```text
{"action":"key_create","actor":"cli","status":201}
```

## Read a log over HTTPS

`GET /audit` answers the same lines to a key with the `audit` scope, from
any machine that reaches the server:

```sh
curl -sS --cacert /usr/local/share/vcon-server/ca.pem "$URL/audit?limit=10000" -H "Authorization: Bearer $AUDITOR" \
  | jq -c --arg uuid "$UUID" 'select(.target == $uuid) | {action, status}'
```

```text
{"action":"create","status":201}
{"action":"read","status":200}
{"action":"reveal_parties","status":200}
```

A page holds at most `limit` records (1000 unless given, 10000 at most).
To copy a whole log, ask for the page after the last `seq` you read, until
a page is shorter than the limit:

```sh
after=0
: > audit.jsonl
while :; do
  page=$(curl -sS --cacert /usr/local/share/vcon-server/ca.pem "$URL/audit?after=$after" -H "Authorization: Bearer $AUDITOR")
  [ -n "$page" ] || break
  printf '%s\n' "$page" >> audit.jsonl
  [ "$(printf '%s\n' "$page" | wc -l)" -lt 1000 ] && break
  after=$(printf '%s\n' "$page" | tail -n 1 | jq .seq)
done
jq -c --arg uuid "$UUID" 'select(.target == $uuid) | .action' audit.jsonl
```

```text
"create"
"read"
"reveal_parties"
```

Each read over HTTPS is itself recorded, as `audit_read`, in the log it
read, with the id of the key that read:

```sh
curl -sS --cacert /usr/local/share/vcon-server/ca.pem "$URL/audit?limit=10000" -H "Authorization: Bearer $AUDITOR" \
  | jq -c --arg id "$AUDITOR_ID" 'select(.actor == $id) | {action, status}' | sort -u
```

```text
{"action":"audit_read","status":200}
```

A key without the `audit` scope is refused with `403`, and an operator key
reads the admin log at `GET /admin/audit` in the same way:

```sh
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem "$URL/audit" -H "Authorization: Bearer $WORKER"
```

```text
403
```

## See who revealed what

A client that shows a vCon masked, as the [console](@/docs/console.md)
does, marks each read a person asks for with the part it is about to show.
List the reveals of one vCon, with the part, the status answered and the id
of the key that asked:

```sh
curl -sS --cacert /usr/local/share/vcon-server/ca.pem "$URL/audit?limit=10000" -H "Authorization: Bearer $AUDITOR" \
  | jq -r --arg uuid "$UUID" 'select(.target == $uuid and (.action | test("^(legal_)?reveal_"))) | "\(.action) \(.status) \(.actor)"'
```

```text
reveal_parties 200 <key id>
```

The id is the worker key's:

```sh
[ "$(jq -r --arg uuid "$UUID" 'select(.target == $uuid and .action == "reveal_parties") | .actor' audit.jsonl)" = "$WORKER_ID" ] && echo "the worker key"
```

```text
the worker key
```

In each line, `action` ends with the part shown, `at` is when, `version` is
the version read, and `status` is what the server answered. If the answer
held 10,000 lines, read on from the last `seq` with `after=<seq>`, as
above. To check it against the console, open the vCon there, choose
**Reveal parties**, and run the command again: one more line appears, with
the id of the key you signed in with.

## When a step fails

- **A `curl` error, or `401` or `503` where this page shows another
  answer:** see [when a request fails](@/docs/getting-started.md#when-a-request-fails).
- **`403` from `/audit`:** the key has no `audit` scope. `GET /key` with
  that key shows its scopes.
- **`400`, [`bad-query`](@/problems/bad-query.md), from `/audit`:** a
  parameter the route does not take, one given twice, `limit` outside 1 to
  10000, or `after` that is not a number.
- **A reveal shows as `read`:** the server is release 0.4.2, which has no
  `reveal` parameter, or the client read without the marker. A console
  loaded before the server was upgraded reads without it: reload the page.
- **A reveal's line has status `410` or `404` and no version:** the reveal
  was refused. The vCon was deleted or was never stored, and the record
  shows what was asked for.
- **A read with `?reveal=` answers `400`,
  [`bad-query`](@/problems/bad-query.md):** the marker was not the only
  parameter, was given twice or names no part. The refusal is recorded as a
  `read` (or `legal_read`) with status 400, and no vCon is sent.
- **`audit export` stops with `the output could not be written: Broken pipe`:**
  the command it was piped to, such as `head`, stopped reading. The log is
  unchanged.

## Clean up

Delete the files, and revoke the two keys this page made:

```sh
rm audit.vcon audit.jsonl
sudo -u vcon vcon-server key revoke "$AUDITOR_ID" --config $CONFIG
sudo -u vcon vcon-server key revoke "$WORKER_ID" --config $CONFIG
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem "$URL/audit" -H "Authorization: Bearer $AUDITOR"
```

```text
401
```
