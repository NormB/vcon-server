// The console opens a vCon with the server's masked view
// (`?view=masked`, src/masked.rs), so the browser receives no name, number,
// subject or content until a Reveal asks for a part. The tenant view's
// open is recorded as `read_masked`, the Legal view's as
// `legal_read_masked`; Reveal stays `?reveal=<part>` and Download the
// plain read.
import { readFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { test, expect, signIn, store, openView, auditLog, inline, wav } from "./fixtures.js";

/** The cases the server's rule is tested against (src/masked.rs). */
const MASKED_CASES = JSON.parse(
  readFileSync(new URL("../../crates/vcon-server/tests/data/masked-cases.json", import.meta.url), "utf8"),
);

/** What no masked page may show: the vCon's names, number, subject and text. */
const HIDDEN = ["Alice", "Bob Example", "Billing question", "5555550123", "bob@", "hello there"];

/** An unsigned vCon with two parties and one text dialog, as an object. */
function call(uuid, extra = {}) {
  return {
    uuid,
    created_at: "2026-10-23T12:00:00Z",
    subject: "Billing question",
    parties: [
      { tel: "+15555550123", name: "Alice Example" },
      { mailto: "bob@example.com", name: "Bob Example" },
    ],
    dialog: [{ type: "text", start: "2026-10-23T12:00:00Z", parties: [0, 1], body: "hello there", duration: 30 }],
    ...extra,
  };
}

/** Stores `vcon` with a new write key; its tag. */
async function stored(request, tenant, vcon) {
  return store(request, await tenant.key(["write"]), Buffer.from(JSON.stringify(vcon)));
}

/**
 * Every request the page sends for one vCon's path, with its query, in the
 * order they were sent, and a promise of the text of each answer. Each is
 * listed when it is sent, so a check made once the page shows the answer
 * finds it there.
 */
function watched(page, uuid) {
  const seen = [];
  page.on("request", (asked) => {
    const url = new URL(asked.url());
    if (url.pathname.endsWith(`/vcons/${uuid}`) || url.pathname.includes(`/vcons/${uuid}/versions/`)) {
      const text = asked
        .response()
        .then((answer) => (answer === null ? "" : answer.text()))
        .catch(() => "");
      seen.push({ path: url.pathname, query: url.search, text });
    }
  });
  return seen;
}

/** Opens `uuid` in the vCons view by the uuid form. */
async function openByUuid(page, uuid) {
  await openView(page, "vcons");
  await page.fill("#uuid", uuid);
  await page.click("#open-uuid");
}

/** Finds `uuid` in the Legal view. */
async function find(page, uuid) {
  await openView(page, "legal");
  await page.fill("#legal-uuid", uuid);
  await page.click("#legal-find button[type=submit]");
}

/** The actions of the tenant's records of `uuid` that answered 200. */
async function actions(request, tenant, uuid) {
  const log = await auditLog(request, await tenant.key(["audit"]), "/audit");
  return log.filter((r) => r.target === uuid && r.status === 200).map((r) => r.action);
}

/**
 * The terms and values the console shows for a summary, in order: the
 * display rule of showSummary in console.js, stated for the cases.
 */
function rows(s) {
  const made = [
    ["Form", s.form],
    ["Created", s.created === "" ? "not stated" : s.created],
    ["Subject", s.subject],
    ["Dialogs", String(s.dialogs)],
    ["Dialog seconds", String(s.seconds)],
    ["Attachments", String(s.attachments)],
    ["Analyses", String(s.analyses)],
    ["Links outside the server", String(s.links)],
  ];
  if (s.signature !== null) {
    for (const name of ["alg", "kid", "x5t"]) {
      made.push([`Signature ${name}`, s.signature[name] === "" ? "not stated" : s.signature[name]]);
    }
    made.push(["Signature check", "not verified by the console"]);
  }
  made.push(["Parties", s.parties.length === 0 ? "none" : s.parties.join("\n")]);
  made.push(["Dialog items", s.dialog.length === 0 ? "none" : s.dialog.join("\n")]);
  return made;
}

/** The terms and values a summary list shows. */
async function shownRows(list) {
  return list.evaluate((dl) => {
    const made = [];
    for (const dt of dl.querySelectorAll("dt")) {
      const dd = dt.nextElementSibling;
      const items = [...dd.querySelectorAll("li")].map((li) => li.textContent);
      made.push([dt.textContent, items.length === 0 ? dd.textContent : items.join("\n")]);
    }
    return made;
  });
}

/** Answers the page's masked view of any vCon with `summary`. */
async function answerMasked(page, summary, status = 200, type = "application/json") {
  await page.route(
    (url) => url.searchParams.get("view") === "masked",
    (route) =>
      route.fulfill({
        status,
        contentType: type,
        headers: { ETag: '"0199e5c6-0d7a-7c11-8000-000000000001"' },
        body: typeof summary === "string" ? summary : JSON.stringify(summary),
      }),
  );
}

// Success.

test("opening a vCon asks the server for its masked view", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  const seen = watched(page, uuid);
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await expect(page.locator("#vcon-summary")).toContainText("Party 1: number ending 0123; name hidden");
  expect(seen.map((s) => s.query)).toEqual(["?view=masked"]);
});

