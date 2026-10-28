// Every page the site builds, from its sitemap, meets WCAG 2.2 AA: axe-core's
// rules in light and dark on a phone and a desktop, and three AA criteria axe
// does not test: reflow at 320 px (1.4.10), focus not obscured by the sticky
// header (2.4.11), and anchor targets not scrolled under it. The home page
// has its own, closer tests in home.spec.js.
import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { TAGS, describe } from "./axe.js";

async function sitemapPaths(request) {
  const sitemap = await (await request.get("/sitemap.xml")).text();
  const paths = [...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => new URL(m[1]).pathname);
  expect(paths.length, "the sitemap lists the docs and problem pages").toBeGreaterThan(20);
  return paths;
}

// The part of the viewport the site header covers: its bottom edge while it
// is pinned (sticky or fixed) and on screen, else 0.
function headerCover() {
  const h = document.querySelector("header.top");
  if (!h) return 0;
  const pos = getComputedStyle(h).position;
  if (pos !== "sticky" && pos !== "fixed") return 0;
  return Math.max(0, h.getBoundingClientRect().bottom);
}

test("every page meets WCAG 2.2 AA in light and dark at 360 and 1440 px", async ({ page, request }) => {
  test.setTimeout(15 * 60 * 1000);
  const paths = await sitemapPaths(request);
  const found = [];
  for (const scheme of ["light", "dark"]) {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    for (const width of [360, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const p of paths) {
        await page.goto(p);
        const { violations } = await new AxeBuilder({ page }).withTags(TAGS).analyze();
        if (violations.length) found.push(`${p} (${scheme}, ${width} px)\n${describe(violations)}`);
      }
    }
  }
  expect(found, found.join("\n\n")).toEqual([]);
});

test("no page scrolls sideways at 320 px (WCAG 1.4.10 reflow)", async ({ page, request }) => {
  test.setTimeout(10 * 60 * 1000);
  const paths = await sitemapPaths(request);
  await page.setViewportSize({ width: 320, height: 700 });
  const found = [];
  for (const p of paths) {
    await page.goto(p);
    const over = await page.evaluate(() => {
      const vw = document.documentElement.clientWidth;
      const wide = [...document.querySelectorAll("body *")]
        .filter((e) => e.getBoundingClientRect().right > vw + 1 && !e.parentElement.closest("pre, table"))
        .map((e) => `${e.tagName.toLowerCase()} "${(e.textContent || "").trim().slice(0, 50)}"`);
      return { by: document.documentElement.scrollWidth - vw, wide: wide.slice(0, 3) };
    });
    if (over.by > 0) found.push(`${p}: ${over.by} px, ${over.wide.join(", ")}`);
  }
  expect(found, found.join("\n")).toEqual([]);
});

test("no keyboard focus is hidden under the header (WCAG 2.4.11)", async ({ page, request }) => {
  test.setTimeout(20 * 60 * 1000);
  const paths = await sitemapPaths(request);
  const found = [];
  for (const width of [360, 900, 1440]) {
    await page.setViewportSize({ width, height: 700 });
    for (const p of paths) {
      await page.goto(p);
      const stops = await page.evaluate(
        () => document.querySelectorAll('a[href], input:not([type="hidden"]), button, [tabindex="0"]').length,
      );
      // Backward from the end of the page: the browser scrolls each stop to
      // the top edge, where a pinned header would cover it.
      await page.evaluate(() => [...document.querySelectorAll("footer a")].pop().focus());
      for (let i = 0; i < stops; i++) {
        await page.keyboard.press("Shift+Tab");
        const hidden = await page.evaluate((cover) => {
          const e = document.activeElement;
          if (!e || e === document.body) return "end";
          if (e.closest("header.top") || e.classList.contains("skip")) return null;
          const r = e.getBoundingClientRect();
          const top = new Function(`return (${cover})()`)();
          if (r.bottom <= top + 1 || r.top >= innerHeight) {
            return `${e.tagName.toLowerCase()} "${(e.textContent || "").trim().slice(0, 40)}" spans ${Math.round(r.top)} to ${Math.round(r.bottom)}, header covers 0 to ${Math.round(top)}`;
          }
          return null;
        }, headerCover.toString());
        if (hidden === "end") break;
        if (hidden) found.push(`${p} at ${width} px: ${hidden}`);
      }
    }
  }
  expect(found, `${found.length} focus stops hidden:\n${found.slice(0, 40).join("\n")}`).toEqual([]);
});

test("a link to a place on a page shows that place below the header", async ({ page, request }) => {
  test.setTimeout(10 * 60 * 1000);
  const paths = await sitemapPaths(request);
  const found = [];
  for (const width of [360, 1440]) {
    await page.setViewportSize({ width, height: 700 });
    for (const p of paths) {
      await page.goto(p);
      const targets = await page.$$eval('a[href^="#"]', (as) => [...new Set(as.map((a) => a.getAttribute("href")).filter((h) => h.length > 1))].slice(0, 8));
      for (const hash of targets) {
        await page.goto(p + hash);
        const under = await page.evaluate(
          ([id, cover]) => {
            const t = document.getElementById(decodeURIComponent(id.slice(1)));
            if (!t) return `${id} names no element`;
            const top = new Function(`return (${cover})()`)();
            const r = t.getBoundingClientRect();
            // A target at the page's very top cannot scroll further down.
            const atTop = scrollY === 0;
            return r.top < top - 1 && !atTop ? `${id} top ${Math.round(r.top)} under the header to ${Math.round(top)}` : null;
          },
          [hash, headerCover.toString()],
        );
        if (under) found.push(`${p} at ${width} px: ${under}`);
      }
    }
  }
  expect(found, found.join("\n")).toEqual([]);
});
