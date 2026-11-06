// Works through the HTTP API with Node's own fetch: stores a vCon, finds it
// in the list, reads it back, stores and reads back a gzipped vCon, pages
// through both with a cursor, replaces the first under If-Match, places and
// releases a legal hold, finds its own create in the audit log, lists the
// tenants with an operator key, reads the vCon over MCP, and deletes both.
// Run it with `node vcon-client.ts` (Node 22.18 or later runs TypeScript as
// it is).
//
// Set VCON_URL (https://vcon.example.com) and four keys: VCON_KEY (the
// write, read and delete scopes), VCON_LEGAL_KEY (legal), VCON_AUDIT_KEY
// (audit) and VCON_OPERATOR_KEY (an operator key). For a certificate your
// system does not trust, set NODE_EXTRA_CA_CERTS to its PEM file; Node
// reads it at start.

import { randomUUID } from "node:crypto";
import { gzipSync } from "node:zlib";

const url = process.env.VCON_URL ?? "";
const key = process.env.VCON_KEY ?? "";
const legalKey = process.env.VCON_LEGAL_KEY ?? "";
const auditKey = process.env.VCON_AUDIT_KEY ?? "";
const operatorKey = process.env.VCON_OPERATOR_KEY ?? "";

interface Listing {
  vcons: { uuid: string; version: string; stored_at: string; updated_at: string }[];
  next: string | null;
}

interface AuditRecord {
  seq: number;
  action: string;
  actor: string;
  target: string | null;
  status: number;
}

interface Options {
  body?: BodyInit;
  headers?: Record<string, string>;
  as?: string;
}

// Sends one request, with VCON_KEY unless `as` names another key. An answer
// of 400 or more is a problem document, thrown with its type.
async function call(method: string, path: string, options: Options = {}): Promise<Response> {
  const headers = { Authorization: `Bearer ${options.as ?? key}`, ...options.headers };
  const answer = await fetch(url + path, { method, headers, body: options.body });
  if (answer.status >= 400) {
    const problem = (await answer.json()) as { type: string };
    throw new Error(`${method} ${path}: ${answer.status} ${problem.type}`);
  }
  return answer;
}

// Sends one request that must be refused; returns the refusal's
// "METHOD PATH: status type" text.
async function refused(method: string, path: string, options: Options = {}): Promise<string> {
  try {
    await call(method, path, options);
  } catch (refusal: unknown) {
    return refusal instanceof Error ? refusal.message : String(refusal);
  }
  throw new Error(`${method} ${path} was not refused`);
}

// Posts one JSON-RPC message to /mcp; returns the answer's result, or
// undefined for a notification.
async function mcp<T>(message: object): Promise<T> {
  const answer = await call("POST", "/mcp", {
    body: JSON.stringify({ jsonrpc: "2.0", ...message }),
    headers: { "Content-Type": "application/json", Accept: "application/json, text/event-stream" },
  });
  const text = await answer.text();
  if (text === "") return undefined as T;
  const rpc = JSON.parse(text) as { result?: T; error?: unknown };
  if (rpc.error !== undefined) throw new Error(`MCP answered ${JSON.stringify(rpc.error)}`);
  return rpc.result as T;
}

function expect(holds: boolean, what: string): void {
  if (!holds) throw new Error(what);
}

function encode(uuid: string, subject: string): string {
  return JSON.stringify({ created_at: "2026-10-25T12:00:00Z", parties: [], subject, uuid });
}

