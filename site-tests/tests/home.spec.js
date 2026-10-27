// The home page, built and served by scripts/test-site.sh: what it shows,
// and that it meets WCAG 2.2 AA (axe-core) in light and dark at the widths a
// phone, a tablet and a desktop use, without scrolling sideways and without a
// console error (a content-security-policy refusal is one).
import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import path from "node:path";
import { TAGS, describe } from "./axe.js";

const WIDTHS = [360, 768, 1440];
const SCHEMES = ["light", "dark"];
const LANDMARK_RULES = ["landmark-one-main", "region", "landmark-unique", "page-has-heading-one"];

test.describe("what the home page shows", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/");
  });

  test("the hero has the logo, the headline and the two calls to action", async ({ page }) => {
    const hero = page.locator(".hero");
    await expect(hero.locator("img.hero-mark")).toHaveAttribute("src", /favicon\.svg$/);
    await expect(hero.locator("h1")).toHaveText("Encrypted, multi-tenant, audited storage for vCons");
    const install = hero.getByRole("link", { name: "Install", exact: true });
    await expect(install).toHaveClass(/button-primary/);
    await expect(install).toHaveAttribute("href", /\/docs\//);
    const docs = hero.getByRole("link", { name: "Docs", exact: true });
    await expect(docs).toHaveAttribute("href", /\/docs\/$/);
  });

  test("the numbers are tiles read from the generated data file", async ({ page }) => {
    const metrics = page.locator(".metrics");
    await expect(metrics).toHaveAttribute("data-tests", /^[0-9]+$/);
    await expect(metrics.locator(".stat")).toHaveCount(3);
    const tests = await metrics.getAttribute("data-tests");
    await expect(metrics.locator(".stat").first()).toContainText(`${tests} automated tests`);
  });

  test("the trust badges link to their sources", async ({ page }) => {
    const badges = page.locator(".badges");
    for (const href of [
      "https://github.com/NormB/vcon-server/actions/workflows/ci.yml",
      "https://scorecard.dev/viewer/?uri=github.com/NormB/vcon-server",
      "https://www.bestpractices.dev/projects/15941",
      "https://codecov.io/gh/NormB/vcon-server",
      "https://github.com/NormB/vcon-server/releases/latest",
    ]) {
      await expect(badges.locator(`a[href="${href}"]`)).toHaveCount(1);
    }
  });

  test("the guarantees are a grid of six linked cards", async ({ page }) => {
    const cards = page.locator("#features .feature");
    await expect(cards).toHaveCount(6);
    for (const card of await cards.all()) {
      await expect(card.locator("h3")).not.toBeEmpty();
      await expect(card.locator("a")).toHaveAttribute("href", /\/(docs|problems)\//);
    }
  });

  test("the first look stores a vCon and reads it back", async ({ page }) => {
    const steps = page.locator("#first-look .code-frame");
    await expect(steps).toHaveCount(2);
    await expect(steps.nth(0).locator("pre code")).toContainText("--data-binary @call.vcon");
    await expect(steps.nth(1).locator("pre code")).toContainText("/vcons/");
  });

  test("the install and how-to slots are on the page", async ({ page }) => {
    await expect(page.locator("#install h2")).toHaveText("Install");
    await expect(page.locator("#install .code-frame").first()).toBeVisible();
    await expect(page.locator("#how-to h2")).toHaveText("How-to guides");
    expect(await page.locator("#how-to .cards a").count()).toBeGreaterThan(0);
  });

  test("the footer groups its links", async ({ page }) => {
    const foot = page.getByRole("contentinfo");
    for (const name of ["Docs", "Issues", "Releases", "Security policy", "Patreon", "GitHub Sponsors"]) {
      await expect(foot.getByRole("link", { name, exact: true })).toHaveCount(1);
    }
    await expect(foot.locator("nav")).toHaveCount(3);
  });
});

for (const scheme of SCHEMES) {
  for (const width of WIDTHS) {
    test(`the home page meets WCAG 2.2 AA in ${scheme} at ${width} px`, async ({ page }) => {
      const errors = [];
      page.on("console", (m) => {
        if (m.type() === "error") errors.push(m.text());
      });
      page.on("pageerror", (e) => errors.push(e.message));
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/");

      const result = await new AxeBuilder({ page }).withTags(TAGS).analyze();
      expect(result.violations, describe(result.violations)).toEqual([]);
      // The landmark rules ran, rather than being skipped as not applicable.
      const ran = [...result.passes, ...result.violations].map((r) => r.id);
      for (const rule of LANDMARK_RULES) expect(ran, `axe did not run ${rule}`).toContain(rule);

      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow, "the page scrolls sideways").toBeLessThanOrEqual(0);
      expect(errors, errors.join("\n")).toEqual([]);

      if (process.env.SITE_SCREENSHOTS) {
        await page.screenshot({
          path: path.join(process.env.SITE_SCREENSHOTS, `home-${scheme}-${width}.png`),
          fullPage: true,
        });
      }
    });
  }
}
