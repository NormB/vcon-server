#!/usr/bin/env python3
"""Write the on-disk format fixtures: a data directory made by each released
binary, for crates/vcon-server/tests/formats.rs to open with this build.

    python3 -I scripts/make-format-fixtures.py --cache DIR [--release X ...]
                                               [--out DIR] [--arch ARCH]

For each release (every one in RELEASES when none is named) it:

1. finds the release archive for ARCH in scripts/docs-test-pins.txt and
   puts its pinned bytes in the cache with scripts/fetch-pinned.sh, which
   checks the sha256 on every download and every cache hit;
2. unpacks the binary and runs it in throw-away containers of IMAGE (pinned
   by digest), with no network but the one published port: the commands
   that run while no server does (`tenant create`, `key create`, `user add`,
   `membership grant`), then `serve`, driven over HTTPS from this script
   with the WORKLOAD below, then `serve` again (its start-up sweep purges
   what is due), then a stop by SIGTERM;
3. checks over the legal routes, before the last stop, that the server
   kept exactly the versions the workload expects, each body byte-equal to
   the one sent, and that each purged uuid answers 410;
4. writes <out>/<release>/: the data directory as the release left it
   (`data/`), the test master key (`master.key`) and `manifest.json` (every
   kept version with the SHA-256 of its body, the purged uuids, the tenants,
   the users, and what made the fixture).

With --postgres, for each release in PG_RELEASES (0.4.6 on) it writes
PG_OUT/<release>/ instead: the same binary with its catalogs in the
primary of scripts/pg-test-server.sh (PG_IMAGE) and its objects in a
RustFS container (RUSTFS_IMAGE) on that network; `migrate`, the same
workload, a restart, then Parquet storage on, whose pack run at start packs
every object, then LATE_VCONS more vCons per tenant, stored as objects. It
keeps `dump.sql` (pg_dump --inserts, without its \\restrict lines), every
bucket object by key under `bucket/`, the lock files, the master key and a
manifest that also names each version's place and the schema version.

The master key of each fixture is SHA-256 of a fixed text and the release,
so the key is published with the fixture: it protects test data only. Every
party is a reserved value (example.com, 555-01xx numbers).

The container and HTTP steps cannot run in a unit test (they need Docker
and a release binary); scripts/test_make_format_fixtures.py tests every
step that turns inputs into the plan, the configuration, the expected
versions and the manifest.
"""

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import ssl
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import uuid as uuidlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
PINS = ROOT / "scripts" / "docs-test-pins.txt"
FETCH = ROOT / "scripts" / "fetch-pinned.sh"
OUT = ROOT / "crates" / "vcon-server" / "tests" / "data" / "formats"

# Every release whose data directory this build must open. 0.3.0 and
# earlier are not here: 0.4.0 is the first release the rule covers.
RELEASES = ("0.4.0", "0.4.1", "0.4.2", "0.4.3", "0.4.4", "0.4.5", "0.4.6", "0.4.7", "0.4.8", "0.4.9",
            "0.4.10", "0.4.11", "0.4.12", "0.4.13", "0.4.14", "0.4.15", "0.4.16", "0.4.17",
            "0.4.18")

# debian:trixie-slim, as pulled 2026-11-04.
IMAGE = "debian@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2"

KEY_TEXT = b"vcon-server on-disk format fixture, test data only: "

# The port the server listens on in its container, and the address its
# published port is bound to on this host.
PORT = 8443
LOOPBACK = "127.0.0.1"

# Each step's own bound, in seconds.
DOCKER_TIMEOUT = 120
SERVE_START = 60


class FixtureError(Exception):
    """A step failed; the message says which."""


def pin_for(pins_text, release, arch):
    """The (sha256, url) of the release archive for `arch` in the pins."""
    name = f"/v{release}/vcon-server-{release}-{arch}-unknown-linux-gnu.tar.gz"
    found = []
    for line in pins_text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) == 2 and parts[1].endswith(name):
            found.append((parts[0], parts[1]))
    if len(found) != 1:
        raise FixtureError(f"{len(found)} pins name {name}, not one")
    sha, url = found[0]
    if len(sha) != 64 or any(c not in "0123456789abcdef" for c in sha):
        raise FixtureError(f"the pin of {name} is not a sha256: {sha}")
    return sha, url


def master_key(release):
    """The 32 bytes of the fixture's test master key."""
    return hashlib.sha256(KEY_TEXT + release.encode()).digest()


