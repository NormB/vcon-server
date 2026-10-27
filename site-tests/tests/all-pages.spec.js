// Every page the site builds, from its sitemap, meets WCAG 2.2 AA (axe-core)
// in light and dark on a phone and a desktop. The home page has its own,
// closer tests in home.spec.js.
import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { TAGS, describe } from "./axe.js";

test("every page meets WCAG 2.2 AA in light and dark at 360 and 1440 px", async ({ page, request }) => {
  test.setTimeout(15 * 60 * 1000);
  const sitemap = await (await request.get("/sitemap.xml")).text();
  const paths = [...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => new URL(m[1]).pathname);
  expect(paths.length, "the sitemap lists the docs and problem pages").toBeGreaterThan(20);
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
