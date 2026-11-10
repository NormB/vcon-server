// The console on a server with an operator listener: each address serves
// one kind of key, and a key of the other kind is told to use the other
// address. scripts/test-console.sh starts that second server and passes
// its two addresses, an operator key and a tenant key.
import { test as base, expect } from "@playwright/test";
import { stored, nothingStored } from "./fixtures.js";

const tenantUrl = process.env.VCON_SPLIT_TENANT_URL;
const operatorUrl = process.env.VCON_SPLIT_OPERATOR_URL;
const operatorKey = process.env.VCON_SPLIT_OPERATOR_KEY;
const tenantKey = process.env.VCON_SPLIT_TENANT_KEY;

const ELSEWHERE =
  "This listener does not serve this kind of key (403). " +
  "Open the console at the server's other address and sign in there: " +
  "one address serves tenant keys and the other serves operator keys.";

/** The page, failing the test on any content-security violation. */
const test = base.extend({
  page: async ({ page }, use) => {
    const violations = [];
    await page.exposeFunction("reportViolation", (v) => violations.push(v));
    await page.addInitScript(() => {
      window.addEventListener("securitypolicyviolation", (e) =>
        window.reportViolation(`${e.violatedDirective} ${e.blockedURI}`),
      );
    });
    await use(page);
    expect(violations, "content-security violations").toEqual([]);
  },
});

/** Opens the console at `origin` and signs in with `token`. */
async function signInAt(page, origin, token) {
  await page.goto(`${origin}/console/`);
  await page.fill("#key", token);
  await page.click("#sign-in button[type=submit]");
}

test("the run was given both addresses and both keys", () => {
  for (const value of [tenantUrl, operatorUrl, operatorKey, tenantKey]) {
    expect(value, "set by scripts/test-console.sh").toBeTruthy();
  }
  expect(tenantUrl).not.toBe(operatorUrl);
});

test("a tenant key signs in at the tenant address", async ({ page }) => {
  await signInAt(page, tenantUrl, tenantKey);
  await expect(page.locator("#key-info")).toBeVisible();
  await expect(page.locator("#key-fields")).toContainText("read");
  await expect(page.locator("#views button")).toHaveText(["vCons"]);
});

test("an operator key signs in at the operator address and sees its views", async ({ page }) => {
  await signInAt(page, operatorUrl, operatorKey);
  await expect(page.locator("#key-info")).toBeVisible();
  await expect(page.locator("#key-fields")).toContainText("none (operator key)");
  await expect(page.locator("#views button")).toHaveText(["Audit log", "Tenants"]);
});

test("an operator key at the operator address lists the tenants", async ({ page }) => {
  await signInAt(page, operatorUrl, operatorKey);
  await page.click("#views button[data-view=tenants]");
  await expect(page.locator("#tenants tbody tr")).not.toHaveCount(0);
  await expect(page.locator("#tenant-message")).toHaveText("");
});

test("an operator key at the tenant address is told to use the other address", async ({ page }) => {
  await signInAt(page, tenantUrl, operatorKey);
  await expect(page.locator("#sign-in-error")).toHaveText(ELSEWHERE);
  await expect(page.locator("#key-info")).toBeHidden();
  await expect(page.locator("#views")).toBeHidden();
  await expect(page.locator("#sign-in")).toBeVisible();
});

test("a tenant key at the operator address is told to use the other address", async ({ page }) => {
  await signInAt(page, operatorUrl, tenantKey);
  await expect(page.locator("#sign-in-error")).toHaveText(ELSEWHERE);
  await expect(page.locator("#key-info")).toBeHidden();
  await expect(page.locator("#sign-in")).toBeVisible();
});

test("a key refused for its listener is not kept by the page", async ({ page }) => {
  await signInAt(page, tenantUrl, operatorKey);
  await expect(page.locator("#sign-in-error")).toHaveText(ELSEWHERE);
  await expect(page.locator("#key")).toHaveValue("");
  expect(await stored(page)).toEqual(nothingStored);
  // The page holds no key: a request it makes now carries none.
  const status = await page.evaluate(async () => {
    const { request } = await import("./console.js");
    return (await request("GET", "/key")).status;
  });
  expect(status).toBe(401);
});

test("the refusal is announced where a refused key is announced", async ({ page }) => {
  await signInAt(page, tenantUrl, operatorKey);
  const message = page.locator("#sign-in-error");
  await expect(message).toHaveText(ELSEWHERE);
  await expect(message).toHaveAttribute("role", "alert");
});

test("after the refusal the right kind of key signs in on the same page", async ({ page }) => {
  await signInAt(page, tenantUrl, operatorKey);
  await expect(page.locator("#sign-in-error")).toHaveText(ELSEWHERE);
  await page.fill("#key", tenantKey);
  await page.click("#sign-in button[type=submit]");
  await expect(page.locator("#key-info")).toBeVisible();
  // The refusal's text goes out of sight with the sign-in form.
  await expect(page.locator("#sign-in-error")).toBeHidden();
});

test("a key no listener accepts is refused the same way at both addresses", async ({ page }) => {
  for (const origin of [tenantUrl, operatorUrl]) {
    await signInAt(page, origin, "vcs_0123456789abcdef_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    await expect(page.locator("#sign-in-error")).toHaveText("The key was not accepted.");
  }
});

test("both addresses serve the same console files", async ({ request }) => {
  for (const file of ["", "console.js", "format.js", "console.css", "player.html", "player.js"]) {
    const [a, b] = await Promise.all([
      request.get(`${tenantUrl}/console/${file}`),
      request.get(`${operatorUrl}/console/${file}`),
    ]);
    expect(a.status(), file).toBe(200);
    expect(b.status(), file).toBe(200);
    expect(await a.text(), file).toBe(await b.text());
    expect(a.headers()["content-security-policy"], file).toBe(b.headers()["content-security-policy"]);
  }
});
