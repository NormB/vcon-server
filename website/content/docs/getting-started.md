+++
title = "Getting started"
description = "From an installed binary to a running server: the service account, memory locking, the master key, a certificate, the configuration, the first tenant and its keys, and the server started under systemd or in the foreground."
weight = 10
+++

This page takes a host with `vcon-server` installed to a running server that
answers a health check. Paste the commands in order, in one shell:
later steps use shell variables that earlier steps set. CI runs
the page from top to bottom on each change. It runs in a fresh Debian 13
container and a fresh Fedora 44 container, each with the server in the
foreground, and in a Debian 13 container under systemd.

## Before you start

- **The binary.** `vcon-server --version` prints its version. If it does
  not, follow [install](@/docs/install.md) first.
- **`sudo`, `curl`, `openssl` and `jq`.** The steps below use all four. A
  minimal Debian or Ubuntu has none of them. `procps` provides `kill`, which
  stops a server that runs without systemd. As root, on Debian or Ubuntu:

  <!-- docs-test: only apt -->
  ```sh
  apt-get update -qq
  apt-get install -y -qq sudo curl openssl jq ca-certificates procps >/dev/null
  ```

  On Fedora, as root. There, `procps-ng` provides `pidof`, and `diffutils`
  provides `cmp`, which compares a vCon read back with the one sent:

  <!-- docs-test: only dnf -->
  ```sh
  dnf install -y -q sudo curl openssl jq procps-ng diffutils
  ```

- **A name for the server.** Clients reach the server by a DNS name that its
  certificate holds. This page uses `vcon.example.com`. Write your own name
  wherever it appears.

## Make the service account and its directories

The server runs as its own account, `vcon`. That account owns the data
directory and the files in `/etc/vcon-server`:

```sh
sudo useradd --system --user-group --no-create-home --home-dir /var/lib/vcon-server --shell /usr/sbin/nologin vcon
sudo install -d -m 0700 -o vcon -g vcon /var/lib/vcon-server
sudo install -d -m 0700 -o vcon -g vcon /etc/vcon-server
```

The server makes everything inside the data directory.

## Allow it to lock its memory

