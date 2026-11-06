#!/usr/bin/env python3
"""Works through the HTTP API with the Python standard library only: stores
a vCon, finds it in the list, reads it back, stores and reads back a
gzipped vCon, pages through both with a cursor, replaces the first under
If-Match, places and releases a legal hold, finds its own create in the
audit log, lists the tenants with an operator key, reads the vCon over MCP,
and deletes both.

Set VCON_URL (https://vcon.example.com) and four keys: VCON_KEY (the
write, read and delete scopes), VCON_LEGAL_KEY (legal), VCON_AUDIT_KEY
(audit) and VCON_OPERATOR_KEY (an operator key). For a certificate your
system does not trust, set VCON_CA (its PEM file).
"""

import gzip
import json
import os
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid

URL = os.environ["VCON_URL"]
KEY = os.environ["VCON_KEY"]
LEGAL_KEY = os.environ["VCON_LEGAL_KEY"]
AUDIT_KEY = os.environ["VCON_AUDIT_KEY"]
OPERATOR_KEY = os.environ["VCON_OPERATOR_KEY"]
TLS = ssl.create_default_context(cafile=os.environ.get("VCON_CA") or None)
MARKED = "⟦untrusted-vcon-data⟧{}⟦/untrusted-vcon-data⟧"


def call(method, path, body=None, headers=None, key=KEY):
    """Sends one request; returns the status, the headers and the body. An
    answer of 400 or more is a problem document, raised with its type."""
    request = urllib.request.Request(URL + path, data=body, method=method)
    request.add_header("Authorization", "Bearer " + key)
    for name, value in (headers or {}).items():
        request.add_header(name, value)
    try:
        with urllib.request.urlopen(request, context=TLS) as answer:
            return answer.status, answer.headers, answer.read()
    except urllib.error.HTTPError as answer:
        problem = json.loads(answer.read())
        raise SystemExit(f"{method} {path}: {answer.code} {problem['type']}") from None


def refused(method, path, body=None, headers=None):
    """Sends one request that must be refused; returns the refusal's
    "METHOD PATH: status type" text."""
    try:
        call(method, path, body, headers)
    except SystemExit as refusal:
        return str(refusal)
    raise SystemExit(f"{method} {path} was not refused")