test("the browser receives none of the vCon's content when it opens one", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  const seen = watched(page, uuid);
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await expect(page.locator("#vcon-summary")).toContainText("present, hidden");
  expect(seen.length).toBe(1);
  const text = await seen[0].text;
  expect(text).toContain("present, hidden");
  for (const hidden of HIDDEN) {
    expect(text, hidden).not.toContain(hidden);
  }
});

test("an opened vCon is recorded as read_masked and a reveal as the reveal", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await page.click("#vcon-reveal-subject");
  await expect(page.locator("#vcon-section")).toContainText("Billing question");
  expect(await actions(request, tenant, uuid)).toEqual(["read_masked", "reveal_subject"]);
});

test("the Legal view finds a vCon by its masked view, recorded as legal_read_masked", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  const seen = watched(page, uuid);
  await signIn(page, await tenant.key(["legal"]));
  await find(page, uuid);
  await expect(page.locator("#legal-summary")).toContainText("Party 2: address at example.com; name hidden");
  await expect(page.locator("#legal-state")).toHaveText("live");
  expect(seen.filter((s) => s.path.endsWith(uuid)).map((s) => s.query)).toEqual(["?view=masked"]);
  expect(await actions(request, tenant, uuid)).toEqual(["legal_read_masked", "legal_versions"]);
});

test("a kept version opens by its masked view", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  const seen = watched(page, uuid);
  await signIn(page, await tenant.key(["legal"]));
  await find(page, uuid);
  await page.locator("#versions button.open-version").first().click();
  await expect(page.locator("#version-summary")).toContainText("Party 1: number ending 0123; name hidden");
  const version = seen.filter((s) => s.path.includes("/versions/"));
  expect(version.map((s) => s.query)).toEqual(["?view=masked"]);
  const text = await version[0].text;
  expect(text).toContain("name hidden");
  for (const hidden of HIDDEN) {
    expect(text, hidden).not.toContain(hidden);
  }
  expect(await actions(request, tenant, uuid)).toEqual(["legal_read_masked", "legal_versions", "legal_read_masked"]);
});

