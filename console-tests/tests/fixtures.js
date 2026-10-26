// What every console test shares: a tenant of its own and keys for it,
// made through the admin API with the operator key the run was given, and
// checks that the page broke no content-security rule and kept nothing in
// the browser.
import { test as base, expect } from "@playwright/test";

const operator = process.env.VCON_OPERATOR_KEY;

/** One admin API request with the operator key. */
async function admin(request, method, path, data) {
  const answer = await request.fetch(path, {
    method,
    headers: { Authorization: `Bearer ${operator}` },
    data,
  });
  expect(answer.ok(), `${method} ${path}: ${answer.status()}`).toBeTruthy();
  return answer.status() === 204 ? null : answer.json();
}

export const test = base.extend({
  /** A new tenant for this test, and a function that makes keys for it. */
  tenant: async ({ request }, use, info) => {
    const name = `console-${info.testId}-${Date.now()}`;
    const { id } = await admin(request, "POST", "/admin/tenants", { name });
    const key = async (scopes) =>
      (await admin(request, "POST", `/admin/tenants/${id}/keys`, { scopes, label: scopes.join("+") }))
        .token;
    await use({ id, key, admin: (method, path, data) => admin(request, method, path, data) });
  },

  /** The page, failing the test on any content-security violation. */
  page: async ({ page }, use) => {
    const violations = [];
    await page.exposeFunction("reportViolation", (v) => violations.push(v));
    await page.addInitScript(() => {
      document.addEventListener("securitypolicyviolation", (e) =>
        window.reportViolation(`${e.violatedDirective} ${e.blockedURI}`),
      );
    });
    await use(page);
    expect(violations, "content-security violations").toEqual([]);
  },
});

export { expect };

/** Opens the console and signs in with `token`. */
export async function signIn(page, token) {
  await page.goto("/console/");
  await page.fill("#key", token);
  await page.click("#sign-in button[type=submit]");
}

/** Everything the page could have kept in the browser: all must be empty. */
export async function stored(page) {
  return page.evaluate(async () => ({
    localStorage: localStorage.length,
    sessionStorage: sessionStorage.length,
    indexedDB: (await indexedDB.databases()).length,
    caches: (await caches.keys()).length,
    cookie: document.cookie,
    // window.name and the history entry's state survive a reload.
    windowName: window.name,
    historyState: history.state,
  }));
}

export const nothingStored = {
  localStorage: 0,
  sessionStorage: 0,
  indexedDB: 0,
  caches: 0,
  cookie: "",
  windowName: "",
  historyState: null,
};

/** A small unsigned vCon with this uuid and subject, as bytes. */
export function vcon(uuid, subject) {
  const body = { uuid, created_at: "2026-10-23T12:00:00Z", subject, parties: [] };
  return Buffer.from(JSON.stringify(body));
}

/** Stores a vCon through the API with `token`, outside the page. */
export async function store(request, token, body, contentType = "application/vcon") {
  const answer = await request.post("/vcons", {
    headers: { Authorization: `Bearer ${token}`, "Content-Type": contentType },
    data: body,
  });
  expect(answer.status(), `POST /vcons: ${await answer.text()}`).toBe(201);
  return answer.headers()["etag"];
}

/** Opens a view of the signed-in console by its navigation button. */
export async function openView(page, name) {
  await page.click(`#views button[data-view="${name}"]`);
}

/** The operator key the run was given. */
export const operatorKey = operator;

/** Every record of a log, read through the API a page at a time. */
export async function auditLog(request, token, path) {
  const records = [];
  let after = 0;
  for (;;) {
    const answer = await request.get(`${path}?after=${after}&limit=1000`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    expect(answer.status(), `${path}: ${answer.status()}`).toBe(200);
    const lines = (await answer.text()).split("\n").filter((l) => l !== "");
    records.push(...lines.map((l) => JSON.parse(l)));
    if (lines.length < 1000) {
      return records;
    }
    after = records[records.length - 1].seq;
  }
}

/** What a record says was done, without where it sits in its log or when. */
export function done(record) {
  return {
    actor: record.actor,
    action: record.action,
    status: record.status,
    version: record.version === null ? null : "a version",
    count: record.count,
  };
}
