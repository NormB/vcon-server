// The documentation's sidebar and index: the pages under the heading of
// their group, in the order a new reader needs them, with the page being
// read marked. The groups are docs_groups in website/config.toml, and
// crates/vcon-server/tests/docs_index.rs holds every page to one of them.
import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { TAGS, describe, borderContrast, ringCutBy } from "./axe.js";

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

// On a phone the sidebar's 36 links would come before the page's heading,
// about a screen and a half of them. Below the width where the sidebar sits
// beside the page it is folded behind one button, so the page's own content
// comes first (WCAG 2.4.1), in the order it is read (1.3.2), and the links
// stay one key press away. Without JavaScript the links are simply shown.
test("beside the page, the scrolling sidebar does not cut the focus ring of its links", async ({ page }) => {
  await page.goto("/docs/install/");
  const nav = page.getByRole("navigation", { name: "Documentation" });
  for (const name of ["Documentation", "Install", "Keep vCons as Parquet files in a bucket", "Standards"]) {
    const link = nav.getByRole("link", { name, exact: true });
    await link.focus();
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Tab");
    await expect(link).toBeFocused();
    expect(await link.evaluate(ringCutBy), `${name}: boxes that cut the ring`).toEqual([]);
  }
});

const PHONE = { width: 360, height: 800 };
const TOGGLE = "Documentation pages";

test.describe("the sidebar on a phone", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize(PHONE);
    await page.goto("/docs/install/");
  });

  test("the page's heading is on the first screen", async ({ page }) => {
    const top = await page.locator("article h1").evaluate((h) => h.getBoundingClientRect().top);
    expect(top, "the h1's distance from the top of the page").toBeLessThan(PHONE.height / 2);
  });

  test("the links are folded behind a button that says so", async ({ page }) => {
    const toggle = page.getByRole("button", { name: TOGGLE });
    await expect(toggle).toBeVisible();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    const controls = await toggle.getAttribute("aria-controls");
    await expect(page.locator(`#${controls}`)).toBeHidden();
    await expect(page.getByRole("navigation", { name: "Documentation" }).getByRole("link", { name: "Getting started" })).toBeHidden();
  });

  test("the keyboard goes from the button to the page, not through the folded links", async ({ page }) => {
    const toggle = page.getByRole("button", { name: TOGGLE });
    await toggle.focus();
    await page.keyboard.press("Tab");
    const where = await page.evaluate(() => (document.activeElement.closest("article") ? "article" : document.activeElement.outerHTML.slice(0, 80)));
    expect(where, "the stop after the button").toBe("article");
  });

  test("the button opens the links, with the page being read marked, and closes them", async ({ page }) => {
    const nav = page.getByRole("navigation", { name: "Documentation" });
    const toggle = page.getByRole("button", { name: TOGGLE });
    await toggle.focus();
    await page.keyboard.press("Enter");
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(nav.locator(".doc-nav-group")).toHaveText(GROUPS);
    await expect(nav.locator('[aria-current="page"]')).toBeVisible();
    await page.keyboard.press("Tab");
    await expect(nav.getByRole("link", { name: "Documentation", exact: true })).toBeFocused();
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(nav.getByRole("link", { name: "Getting started" })).toBeHidden();
  });

  test("Escape closes the open links and puts the keyboard back on the button", async ({ page }) => {
    const nav = page.getByRole("navigation", { name: "Documentation" });
    const toggle = page.getByRole("button", { name: TOGGLE });
    await toggle.click();
    await nav.getByRole("link", { name: "Getting started" }).focus();
    await page.keyboard.press("Escape");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(toggle).toBeFocused();
  });

  test("the open links keep their headings above them and do not widen the page", async ({ page }) => {
    await page.getByRole("button", { name: TOGGLE }).click();
    const nav = page.getByRole("navigation", { name: "Documentation" });
    const tops = await nav.locator(".doc-nav-group, ul").evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().top)));
    expect(tops, "each heading is above its list, and each list above the next heading").toEqual([...tops].sort((a, b) => a - b));
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, "the page scrolls sideways").toBeLessThanOrEqual(0);
  });

  test("the folded and the open sidebar meet WCAG 2.2 AA in light and dark", async ({ page }) => {
    for (const scheme of ["light", "dark"]) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.reload();
      for (const open of [false, true]) {
        if (open) await page.getByRole("button", { name: TOGGLE }).click();
        const { violations } = await new AxeBuilder({ page }).withTags(TAGS).analyze();
        expect(violations, `${scheme}, ${open ? "open" : "folded"}\n${describe(violations)}`).toEqual([]);
      }
    }
  });

  test("the button is a target of at least 24 by 24 pixels with a visible focus ring", async ({ page }) => {
    const toggle = page.getByRole("button", { name: TOGGLE });
    const box = await toggle.boundingBox();
    expect(Math.min(box.width, box.height), "WCAG 2.5.8 target size").toBeGreaterThanOrEqual(24);
    await page.keyboard.press("Tab");
    await toggle.focus();
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Tab");
    const outline = await toggle.evaluate((b) => getComputedStyle(b).outlineStyle + " " + getComputedStyle(b).outlineWidth);
    expect(outline, "the focus ring, reached with the keyboard").toBe("solid 3px");
  });

  test("no box that clips its contents cuts the button's focus ring", async ({ page }) => {
    const toggle = page.getByRole("button", { name: TOGGLE });
    await toggle.focus();
    expect(await toggle.evaluate(ringCutBy), "boxes that cut the ring").toEqual([]);
  });

  test("the button's edge stands out from the page in light and dark (WCAG 1.4.11)", async ({ page }) => {
    for (const scheme of ["light", "dark"]) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.reload();
      const ratio = await page.getByRole("button", { name: TOGGLE }).evaluate(borderContrast);
      expect(ratio, `${scheme}: the border against the page's background`).toBeGreaterThanOrEqual(3);
    }
  });

  test("the problem pages and the documentation index fold it too", async ({ page }) => {
    for (const path of ["/docs/", "/problems/", "/problems/gone/", "/standards/"]) {
      await page.goto(path);
      await expect(page.getByRole("button", { name: TOGGLE }), path).toHaveAttribute("aria-expanded", "false");
      const top = await page.locator("article h1").evaluate((h) => h.getBoundingClientRect().top);
      expect(top, `${path}: the h1's distance from the top`).toBeLessThan(PHONE.height / 2);
    }
  });
});