def config_text(listen):
    """The configuration every container reads, at /w/vcon-server.toml."""
    return (
        f'listen = "{listen}"\n'
        'data_dir = "/w/data"\n'
        'master_key = "/w/master.key"\n'
        "\n"
        "[tls]\n"
        'cert = "/w/cert.pem"\n'
        'key = "/w/key.pem"\n'
    )


def vcon_uuid(tenant, n):
    """A fixed uuid (version 4 layout) for vCon `n` of `tenant`."""
    digest = hashlib.sha256(f"format fixture {tenant} {n}".encode()).digest()
    return str(uuidlib.UUID(bytes=digest[:16], version=4))


def vcon_body(tenant, n, edition):
    """The bytes of edition `edition` of vCon `n` of `tenant`: reserved
    values only, one dialog line that names the edition."""
    doc = {
        "vcon": "0.4.0",
        "uuid": vcon_uuid(tenant, n),
        "created_at": f"2026-10-{1 + n:02d}T12:00:00Z",
        "subject": f"{tenant} call {n} edition {edition}",
        "parties": [
            {"tel": f"+1555555{100 + n:04d}", "name": "Caller Example"},
            {"mailto": f"agent{n}@example.com", "name": "Agent Example"},
        ],
        "dialog": [
            {
                "type": "text",
                "start": f"2026-10-{1 + n:02d}T12:00:01Z",
                "parties": [0, 1],
                "body": f"edition {edition} of call {n}",
                "encoding": "none",
                "mediatype": "text/plain",
            }
        ],
        "attachments": [],
        "analysis": [],
    }
    return json.dumps(doc, separators=(",", ":"), sort_keys=True).encode()


# The tenants: a name and its retention period. tenant-b's 0d makes every
# hidden version due at once, so its purges happen inside the workload.
TENANTS = (("tenant-a", None), ("tenant-b", "0d"))

# What the workload does to each tenant's ten vCons, in order. Every vCon
# is stored first. "replace" stores edition 1 over edition 0; "hold" sets a
# legal hold; "delete" deletes the uuid.
WORKLOAD = {
    "tenant-a": [
        ("replace", 0),  # edition 0 hidden, kept (7y)
        ("replace", 1),
        ("delete", 2),  # deleted, kept (7y)
        ("delete", 3),
        ("hold", 4),
        ("delete", 4),  # deleted and held
        ("hold", 5),  # live and held
    ],
    "tenant-b": [
        ("replace", 0),  # edition 0 hidden and due: purged by the sweep
        ("delete", 1),  # due at once: purged by the delete
        ("hold", 2),
        ("delete", 2),  # held, so kept though due
        ("hold", 3),
        ("replace", 3),  # held: edition 0 kept though hidden and due
    ],
}

VCONS_PER_TENANT = 10


def expected(workload=None):
    """What each tenant must keep once the workload and the sweep are done:
    {tenant: {"kept": {uuid: [(edition, state), ...]}, "purged": [uuid]}},
    oldest version first. A state is "current", "hidden" or "deleted" (the
    latest version of a deleted uuid), and "held" is added for a held uuid.
    """
    workload = WORKLOAD if workload is None else workload
    retention = dict(TENANTS)
    out = {}
    for tenant, _ in TENANTS:
        due = retention[tenant] == "0d"
        uuids = {}
        for n in range(VCONS_PER_TENANT):
            uuids[n] = {"versions": [0], "deleted": False, "held": False, "purged": False}
        for op, n in workload.get(tenant, []):
            v = uuids[n]
            if op == "replace":
                v["versions"].append(v["versions"][-1] + 1)
            elif op == "hold":
                v["held"] = True
            elif op == "delete":
                v["deleted"] = True
            else:
                raise FixtureError(f"unknown step {op}")
        kept, purged = {}, []
        for n, v in uuids.items():
            uuid = vcon_uuid(tenant, n)
            hidden = v["versions"][:-1]
            last = v["versions"][-1]
            if due and not v["held"]:
                hidden = []
                if v["deleted"]:
                    purged.append(uuid)
                    continue
            rows = [(e, "hidden") for e in hidden]
            rows.append((last, "deleted" if v["deleted"] else "current"))
            if v["held"]:
                rows = [(e, s + "+held") for e, s in rows]
            kept[uuid] = rows
        out[tenant] = {"kept": kept, "purged": sorted(purged)}
    return out