def mcp(message):
    """Posts one JSON-RPC message to /mcp; returns the answer, or None for a
    notification."""
    _, _, body = call(
        "POST",
        "/mcp",
        json.dumps(message).encode(),
        {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
    )
    return json.loads(body) if body else None


def expect(holds, what):
    """Stops with `what` unless `holds`. Unlike an assert statement, it is
    never skipped."""
    if not holds:
        raise SystemExit(what)


def encode(vcon):
    return json.dumps(vcon, separators=(",", ":")).encode()


def main():
    vcon_uuid = str(uuid.uuid4())
    document = {
        "created_at": "2026-10-25T12:00:00Z",
        "parties": [],
        "subject": "client example",
        "uuid": vcon_uuid,
    }
    vcon = encode(document)

    # Store: 201, with the new version in ETag.
    status, headers, _ = call("POST", "/vcons", vcon, {"Content-Type": "application/vcon"})
    expect(status == 201, f"a store answered {status}, not 201")
    first_tag = headers["ETag"]
    print("stored", vcon_uuid)

    # List: the newest vCon comes first.
    _, _, page = call("GET", "/vcons?limit=10")
    mine = [v for v in json.loads(page)["vcons"] if v["uuid"] == vcon_uuid]
    expect(mine, "the stored vCon is not in the list")
    print("listed", vcon_uuid)

    # Read: the body comes back exactly as stored.
    _, _, body = call("GET", f"/vcons/{vcon_uuid}")
    expect(body == vcon, "the body read differs from the one stored")
    print("read", vcon_uuid, len(body), "bytes")

    # Gzip: a gzipped body is stored as sent, and read back with its media type.
    gzip_uuid = str(uuid.uuid4())
    sent = gzip.compress(
        encode({**document, "subject": "gzip example", "uuid": gzip_uuid}), mtime=0
    )
    call("POST", "/vcons", sent, {"Content-Type": "application/vcon+gzip"})
    _, headers, body = call("GET", f"/vcons/{gzip_uuid}")
    expect(body == sent, "the gzipped body read differs from the one stored")
    expect(
        headers["Content-Type"] == "application/vcon+gzip",
        "a gzipped vCon was not read back as application/vcon+gzip",
    )
    print("gzip", gzip_uuid, "read back as application/vcon+gzip")

    # Page: one vCon a page, from the first one's stored_at on, following
    # next until it is null. Each of the two must be seen exactly once.
    query = "limit=1&since=" + urllib.parse.quote(mine[0]["stored_at"], safe="")
    seen, pages, cursor = [], 0, None
    while True:
        _, _, body = call("GET", "/vcons?" + query + (f"&cursor={cursor}" if cursor else ""))
        page = json.loads(body)
        pages += 1
        seen += [v["uuid"] for v in page["vcons"]]
        cursor = page["next"]
        if cursor is None:
            break
    for each in (vcon_uuid, gzip_uuid):
        expect(seen.count(each) == 1, f"the pages did not list {each} exactly once")
    print("paged", len(seen), "vCons in", pages, "pages of 1")

    # Replace: If-Match names the current version; the answer has the new
    # one. The first version's tag is then stale, and refused with 412.
    replaced = encode({**document, "subject": "client example, replaced"})
    put = {"If-Match": first_tag, "Content-Type": "application/vcon"}
    _, headers, _ = call("PUT", f"/vcons/{vcon_uuid}", replaced, put)
    tag = headers["ETag"]
    expect(tag and tag != first_tag, "a replace did not answer a new ETag")
    stale = refused("PUT", f"/vcons/{vcon_uuid}", replaced, put)
    expect(stale.endswith(": 412 https://vcon-server.com/problems/precondition-failed"), stale)
    print("replaced", vcon_uuid + ", a stale If-Match refused with 412")

    # Legal hold: placed and released with the legal key, 204 each.
    for method in ("PUT", "DELETE"):
        status, _, _ = call(method, f"/legal/vcons/{vcon_uuid}/hold", key=LEGAL_KEY)
        expect(status == 204, f"{method} hold answered {status}, not 204")
    print("held", vcon_uuid, "and released")

    # Audit: the tenant's log, read with the audit key, holds this program's
    # create, made by VCON_KEY, whose id and tenant GET /key gives.
    me = json.loads(call("GET", "/key")[2])
    after = 0
    while True:
        _, _, body = call("GET", f"/audit?after={after}&limit=1000", key=AUDIT_KEY)
        records = [json.loads(line) for line in body.splitlines()]
        if any(
            r["action"] == "create"
            and r["target"] == vcon_uuid
            and r["actor"] == me["id"]
            and r["status"] == 201
            for r in records
        ):
            break
        expect(len(records) >= 1000, f"the audit log has no create of {vcon_uuid}")
        after = records[-1]["seq"]
    print("audit create", vcon_uuid, "by", me["id"])

    # Admin: the operator key lists every tenant, this key's among them.
    tenants = json.loads(call("GET", "/admin/tenants", key=OPERATOR_KEY)[2])
    expect(
        any(t["id"] == me["tenant"] for t in tenants),
        f"the admin tenant list does not have {me['tenant']}",
    )
    print("admin tenant", me["tenant"], "listed")

    # MCP: initialize, tools/list, and vcon_get, which answers the replaced
    # vCon's text between the untrusted-data markers, and its version.
    init = mcp(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "vcon-client", "version": "1"},
            },
        }
    )["result"]
    expect(init["protocolVersion"] == "2025-06-18", "initialize did not agree to 2025-06-18")
    mcp({"jsonrpc": "2.0", "method": "notifications/initialized"})
    tools = [t["name"] for t in mcp({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})["result"]["tools"]]
    expect("vcon_get" in tools, "tools/list has no vcon_get")
    got = mcp(
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "vcon_get", "arguments": {"uuid": vcon_uuid}},
        }
    )["result"]
    expect(got["isError"] is False, f"vcon_get answered an error: {got}")
    expect(f'"{got["structuredContent"]["version"]}"' == tag, "vcon_get answered another version")
    expect(
        got["structuredContent"]["vcon"] == MARKED.format(replaced.decode()),
        "vcon_get answered another text",
    )
    print(
        "mcp vcon_get", vcon_uuid, "from", init["serverInfo"]["name"] + ",", len(tools), "tools listed"
    )

    # Delete: 204, and each vCon is gone (410) from then on.
    for each in (vcon_uuid, gzip_uuid):
        status, _, _ = call("DELETE", f"/vcons/{each}")
        expect(status == 204, f"a delete answered {status}, not 204")
        gone = refused("GET", f"/vcons/{each}")
        expect(gone.endswith(": 410 https://vcon-server.com/problems/gone"), gone)
    print("deleted", vcon_uuid, gzip_uuid)


if __name__ == "__main__":
    sys.exit(main())