test("beside the page the sidebar has no button and always shows its links", async ({ page }) => {
  await page.goto("/docs/install/");
  await expect(page.getByRole("button", { name: TOGGLE })).toBeHidden();
  await expect(page.getByRole("navigation", { name: "Documentation" }).getByRole("link", { name: "Getting started" })).toBeVisible();
});

test("links opened on a phone stay shown when the window widens, and fold again when it narrows", async ({ page }) => {
  await page.setViewportSize(PHONE);
  await page.goto("/docs/install/");
  const link = page.getByRole("navigation", { name: "Documentation" }).getByRole("link", { name: "Getting started" });
  await expect(link).toBeHidden();
  await page.getByRole("button", { name: TOGGLE }).click();
  await expect(link).toBeVisible();
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(link).toBeVisible();
  await expect(page.getByRole("button", { name: TOGGLE })).toBeHidden();
  await page.setViewportSize(PHONE);
  await expect(link).toBeHidden();
  await expect(page.getByRole("button", { name: TOGGLE })).toHaveAttribute("aria-expanded", "false");
});

test("without JavaScript a phone shows every link and no button", async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, viewport: PHONE, baseURL: process.env.SITE_URL });
  const page = await context.newPage();
  await page.goto("/docs/install/");
  await expect(page.getByRole("navigation", { name: "Documentation" }).getByRole("link", { name: "Getting started" })).toBeVisible();
  await expect(page.getByRole("button", { name: TOGGLE })).toHaveCount(0);
  await context.close();
});

test("without JavaScript and without the stylesheet there is still no button that does nothing", async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false, viewport: PHONE, baseURL: process.env.SITE_URL });
  await context.route(/style\.css/, (route) => route.abort());
  const page = await context.newPage();
  await page.goto("/docs/install/");
  await expect(page.getByRole("navigation", { name: "Documentation" }).getByRole("link", { name: "Getting started" })).toBeVisible();
  await expect(page.getByRole("button", { name: TOGGLE })).toHaveCount(0);
  await expect(page.locator("#doc-nav-toggle")).toBeHidden();
  await context.close();
});