def manifest(release, archive_sha, binary_sha, kept_versions, purged, tenants, users, postgres=None):
    """The manifest.json text. `kept_versions` is a list of
    {"tenant", "uuid", "version", "state", "sha256", "bytes"} (and "place"
    in a PostgreSQL fixture); `postgres`, when given, is that fixture's own
    section."""
    doc = {
        "release": release,
        "archive_sha256": archive_sha,
        "binary_sha256": binary_sha,
        "image": IMAGE,
        "made_by": "scripts/make-format-fixtures.py",
        "master_key": "master.key, SHA-256 of a fixed text and the release: test data only",
        "tenants": tenants,
        "users": users,
        "versions": sorted(kept_versions, key=lambda r: (r["tenant"], r["uuid"], r["order"])),
        "purged": sorted(purged, key=lambda r: (r["tenant"], r["uuid"])),
    }
    if postgres is not None:
        doc["postgres"] = postgres
    return json.dumps(doc, indent=1, sort_keys=True) + "\n"


def has_users(help_text):
    """Whether the release's --help lists the `user` command (0.4.4 on)."""
    return any(line.split()[:1] == ["user"] for line in help_text.splitlines())


def tree_bytes(path):
    """The total size of the regular files under `path`."""
    return sum(p.stat().st_size for p in pathlib.Path(path).rglob("*") if p.is_file())


# ---- The PostgreSQL fixtures (--postgres). ----

# The releases whose binaries have the `postgres` feature: every release
# from 0.4.6 on (`strings` finds tokio-postgres in 0.4.6's binary and in
# none before it), so a release added to RELEASES is added here too.
PG_RELEASES = RELEASES[RELEASES.index("0.4.6"):]

# The images of CI's backends-postgres job: PostgreSQL 16.15, the oldest
# major CI runs, so its pg_dump loads into each newer one; RustFS 1.0.4.
PG_IMAGE = "postgres@sha256:f4610795587492cd4fb887c2dc3de301b813278bb6f616992061141b5f15ac36"
RUSTFS_IMAGE = "rustfs/rustfs@sha256:ed9210d6411d77ffceb70d94b964a3cb7d225983368bc25bebe01e995293dcc2"

PG_SERVER = ROOT / "scripts" / "pg-test-server.sh"
PG_OUT = ROOT / "crates" / "vcon-server" / "tests" / "data" / "formats-postgres"

# The bucket and the keys of the RustFS container, as CI sets them.
BUCKET = "vcon-formats"
S3_ACCESS_KEY = "vcontest"
S3_SECRET_KEY = "vcon-ci-only-secret"

# The pack run comes at start, then not again while the script stores the
# late vCons and stops the server, so those stay objects.
PACK_INTERVAL = 3600

# vCons stored per tenant after the pack run, numbered on from the workload's.
LATE_VCONS = 2

LOGIN_ROLES = ("vcon_app", "vcon_migrate")


def pg_config_text(listen, user, parquet):
    """The configuration of a PostgreSQL fixture: the base one, the objects
    in the RustFS bucket and the catalogs in the primary of
    scripts/pg-test-server.sh, reached by its certificate's name. `user` is
    the login role, vcon_app for the server and the commands, vcon_migrate
    for `migrate`; `parquet` adds the [storage.parquet] table."""
    if user not in LOGIN_ROLES:
        raise FixtureError(f"no role {user} logs in: one of {', '.join(LOGIN_ROLES)}")
    text = config_text(listen) + (
        "\n[storage]\n"
        'objects = "s3"\n'
        "\n[storage.s3]\n"
        'endpoint = "http://rustfs:9000"\n'
        'region = "us-east-1"\n'
        f'bucket = "{BUCKET}"\n'
        "path_style = true\n"
        "allow_http = true\n"
    )
    if parquet:
        text += f"\n[storage.parquet]\npack_interval = {PACK_INTERVAL}\n"
    return text + (
        "\n[catalog]\n"
        'backend = "postgres"\n'
        "\n[catalog.postgres]\n"
        'hosts = ["primary"]\n'
        "port = 5432\n"
        'dbname = "vcon"\n'
        f'user = "{user}"\n'
        'ca_file = "/w/pg-ca.pem"\n'
        'password_file = "/w/pg.password"\n'
    )


def late_numbers():
    """The numbers of the vCons stored after the pack run."""
    return range(VCONS_PER_TENANT, VCONS_PER_TENANT + LATE_VCONS)


