// A masked vCon is revealed a part at a time: parties, subject, dialog,
// analysis or links, each by a read of its own that the audit log records.
// Links are text that is never loaded; a download is a recorded read.
import { randomUUID } from "node:crypto";
import { test, expect, signIn, store, openView, auditLog } from "./fixtures.js";

/** An unsigned vCon with content in every part, as an object. */
function call(uuid, extra = {}) {
  return {
    uuid,
    created_at: "2026-10-23T12:00:00Z",
    subject: "Billing question",
    parties: [{ tel: "+15555550123", name: "Alice Example" }, { mailto: "bob@example.com" }],
    dialog: [
      { type: "text", start: "2026-10-23T12:00:00Z", parties: [0, 1], body: "hello there", duration: 30 },
      { type: "recording", start: "2026-10-23T12:01:00Z", parties: 0, url: "https://media.example.com/a.wav" },
    ],
    analysis: [{ type: "transcript", dialog: 0, vendor: "example", body: "Alice: hello there" }],
    ...extra,
  };
}

async function opened(page, request, tenant, body = null, scopes = ["read"]) {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), Buffer.from(JSON.stringify(body ?? call(uuid))));
  const stored = body === null ? uuid : body.uuid;
  await signIn(page, await tenant.key(scopes));
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${stored}") button.open`);
  await expect(page.locator("#vcon")).toBeVisible();
  return stored;
}

test("the summary lists each dialog without what was said", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const summary = page.locator("#vcon-summary");
  await expect(summary).toContainText("Dialog 1: text, 2026-10-23 12:00:00 UTC+00:00, 30 s, parties 1, 2; content hidden");
  await expect(summary).toContainText("Dialog 2: recording, 2026-10-23 12:01:00 UTC+00:00, no length, party 1; content at a link outside the server");
  await expect(summary).not.toContainText("media.example.com");
});

test("Reveal parties shows the parties and nothing else", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  await page.click("#vcon-reveal-parties");
  const part = page.locator("#vcon-section");
  await expect(part).toBeVisible();
  await expect(part).toBeFocused();
  await expect(part).toContainText("Alice Example");
  await expect(part).not.toContainText("hello there");
  await expect(page.locator("#vcon-section-title")).toHaveText("Parties, revealed");
  await expect(page.locator("#vcon-body")).toBeHidden();
});

test("Reveal subject, dialog and analysis each show their own part", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  const part = page.locator("#vcon-section");
  await page.click("#vcon-reveal-subject");
  await expect(part).toHaveText("Billing question");
  await page.click("#vcon-reveal-dialog");
  await expect(part).toContainText("hello there");
  await expect(part).not.toContainText("Billing question");
  await page.click("#vcon-reveal-analysis");
  await expect(part).toContainText("Alice: hello there");
  await expect(page.locator("#vcon-section-title")).toHaveText("Analysis, revealed");
});

test("each part revealed is a read of its own in the audit log", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  for (const name of ["parties", "subject", "dialog"]) {
    await page.click(`#vcon-reveal-${name}`);
    await expect(page.locator("#vcon-section-title")).toContainText("revealed");
  }
  await expect(page.locator("#vcon-section")).toContainText("hello there");
  const log = await auditLog(request, await tenant.key(["audit"]), "/audit");
  const reads = log.filter((r) => r.action === "read" && r.target === uuid && r.status === 200);
  expect(reads).toHaveLength(4);
});

test("a link is shown as text with a label, and nothing is loaded from it", async ({ page, request, tenant }) => {
  const asked = [];
  page.on("request", (r) => asked.push(r.url()));
  await opened(page, request, tenant);
  await page.click("#vcon-reveal-links");
  const item = page.locator("#vcon-links li");
  await expect(item).toHaveCount(1);
  await expect(item).toContainText("https://media.example.com/a.wav");
  await expect(item).toContainText("dialog 2");
  await expect(item).toContainText("points outside the server and is never loaded");
  expect(await page.locator("#view a[href], #view img, #view audio, #view video, #view iframe").count()).toBe(0);
  // Every request the page made went to the console's own origin.
  const own = new URL(page.url()).origin;
  expect(asked.length).toBeGreaterThan(0);
  expect(asked.filter((u) => new URL(u).origin !== own)).toEqual([]);
});

