// The admin views for an operator key: tenants and their keys, each action
// leaving the same admin-log record as the same request made to the API.
import { randomUUID } from "node:crypto";
import { test, expect, signIn, openView, auditLog, done, operatorKey } from "./fixtures.js";

const auth = { Authorization: `Bearer ${operatorKey}` };

/** The admin log's records about `target`, as what was done. */
async function about(request, target) {
  const log = await auditLog(request, operatorKey, "/admin/audit");
  return log.filter((r) => r.target === target).map(done);
}

test("tenants: create, set retention and remove leave the records the API leaves", async ({ page, request }) => {
  const viaConsole = `console-${randomUUID()}`;
  await signIn(page, operatorKey);
  await openView(page, "tenants");
  await page.fill("#tenant-name", viaConsole);
  await page.fill("#tenant-retention", "90d");
  await page.click("#tenant-create button[type=submit]");
  const row = page.locator(`#tenants tbody tr:has-text("${viaConsole}")`);
  await expect(row).toContainText("90d");
  const consoleId = (await row.locator("td").first().textContent()).trim();
  const asked = [];
  page.on("dialog", (d) => {
    asked.push(d.message());
    return d.type() === "prompt" ? d.accept("30d") : d.accept();
  });
  await row.locator("button.set-retention").click();
  await expect(row).toContainText("30d");
  await row.locator("button.remove").click();
  await expect(row).toHaveCount(0);
  expect(asked).toEqual([
    `Retention period for ${viaConsole}, such as 7y, 18m or 90d; empty for the server default`,
    `Remove tenant ${viaConsole}?`,
  ]);

  const made = await request.post("/admin/tenants", { headers: auth, data: { name: `api-${randomUUID()}`, retention: "90d" } });
  expect(made.status()).toBe(201);
  const apiId = (await made.json()).id;
  expect((await request.patch(`/admin/tenants/${apiId}`, { headers: auth, data: { retention: "30d" } })).status()).toBe(200);
  expect((await request.delete(`/admin/tenants/${apiId}`, { headers: auth })).status()).toBe(204);

  const recorded = await about(request, consoleId);
  expect(recorded.map((r) => r.action)).toEqual(["tenant_create", "tenant_retention", "tenant_remove"]);
  expect(recorded).toEqual(await about(request, apiId));
});

test("keys: a new token is shown once, revoking works, both recorded as by the API", async ({ page, request, tenant }) => {
  await signIn(page, operatorKey);
  await openView(page, "tenants");
  await page.locator(`#tenants tbody tr:has-text("${tenant.id}") button.keys`).click();
  await page.check('#key-create input[value="read"]');
  await page.check('#key-create input[value="audit"]');
  await page.fill("#key-label", "made-in-console");
  await page.click("#key-create button[type=submit]");
  await expect(page.locator("#new-token")).toHaveText(/^vcs_/);
  const token = (await page.locator("#new-token").textContent()).trim();
  expect(token).toMatch(/^vcs_[0-9a-f]{16}_/);
  const own = await request.get("/key", { headers: { Authorization: `Bearer ${token}` } });
  expect(await own.json()).toMatchObject({ tenant: tenant.id, scopes: ["read", "audit"] });
  const consoleKey = token.slice(4, 20);
  const listed = page.locator(`#key-list tbody tr:has-text("${consoleKey}")`);
  await expect(listed).toContainText("made-in-console");

  // Leaving the view forgets the token; it is never shown again.
  await openView(page, "tenants");
  await page.locator(`#tenants tbody tr:has-text("${tenant.id}") button.keys`).click();
  await expect(page.locator(`#key-list tbody tr:has-text("${consoleKey}")`)).toBeVisible();
  expect(await page.content()).not.toContain(token);

  const asked = [];
  page.on("dialog", (d) => {
    asked.push(d.message());
    return d.accept();
  });
  await page.locator(`#key-list tbody tr:has-text("${consoleKey}") button.revoke`).click();
  await expect(page.locator(`#key-list tbody tr:has-text("${consoleKey}") td`).nth(4)).not.toHaveText("");
  expect(asked).toEqual([`Revoke key ${consoleKey}?`]);
  expect((await request.get("/key", { headers: { Authorization: `Bearer ${token}` } })).status()).toBe(401);

  const made = await request.post(`/admin/tenants/${tenant.id}/keys`, { headers: auth, data: { scopes: ["read", "audit"], label: "made-by-api" } });
  expect(made.status()).toBe(201);
  const apiKey = (await made.json()).id;
  expect((await request.delete(`/admin/keys/${apiKey}`, { headers: auth })).status()).toBe(204);

  const recorded = await about(request, consoleKey);
  expect(recorded.map((r) => r.action)).toEqual(["key_create", "key_revoke"]);
  expect(recorded).toEqual(await about(request, apiKey));
});

test("a tenant key gets no admin view", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["write", "read", "delete", "legal", "audit"]));
  await expect(page.locator('#views button[data-view="vcons"]')).toBeVisible();
  await expect(page.locator('#views button[data-view="tenants"]')).toHaveCount(0);
});