def place(n):
    """Where vCon `n`'s versions are kept in a PostgreSQL fixture: the
    workload's were packed into Parquet files by the pack run, the late
    ones stay objects."""
    if 0 <= n < VCONS_PER_TENANT:
        return "packed"
    if n in late_numbers():
        return "object"
    raise FixtureError(f"no vCon {n}")


def strip_dump(text):
    """`pg_dump`'s plain text without its `\\restrict` and `\\unrestrict`
    lines (pg_dump 16.10 and later write them for psql), so that the test
    can send it to the server as one simple query. Any other line that
    starts with a backslash, a psql meta-command or COPY data, is refused:
    the server would not take it."""
    out, restricts = [], 0
    for number, line in enumerate(text.splitlines(keepends=True), 1):
        bare = line.rstrip("\n")
        if not bare.lstrip().startswith("\\"):
            out.append(line)
            continue
        words = bare.split(" ")
        if len(words) == 2 and words[0] in ("\\restrict", "\\unrestrict") and words[1].isalnum():
            restricts += 1
            continue
        if bare == "\\.":
            raise FixtureError(f"line {number}: COPY data; dump with --inserts")
        raise FixtureError(f"line {number}: {bare[:60]} is a psql meta-command")
    if restricts == 0:
        raise FixtureError("the dump has no \\restrict line: it is not pg_dump 16.10 or later's")
    return "".join(out)


def listed_keys(xml_text):
    """The keys of a ListObjectsV2 answer, in order. A truncated listing is
    refused: a fixture's bucket holds far fewer than one page."""
    import xml.etree.ElementTree as ET

    try:
        doc = ET.fromstring(xml_text)
    except ET.ParseError as e:
        raise FixtureError(f"not a bucket listing: {e}") from e
    local = lambda tag: tag.rsplit("}", 1)[-1]  # noqa: E731
    if local(doc.tag) != "ListBucketResult":
        raise FixtureError(f"not a bucket listing: {local(doc.tag)}")
    keys, truncated = [], None
    for child in doc:
        if local(child.tag) == "IsTruncated":
            truncated = child.text
        elif local(child.tag) == "Contents":
            keys.extend(k.text for k in child if local(k.tag) == "Key")
    if truncated != "false":
        raise FixtureError(f"the bucket listing is truncated ({truncated})")
    return keys


def bucket_path(key):
    """The path under the fixture's `bucket/` of the object `key`: only a
    key made of plain names separated by single slashes."""
    parts = key.split("/")
    if not key or any(p in ("", ".", "..") for p in parts):
        raise FixtureError(f"the key {key!r} is not a plain path")
    return pathlib.PurePosixPath(*parts)


# ---- The parts below need Docker and a release binary. ----


def run(args, timeout=DOCKER_TIMEOUT, check=True):
    out = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
    if check and out.returncode != 0:
        raise FixtureError(f"{args[:4]}... exited {out.returncode}: {out.stderr.strip()[-2000:]}")
    return out


