"""Writers that replace vCons of a tenant while vcon-scale reads from it.

scripts/scale-run.sh runs this when SCALE_WRITERS is set, so a run measures
reads while other clients of the same tenant write. Each of the N writers
stores one vCon of its own, then replaces it again and again until the
stop file exists, each replacement naming (If-Match) the version the write
before it returned. When every writer has stopped it writes a JSON summary:
writes, writes a second, write latency p50 and p99, and the error of each
writer that a refused or failed request ended.

  python3 -I scripts/contend_writes.py --port PORT --ca CA --key-file KEY
      --stop FILE --out FILE [--writers N]

Exits 1 when any writer ended on an error, 0 otherwise.
"""

import argparse
import collections
import http.client
import json
import os
import ssl
import sys
import threading
import time
import uuid as uuidlib

# The writes one writer made, each one's latency in milliseconds, and the
# error that ended it (None when the stop file did).
Done = collections.namedtuple("Done", "writes latencies error")

CREATED_AT = "2026-10-25T12:00:00Z"


def body(uuid, n):
    """The JSON text of write `n` of the vCon `uuid`: a new subject each time."""
    return json.dumps(
        {
            "created_at": CREATED_AT,
            "dialog": [
                {
                    "body": "contention harness write",
                    "encoding": "none",
                    "mediatype": "text/plain",
                    "parties": [0, 1],
                    "start": CREATED_AT,
                    "type": "text",
                }
            ],
            "parties": [
                {"mailto": "writer00@example.com", "tel": "+1-555-0100"},
                {"mailto": "writer01@example.com", "tel": "+1-555-0101"},
            ],
            "subject": f"contention harness write {n}",
            "uuid": uuid,
        },
        sort_keys=True,
    ).encode()


def write_loop(send, uuid, stop, clock, token):
    """Stores `uuid`, then replaces it until `stop()` answers True.

    `send(method, path, body, headers)` answers (status, ETag or None);
    `clock()` reads seconds. The first refused or failed request ends the
    loop, and its Done carries why.
    """
    headers = {"Authorization": "Bearer " + token, "Content-Type": "application/vcon"}
    writes, latencies, n = 0, [], 0
    request = ("POST", "/vcons", 201)
    tag = None
    while True:
        method, path, want = request
        sent = dict(headers)
        if tag is not None:
            sent["If-Match"] = tag
        started = clock()
        try:
            status, tag = send(method, path, body(uuid, n), sent)
        except OSError as e:
            return Done(writes, latencies, f"{method} {path}: {e}")
        ended = clock()
        if status != want:
            return Done(writes, latencies, f"{method} {path}: {status}, not {want}")
        if not tag:
            return Done(writes, latencies, f"{method} {path}: {status} without an ETag")
        writes += 1
        n += 1
        latencies.append((ended - started) * 1000.0)
        if stop():
            return Done(writes, latencies, None)
        request = ("PUT", "/vcons/" + uuid, 200)


def percentile(values, p):
    """The nearest-rank `p`th percentile of `values`, or None of none."""
    if not values:
        return None
    ranked = sorted(values)
    rank = max(1, -(-len(ranked) * p // 100))
    return ranked[int(rank) - 1]


def summary(done, seconds):
    """The JSON summary of the writers' Done values over `seconds`."""
    writes = sum(d.writes for d in done)
    latencies = [ms for d in done for ms in d.latencies]
    return {
        "writers": len(done),
        "writes": writes,
        "seconds": seconds,
        "writes_per_s": writes / seconds if seconds > 0 else None,
        "p50_ms": percentile(latencies, 50),
        "p99_ms": percentile(latencies, 99),
        "errors": [d.error for d in done if d.error is not None],
    }


def stop_when_exists(path):
    """A stop test that answers True once `path` exists."""
    return lambda: os.path.exists(path)


def connect(port, ca):
    """A send function over one kept TLS connection to localhost:`port`."""
    context = ssl.create_default_context(cafile=ca)
    conn = http.client.HTTPSConnection("localhost", port, context=context, timeout=60)

    def send(method, path, data, headers):
        conn.request(method, path, body=data, headers=headers)
        answer = conn.getresponse()
        answer.read()
        return answer.status, answer.getheader("ETag")

    return send


def parse(argv):
    parser = argparse.ArgumentParser(description="Writers that replace vCons until a stop file exists.")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--ca", required=True)
    parser.add_argument("--key-file", required=True)
    parser.add_argument("--stop", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--writers", type=int, default=2)
    args = parser.parse_args(argv)
    if args.writers < 1:
        parser.error("--writers must be at least 1")
    return args


def main(argv):
    args = parse(argv)
    with open(args.key_file, encoding="utf-8") as f:
        token = f.read().strip()
    stop = stop_when_exists(args.stop)
    done = [None] * args.writers

    def run(i):
        try:
            send = connect(args.port, args.ca)
        except (OSError, ssl.SSLError) as e:
            done[i] = Done(0, [], f"connect: {e}")
            return
        done[i] = write_loop(send, str(uuidlib.uuid4()), stop, time.monotonic, token)

    started = time.monotonic()
    threads = [threading.Thread(target=run, args=(i,)) for i in range(args.writers)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    result = summary(done, time.monotonic() - started)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(result, f, sort_keys=True)
        f.write("\n")
    for error in result["errors"]:
        print(f"contend_writes: {error}", file=sys.stderr)
    return 1 if result["errors"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
