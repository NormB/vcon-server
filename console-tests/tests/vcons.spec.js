// The vCons view: list, read and store, and delete, each shown only to a
// key whose scopes allow it; stored text is shown as text.
import { randomUUID } from "node:crypto";
import { gzipSync } from "node:zlib";
import { test, expect, signIn, stored, nothingStored, vcon, store, openView } from "./fixtures.js";

test("a read key lists the tenant's vCons, newest first, a page at a time", async ({ page, request, tenant }) => {
  const writer = await tenant.key(["write"]);
  const uuids = [];
  for (let i = 0; i < 27; i++) {
    const uuid = randomUUID();
    uuids.push(uuid);
    await store(request, writer, vcon(uuid, `call ${i}`));
  }
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  const rows = page.locator("#vcon-list tbody tr");
  await expect(rows).toHaveCount(25);
  await page.click("#more");
  await expect(rows).toHaveCount(27);
  await expect(page.locator("#more")).toBeHidden();
  const listed = await page.locator("#vcon-list tbody tr td:first-child").allTextContents();
  expect(new Set(listed)).toEqual(new Set(uuids));
  expect(listed[0]).toBe(uuids[26]);
  expect(await stored(page)).toEqual(nothingStored);
});

test("a stored vCon is shown as text: its markup never runs", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  const subject = '<img src=x onerror="window.ran=1"><script>window.ran=2</script><b>bold</b>';
  await store(request, await tenant.key(["write"]), vcon(uuid, subject));
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  const body = page.locator("#vcon-body");
  await expect(body).toContainText(JSON.stringify(subject).slice(1, -1));
  expect(await body.locator("*").count(), "no element made from the text").toBe(0);
  expect(await page.locator("#view img, #view script:not([src]), #view b").count()).toBe(0);
  expect(await page.evaluate(() => window.ran)).toBeUndefined();
  await expect(page.locator("#vcon-etag")).toHaveText(/^".+"$/);
});

test("a gzipped vCon is shown inflated", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), gzipSync(vcon(uuid, "zipped call")), "application/vcon+gzip");
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  await page.fill("#uuid", uuid);
  await page.click("#open-uuid");
  await expect(page.locator("#vcon-body")).toContainText('"subject":"zipped call"');
});

test("a read-only key gets no write or delete control, and the API refuses one", async ({ page, request, tenant }) => {
  const reader = await tenant.key(["read"]);
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), vcon(uuid, "kept"));
  await signIn(page, reader);
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  await expect(page.locator("#vcon-body")).toContainText("kept");
  await expect(page.locator("#store")).toHaveCount(0);
  await expect(page.locator("#delete")).toHaveCount(0);
  await expect(page.locator("#delete-uuid")).toHaveCount(0);
  const post = await request.post("/vcons", {
    headers: { Authorization: `Bearer ${reader}`, "Content-Type": "application/vcon" },
    data: vcon(randomUUID(), "refused"),
  });
  expect(post.status()).toBe(403);
  const del = await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${reader}` } });
  expect(del.status()).toBe(403);
});

test("a write key stores a vCon from a file and sees no list", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await signIn(page, await tenant.key(["write"]));
  await openView(page, "vcons");
  await expect(page.locator("#vcon-list")).toHaveCount(0);
  await page.setInputFiles("#store-file", { name: "call.vcon", mimeType: "application/vcon", buffer: vcon(uuid, "from a file") });
  await page.click("#store button[type=submit]");
  await expect(page.locator("#store-result")).toContainText(`Stored ${uuid}`);
  const read = await request.get(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${await tenant.key(["read"])}` } });
  expect(read.status()).toBe(200);
  expect(await read.text()).toContain("from a file");
});

test("a refused store shows the problem as text", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["write"]));
  await openView(page, "vcons");
  await page.setInputFiles("#store-file", { name: "bad.vcon", mimeType: "application/vcon", buffer: Buffer.from("<b>not json</b>") });
  await page.click("#store button[type=submit]");
  await expect(page.locator("#store-result")).toHaveText("The body is not one JSON object (422)");
  expect(await page.locator("#store-result *").count()).toBe(0);
});

test("a delete key deletes a vCon after the user confirms", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), vcon(uuid, "to go"));
  await signIn(page, await tenant.key(["read", "delete"]));
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  page.once("dialog", (d) => d.dismiss());
  await page.click("#delete");
  const reader = await tenant.key(["read"]);
  let read = await request.get(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${reader}` } });
  expect(read.status(), "dismissed: still there").toBe(200);
  page.once("dialog", (d) => d.accept());
  await page.click("#delete");
  await expect(page.locator("#vcon-message")).toContainText(`Deleted ${uuid}`);
  read = await request.get(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${reader}` } });
  expect(read.status()).toBe(410);
});

test("a legal key lists a vCon's versions and reads an earlier one", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  const writer = await tenant.key(["write"]);
  const first = await store(request, writer, vcon(uuid, "first words"));
  const replaced = await request.put(`/vcons/${uuid}`, {
    headers: { Authorization: `Bearer ${writer}`, "If-Match": first, "Content-Type": "application/vcon" },
    data: vcon(uuid, "second words"),
  });
  expect(replaced.status()).toBe(200);
  await signIn(page, await tenant.key(["legal"]));
  await openView(page, "legal");
  await page.fill("#legal-uuid", uuid);
  await page.click("#legal-find button[type=submit]");
  await expect(page.locator("#legal-state")).toHaveText("live");
  await expect(page.locator("#legal-body")).toContainText("second words");
  const versions = page.locator("#versions tbody tr");
  await expect(versions).toHaveCount(2);
  await versions.nth(0).locator("button.open-version").click();
  await expect(page.locator("#version-body")).toContainText("first words");
  expect(await stored(page)).toEqual(nothingStored);
});
