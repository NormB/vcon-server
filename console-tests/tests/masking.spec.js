// An opened vCon is masked first: a summary with counts and masked parties,
// and the whole text only after Reveal, which reads it again so the audit
// log holds one record for the summary and one for each reveal. Times are
// shown in UTC with the offset stated.
import { randomUUID } from "node:crypto";
import { test, expect, signIn, store, openView, auditLog } from "./fixtures.js";

/** An unsigned vCon with two parties, as bytes. */
function call(uuid) {
  const body = {
    uuid,
    created_at: "2026-10-23T12:00:00Z",
    subject: "Billing question",
    parties: [
      { tel: "+15555550123", name: "Alice Example" },
      { mailto: "bob@example.com", name: "Bob Example" },
    ],
    dialog: [{ type: "text", start: "2026-10-23T12:00:00Z", parties: [0, 1], body: "hello there", duration: 30 }],
  };
  return Buffer.from(JSON.stringify(body));
}

async function opened(page, request, tenant, scopes = ["read"]) {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), call(uuid));
  await signIn(page, await tenant.key(scopes));
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  await expect(page.locator("#vcon")).toBeVisible();
  return uuid;
}

test("an opened vCon shows its summary and none of its content", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const summary = page.locator("#vcon-summary");
  await expect(summary).toContainText("unsigned");
  await expect(summary).toContainText("2026-10-23 12:00:00 UTC+00:00");
  await expect(summary).toContainText("Party 1: number ending 0123; name hidden");
  await expect(summary).toContainText("Party 2: address at example.com; name hidden");
  await expect(summary).toContainText("present, hidden");
  await expect(page.locator("#vcon-body")).toBeHidden();
  const shown = await page.locator("#view").innerText();
  for (const hidden of ["Alice", "Bob Example", "Billing question", "5555550123", "bob@", "hello there"]) {
    expect(shown).not.toContain(hidden);
  }
});

test("Reveal shows the whole vCon as text", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  await page.click("#reveal");
  const body = page.locator("#vcon-body");
  await expect(body).toBeVisible();
  await expect(body).toContainText("Alice Example");
  await expect(body).toContainText("hello there");
  await expect(page.locator("#reveal")).toBeHidden();
});

test("the summary is a recorded read and the reveal a recorded reveal", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  await page.click("#reveal");
  await expect(page.locator("#vcon-body")).toBeVisible();
  const log = await auditLog(request, await tenant.key(["audit"]), "/audit");
  const recorded = log.filter((r) => r.target === uuid && r.status === 200).map((r) => r.action);
  expect(recorded).toEqual(["read", "reveal_all"]);
});

test("opening another vCon masks again", async ({ page, request, tenant }) => {
  const first = await opened(page, request, tenant);
  await page.click("#reveal");
  await expect(page.locator("#vcon-body")).toContainText("Alice Example");
  await page.click(`#vcon-list tbody tr:has-text("${first}") button.open`);
  await expect(page.locator("#reveal")).toBeVisible();
  await expect(page.locator("#vcon-body")).toBeHidden();
  await expect(page.locator("#vcon-body")).toHaveText("");
});

test("a reveal the server refuses shows why and keeps the vCon masked", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${await tenant.key(["delete"])}` } });
  await page.click("#reveal");
  await expect(page.locator("#vcon-message")).toContainText("(410). It was deleted: a key with the legal scope can still find it in the Legal view.");
  await expect(page.locator("#vcon-body")).toBeHidden();
});

test("the list shows stored times in UTC with the offset stated", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const cells = page.locator("#vcon-list tbody tr").first().locator("td");
  await expect(cells.nth(2)).toHaveText(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC\+00:00$/);
  await expect(cells.nth(3)).toHaveText(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC\+00:00$/);
});

test("the legal view masks the vCon it finds until Reveal", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), call(uuid));
  await signIn(page, await tenant.key(["legal"]));
  await openView(page, "legal");
  await page.fill("#legal-uuid", uuid);
  await page.click("#legal-find button[type=submit]");
  await expect(page.locator("#legal-summary")).toContainText("Party 1: number ending 0123; name hidden");
  await expect(page.locator("#legal-body")).toBeHidden();
  expect(await page.locator("#view").innerText()).not.toContain("Alice");
  await page.click("#legal-reveal");
  await expect(page.locator("#legal-body")).toContainText("Alice Example");
});

test("a kept version opens masked, and its Reveal shows it", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), call(uuid));
  await signIn(page, await tenant.key(["legal"]));
  await openView(page, "legal");
  await page.fill("#legal-uuid", uuid);
  await page.click("#legal-find button[type=submit]");
  await page.locator("#versions button.open-version").first().click();
  await expect(page.locator("#version-summary")).toContainText("Party 2: address at example.com; name hidden");
  await expect(page.locator("#version-body")).toBeHidden();
  expect(await page.locator("#view").innerText()).not.toContain("Bob Example");
  await page.click("#version-reveal");
  await expect(page.locator("#version-body")).toContainText("Bob Example");
});
