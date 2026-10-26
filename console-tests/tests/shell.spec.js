// The console shell: signing in checks the key against the API, the key is
// kept in page memory only, and a reload forgets it.
import { test, expect, signIn, stored, nothingStored } from "./fixtures.js";

test("a key the API accepts signs in and shows what it may do", async ({ page, tenant }) => {
  const token = await tenant.key(["read", "delete"]);
  await signIn(page, token);
  await expect(page.locator("#key-info")).toBeVisible();
  await expect(page.locator("#key-fields")).toContainText("read, delete");
  await expect(page.locator("#key-fields")).toContainText(tenant.id);
  await expect(page.locator("#sign-in")).toBeHidden();
  await expect(page.locator("#key")).toHaveValue("");
});

test("a key the API refuses does not sign in", async ({ page, tenant }) => {
  const token = await tenant.key(["read"]);
  const keys = await tenant.admin("GET", `/admin/tenants/${tenant.id}/keys`);
  await tenant.admin("DELETE", `/admin/keys/${keys[0].id}`);
  await signIn(page, token);
  await expect(page.locator("#sign-in-error")).toHaveText("The key was not accepted.");
  await expect(page.locator("#key-info")).toBeHidden();
});

test("a reload forgets the key", async ({ page, tenant }) => {
  const token = await tenant.key(["read"]);
  await signIn(page, token);
  await expect(page.locator("#key-info")).toBeVisible();
  const after = [];
  page.on("request", (r) => after.push(r));
  await page.reload();
  // Everything the reloaded page asks for has been asked once it is idle.
  await page.waitForLoadState("networkidle");
  await expect(page.locator("#sign-in")).toBeVisible();
  await expect(page.locator("#key-info")).toBeHidden();
  await expect(page.locator("#key")).toHaveValue("");
  for (const r of after) {
    expect(await r.headerValue("authorization"), r.url()).toBeNull();
  }
  expect(await stored(page)).toEqual(nothingStored);
});

test("signing out forgets the key and nothing is kept in the browser", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read", "write"]));
  await expect(page.locator("#key-info")).toBeVisible();
  expect(await stored(page)).toEqual(nothingStored);
  await page.click("#sign-out");
  await expect(page.locator("#sign-in")).toBeVisible();
  await expect(page.locator("#key-fields")).toBeEmpty();
  expect(await stored(page)).toEqual(nothingStored);
});

test("the sign-in request carries the key in its header, never in the URL", async ({ page, tenant }) => {
  const token = await tenant.key(["read"]);
  const requests = [];
  page.on("request", (r) => requests.push(r));
  await signIn(page, token);
  await expect(page.locator("#key-info")).toBeVisible();
  const asked = requests.filter((r) => new URL(r.url()).pathname === "/key");
  expect(asked).toHaveLength(1);
  expect(await asked[0].headerValue("authorization")).toBe(`Bearer ${token}`);
  for (const r of requests) {
    expect(r.url()).not.toContain(token);
  }
});
