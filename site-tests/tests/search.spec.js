// The docs search: labelled, reachable by keyboard, and its outcome is a
// status message a screen reader announces without moving focus (WCAG 4.1.3).
import { test, expect } from "@playwright/test";

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
  await box.press("Escape");
  await expect(page.getByRole("status")).toHaveText("");
  await expect(page.locator("#search-results")).toBeHidden();
});

test("the results come next in the keyboard order", async ({ page }) => {
  const box = page.getByRole("searchbox", { name: "Search the docs" });
  await box.fill("tenant");
  await box.press("Tab");
  await expect(page.locator("#search-results a").first()).toBeFocused();
});