class Release:
    def __init__(self, release, arch, cache, work):
        self.release = release
        self.work = pathlib.Path(work)
        sha, url = pin_for(PINS.read_text(), release, arch)
        archive = pathlib.Path(cache) / url.rsplit("/", 1)[1]
        run(["bash", str(FETCH), "--url", url, "--sha256", sha, "--dest", str(archive)], timeout=400)
        self.archive_sha = sha
        with tarfile.open(archive) as tar:
            member = next(m for m in tar.getmembers() if m.name.endswith("/vcon-server") and m.isfile())
            data = tar.extractfile(member).read()
        self.binary_sha = hashlib.sha256(data).hexdigest()
        (self.work / "bin").mkdir(parents=True)
        binary = self.work / "bin" / "vcon-server"
        binary.write_bytes(data)
        binary.chmod(0o755)
        (self.work / "data").mkdir(mode=0o700)
        key = self.work / "master.key"
        key.write_bytes(master_key(release))
        key.chmod(0o400)
        (self.work / "vcon-server.toml").write_text(config_text(f"0.0.0.0:{PORT}"))
        run(
            [
                "openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
                "-nodes", "-days", "2", "-subj", "/CN=localhost",
                "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1",
                "-addext", "basicConstraints=critical,CA:FALSE",
                "-addext", "keyUsage=critical,digitalSignature",
                "-addext", "extendedKeyUsage=serverAuth",
                "-keyout", str(self.work / "key.pem"), "-out", str(self.work / "cert.pem"),
            ]
        )
        (self.work / "key.pem").chmod(0o400)
        self.name = f"vcon-format-fixture-{release}-{os.getpid()}"
        self.containers = []
        # The commands run with no network and the server on a bridge,
        # unless a PostgreSQL deployment puts both on its network.
        self.cli_network, self.serve_network = "none", "bridge"
        self.env = []

    def docker(self, extra, args, network="none"):
        return [
            "docker", "run", "--rm", "--network", network, "--ulimit", "memlock=-1",
            "--user", f"{os.getuid()}:{os.getgid()}", "-v", f"{self.work}:/w", *self.env, *extra,
            IMAGE, "/w/bin/vcon-server", *args,
        ]

    def cli(self, *args, config="vcon-server.toml"):
        out = run(self.docker([], [*args, "--config", f"/w/{config}"], network=self.cli_network))
        return out.stdout

    def help(self):
        return run(self.docker([], ["--help"])).stdout

    def serve(self):
        name = f"{self.name}-{len(self.containers)}"
        self.containers.append(name)
        run(self.serve_args(name))
        port = run(["docker", "port", name, f"{PORT}/tcp"]).stdout.split()[0].rsplit(":", 1)[1]
        return Http(f"https://127.0.0.1:{port}", self.work / "cert.pem", name)

    def serve_args(self, name):
        # A published port needs a bridge network. The data directory
        # configuration names no S3 service and no PostgreSQL, so that
        # server connects to nothing; a PostgreSQL deployment's network
        # holds only its database servers and its RustFS.
        # Docker's HOST_IP::CONTAINER_PORT: the loopback address, a port Docker picks.
        publish = f"{LOOPBACK}::{PORT}"
        args = self.docker(["-d", "--name", name, "-p", publish], ["serve", "--config", "/w/vcon-server.toml"],
                           network=self.serve_network)
        args.remove("--rm")
        return args

    def stop(self, name):
        run(["docker", "stop", "--time", "30", name], timeout=60)
        logs = run(["docker", "logs", name], check=False)
        state = run(["docker", "inspect", "--format", "{{.State.ExitCode}}", name]).stdout.strip()
        run(["docker", "rm", name], check=False)
        self.containers.remove(name)
        if state != "0":
            raise FixtureError(f"serve exited {state}: {logs.stderr[-2000:]}")

    def cleanup(self):
        for name in list(self.containers):
            run(["docker", "rm", "-f", name], check=False)
        self.containers.clear()


class Http:
    def __init__(self, base, cert, name):
        self.base, self.name = base, name
        self.context = ssl.create_default_context(cafile=str(cert))
        deadline = time.monotonic() + SERVE_START
        while True:
            try:
                self.call("GET", "/health", None)
                return
            except (urllib.error.URLError, ConnectionError, ssl.SSLError, OSError):
                if time.monotonic() > deadline:
                    raise FixtureError(f"serve at {base} never answered /health")
                time.sleep(0.5)

    def call(self, method, path, token, body=None, headers=None, expect=None):
        request = urllib.request.Request(self.base + path, data=body, method=method)
        if token:
            request.add_header("Authorization", f"Bearer {token}")
        for k, v in (headers or {}).items():
            request.add_header(k, v)
        try:
            with urllib.request.urlopen(request, context=self.context, timeout=30) as answer:
                status, got, etag = answer.status, answer.read(), answer.headers.get("ETag")
        except urllib.error.HTTPError as e:
            status, got, etag = e.code, e.read(), e.headers.get("ETag")
        if expect is not None and status not in expect:
            raise FixtureError(f"{method} {path}: {status}, not {expect}: {got[:400]!r}")
        return status, got, (etag or "").strip('"')


def make(release, arch, cache, out_root):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f"fixture-{release}-", dir=cache))
    rel = Release(release, arch, cache, work)
    try:
        return build(rel, out_root)
    finally:
        rel.cleanup()
        shutil.rmtree(work)