async function main(): Promise<void> {
  const uuid = randomUUID();
  const vcon = encode(uuid, "client example");

  // Store: 201, with the new version in ETag.
  const stored = await call("POST", "/vcons", {
    body: vcon,
    headers: { "Content-Type": "application/vcon" },
  });
  expect(stored.status === 201, `a store answered ${stored.status}, not 201`);
  const firstTag = stored.headers.get("ETag") ?? "";
  console.log("stored", uuid);

  // List: the newest vCon comes first.
  const page = (await (await call("GET", "/vcons?limit=10")).json()) as Listing;
  const mine = page.vcons.find((v) => v.uuid === uuid);
  expect(mine !== undefined, "the stored vCon is not in the list");
  console.log("listed", uuid);

  // Read: the body comes back exactly as stored.
  const body = Buffer.from(await (await call("GET", `/vcons/${uuid}`)).arrayBuffer());
  expect(body.equals(Buffer.from(vcon)), "the body read differs from the one stored");
  console.log("read", uuid, body.length, "bytes");

  // Gzip: a gzipped body is stored as sent, and read back with its media type.
  const gzipUuid = randomUUID();
  const sent = gzipSync(encode(gzipUuid, "gzip example"));
  await call("POST", "/vcons", { body: sent, headers: { "Content-Type": "application/vcon+gzip" } });
  const gzipped = await call("GET", `/vcons/${gzipUuid}`);
  const read = Buffer.from(await gzipped.arrayBuffer());
  expect(read.equals(sent), "the gzipped body read differs from the one stored");
  expect(
    gzipped.headers.get("Content-Type") === "application/vcon+gzip",
    "a gzipped vCon was not read back as application/vcon+gzip",
  );
  console.log("gzip", gzipUuid, "read back as application/vcon+gzip");

  // Page: one vCon a page, from the first one's stored_at on, following
  // next until it is null. Each of the two must be seen exactly once.
  const query = `/vcons?limit=1&since=${encodeURIComponent(mine?.stored_at ?? "")}`;
  const seen: string[] = [];
  let pages = 0;
  for (let cursor: string | null = null; ; ) {
    const path: string = cursor === null ? query : `${query}&cursor=${encodeURIComponent(cursor)}`;
    const each = (await (await call("GET", path)).json()) as Listing;
    pages += 1;
    seen.push(...each.vcons.map((v) => v.uuid));
    cursor = each.next;
    if (cursor === null) break;
  }
  for (const each of [uuid, gzipUuid]) {
    expect(seen.filter((s) => s === each).length === 1, `the pages did not list ${each} exactly once`);
  }
  console.log("paged", seen.length, "vCons in", pages, "pages of 1");

  // Replace: If-Match names the current version; the answer has the new
  // one. The first version's tag is then stale, and refused with 412.
  const replaced = encode(uuid, "client example, replaced");
  const put = { body: replaced, headers: { "If-Match": firstTag, "Content-Type": "application/vcon" } };
  const tag = (await call("PUT", `/vcons/${uuid}`, put)).headers.get("ETag") ?? "";
  expect(tag !== "" && tag !== firstTag, "a replace did not answer a new ETag");
  const stale = await refused("PUT", `/vcons/${uuid}`, put);
  expect(stale.endsWith(": 412 https://vcon-server.com/problems/precondition-failed"), stale);
  console.log(`replaced ${uuid}, a stale If-Match refused with 412`);

  // Legal hold: placed and released with the legal key, 204 each.
  for (const method of ["PUT", "DELETE"]) {
    const held = await call(method, `/legal/vcons/${uuid}/hold`, { as: legalKey });
    expect(held.status === 204, `${method} hold answered ${held.status}, not 204`);
  }
  console.log("held", uuid, "and released");

  // Audit: the tenant's log, read with the audit key, holds this program's
  // create, made by VCON_KEY, whose id and tenant GET /key gives.
  const me = (await (await call("GET", "/key")).json()) as { id: string; tenant: string };
  for (let after = 0; ; ) {
    const text = await (await call("GET", `/audit?after=${after}&limit=1000`, { as: auditKey })).text();
    const records = text
      .split("\n")
      .filter((line) => line !== "")
      .map((line) => JSON.parse(line) as AuditRecord);
    const found = records.some(
      (r) => r.action === "create" && r.target === uuid && r.actor === me.id && r.status === 201,
    );
    if (found) break;
    expect(records.length >= 1000, `the audit log has no create of ${uuid}`);
    after = records[records.length - 1].seq;
  }
  console.log("audit create", uuid, "by", me.id);

  // Admin: the operator key lists every tenant, this key's among them.
  const tenants = (await (await call("GET", "/admin/tenants", { as: operatorKey })).json()) as { id: string }[];
  expect(tenants.some((t) => t.id === me.tenant), `the admin tenant list does not have ${me.tenant}`);
  console.log("admin tenant", me.tenant, "listed");

  // MCP: initialize, tools/list, and vcon_get, which answers the replaced
  // vCon's text between the untrusted-data markers, and its version.
  const init = await mcp<{ protocolVersion: string; serverInfo: { name: string } }>({
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "vcon-client", version: "1" },
    },
  });
  expect(init.protocolVersion === "2025-06-18", "initialize did not agree to 2025-06-18");
  await mcp<undefined>({ method: "notifications/initialized" });
  const tools = (await mcp<{ tools: { name: string }[] }>({ id: 2, method: "tools/list" })).tools;
  expect(tools.some((t) => t.name === "vcon_get"), "tools/list has no vcon_get");
  const got = await mcp<{ isError: boolean; structuredContent: { version: string; vcon: string } }>({
    id: 3,
    method: "tools/call",
    params: { name: "vcon_get", arguments: { uuid } },
  });
  expect(got.isError === false, `vcon_get answered an error: ${JSON.stringify(got)}`);
  expect(`"${got.structuredContent.version}"` === tag, "vcon_get answered another version");
  expect(
    got.structuredContent.vcon === `⟦untrusted-vcon-data⟧${replaced}⟦/untrusted-vcon-data⟧`,
    "vcon_get answered another text",
  );
  console.log(`mcp vcon_get ${uuid} from ${init.serverInfo.name}, ${tools.length} tools listed`);

  // Delete: 204, and each vCon is gone (410) from then on.
  for (const each of [uuid, gzipUuid]) {
    const deleted = await call("DELETE", `/vcons/${each}`);
    expect(deleted.status === 204, `a delete answered ${deleted.status}, not 204`);
    const gone = await refused("GET", `/vcons/${each}`);
    expect(gone.endsWith(": 410 https://vcon-server.com/problems/gone"), gone);
  }
  console.log("deleted", uuid, gzipUuid);
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
