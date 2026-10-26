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
