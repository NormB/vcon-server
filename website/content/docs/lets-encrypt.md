+++
title = "Get a Let's Encrypt certificate"
description = "Replace the self-signed certificate with a free, publicly trusted one from Let's Encrypt, by HTTP-01 or DNS-01 with certbot, and renew it automatically with a hook that installs it for the server and restarts it."
weight = 35

[extra]
group = "how-to"
+++

Every request to vcon-server travels over TLS, so every client has to trust
the server's certificate. A certificate from
[Let's Encrypt](https://letsencrypt.org/) is free and is trusted by every
system's default roots. Clients then need no CA file: no `--cacert` for
`curl`, and no `ca` in the files that
[`vcon-server tui`](@/docs/tui.md) and [`vcon-server mcp`](@/docs/mcp.md)
read.

This page gets one with [certbot](https://certbot.eff.org/) and keeps it
renewed. A Let's Encrypt certificate is valid for 90 days at most, and
certbot renews it before then. The server reads its certificate
[at start only](@/docs/getting-started.md#get-a-certificate), so a deploy
hook installs each new certificate for the server and restarts it.

CI runs this page after [getting started](@/docs/getting-started.md), once
by HTTP-01 and once by DNS-01, in a Debian 13 container under systemd. There
[Pebble](https://github.com/letsencrypt/pebble), Let's Encrypt's test
server, answers in Let's Encrypt's place, so the certificate CI gets names
Pebble as its issuer.

## Before you start

- **A server set up as in [getting started](@/docs/getting-started.md)**,
  started with systemd. Without systemd, change the hook's last line to the
  command that restarts the server under your supervisor.
- **A public DNS name for the server**, whose `A` record (and `AAAA` record,
  if it has one) gives this host's public address. This page uses
  `vcon.example.com`. Write your own name wherever it appears. A name that
  only `/etc/hosts` or a private DNS knows cannot get one.
- **One way for Let's Encrypt to check that you control the name:**
  - **HTTP-01:** TCP port 80 on this host reachable from the internet while
    certbot runs, with nothing else listening on it. The server keeps its
    own port, such as 8443; certbot listens on port 80 only for the check.
  - **DNS-01:** a DNS server for the name's zone that takes updates (RFC
    2136) with a TSIG key, or a DNS provider that certbot has a plugin for.
    Nothing has to reach this host. Use it for a server the internet cannot
    reach.
- **An email address**, for Let's Encrypt's notices about your account.
- One shell for the steps, as root or with `sudo`, as in getting started.

## Install certbot

<!-- docs-test: only apt -->
```sh
sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq certbot >/dev/null
certbot --version
```

```text
certbot <version>
```

Debian's and Ubuntu's package also installs and enables `certbot.timer`,
which runs `certbot renew` twice a day. On Fedora, install it with
`sudo dnf install -y certbot` and enable its timer with
`sudo systemctl enable --now certbot-renew.timer`.

## Write the deploy hook

certbot runs this script each time it gets a certificate, first or renewed,
with `RENEWED_LINEAGE` naming the directory that holds it. The script copies
the certificate chain and key to where the server reads them, readable by
`vcon` only, and restarts the server. Install it as
`/usr/local/sbin/vcon-server-deploy-cert`, executable by root only:

```sh
sudo tee /usr/local/sbin/vcon-server-deploy-cert >/dev/null <<'EOF'
#!/bin/sh
# Installs a certificate certbot got for vcon-server, then restarts it.
set -eu
install -m 0400 -o vcon -g vcon "$RENEWED_LINEAGE/fullchain.pem" /etc/vcon-server/cert.pem
install -m 0400 -o vcon -g vcon "$RENEWED_LINEAGE/privkey.pem" /etc/vcon-server/key.pem
systemctl restart vcon-server
EOF
sudo chmod 0700 /usr/local/sbin/vcon-server-deploy-cert
```

An [operator listener](@/docs/operator-listener.md) presents the same
certificate, so the restart renews it there too, unless `[operator.tls]`
names a certificate of its own.

## Get the certificate

Let's Encrypt limits how many certificates it issues for one name each week.
Each command below runs first with `--dry-run`, which asks Let's Encrypt's
staging server and saves nothing, so a mistake does not use up that limit.
Use one of the two ways.

`--key-type ecdsa` asks for a P-256 key, which the server reads. An RSA key
works too.

### By HTTP-01

certbot listens on port 80 itself (`--standalone`), and Let's Encrypt asks
it for a file there:

<!-- docs-test: only http-01 -->
```sh
sudo certbot certonly --dry-run --standalone --non-interactive --agree-tos --email admin@example.com \
  --key-type ecdsa -d vcon.example.com
```

```text
…
The dry run was successful.
```

Then get it, with the hook:

<!-- docs-test: only http-01 -->
```sh
sudo certbot certonly --standalone --non-interactive --agree-tos --email admin@example.com \
  --key-type ecdsa -d vcon.example.com --deploy-hook /usr/local/sbin/vcon-server-deploy-cert
```

```text
…
Successfully received certificate.
Certificate is saved at: /etc/letsencrypt/live/vcon.example.com/fullchain.pem
Key is saved at:         /etc/letsencrypt/live/vcon.example.com/privkey.pem
This certificate expires on <date>.
These files will be updated when the certificate renews.
…
```

If port 80 already serves a web site on this host, use its document root
instead of `--standalone`: `--webroot -w /var/www/html`.

### By DNS-01

certbot adds a `TXT` record, `_acme-challenge.vcon.example.com`, through
your DNS server, and Let's Encrypt looks it up. These steps use the RFC 2136
plugin, which works with BIND, Knot, PowerDNS and any other server that
takes signed updates. certbot has a plugin for many DNS providers as well,
such as Cloudflare and Route 53 (Debian packages them as
`python3-certbot-dns-<provider>`); with one, only the credentials file and
the `--dns-...` options change. See certbot's
[DNS plugins](https://eff-certbot.readthedocs.io/en/stable/using.html#dns-plugins).

Install the plugin:

<!-- docs-test: only dns-01 -->
```sh
sudo env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3-certbot-dns-rfc2136 >/dev/null
```

On your DNS server, make a TSIG key that may change only that `TXT` record.
With BIND, `tsig-keygen -a hmac-sha512 certbot.` prints the key, and this
in the zone's statement allows it:

```conf
update-policy { grant certbot. name _acme-challenge.vcon.example.com. TXT; };
```

Save your DNS server's address, the key's name and its secret as
`rfc2136.ini`. The address and the secret below are examples (CI's DNS
server runs on the same host); write yours:

<!-- docs-test: only dns-01; file rfc2136.ini -->
```ini
dns_rfc2136_server = 127.0.0.1
dns_rfc2136_name = certbot.
dns_rfc2136_secret = ZXhhbXBsZS10c2lnLXNlY3JldC1yZXBsYWNlLXdpdGgtdGhlLW9uZS15b3VyLWRucy1zZXJ2ZXItbWFkZSEh
dns_rfc2136_algorithm = HMAC-SHA512
```

Install it readable by root only, and remove your copy:

<!-- docs-test: only dns-01 -->
```sh
sudo install -m 0600 rfc2136.ini /etc/letsencrypt/rfc2136.ini
rm rfc2136.ini
```

Try it, then get it, with the hook. certbot waits 60 seconds after each
update for the record to reach every DNS server of the zone:

<!-- docs-test: only dns-01 -->
```sh
sudo certbot certonly --dry-run --dns-rfc2136 --dns-rfc2136-credentials /etc/letsencrypt/rfc2136.ini \
  --non-interactive --agree-tos --email admin@example.com --key-type ecdsa -d vcon.example.com
```

```text
…
The dry run was successful.
```

<!-- docs-test: only dns-01 -->
```sh
sudo certbot certonly --dns-rfc2136 --dns-rfc2136-credentials /etc/letsencrypt/rfc2136.ini \
  --non-interactive --agree-tos --email admin@example.com --key-type ecdsa -d vcon.example.com \
  --deploy-hook /usr/local/sbin/vcon-server-deploy-cert
```

```text
…
Successfully received certificate.
Certificate is saved at: /etc/letsencrypt/live/vcon.example.com/fullchain.pem
Key is saved at:         /etc/letsencrypt/live/vcon.example.com/privkey.pem
This certificate expires on <date>.
These files will be updated when the certificate renews.
…
```

## Check that it works

The hook has already installed the certificate and restarted the server.
Wait until it answers, with no CA file:

```sh
timeout 30 sh -c 'until curl -s -o /dev/null https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
curl -sS https://vcon.example.com:8443/health -H "Authorization: Bearer $MONITOR"
```

```text
answering
{"status":"ok"}
```

The certificate the server presents checks out against the system's roots,
and names Let's Encrypt as its issuer, such as
`issuer=C=US, O=Let's Encrypt, CN=E7`:

```sh
echo | openssl s_client -connect vcon.example.com:8443 -servername vcon.example.com -verify_return_error 2>/dev/null \
  | grep -E '^(issuer=|Verify return code)'
```

```text
issuer=<issuer>
Verify return code: 0 (ok)
```

Clients can now leave out the CA file: `--cacert` for `curl`, and `ca` in
the client files of `vcon-server tui` and `vcon-server mcp`. The copy for
clients that getting started made, `/usr/local/share/vcon-server/ca.pem`, is
no longer needed.

## Check that it renews

The timer runs `certbot renew`, which renews each certificate well before it
ends, and runs the hook for each one it renews. certbot keeps the hook in
the certificate's renewal file:

<!-- docs-test: only systemd -->
```sh
systemctl is-enabled certbot.timer
sudo grep renew_hook /etc/letsencrypt/renewal/vcon.example.com.conf
```

```text
enabled
renew_hook = /usr/local/sbin/vcon-server-deploy-cert
```

Try a renewal against the staging server, with the hook. In a dry run the
hook installs the current certificate again and restarts the server:

<!-- docs-test: only systemd -->
```sh
sudo certbot renew --dry-run --run-deploy-hooks
```

```text
…
Congratulations, all simulated renewals succeeded:
  /etc/letsencrypt/live/vcon.example.com/fullchain.pem (success)
…
```

The server answers again after the restart:

```sh
timeout 30 sh -c 'until curl -s -o /dev/null https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
```

```text
answering
```

A restart drops the connections open at that moment. Clients that retry, as
a producer should, carry on. To renew at a quieter time, change the timer's
schedule with `systemctl edit certbot.timer`.

## When it does not work

| Message | Cause | Do |
|---|---|---|
| `Timeout during connect (likely firewall problem)`, by HTTP-01 | Let's Encrypt cannot reach port 80 on this host | Open TCP port 80 from the internet, in the host's firewall and any router or cloud security group, and forward it to this host |
| `Could not bind TCP port 80 because it is already in use` | A web server listens on port 80 | Use `--webroot -w <its document root>`, or stop it while certbot runs |
| `DNS problem: NXDOMAIN looking up A for vcon.example.com` | The name has no public `A` record | Add it at your DNS provider, wait for it to reach everyone, then try again |
| `Incorrect TXT record` or `No TXT record found`, by DNS-01 | The record had not reached every DNS server of the zone | Raise `--dns-rfc2136-propagation-seconds` above 60 |
| `The peer didn't know the key we used` | The DNS server has no TSIG key of that name | Check `dns_rfc2136_name` in `rfc2136.ini` against the key's name on the DNS server |
| `The peer didn't like the signature we sent` | The secret or the algorithm is not the DNS server's | Check `dns_rfc2136_secret` and `dns_rfc2136_algorithm` in `rfc2136.ini` |
| `Received response from server: REFUSED` | The key may not change that record | Check the zone's `update-policy`: it names the key and `_acme-challenge.vcon.example.com.` |
| `Unable to determine base domain for _acme-challenge.vcon.example.com` | The DNS server in `rfc2136.ini` does not serve the name's zone | Set `dns_rfc2136_server` to the zone's primary server |
| `too many certificates`, and a time to retry after | The weekly limit for this name is used up | Wait as the message says; try changes with `--dry-run` first |
| The server does not start after the hook | The journal says why: see [getting started](@/docs/getting-started.md#when-it-does-not-start) | `journalctl -u vcon-server`; check that `/etc/vcon-server/cert.pem` and `key.pem` belong to `vcon` |
| A client says `no certificate this client trusts signed it` | The client still names the old CA file | Remove `--cacert`, or `ca` from its client file |