def prepare(rel):
    """The tenants, their keys and the users, made by the commands while no
    server runs: (tenant ids, each tenant's producer token, the users)."""
    ids, keys = {}, {}
    for tenant, retention in TENANTS:
        args = ["tenant", "create", tenant] + (["--retention", retention] if retention else [])
        ids[tenant] = json.loads(rel.cli(*args))["id"]
        keys[tenant] = json.loads(
            rel.cli("key", "create", "--tenant", ids[tenant], "--label", f"{tenant}-producer",
                    *sum((["--scope", s] for s in ("write", "read", "delete", "legal", "audit")), []))
        )["token"]
        rel.cli("key", "create", "--tenant", ids[tenant], "--label", f"{tenant}-reader", "--scope", "read")
    rel.cli("key", "create", "--operator", "--label", "ops")
    users = []
    if has_users(rel.help()):
        a = json.loads(rel.cli("user", "add", "Reader Example"))["id"]
        b = json.loads(rel.cli("user", "add", "Records Example", "--issuer", "https://id.example.com",
                               "--subject", "user-0002"))["id"]
        rel.cli("membership", "grant", a, "--tenant", ids["tenant-a"], "--role", "reader")
        rel.cli("membership", "grant", b, "--tenant", ids["tenant-b"], "--role", "records")
        users = [{"label": "Reader Example", "roles": "reader in tenant-a"},
                 {"label": "Records Example", "roles": "records in tenant-b"}]
    return ids, keys, users


def store_workload(http, keys):
    """Stores each tenant's vCons and runs the WORKLOAD: what was sent,
    {tenant: {n: [(etag, body), ...]}}, oldest edition first."""
    sent = {}
    for tenant, _ in TENANTS:
        token = keys[tenant]
        editions = {}
        for n in range(VCONS_PER_TENANT):
            body = vcon_body(tenant, n, 0)
            _, _, etag = http.call("POST", "/vcons", token, body, {"Content-Type": "application/vcon"}, (201,))
            editions[n] = [(etag, body)]
        for op, n in WORKLOAD[tenant]:
            uuid = vcon_uuid(tenant, n)
            if op == "replace":
                body = vcon_body(tenant, n, len(editions[n]))
                _, _, etag = http.call("PUT", f"/vcons/{uuid}", token, body,
                                       {"Content-Type": "application/vcon", "If-Match": f'"{editions[n][-1][0]}"'}, (200,))
                editions[n].append((etag, body))
            elif op == "hold":
                http.call("PUT", f"/legal/vcons/{uuid}/hold", token, None, None, (204,))
            elif op == "delete":
                http.call("DELETE", f"/vcons/{uuid}", token, None, None, (204,))
        sent[tenant] = editions
    return sent


def read_back(http, keys, sent, want, placed=False):
    """Checks over the legal routes that each tenant kept exactly the
    versions `want` names, in order and byte-equal to what was sent, and
    that each purged uuid answers 410: (the manifest's kept rows, its
    purged rows). `placed` adds each row's place (a PostgreSQL fixture)."""
    kept, purged = [], []
    for tenant, _ in TENANTS:
        token = keys[tenant]
        for n in sorted(sent[tenant]):
            uuid = vcon_uuid(tenant, n)
            if uuid in want[tenant]["purged"]:
                http.call("GET", f"/legal/vcons/{uuid}/versions", token, None, None, (410,))
                purged.append({"tenant": tenant, "uuid": uuid})
                continue
            _, listed, _ = http.call("GET", f"/legal/vcons/{uuid}/versions", token, None, None, (200,))
            listed = json.loads(listed)
            rows = want[tenant]["kept"][uuid]
            if len(listed) != len(rows):
                raise FixtureError(f"{tenant} {uuid}: {len(listed)} versions kept, {len(rows)} expected")
            for order, ((edition, state), item) in enumerate(zip(rows, listed)):
                etag, body = sent[tenant][n][edition]
                if item["version"] != etag:
                    raise FixtureError(f"{tenant} {uuid}: version {item['version']}, sent {etag}")
                _, got, _ = http.call("GET", f"/legal/vcons/{uuid}/versions/{etag}", token, None, None, (200,))
                if got != body:
                    raise FixtureError(f"{tenant} {uuid} {etag}: the body read back differs")
                row = {"tenant": tenant, "uuid": uuid, "version": etag, "order": order,
                       "state": state, "sha256": hashlib.sha256(body).hexdigest(), "bytes": len(body)}
                if placed:
                    row["place"] = place(n)
                kept.append(row)
    return kept, purged


