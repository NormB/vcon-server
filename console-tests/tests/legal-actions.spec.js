// The legal view's actions: hold, release and restore, each leaving the
// same audit record as the same request made straight to the API.
import { randomUUID } from "node:crypto";
import { test, expect, signIn, vcon, store, openView, auditLog, done } from "./fixtures.js";

test("hold, release and restore leave the records the API leaves", async ({ page, request, tenant }) => {
  const writer = await tenant.key(["write", "delete"]);
  const legal = await tenant.key(["legal"]);
  const auditor = await tenant.key(["audit"]);
  const [byConsole, byApi] = [randomUUID(), randomUUID()];
  for (const uuid of [byConsole, byApi]) {
    await store(request, writer, vcon(uuid, "a call"));
    const gone = await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${writer}` } });
    expect(gone.status()).toBe(204);
  }

  await signIn(page, legal);
  await openView(page, "legal");
  await page.fill("#legal-uuid", byConsole);
  await page.click("#legal-find button[type=submit]");
  await expect(page.locator("#legal-state")).toHaveText("deleted");
  await page.click("#hold");
  await expect(page.locator("#legal-message")).toHaveText(`Held ${byConsole}`);
  await page.click("#release");
  await expect(page.locator("#legal-message")).toHaveText(`Released ${byConsole}`);
  await page.click("#restore");
  await expect(page.locator("#legal-message")).toContainText(`Restored ${byConsole}`);
  await expect(page.locator("#legal-state")).toHaveText("live");

  const auth = { Authorization: `Bearer ${legal}` };
  expect((await request.put(`/legal/vcons/${byApi}/hold`, { headers: auth })).status()).toBe(204);
  expect((await request.delete(`/legal/vcons/${byApi}/hold`, { headers: auth })).status()).toBe(204);
  expect((await request.post(`/legal/vcons/${byApi}/restore`, { headers: auth })).status()).toBe(200);

  const log = await auditLog(request, auditor, "/audit");
  const actions = ["hold", "release", "restore"];
  const of = (uuid) => log.filter((r) => r.target === uuid && actions.includes(r.action)).map(done);
  expect(of(byConsole)).toHaveLength(3);
  expect(of(byConsole)).toEqual(of(byApi));
});

test("a legal action the API refuses shows the problem as text", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), vcon(uuid, "still live"));
  await signIn(page, await tenant.key(["legal"]));
  await openView(page, "legal");
  await page.fill("#legal-uuid", uuid);
  await page.click("#legal-find button[type=submit]");
  await expect(page.locator("#legal-state")).toHaveText("live");
  await page.click("#restore");
  await expect(page.locator("#legal-message")).toHaveText(/\(409\)$/);
});
