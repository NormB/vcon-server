// Every list says when it is loading, when it is empty and when it could
// not be read, and a request that gets no answer is reported, not lost.
import { randomUUID } from "node:crypto";
import { test, expect, signIn, vcon, store, openView, operatorKey } from "./fixtures.js";

const UNREACHABLE = "The server could not be reached. Check the connection and try again.";

test("an empty vCon list says there are none", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  await expect(page.locator("#vcon-list-state")).toHaveText("No vCons to show.");
  await expect(page.locator("#vcon-list tbody tr")).toHaveCount(0);
});

test("a vCon list with rows shows no empty line", async ({ page, request, tenant }) => {
  await store(request, await tenant.key(["write"]), vcon(randomUUID(), "one"));
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  await expect(page.locator("#vcon-list tbody tr")).toHaveCount(1);
  await expect(page.locator("#vcon-list-state")).toHaveText("");
});

test("a list says it is loading until the answer comes", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read"]));
  let release;
  const held = new Promise((r) => (release = r));
  await page.route("**/vcons?*", async (route) => {
    await held;
    await route.continue();
  });
  await openView(page, "vcons");
  await expect(page.locator("#vcon-list-state")).toHaveText("Loading vCons…");
  release();
  await expect(page.locator("#vcon-list-state")).toHaveText("No vCons to show.");
});

test("a list state is announced politely", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  await expect(page.locator("#vcon-list-state")).toHaveAttribute("role", "status");
});

test("a vCon list that cannot be read says why and what to do", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read"]));
  await page.route("**/vcons?*", (route) => route.abort());
  await openView(page, "vcons");
  await expect(page.locator("#view [role=alert]").filter({ hasText: UNREACHABLE })).toHaveCount(1);
  await expect(page.locator("#vcon-list-state")).toHaveText("");
});

test("a list the server refuses shows the refusal, not an empty list", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read"]));
  await page.route("**/vcons?*", (route) =>
    route.fulfill({ status: 503, contentType: "application/problem+json", body: '{"title":"Not available","status":503}' }),
  );
  await openView(page, "vcons");
  await expect(page.locator("#view")).toContainText("Not available (503). Try again; if it repeats, tell the server's operator.");
  await expect(page.locator("#vcon-list-state")).toHaveText("");
});

test("an audit log that cannot be read says why", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["audit"]));
  await page.route("**/audit?*", (route) => route.abort());
  await openView(page, "audit");
  await expect(page.locator("#audit-message")).toHaveText(UNREACHABLE);
});

test("an audit log with records shows them and no empty line", async ({ page, request, tenant }) => {
  await store(request, await tenant.key(["write"]), vcon(randomUUID(), "one"));
  await signIn(page, await tenant.key(["audit"]));
  await openView(page, "audit");
  await expect(page.locator("#audit-log tbody tr").first()).toBeVisible();
  await expect(page.locator("#audit-log-state")).toHaveText("");
});

test("a tenant list that cannot be read says why", async ({ page }) => {
  await signIn(page, operatorKey);
  await page.route("**/admin/tenants", (route) => route.abort());
  await openView(page, "tenants");
  await expect(page.locator("#tenant-message")).toHaveText(UNREACHABLE);
});

test("a tenant with no keys says there are none", async ({ page, tenant }) => {
  await signIn(page, operatorKey);
  await openView(page, "tenants");
  await page.locator(`#tenants tbody tr:has-text("${tenant.id}") button.keys`).click();
  await expect(page.locator("#key-list-state")).toHaveText("No keys to show.");
});

test("a sign-in that gets no answer says the server could not be reached", async ({ page, tenant }) => {
  const token = await tenant.key(["read"]);
  await page.goto("/console/");
  await page.route("**/key", (route) => route.abort());
  await page.fill("#key", token);
  await page.click("#sign-in button[type=submit]");
  await expect(page.locator("#sign-in-error")).toHaveText(UNREACHABLE);
  await expect(page.locator("#sign-in")).toBeVisible();
});

test("a store that gets no answer says so", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["write"]));
  await openView(page, "vcons");
  await page.route("**/vcons", (route) => route.abort());
  await page.setInputFiles("#store-file", { name: "c.vcon", mimeType: "application/vcon", buffer: vcon(randomUUID(), "x") });
  await page.click("#store button[type=submit]");
  await expect(page.locator("#store-result")).toHaveText(UNREACHABLE);
});

test("opening a uuid that is not stored says what to check", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  await page.fill("#uuid", randomUUID());
  await page.click("#open-uuid");
  await expect(page.locator("#vcon-message")).toContainText("(404). Check the uuid or id: it may be deleted or belong to another tenant.");
  await expect(page.locator("#vcon")).toBeHidden();
});