test("a deleted vCon found in the Legal view says so, masked", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${await tenant.key(["delete"])}` } });
  await signIn(page, await tenant.key(["legal"]));
  await find(page, uuid);
  await expect(page.locator("#legal-state")).toHaveText("deleted");
  await expect(page.locator("#legal-summary")).toContainText("name hidden");
  expect(await page.locator("#view").innerText()).not.toContain("Alice");
});

test("a dialog with a recording the server lists has a Play button", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid, { dialog: [{ type: "text", body: "hi" }, inline(wav())] }));
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await expect(page.locator("#vcon-recordings button")).toHaveCount(1);
  await expect(page.locator("#vcon-play-2")).toBeVisible();
});

test("the version shown is the one the masked view's tag names", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  const tag = await stored(request, tenant, call(uuid));
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await expect(page.locator("#vcon-etag")).toHaveText(tag);
});

test.describe("the console shows each case of the server's rule as the server answers it", () => {
  test("there are cases to show", () => {
    expect(MASKED_CASES.length).toBeGreaterThan(20);
  });
  for (const { name, summary } of MASKED_CASES) {
    test(`shown case: ${name}`, async ({ page, tenant }) => {
      await answerMasked(page, summary);
      await signIn(page, await tenant.key(["read"]));
      await openByUuid(page, randomUUID());
      const list = page.locator("#vcon-summary");
      await expect(list.locator("dt").first()).toHaveText("Form");
      expect(await shownRows(list)).toEqual(rows(summary));
      const plays = await page.locator("#vcon-recordings button").evaluateAll((all) => all.map((b) => b.id));
      expect(plays).toEqual(summary.recordings.map((r) => `vcon-play-${r.dialog}`));
    });
  }
});

// Failure.

test("a vCon the tenant does not hold is refused and nothing is shown", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, randomUUID());
  await expect(page.locator("#vcon-message")).toContainText("(404)");
  await expect(page.locator("#vcon")).toBeHidden();
});

test("a deleted vCon's masked view is refused in the vCons view", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  await request.delete(`/vcons/${uuid}`, { headers: { Authorization: `Bearer ${await tenant.key(["delete"])}` } });
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await expect(page.locator("#vcon-message")).toContainText("(410)");
  await expect(page.locator("#vcon")).toBeHidden();
});

test("a uuid the Legal view cannot find is refused and nothing is shown", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["legal"]));
  await find(page, randomUUID());
  await expect(page.locator("#legal-message")).toContainText("(404)");
  await expect(page.locator("#legal-vcon")).toBeHidden();
});

/** The errors the page throws, listed as they are thrown. */
function thrown(page) {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  return errors;
}

test("a masked view that is not JSON is told and nothing is shown", async ({ page, tenant }) => {
  const errors = thrown(page);
  await answerMasked(page, "not JSON");
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, randomUUID());
  await expect(page.locator("#vcon-message")).toHaveText(
    "The server's masked view of this vCon could not be read. Open it again; if this repeats, tell the server's operator.",
  );
  await expect(page.locator("#vcon")).toBeHidden();
  expect(errors).toEqual([]);
});

test("a masked view without the summary's fields is told and nothing is shown", async ({ page, tenant }) => {
  const errors = thrown(page);
  await answerMasked(page, { form: "unsigned" });
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, randomUUID());
  await expect(page.locator("#vcon-message")).toContainText("could not be read");
  await expect(page.locator("#vcon")).toBeHidden();
  expect(errors).toEqual([]);
});

test("a masked view the Legal view cannot read is told and nothing is shown", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  const errors = thrown(page);
  await answerMasked(page, "[]");
  await signIn(page, await tenant.key(["legal"]));
  await find(page, uuid);
  await expect(page.locator("#legal-message")).toContainText("could not be read");
  await expect(page.locator("#legal-vcon")).toBeHidden();
  expect(errors).toEqual([]);
});

test("a key without read cannot open a vCon from the uuid form", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  await signIn(page, await tenant.key(["delete"]));
  await openView(page, "vcons");
  await expect(page.locator("#open-uuid")).toHaveCount(0);
});

test("a recording the server does not list has no Play button", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid, { dialog: [inline(wav(), "video/mp4")] }));
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await expect(page.locator("#vcon-summary")).toContainText("Dialog 1: recording");
  await expect(page.locator("#vcon-recordings button")).toHaveCount(0);
});

// Edge.

test("Reveal reads the part and Download the whole vCon", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  const seen = watched(page, uuid);
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await expect(page.locator("#vcon-summary")).toContainText("name hidden");
  await page.click("#vcon-reveal-parties");
  await expect(page.locator("#vcon-section")).toContainText("Alice Example");
  const download = page.waitForEvent("download");
  await page.click("#vcon-download");
  await download;
  expect(seen.map((s) => s.query)).toEqual(["?view=masked", "?reveal=parties", ""]);
  expect(await actions(request, tenant, uuid)).toEqual(["read_masked", "reveal_parties", "read"]);
});

test("a Legal reveal and Download read the version, not the masked view", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  const seen = watched(page, uuid);
  await signIn(page, await tenant.key(["legal"]));
  await find(page, uuid);
  await page.click("#legal-reveal");
  await expect(page.locator("#legal-body")).toContainText("Alice Example");
  const download = page.waitForEvent("download");
  await page.click("#legal-download");
  await download;
  expect(seen.filter((s) => s.path.endsWith(uuid)).map((s) => s.query)).toEqual(["?view=masked", "?reveal=all", ""]);
});

test("text in a summary is shown as text, never as markup", async ({ page, tenant }) => {
  const summary = structuredClone(MASKED_CASES.find((c) => c.summary.signature !== null).summary);
  summary.signature.kid = '<img src="x" id="injected">';
  await answerMasked(page, summary);
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, randomUUID());
  await expect(page.locator("#vcon-summary")).toContainText('<img src="x" id="injected">');
  await expect(page.locator("#injected")).toHaveCount(0);
});

test("a summary from an older server without recordings shows no Play button", async ({ page, tenant }) => {
  const summary = structuredClone(MASKED_CASES[0].summary);
  delete summary.recordings;
  await answerMasked(page, summary);
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, randomUUID());
  await expect(page.locator("#vcon-summary")).toContainText("Party 1");
  await expect(page.locator("#vcon-recordings button")).toHaveCount(0);
});

test("opening a second vCon replaces the first one's summary and buttons", async ({ page, request, tenant }) => {
  const first = randomUUID();
  const second = randomUUID();
  await stored(request, tenant, call(first, { dialog: [inline(wav())] }));
  await stored(request, tenant, call(second));
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, first);
  await expect(page.locator("#vcon-recordings button")).toHaveCount(1);
  await page.click("#vcon-reveal-parties");
  await expect(page.locator("#vcon-section")).toBeVisible();
  await openByUuid(page, second);
  await expect(page.locator("#vcon-summary")).toContainText("Dialog 1: text");
  await expect(page.locator("#vcon-recordings button")).toHaveCount(0);
  await expect(page.locator("#vcon-section")).toBeHidden();
});

test("a gzipped vCon is opened by the same masked view", async ({ page, request, tenant }) => {
  const { gzipSync } = await import("node:zlib");
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), gzipSync(Buffer.from(JSON.stringify(call(uuid)))), "application/vcon+gzip");
  const seen = watched(page, uuid);
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  await expect(page.locator("#vcon-summary")).toContainText("Party 1: number ending 0123; name hidden");
  expect(seen.map((s) => s.query)).toEqual(["?view=masked"]);
});

test("a uuid with spaces around it is opened by its masked view", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid));
  const seen = watched(page, uuid);
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, `  ${uuid} `);
  await expect(page.locator("#vcon-summary")).toContainText("present, hidden");
  expect(seen.map((s) => s.query)).toEqual(["?view=masked"]);
});

test("the Play button of a listed recording plays it after a reveal of audio", async ({ page, request, tenant, context }) => {
  const uuid = randomUUID();
  await stored(request, tenant, call(uuid, { dialog: [inline(wav())] }));
  await signIn(page, await tenant.key(["read"]));
  await openByUuid(page, uuid);
  const tab = context.waitForEvent("page");
  await page.click("#vcon-play-1");
  await tab;
  await expect(page.locator("#vcon-recordings-state")).toHaveText("Dialog 1's recording was handed to the player's tab.");
  expect(await actions(request, tenant, uuid)).toEqual(["read_masked", "reveal_audio"]);
});
