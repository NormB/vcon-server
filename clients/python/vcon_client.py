#!/usr/bin/env python3
"""Stores a vCon, finds it in the list, reads it back, and deletes it, with
the Python standard library only.

Set VCON_URL (https://vcon.example.com), VCON_KEY (a key with the write,
read and delete scopes) and, for a certificate your system does not trust,
VCON_CA (its PEM file).
"""

import json
import os
import ssl
import sys
import urllib.error
import urllib.request
import uuid

URL = os.environ["VCON_URL"]
KEY = os.environ["VCON_KEY"]
TLS = ssl.create_default_context(cafile=os.environ.get("VCON_CA") or None)


def call(method, path, body=None, content_type=None):
    """Sends one request; returns the status and the body. An answer of 400
    or more is a problem document, raised with its type."""
    request = urllib.request.Request(URL + path, data=body, method=method)
    request.add_header("Authorization", "Bearer " + KEY)
    if content_type:
        request.add_header("Content-Type", content_type)
    try:
        with urllib.request.urlopen(request, context=TLS) as answer:
            return answer.status, answer.read()
    except urllib.error.HTTPError as answer:
        problem = json.loads(answer.read())
        raise SystemExit(f"{method} {path}: {answer.code} {problem['type']}") from None


def expect(holds, what):
    """Stops with `what` unless `holds`. Unlike an assert statement, it is
    never skipped."""
    if not holds:
        raise SystemExit(what)


def main():
    vcon_uuid = str(uuid.uuid4())
    vcon = json.dumps(
        {
            "created_at": "2026-10-25T12:00:00Z",
            "parties": [],
            "subject": "client example",
            "uuid": vcon_uuid,
        },
        separators=(",", ":"),
    ).encode()

    # Store: 201, with the new version in ETag.
    status, _ = call("POST", "/vcons", vcon, "application/vcon")
    expect(status == 201, f"a store answered {status}, not 201")
    print("stored", vcon_uuid)

    # List: the newest vCon comes first.
    _, page = call("GET", "/vcons?limit=10")
    expect(
        any(v["uuid"] == vcon_uuid for v in json.loads(page)["vcons"]),
        "the stored vCon is not in the list",
    )
    print("listed", vcon_uuid)

    # Read: the body comes back exactly as stored.
    _, body = call("GET", f"/vcons/{vcon_uuid}")
    expect(body == vcon, "the body read differs from the one stored")
    print("read", vcon_uuid, len(body), "bytes")

    # Delete: 204, and the vCon is gone (410) from then on.
    status, _ = call("DELETE", f"/vcons/{vcon_uuid}")
    expect(status == 204, f"a delete answered {status}, not 204")
    try:
        call("GET", f"/vcons/{vcon_uuid}")
    except SystemExit as gone:
        expect("410 https://vcon-server.com/problems/gone" in str(gone), str(gone))
    else:
        raise SystemExit("a deleted vCon was read")
    print("deleted", vcon_uuid)


if __name__ == "__main__":
    sys.exit(main())