test("a hostile link is text: it is not a link and does not run", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  const body = call(uuid, { attachments: [{ url: "javascript:window.ran=1" }, { url: "data:text/html,<script>window.ran=2</script>" }] });
  await opened(page, request, tenant, body);
  await page.click("#vcon-reveal-links");
  await expect(page.locator("#vcon-links li")).toHaveCount(3);
  await expect(page.locator("#vcon-links")).toContainText("javascript:window.ran=1");
  expect(await page.locator("#view a[href], #view script:not([src])").count()).toBe(0);
  expect(await page.evaluate(() => window.ran)).toBeUndefined();
});

test("Copy puts the link's text on the clipboard and says so", async ({ page, context, request, tenant }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await opened(page, request, tenant);
  await page.click("#vcon-reveal-links");
  await page.click("#vcon-links li button");
  await expect(page.locator("#vcon-links-state")).toHaveText("Copied the link at dialog 2.");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("https://media.example.com/a.wav");
});

test("a vCon with no links says so", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await opened(page, request, tenant, { uuid, created_at: "2026-10-23T12:00:00Z", parties: [] });
  await page.click("#vcon-reveal-links");
  await expect(page.locator("#vcon-links-state")).toHaveText("No links in this vCon.");
  await expect(page.locator("#vcon-links li")).toHaveCount(0);
});

test("a part the server refuses to read again stays hidden and says why", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${await tenant.key(["delete"])}` } });
  await page.click("#vcon-reveal-parties");
  await expect(page.locator("#vcon-message")).toContainText("(410). It was deleted");
  await expect(page.locator("#vcon-section")).toBeHidden();
});

test("opening a vCon again hides every revealed part", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  await page.click("#vcon-reveal-parties");
  await expect(page.locator("#vcon-section")).toContainText("Alice Example");
  await page.click("#vcon-reveal-links");
  await expect(page.locator("#vcon-links li")).toHaveCount(1);
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  await expect(page.locator("#vcon-section")).toBeHidden();
  await expect(page.locator("#vcon-section")).toHaveText("");
  await expect(page.locator("#vcon-links li")).toHaveCount(0);
});

test("a vCon that cannot be read in parts says so, and to reveal it whole", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  // The server stores no encrypted vCon made up for a test, so the read a
  // Reveal makes is answered here with a body in the encrypted form.
  const sealed = { uuid, protected: "eA", ciphertext: "Y2lwaGVy", iv: "aXY", tag: "dGFn" };
  await page.route(`**/vcons/${uuid}`, (route) =>
    route.fulfill({ status: 200, contentType: "application/vcon", headers: { ETag: '"sealed"' }, body: JSON.stringify(sealed) }),
  );
  await page.click("#vcon-reveal-parties");
  await expect(page.locator("#vcon-section")).toHaveText("This vCon cannot be read in parts. Reveal the whole vCon instead.");
  await page.click("#vcon-reveal-links");
  await expect(page.locator("#vcon-links-state")).toHaveText("No links in this vCon.");
});

test("Download saves the vCon under its uuid, and is a recorded read", async ({ page, request, tenant }) => {
  const uuid = await opened(page, request, tenant);
  const saving = page.waitForEvent("download");
  await page.click("#vcon-download");
  const saved = await saving;
  expect(saved.suggestedFilename()).toBe(`${uuid}.json`);
  const stream = await saved.createReadStream();
  let text = "";
  for await (const chunk of stream) {
    text += chunk;
  }
  expect(JSON.parse(text).uuid).toBe(uuid);
  await expect(page.locator("#vcon-message")).toHaveText(`Downloaded ${uuid}`);
  const log = await auditLog(request, await tenant.key(["audit"]), "/audit");
  expect(log.filter((r) => r.action === "read" && r.target === uuid && r.status === 200)).toHaveLength(2);
});

test("a vCon past the display limit is cut, and says to download the rest", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  const body = call(uuid, { analysis: [{ type: "transcript", body: "word ".repeat(450000) }] });
  await opened(page, request, tenant, body);
  await page.click("#reveal");
  await expect(page.locator("#vcon-cut")).toBeVisible();
  await expect(page.locator("#vcon-cut")).toContainText("Only the first 2097152 characters are shown");
  await expect(page.locator("#vcon-cut")).toContainText("Download");
  expect(await page.locator("#vcon-body").evaluate((e) => e.textContent.length)).toBe(2097152);
});

test("a vCon within the display limit shows no cut line", async ({ page, request, tenant }) => {
  await opened(page, request, tenant);
  await page.click("#reveal");
  await expect(page.locator("#vcon-body")).toBeVisible();
  await expect(page.locator("#vcon-cut")).toBeHidden();
});
