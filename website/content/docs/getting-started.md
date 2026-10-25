+++
title = "Getting started"
description = "Build vcon-server, make its master key and certificate, write a configuration, and start it."
weight = 10
+++

## Build

vcon-server is written in Rust. The toolchain is pinned in
`rust-toolchain.toml`, so `rustup` installs the right one on first build. The
build also compiles SQLCipher and the OpenSSL it uses, and AWS-LC for the
cryptography, so the host needs a C compiler, `make` and `perl`.

```sh
git clone https://github.com/NormB/vcon-server.git
cd vcon-server
cargo build --release
./target/release/vcon-server --version
```

The binary is `target/release/vcon-server`. Run the tests with
`cargo test --workspace`.

## Make the master key

The master key is a file of exactly 32 random bytes. It wraps every tenant key
and opens the server's own catalog, so whoever holds it can read everything.
Keep it on the server, readable only by the service account, and back it up
separately from the data (see [operations](@/docs/operations.md#backups)).

```sh
sudo install -d -m 0700 -o vcon -g vcon /etc/vcon-server
head -c 32 /dev/urandom | sudo -u vcon tee /etc/vcon-server/master.key >/dev/null
sudo chmod 0400 /etc/vcon-server/master.key
```

A file of any other length is refused at start. Starting with a different key
than the one the data directory was made under is refused too.

## Get a certificate

The API is served over TLS only, HTTP/2 and HTTP/1.1. Give the server a PEM
certificate chain and its PEM private key, for example from your ACME client.

## Write the configuration

One TOML file. Only these keys are required; every other key has a default
(see [configuration](@/docs/configuration.md)):

```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"
```

The data directory must exist and belong to the service account. The server
makes everything inside it.

## Allow it to lock its memory

At start the server locks all its memory, so no key or vCon body can reach
swap, and turns core dumps off. It refuses to start if it cannot lock its
memory. Under systemd, allow it with `LimitMEMLOCK=infinity`:

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

Run by hand, raise the limit first with `ulimit -l unlimited` (as root, or
after raising the hard limit).

## Start it

```sh
vcon-server serve --config /etc/vcon-server/vcon-server.toml
```

It logs structured lines to standard error: `serving` once it listens, a line
per request, and `stopped` after SIGTERM or SIGINT. It refuses to start, with
the reason, when the configuration is wrong, the master key is not the data
directory's, memory cannot be locked, core dumps cannot be turned off,
another server already holds the data directory, or a tenant's data is not
all under its current key (a key rotation stopped part way: run it again;
`vcon-server check` names the tenants).

## Tenants and keys

Every request carries a bearer key, and every vCon belongs to a tenant. Make
the first tenant and its keys on the server's host:

```sh
vcon-server tenant create acme --config /etc/vcon-server/vcon-server.toml
vcon-server key create --operator --label ops --config /etc/vcon-server/vcon-server.toml
vcon-server key create --tenant <tenant id> --scope write --label recorder-1 \
  --config /etc/vcon-server/vcon-server.toml
```

Each prints a JSON line; a key's token is shown this once. The operator key
(scope `admin`) manages tenants and keys over the
[admin routes](@/docs/http-api.md#admin) from then on. See the
[command line](@/docs/command-line.md) and [tenants and keys](@/docs/tenants-and-keys.md).