def build(rel, out_root):
    ids, keys, users = prepare(rel)
    http = rel.serve()
    sent = store_workload(http, keys)
    rel.stop(http.name)
    # The start-up sweep purges what is due.
    http = rel.serve()
    kept, purged = read_back(http, keys, sent, expected())
    rel.stop(http.name)

    dest = pathlib.Path(out_root) / rel.release
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    shutil.copytree(rel.work / "data", dest / "data")
    shutil.copyfile(rel.work / "master.key", dest / "master.key")
    tenants = [{"name": t, "retention": r or "default"} for t, r in TENANTS]
    (dest / "manifest.json").write_text(
        manifest(rel.release, rel.archive_sha, rel.binary_sha, kept, purged, tenants, users)
    )
    return tree_bytes(dest)


class Deployment:
    """The PostgreSQL servers of scripts/pg-test-server.sh and a RustFS on
    their network, for one release's fixture; `close` removes them."""

    def __init__(self, rel):
        self.rel = rel
        self.dir = rel.work / "pg"
        self.rustfs = None
        run(["bash", str(PG_SERVER), "start", PG_IMAGE, str(self.dir)], timeout=300)
        self.net = (self.dir / "network").read_text().strip()
        self.primary = (self.dir / "containers").read_text().split()[0]
        shutil.copyfile(self.dir / "ca.pem", rel.work / "pg-ca.pem")
        shutil.copyfile(self.dir / "password", rel.work / "pg.password")
        (rel.work / "pg.password").chmod(0o600)
        name = f"{rel.name}-rustfs"
        # Published on the loopback address, at a free host port docker
        # picks (the empty middle part), to the container's port 9000.
        publish = ":".join((LOOPBACK, "", "9000"))
        run(["docker", "run", "-d", "--name", name, "--network", self.net, "--network-alias", "rustfs",
             "-p", publish, "-e", f"RUSTFS_ACCESS_KEY={S3_ACCESS_KEY}",
             "-e", f"RUSTFS_SECRET_KEY={S3_SECRET_KEY}", RUSTFS_IMAGE])
        self.rustfs = name
        port = run(["docker", "port", name, "9000/tcp"]).stdout.split()[0].rsplit(":", 1)[1]
        self.s3 = f"http://{LOOPBACK}:{port}"
        deadline = time.monotonic() + SERVE_START
        while self.curl("GET", "", check=False).returncode != 0:
            if time.monotonic() > deadline:
                raise FixtureError("RustFS never answered a signed request")
            time.sleep(0.5)
        self.curl("PUT", BUCKET)
        rel.cli_network = rel.serve_network = self.net
        rel.env = ["-e", f"AWS_ACCESS_KEY_ID={S3_ACCESS_KEY}", "-e", f"AWS_SECRET_ACCESS_KEY={S3_SECRET_KEY}",
                   "-e", "AWS_EC2_METADATA_DISABLED=true"]

    def curl(self, method, path, out=None, check=True):
        args = ["curl", "-sS", "--fail-with-body", "-X", method, "--aws-sigv4", "aws:amz:us-east-1:s3",
                "--user", f"{S3_ACCESS_KEY}:{S3_SECRET_KEY}", f"{self.s3}/{path}"]
        if out is not None:
            args[1:1] = ["-o", str(out)]
        return run(args, timeout=60, check=check)

    def keys(self):
        return listed_keys(self.curl("GET", f"{BUCKET}?list-type=2").stdout)

    def psql(self, sql):
        return run(["docker", "exec", self.primary, "psql", "-U", "postgres", "-d", "vcon", "-At",
                    "-v", "ON_ERROR_STOP=1", "-c", sql]).stdout.strip()

    def dump(self):
        # As the superuser, which row-level security does not filter.
        out = run(["docker", "exec", self.primary, "pg_dump", "-U", "postgres", "--inserts", "vcon"], timeout=120)
        return strip_dump(out.stdout)

    def close(self):
        if self.rustfs:
            run(["docker", "rm", "-f", self.rustfs], check=False)
        if (self.dir / "containers").exists():
            run(["bash", str(PG_SERVER), "stop", str(self.dir)], timeout=120, check=False)


def objects_left(keys):
    """The keys of a bucket listing that are versions' own objects."""
    return [k for k in keys if k.startswith("tenants/") and "/objects/" in k]


def make_pg(release, arch, cache, out_root):
    work = pathlib.Path(tempfile.mkdtemp(prefix=f"fixture-pg-{release}-", dir=cache))
    rel = Release(release, arch, cache, work)
    deployment = None
    try:
        deployment = Deployment(rel)
        return build_pg(rel, deployment, out_root)
    finally:
        rel.cleanup()
        if deployment is not None:
            deployment.close()
        shutil.rmtree(work)


