"""Kills a server with SIGKILL again and again under writers and readers of
one tenant, and checks after each kill that nothing it answered was lost.

Each round starts `vcon-server serve` on a data directory this program
made, runs writers (each stores vCons of its own with POST /vcons and
replaces them with PUT /vcons/{uuid}, naming the version the last answer
gave) and readers (GET /vcons/{uuid} of vCons already answered), and kills
the server's process group with SIGKILL at a random moment. With the
server down it then runs `vcon-server check` (exit 0 required) and
`vcon-server audit export --tenant`, and the next round's server, which
runs start-up recovery, answers a GET of the vCons written. The checks:

- every write the server answered is there: the vCon's current body is the
  last answered one, or one a writer sent after it and saw no answer to
  (that request may have committed before the kill); once seen, a body
  never goes back to an older one;
- every version that is there has its audit record (`create` or
  `replace`, its uuid, its version, status 201 or 200), and every answered
  read has a `read` record: per uuid, the read records' counts sum to at
  least the reads answered;
- `check` reports no problem.

A kill keeps what reached the kernel, so this tests the process-crash
case only. A power loss cannot be tested on this host: the page cache
survives a killed process, and nothing here can drop it the way a power
loss does. The argument for the power-loss case is in
website/content/docs/durability.md.

  python3 -I scripts/crash_run.py --server BIN --kills N --out FILE
      [--parent DIR] [--seed S] [--writers 2] [--readers 2] [--longest 1.5]

Writes a JSON summary to FILE; exits 1 on any violation, 2 on a usage or
set-up error. Every file it made is deleted whatever the outcome.
"""

import argparse
import hashlib
import http.client
import importlib.util
import json
import os
import pathlib
import random
import re
import shutil
import signal
import ssl
import subprocess
import sys
import tempfile
import threading
import time

HERE = pathlib.Path(__file__).resolve().parent

# How long a server may take to say where it listens.
START_PATIENCE = 30.0
# How long a command (check, audit export, tenant create) may take.
COMMAND_PATIENCE = 120.0
# How long a client may take to end after the kill.
CLIENT_PATIENCE = 60.0


