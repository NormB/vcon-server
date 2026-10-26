// The audit view: the key's tenant log for an audit key, the admin log for
// an operator key, a page at a time, as the API serves them.
import { randomUUID } from "node:crypto";
import { test, expect, signIn, vcon, store, openView, auditLog, operatorKey } from "./fixtures.js";

/** The rows of the audit table, as the records' fields in column order. */
function fields(r) {
  return [r.seq, r.at, r.actor, r.action, r.target ?? "", r.version ?? "", r.status, r.count].map(String);
}

test("an audit key reads its tenant's log, a page at a time", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  const reader = await tenant.key(["read"]);
  await store(request, await tenant.key(["write"]), vcon(uuid, "read often"));
  for (let i = 0; i < 60; i++) {
    expect((await request.get(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${reader}` } })).status()).toBe(200);
  }
  const auditor = await tenant.key(["audit"]);
  await signIn(page, auditor);
  await openView(page, "audit");
  const rows = page.locator("#audit-log tbody tr");
  await expect(rows).toHaveCount(50);
  await page.click("#audit-more");
  await expect(page.locator("#audit-more")).toBeHidden();
  const shown = [];
  for (const r of await rows.all()) {
    shown.push(await r.locator("td").allTextContents());
  }
  expect(shown.length).toBeGreaterThan(60);
  // The same records, field for field, as the API serves them; the API's
  // own read is recorded after everything the console showed.
  const log = await auditLog(request, auditor, "/audit");
  expect(shown).toEqual(log.slice(0, shown.length).map(fields));
  expect(shown.filter((r) => r[3] === "read")).toHaveLength(60);
});

test("an operator key reads the admin log", async ({ page, tenant }) => {
  await signIn(page, operatorKey);
  await openView(page, "audit");
  await expect(page.locator("#audit-title")).toHaveText("Admin log");
  for (;;) {
    const more = page.locator("#audit-more");
    if (await more.isHidden()) break;
    await more.click();
  }
  await expect(page.locator(`#audit-log tbody tr:has-text("${tenant.id}")`).first()).toContainText("tenant_create");
});

test("a key without audit gets no audit view", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read", "legal"]));
  await expect(page.locator('#views button[data-view="vcons"]')).toBeVisible();
  await expect(page.locator('#views button[data-view="audit"]')).toHaveCount(0);
});