def build_pg(rel, dep, out_root):
    listen = f"0.0.0.0:{PORT}"
    (rel.work / "vcon-migrate.toml").write_text(pg_config_text(listen, "vcon_migrate", False))
    (rel.work / "vcon-server.toml").write_text(pg_config_text(listen, "vcon_app", False))
    rel.cli("migrate", config="vcon-migrate.toml")
    _, keys, users = prepare(rel)
    http = rel.serve()
    sent = store_workload(http, keys)
    rel.stop(http.name)
    # The start-up sweep purges what is due.
    http = rel.serve()
    read_back(http, keys, sent, expected())
    rel.stop(http.name)

    # Parquet storage on: the pack run at start packs every object.
    (rel.work / "vcon-server.toml").write_text(pg_config_text(listen, "vcon_app", True))
    http = rel.serve()
    deadline = time.monotonic() + SERVE_START
    while objects_left(dep.keys()):
        if time.monotonic() > deadline:
            raise FixtureError(f"the pack run left {len(objects_left(dep.keys()))} objects")
        time.sleep(1)
    want = expected()
    for tenant, _ in TENANTS:
        for n in late_numbers():
            body = vcon_body(tenant, n, 0)
            _, _, etag = http.call("POST", "/vcons", keys[tenant], body, {"Content-Type": "application/vcon"}, (201,))
            sent[tenant][n] = [(etag, body)]
            want[tenant]["kept"][vcon_uuid(tenant, n)] = [(0, "current")]
    kept, purged = read_back(http, keys, sent, want, placed=True)
    rel.stop(http.name)
    late = sum(1 for r in kept if r["place"] == "object")
    if len(objects_left(dep.keys())) != late:
        raise FixtureError(f"{len(objects_left(dep.keys()))} objects in the bucket, {late} late versions")

    schema = int(dep.psql("SELECT max(version) FROM vcon.schema_version"))
    dest = pathlib.Path(out_root) / rel.release
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    (dest / "dump.sql").write_text(dep.dump())
    for key in dep.keys():
        path = dest / "bucket" / bucket_path(key)
        path.parent.mkdir(parents=True, exist_ok=True)
        dep.curl("GET", f"{BUCKET}/{key}", out=path)
    shutil.copytree(rel.work / "data", dest / "data")
    shutil.copyfile(rel.work / "master.key", dest / "master.key")
    tenants = [{"name": t, "retention": r or "default"} for t, r in TENANTS]
    section = {"schema": schema, "dump": "dump.sql", "bucket": "bucket", "image": PG_IMAGE,
               "rustfs": RUSTFS_IMAGE, "pack_interval": PACK_INTERVAL}
    (dest / "manifest.json").write_text(
        manifest(rel.release, rel.archive_sha, rel.binary_sha, kept, purged, tenants, users, postgres=section)
    )
    return tree_bytes(dest)


def cache_dir(path):
    """The cache directory `path`, made with its parents when missing; a
    file there is refused by name. Both modes make their work directories
    in it, so a first run with a new path must not fail on it."""
    cache = pathlib.Path(path)
    if cache.exists() and not cache.is_dir():
        raise SystemExit(f"make-format-fixtures: --cache {path}: not a directory")
    cache.mkdir(parents=True, exist_ok=True)
    return cache


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--cache", required=True, help="where the release archives are kept")
    parser.add_argument("--release", action="append", choices=RELEASES)
    parser.add_argument("--postgres", action="store_true",
                        help="the PostgreSQL deployments of the releases in PG_RELEASES")
    parser.add_argument("--out")
    parser.add_argument("--arch", default=os.uname().machine)
    args = parser.parse_args(argv)
    releases = args.release or (PG_RELEASES if args.postgres else RELEASES)
    if args.postgres and not set(releases) <= set(PG_RELEASES):
        parser.error(f"--postgres takes only {', '.join(PG_RELEASES)}")
    out = args.out or str(PG_OUT if args.postgres else OUT)
    args.cache = str(cache_dir(args.cache))
    total = 0
    for release in releases:
        size = (make_pg if args.postgres else make)(release, args.arch, args.cache, out)
        total += size
        print(f"{release}: {size} bytes")
    print(f"total: {total} bytes")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except FixtureError as e:
        print(f"make-format-fixtures: {e}", file=sys.stderr)
        sys.exit(1)