def load_writers():
    """scripts/contend_writes.py, for its vCon body and TLS connection."""
    spec = importlib.util.spec_from_file_location("contend_writes", HERE / "contend_writes.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest(data):
    return hashlib.sha256(data).hexdigest()


def parse_port(log):
    """The port in the server's `serving listen=127.0.0.1:PORT` line, or None."""
    found = re.search(r" serving listen=127\.0\.0\.1:(\d+)$", log, re.MULTILINE)
    return int(found.group(1)) if found else None


def version_of(tag):
    """The version id in an ETag (`"<id>"`), or None."""
    if not tag or len(tag) < 3 or tag[0] != '"' or tag[-1] != '"':
        return None
    return tag[1:-1]


class Ledger:
    """What the clients sent and what the server answered, per uuid.

    For each uuid: `base`, the body digest and version last answered or
    seen (None before any); `pending`, the digests sent since `base` with
    no answer; `versions`, every version id answered or seen, with its
    action. Reads: answered reads per uuid.
    """

    def __init__(self):
        self.lock = threading.Lock()
        self.base = {}
        self.pending = {}
        self.versions = {}
        self.reads = {}
        # The uuids written to since the last `take_touched`.
        self.touched = set()

    def sent(self, uuid, body_digest):
        with self.lock:
            self.pending.setdefault(uuid, []).append(body_digest)
            self.touched.add(uuid)

    def take_touched(self):
        with self.lock:
            touched, self.touched = self.touched, set()
            return touched

    def answered(self, uuid, body_digest, version, action):
        with self.lock:
            self.base[uuid] = (body_digest, version)
            self.pending[uuid] = []
            self.versions.setdefault(uuid, {})[version] = action

    def read(self, uuid):
        with self.lock:
            self.reads[uuid] = self.reads.get(uuid, 0) + 1

    def answered_uuids(self):
        with self.lock:
            return sorted(self.base)

    def uuids(self):
        with self.lock:
            return sorted(set(self.base) | set(self.pending))


def judge_current(ledger, uuid, status, body_digest, version):
    """Checks one GET after a kill against the ledger and, when it passes,
    takes what it saw as the uuid's new base (a write in flight at the kill
    that committed). Answers a violation's text, or None."""
    base = ledger.base.get(uuid)
    pending = ledger.pending.get(uuid, [])
    if status == 404:
        if base is None:
            return None
        return f"{uuid}: answered write lost (404; last answered version {base[1]})"
    if status != 200:
        return f"{uuid}: GET answered {status}"
    if base is not None and body_digest == base[0] and version == base[1]:
        return None
    if body_digest in pending:
        action = "create" if base is None else "replace"
        ledger.answered(uuid, body_digest, version, action)
        return None
    if base is None:
        return f"{uuid}: a body no client sent (version {version})"
    return (
        f"{uuid}: version {version} is neither the last answered ({base[1]}) "
        "nor one sent after it"
    )


def judge_audit(ledger, records):
    """Checks the tenant's audit records against every version answered or
    seen, and the read records against the reads answered. Answers the
    violations' texts."""
    writes = set()
    reads = {}
    for r in records:
        action, target, status = r.get("action"), r.get("target"), r.get("status")
        if action in ("create", "replace") and status in (200, 201):
            writes.add((target, r.get("version"), action))
        elif action == "read" and status == 200:
            reads[target] = reads.get(target, 0) + int(r.get("count") or 1)
    problems = []
    for uuid, versions in sorted(ledger.versions.items()):
        for version, action in sorted(versions.items()):
            if (uuid, version, action) not in writes:
                problems.append(f"{uuid}: no {action} record for version {version}")
    for uuid, answered in sorted(ledger.reads.items()):
        if reads.get(uuid, 0) < answered:
            problems.append(
                f"{uuid}: {answered} reads answered, {reads.get(uuid, 0)} read records"
            )
    return problems


def judge_check(rc, output):
    """`vcon-server check`'s verdict: None when it exited 0 and its summary
    says no problem, else why not."""
    lines = [line for line in output.splitlines() if line.strip()]
    summary = None
    for line in lines:
        try:
            parsed = json.loads(line)
        except ValueError:
            continue
        if isinstance(parsed, dict) and "problems" in parsed and parsed.get("name") is None:
            summary = parsed
    if rc != 0:
        return f"check exited {rc}: {lines[-3:]}"
    if summary is None or summary.get("problems") != 0:
        return f"check reported {summary}"
    return None


class Writer(threading.Thread):
    """Stores vCons of its own and replaces them until `stop` is set or a
    request fails (the kill)."""

    def __init__(self, cw, send, token, ledger, rng, stop, own, ids):
        super().__init__(daemon=True)
        self.cw, self.send, self.ledger, self.rng, self.stop = cw, send, ledger, rng, stop
        self.own = own
        self.ids = ids
        self.headers = {"Authorization": "Bearer " + token, "Content-Type": "application/vcon"}
        self.answered = 0

    def run(self):
        while not self.stop.is_set():
            if not self.own or self.rng.random() < 0.3:
                uuid = self.ids()
                method, path, want, headers = "POST", "/vcons", 201, dict(self.headers)
                action = "create"
            else:
                uuid = self.rng.choice(sorted(self.own))
                method, path, want = "PUT", "/vcons/" + uuid, 200
                headers = dict(self.headers, **{"If-Match": '"' + self.own[uuid] + '"'})
                action = "replace"
            data = self.cw.body(uuid, self.rng.randrange(1 << 30))
            self.ledger.sent(uuid, digest(data))
            try:
                status, tag = self.send(method, path, data, headers)
            except (OSError, http.client.HTTPException):
                return
            version = version_of(tag)
            if status != want or version is None:
                return
            self.ledger.answered(uuid, digest(data), version, action)
            self.own[uuid] = version
            self.answered += 1


class Reader(threading.Thread):
    """Reads answered vCons until `stop` is set or a request fails."""

    def __init__(self, get, ledger, rng, stop):
        super().__init__(daemon=True)
        self.get, self.ledger, self.rng, self.stop = get, ledger, rng, stop
        self.answered = 0

    def run(self):
        while not self.stop.is_set():
            known = self.ledger.answered_uuids()
            if not known:
                time.sleep(0.01)
                continue
            uuid = self.rng.choice(known)
            try:
                status, _, _ = self.get(uuid)
            except (OSError, http.client.HTTPException):
                return
            if status == 200:
                self.ledger.read(uuid)
                self.answered += 1


def connect_get(port, ca, token):
    """A GET of /vcons/{uuid} over one kept TLS connection: (status, body
    digest, version)."""
    context = ssl.create_default_context(cafile=ca)
    conn = http.client.HTTPSConnection("localhost", port, context=context, timeout=30)

    def get(uuid):
        conn.request("GET", "/vcons/" + uuid, headers={"Authorization": "Bearer " + token})
        answer = conn.getresponse()
        data = answer.read()
        return answer.status, digest(data), version_of(answer.getheader("ETag"))

    return get


def run_command(args):
    done = subprocess.run(
        args, capture_output=True, text=True, timeout=COMMAND_PATIENCE, check=False
    )
    return done.returncode, done.stdout, done.stderr


class Server:
    """`serve` in a process group of its own, so the kill ends all of it."""

    def __init__(self, binary, config, log):
        self.binary, self.config, self.log = binary, config, log
        self.proc = None

    def start(self):
        with open(self.log, "w", encoding="utf-8") as out:
            self.proc = subprocess.Popen(
                [self.binary, "serve", "--config", self.config],
                stdout=subprocess.DEVNULL, stderr=out, start_new_session=True,
            )
        deadline = time.monotonic() + START_PATIENCE
        while time.monotonic() < deadline:
            port = parse_port(pathlib.Path(self.log).read_text(errors="replace"))
            if port:
                return port
            if self.proc.poll() is not None:
                raise RuntimeError("the server stopped: " + pathlib.Path(self.log).read_text()[-2000:])
            time.sleep(0.05)
        raise RuntimeError(f"the server did not say where it listens within {START_PATIENCE} s")

    def kill(self):
        if self.proc is not None and self.proc.poll() is None:
            os.killpg(self.proc.pid, signal.SIGKILL)
        if self.proc is not None:
            self.proc.wait(timeout=COMMAND_PATIENCE)
        self.proc = None

    def stop(self):
        if self.proc is not None and self.proc.poll() is None:
            os.killpg(self.proc.pid, signal.SIGTERM)
            try:
                self.proc.wait(timeout=COMMAND_PATIENCE)
            except subprocess.TimeoutExpired:
                self.kill()
        self.proc = None


def make_work(work, binary):
    """The data directory, master key, certificates, configuration, tenant
    and key: (config, ca, tenant, token)."""
    (work / "data").mkdir()
    (work / "master.key").write_bytes(os.urandom(32))
    os.chmod(work / "master.key", 0o400)
    openssl = [
        ["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
         "-nodes", "-days", "1", "-subj", "/CN=vcon-server crash test CA",
         "-addext", "basicConstraints=critical,CA:TRUE",
         "-addext", "keyUsage=critical,keyCertSign,cRLSign",
         "-keyout", str(work / "ca.key"), "-out", str(work / "ca.pem")],
        ["openssl", "req", "-new", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256",
         "-nodes", "-subj", "/CN=localhost", "-keyout", str(work / "key.pem"),
         "-out", str(work / "server.csr")],
    ]
    for args in openssl:
        subprocess.run(args, capture_output=True, check=True, timeout=COMMAND_PATIENCE)
    (work / "server.ext").write_text(
        "subjectAltName=DNS:localhost\nbasicConstraints=critical,CA:FALSE\n"
        "keyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n"
    )
    subprocess.run(
        ["openssl", "x509", "-req", "-in", str(work / "server.csr"), "-CA", str(work / "ca.pem"),
         "-CAkey", str(work / "ca.key"), "-days", "1", "-extfile", str(work / "server.ext"),
         "-out", str(work / "cert.pem")],
        capture_output=True, check=True, timeout=COMMAND_PATIENCE,
    )
    os.chmod(work / "key.pem", 0o600)
    config = work / "vcon-server.toml"
    config.write_text(
        f'listen = "127.0.0.1:0"\ndata_dir = "{work / "data"}"\n'
        f'master_key = "{work / "master.key"}"\n\n[tls]\n'
        f'cert = "{work / "cert.pem"}"\nkey = "{work / "key.pem"}"\n'
    )
    rc, out, err = run_command([binary, "tenant", "create", "crash", "--config", str(config)])
    if rc != 0:
        raise RuntimeError("tenant create: " + err)
    tenant = json.loads(out)["id"]
    rc, out, err = run_command(
        [binary, "key", "create", "--tenant", tenant, "--scope", "write", "--scope", "read",
         "--label", "crash", "--config", str(config)])
    if rc != 0:
        raise RuntimeError("key create: " + err)
    return str(config), str(work / "ca.pem"), tenant, json.loads(out)["token"]


def verify_live(ledger, get, uuids):
    """GETs each of `uuids` from the server just started; answers the
    violations. Each 200 is an answered read too."""
    problems = []
    for uuid in uuids:
        status, body_digest, version = get(uuid)
        problem = judge_current(ledger, uuid, status, body_digest, version)
        if problem:
            problems.append(problem)
        if status == 200:
            ledger.read(uuid)
    return problems


def parse(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--server", required=True)
    p.add_argument("--kills", type=int, required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--parent", default=None)
    p.add_argument("--seed", type=int, default=None)
    p.add_argument("--writers", type=int, default=2)
    p.add_argument("--readers", type=int, default=2)
    p.add_argument("--longest", type=float, default=1.5,
                   help="the longest a round runs before its kill, in seconds")
    args = p.parse_args(argv)
    if args.kills < 1 or args.writers < 1 or args.readers < 0 or args.longest <= 0:
        p.error("--kills and --writers must be at least 1, --readers at least 0, --longest above 0")
    return args


def stop_on_signal(signum, _frame):
    """A TERM or INT ends the run through SystemExit, so `main`'s `finally`
    kills the server's process group and deletes the files: the server
    runs in a session of its own, which the signal does not reach."""
    raise SystemExit(128 + signum)


def main(argv):
    signal.signal(signal.SIGTERM, stop_on_signal)
    signal.signal(signal.SIGINT, stop_on_signal)
    args = parse(argv)
    seed = args.seed if args.seed is not None else random.SystemRandom().randrange(1 << 32)
    rng = random.Random(seed)
    cw = load_writers()
    binary = os.path.realpath(args.server)
    work = pathlib.Path(tempfile.mkdtemp(prefix="vcon-crash.", dir=args.parent))
    server = None
    ledger = Ledger()
    problems = []
    totals = {"writes": 0, "reads": 0, "kills": 0}
    started = time.monotonic()
    try:
        config, ca, tenant, token = make_work(work, binary)
        server = Server(binary, config, str(work / "server.log"))
        serial = iter(range(1 << 40))

        def ids():
            n = next(serial)
            return f"01a07da8-c2bb-83e5-b9a2-{seed % (1 << 16):04x}{n:08x}"

        own = [dict() for _ in range(args.writers)]
        for round_no in range(args.kills + 1):
            port = server.start()
            get = connect_get(port, ca, token)
            # After a kill: every vCon written last round, and a sample of
            # the older ones.
            every = ledger.uuids()
            sample = every if round_no == args.kills else rng.sample(every, min(len(every), 40))
            for p in verify_live(ledger, get, sorted(set(sample) | ledger.take_touched())):
                problems.append(f"round {round_no}: {p}")
            # A writer goes on from what the server now has.
            for mine in own:
                for uuid in list(mine):
                    base = ledger.base.get(uuid)
                    if base is None:
                        del mine[uuid]
                    else:
                        mine[uuid] = base[1]
            if round_no == args.kills:
                server.stop()
                break
            stop = threading.Event()
            clients = [
                Writer(cw, cw.connect(port, ca), token, ledger, random.Random(rng.random()),
                       stop, own[i], ids)
                for i in range(args.writers)
            ] + [
                Reader(connect_get(port, ca, token), ledger, random.Random(rng.random()), stop)
                for _ in range(args.readers)
            ]
            for c in clients:
                c.start()
            time.sleep(rng.uniform(0.05, args.longest))
            server.kill()
            stop.set()
            for c in clients:
                c.join(CLIENT_PATIENCE)
                if c.is_alive():
                    raise RuntimeError(f"a client did not end within {CLIENT_PATIENCE} s")
                totals["writes" if isinstance(c, Writer) else "reads"] += c.answered
            totals["kills"] += 1
            rc, out, err = run_command([binary, "check", "--config", config])
            verdict = judge_check(rc, out + err)
            if verdict:
                problems.append(f"round {round_no}: {verdict}")
        rc, out, err = run_command(
            [binary, "audit", "export", "--tenant", tenant, "--config", config])
        if rc != 0:
            problems.append(f"audit export exited {rc}: {err[-500:]}")
        else:
            records = [json.loads(line) for line in out.splitlines() if line.strip()]
            problems.extend(judge_audit(ledger, records))
    except (RuntimeError, subprocess.SubprocessError, OSError) as e:
        print(f"crash_run: {e}", file=sys.stderr)
        return 2
    finally:
        if server is not None:
            server.kill()
        shutil.rmtree(work, ignore_errors=True)
    summary = {
        "seed": seed,
        "kills": totals["kills"],
        "writes_answered": totals["writes"],
        "reads_answered": totals["reads"],
        "vcons": len(ledger.uuids()),
        "versions_checked": sum(len(v) for v in ledger.versions.values()),
        "seconds": round(time.monotonic() - started, 1),
        "problems": problems,
    }
    pathlib.Path(args.out).write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({k: v for k, v in summary.items() if k != "problems"}))
    for p in problems[:20]:
        print("PROBLEM " + p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
