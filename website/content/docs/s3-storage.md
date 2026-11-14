+++
title = "Keep vCons in an S3 bucket"
description = "Keep each version's sealed object in an S3-compatible bucket (RustFS, MinIO or AWS S3) instead of the data directory: the bucket and its keys on the storage side, the configuration, a check that the objects go there and that a purge removes them, what each refusal means, and how to clean up."
weight = 27

[extra]
group = "how-to"
+++

**S3 storage is in release 0.4.0 and later.** Release 0.3.0 and earlier
keep their objects in the data directory only, and their server refuses a
`[storage]` table. Install 0.4.0 or later from a release, with cargo or from
source, as [install](@/docs/install.md) shows; `vcon-server --version` names
the release a host runs.

The release binaries, `cargo install` and a build from source include S3
support through the `s3` cargo feature, which is on by default;
`--no-default-features` leaves it out.

By default the server keeps every stored version as a sealed object file in
its data directory. This page moves those objects to a bucket in an
S3-compatible service: [RustFS](https://rustfs.com/),
[MinIO](https://github.com/minio/minio) or
[AWS S3](https://aws.amazon.com/s3/). The catalogs, `server.sqlite`, the lock
files and the master key stay on the server's host. Each object in the
bucket is the same sealed bytes the server would write to a file, so the
bucket's operator sees random names, sizes and times, and no vCon.

A server that already stored vCons in its data directory moves their
objects to the bucket with `vcon-server storage move-objects`, run while the
server is stopped (see
[move objects stored in the data directory](#move-objects-stored-in-the-data-directory)).
Started with the storage table and without that step, it answers `500` for
the versions still in the data directory, and `vcon-server check` lists
them as damaged.

## What was tested, and what was not

- **RustFS 1.0.4** and **MinIO RELEASE.2025-09-07T16-13-09Z.** CI runs the
  store's conformance suite, the S3 backend's own tests and `serve` with its
  objects in a bucket against each, on each change. CI also runs this page
  from top to bottom twice, once with RustFS and once with MinIO, after
  [getting started](@/docs/getting-started.md), in a Debian 13 container
  under systemd. The storage service runs in the same container there, at
  the name `objects.example.com`. In a real deployment the storage service
  is another host: run the steps marked "on the storage host" there.
- **AWS S3** is checked against its documented API only. No CI job reaches
  AWS: the [AWS S3 section](#aws-s3) and its configuration and policy are
  checked by a test that reads them from this page, but no request was sent
  to AWS, no IAM role was tried, and the messages AWS gives for a wrong
  region or a missing permission were not captured.

MinIO's community edition is no longer maintained: its
[repository](https://github.com/minio/minio) is archived, and its last
release with Linux binaries is the one above. MinIO's commercial editions
were not tested.

## What the service must support

- **Conditional writes.** The server writes its claim on a prefix (see
  [one server per bucket and prefix](#one-server-per-bucket-and-prefix))
  with `If-None-Match: *`, so that of two servers claiming one prefix at once,
  one is refused with `412`. A service that ignores the header lets both
  claims through. RustFS 1.0.4 and MinIO RELEASE.2025-09-07T16-13-09Z refuse
  the second write with `412`: CI checks it on each change. AWS documents
  the same behavior in
  [conditional writes](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html).
- **Signature Version 4**, `GetBucketVersioning`, `ListObjectsV2`,
  `GetObject`, `PutObject`, `DeleteObject` and `DeleteObjects`.
- **A bucket whose versioning has never been turned on**, as
  [versioning](#the-bucket-has-versioning-on) explains.

## Before you start

- **A server set up as [getting started](@/docs/getting-started.md) shows,**
  that has stored no vCon yet, running 0.4.0 or later (see the top of this
  page). The steps below use its shell variables (`$TENANT`,
  `$RECORDER`, `$OPS`) and its files in `/etc/vcon-server`.
- **An S3-compatible service and a bucket in it** whose versioning has never
  been turned on. The next sections set up RustFS or MinIO; for AWS S3, see
  [AWS S3](#aws-s3).
- **An access key for the bucket** that may get, put, delete and list
  objects and read the bucket's versioning (`s3:GetObject`, `s3:PutObject`,
  `s3:DeleteObject`, `s3:ListBucket`, `s3:GetBucketVersioning`), and nothing
  else.
- **TLS.** The server reaches the service over `https://`. When the
  service's certificate comes from a private certificate authority, the
  server needs that authority's certificate. The steps below make one.
- **`unzip`,** for RustFS only, to unpack its release. On Debian or Ubuntu:

  <!-- docs-test: only apt rustfs -->
  ```sh
  sudo apt-get install -y -qq unzip >/dev/null
  ```

## Give the storage host a certificate (on the storage host)

With a certificate from your certificate authority for the storage host's
name, use it as `objects.pem` and its key as `objects.key`, and put your
authority's certificate at `/usr/local/share/vcon-server/s3-ca.pem` on the
vcon-server host.

Otherwise, make a certificate authority of your own and a certificate it
signs for the storage host's name, `objects.example.com` here. The server
trusts that authority through `storage.s3.ca_file`, and no other client
trusts it:

```sh
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 365 \
  -subj /CN=objects-ca -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign \
  -keyout objects-ca.key -out objects-ca.pem 2>/dev/null
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj /CN=objects.example.com \
  -keyout objects.key -out objects.csr 2>/dev/null
printf '%s\n' subjectAltName=DNS:objects.example.com basicConstraints=critical,CA:FALSE \
  keyUsage=critical,digitalSignature extendedKeyUsage=serverAuth > objects.ext
openssl x509 -req -in objects.csr -CA objects-ca.pem -CAkey objects-ca.key -days 365 \
  -extfile objects.ext -out objects.pem 2>/dev/null
sudo install -d -m 0755 /usr/local/share/vcon-server
sudo install -m 0644 objects-ca.pem /usr/local/share/vcon-server/s3-ca.pem
openssl verify -CAfile /usr/local/share/vcon-server/s3-ca.pem objects.pem
```

```text
objects.pem: OK
```

The authority's certificate is now at
`/usr/local/share/vcon-server/s3-ca.pem`, readable by every account, for
the server, for `curl` and for MinIO's `mc`. Copy it to the vcon-server
host when that is another host. `objects-ca.key` signs certificates the
server trusts for the bucket; [clean up](#clean-up) says where it goes.

Until DNS gives `objects.example.com` to the storage host, add the name to
`/etc/hosts` on the vcon-server host (here both are one host):

```sh
echo '127.0.0.1 objects.example.com' | sudo tee -a /etc/hosts >/dev/null
```

Then follow either [RustFS](#set-up-rustfs-on-the-storage-host) or
[MinIO](#set-up-minio-on-the-storage-host).

## Set up RustFS (on the storage host)

Download a RustFS release for the host's CPU and check it against the
SHA-256 that RustFS publishes for it:

<!-- docs-test: only rustfs -->
```sh
RUSTFS_VERSION=1.0.4
case "$(uname -m)" in
  x86_64) RUSTFS_SHA256=4924ad4b699b9f59943051efedae2d1eb6d4cac503a1b0384c4b25f7c2c31b5c ;;
  aarch64) RUSTFS_SHA256=81414fd0e41969f4f5c2e0555951cee76d3f224c112c269c80f52b31276f187a ;;
esac
RUSTFS_ZIP="rustfs-linux-$(uname -m)-musl-v$RUSTFS_VERSION.zip"
curl -fsSL --retry 5 --retry-all-errors -O "https://github.com/rustfs/rustfs/releases/download/$RUSTFS_VERSION/$RUSTFS_ZIP"
echo "$RUSTFS_SHA256  $RUSTFS_ZIP" | sha256sum -c
```

```text
rustfs-linux-<arch>-musl-v1.0.4.zip: OK
```

Install it, with an account of its own and a directory for its data:

<!-- docs-test: only rustfs -->
```sh
unzip -q "$RUSTFS_ZIP" rustfs
sudo install -m 0755 rustfs /usr/local/bin/rustfs
sudo useradd --system --user-group --no-create-home --home-dir /var/lib/rustfs --shell /usr/sbin/nologin rustfs
sudo install -d -m 0700 -o rustfs -g rustfs /var/lib/rustfs /var/lib/rustfs/data /var/lib/rustfs/logs
sudo install -d -m 0700 -o rustfs -g rustfs /etc/rustfs /etc/rustfs/tls
rustfs --version | head -n 1
```

```text
rustfs 1.0.4
```

Give it the certificate, as `rustfs_cert.pem` and `rustfs_key.pem`:

<!-- docs-test: only rustfs -->
```sh
sudo install -m 0400 -o rustfs -g rustfs objects.pem /etc/rustfs/tls/rustfs_cert.pem
sudo install -m 0400 -o rustfs -g rustfs objects.key /etc/rustfs/tls/rustfs_key.pem
```

Make the access key that vcon-server will use. RustFS reads its keys from
files. The secret is 64 random hexadecimal digits:

<!-- docs-test: only rustfs -->
```sh
printf %s vcon-server | sudo install -m 0400 -o rustfs -g rustfs /dev/stdin /etc/rustfs/access_key
openssl rand -hex 32 | tr -d '\n' | sudo install -m 0400 -o rustfs -g rustfs /dev/stdin /etc/rustfs/secret_key
sudo wc -c /etc/rustfs/access_key /etc/rustfs/secret_key
```

```text
11 /etc/rustfs/access_key
64 /etc/rustfs/secret_key
75 total
```

These are RustFS's own keys, which may do anything on it. On a RustFS that
other programs use too, make a user for vcon-server in the RustFS console
instead, with a policy that allows only the actions in
[before you start](#before-you-start) on its bucket.

Save this unit as `/etc/systemd/system/rustfs.service`:

<!-- docs-test: only rustfs; file /etc/systemd/system/rustfs.service -->
```ini
[Unit]
Description=RustFS
After=network-online.target

[Service]
User=rustfs
Group=rustfs
WorkingDirectory=/var/lib/rustfs
Environment=RUSTFS_CONSOLE_ENABLE=false
Environment=RUSTFS_OBS_LOG_DIRECTORY=/var/lib/rustfs/logs
ExecStart=/usr/local/bin/rustfs server --address :9000 --access-key-file /etc/rustfs/access_key --secret-key-file /etc/rustfs/secret_key --tls-path /etc/rustfs/tls /var/lib/rustfs/data
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Start it, now and at each boot. RustFS answers a request without a
signature with `403` once it is ready; while it starts it can answer
`503`, so the loop waits for the `403`:

<!-- docs-test: only rustfs -->
```sh
sudo systemctl daemon-reload
sudo systemctl enable --now rustfs
timeout 60 sh -c 'until [ "$(curl -s -o /dev/null -w "%{http_code}" --cacert /usr/local/share/vcon-server/s3-ca.pem https://objects.example.com:9000/)" = 403 ]; do sleep 1; done'
curl -s -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/s3-ca.pem https://objects.example.com:9000/
```

```text
Created symlink <unit link> → <unit file>.
403
```

Keep the secret in a variable for the next commands, and make the bucket
`vcon`. curl signs S3 requests itself with `--aws-sigv4`:

<!-- docs-test: only rustfs -->
```sh
S3_SECRET=$(sudo cat /etc/rustfs/secret_key)
S3=(--cacert /usr/local/share/vcon-server/s3-ca.pem --aws-sigv4 aws:amz:us-east-1:s3 --user "vcon-server:$S3_SECRET")
curl -sS --fail-with-body "${S3[@]}" -X PUT https://objects.example.com:9000/vcon && echo "bucket made"
```

```text
bucket made
```

Go on with [check the bucket](#check-the-bucket).

## Set up MinIO (on the storage host)

Download the MinIO server and its client, `mc`, for the host's CPU, from
their GitHub releases, and check each against the SHA-256 that MinIO
publishes beside it:

<!-- docs-test: only minio -->
```sh
MINIO_RELEASE=RELEASE.2025-09-07T16-13-09Z
MC_RELEASE=RELEASE.2025-08-13T08-35-41Z
case "$(uname -m)" in
  x86_64) MINIO_ARCH=amd64
    MINIO_SHA256=7c5bd8512c6e966455b1d198209358b2d191c77a83ab377c4073281065fb855f
    MC_SHA256=01f866e9c5f9b87c2b09116fa5d7c06695b106242d829a8bb32990c00312e891 ;;
  aarch64) MINIO_ARCH=arm64
    MINIO_SHA256=5c83cd2cf151717ba0243f73e1c7802ff36e272b67144bdd7f1f7d684fd6f03d
    MC_SHA256=14c8c9616cfce4636add161304353244e8de383b2e2752c0e9dad01d4c27c12c ;;
esac
curl -fsSL --retry 5 --retry-all-errors -o minio "https://github.com/minio/minio/releases/download/$MINIO_RELEASE/minio.linux-$MINIO_ARCH.$MINIO_RELEASE"
curl -fsSL --retry 5 --retry-all-errors -o mc "https://github.com/minio/mc/releases/download/$MC_RELEASE/mc.linux-$MINIO_ARCH.$MC_RELEASE"
printf '%s  %s\n' "$MINIO_SHA256" minio "$MC_SHA256" mc | sha256sum -c
```

```text
minio: OK
mc: OK
```

Install both, with an account of its own for the server and directories
for its data, its settings and its certificate:

<!-- docs-test: only minio -->
```sh
sudo install -m 0755 minio /usr/local/bin/minio
sudo install -m 0755 mc /usr/local/bin/mc
sudo useradd --system --user-group --no-create-home --home-dir /var/lib/minio --shell /usr/sbin/nologin minio
sudo install -d -m 0700 -o minio -g minio /var/lib/minio /var/lib/minio/data /etc/minio /etc/minio/certs
minio --version | head -n 1
```

```text
minio version RELEASE.2025-09-07T16-13-09Z (commit-id=07c3a429bfed433e49018cb0f78a52145d4bedeb)
```

MinIO reads its certificate and key from its certificate directory, as
`public.crt` and `private.key`:

<!-- docs-test: only minio -->
```sh
sudo install -m 0400 -o minio -g minio objects.pem /etc/minio/certs/public.crt
sudo install -m 0400 -o minio -g minio objects.key /etc/minio/certs/private.key
```

MinIO's root user may do anything on it, so vcon-server gets a user of its
own below, and the root user's password stays in a file only root may read.
systemd reads that file for the unit. `MINIO_SITE_REGION` names the region
requests must be signed for, and `MINIO_BROWSER=off` turns the web console
off:

<!-- docs-test: only minio -->
```sh
printf '%s\n' MINIO_ROOT_USER=minio-admin "MINIO_ROOT_PASSWORD=$(openssl rand -hex 32)" \
  MINIO_SITE_REGION=us-east-1 MINIO_BROWSER=off | sudo install -m 0400 /dev/stdin /etc/minio/minio.env
sudo wc -l /etc/minio/minio.env
```

```text
4 /etc/minio/minio.env
```

Save this unit as `/etc/systemd/system/minio.service`:

<!-- docs-test: only minio; file /etc/systemd/system/minio.service -->
```ini
[Unit]
Description=MinIO
After=network-online.target

[Service]
User=minio
Group=minio
WorkingDirectory=/var/lib/minio
EnvironmentFile=/etc/minio/minio.env
ExecStart=/usr/local/bin/minio server --address :9000 --console-address 127.0.0.1:9001 --certs-dir /etc/minio/certs /var/lib/minio/data
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Start it, now and at each boot. MinIO answers a request without a
signature with `403` once it is ready; while it starts it can answer
`503`, so the loop waits for the `403`:

<!-- docs-test: only minio -->
```sh
sudo systemctl daemon-reload
sudo systemctl enable --now minio
timeout 60 sh -c 'until [ "$(curl -s -o /dev/null -w "%{http_code}" --cacert /usr/local/share/vcon-server/s3-ca.pem https://objects.example.com:9000/)" = 403 ]; do sleep 1; done'
curl -s -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/s3-ca.pem https://objects.example.com:9000/
```

```text
Created symlink <unit link> → <unit file>.
403
```

`mc` trusts the certificate authorities in `~/.mc/certs/CAs/` as well as
the system's. Give it the private authority, and the root user's keys in
the `MC_HOST_storage` variable, which names the alias `storage` without
writing the password to `mc`'s configuration file. Then make the bucket
`vcon`:

<!-- docs-test: only minio -->
```sh
install -d -m 0700 ~/.mc ~/.mc/certs ~/.mc/certs/CAs
install -m 0644 /usr/local/share/vcon-server/s3-ca.pem ~/.mc/certs/CAs/objects-ca.pem
MINIO_ROOT_PASSWORD=$(sudo sed -n 's/^MINIO_ROOT_PASSWORD=//p' /etc/minio/minio.env)
export MC_HOST_storage="https://minio-admin:$MINIO_ROOT_PASSWORD@objects.example.com:9000"
mc mb storage/vcon
```

```text
Bucket created successfully `storage/vcon`.
```

Save the policy for vcon-server's user as `vcon-server-policy.json`. It
allows the five actions in [before you start](#before-you-start) on the
bucket `vcon` alone:

<!-- docs-test: only minio; file vcon-server-policy.json -->
```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:ListBucket", "s3:GetBucketVersioning"],
      "Resource": "arn:aws:s3:::vcon"
    },
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
      "Resource": "arn:aws:s3:::vcon/*"
    }
  ]
}
```

Make the user `vcon-server` with a secret of 64 random hexadecimal digits,
and attach the policy to it:

<!-- docs-test: only minio -->
```sh
S3_SECRET=$(openssl rand -hex 32)
mc admin user add storage vcon-server "$S3_SECRET"
mc admin policy create storage vcon-server vcon-server-policy.json
mc admin policy attach storage vcon-server --user vcon-server
S3=(--cacert /usr/local/share/vcon-server/s3-ca.pem --aws-sigv4 aws:amz:us-east-1:s3 --user "vcon-server:$S3_SECRET")
```

```text
Added user `vcon-server` successfully.
Created policy `vcon-server` successfully.
Attached Policies: [vcon-server]
To User: vcon-server
```

The user may not make a bucket, which shows that the policy holds:

<!-- docs-test: only minio -->
```sh
curl -s -o /dev/null -w '%{http_code}\n' "${S3[@]}" -X PUT https://objects.example.com:9000/another-bucket
```

```text
403
```

`mc version info` tells whether the bucket was ever versioned:

<!-- docs-test: only minio -->
```sh
mc version info storage/vcon
```

```text
storage/vcon is un-versioned
```

`mc admin user add` and curl's `--user` put the secret on the command line,
where other accounts on the host can see it while the command runs. Run
these commands on a host whose other accounts you trust.

## Check the bucket

With the key vcon-server will use, read the bucket's versioning:

```sh
curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon?versioning" | grep -o '<VersioningConfiguration.*'
```

```text
<VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"></VersioningConfiguration>
```

The answer has no `<Status>`: versioning was never turned on. A bucket that
had it on answers `<Status>Enabled</Status>` or `<Status>Suspended</Status>`,
and the server refuses it (see [versioning](#the-bucket-has-versioning-on)).
`--user` puts the secret on curl's command line, where other accounts on the
host can see it while curl runs. Run these commands on a host whose other
accounts you trust, or use your service's own client.

Port 9000 must be open from the vcon-server host to the storage host, and
from nowhere else that has no use for the bucket.

## Give vcon-server the key

The server reads the secret, when it starts, from a file that only its
account may read. Install it:

```sh
printf %s "$S3_SECRET" | sudo install -m 0400 -o vcon -g vcon /dev/stdin /etc/vcon-server/s3.secret
sudo stat -c '%a %U %n' /etc/vcon-server/s3.secret
```

```text
400 vcon /etc/vcon-server/s3.secret
```

The file holds the secret on one line; a line ending after it is left out.
An empty file, or one with a second line, is refused at start. The server
keeps the secret in memory that is zeroed when it is freed, and never logs
it or puts it in an error.

Instead of a key in the file, the server can take its keys from the AWS
default credential chain: the `AWS_ACCESS_KEY_ID` and
`AWS_SECRET_ACCESS_KEY` variables, the shared credentials file, web
identity, or an EC2 instance's or ECS task's role. It does so when
`storage.s3.access_key_id` is left out. Under systemd, set the variables in
a file that only root may read, and name it in a drop-in for the unit:

<!-- docs-test: skip the run gives the key in the configuration and the key file -->
```sh
printf '%s\n' AWS_ACCESS_KEY_ID=vcon-server "AWS_SECRET_ACCESS_KEY=$S3_SECRET" | sudo install -m 0400 /dev/stdin /etc/vcon-server/s3.env
sudo install -d -m 0755 /etc/systemd/system/vcon-server.service.d
printf '%s\n' '[Service]' 'EnvironmentFile=/etc/vcon-server/s3.env' | sudo tee /etc/systemd/system/vcon-server.service.d/s3.conf >/dev/null
sudo systemctl daemon-reload
```

## Add the storage table

Save the configuration again, with a `[storage]` table. RustFS and MinIO
name the bucket in the request path (`path_style = true`). AWS S3 names it
in the host name, which is the default (see [AWS S3](#aws-s3)):

<!-- docs-test: file vcon-server.toml -->
```toml
listen = "0.0.0.0:8443"
data_dir = "/var/lib/vcon-server"
master_key = "/etc/vcon-server/master.key"

[tls]
cert = "/etc/vcon-server/cert.pem"
key = "/etc/vcon-server/key.pem"

[storage]
objects = "s3"

[storage.s3]
endpoint = "https://objects.example.com:9000"
region = "us-east-1"
bucket = "vcon"
path_style = true
access_key_id = "vcon-server"
secret_access_key_file = "/etc/vcon-server/s3.secret"
ca_file = "/usr/local/share/vcon-server/s3-ca.pem"
```

`region` is the region the requests are signed for. RustFS accepts any
region. MinIO accepts any region unless `MINIO_SITE_REGION` names one, as
above, and then only that one. Every key of the table is on the
[configuration page](@/docs/configuration.md#storage).

## Move objects stored in the data directory

Stop the server, install the configuration, and move the objects already in
the data directory to the bucket, then start the server again. `storage
move-objects` writes each object to the bucket, reads it back and compares
it byte for byte, and only then removes its file, so a run that stops
(the bucket cannot be reached, a file cannot be read) loses nothing: run it
again to move the rest. It prints one line per tenant with how many objects
it moved and how many versions `check` then finds without an object, and a
total. The server on this page has stored nothing yet, so it moves none:

```sh
sudo install -m 0400 -o vcon -g vcon vcon-server.toml /etc/vcon-server/vcon-server.toml
sudo systemctl stop vcon-server
sudo -u vcon vcon-server storage move-objects --config /etc/vcon-server/vcon-server.toml | jq -c 'select(.tenant != null) | {missing}'
sudo systemctl start vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
```

```text
{"missing":0}
answering
```

The command claims the bucket and prefix for this data directory, as the
server does. At start, the server reads the secret file, reaches the bucket
and reads its versioning, checks that claim, and stops with an error naming
the setting or the bucket when one of these fails. `missing` above 0 names
versions whose object is in neither place: `vcon-server check` lists them.

## Check that the objects go to the bucket

Store a vCon with the write key:

```sh
UUID=$(cat /proc/sys/kernel/random/uuid)
cat > call.vcon <<EOF
{"vcon":"0.4.0","uuid":"$UUID","created_at":"2026-10-27T12:00:00Z","parties":[{"tel":"+15555550101","name":"Caller Example"}],"dialog":[],"attachments":[],"analysis":[]}
EOF
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/vcons \
  -H "Authorization: Bearer $RECORDER" -H "Content-Type: application/vcon" --data-binary @call.vcon
```

```text
201
```

List the bucket. It holds one object for the version, under the tenant's
id, and the server's claim on the bucket (see
[one server per bucket and prefix](#one-server-per-bucket-and-prefix)):

```sh
curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon?list-type=2" | grep -o '<Key>[^<]*' | cut -c6-
```

```text
tenants/<tenant id>/objects/<version id>
vcon-server.claim
```

The object is sealed: it does not hold the vCon's uuid, or any other part
of it. The data directory holds no object file:

```sh
KEY=$(curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon?list-type=2" | grep -o '<Key>tenants/[^<]*' | cut -c6-)
curl -sS --fail-with-body "${S3[@]}" -o object.bin "https://objects.example.com:9000/vcon/$KEY"
grep -q "$UUID" object.bin || echo "no uuid in the object"
sudo find /var/lib/vcon-server/tenants -path '*/objects/*' -type f | wc -l
```

```text
no uuid in the object
0
```

## Check that a purge removes the object

A purge removes the version's object from the bucket, not only its row
from the catalog. With a retention period of `0d`, a delete purges at once.
Make a key that may delete, set acme's period to `0d`, and delete the vCon:

```sh
DELETER=$(sudo -u vcon vcon-server key create --tenant "$TENANT" --scope delete --label s3-check \
  --config /etc/vcon-server/vcon-server.toml | jq -r .token)
sudo -u vcon vcon-server tenant set-retention "$TENANT" 0d --config /etc/vcon-server/vcon-server.toml | jq -c '{retention}'
curl -sS -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/ca.pem -X DELETE \
  "https://vcon.example.com:8443/vcons/$UUID" -H "Authorization: Bearer $DELETER"
curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon?list-type=2" | grep -o '<Key>[^<]*' | cut -c6-
```

```text
{"retention":"0d"}
204
vcon-server.claim
```

A vCon under a [legal hold](@/docs/retention-and-holds.md) is never purged,
and its objects stay in the bucket. Put acme's period back to the server's
default:

```sh
sudo -u vcon vcon-server tenant set-retention "$TENANT" default --config /etc/vcon-server/vcon-server.toml | jq -c '{retention}'
```

```text
{"retention":null}
```

## One server per bucket and prefix

At start, the server removes every object under its prefix that no catalog
of its own names: these are the leftovers of a write cut off by a crash.
When it removes any, it logs how many, with no tenant and no name:

```text
INFO vcon_server::store: start-up recovery removed objects that no catalog row names removed=1
```

So two servers must never share a bucket and prefix, or each would remove
the other's vCons. To prevent that, the first server to start on an empty
prefix claims it: it writes a random claim id to `storage.claim` in its data
directory and to `vcon-server.claim` under the prefix, with
`If-None-Match: *`. The two match:

```sh
sudo -u vcon cat /var/lib/vcon-server/storage.claim | cmp - <(curl -sS --fail-with-body "${S3[@]}" https://objects.example.com:9000/vcon/vcon-server.claim) && echo "the same claim"
```

```text
the same claim
```

A server whose data directory holds another claim, or none, refuses to
start on that prefix, and so does a server on a prefix that holds objects
but no claim. To run a second server on the same bucket, give it a prefix
of its own, such as `prefix = "server-b/"`. A restored
[backup](@/docs/backup-and-restore.md#objects-in-an-s3-bucket) keeps its
objects as files and has no `storage.claim`, so a server started from it
with this storage table refuses this prefix too.

The steps above are those of a server with its catalogs in the data
directory. The [PostgreSQL catalog preview](@/docs/postgres-catalog.md)
keeps the same id in the row of the table `storage_claim` of its
database, from schema version 13, where a data directory keeps it in
`storage.claim`: a server with its catalogs in PostgreSQL writes that row
at its first start, and every host that serves the database shares it.
`vcon-server storage move-objects` is refused with a PostgreSQL catalog,
whose objects are in the bucket from the first write.

## Clean up

The working directory holds the files the steps above made. Remove the
copies of the keys and the vCon:

```sh
rm -f objects.key objects.csr objects.ext objects.pem objects-ca.pem call.vcon object.bin vcon-server.toml
unset S3_SECRET S3 MINIO_ROOT_PASSWORD MC_HOST_storage
```

`objects-ca.key` is the private authority's key: whoever holds it can make
a certificate the server trusts for the bucket. Keep it offline, on
removable media kept apart from both hosts, and remove it from this host.
You need it again only to sign the storage host's next certificate, before
`objects.pem` expires in 365 days. Copy it to that media first, then:

```sh
shred -u objects-ca.key
ls objects-ca.key 2>/dev/null || echo "objects-ca.key is gone"
```

```text
objects-ca.key is gone
```

To take the storage table away from a server that has stored vCons in the
bucket is a change of storage: see the top of this page.

## AWS S3

This section was not run: see [what was tested](#what-was-tested-and-what-was-not).
The bucket name `amzn-s3-demo-vcon-objects` is a placeholder. AWS reserves
every bucket name that starts with `amzn-s3-demo-` for its own documentation
and lets no one make such a bucket, so a command copied from here cannot
reach someone else's bucket. Write your own bucket's name wherever it
appears.

### Set up the bucket and its role

Make the bucket in the region the server will use, with the AWS CLI. New
buckets block public access and have versioning off by default:

<!-- docs-test: skip the run has no AWS account -->
```sh
aws s3api create-bucket --bucket amzn-s3-demo-vcon-objects --region eu-west-1 \
  --create-bucket-configuration LocationConstraint=eu-west-1
```

Give the server's IAM role, or the IAM user whose keys it uses, this policy
on the bucket alone:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:ListBucket", "s3:GetBucketVersioning"],
      "Resource": "arn:aws:s3:::amzn-s3-demo-vcon-objects"
    },
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
      "Resource": "arn:aws:s3:::amzn-s3-demo-vcon-objects/*"
    }
  ]
}
```

### Configure the server

AWS S3 names the bucket in the host name (virtual-hosted style), so
`path_style` stays `false`, and with no `endpoint` the server uses the
region's own S3 endpoint. With no `access_key_id`, the server takes its
keys from the AWS default credential chain, such as the EC2 instance's role:

```toml
[storage]
objects = "s3"

[storage.s3]
region = "eu-west-1"
bucket = "amzn-s3-demo-vcon-objects"
```

With an IAM user's keys instead of a role, set `access_key_id` to the key's
id and `secret_access_key_file` to a file holding its secret, installed as
[give vcon-server the key](#give-vcon-server-the-key) shows.

### Check it

Before the server starts on it, check that the bucket has never had
versioning on. The answer must have no `Status`:

<!-- docs-test: skip the run has no AWS account -->
```sh
aws s3api get-bucket-versioning --bucket amzn-s3-demo-vcon-objects
```

After a vCon is stored, the bucket holds one object under
`tenants/<tenant id>/objects/` and `vcon-server.claim`:

<!-- docs-test: skip the run has no AWS account -->
```sh
aws s3api list-objects-v2 --bucket amzn-s3-demo-vcon-objects --query 'Contents[].Key'
```

### When it does not start on AWS S3

- **No credentials.** With no `access_key_id` and nothing in the credential
  chain, the start stops with the reason for each source. This was captured
  with no credentials set, against no service:

  ```text
  vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: dispatch failure: other: the credential provider was not enabled: no credentials found in chain. Attempted:
    Environment: the credential provider was not enabled: environment variable not set
    Profile: the credential provider was not enabled
    WebIdentityToken: the credential provider was not enabled: $AWS_WEB_IDENTITY_TOKEN_FILE was not set
    EcsContainer: the credential provider was not enabled: ECS provider not configured
    Ec2InstanceMetadata: the credential provider was not enabled: IMDS disabled by AWS_EC2_METADATA_DISABLED env var set to `true`
  ```

  Attach the role to the instance or task, or set the keys as
  [give vcon-server the key](#give-vcon-server-the-key) shows.
- **A missing permission, or a wrong region.** The line starts
  `bucket "<bucket>" cannot be reached or read: the object storage failed: service error:`
  and then gives AWS's error code. These were not captured from AWS; the
  same refusals from MinIO are under
  [access denied](#the-key-may-not-do-what-the-server-needs) and
  [wrong region](#the-region-is-wrong). Check the policy above, and that
  `region` is the bucket's region.

## When it does not start

The server stops at start with an error that names the setting or the
bucket, and exits 1. Each message below was printed by the server, except
where it says otherwise; paths, request ids and claim ids differ.

### The build has no S3 support

```text
vcon-server: the storage cannot be used: storage.objects is "s3", but this build has no S3 support (cargo feature s3)
```

The server was built with `--no-default-features`. Release 0.3.0 has no
storage table at all, and refuses it as a key it does not know:

```text
vcon-server: configuration: TOML parse error at line 8, column 2
  |
8 | [storage]
  |  ^^^^^^^
unknown field `storage`, expected one of `listen`, `tls`, `data_dir`, `master_key`, `retention_default`, `purge_interval`, `rotation_warn`, `limits`, `supported_critical`, `mcp`
```

Install 0.4.0 or later from a release, or build it with the default
features, as the top of this page shows.

### The storage table does not hold together

The configuration is checked before anything is reached, and the start
stops with `configuration:` and the reason:

```text
vcon-server: configuration: storage.s3 is set but storage.objects is not "s3"
vcon-server: configuration: storage.objects is "s3" but there is no storage.s3 table
vcon-server: configuration: storage.objects: "minio" is not "local" or "s3"
vcon-server: configuration: storage.s3.bucket is empty
vcon-server: configuration: storage.s3.prefix "server-a" must not start with / and must end with /, such as "server-a/"
vcon-server: configuration: storage.s3.endpoint "http://objects.example.com:9000" is plain HTTP; set storage.s3.allow_http = true only for a test service on this host
vcon-server: configuration: storage.s3.ca_file is set, but storage.s3.endpoint "http://objects.example.com:9000" is plain HTTP, which uses no certificate
vcon-server: configuration: storage.s3.endpoint "objects.example.com:9000" is not an https:// URL
vcon-server: configuration: storage.s3.access_key_id is set without storage.s3.secret_access_key_file
vcon-server: configuration: storage.s3.secret_access_key_file is set without storage.s3.access_key_id
vcon-server: configuration: storage.s3.timeout must be more than 0
```

Each line is one refusal. A key the server does not know, or a missing
`region` or `bucket`, is a TOML error that names the line and the key.
Every key is on the [configuration page](@/docs/configuration.md#storage).

### The secret file cannot be used

```text
vcon-server: the storage cannot be used: storage.s3.secret_access_key_file /etc/vcon-server/s3.secret: No such file or directory (os error 2)
vcon-server: the storage cannot be used: storage.s3.secret_access_key_file /etc/vcon-server/s3.secret: the file is empty
vcon-server: the storage cannot be used: storage.s3.secret_access_key_file /etc/vcon-server/s3.secret: the file holds more than one line
```

The file is missing or the server's account may not read it, holds
nothing, or holds more than one line. The message never holds the file's
text. Install the secret again, as [above](#give-vcon-server-the-key).

### The secret is wrong

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: service error: unhandled error (SignatureDoesNotMatch): Error { code: "SignatureDoesNotMatch", message: "The request signature we calculated does not match the signature you provided. Check your AWS secret access key and signing method. For more information, see REST Authentication and SOAP Authentication for details.", aws_request_id: "<id>" }
```

That is RustFS's answer; MinIO's message is
`The request signature we calculated does not match the signature you provided. Check your key and signing method.`
The secret file does not hold the secret for `access_key_id`. Install the
secret again, as [above](#give-vcon-server-the-key).

### The key may not do what the server needs

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: service error: unhandled error (AccessDenied): Error { code: "AccessDenied", message: "Access Denied.", aws_request_id: "<id>", s3_extended_request_id: "<id>" }
```

This is MinIO's answer for a user whose policy leaves out
`s3:GetBucketVersioning`. Give the key every action in
[before you start](#before-you-start), on this bucket.

### The region is wrong

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: service error: unhandled error (AuthorizationHeaderMalformed): Error { code: "AuthorizationHeaderMalformed", message: "The authorization header is malformed; the region is wrong; expecting 'eu-west-1'.", s3_extended_request_id: "<id>", aws_request_id: "<id>" }
```

This is MinIO's answer when `MINIO_SITE_REGION` is `eu-west-1` and
`region` is `us-east-1`. Set `region` to the one the message expects.
RustFS, and MinIO with no `MINIO_SITE_REGION`, accept any region.

### The bucket does not exist

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: service error: unhandled error (NoSuchBucket): Error { code: "NoSuchBucket", message: "The specified bucket does not exist", aws_request_id: "<id>" }
```

Make the bucket first. With AWS S3, check `region`, and that `path_style`
is `false`.

### The certificate authority file cannot be used

```text
vcon-server: the storage cannot be used: storage.s3.ca_file /usr/local/share/vcon-server/s3-ca.pem: No such file or directory (os error 2)
vcon-server: the storage cannot be used: storage.s3.ca_file /usr/local/share/vcon-server/s3-ca.pem: the file holds no certificate
vcon-server: the storage cannot be used: storage.s3.ca_file /usr/local/share/vcon-server/s3-ca.pem: a certificate cannot be trusted: invalid peer certificate: BadEncoding
```

The file is missing, holds no PEM certificate, or holds one that cannot be
read. Install the authority's certificate again, as
[above](#give-the-storage-host-a-certificate-on-the-storage-host).

### The certificate is not trusted

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: dispatch failure: io error: client error (Connect): TLS handshake failed: invalid peer certificate: UnknownIssuer
```

The service's certificate was signed by an authority the server does not
trust. Set `ca_file` to that authority's certificate. With `ca_file` set,
the server trusts only the authorities in that file, not the system's.
For MinIO's `mc`, the same certificate goes in `~/.mc/certs/CAs/`.

### The service does not answer

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: dispatch failure: io error: client error (Connect): tcp connect error: Connection refused (os error 111)
```

Nothing listens at `endpoint`, or a firewall refuses the connection. Check
that the service runs and that its port is open from this host.

### The service takes too long

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: request has timed out: operation timeout (all attempts including retries) occurred after 2s
```

The service accepted the connection and did not answer within
`storage.s3.timeout` seconds (2 here; 30 by default), retries included.
Check the service's own log and load, and the network between the hosts.
Raise `timeout` only when the service is slow but working.

### The bucket has versioning on

```text
vcon-server: the storage cannot be used: bucket "vcon" has versioning Enabled, which keeps a removed object's earlier versions; turn it off or set storage.s3.allow_versioning = true
vcon-server: the storage cannot be used: bucket "vcon" has versioning Suspended, which keeps a removed object's earlier versions; turn it off or set storage.s3.allow_versioning = true
```

A versioned bucket keeps each purged object's earlier version until the
bucket's own lifecycle rules remove it. That version cannot be opened once
its row is purged, because its key was kept only in that row, but its bytes
stay in the bucket. `Suspended` means versioning was on once: S3 cannot turn
it off again, and suspending it does not remove the versions already kept.
Use a bucket that never had versioning, or set `allow_versioning = true`
if the bucket's rules remove noncurrent versions.

### Another server holds the prefix

```text
vcon-server: the storage cannot be used: bucket "vcon", prefix "" belongs to another data directory (its claim is <claim id>); start-up recovery here would remove that server's vCons. Give this server a prefix of its own, or point it at the data directory that claimed this one
```

Another data directory claimed this bucket and prefix (see
[one server per bucket and prefix](#one-server-per-bucket-and-prefix)).
Give this server a `prefix` of its own.

### The prefix holds objects but no claim

```text
vcon-server: the storage cannot be used: bucket "vcon", prefix "" holds tenant objects but no claim; start-up recovery would remove every object no catalog here names. Give this server an empty prefix
```

The prefix holds objects that no claim covers, such as those of a server
whose data directory was lost, or of a server on this prefix whose claim
object was removed. The server does not take them over, because start-up
recovery would remove every object its catalogs do not name. Give it an
empty prefix.

### The prefix cannot be claimed or read

```text
vcon-server: the storage cannot be used: bucket "vcon", prefix "" cannot be claimed: the object storage failed: service error: unhandled error (AccessDenied): Error { code: "AccessDenied", message: "Access Denied.", aws_request_id: "<id>", s3_extended_request_id: "<id>" }
vcon-server: the storage cannot be used: bucket "vcon", prefix "" cannot be read: the object storage failed: vcon-server.claim does not hold a claim id
```

The first is MinIO's answer for a key that may not write
`vcon-server.claim`; the claim is written with `If-None-Match: *`, so
another server's claim written at the same moment is refused the same way,
with `412` (`PreconditionFailed`) as the code; that case was not captured.
The second means `vcon-server.claim` holds something other than a claim id.
Give the key `s3:PutObject` on the whole prefix, and do not write
`vcon-server.claim` by hand.

`vcon-server check` lists, for each tenant, the versions whose object is
missing and the objects no row names. Under a bucket, it reads every
object, so it makes one request for each version.
