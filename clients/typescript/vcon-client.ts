// Stores a vCon, finds it in the list, reads it back, and deletes it, with
// Node's own fetch. Run it with `node vcon-client.ts` (Node 22.18 or later
// runs TypeScript as it is).
//
// Set VCON_URL (https://vcon.example.com) and VCON_KEY (a key with the
// write, read and delete scopes). For a certificate your system does not
// trust, set NODE_EXTRA_CA_CERTS to its PEM file; Node reads it at start.

import { randomUUID } from "node:crypto";

const url = process.env.VCON_URL ?? "";
const key = process.env.VCON_KEY ?? "";

interface Listing {
  vcons: { uuid: string; version: string; stored_at: string; updated_at: string }[];
  next: string | null;
}

// Sends one request. An answer of 400 or more is a problem document,
// thrown with its type.
async function call(method: string, path: string, body?: string): Promise<Response> {
  const headers: Record<string, string> = { Authorization: `Bearer ${key}` };
  if (body !== undefined) headers["Content-Type"] = "application/vcon";
  const answer = await fetch(url + path, { method, headers, body });
  if (answer.status >= 400) {
    const problem = (await answer.json()) as { type: string };
    throw new Error(`${method} ${path}: ${answer.status} ${problem.type}`);
  }
  return answer;
}

function expect(holds: boolean, what: string): void {
  if (!holds) throw new Error(what);
}

async function main(): Promise<void> {
  const uuid = randomUUID();
  const vcon = JSON.stringify({
    created_at: "2026-10-25T12:00:00Z",
    parties: [],
    subject: "client example",
    uuid,
  });

  // Store: 201, with the new version in ETag.
  const stored = await call("POST", "/vcons", vcon);
  expect(stored.status === 201, `a store answered ${stored.status}, not 201`);
  console.log("stored", uuid);

  // List: the newest vCon comes first.
  const page = (await (await call("GET", "/vcons?limit=10")).json()) as Listing;
  expect(page.vcons.some((v) => v.uuid === uuid), "the stored vCon is not in the list");
  console.log("listed", uuid);

  // Read: the body comes back exactly as stored.
  const body = Buffer.from(await (await call("GET", `/vcons/${uuid}`)).arrayBuffer());
  expect(body.equals(Buffer.from(vcon)), "the body read differs from the one stored");
  console.log("read", uuid, body.length, "bytes");

  // Delete: 204, and the vCon is gone (410) from then on.
  const deleted = await call("DELETE", `/vcons/${uuid}`);
  expect(deleted.status === 204, `a delete answered ${deleted.status}, not 204`);
  const gone = await fetch(`${url}/vcons/${uuid}`, { headers: { Authorization: `Bearer ${key}` } });
  expect(gone.status === 410, `a deleted vCon answered ${gone.status}, not 410`);
  console.log("deleted", uuid);
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
