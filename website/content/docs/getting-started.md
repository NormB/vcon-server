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
`cargo test --workspace`. SQLCipher and OpenSSL are compiled into the binary;
it links only the system's `libc`, `libm` and `libgcc_s`.

The server is built with rusqlite from crates.io. The tests also use a fork of
rusqlite from `github.com/NormB/rusqlite` (see
[development](@/docs/development.md#test-dependency-from-a-fork)). Cargo
compiles the fork only for the tests, but every build of this checkout,
`cargo build --release` included, fetches it, because `Cargo.lock` lists it.
So a build needs access to github.com as well as crates.io. To build without
network access, copy every source into the checkout once while online:

```sh
mkdir -p .cargo && cargo vendor >> .cargo/config.toml
cargo build --release --offline
```

`cargo vendor` copies the fork with the other sources, and the tests build
offline from the copy as well. A build that reads crates.io through a mirror
still needs github.com for the fork, or the vendored copy.

## Or install from crates.io

With Rust 1.99 or later:

```sh
cargo install vcon-server --locked
vcon-server --version
```

`cargo install` puts the binary in `~/.cargo/bin`. It compiles SQLCipher, the
OpenSSL it uses, and AWS-LC, so the host needs a C compiler with the C library
headers, `make` and `perl`. On Debian or Ubuntu:

```sh
sudo apt-get install gcc libc6-dev make perl
```

The package on crates.io holds only the server's sources and depends only on
crates from crates.io, so this build does not fetch anything from github.com.

## Or download a release

Each release on GitHub holds the server for Linux on x86_64 and aarch64, a
CycloneDX SBOM listing every crate built into it, and `SHA256SUMS.txt`. Every
file carries a signed build provenance attestation, and each archive carries
its SBOM as one too, so a download can be traced to the workflow run and the
commit that built it. The binary has SQLCipher, OpenSSL and rusqlite from
crates.io compiled in, and links only the system's `libc`, `libm` and `libgcc_s`;
nothing in it comes from the rusqlite fork the tests use. The releases are on
[GitHub](https://github.com/NormB/vcon-server/releases).

To check a download, with the [GitHub CLI](https://cli.github.com/):

```sh
gh attestation verify vcon-server-VERSION-x86_64-unknown-linux-gnu.tar.gz \
  --repo NormB/vcon-server
sha256sum -c SHA256SUMS.txt --ignore-missing
```

The first command fails on any file that was not built by this repository's
release workflow, or that changed after it was built.

## Install it

```sh
sudo install -m 0755 target/release/vcon-server /usr/local/bin/vcon-server
```

From a release archive, install the `vcon-server` file it holds the same way.

## Make the service account and its directories

The server runs as its own account, `vcon`, which owns the data directory and
the files in `/etc/vcon-server`:

```sh
sudo useradd --system --user-group --no-create-home --home-dir /var/lib/vcon-server --shell /usr/sbin/nologin vcon
sudo install -d -m 0700 -o vcon -g vcon /var/lib/vcon-server
sudo install -d -m 0700 -o vcon -g vcon /etc/vcon-server
```

The server makes everything inside the data directory.

## Allow it to lock its memory

At start the server locks all its memory, so no key or vCon body can reach
swap, and turns core dumps off. It refuses to start if it cannot lock its
memory. Every other command does the same, so the account that runs them
needs the same allowance. The [systemd unit](#start-it) below allows it for
the server. For the commands you run by hand, allow it for the `vcon`
account:

```sh
echo 'vcon - memlock unlimited' | sudo tee /etc/security/limits.d/vcon-server.conf
```

`sudo -u vcon` then applies it, through `pam_limits`. Where the account's
limit is too small and this file is missing, a command stops with:

```text
vcon-server: memory cannot be locked: Cannot allocate memory (os error 12); allow it, for example LimitMEMLOCK=infinity
```

Run every command as `vcon`, with `sudo -u vcon`, never as root. A command
run as root leaves files in the data directory that `vcon` cannot open. The
server then refuses to start, with
`the data directory cannot be locked: Permission denied (os error 13)`, or,
for a tenant made as root,
`1 tenant catalog(s) cannot be opened: Permission denied (os error 13)`;
`vcon-server check` names the path. Give the files back to `vcon`:

```sh
sudo chown -R vcon:vcon /var/lib/vcon-server
```

## Make the master key

The master key is a file of exactly 32 random bytes. It wraps every tenant key
and opens the server's own catalog, so whoever holds it can read everything.
Keep it on the server, readable only by the service account, and back it up
separately from the data (see [back up and restore](@/docs/operations.md#back-up-and-restore)).

```sh
head -c 32 /dev/urandom | sudo -u vcon tee /etc/vcon-server/master.key >/dev/null
sudo chmod 0400 /etc/vcon-server/master.key
```

A file of any other length is refused at start. Starting with a different key
than the one the data directory was made under is refused too.

The key's id is the first 16 hexadecimal digits of its SHA-256, the same
`master_key_id` that `vcon-server rekey master` prints. Label each copy of the
key with it:

```sh
sudo sha256sum /etc/vcon-server/master.key | cut -c1-16
```

## Get a certificate

The API is served over TLS only, HTTP/2 and HTTP/1.1. Give the server a PEM
certificate chain and its PEM private key, for example from your ACME client,
readable by the service account:

```sh
sudo install -m 0400 -o vcon -g vcon fullchain.pem /etc/vcon-server/cert.pem
sudo install -m 0400 -o vcon -g vcon privkey.pem /etc/vcon-server/key.pem
```

The server reads them at start only: after a renewal, install the new files
and restart it.

To try the server before you have a certificate, make a self-signed one for
the name clients will use:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 30 \
  -subj /CN=vcon.example.com -addext subjectAltName=DNS:vcon.example.com \
  -keyout privkey.pem -out fullchain.pem
```

Clients then need `fullchain.pem` to trust it.

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

Save it as `vcon-server.toml`, then install it where the server reads it:

```sh
sudo install -m 0400 -o vcon -g vcon vcon-server.toml /etc/vcon-server/vcon-server.toml
```

## Make the first tenant and its keys

Every request carries a bearer key, and every vCon belongs to a tenant. Make
the first tenant before you start the server: `tenant create` runs only
while no server does.

```sh
sudo -u vcon vcon-server tenant create acme --config /etc/vcon-server/vcon-server.toml
sudo -u vcon vcon-server key create --operator --label ops --config /etc/vcon-server/vcon-server.toml
sudo -u vcon vcon-server key create --tenant <tenant id> --scope write --label recorder-1 \
  --config /etc/vcon-server/vcon-server.toml
sudo -u vcon vcon-server key create --tenant <tenant id> --scope read --label monitor \
  --config /etc/vcon-server/vcon-server.toml
```

Each prints a JSON line: the tenant's `id`, then each key's `id` and `token`.
A token is shown this once. The operator key (scope `admin`) manages tenants
and keys over the [admin routes](@/docs/http-api.md#admin) once the server
runs: further tenants are made with `POST /admin/tenants`. `key create` also
runs beside the server. The `monitor` key is for the
[health check](@/docs/operations.md#health-checks). See the
[command line](@/docs/command-line.md) and [tenants and keys](@/docs/tenants-and-keys.md).

## Start it

Save this unit as `/etc/systemd/system/vcon-server.service`.
`LimitMEMLOCK=infinity` allows the server to lock its memory:

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

Then start it, now and at each boot, and read its first lines:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now vcon-server
sudo journalctl -u vcon-server -n 5
```

It logs structured lines to standard error, which the journal keeps:
`serving` once it listens, a line per request, and `stopped` after SIGTERM or
SIGINT:

```text
2026-10-26T12:53:48.651326Z  INFO vcon_server::serve: serving listen=0.0.0.0:8443
2026-10-26T12:53:48.653954Z  INFO vcon_server::serve: purge sweep purged=0 failed=0
```

It refuses to start, with the reason, when the configuration is wrong, the
master key is not the data directory's, memory cannot be locked, core dumps
cannot be turned off, another server already holds the data directory, or a
tenant's data is not all under its current key (a key rotation stopped part
way: run it again; `vcon-server check` names the tenants).

## Check that it answers

Ask for its health with the `monitor` key's token:

```sh
curl --cacert fullchain.pem https://vcon.example.com:8443/health \
  -H "Authorization: Bearer <monitor token>"
```

It answers `{"status":"ok"}`. Leave out `--cacert fullchain.pem` when the
certificate comes from a public certificate authority. Without a key, the
answer is `401`.