At start the server locks all of its memory, so that no key or vCon body can
reach swap, and it turns core dumps off. It refuses to start if it cannot
lock its memory. Every other `vcon-server` command does the same, so the
account that runs them needs the same allowance. The
[systemd unit](#start-it-with-systemd) below allows it for the server. For
the commands you run by hand, allow it for the `vcon` account:

```sh
echo 'vcon - memlock unlimited' | sudo tee /etc/security/limits.d/vcon-server.conf
```

```text
vcon - memlock unlimited
```

`sudo -u vcon` then applies it, through `pam_limits`. Check it:

```sh
sudo -u vcon bash -c 'ulimit -l'
```

```text
unlimited
```

Where the limit stays too small, every command stops with this error:

```text
vcon-server: memory cannot be locked: Cannot allocate memory (os error 12); allow it, for example LimitMEMLOCK=infinity
```

In a container, the limits file cannot raise the limit past the container's
own. Start the container with `--ulimit memlock=-1`.

Run every command as `vcon`, with `sudo -u vcon`, never as root. A command
run as root leaves files in the data directory that `vcon` cannot open. The
server then refuses to start, with
`the data directory cannot be locked: Permission denied (os error 13)`, or,
for a tenant made as root,
`1 tenant catalog(s) cannot be opened: Permission denied (os error 13)`.
`vcon-server check` names the path. Give the files back to `vcon`:

<!-- docs-test: skip a repair, for after a command was run as root -->
```sh
sudo chown -R vcon:vcon /var/lib/vcon-server
```

## Make the master key

The master key is a file of exactly 32 random bytes. It wraps every tenant
key and opens the server's own catalog, so whoever holds it can read
everything. Keep it on the server, readable only by the service account, and
back it up apart from the data (see [back up and restore](@/docs/operations.md#back-up-and-restore)).

```sh
head -c 32 /dev/urandom | sudo -u vcon tee /etc/vcon-server/master.key >/dev/null
sudo chmod 0400 /etc/vcon-server/master.key
sudo stat -c '%a %U %s' /etc/vcon-server/master.key
```

```text
400 vcon 32
```

The server refuses to start with a file of any other length. It also
refuses a key other than the one it made the data directory under.

The key's id is the first 16 hexadecimal digits of its SHA-256. It is the
`master_key_id` that `vcon-server rekey master` prints. Label each copy of
the key with it:

```sh
sudo sha256sum /etc/vcon-server/master.key | cut -c1-16
```

```text
<master key id>
```

## Get a certificate

The API is served over TLS only, with HTTP/2 and HTTP/1.1. The server needs
a PEM certificate chain and its PEM private key, readable by the service
account. With a certificate from your certificate authority or ACME client,
install the two files as `cert.pem` and `key.pem`:

<!-- docs-test: skip the run makes a self-signed certificate below instead -->
```sh
sudo install -m 0400 -o vcon -g vcon fullchain.pem /etc/vcon-server/cert.pem
sudo install -m 0400 -o vcon -g vcon privkey.pem /etc/vcon-server/key.pem
```

The server reads them at start only. After a renewal, install the new files
and restart it.

To try the server before you have a certificate, make a self-signed one for
the name that clients use, and install it the same way:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 30 \
  -subj /CN=vcon.example.com -addext subjectAltName=DNS:vcon.example.com \
  -keyout privkey.pem -out fullchain.pem 2>/dev/null
sudo install -m 0400 -o vcon -g vcon fullchain.pem /etc/vcon-server/cert.pem
sudo install -m 0400 -o vcon -g vcon privkey.pem /etc/vcon-server/key.pem
openssl x509 -in fullchain.pem -noout -subject -ext subjectAltName
```

```text
subject=CN=vcon.example.com
X509v3 Subject Alternative Name:
    DNS:vcon.example.com
```

Clients then need `fullchain.pem` to trust it: the `curl` commands on these
pages pass it with `--cacert fullchain.pem`. With a certificate from a public
certificate authority, leave `--cacert fullchain.pem` out.

## Write the configuration

The configuration is one TOML file. It must set these keys. Every other
key has a default (see [configuration](@/docs/configuration.md)). Save this
as `vcon-server.toml`:

<!-- docs-test: file vcon-server.toml -->
```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"
```

Install it where the server reads it:

```sh
sudo install -m 0400 -o vcon -g vcon vcon-server.toml /etc/vcon-server/vcon-server.toml
```

## Make the first tenant and its keys

Every vCon belongs to a tenant, and every request carries a bearer key. Make
the first tenant before you start the server: `tenant create` runs only
while no server does. It prints the new tenant's id:

```sh
sudo -u vcon vcon-server tenant create acme --config /etc/vcon-server/vcon-server.toml
```

```text
{"id":"<tenant id>"}
```

Keep the id in a variable for the next steps:

```sh
TENANT=$(sudo -u vcon vcon-server tenant list --config /etc/vcon-server/vcon-server.toml | jq -r 'select(.name == "acme") | .id')
echo "$TENANT"
```

```text
<tenant id>
```

Then make three keys. Each `key create` prints one JSON line, such as
`{"id":"5b0c3f9a1d2e4f60","token":"vcs_5b0c3f9a1d2e4f60_…"}`. The commands
below keep each token in a variable. The server shows a token this once
and keeps only its hash, so store each one where its holder keeps secrets.

- An **operator key** (scope `admin`) manages tenants and keys over the
  [admin routes](@/docs/http-api.md#admin). It never reaches a vCon.
- A **write** key, `recorder-1`, for a producer that stores vCons.
- A **read** key, `monitor`, for the [health check](@/docs/operations.md#health-checks).

```sh
OPS=$(sudo -u vcon vcon-server key create --operator --label ops --config /etc/vcon-server/vcon-server.toml | jq -r .token)
RECORDER=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope write --label recorder-1 \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
MONITOR=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope read --label monitor \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
sudo -u vcon vcon-server key list --config /etc/vcon-server/vcon-server.toml | jq -c '{label, scopes}'
```

```text
{"label":"ops","scopes":["admin"]}
{"label":"recorder-1","scopes":["write"]}
{"label":"monitor","scopes":["read"]}
```

`key create` also runs while the server runs. Once the server runs, more
tenants come from `POST /admin/tenants`, with the operator key. See the
[admin routes](@/docs/http-api.md#admin). The
[command line](@/docs/command-line.md) and
[tenants and keys](@/docs/tenants-and-keys.md) pages describe every command
and scope.

## Start it with systemd

On a host with systemd, save this unit as
`/etc/systemd/system/vcon-server.service`. `LimitMEMLOCK=infinity` allows
the server to lock its memory:

<!-- docs-test: only systemd; file /etc/systemd/system/vcon-server.service -->
```ini
[Unit]
Description=vcon-server
After=network-online.target

[Service]
User=vcon
Group=vcon
ExecStart=/usr/local/bin/vcon-server serve --config /etc/vcon-server/vcon-server.toml
LimitMEMLOCK=infinity
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Start it, now and at each boot:

<!-- docs-test: only systemd -->
```sh
sudo systemctl daemon-reload
sudo systemctl enable --now vcon-server
```

The server logs structured lines to standard error, which the journal keeps.
It logs `serving` once it listens, a line per request, and `stopped` after
SIGTERM or SIGINT. Wait for the `serving` line:

<!-- docs-test: only systemd -->
```sh
timeout 30 sh -c 'until sudo journalctl -u vcon-server -o cat | grep -q serving; do sleep 1; done'
sudo journalctl -u vcon-server -o cat | grep vcon_server
```

```text
<time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443
<time>  INFO vcon_server::serve: purge sweep purged=0 failed=0
```

`systemctl status vcon-server` shows whether it runs. When it does not,
`journalctl -u vcon-server` shows why.

## Start it without systemd

On a host or in a container without systemd, run the server as `vcon` in
the foreground. The [limits file](#allow-it-to-lock-its-memory) allows it to
lock its memory:

<!-- docs-test: only no-systemd; background serving -->
```sh
sudo -u vcon vcon-server serve --config /etc/vcon-server/vcon-server.toml
```

```text
<time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443
<time>  INFO vcon_server::serve: purge sweep purged=0 failed=0
…
<time>  INFO vcon_server::serve: stopped
```

It keeps the terminal and logs there. Leave it running, and open a second
terminal for the next steps. To keep it running after you log out, run it
under your init system or a supervisor, with the same command, as `vcon`,
and with a memory-lock limit of `unlimited`.

## When it does not start

It refuses to start, with the reason on standard error, when:

- the configuration is wrong. The error names the key.
- the master key is not the data directory's.
- memory cannot be locked, or core dumps cannot be turned off.
- another server already holds the data directory.
- a tenant's data is not all under its current key, because a key rotation
  stopped part way. Run the rotation again; `vcon-server check` names the
  tenants.

## Check that it answers

The certificate names `vcon.example.com`. Until DNS gives that name to this
host, add it to `/etc/hosts` on the host you test from:

```sh
echo '127.0.0.1 vcon.example.com' | sudo tee -a /etc/hosts
```

```text
127.0.0.1 vcon.example.com
```

Ask for the server's health with the `monitor` key:

```sh
curl -sS --cacert fullchain.pem https://vcon.example.com:8443/health -H "Authorization: Bearer $MONITOR"
```

```text
{"status":"ok"}
```

Without a key, the answer is `401`, as an RFC 9457 problem document:

```sh
curl -sS --cacert fullchain.pem https://vcon.example.com:8443/health
```

```text
{"status":401,"title":"A valid bearer key is needed","type":"https://vcon-server.com/problems/unauthorized"}
```

## Stop it and start it again

SIGTERM or SIGINT stops the server. It logs `stopped` and exits. Under
systemd:

<!-- docs-test: only systemd -->
```sh
sudo systemctl stop vcon-server
sudo journalctl -u vcon-server -o cat | grep -c 'vcon_server::serve: stopped'
```

```text
1
```

In the foreground, press Ctrl-C in its terminal. From another terminal, send
it SIGTERM and wait until it has exited:

<!-- docs-test: only no-systemd -->
```sh
sudo kill -TERM "$(pidof vcon-server)"
while pidof vcon-server >/dev/null; do sleep 0.2; done
```

Every write is crash-safe, so a kill or a power cut loses at most the
requests not yet answered. Start it again, under systemd. `systemctl start`
returns before the server listens, so the second command waits until it
answers:

<!-- docs-test: only systemd -->
```sh
sudo systemctl start vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert fullchain.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
```

```text
answering
```

Or in the foreground:

<!-- docs-test: only no-systemd; background serving -->
```sh
sudo -u vcon vcon-server serve --config /etc/vcon-server/vcon-server.toml
```

```text
<time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443
…
```

The tenant and the keys are still there:

```sh
curl -sS --cacert fullchain.pem https://vcon.example.com:8443/health -H "Authorization: Bearer $MONITOR"
```

```text
{"status":"ok"}
```

## Next

- [Store your first vCon](@/docs/first-vcon.md): store, read, list and
  delete a vCon, with a key for each scope.
- [The how-to guides](@/docs/how-to.md): every guide, and the ones not
  written yet.
- [Operations](@/docs/operations.md): the data directory, logs, health
  checks, backups and key rotation.
- [Configuration](@/docs/configuration.md): every key, and its default.
