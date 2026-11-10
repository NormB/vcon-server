// The console meets WCAG 2.2 AA: axe-core's rules on every view, in light
// and dark, on a phone and a desktop, and what axe cannot see: keyboard
// use, where focus goes, a visible focus ring and no sideways scrolling.
import { randomUUID } from "node:crypto";
import AxeBuilder from "@axe-core/playwright";
import { TAGS, describe } from "../../site-tests/tests/axe.js";
import { test, expect, signIn, vcon, store, openView, operatorKey } from "./fixtures.js";

const SCHEMES = ["light", "dark"];
const SIZES = [
  { name: "phone", width: 320, height: 640 },
  { name: "desktop", width: 1280, height: 800 },
];

/** Fails with each rule broken and the element that broke it. */
async function meetsRules(page) {
  const { violations } = await new AxeBuilder({ page }).withTags(TAGS).analyze();
  expect(violations, describe(violations)).toEqual([]);
}

/** Fails when the page scrolls sideways. */
async function fitsItsWidth(page) {
  const [content, width] = await page.evaluate(() => [
    document.documentElement.scrollWidth,
    document.documentElement.clientWidth,
  ]);
  expect(content, "the page is no wider than the window").toBeLessThanOrEqual(width);
}

for (const scheme of SCHEMES) {
  for (const size of SIZES) {
    test.describe(`${scheme}, ${size.name}`, () => {
      test.use({ colorScheme: scheme, viewport: { width: size.width, height: size.height } });

      test("the sign-in page", async ({ page }) => {
        await page.goto("/console/");
        await meetsRules(page);
        await fitsItsWidth(page);
      });

      test("a refused sign-in", async ({ page }) => {
        await signIn(page, "vcs_0000000000000000_not-a-key");
        await expect(page.locator("#sign-in-error")).toHaveText("The key was not accepted.");
        await meetsRules(page);
      });

      test("the vCons view with a vCon opened and revealed", async ({ page, request, tenant }) => {
        const uuid = randomUUID();
        await store(request, await tenant.key(["write"]), vcon(uuid, "a call"));
        await signIn(page, await tenant.key(["read", "write", "delete"]));
        await openView(page, "vcons");
        await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
        await expect(page.locator("#vcon-summary")).toBeVisible();
        await meetsRules(page);
        await page.click("#reveal");
        await expect(page.locator("#vcon-body")).toBeVisible();
        await meetsRules(page);
        await fitsItsWidth(page);
      });

      test("the legal view with a vCon found", async ({ page, request, tenant }) => {
        const uuid = randomUUID();
        await store(request, await tenant.key(["write"]), vcon(uuid, "a call"));
        await signIn(page, await tenant.key(["legal"]));
        await openView(page, "legal");
        await page.fill("#legal-uuid", uuid);
        await page.click("#legal-find button[type=submit]");
        await expect(page.locator("#legal-vcon")).toBeVisible();
        await page.locator("#versions button.open-version").first().click();
        await expect(page.locator("#version-summary")).toBeVisible();
        await meetsRules(page);
        await fitsItsWidth(page);
      });

      test("the audit view", async ({ page, request, tenant }) => {
        await store(request, await tenant.key(["write"]), vcon(randomUUID(), "a call"));
        await signIn(page, await tenant.key(["audit"]));
        await openView(page, "audit");
        await expect(page.locator("#audit-log tbody tr").first()).toBeVisible();
        await meetsRules(page);
        await fitsItsWidth(page);
      });

      test("the tenants view with a tenant's keys", async ({ page, tenant }) => {
        await tenant.key(["read"]);
        await signIn(page, operatorKey);
        await openView(page, "tenants");
        await page.locator(`#tenants tbody tr:has-text("${tenant.id}") button.keys`).click();
        await expect(page.locator("#key-list tbody tr").first()).toBeVisible();
        await meetsRules(page);
        await fitsItsWidth(page);
      });
    });
  }
}

test("the first Tab stop is a link that skips to the content", async ({ page }) => {
  await page.goto("/console/");
  await page.keyboard.press("Tab");
  const first = page.locator(":focus");
  await expect(first).toHaveText("Skip to the content");
  await expect(first).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(page.locator("#main")).toBeFocused();
});

test("signing in and opening a view needs only the keyboard", async ({ page, tenant }) => {
  const token = await tenant.key(["read"]);
  await page.goto("/console/");
  await page.locator("#key").focus();
  await page.keyboard.type(token);
  await page.keyboard.press("Enter");
  await expect(page.locator("#key-info")).toBeVisible();
  await page.locator('#views button[data-view="vcons"]').focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#view h2")).toBeFocused();
  await expect(page.locator("#view h2")).toHaveText("vCons");
});

test("the view shown is marked as the current one, in text and not by color alone", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read", "audit"]));
  await openView(page, "vcons");
  await expect(page.locator('#views button[data-view="vcons"]')).toHaveAttribute("aria-current", "page");
  await expect(page.locator('#views button[data-view="audit"]')).not.toHaveAttribute("aria-current", "page");
  await openView(page, "audit");
  await expect(page.locator('#views button[data-view="audit"]')).toHaveAttribute("aria-current", "page");
  await expect(page.locator('#views button[data-view="vcons"]')).not.toHaveAttribute("aria-current", "page");
});

test("a control focused from the keyboard shows a focus ring", async ({ page }) => {
  await page.goto("/console/");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  await expect(page.locator("#key")).toBeFocused();
  const ring = await page.locator("#key").evaluate((e) => {
    const s = getComputedStyle(e);
    return { style: s.outlineStyle, width: parseFloat(s.outlineWidth) };
  });
  expect(ring.style).toBe("solid");
  expect(ring.width).toBeGreaterThanOrEqual(2);
});

test("the page has one main landmark, a labeled navigation and one h1", async ({ page, tenant }) => {
  await signIn(page, await tenant.key(["read"]));
  await expect(page.locator("main")).toHaveCount(1);
  await expect(page.locator("h1")).toHaveCount(1);
  await expect(page.locator("nav#views")).toHaveAttribute("aria-label", "Views");
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
});

test("the page says its times are in UTC", async ({ page }) => {
  await page.goto("/console/");
  await expect(page.locator("footer")).toContainText("UTC (offset +00:00)");
});

test("Reveal moves focus to the revealed text", async ({ page, request, tenant }) => {
  const uuid = randomUUID();
  await store(request, await tenant.key(["write"]), vcon(uuid, "a call"));
  await signIn(page, await tenant.key(["read"]));
  await openView(page, "vcons");
  await page.click(`#vcon-list tbody tr:has-text("${uuid}") button.open`);
  await expect(page.locator("#reveal")).toBeVisible();
  await page.locator("#reveal").focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#vcon-body")).toBeFocused();
});
