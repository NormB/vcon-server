// A Reveal reads the vCon again with a marker that names the part, so the
// audit log records a reveal of that part and not an ordinary read. The
// marker is the only thing the reveal adds: the key is sent as on every
// request, and a read that reveals nothing carries no marker.
import { randomUUID } from "node:crypto";
import { test, expect, signIn, store, openView, auditLog } from "./fixtures.js";

/** An unsigned vCon with content in every part, as bytes. */
function call(uuid) {
  const body = {
    uuid,
    created_at: "2026-10-23T12:00:00Z",
    subject: "Billing question",
    parties: [{ tel: "+15555550123", name: "Alice Example" }, { mailto: "bob@example.com" }],
    dialog: [{ type: "text", start: "2026-10-23T12:00:00Z", parties: [0, 1], body: "hello there", duration: 30 }],
    analysis: [{ type: "transcript", dialog: 0, vendor: "example", body: "Alice: hello there" }],
  };
  return Buffer.from(JSON.stringify(body));
}

/** Stores a vCon and opens it masked in the vCons view. */
async function opened(page, request, tenant) {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), call(uuid));
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  await expect(page.locator("#vcon")).toBeVisible();
  return uuid;
}

/** Stores a vCon and finds it in the Legal view. */
async function found(page, request, tenant) {
  const uuid = randomUUID();
  const version = await store(request, await tenant.key(["write"]), call(uuid));
  await signIn(page, await tenant.key(["legal"]));
  await openView(page, "legal");
  await page.fill("#legal-uuid", uuid);
  await page.click("#legal-find button[type=submit]");
  await expect(page.locator("#legal-summary")).toBeVisible();
  return { uuid, version: version.replaceAll('"', "") };
}

/** What the tenant's log holds for `uuid`: each record's action, status and version. */
async function recorded(request, tenant, uuid) {
  const log = await auditLog(request, await tenant.key(["audit"]), "/audit");
  return log.filter((r) => r.target === uuid).map((r) => [r.action, r.status, r.version]);
}

/** The actions the tenant's log holds for `uuid`, without the create. */
async function actions(request, tenant, uuid) {
  return (await recorded(request, tenant, uuid)).map((r) => r[0]).filter((a) => a !== "create");
}

for (const [part, button, revealed] of [
  ["parties", "#vcon-reveal-parties", "#vcon-section"],
  ["subject", "#vcon-reveal-subject", "#vcon-section"],
  ["dialog", "#vcon-reveal-dialog", "#vcon-section"],
  ["analysis", "#vcon-reveal-analysis", "#vcon-section"],
  ["links", "#vcon-reveal-links", "#vcon-links-state"],
  ["all", "#reveal", "#vcon-body"],
]) {
  test(`a reveal of ${part} is recorded as reveal_${part}`, async ({ page, request, tenant }) => {
    const uuid = await opened(page, request, tenant);
    await page.click(button);
    await expect(page.locator(revealed)).not.toHaveText("");
    expect(await actions(request, tenant, uuid)).toEqual(["read", `reveal_${part}`]);
  });
}

test("a reveal in the Legal view is recorded as a legal reveal of its part", async ({ page, request, tenant }) => {
  const { uuid, version } = await found(page, request, tenant);
  await page.click("#legal-reveal-dialog");
  await expect(page.locator("#legal-section")).toContainText("hello there");
  expect(await recorded(request, tenant, uuid)).toContainEqual(["legal_reveal_dialog", 200, version]);
});

test("a reveal of the whole version in the Legal view is recorded as legal_reveal_all", async ({ page, request, tenant }) => {
  const { uuid, version } = await found(page, request, tenant);
  await page.click("#legal-reveal");
  await expect(page.locator("#legal-body")).toContainText("Alice Example");
  expect(await recorded(request, tenant, uuid)).toContainEqual(["legal_reveal_all", 200, version]);
});

