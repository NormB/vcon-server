+++
title = "Keep vCons in an S3 bucket"
description = "Keep each version's sealed object in an S3-compatible bucket (RustFS, MinIO or AWS S3) instead of the data directory: the bucket and its keys on the storage side, the configuration, a check that the objects go there and that a purge removes them, and what each refusal means."
weight = 11
+++

By default the server keeps every stored version as a sealed object file in
its data directory. This page moves those objects to a bucket in an
S3-compatible service: [RustFS](https://rustfs.com/),
[MinIO](https://min.io/) or [AWS S3](https://aws.amazon.com/s3/). The
catalogs, `server.sqlite`, the lock files and the master key stay on the
server's host. Each object in the bucket is the same sealed bytes the server
would write to a file, so the bucket's operator sees random names, sizes and
times, and no vCon.

Choose the storage before the first vCon is stored. A server that changes
its storage later leaves each version it stored where it was: it answers
`500` for those versions, and `vcon-server check` lists them as damaged.
vcon-server has no command that moves objects from one storage to another.

S3 storage is in vcon-server after release 0.3.0; 0.3.0 keeps its objects in
the data directory only. It is built with the `s3` cargo feature, which is
on by default.

CI runs this page from top to bottom on each change, after
[getting started](@/docs/getting-started.md), in a Debian 13 container under
systemd. RustFS runs in the same container there, at the name
`objects.example.com`. In a real deployment the storage service is another
host: run the steps marked "on the storage host" there.

## Before you start

- **A server set up as [getting started](@/docs/getting-started.md) shows,**
  that has stored no vCon yet. The steps below use its shell variables
  (`$TENANT`, `$RECORDER`, `$OPS`) and its files in `/etc/vcon-server`.
- **An S3-compatible service and a bucket in it** whose versioning has never
  been turned on. A versioned bucket keeps a purged object's earlier
  versions, so the server refuses it unless told otherwise (see
  [versioning](#the-bucket-has-versioning-on)). The next section sets up
  RustFS. For MinIO or AWS S3, see [other services](#other-services).
- **An access key for the bucket** that may get, put, delete and list
  objects and read the bucket's versioning (`s3:GetObject`, `s3:PutObject`,
  `s3:DeleteObject`, `s3:ListBucket`, `s3:GetBucketVersioning`), and nothing
  else.
- **TLS.** The server reaches the service over `https://`. When the
  service's certificate comes from a private certificate authority, the
  server needs that authority's certificate. The RustFS steps below make one.
- **`unzip`,** to unpack the RustFS release. On Debian or Ubuntu:

  <!-- docs-test: only apt -->
  ```sh
  sudo apt-get install -y -qq unzip >/dev/null
  ```

## Set up RustFS (on the storage host)

Download a RustFS release for the host's CPU and check it against the
SHA-256 that RustFS publishes for it:

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

Give it a certificate. With a certificate from your certificate authority
for the storage host's name, install it as `rustfs_cert.pem` and its key as
`rustfs_key.pem`:

<!-- docs-test: skip the run makes a private certificate authority below instead -->
```sh
sudo install -m 0400 -o rustfs -g rustfs fullchain.pem /etc/rustfs/tls/rustfs_cert.pem
sudo install -m 0400 -o rustfs -g rustfs privkey.pem /etc/rustfs/tls/rustfs_key.pem
```

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
sudo install -m 0400 -o rustfs -g rustfs objects.pem /etc/rustfs/tls/rustfs_cert.pem
sudo install -m 0400 -o rustfs -g rustfs objects.key /etc/rustfs/tls/rustfs_key.pem
sudo install -m 0644 objects-ca.pem /usr/local/share/vcon-server/s3-ca.pem
openssl verify -CAfile /usr/local/share/vcon-server/s3-ca.pem objects.pem
```

```text
objects.pem: OK
```

The authority's certificate is now at
`/usr/local/share/vcon-server/s3-ca.pem`, readable by every account, for
the server and for `curl`. Keep `objects-ca.key` offline: whoever holds it
can make a certificate the server trusts for the bucket.

Make the access key that vcon-server will use. RustFS reads its keys from
files. The secret is 64 random hexadecimal digits:

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

<!-- docs-test: file /etc/systemd/system/rustfs.service -->
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

Start it, now and at each boot. Until DNS gives `objects.example.com` to the
storage host, add the name to `/etc/hosts` on the vcon-server host. RustFS
answers a request without a signature with `403`, which shows that it
listens:

```sh
echo '127.0.0.1 objects.example.com' | sudo tee -a /etc/hosts >/dev/null
sudo systemctl daemon-reload
sudo systemctl enable --now rustfs
timeout 60 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/s3-ca.pem https://objects.example.com:9000/; do sleep 1; done'
curl -s -o /dev/null -w '%{http_code}\n' --cacert /usr/local/share/vcon-server/s3-ca.pem https://objects.example.com:9000/
```

```text
Created symlink <unit link> → <unit file>.
403
```

Port 9000 must be open from the vcon-server host to the storage host, and
from nowhere else that has no use for the bucket.

## Make the bucket

curl signs S3 requests itself with `--aws-sigv4`. Keep the secret in a
variable for the next commands, and make the bucket `vcon`:

```sh
S3_SECRET=$(sudo cat /etc/rustfs/secret_key)
S3=(--cacert /usr/local/share/vcon-server/s3-ca.pem --aws-sigv4 aws:amz:us-east-1:s3 --user "vcon-server:$S3_SECRET")
curl -sS --fail-with-body "${S3[@]}" -X PUT https://objects.example.com:9000/vcon
curl -sS --fail-with-body "${S3[@]}" "https://objects.example.com:9000/vcon?versioning"; echo
```

```text
<?xml version="1.0" encoding="UTF-8"?><VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"></VersioningConfiguration>
```

The versioning answer has no `<Status>`: versioning was never turned on.
`--user` puts the secret on curl's command line, where other accounts on the
host can see it while curl runs. Run these commands on a host whose other
accounts you trust, or use your service's own client.

## Give vcon-server the key

The server reads the secret, when it starts, from a file that only its
account may read. Install it:

```sh
sudo install -m 0400 -o vcon -g vcon /etc/rustfs/secret_key /etc/vcon-server/s3.secret
sudo stat -c '%a %U %n' /etc/vcon-server/s3.secret
```

```text
400 vcon /etc/vcon-server/s3.secret
```

The file holds the secret on one line; a line ending after it is left out.
The server keeps the secret in memory that is zeroed when it is freed, and
never logs it or puts it in an error.

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
in the host name, which is the default (see
[other services](#other-services)):

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

`region` is the region the requests are signed for. RustFS and MinIO accept
`us-east-1` unless they were set up with another. Every key of the table is
on the [configuration page](@/docs/configuration.md#storage).

Install it and restart the server. At start, the server reads the secret
file, reaches the bucket and reads its versioning, and stops with an error
naming the setting or the bucket when one of these fails:

```sh
sudo install -m 0400 -o vcon -g vcon vcon-server.toml /etc/vcon-server/vcon-server.toml
sudo systemctl restart vcon-server
timeout 30 sh -c 'until curl -s -o /dev/null --cacert /usr/local/share/vcon-server/ca.pem https://vcon.example.com:8443/health; do sleep 1; done' && echo answering
```

```text
answering
```

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
of its own names: these are the leftovers of a write cut off by a crash. So
two servers must never share a bucket and prefix, or each would remove the
other's vCons. To prevent that, the first server to start on an empty
prefix claims it: it writes a random claim id to `storage.claim` in its data
directory and to `vcon-server.claim` under the prefix. The two match:

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
[backup](@/docs/backup-and-restore.md) keeps its objects as files and has
no `storage.claim`, so a server started from it with this storage table
refuses this prefix too.

## Other services

**MinIO** takes the same table as RustFS, with its own address. Make the
bucket and a user for vcon-server with MinIO's `mc` client:

<!-- docs-test: skip the run sets up RustFS, not MinIO -->
```sh
mc alias set storage https://objects.example.com:9000 ROOT_USER ROOT_PASSWORD
mc mb storage/vcon
mc admin user add storage vcon-server "$S3_SECRET"
mc admin policy create storage vcon-server vcon-server-policy.json
mc admin policy attach storage vcon-server --user vcon-server
```

**AWS S3** names the bucket in the host name (virtual-hosted style), so
`path_style` stays `false`, and with no `endpoint` the server uses the
region's own S3 endpoint:

```toml
[storage]
objects = "s3"

[storage.s3]
region = "eu-west-1"
bucket = "example-vcon-objects"
```

With no `access_key_id`, the server takes its keys from the AWS default
credential chain, such as the EC2 instance's role. The role's policy, on
the bucket alone (this is also the policy file for MinIO above, with its
own bucket name):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:ListBucket", "s3:GetBucketVersioning"],
      "Resource": "arn:aws:s3:::example-vcon-objects"
    },
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
      "Resource": "arn:aws:s3:::example-vcon-objects/*"
    }
  ]
}
```

Check that the bucket has never had versioning on. The answer must have no
`Status`:

<!-- docs-test: skip the run has no AWS account -->
```sh
aws s3api get-bucket-versioning --bucket example-vcon-objects
```

## When it does not start

The server stops at start with `the storage cannot be used:` and the
reason. Each reason below is what the server prints for it.

### The secret is wrong

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: service error: unhandled error (SignatureDoesNotMatch): Error { code: "SignatureDoesNotMatch", message: "The request signature we calculated does not match the signature you provided. Check your AWS secret access key and signing method. For more information, see REST Authentication and SOAP Authentication for details.", aws_request_id: "<id>" }
```

The secret file does not hold the secret for `access_key_id`, or the region
is not the one the service signs for. Install the secret again, as
[above](#give-vcon-server-the-key).

### The bucket does not exist

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: service error: unhandled error (NoSuchBucket): Error { code: "NoSuchBucket", message: "The specified bucket does not exist", aws_request_id: "<id>" }
```

Make the bucket first. With AWS S3, check `region`, and that `path_style`
is `false`.

### The certificate is not trusted

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: dispatch failure: io error: client error (Connect): TLS handshake failed: invalid peer certificate: UnknownIssuer
```

The service's certificate was signed by an authority the server does not
trust. Set `ca_file` to that authority's certificate. With `ca_file` set,
the server trusts only the authorities in that file, not the system's.

### The service does not answer

```text
vcon-server: the storage cannot be used: bucket "vcon" cannot be reached or read: the object storage failed: dispatch failure: io error: client error (Connect): tcp connect error: Connection refused (os error 111)
```

Nothing listens at `endpoint`, or a firewall refuses the connection. Check
that the service runs and that its port is open from this host.

### The bucket has versioning on

```text
vcon-server: the storage cannot be used: bucket "vcon" has versioning Enabled, which keeps a removed object's earlier versions; turn it off or set storage.s3.allow_versioning = true
```

A versioned bucket keeps each purged object's earlier version until the
bucket's own lifecycle rules remove it. That version cannot be opened once
its row is purged, because its key was kept only in that row, but its bytes
stay in the bucket. Use a bucket that never had versioning, or set
`allow_versioning = true` if the bucket's rules remove noncurrent versions.
Suspending versioning does not remove the versions already kept.

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

`vcon-server check` lists, for each tenant, the versions whose object is
missing and the objects no row names. Under a bucket, it reads every
object, so it makes one request for each version.
