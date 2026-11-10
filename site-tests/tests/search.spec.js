// The docs search: labeled, reachable by keyboard, and its outcome is a
// status message a screen reader announces without moving focus (WCAG 4.1.3).
// The search index is the site's largest file by far, so a page fetches it
// only once the search box is used.
import { test, expect } from "@playwright/test";
import { borderContrast } from "./axe.js";

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/docs/");
});

test("the search box has a label", async ({ page }) => {
  await expect(page.getByRole("searchbox", { name: "Search the docs" })).toBeVisible();
});

test("the number of results is announced as a status", async ({ page }) => {
  const status = page.getByRole("status");
  await expect(status).toHaveText("");
  await page.getByRole("searchbox", { name: "Search the docs" }).fill("tenant");
  await expect(page.locator("#search-results a").first()).toBeVisible();
  const shown = await page.locator("#search-results a").count();
  expect(shown).toBeGreaterThan(0);
  await expect(status).toHaveText(`${shown} ${shown === 1 ? "result" : "results"} for tenant`);
});

test("no match is announced as a status", async ({ page }) => {
  await page.getByRole("searchbox", { name: "Search the docs" }).fill("qqzzxxj");
  await expect(page.getByRole("status")).toHaveText("No match for qqzzxxj");
});

test("clearing the search clears the status", async ({ page }) => {
  const box = page.getByRole("searchbox", { name: "Search the docs" });
  await box.fill("tenant");
  await expect(page.locator("#search-results a").first()).toBeVisible();
  await box.press("Escape");
  await expect(page.getByRole("status")).toHaveText("");
  await expect(page.locator("#search-results")).toBeHidden();
});

test("the results come next in the keyboard order", async ({ page }) => {
  const box = page.getByRole("searchbox", { name: "Search the docs" });
  await box.fill("tenant");
  await expect(page.locator("#search-results a").first()).toBeVisible();
  await box.press("Tab");
  await expect(page.locator("#search-results a").first()).toBeFocused();
});

// The two files the search needs: the library and the index Zola builds.
const SEARCH_FILES = /\/(elasticlunr\.min|search_index\.en)\.js(\?|$)/;

for (const path of ["/", "/docs/", "/docs/install/", "/problems/"]) {
  test(`${path} does not fetch the search files until the search box is used`, async ({ page }) => {
    const fetched = [];
    page.on("request", (r) => {
      if (SEARCH_FILES.test(r.url())) fetched.push(new URL(r.url()).pathname);
    });
    await page.goto(path, { waitUntil: "networkidle" });
    const box = page.getByRole("searchbox", { name: "Search the docs" });
    await expect(box).toBeVisible();
    expect(fetched, "fetched before the search box was used").toEqual([]);
    await box.focus();
    await box.fill("tenant");
    await expect(page.locator("#search-results a").first()).toBeVisible();
    expect(fetched.sort(), "fetched once each, when the box was used").toEqual([
      "/elasticlunr.min.js",
      "/search_index.en.js",
    ]);
  });
}

test("the search files are fetched once, however often the box is used", async ({ page }) => {
  const fetched = [];
  page.on("request", (r) => {
    if (SEARCH_FILES.test(r.url())) fetched.push(new URL(r.url()).pathname);
  });
  await page.reload({ waitUntil: "networkidle" });
  const box = page.getByRole("searchbox", { name: "Search the docs" });
  await box.fill("tenant");
  await expect(page.locator("#search-results a").first()).toBeVisible();
  await page.locator("h1").click();
  await box.focus();
  await box.fill("retention");
  await expect(page.getByRole("status")).toHaveText(/results? for retention$/);
  expect(fetched.length, fetched.join(", ")).toBe(2);
});

test("text typed before the index arrives is searched when it does", async ({ page }) => {
  let release;
  const held = new Promise((resolve) => {
    release = resolve;
  });
  await page.route(/search_index\.en\.js/, async (route) => {
    await held;
    await route.continue();
  });
  await page.reload();
  const box = page.getByRole("searchbox", { name: "Search the docs" });
  await box.fill("tenant");
  await expect(page.locator("#search-results")).toBeHidden();
  await expect(page.getByRole("status")).toHaveText("");
  release();
  await expect(page.locator("#search-results a").first()).toBeVisible();
  await expect(page.getByRole("status")).toHaveText(/results? for tenant$/);
});

test("a search file that cannot be fetched is announced, and the next use tries again", async ({ page }) => {
  let refuse = true;
  await page.route(/search_index\.en\.js/, (route) => (refuse ? route.abort() : route.continue()));
  await page.reload();
  const box = page.getByRole("searchbox", { name: "Search the docs" });
  await box.fill("tenant");
  await expect(page.getByRole("status")).toHaveText("Search could not be loaded");
  await expect(page.locator("#search-results")).toBeHidden();
  refuse = false;
  await box.fill("tenants");
  await expect(page.locator("#search-results a").first()).toBeVisible();
});

for (const scheme of ["light", "dark"]) {
  test(`the search box's edge stands out from the header in ${scheme} (WCAG 1.4.11)`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.reload();
    const ratio = await page.getByRole("searchbox", { name: "Search the docs" }).evaluate(borderContrast);
    expect(ratio, "the border against the header's background").toBeGreaterThanOrEqual(3);
  });
}
