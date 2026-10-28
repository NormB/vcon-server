+++
title = "Run it in a container"
description = "Build a container image from a checked release, keep the configuration and the data in volumes, allow the server to lock its memory, make the first keys, run it, back it up, and stop it, with Docker or Docker Compose."
weight = 17
+++

The server runs in a container as it runs on a host, with one difference: a
container's memory-lock limit is a ceiling that nothing inside it can raise.
The server locks all of its memory at start and refuses to start if it
cannot, so every container that runs `vcon-server` needs
`--ulimit memlock=-1`.

This how-to builds an image from a [release](@/docs/install.md#download-a-release),
checked against its checksum while it is built. It keeps the configuration,
the data and the backups in three named volumes, and runs the server with
Docker, then with Docker Compose.

CI runs every command on this page with Docker on each change, and checks
each answer.

## Before you start

- Docker, and a user that may run it (a member of the `docker` group, or
  root). `docker version` answers.
- `openssl`, `curl` and `jq` on the host, for the certificate and the checks.
  On Debian or Ubuntu: `sudo apt-get install -y openssl curl jq`.
- One shell for every step: later steps use variables that earlier steps set.

Work in a new directory:

```sh
mkdir vcon-container && cd vcon-container
```

## Build the image

Save this as `Dockerfile`. The first stage downloads the release for the
build host's CPU and checks it against `SHA256SUMS.txt`. The build stops if
the checksum does not match. The second stage holds only Debian's base, the
binary and the `vcon` account:

<!-- docs-test: file Dockerfile -->
```dockerfile
FROM debian:trixie-slim AS download
ARG VERSION
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends curl ca-certificates >/dev/null
WORKDIR /download
RUN archive="vcon-server-${VERSION}-$(uname -m)-unknown-linux-gnu.tar.gz" \
 && curl -fsSLO "https://github.com/NormB/vcon-server/releases/download/v${VERSION}/${archive}" \
 && curl -fsSLO "https://github.com/NormB/vcon-server/releases/download/v${VERSION}/SHA256SUMS.txt" \
 && sha256sum -c SHA256SUMS.txt --ignore-missing \
 && tar -xzf "${archive}" \
 && mv "${archive%.tar.gz}/vcon-server" /download/vcon-server

FROM debian:trixie-slim
COPY --from=download /download/vcon-server /usr/local/bin/vcon-server
RUN useradd --system --user-group --uid 10001 --no-create-home --home-dir /var/lib/vcon-server \
      --shell /usr/sbin/nologin vcon \
 && install -d -m 0700 -o vcon -g vcon /var/lib/vcon-server /etc/vcon-server /backup
USER vcon
EXPOSE 8443
ENTRYPOINT ["vcon-server"]
CMD ["serve", "--config", "/etc/vcon-server/vcon-server.toml"]
```

Build it, naming the version:

```sh
VERSION=0.2.0
docker build -q --build-arg VERSION=$VERSION -t vcon-server:$VERSION . >/dev/null
docker run --rm vcon-server:$VERSION --version
```

```text
vcon-server <version>
```

`docker build` without `-q` shows each step, and `sha256sum` prints the
archive's name and `OK`.

## Make the volumes

Three named volumes hold what must outlive a container: the configuration
with the master key, the data directory, and the backups. A named volume
takes the owner and the mode of the image's directory the first time a
container mounts it, so each starts out owned by `vcon` with mode `0700`:

```sh
docker volume create vcon-etc >/dev/null
docker volume create vcon-data >/dev/null
docker volume create vcon-backup >/dev/null
docker volume inspect vcon-etc vcon-data vcon-backup | jq -r '.[].Name'
```

```text
vcon-etc
vcon-data
vcon-backup
```

Keep `vcon-etc` and `vcon-backup` apart when you copy them off the host: one
holds the master key, the other holds data that the master key opens.

## Fill the configuration volume

`vcs` runs a `vcon-server` command in a new container with both volumes and
the memory-lock limit, as `vcon`. The steps below use it:

```sh
vcs() {
  docker run --rm -i --ulimit memlock=-1 \
    -v vcon-etc:/etc/vcon-server -v vcon-data:/var/lib/vcon-server -v vcon-backup:/backup \
    "$@"
}
```

Make the master key inside the volume, so it never sits on the host:

```sh
vcs --entrypoint sh vcon-server:$VERSION -c \
  'head -c 32 /dev/urandom > /etc/vcon-server/master.key && chmod 0400 /etc/vcon-server/master.key'
```

Make a certificate for the name clients use, here a self-signed one for
`vcon.example.com` (see [getting started](@/docs/getting-started.md#get-a-certificate)
for its extensions), and copy it in with the private key:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 30 \
  -subj /CN=vcon.example.com -addext subjectAltName=DNS:vcon.example.com \
  -addext basicConstraints=critical,CA:FALSE -addext keyUsage=critical,digitalSignature -addext extendedKeyUsage=serverAuth \
  -keyout privkey.pem -out fullchain.pem 2>/dev/null
vcs --entrypoint sh vcon-server:$VERSION -c 'cat > /etc/vcon-server/cert.pem' < fullchain.pem
vcs --entrypoint sh vcon-server:$VERSION -c 'umask 077; cat > /etc/vcon-server/key.pem' < privkey.pem
rm privkey.pem
```

Write the configuration, the same as on a host, and copy it in:

<!-- docs-test: file vcon-server.toml -->
```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"
```

```sh
vcs --entrypoint sh vcon-server:$VERSION -c 'cat > /etc/vcon-server/vcon-server.toml' < vcon-server.toml
vcs --entrypoint ls vcon-server:$VERSION -l /etc/vcon-server | awk 'NR > 1 { print $1, $3, $NF }'
```

```text
-rw-r--r-- vcon cert.pem
-rw------- vcon key.pem
-r-------- vcon master.key
-rw-r--r-- vcon vcon-server.toml
```

## Make the first tenant and its keys

`tenant create` runs only while no server does, so make the first tenant
before you start one:

```sh
TENANT=$(vcs vcon-server:$VERSION tenant create acme --config /etc/vcon-server/vcon-server.toml | jq -r .id)
OPS=$(vcs vcon-server:$VERSION key create --operator --label ops --config /etc/vcon-server/vcon-server.toml | jq -r .token)
MONITOR=$(vcs vcon-server:$VERSION key create --tenant "$TENANT" --scope read --label monitor \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
vcs vcon-server:$VERSION key list --config /etc/vcon-server/vcon-server.toml | jq -c '{label, scopes}'
```

```text
{"label":"ops","scopes":["admin"]}
{"label":"monitor","scopes":["read"]}
```

## Without the memory-lock limit

A container started without `--ulimit memlock=-1` cannot lock the memory,
and every command, `serve` included, refuses:

<!-- docs-test: exit 1 -->
```sh
docker run --rm -v vcon-etc:/etc/vcon-server -v vcon-data:/var/lib/vcon-server \
  vcon-server:$VERSION check --config /etc/vcon-server/vcon-server.toml
```

```text
vcon-server: memory cannot be locked: Cannot allocate memory (os error 12); allow it, for example LimitMEMLOCK=infinity
```

## Run the server

Start it in the background, publishing port 8443. `--restart unless-stopped`
starts it again after a crash or a reboot of the host:

```sh
docker run -d --name vcon-server --restart unless-stopped --ulimit memlock=-1 -p 8443:8443 \
  -v vcon-etc:/etc/vcon-server -v vcon-data:/var/lib/vcon-server -v vcon-backup:/backup \
  vcon-server:$VERSION >/dev/null
timeout 30 sh -c 'until docker logs vcon-server 2>&1 | grep -q serving; do sleep 1; done'
docker logs vcon-server 2>&1 | grep vcon_server
```

```text
<time>  INFO vcon_server::serve: serving listen=0.0.0.0:8443
<time>  INFO vcon_server::serve: purge sweep purged=0 failed=0
```

The server logs to standard error, which `docker logs` shows.

## Check that it answers

The certificate names `vcon.example.com`. `--resolve` points that name at
this host for one request, without a DNS entry:

```sh
curl -sS --cacert fullchain.pem --resolve vcon.example.com:8443:127.0.0.1 \
  https://vcon.example.com:8443/health -H "Authorization: Bearer $MONITOR"
```

```text
{"status":"ok"}
```

## Run commands beside it

The commands that run beside the server run in its container with
`docker exec`. They take the container's memory-lock limit and its user:

```sh
WRITER=$(docker exec vcon-server vcon-server key create --tenant "$TENANT" --scope write --label recorder-1 \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
docker exec vcon-server vcon-server key list --tenant "$TENANT" --config /etc/vcon-server/vcon-server.toml | jq -r .label
```

```text
monitor
recorder-1
```

[Store your first vCon](@/docs/first-vcon.md) works the same against this
server, with `--resolve vcon.example.com:8443:127.0.0.1` added to each
`curl`.

## Back it up

`backup` writes into the backup volume while the server runs:

```sh
BACKUP=/backup/data-$(date -u +%Y%m%dT%H%M%SZ)
docker exec vcon-server vcon-server backup "$BACKUP" --config /etc/vcon-server/vcon-server.toml
```

```text
{"tenants":1,"tries":1,"versions":0}
```

Copy the backup volume off the host, and the master key apart from it.
`docker run --rm -v vcon-backup:/backup debian:trixie-slim tar -C /backup -c .`
writes the volume to standard output as a tar archive. See
[back up and restore](@/docs/backup-and-restore.md) for the restore.

## Stop it and start it

`docker stop` sends SIGTERM. The server logs `stopped` and exits:

```sh
docker stop vcon-server >/dev/null
docker logs vcon-server 2>&1 | grep -c 'vcon_server::serve: stopped'
```

```text
1
```

The data stays in its volume. Start the same container again:

```sh
docker start vcon-server >/dev/null
timeout 30 sh -c 'until curl -s -o /dev/null --cacert fullchain.pem --resolve vcon.example.com:8443:127.0.0.1 https://vcon.example.com:8443/health; do sleep 1; done'
curl -sS --cacert fullchain.pem --resolve vcon.example.com:8443:127.0.0.1 \
  https://vcon.example.com:8443/health -H "Authorization: Bearer $MONITOR"
```

```text
{"status":"ok"}
```

## Or with Docker Compose

The same container as a Compose service. The volumes are the ones made
above, so Compose uses them as they are (`external: true`). Save this as
`compose.yaml`:

<!-- docs-test: file compose.yaml -->
```yaml
services:
  vcon-server:
    image: vcon-server:${VERSION}
    restart: unless-stopped
    ulimits:
      memlock: -1
    ports:
      - "8443:8443"
    volumes:
      - vcon-etc:/etc/vcon-server
      - vcon-data:/var/lib/vcon-server
      - vcon-backup:/backup
volumes:
  vcon-etc:
    external: true
  vcon-data:
    external: true
  vcon-backup:
    external: true
```

Remove the container started by hand, then start the service. Compose reads
`VERSION` from the environment, which `env` sets for it:

```sh
docker rm -f vcon-server >/dev/null
env VERSION=$VERSION docker compose up -d --quiet-pull 2>/dev/null
timeout 30 sh -c 'until curl -s -o /dev/null --cacert fullchain.pem --resolve vcon.example.com:8443:127.0.0.1 https://vcon.example.com:8443/health; do sleep 1; done'
curl -sS --cacert fullchain.pem --resolve vcon.example.com:8443:127.0.0.1 \
  https://vcon.example.com:8443/health -H "Authorization: Bearer $MONITOR"
```

```text
{"status":"ok"}
```

Stop and remove the service. The volumes stay:

```sh
env VERSION=$VERSION docker compose down 2>/dev/null
docker volume inspect vcon-data | jq -r '.[].Name'
```

```text
vcon-data
```

## Remove everything

This deletes the data and the master key with their volumes. Do it only for
a server you are trying out:

<!-- docs-test: always -->
```sh
docker rm -f vcon-server >/dev/null 2>&1
docker volume rm vcon-etc vcon-data vcon-backup >/dev/null
docker image rm vcon-server:$VERSION >/dev/null
cd .. && rm -r vcon-container
```