test("a reveal of an opened kept version is recorded with that version", async ({ page, request, tenant }) => {
  const { uuid, version } = await found(page, request, tenant);
  await page.locator("#versions button.open-version").first().click();
  await expect(page.locator("#version-summary")).toBeVisible();
  await page.click("#version-reveal-parties");
  await expect(page.locator("#version-section")).toContainText("Alice Example");
  await page.click("#version-reveal");
  await expect(page.locator("#version-body")).toContainText("Alice Example");
  const log = await recorded(request, tenant, uuid);
  expect(log).toContainEqual(["legal_reveal_parties", 200, version]);
  expect(log).toContainEqual(["legal_reveal_all", 200, version]);
});

test("the Audit view shows a reveal under its own action", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), call(uuid));
  await signIn(page, await tenant.key(["read", "audit"]));
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  await page.click("#vcon-reveal-parties");
  await expect(page.locator("#vcon-section")).toContainText("Alice Example");
  await openView(page, "audit");
  const row = page.locator(`#audit-log tbody tr:has-text("${uuid}")`).filter({ hasText: "reveal_parties" });
  await expect(row).toHaveCount(1);
  await expect(row.locator("td").nth(6)).toHaveText("200");
});

test("a reveal the server refuses is recorded as the reveal it was", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${await tenant.key(["delete"])}` } });
  await page.click("#vcon-reveal-dialog");
  await expect(page.locator("#vcon-message")).toContainText("(410)");
  await expect(page.locator("#vcon-section")).toBeHidden();
  expect(await recorded(request, tenant, uuid)).toContainEqual(["reveal_dialog", 410, null]);
});

test("a legal reveal of a purged vCon is refused and recorded as the reveal it was", async ({ page, request, tenant }) => {
  const { uuid } = await found(page, request, tenant);
  await tenant.admin("PATCH", `/admin/tenants/${tenant.id}`, { retention: "0d" });
  const gone = await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${await tenant.key(["delete"])}` } });
  expect(gone.status()).toBe(204);
  await page.click("#legal-reveal");
  await expect(page.locator("#legal-message")).toContainText("(410)");
  await expect(page.locator("#legal-body")).toBeHidden();
  expect(await recorded(request, tenant, uuid)).toContainEqual(["legal_reveal_all", 410, null]);
});

test("a reveal that gets no answer says so, shows nothing and leaves no record", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  await page.route("**/vcons/*?reveal=*", (route) => route.abort());
  await page.click("#vcon-reveal-parties");
  await expect(page.locator("#vcon-message")).toContainText("could not be reached");
  await expect(page.locator("#vcon-section")).toBeHidden();
  expect(await actions(request, tenant, uuid)).toEqual(["read"]);
});

test("revealing the same part twice leaves two records", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  await page.click("#vcon-reveal-subject");
  await expect(page.locator("#vcon-section")).toHaveText("Billing question");
  await page.click("#vcon-reveal-parties");
  await expect(page.locator("#vcon-section")).toContainText("Alice Example");
  await page.click("#vcon-reveal-subject");
  await expect(page.locator("#vcon-section")).toHaveText("Billing question");
  expect(await actions(request, tenant, uuid)).toEqual(["read", "reveal_subject", "reveal_parties", "reveal_subject"]);
});

test("a reveal adds the marker to its request and nothing else", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  const sent = [];
  page.on("request", (r) => sent.push(r));
  await page.click("#vcon-reveal-analysis");
  await expect(page.locator("#vcon-section")).toContainText("Alice: hello there");
  expect(sent).toHaveLength(1);
  const url = new URL(sent[0].url());
  expect(sent[0].method()).toBe("GET");
  expect(url.pathname).toBe(`/vcons/${uuid}`);
  expect(url.search).toBe("?reveal=analysis");
  expect(url.hash).toBe("");
  expect(await sent[0].headerValue("authorization")).toMatch(/^Bearer vcs_/);
});

test("the masked note says a reveal is recorded with the part it showed", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  await expect(page.locator("#vcon .note").first()).toContainText(
    "The audit log records each Reveal with the part it showed, and each Download as a read.",
  );
});
