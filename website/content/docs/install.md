+++
title = "Install"
description = "Put the vcon-server binary on a Linux host: download a release and check it, install it with cargo, build it from source, or run it in a container. Each path lists what to install first on Debian, Ubuntu and Fedora, what each step prints, and what to do when a step fails."
weight = 8
+++

vcon-server is one binary, `vcon-server`. This page puts it at
`/usr/local/bin/vcon-server`. [Getting started](@/docs/getting-started.md)
then makes its account, its keys and its configuration, and starts it.

CI runs the commands on this page on each change. Each install path runs in a
fresh Debian 13 container and a fresh Fedora 44 container. The release path
also runs in a fresh Ubuntu 24.04 container. Each command must print what this
page shows.

## Choose a path

| You want | Path |
|---|---|
| The binary the project built and signed, checked before you run it | [Download a release](#download-a-release) |
| A build on a host the release binary does not run on, such as Debian 12 | [Install with cargo](#install-with-cargo) |
| A build of the newest code, or a build you have changed | [Build from source](#build-from-source) |
| The server in Docker or another container runtime | [Run it in a container](@/docs/container.md) |
| A newer version over one you already run | [Upgrade](@/docs/upgrading.md) |

## What the host needs

- **Linux on x86_64 or aarch64.** No other system is built or tested.
- **For the release binary, glibc 2.38 or later.** The binary links only the
  system's `libc`, `libm` and `libgcc_s`; SQLCipher, OpenSSL and the
  cryptography are compiled in. Debian 13 (glibc 2.41), Ubuntu 24.04 (2.39)
  and Fedora 44 (2.43) qualify. Debian 12 (2.36) does not. Check yours:

  ```sh
  ldd --version | sed -n 1p
  ```

  ```text
  ldd (<distribution>) <glibc version>
  ```

  The last number is glibc's version.

  The 0.2.0 binaries need glibc 2.38. On an older glibc,
  [install with cargo](#install-with-cargo) or
  [build from source](#build-from-source) instead. Both build against the
  host's own glibc.
- **The right to lock memory.** At start the server locks all of its memory,
  and it refuses to start if it cannot. [Getting started](@/docs/getting-started.md#allow-it-to-lock-its-memory)
  shows how to allow it.
- **`sudo`.** The commands below run as an ordinary user with `sudo`. The
  steps for a system with no `sudo` yet run as root.

## Download a release

Each release on [GitHub](https://github.com/NormB/vcon-server/releases) holds
these files:

- `vcon-server-<version>-x86_64-unknown-linux-gnu.tar.gz` and
  `vcon-server-<version>-aarch64-unknown-linux-gnu.tar.gz`: the server, its
  `README.md` and the two licence files;
- `vcon-server-<version>.cdx.json`: a CycloneDX SBOM listing every crate
  built into the server;
- `SHA256SUMS.txt`: the SHA-256 of each file.

Every file also carries a signed build provenance attestation. The attestation
ties the file to the workflow run and the commit that built it. The binary has
SQLCipher, OpenSSL and rusqlite from crates.io compiled in. Nothing in it comes
from the rusqlite fork the tests use (see
[building without network access](#building-without-network-access)).

### What to install first

The release path needs `sudo`, `curl`, `tar`, `gzip` and the CA certificates.
A minimal Debian or Ubuntu has none of `sudo`, `curl` or the certificates. As
root:

<!-- docs-test: only apt release -->
```sh
apt-get update -qq
apt-get install -y -qq sudo curl ca-certificates tar gzip >/dev/null
```

On Fedora, as root:

<!-- docs-test: only dnf release -->
```sh
dnf install -y -q sudo curl tar gzip
```

### Download and check it

Set the version, then download the archive for this host's CPU and the
checksums. `uname -m` prints `x86_64` or `aarch64`, the CPU names in the
archive names:

<!-- docs-test: only release -->
```sh
VERSION=0.2.0
ARCHIVE=vcon-server-$VERSION-$(uname -m)-unknown-linux-gnu.tar.gz
curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$VERSION/$ARCHIVE
curl -fsSLO https://github.com/NormB/vcon-server/releases/download/v$VERSION/SHA256SUMS.txt
sha256sum -c SHA256SUMS.txt --ignore-missing
```

`sha256sum` checks each file named in `SHA256SUMS.txt` that is present, here
only the archive:

```text
vcon-server-<version>-<cpu>-unknown-linux-gnu.tar.gz: OK
```

Unpack it. The archive holds one directory:

<!-- docs-test: only release -->
```sh
tar -xzf "$ARCHIVE"
ls "${ARCHIVE%.tar.gz}"
```

```text
LICENSE-APACHE
LICENSE-MIT
README.md
vcon-server
```

Install the binary and check that it runs:

<!-- docs-test: only release -->
```sh
sudo install -m 0755 "${ARCHIVE%.tar.gz}/vcon-server" /usr/local/bin/vcon-server
vcon-server --version
```

```text
vcon-server <version>
```

### Check where it was built

The checksum shows that the archive is the one `SHA256SUMS.txt` names. The
attestation shows that the release workflow of this repository built it, from
which commit. Checking it needs the [GitHub CLI](https://cli.github.com/),
signed in with `gh auth login`, or a token in `GH_TOKEN`. Where you cannot
install `gh`, the checksum check above is the only check.

Debian 13 packages `gh` 2.46 and Ubuntu 24.04 packages `gh` 2.45. Neither
has the `attestation` command. Install `gh` from GitHub's own package
repository instead:

<!-- docs-test: only apt release -->
```sh
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
  | sudo tee /etc/apt/keyrings/githubcli-archive-keyring.gpg >/dev/null
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
  | sudo tee /etc/apt/sources.list.d/github-cli.list >/dev/null
sudo apt-get update -qq
sudo apt-get install -y -qq gh >/dev/null
```

Fedora's own `gh` has it:

<!-- docs-test: only dnf release -->
```sh
sudo dnf install -y -q gh
```

Check that this `gh` has the command:

<!-- docs-test: only release -->
```sh
gh attestation --help | sed -n 1p
```

```text
Download and verify artifact attestations.
```

Check the archive:

<!-- docs-test: only release -->
```sh
gh attestation verify "$ARCHIVE" --repo NormB/vcon-server >/dev/null && echo verified
```

```text
verified
```

`gh` exits 0 for a file that this repository's release workflow built and
that has not changed since. Run without `>/dev/null`, it also prints the
workflow and the commit. A changed file has no attestation, so `gh` exits 1
and says so:

<!-- docs-test: only release -->
```sh
cp "$ARCHIVE" changed.tar.gz && printf x >> changed.tar.gz
```

<!-- docs-test: only release; exit 1 -->
```sh
gh attestation verify changed.tar.gz --repo NormB/vcon-server
```

```text
Error: HTTP 404: Not Found (<url of the changed file's attestations>)
```

<!-- docs-test: only release -->
```sh
rm changed.tar.gz
```

### When a step fails

- **`curl: (22) The requested URL returned error: 404`.** No release has that
  version, or the archive name is wrong. The releases page lists every
  version.
- **`sha256sum: WARNING: 1 computed checksum did NOT match`.** The line before
  it names the file, followed by `FAILED`, and `sha256sum` exits 1. The
  download is damaged or is not the released file. Delete it and download
  it again. If it fails again, do not run it, and report it as a
  [security problem](@/docs/security.md#reporting-a-vulnerability).
- **`sha256sum: SHA256SUMS.txt: no file was verified`, with no `OK` or
  `FAILED` line before it.** The archive is not in the current directory, or
  its name differs from the one in `SHA256SUMS.txt`.
- **`version 'GLIBC_2.38' not found`.** The host's glibc is older than 2.38.
  This is the Debian 12 output:

  ```text
  vcon-server: /lib/aarch64-linux-gnu/libc.so.6: version `GLIBC_2.38' not found (required by vcon-server)
  ```

  [Install with cargo](#install-with-cargo) instead.
- **`To get started with GitHub CLI, please run:  gh auth login`**, with exit
  status 4. `gh` is not signed in. Run `gh auth login`, or set `GH_TOKEN`.
- **`unknown command "attestation" for "gh"`.** This `gh` is too old. Install
  it from GitHub's package repository, as shown above.

## Install with cargo

`cargo install` builds the server from its package on
[crates.io](https://crates.io/crates/vcon-server). That package holds only the
server's sources and depends only on crates from crates.io. The build compiles
SQLCipher, the OpenSSL it uses, and AWS-LC. It needs Rust 1.99 or later, a C
compiler with the C library headers, `make` and `perl`. The release build
takes several minutes.

### What to install first

On Debian or Ubuntu, as root:

<!-- docs-test: only apt cargo -->
```sh
apt-get update -qq
apt-get install -y -qq sudo curl ca-certificates gcc libc6-dev make perl >/dev/null
```

On Fedora, as root:

<!-- docs-test: only dnf cargo -->
```sh
dnf install -y -q sudo curl gcc make perl
```

The distributions' own Rust packages are older than 1.99: Debian 13 has 1.85.
Install Rust with [rustup](https://rustup.rs/), as the user who builds:

<!-- docs-test: only cargo -->
```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal
. "$HOME/.cargo/env"
cargo --version
```

`rustup` reports what it installs, then `cargo` prints its version:

```text
…
cargo <version>
```

### Build and install it

<!-- docs-test: only cargo -->
```sh
cargo install vcon-server --locked
```

`--locked` builds with the exact dependency versions the release was tested
with. Cargo prints each crate as it compiles it, and near the end:

```text
…
   Installed package `vcon-server v<version>` (executable `vcon-server`)
…
```

The binary is in `~/.cargo/bin`. Install it where the other pages expect it:

<!-- docs-test: only cargo -->
```sh
sudo install -m 0755 "$HOME/.cargo/bin/vcon-server" /usr/local/bin/vcon-server
vcon-server --version
```

```text
vcon-server <version>
```

### When a step fails

- **``error: linker `cc` not found``.** There is no C compiler. Install the
  packages shown above.
- **`it requires rustc 1.99 or newer, while the currently active rustc
  version is`**, after `error: cannot install package`. The Rust in `PATH`
  is older. Debian 13's own `cargo`
  gives this error, naming its rustc 1.85.1. Install Rust with rustup as
  shown above, and check that `cargo --version` names rustup's cargo.

## Build from source

A build from the repository builds the newest code on `main`. It needs what
[cargo install](#install-with-cargo) needs, and `git`.

### What to install first

On Debian or Ubuntu, as root:

<!-- docs-test: only apt source -->
```sh
apt-get update -qq
apt-get install -y -qq sudo curl ca-certificates git gcc libc6-dev make perl >/dev/null
```

On Fedora, as root:

<!-- docs-test: only dnf source -->
```sh
dnf install -y -q sudo curl git gcc make perl
```

Then install rustup. The repository's `rust-toolchain.toml` names the exact
Rust version. rustup installs that version at the first build:

<!-- docs-test: only source -->
```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal
. "$HOME/.cargo/env"
```

### Build and install it

<!-- docs-test: only source -->
```sh
git clone -q https://github.com/NormB/vcon-server.git
cd vcon-server
cargo build --release --locked -p vcon-server
./target/release/vcon-server --version
```

```text
…
vcon-server <version>
```

`-p vcon-server` builds the server alone. Without it, `cargo build` builds
every package in the repository. Install the binary:

<!-- docs-test: only source -->
```sh
sudo install -m 0755 target/release/vcon-server /usr/local/bin/vcon-server
vcon-server --version
```

```text
vcon-server <version>
```

### Building without network access

The server is built with rusqlite from crates.io. The tests also use a fork
of rusqlite from `github.com/NormB/rusqlite` (see
[development](@/docs/development.md#test-dependency-from-a-fork)). Cargo
compiles the fork only for the tests. Every build of this checkout fetches
it, `cargo build --release` included, because `Cargo.lock` lists it. A build
needs access to github.com as well as crates.io. To build without network
access, copy every source into the checkout once, while online:

<!-- docs-test: skip the runner does not build offline -->
```sh
mkdir -p .cargo && cargo vendor >> .cargo/config.toml
cargo build --release --offline
```

`cargo vendor` copies the fork with the other sources, and the tests then
build offline from the copy as well. A build that reads crates.io through a
mirror still needs github.com for the fork, or the vendored copy.

### When a step fails

The failures of [cargo install](#install-with-cargo) apply here too. Also:

- **`git: command not found`.** Install `git`, as shown above.
- **A build with no access to github.com.** See
  [building without network access](#building-without-network-access).

## Next

[Getting started](@/docs/getting-started.md) makes the service account, the
master key, the certificate, the configuration and the first keys. It then
starts the server.
