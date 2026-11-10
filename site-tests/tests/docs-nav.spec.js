// The documentation's sidebar and index: the pages under the heading of
// their group, in the order a new reader needs them, with the page being
// read marked. The groups are docs_groups in website/config.toml, and
// crates/vcon-server/tests/docs_index.rs holds every page to one of them.
import { test, expect } from "@playwright/test";

const GROUPS = ["Start here", "How-to guides", "Concepts", "Reference", "Project"];

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
});

test("the sidebar lists the pages under five group headings, in order", async ({ page }) => {
  await page.goto("/docs/install/");
  const nav = page.getByRole("navigation", { name: "Documentation" });
  await expect(nav.locator(".doc-nav-group")).toHaveText(GROUPS);
  for (const group of GROUPS) {
    const list = nav.getByRole("list", { name: group, exact: true });
    expect(await list.getByRole("link").count(), `${group} has pages`).toBeGreaterThan(0);
  }
});

test("the first group is the first-visit path: install, start, store a vCon", async ({ page }) => {
  await page.goto("/docs/");
  const start = page
    .getByRole("navigation", { name: "Documentation" })
    .getByRole("list", { name: "Start here", exact: true });
  await expect(start.getByRole("link")).toHaveText(["Install", "Getting started", "Store your first vCon"]);
});

test("every sidebar link is in exactly one group", async ({ page }) => {
  await page.goto("/docs/");
  const nav = page.getByRole("navigation", { name: "Documentation" });
  const hrefs = await nav.locator("ul a").evaluateAll((links) => links.map((a) => a.getAttribute("href")));
  expect(hrefs.length).toBeGreaterThan(30);
  expect(new Set(hrefs).size, "a page is listed twice").toBe(hrefs.length);
});

for (const [path, name] of [
  ["/docs/", "Documentation"],
  ["/docs/install/", "Install"],
  ["/docs/http-api/", "HTTP API"],
  ["/problems/", "Problem types"],
  ["/standards/", "Standards"],
]) {
  test(`the sidebar marks ${name} as the page being read on ${path}`, async ({ page }) => {
    await page.goto(path);
    const current = page.getByRole("navigation", { name: "Documentation" }).locator('[aria-current="page"]');
    await expect(current).toHaveCount(1);
    await expect(current).toHaveText(name);
  });
}

test("a problem type's page marks no sidebar entry, and keeps the groups", async ({ page }) => {
  await page.goto("/problems/gone/");
  const nav = page.getByRole("navigation", { name: "Documentation" });
  await expect(nav.locator('[aria-current="page"]')).toHaveCount(0);
  await expect(nav.locator(".doc-nav-group")).toHaveText(GROUPS);
});

test("the documentation index has a heading and cards for each group", async ({ page }) => {
  await page.goto("/docs/");
  const main = page.getByRole("main");
  await expect(main.locator("article h2")).toHaveText(GROUPS);
  const cards = await main.locator("article .cards a").evaluateAll((links) => links.map((a) => a.getAttribute("href")));
  const side = await page
    .getByRole("navigation", { name: "Documentation" })
    .locator("ul a")
    .evaluateAll((links) => links.map((a) => a.getAttribute("href")));
  // The sidebar's Reference group also links the problem types and the
  // standards, which are not pages of the documentation section.
  const pages = side.filter((href) => new URL(href).pathname.startsWith("/docs/"));
  expect(cards, "the index's cards are the sidebar's pages, in its order").toEqual(pages);
});

test("the group headings stay with their pages on a phone", async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto("/docs/install/");
  const nav = page.getByRole("navigation", { name: "Documentation" });
  const tops = await nav.locator(".doc-nav-group, ul").evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().top)));
  expect(tops, "each heading is above its list, and each list above the next heading").toEqual([...tops].sort((a, b) => a - b));
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow, "the page scrolls sideways").toBeLessThanOrEqual(0);
});
